//! Step 3, Machines: the ssh machines the agents run on.
//!
//! Found in three places, all read locally: your `~/.ssh/config` Host
//! aliases (resolved with `ssh -G`), the peers `tailscale status --json`
//! lists (when Tailscale is here; read-only), and `~/.ssh/known_hosts`. Pick
//! some; each picked machine gets a login check right away (BatchMode, never
//! a prompt). A host key that isn't known yet is fetched and its fingerprints
//! shown — you decide whether to trust it; a key that changed is refused. A
//! machine that wants a password gets key login (`ssh-copy-id`, the password
//! typed into ssh's own prompt). A machine that isn't in your ssh config yet
//! gets a Host block (a backup of the file is kept). Then each machine's
//! colour and tags, and the default one.

use super::rows::{Row, St};
use super::{Aside, Ev, Hand, Msg, Run, Step, Wiz};
use crate::sshcfg::{self, Fail};
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cand {
    /// its ssh alias (a suggested one when `new`)
    pub alias: String,
    /// where it points (HostName, a Tailscale name, an address)
    pub hostname: String,
    /// `user@host:port` as ssh resolves it
    pub addr: String,
    pub port: Option<u16>,
    /// ssh config · fleet · tailscale · known_hosts
    pub from: Vec<&'static str>,
    /// Tailscale's OS and whether the peer is online
    pub note: String,
    /// not in your ssh config yet (picking it adds a Host block)
    pub new: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Login {
    /// logged in without a prompt, in this many ms
    Ok(u128),
    /// first contact: the key's fingerprints and the known_hosts lines
    NewKey {
        fps: Vec<String>,
        lines: Vec<String>,
    },
    Changed(String),
    Auth(String),
    Fail(String),
}

#[derive(Default)]
pub(crate) struct State {
    pub cands: Vec<Cand>,
    pub found: bool,
    pub login: HashMap<String, Login>,
    /// the alias typed for a new machine, waiting for its user@host
    pub new_alias: Option<String>,
}

pub(crate) fn init(_w: &mut Wiz) {}

pub(crate) fn enter(w: &mut Wiz) {
    if !w.mach.found {
        discover(w);
    }
    check_all(w, false);
}

pub(crate) fn retry(w: &mut Wiz) {
    w.mach.found = false;
    discover(w);
    check_all(w, true);
}

fn discover(w: &mut Wiz) {
    let fleet = w.machines();
    w.spawn(
        "discover",
        "Looking in ~/.ssh/config, Tailscale and known_hosts",
        move || Msg::Found(find_all(&fleet)),
    );
}

pub(crate) fn found(w: &mut Wiz, v: Vec<Cand>) {
    w.mach.found = true;
    w.mach.cands = v;
}

/// Check key login to every picked machine not checked yet (`again`: all).
fn check_all(w: &mut Wiz, again: bool) {
    let todo: Vec<String> = w
        .machines()
        .into_iter()
        .filter(|h| !sshcfg::is_local(h))
        .filter(|h| again || !w.mach.login.contains_key(h))
        .collect();
    for h in todo {
        check(w, &h);
    }
}

fn check(w: &mut Wiz, alias: &str) {
    if w.no_network() {
        w.toast(
            false,
            "offline — the machine checks wait for the network (r checks again)",
        );
        return;
    }
    w.mach.login.remove(alias);
    let a = alias.to_string();
    w.spawn(
        &format!("login:{alias}"),
        format!("Checking key login to {alias}"),
        move || {
            let l = check_login(&a);
            Msg::Login(a, l)
        },
    );
}

pub(crate) fn login(w: &mut Wiz, host: &str, l: Login) {
    w.mach.login.insert(host.to_string(), l);
}

pub(crate) fn trusted(w: &mut Wiz, host: &str, r: Result<(), String>) {
    match r {
        Ok(()) => {
            w.toast(
                false,
                format!("{host}'s host key is trusted (~/.ssh/known_hosts)"),
            );
            check(w, host);
        }
        Err(e) => w.toast(true, format!("couldn't add the key: {e}")),
    }
}

/// One BatchMode login; for an unknown host key, its fingerprints.
fn check_login(alias: &str) -> Login {
    let t0 = Instant::now();
    let p = sshcfg::probe(alias);
    match p.fail {
        None => Login::Ok(t0.elapsed().as_millis()),
        Some(Fail::HostKeyUnknown) => {
            let (host, port) = sshcfg::resolve(alias)
                .map(|r| (r.hostname, r.port.parse::<u16>().ok().filter(|p| *p != 22)))
                .unwrap_or((alias.to_string(), None));
            let (fps, lines) = sshcfg::host_keys(&host, port);
            if lines.is_empty() {
                Login::Fail("its host key isn't known, and ssh-keyscan couldn't fetch it".into())
            } else {
                Login::NewKey { fps, lines }
            }
        }
        Some(Fail::HostKeyChanged) => Login::Changed(p.detail),
        Some(Fail::Auth) => Login::Auth(p.explain()),
        Some(_) => Login::Fail(p.explain()),
    }
}

// ------------------------------------------------------------------ finding them

/// A Tailscale peer (`tailscale status --json`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Peer {
    pub name: String,
    pub dns: String,
    pub ips: Vec<String>,
    pub os: String,
    pub online: bool,
}

/// Pure: the peers in `tailscale status --json` (this computer left out).
pub(crate) fn parse_tailscale(text: &str) -> Vec<Peer> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return vec![];
    };
    let Some(peers) = v.get("Peer").and_then(|p| p.as_object()) else {
        return vec![];
    };
    let s = |p: &serde_json::Value, k: &str| {
        p.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let mut out: Vec<Peer> = peers
        .values()
        .map(|p| Peer {
            name: s(p, "HostName"),
            dns: s(p, "DNSName").trim_end_matches('.').to_string(),
            ips: p
                .get("TailscaleIPs")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            os: s(p, "OS"),
            online: p.get("Online").and_then(|x| x.as_bool()).unwrap_or(false),
        })
        .filter(|p| !p.name.is_empty())
        .collect();
    out.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.cmp(&b.name)));
    out
}

/// Pure: (host, port) for each machine in a known_hosts file. Hashed
/// entries can't be read back and are skipped; an address is kept only
/// when its line names nothing else.
pub(crate) fn parse_known_hosts(text: &str) -> Vec<(String, Option<u16>)> {
    let mut out: Vec<(String, Option<u16>)> = Vec::new();
    for l in text.lines() {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with('@') || t.starts_with("|1|") {
            continue;
        }
        let Some(first) = t.split_whitespace().next() else {
            continue;
        };
        let names: Vec<(String, Option<u16>)> = first
            .split(',')
            .map(
                |n| match n.strip_prefix('[').and_then(|r| r.split_once("]:")) {
                    Some((h, p)) => (h.to_string(), p.parse().ok()),
                    None => (n.to_string(), None),
                },
            )
            .filter(|(h, _)| !h.is_empty() && !h.contains(['*', '?']))
            .collect();
        let pick = names
            .iter()
            .find(|(h, _)| h.parse::<std::net::IpAddr>().is_err())
            .or_else(|| names.first())
            .cloned();
        if let Some(p) = pick {
            if !out.iter().any(|x| x.0 == p.0) {
                out.push(p);
            }
        }
    }
    out
}

/// A name ssh accepts as an alias: `GPU-Box.lan` → `gpu-box`.
pub(crate) fn alias_for(host: &str) -> String {
    let base = if host.parse::<std::net::IpAddr>().is_ok() {
        host.to_string()
    } else {
        host.split('.').next().unwrap_or(host).to_string()
    };
    base.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Everything there is to pick from (runs off the UI thread).
fn find_all(fleet: &[String]) -> Vec<Cand> {
    let aliases = sshcfg::aliases();
    let mut names: Vec<String> = fleet
        .iter()
        .filter(|h| !sshcfg::is_local(h))
        .cloned()
        .collect();
    for a in &aliases {
        if !names.contains(a) {
            names.push(a.clone());
        }
    }
    let handles: Vec<_> = names
        .iter()
        .map(|n| {
            let n = n.clone();
            std::thread::spawn(move || sshcfg::resolve(&n))
        })
        .collect();
    let mut out: Vec<Cand> = names
        .iter()
        .zip(handles)
        .map(|(n, h)| {
            let r = h.join().ok().flatten();
            Cand {
                alias: n.clone(),
                hostname: r
                    .as_ref()
                    .map(|r| r.hostname.clone())
                    .unwrap_or_else(|| n.clone()),
                addr: r.as_ref().map(|r| r.short()).unwrap_or_default(),
                port: r.as_ref().and_then(|r| r.port.parse().ok()),
                from: vec![if aliases.contains(n) {
                    "ssh config"
                } else {
                    "fleet"
                }],
                note: String::new(),
                new: false,
            }
        })
        .collect();
    if let Some(ts) = crate::proc::which("tailscale") {
        let o = crate::proc::cap(
            &ts.to_string_lossy(),
            &["status", "--json"],
            Duration::from_secs(8),
        );
        if o.ok() {
            for p in parse_tailscale(&o.text()) {
                add_peer(&mut out, p);
            }
        }
    }
    let kh = crate::config::home().join(".ssh/known_hosts");
    let text = std::fs::read_to_string(kh).unwrap_or_default();
    for (h, port) in parse_known_hosts(&text).into_iter().take(40) {
        if out.iter().any(|c| c.hostname == h || c.alias == h) {
            continue;
        }
        let alias = alias_for(&h);
        if alias.is_empty() || out.iter().any(|c| c.alias == alias) {
            continue;
        }
        out.push(Cand {
            alias,
            hostname: h.clone(),
            addr: match port {
                Some(p) => format!("{h}:{p}"),
                None => h,
            },
            port,
            from: vec!["known_hosts"],
            note: String::new(),
            new: true,
        });
    }
    out
}

/// Pure: a Tailscale peer joins the list, or tags the machine it already is.
pub(crate) fn add_peer(out: &mut Vec<Cand>, p: Peer) {
    let note = format!(
        "{}{}",
        if p.os.is_empty() {
            String::new()
        } else {
            format!("{} · ", p.os)
        },
        if p.online { "online" } else { "offline" }
    );
    if let Some(c) = out.iter_mut().find(|c| {
        c.hostname == p.dns
            || p.ips.contains(&c.hostname)
            || c.hostname.split('.').next() == Some(p.name.to_lowercase().as_str())
    }) {
        if !c.from.contains(&"tailscale") {
            c.from.push("tailscale");
        }
        c.note = note;
        return;
    }
    let alias = alias_for(&p.name);
    if alias.is_empty() || out.iter().any(|c| c.alias == alias) {
        return;
    }
    let host = if p.dns.is_empty() {
        p.ips.first().cloned().unwrap_or_default()
    } else {
        p.dns.clone()
    };
    out.push(Cand {
        alias,
        addr: host.clone(),
        hostname: host,
        port: None,
        from: vec!["tailscale"],
        note,
        new: true,
    });
}

// ------------------------------------------------------------------ rows

fn login_row(w: &Wiz, h: &str) -> Row {
    let id = format!("login:{h}");
    let busy = w.running(&id);
    let (st, text, fix, why) = match (busy, w.mach.login.get(h)) {
        (true, _) => (St::Busy, "checking key login".to_string(), None, ""),
        (_, None) if w.no_network() => (St::Todo, "waiting for the network".to_string(), None, ""),
        (_, None) => (St::Todo, "not checked yet".to_string(), Some("check"), ""),
        (_, Some(Login::Ok(ms))) => (St::Ok, format!("key login works · {ms} ms"), None, ""),
        (_, Some(Login::NewKey { fps, .. })) => (
            St::Warn,
            format!("its host key isn't trusted yet · {}", fps.first().cloned().unwrap_or_default()),
            Some("see the key, then trust it"),
            "The first time you reach a machine, ssh can't know that it's really that machine. Aleph fetched its host key (ssh-keyscan) and shows the fingerprints: compare them with what the machine's admin gives you (ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub on it). Only then trust it; Aleph never does that on its own.",
        ),
        (_, Some(Login::Changed(_))) => (
            St::Bad,
            "its host key CHANGED since last time — refused".to_string(),
            None,
            "The machine answered with a different host key than the one in ~/.ssh/known_hosts. That's what a reinstall looks like — and what someone in the middle looks like. Aleph won't touch it. Find out why with the machine's admin; once you're sure, remove the old key yourself (ssh-keygen -R <host>) and check again.",
        ),
        (_, Some(Login::Auth(t))) => (
            St::Warn,
            t.to_string(),
            Some("set up key login"),
            "The machine wants a password, and Aleph never types one. Key login copies your public key to it once (ssh-copy-id): ssh asks for the password itself, in its own prompt — Aleph never sees it.",
        ),
        (_, Some(Login::Fail(t))) => (St::Bad, t.clone(), Some("check again"), "ssh couldn't log in: the reason is ssh's own. aleph doctor --host <machine> looks deeper."),
    };
    let mut r = Row::check(st, id, h.to_string(), text);
    if let Some(f) = fix {
        r = r.fix(f);
    }
    if !why.is_empty() {
        r = r.why(why);
    }
    r
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let chosen = w.machines();
    let mut v = vec![Row::head("Your machines")];
    let mut cands = w.mach.cands.clone();
    // the fleet's machines even before discovery answers
    for h in chosen.iter().filter(|h| !sshcfg::is_local(h)) {
        if !cands.iter().any(|c| c.alias == *h) {
            cands.insert(
                0,
                Cand {
                    alias: h.clone(),
                    hostname: h.clone(),
                    addr: String::new(),
                    port: None,
                    from: vec!["fleet"],
                    note: String::new(),
                    new: false,
                },
            );
        }
    }
    cands.sort_by_key(|c| (!chosen.contains(&c.alias), c.new));
    for cd in &cands {
        let on = chosen.contains(&cd.alias);
        let mut note = vec![];
        if !cd.addr.is_empty() {
            note.push(Span::styled(cd.addr.clone(), ui::muted()));
        }
        note.push(Span::styled(
            format!("  {}", cd.from.join(" · ")),
            ui::faint(),
        ));
        if !cd.note.is_empty() {
            note.push(Span::styled(format!(" · {}", cd.note), ui::faint()));
        }
        if cd.new {
            note.push(Span::styled("  + adds a Host block", ui::fg(c::ACCENT())));
        }
        if on {
            match w.mach.login.get(&cd.alias) {
                Some(Login::Ok(_)) => note.push(Span::styled("  ✓", ui::fg(c::GREEN()))),
                Some(Login::Changed(_)) => {
                    note.push(Span::styled("  ✗ key changed", ui::fg(c::RED())))
                }
                Some(_) => note.push(Span::styled("  !", ui::fg(c::YELLOW()))),
                None => {}
            }
        }
        v.push(
            Row::pick(format!("m:{}", cd.alias), cd.alias.clone(), on)
                .spans(note)
                .why("Space picks it for the fleet: the machines the dashboard shows together. Taking one out only changes the fleet list, never the machine."),
        );
    }
    v.push(
        Row::pick("local", "this computer", w.flag("local"))
            .note("its own herdr: sessions right here")
            .why("This computer's own herdr, as the machine `local`: aleph local <agent> starts a session here, and it's in the dashboard with the others."),
    );
    if w.running("discover") {
        v.push(Row::check(
            St::Busy,
            "discovering",
            "…",
            "looking in ~/.ssh/config, Tailscale and known_hosts".into(),
        ));
    } else if w.mach.found && cands.is_empty() {
        v.push(Row::info("No machines found yet — add one below."));
    }
    v.push(Row::button("add", "+ add a machine…").why(
        "A new Host block in ~/.ssh/config: an alias, and user@hostname[:port]. A backup of the file is kept.",
    ));
    let remote: Vec<String> = chosen
        .iter()
        .filter(|h| !sshcfg::is_local(h))
        .cloned()
        .collect();
    if !remote.is_empty() {
        v.push(Row::head("Login"));
        for h in &remote {
            v.push(login_row(w, h));
        }
        v.push(Row::button("check", "check them again"));
    }
    if !chosen.is_empty() {
        v.push(Row::head("Each machine"));
        let names: Vec<(String, String)> =
            std::iter::once(("auto".to_string(), "auto".to_string()))
                .chain(
                    crate::theme::MACHINES
                        .iter()
                        .map(|(n, _, _)| (n.to_string(), n.to_string())),
                )
                .collect();
        for h in &chosen {
            let key = format!("host.{h}.color");
            let cur = w.value(&key);
            let i = names.iter().position(|(n, _)| *n == cur).unwrap_or(0);
            let col =
                crate::ui::parse_color(&cur).unwrap_or_else(|| crate::theme::machine(&w.cfg, h));
            v.push(
                Row::local(format!("color:{h}"), format!("{h} colour"), names.clone(), i)
                    .spans(vec![Span::raw("  "), Span::styled("▌■", Style::new().fg(col))])
                    .why("A machine keeps its colour everywhere: the dashboard, tiles, the launch card, herdr's sidebar."),
            );
            let tk = format!("host.{h}.tags");
            v.push(
                Row::input(tk.clone(), format!("{h} tags"), w.value(&tk), "e.g. gpu, lab — presets can pick machines by tag", w.pending(&tk))
                    .why("Words that describe the machine (gpu, lab, big). A preset's pane can say host = \"tag:gpu\" and get one of them."),
            );
        }
        let opts: Vec<(String, String)> = chosen.iter().map(|h| (h.clone(), h.clone())).collect();
        let mut cur = w.value("ssh_host");
        if !chosen.contains(&cur) {
            cur = chosen[0].clone();
        }
        v.push(
            Row::choice(
                "ssh_host",
                "default machine",
                opts,
                cur,
                w.pending("ssh_host"),
            )
            .why("Where aleph new starts a session when you don't say --host."),
        );
    }
    v
}

pub(crate) fn aside(w: &Wiz, _width: u16) -> Option<Aside> {
    let r = w.selected()?;
    let id = match &r.kind {
        super::rows::Kind::Pick { id, .. } => id.strip_prefix("m:")?.to_string(),
        super::rows::Kind::Check { id, .. } => id.strip_prefix("login:")?.to_string(),
        _ => return None,
    };
    let cd = w.mach.cands.iter().find(|c| c.alias == id);
    let mut lines = vec![Line::from(vec![
        Span::styled("▌", Style::new().fg(crate::theme::machine(&w.cfg, &id))),
        Span::styled(id.clone(), ui::bold()),
    ])];
    let kv = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<9}"), ui::muted()),
            Span::raw(v),
        ])
    };
    if let Some(cd) = cd {
        if !cd.addr.is_empty() {
            lines.push(kv("address", cd.addr.clone()));
        }
        lines.push(kv("from", cd.from.join(", ")));
        if !cd.note.is_empty() {
            lines.push(kv("tailscale", cd.note.clone()));
        }
        if cd.new {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                format!("Picking it adds `Host {}` to ~/.ssh/config.", cd.alias),
                ui::fg(c::ACCENT()),
            )));
        }
    }
    match w.mach.login.get(&id) {
        Some(Login::NewKey { fps, .. }) => {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "Its host key (not trusted yet):",
                ui::bold(),
            )));
            for f in fps {
                lines.push(Line::from(Span::styled(
                    format!("  {f}"),
                    ui::fg(c::YELLOW()),
                )));
            }
            lines.push(Line::from(Span::styled(
                "Compare before you trust it.",
                ui::muted(),
            )));
        }
        Some(Login::Changed(d)) => {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "Host key CHANGED — Aleph won't connect.",
                ui::fg(c::RED()).add_modifier(Modifier::BOLD),
            )));
            for t in ui::wrap(d, 40).into_iter().take(3) {
                lines.push(Line::from(Span::styled(t, ui::faint())));
            }
        }
        Some(Login::Ok(ms)) => lines.push(kv("login", format!("✓ key login · {ms} ms"))),
        Some(Login::Auth(t)) | Some(Login::Fail(t)) => lines.push(kv("login", t.clone())),
        None => {}
    }
    Some(Aside {
        title: "Machine".into(),
        lines,
    })
}

// ------------------------------------------------------------------ acting

/// Pick or unpick `alias` (the default follows).
fn toggle(w: &mut Wiz, alias: &str) {
    let mut v = w.machines();
    if let Some(i) = v.iter().position(|h| h == alias) {
        v.remove(i);
    } else {
        v.push(alias.to_string());
    }
    let remote: Vec<String> = v.into_iter().filter(|h| !sshcfg::is_local(h)).collect();
    w.set("hosts", &remote.join(","));
    let def = w.value("ssh_host");
    if !remote.contains(&def) {
        w.set("ssh_host", remote.first().map(String::as_str).unwrap_or(""));
    }
    if remote.iter().any(|h| h == alias) {
        check(w, alias);
    }
}

fn add_block(w: &mut Wiz, alias: &str, target: &str) -> bool {
    let (user, hostname, port) = match sshcfg::parse_target(target) {
        Ok(x) => x,
        Err(e) => {
            w.toast(true, e);
            return false;
        }
    };
    let h = sshcfg::NewHost {
        alias: alias.to_string(),
        user,
        hostname: hostname.clone(),
        port,
        // the key Aleph uses (step 1), once it's there
        identity: Some(w.value("ssh_key"))
            .filter(|k| !k.trim().is_empty())
            .filter(|k| {
                std::path::Path::new(&k.replacen('~', &crate::config::home().to_string_lossy(), 1))
                    .is_file()
            }),
        jump: None,
    };
    match sshcfg::add_host(&h) {
        Ok(b) => {
            w.toast(
                false,
                format!(
                    "added Host {alias} to ~/.ssh/config{}",
                    b.map(|b| format!(" (backup {})", crate::doctor::tilde(&b)))
                        .unwrap_or_default()
                ),
            );
            match w.mach.cands.iter_mut().find(|c| c.alias == alias) {
                Some(c) => {
                    c.new = false;
                    c.from.insert(0, "ssh config");
                    c.addr = target.to_string();
                }
                None => w.mach.cands.push(Cand {
                    alias: alias.to_string(),
                    hostname,
                    addr: target.to_string(),
                    port,
                    from: vec!["ssh config"],
                    note: String::new(),
                    new: false,
                }),
            }
            true
        }
        Err(e) => {
            w.toast(true, e);
            false
        }
    }
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Pick(id) if id == "local" => {
            let on = w.flag("local");
            w.set("local", if on { "false" } else { "true" });
        }
        Ev::Pick(id) => {
            let a = id.strip_prefix("m:")?.to_string();
            let cd = w.mach.cands.iter().find(|c| c.alias == a).cloned();
            if cd.as_ref().is_some_and(|c| c.new) {
                let cd = cd?;
                let user = std::env::var("USER").unwrap_or_default();
                let mut t = if user.is_empty() {
                    cd.hostname.clone()
                } else {
                    format!("{user}@{}", cd.hostname)
                };
                if let Some(p) = cd.port.filter(|p| *p != 22) {
                    t.push_str(&format!(":{p}"));
                }
                w.edit(
                    &format!("Add {a} to your ssh config"),
                    &format!("user@hostname[:port] for `Host {a}` — a Host block is added to ~/.ssh/config (a backup is kept)"),
                    &t,
                    &format!("addto:{a}"),
                );
                return None;
            }
            toggle(w, &a);
        }
        Ev::Input(id, text) if id.starts_with("addto:") => {
            let a = id.trim_start_matches("addto:").to_string();
            if !text.is_empty() && add_block(w, &a, &text) {
                toggle(w, &a);
            }
        }
        Ev::Button(id) if id == "add" => {
            w.edit(
                "New machine",
                "an alias for it (letters, digits, . _ -), e.g. gpu3",
                "",
                "new-alias",
            );
        }
        Ev::Input(id, text) if id == "new-alias" => {
            if text.is_empty() {
                return None;
            }
            if let Err(e) = sshcfg::valid_alias(&text) {
                w.toast(true, e);
                return None;
            }
            if sshcfg::aliases().contains(&text) {
                if !w.machines().contains(&text) {
                    toggle(w, &text);
                }
                w.toast(
                    false,
                    format!("{text} is already in your ssh config — picked it"),
                );
                return None;
            }
            w.mach.new_alias = Some(text.clone());
            w.edit(
                &format!("Connect {text} to"),
                "user@hostname[:port] — a Host block is added to ~/.ssh/config (a backup is kept)",
                "",
                "new-target",
            );
        }
        Ev::Input(id, text) if id == "new-target" => {
            let a = w.mach.new_alias.take()?;
            if !text.is_empty() && add_block(w, &a, &text) {
                toggle(w, &a);
            }
        }
        Ev::Button(id) if id == "check" => check_all(w, true),
        Ev::Fix(id) => {
            let h = id.strip_prefix("login:")?.to_string();
            match w.mach.login.get(&h).cloned() {
                Some(Login::NewKey { fps, .. }) => {
                    let mut lines = vec![
                        Line::raw(format!("{h} answered with this host key:")),
                        Line::raw(""),
                    ];
                    for f in &fps {
                        lines.push(Line::from(Span::styled(
                            format!("  {f}"),
                            ui::fg(c::YELLOW()).add_modifier(Modifier::BOLD),
                        )));
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::raw(
                        "Trust it only if it matches what the machine's admin gives you. Aleph adds it to ~/.ssh/known_hosts.".to_string(),
                    ));
                    w.confirm(
                        &format!("Trust {h}'s host key?"),
                        lines,
                        &format!("trust:{h}"),
                    );
                }
                Some(Login::Auth(_)) => {
                    let hand = Hand {
                        title: format!("Key login for {h}"),
                        note: format!("ssh-copy-id copies your public key to {h}. ssh asks for {h}'s password itself — Aleph never sees it."),
                        run: Run::KeySetup(h.clone()),
                        step: Step::Machines,
                        id: format!("key:{h}"),
                    };
                    w.confirm_hand(hand, vec![]);
                }
                _ => check(w, &h),
            }
        }
        Ev::Confirm(id) if id.starts_with("trust:") => {
            let h = id.trim_start_matches("trust:").to_string();
            if let Some(Login::NewKey { lines, .. }) = w.mach.login.get(&h).cloned() {
                w.spawn(
                    &format!("trust:{h}"),
                    format!("Trusting {h}'s host key"),
                    move || Msg::Trusted(h, sshcfg::trust(&lines)),
                );
            }
        }
        Ev::Local(id, i) if id.starts_with("color:") => {
            let h = id.trim_start_matches("color:").to_string();
            let v = if i == 0 {
                String::new()
            } else {
                crate::theme::MACHINES
                    .get(i - 1)
                    .map(|x| x.0.to_string())
                    .unwrap_or_default()
            };
            w.set(&format!("host.{h}.color"), &v);
        }
        Ev::After(id, _) if id.starts_with("key:") => {
            let h = id.trim_start_matches("key:").to_string();
            check(w, &h);
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscale_peers_parse_and_merge() {
        let j = r#"{"Self":{"HostName":"mac"},"Peer":{
            "n1":{"HostName":"dex42","DNSName":"dex42.tail1234.ts.net.","TailscaleIPs":["100.64.0.5"],"OS":"linux","Online":true},
            "n2":{"HostName":"b31","DNSName":"b31.tail1234.ts.net.","TailscaleIPs":["100.64.0.6"],"OS":"linux","Online":false},
            "n3":{"HostName":"Phone","DNSName":"phone.tail1234.ts.net.","TailscaleIPs":["100.64.0.7"],"OS":"iOS","Online":true}}}"#;
        let p = parse_tailscale(j);
        assert_eq!(p.len(), 3);
        assert!(p[0].online && p[1].online && !p[2].online, "{p:?}");
        assert_eq!(
            p.iter().find(|x| x.name == "dex42").unwrap().dns,
            "dex42.tail1234.ts.net"
        );
        let mut out = vec![Cand {
            alias: "b31".into(),
            hostname: "100.64.0.6".into(),
            addr: "selene@100.64.0.6".into(),
            port: None,
            from: vec!["ssh config"],
            note: String::new(),
            new: false,
        }];
        for x in p {
            add_peer(&mut out, x);
        }
        // b31 is the same machine: tagged, not added twice
        assert_eq!(out.iter().filter(|c| c.alias == "b31").count(), 1);
        assert!(out[0].from.contains(&"tailscale"));
        let d = out.iter().find(|c| c.alias == "dex42").unwrap();
        assert!(d.new && d.hostname == "dex42.tail1234.ts.net");
        assert!(out.iter().any(|c| c.alias == "phone"));
        assert!(parse_tailscale("not json").is_empty());
    }

    #[test]
    fn known_hosts_names_ports_and_hashes() {
        let t = "# comment\n\
                 gpu2.lan,10.0.0.7 ssh-ed25519 AAAA\n\
                 [box.example]:2222 ssh-ed25519 AAAA\n\
                 |1|abc=|def= ssh-ed25519 AAAA\n\
                 10.0.0.9 ssh-rsa AAAA\n\
                 @cert-authority *.corp ssh-rsa AAAA\n\
                 gpu2.lan ecdsa-sha2 AAAA\n";
        let v = parse_known_hosts(t);
        assert_eq!(
            v,
            vec![
                ("gpu2.lan".to_string(), None),
                ("box.example".to_string(), Some(2222)),
                ("10.0.0.9".to_string(), None),
            ]
        );
        assert_eq!(alias_for("GPU-Box.lan"), "gpu-box");
        assert_eq!(alias_for("10.0.0.9"), "10.0.0.9");
        assert_eq!(alias_for("My Mac"), "my-mac");
    }
}
