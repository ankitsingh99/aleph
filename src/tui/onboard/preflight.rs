//! Step 1, Preflight: what this computer has — the system and the network,
//! the terminal (colour, font, pictures), ssh (OpenSSH, the agent and keys,
//! the config), the tools Aleph can use, herdr itself and Raycast. Every
//! check is local and read-only (a `--version` at most). What can be
//! installed (herdr, a Nerd Font, an ssh key, the tools) is marked into the
//! queue and installed when you apply, on the install screen; ⚡ quick start
//! queues every default and applies at once.

use super::rows::{Row, St};
use super::{Action, Ev, Hand, Msg, Run, Step, Wiz};
use crate::proc;
use std::time::Duration;

const GROUPS: &[&str] = &["This Mac", "Terminal", "ssh", "Tools", "Apps and herdr"];

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fix {
    /// a command run in the terminal (after a confirm)
    Run {
        title: String,
        prog: String,
        args: Vec<String>,
        note: String,
    },
    /// a command for you to run (copied)
    Copy(String),
    /// an install: marked, it goes in the queue and runs when you apply
    Queue(Action),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Check {
    pub group: &'static str,
    pub id: String,
    pub label: String,
    pub st: St,
    pub text: String,
    pub fix: Option<Fix>,
    pub why: String,
    /// keys to offer to load into ssh-agent, inline (paths, ~/…)
    pub load: Vec<String>,
}

impl Check {
    fn new(group: &'static str, id: &str, label: &str, st: St, text: impl Into<String>) -> Check {
        Check {
            group,
            id: id.to_string(),
            label: label.to_string(),
            st,
            text: text.into(),
            fix: None,
            why: String::new(),
            load: Vec::new(),
        }
    }
    fn why(mut self, w: &str) -> Check {
        self.why = w.to_string();
        self
    }
    fn fix(mut self, f: Fix) -> Check {
        self.fix = Some(f);
        self
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub checks: Vec<Check>,
    pub started: bool,
}

pub(crate) fn enter(w: &mut Wiz) {
    if !w.pre.started {
        start(w);
    }
}

pub(crate) fn retry(w: &mut Wiz) {
    w.pre.checks.clear();
    start(w);
}

fn start(w: &mut Wiz) {
    w.pre.started = true;
    w.spawn("pre:mac", "Looking at this Mac", || Msg::Pre(mac()));
    w.spawn("pre:term", "Looking at the terminal", || {
        Msg::Pre(terminal())
    });
    w.spawn("pre:ssh", "Looking at ssh", || Msg::Pre(ssh()));
    w.spawn("pre:tools", "Looking for tools", || Msg::Pre(tools()));
    w.spawn("pre:apps", "Looking for herdr and Raycast", || {
        Msg::Pre(apps())
    });
}

pub(crate) fn msg(w: &mut Wiz, v: Vec<Check>) {
    for c in v {
        match w.pre.checks.iter_mut().find(|x| x.id == c.id) {
            Some(x) => *x = c,
            None => w.pre.checks.push(c),
        }
    }
    w.pre
        .checks
        .sort_by_key(|c| GROUPS.iter().position(|g| *g == c.group).unwrap_or(9));
}

/// The network, live from the probe the wizard runs.
fn network(w: &Wiz) -> Row {
    use crate::net::State as N;
    let (st, text) = match w.net.state() {
        N::Online { ms } => (
            St::Ok,
            format!("online · {ms} ms to {}", crate::net::target(&w.cfg)),
        ),
        N::Weak { ms, loss } => (
            St::Warn,
            format!("a weak link · {ms} ms, {loss}% lost — remote steps may be slow"),
        ),
        N::Offline { route: true, .. } => (
            St::Warn,
            "the internet is down (the local network is up) — downloads and tests wait".into(),
        ),
        N::Offline { .. } => (
            St::Warn,
            "offline — steps that need the network say so and wait".into(),
        ),
        N::Reconnecting { .. } => (St::Busy, "reconnecting".into()),
        N::Unknown => (St::Busy, "checking".into()),
    };
    Row::check(st, "net", "network", text).why(
        "Aleph probes one address (net.probe, 1.1.1.1:443 by default) every few seconds, without DNS. Offline, the steps that need the internet (tests, downloads) are skipped with a note; machines on your own network still answer.",
    )
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let mut v = vec![
        Row::button("quick", "⚡ quick start — every default: install what's missing here, then open Aleph")
            .why("No questions: Aleph looks at this computer and installs what it's missing — herdr, your default agent (Claude Code) and its herdr integration, terminal-notifier, a Nerd Font, the daemon at login, tab completion — on the install screen, then opens the dashboard. Settings stay at their defaults; add other machines later with aleph setup machines. Same as aleph setup --defaults."),
        Row::blank(),
    ];
    let queued = w
        .pre
        .checks
        .iter()
        .filter(|c| matches!(&c.fix, Some(Fix::Queue(a)) if w.plan.has(a)))
        .count();
    if queued > 0 {
        v.push(Row::info(format!(
            "{queued} install{} queued — {} when you apply (step 12)",
            if queued == 1 { "" } else { "s" },
            if queued == 1 { "it runs" } else { "they run" }
        )));
    }
    for g in GROUPS {
        v.push(Row::head(*g));
        if *g == "This Mac" {
            v.push(network(w));
        }
        let cs: Vec<&Check> = w.pre.checks.iter().filter(|c| c.group == *g).collect();
        if cs.is_empty() {
            let st = if w.pre.started { St::Busy } else { St::Todo };
            v.push(Row::check(
                st,
                format!("pending:{g}"),
                "…",
                "looking".into(),
            ));
            continue;
        }
        for c in cs {
            let mut r =
                Row::check(c.st, c.id.clone(), c.label.clone(), c.text.clone()).why(c.why.clone());
            match &c.fix {
                Some(Fix::Run { title, .. }) => r = r.fix(title.clone()),
                Some(Fix::Copy(_)) => r = r.fix("copy the command"),
                Some(Fix::Queue(a)) if w.plan.has(a) => {
                    r = Row::check(
                        St::Todo,
                        c.id.clone(),
                        c.label.clone(),
                        format!("{} · queued: {}", c.text, a.label()),
                    )
                    .why(c.why.clone())
                    .fix("unqueue it");
                }
                Some(Fix::Queue(a)) => r = r.fix(format!("queue: {}", a.label())),
                None => {}
            }
            v.push(r);
            for k in &c.load {
                let name = k.rsplit('/').next().unwrap_or(k);
                v.push(
                    Row::button(format!("ssh-add:{k}"), format!("load {name}  (ssh-add asks for its passphrase)"))
                        .why("ssh-add puts the key in ssh-agent for this login session; it asks for the passphrase itself and Aleph never sees it. Enter runs it in the terminal and comes back here."),
                );
            }
        }
        if *g == "ssh" && w.pre.checks.iter().any(|c| c.id == "keys") {
            v.extend(key_rows(w));
        }
    }
    v
}

/// Which key Aleph uses (`ssh_key`): one of yours, ssh's own choice, or a
/// new one made when you apply.
fn key_rows(w: &Wiz) -> Vec<Row> {
    let keys = crate::sshcfg::keys();
    let new = w.plan.has(&Action::SshKey);
    let mut opts: Vec<(String, String)> = vec![(
        String::new(),
        "ssh decides (your ~/.ssh/config, then id_ed25519 …)".into(),
    )];
    for k in &keys {
        opts.push((crate::doctor::tilde(&k.path), k.label()));
    }
    let cur = w.value("ssh_key");
    if !cur.is_empty() && !opts.iter().any(|(v, _)| *v == cur) {
        // the new one, or one set by hand
        let name = std::path::Path::new(&cur)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cur.clone());
        opts.push((
            cur.clone(),
            if new { format!("{name} · new") } else { name },
        ));
    }
    let mut v = Vec::new();
    if !keys.is_empty() || new {
        v.push(
            Row::choice("ssh_key", "key Aleph uses", opts, cur, w.pending("ssh_key"))
                .why("The key step 3 copies to each machine (ssh-copy-id) and that machines Aleph adds are given (IdentityFile in ~/.ssh/config). ssh decides = whatever your config and ssh's defaults offer."),
        );
    }
    v.push(
        Row::button(
            "ssh-newkey",
            if new {
                format!(
                    "a new key is queued: {} (enter: take it back)",
                    crate::doctor::tilde(&crate::sshcfg::new_key_path())
                )
            } else {
                "make a new key".to_string()
            },
        )
        .why("An ed25519 key made when you apply (ssh-keygen asks for its passphrase; enter twice for none). It goes to ~/.ssh/id_ed25519 when that's free, else ~/.ssh/aleph_ed25519 — a key you have is never touched — and becomes the key Aleph uses."),
    );
    v
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Button(id) if id == "quick" => {
            super::quick::start(w);
            None
        }
        Ev::Button(id) if id.starts_with("ssh-add:") => {
            let k = id.trim_start_matches("ssh-add:").to_string();
            let path = match k.strip_prefix("~/") {
                Some(rest) => crate::config::home().join(rest),
                None => std::path::PathBuf::from(&k),
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // the button is the yes: straight to ssh-add's own prompt
            Some(Hand {
                title: format!("Load {name} into ssh-agent"),
                note: "ssh-add asks for the key's passphrase itself; Aleph never sees it.".into(),
                run: Run::Cmd {
                    prog: "ssh-add".into(),
                    args: vec![path.to_string_lossy().into_owned()],
                    cwd: None,
                },
                step: Step::Preflight,
                id: id.clone(),
            })
        }
        Ev::Button(id) if id == "ssh-newkey" => {
            let on = !w.plan.has(&Action::SshKey);
            w.plan.want(Step::Preflight, Action::SshKey, on);
            if on {
                let p = crate::doctor::tilde(&crate::sshcfg::new_key_path());
                w.set("ssh_key", &p);
                w.toast(
                    false,
                    format!("queued: a new key, {p} — made when you apply"),
                );
            } else {
                w.plan.forget("ssh_key");
                w.toast(false, "the new key is out of the queue");
            }
            None
        }
        Ev::Fix(id) => {
            let c = w.pre.checks.iter().find(|c| c.id == id)?.clone();
            match c.fix? {
                Fix::Queue(a) => {
                    let on = !w.plan.has(&a);
                    w.toast(
                        false,
                        if on {
                            format!("queued: {} — it runs when you apply", a.label())
                        } else {
                            format!("out of the queue: {}", a.label())
                        },
                    );
                    w.plan.want(Step::Preflight, a, on);
                }
                Fix::Copy(cmd) => {
                    let ok = crate::tui::copy(&cmd);
                    w.toast(
                        !ok,
                        if ok {
                            format!("copied: {cmd}")
                        } else {
                            format!("run it yourself: {cmd}")
                        },
                    );
                }
                Fix::Run {
                    title,
                    prog,
                    args,
                    note,
                } => {
                    let h = Hand {
                        title,
                        note,
                        run: Run::Cmd {
                            prog,
                            args,
                            cwd: None,
                        },
                        step: Step::Preflight,
                        id: c.id.clone(),
                    };
                    w.confirm_hand(h, vec![]);
                }
            }
            None
        }
        Ev::After(..) => {
            retry(w);
            None
        }
        _ => None,
    }
}

// ------------------------------------------------------------------ the checks

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn cap(prog: &str, args: &[&str]) -> proc::Out {
    proc::cap(prog, args, Duration::from_secs(5))
}

fn mac() -> Vec<Check> {
    let g = "This Mac";
    let os = first_line(&cap("uname", &["-sm"]).text());
    let name = if os.starts_with("Darwin") {
        let n = first_line(&cap("sw_vers", &["-productName"]).text());
        let v = first_line(&cap("sw_vers", &["-productVersion"]).text());
        format!("{n} {v}").trim().to_string()
    } else {
        std::fs::read_to_string("/etc/os-release")
            .unwrap_or_default()
            .lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default()
    };
    let arch = os.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut text = match (name.is_empty(), arch.is_empty()) {
        (false, false) => format!("{name} · {arch}"),
        (false, true) => name,
        _ => os.clone(),
    };
    for s in specs() {
        text.push_str(" · ");
        text.push_str(&s);
    }
    vec![Check::new(g, "os", "system", if text.is_empty() { St::Warn } else { St::Ok }, if text.is_empty() { "couldn't tell".to_string() } else { text })
        .why("The system and processor of this computer. Aleph builds for Linux machines from here when you deploy the daemon to one (step 9).")]
}

/// The processor, its cores, memory and free disk, briefly (`Apple M3 ·
/// 8 cores · 16 GB · 210 GB free`).
fn specs() -> Vec<String> {
    let mut v = Vec::new();
    let gb = |b: u64| format!("{} GB", (b as f64 / 1e9).round() as u64);
    if cfg!(target_os = "macos") {
        let cpu = first_line(&cap("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"]).text());
        if !cpu.is_empty() {
            v.push(cpu);
        }
        if let Ok(n) =
            first_line(&cap("/usr/sbin/sysctl", &["-n", "hw.ncpu"]).text()).parse::<u32>()
        {
            v.push(format!("{n} cores"));
        }
        if let Ok(b) =
            first_line(&cap("/usr/sbin/sysctl", &["-n", "hw.memsize"]).text()).parse::<u64>()
        {
            v.push(gb(b));
        }
    } else {
        let info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        if let Some(m) = info.lines().find_map(|l| {
            l.strip_prefix("model name")
                .and_then(|r| r.split_once(':'))
                .map(|(_, m)| m.trim().to_string())
        }) {
            v.push(m);
        }
        let n = info.lines().filter(|l| l.starts_with("processor")).count();
        if n > 0 {
            v.push(format!("{n} cores"));
        }
        let mem = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        if let Some(kb) = mem
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))
            .and_then(|r| r.split_whitespace().next()?.parse::<u64>().ok())
        {
            v.push(gb(kb * 1024));
        }
    }
    // free space where your home is
    let o = cap("df", &["-Pk", &crate::config::home().to_string_lossy()]);
    if let Some(kb) = o
        .text()
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(3)?.parse::<u64>().ok())
    {
        v.push(format!("{} free", gb(kb * 1024)));
    }
    v
}

/// Does the terminal run on another computer (ssh, an OrbStack or
/// container shell)? Then a font has to be installed there, not here.
pub(crate) fn terminal_elsewhere() -> bool {
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return true;
    }
    !cfg!(target_os = "macos")
        && (std::path::Path::new("/mnt/mac").exists()
            || std::fs::read_to_string("/proc/version")
                .is_ok_and(|v| v.to_lowercase().contains("orbstack"))
            || std::path::Path::new("/.dockerenv").exists())
}

fn terminal() -> Vec<Check> {
    let g = "Terminal";
    let mut v = Vec::new();
    let tp = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let term = std::env::var("TERM").unwrap_or_default();
    let name = match tp.as_str() {
        "" => term.clone(),
        "WarpTerminal" => "Warp".into(),
        "Apple_Terminal" => "Terminal".into(),
        "iTerm.app" => "iTerm2".into(),
        "ghostty" => "Ghostty".into(),
        "WezTerm" => "WezTerm".into(),
        o => o.to_string(),
    };
    v.push(Check::new(
        g,
        "term",
        "terminal",
        St::Info,
        if name.is_empty() {
            "unknown".to_string()
        } else {
            name
        },
    ));
    let tc = crate::logo::truecolor();
    v.push(if tc {
        Check::new(g, "truecolor", "colour", St::Ok, "24-bit colour")
    } else {
        Check::new(g, "truecolor", "colour", St::Warn, "256 colours — the palette and logos are approximated")
            .fix(Fix::Copy("export COLORTERM=truecolor".into()))
            .why("Aleph's palette is 24-bit. Most terminals support it but some don't say so: when yours does, COLORTERM=truecolor in your shell's rc file tells programs.")
    }
    .why("Aleph's palette (Graphite + gold) and the logos are 24-bit colours; with 256 colours each is the nearest of the 256."));
    let (nerd, font) = crate::icons::terminal_font();
    let installed = crate::icons::nerd_font_installed();
    let brew = proc::which("brew").is_some();
    let cask = "font-jetbrains-mono-nerd-font";
    let elsewhere = terminal_elsewhere();
    let font_fix = || {
        if elsewhere {
            // the font belongs where the terminal runs
            Fix::Copy(format!("brew install --cask {cask}"))
        } else if brew {
            Fix::Queue(Action::nerd_font())
        } else {
            Fix::Queue(Action::FontDownload)
        }
    };
    v.push(match nerd {
        Some(true) => Check::new(g, "font", "font", St::Ok, format!("{} · Nerd Font logos", font.unwrap_or_else(|| "a Nerd Font".into()))),
        Some(false) if installed => Check::new(
            g,
            "font",
            "font",
            St::Warn,
            format!("{} — a Nerd Font is installed; pick it as the terminal's font", font.unwrap_or_else(|| "this font".into())),
        ),
        Some(false) if elsewhere => Check::new(g, "font", "font", St::Warn, format!("{} has no Nerd Font logos — your terminal runs on another computer: the font goes there", font.unwrap_or_else(|| "this font".into()))).fix(font_fix()),
        Some(false) => Check::new(g, "font", "font", St::Warn, format!("{} has no Nerd Font logos", font.unwrap_or_else(|| "this font".into()))).fix(font_fix()),
        None if installed => Check::new(g, "font", "font", St::Info, "can't tell the terminal's font · a Nerd Font is installed"),
        None => Check::new(g, "font", "font", St::Info, "can't tell the terminal's font").fix(font_fix()),
    }
    .why("Nerd Fonts carry the logos of systems and GPUs as glyphs. Without one, Aleph shows emoji or names (icons = auto picks for you)."));
    v.push(match crate::logo::images::support() {
        Ok((p, t)) => Check::new(g, "images", "pictures", St::Ok, format!("{t} shows images: logos as pictures ({})", p.label())),
        Err(e) => Check::new(g, "images", "pictures", St::Info, format!("{e} — logos as block thumbnails")),
    }
    .why("kitty and Ghostty draw real pictures (the kitty graphics protocol), iTerm2 and WezTerm too (iTerm2's inline images); elsewhere logos are drawn in block characters. ui.logos chooses."));
    let (cols, rows) = crossterm::terminal::size().unwrap_or((0, 0));
    v.push(if cols >= 100 && rows >= 30 {
        Check::new(g, "size", "size", St::Ok, format!("{cols}×{rows}"))
    } else {
        Check::new(g, "size", "size", St::Info, format!("{cols}×{rows} — the dashboard likes 100×30 or more"))
    }
    .why("Every view works from 80×24; wider windows show more side by side (details, previews, the machine hero)."));
    v
}

fn ssh() -> Vec<Check> {
    let g = "ssh";
    let mut v = Vec::new();
    let o = cap(&crate::sshcfg::ssh_bin(), &["-V"]);
    let ver = first_line(&format!("{}{}", o.stderr, o.text()));
    v.push(if o.code.is_some() && !ver.is_empty() {
        Check::new(g, "openssh", "OpenSSH", St::Ok, ver.split(',').next().unwrap_or(&ver).to_string())
    } else {
        Check::new(g, "openssh", "OpenSSH", St::Bad, "ssh isn't on the PATH")
    }
    .why("Every machine is reached with your own ssh: its config, its keys, its agent. Nothing else is installed for it."));
    // every private key in ~/.ssh, whatever it's called; which of them have
    // a passphrase (only those need the agent)
    let all = crate::sshcfg::keys();
    let keys: Vec<String> = all.iter().map(|k| k.label()).collect();
    let locked: Vec<&crate::sshcfg::Key> = all
        .iter()
        .filter(|k| crate::sshcfg::needs_passphrase(&k.path) == Some(true))
        .collect();
    let names = |ks: &[&crate::sshcfg::Key]| {
        ks.iter()
            .map(|k| {
                k.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut agent = match crate::sshcfg::agent() {
        crate::sshcfg::Agent::Keys(n) => Check::new(g, "agent", "ssh-agent", St::Ok, format!("{n} key{} loaded", if n == 1 { "" } else { "s" })),
        crate::sshcfg::Agent::Empty if !locked.is_empty() => Check::new(g, "agent", "ssh-agent", St::Warn, format!("running, empty — {} {} a passphrase: load {} below", names(&locked), if locked.len() == 1 { "has" } else { "have" }, if locked.len() == 1 { "it" } else { "them" })),
        crate::sshcfg::Agent::Empty if !all.is_empty() => Check::new(g, "agent", "ssh-agent", St::Ok, "running, empty — none of your keys has a passphrase, so none needs it"),
        crate::sshcfg::Agent::Empty => Check::new(g, "agent", "ssh-agent", St::Info, "running, no keys loaded"),
        crate::sshcfg::Agent::None if !locked.is_empty() => Check::new(g, "agent", "ssh-agent", St::Warn, format!("not running — {} would ask for {} passphrase on every connection", names(&locked), if locked.len() == 1 { "its" } else { "their" })),
        crate::sshcfg::Agent::None => Check::new(g, "agent", "ssh-agent", St::Info, "not running — none of your keys has a passphrase, so none needs it"),
    }
    .why("Aleph never types a password: every connection is BatchMode. A key with a passphrase has to be loaded into ssh-agent (ssh-add asks for the passphrase once), or connections that use it fail. A key without one works as it is.");
    if matches!(crate::sshcfg::agent(), crate::sshcfg::Agent::Empty) {
        agent.load = locked
            .iter()
            .map(|k| crate::doctor::tilde(&k.path))
            .collect();
    }
    v.push(agent);
    v.push(if keys.is_empty() {
        Check::new(g, "keys", "keys", St::Warn, "no key in ~/.ssh yet — a new one (no passphrase) when you apply").fix(Fix::Queue(Action::SshKey))
    } else {
        Check::new(g, "keys", "keys", St::Ok, format!("{} in ~/.ssh: {}", keys.len(), keys.join("  ·  ")))
    }
    .why("Key login is what lets Aleph reach a machine without a prompt. Pick the key below (or make a new one); step 3 copies it to each machine that doesn't take it yet."));
    let cfgp = crate::sshcfg::user_config();
    let n = crate::sshcfg::aliases().len();
    v.push(if cfgp.exists() {
        Check::new(g, "config", "config", St::Ok, format!("{n} host{} in {}", if n == 1 { "" } else { "s" }, crate::doctor::tilde(&cfgp)))
    } else {
        Check::new(g, "config", "config", St::Info, "no ~/.ssh/config yet — step 3 adds hosts to it")
    }
    .why("Machines are your ssh config's Host aliases. Step 3 lists them, and can add one (a backup of the file is kept)."));
    v
}

/// (name, args for its version, why it's useful, fix when it's missing)
const TOOLS: &[(&str, &[&str], &str)] = &[
    (
        "git",
        &["--version"],
        "worktrees for parallel sessions; presets that start in a repo",
    ),
    (
        "curl",
        &["--version"],
        "notifications (ntfy, Slack, webhooks) and updates",
    ),
    (
        "mosh",
        &["--version"],
        "optional: attach over mosh (attach_via = mosh) — survives sleep and roaming",
    ),
    (
        "tailscale",
        &["version"],
        "optional: reach machines and the daemon from anywhere; step 3 lists its peers",
    ),
    (
        "cloudflared",
        &["--version"],
        "optional: expose the daemon's API through a Cloudflare tunnel",
    ),
    (
        "terminal-notifier",
        &[],
        "optional: desktop notifications with Aleph's icon and a click that opens the session",
    ),
    (
        "zig",
        &["version"],
        "optional: build Aleph for Linux machines from this Mac (with cargo-zigbuild)",
    ),
    (
        "cargo-zigbuild",
        &["--version"],
        "optional: cross builds for deploying the daemon to a Linux machine",
    ),
];

fn tools() -> Vec<Check> {
    let g = "Tools";
    let brew = proc::which("brew").is_some();
    // terminal-notifier, and the cross-build tools for deploying from a Mac
    let here: Vec<&(&str, &[&str], &str)> = TOOLS
        .iter()
        .filter(|(n, _, _)| {
            cfg!(target_os = "macos")
                || !matches!(*n, "terminal-notifier" | "zig" | "cargo-zigbuild")
        })
        .collect();
    let handles: Vec<_> = here
        .iter()
        .map(|(name, args, _)| {
            let (name, args) = (name.to_string(), args.to_vec());
            std::thread::spawn(move || {
                let p = proc::which(&name)?;
                let ver = if args.is_empty() {
                    String::new()
                } else {
                    let o = proc::cap(&p.to_string_lossy(), &args, Duration::from_secs(5));
                    let t = format!("{}\n{}", o.text(), o.stderr);
                    t.lines()
                        .find_map(crate::probe::find_version)
                        .unwrap_or_default()
                };
                Some((p, ver))
            })
        })
        .collect();
    let mut v = Vec::new();
    for ((name, _, why), h) in here.iter().map(|t| **t).zip(handles) {
        let found = h.join().ok().flatten();
        let optional = why.starts_with("optional");
        let c = match found {
            Some((p, ver)) => Check::new(
                g,
                name,
                name,
                St::Ok,
                format!(
                    "{}{}",
                    if ver.is_empty() {
                        String::new()
                    } else {
                        format!("{ver} · ")
                    },
                    crate::doctor::tilde(&p)
                ),
            ),
            None => {
                let pkg = match name {
                    "cargo-zigbuild" => "cargo-zigbuild",
                    "tailscale" => "--cask tailscale",
                    "cloudflared" => "cloudflared",
                    n => n,
                };
                let c = Check::new(
                    g,
                    name,
                    name,
                    if optional { St::Info } else { St::Warn },
                    "not found",
                );
                match Action::tool_here(name) {
                    Some(a) => c.fix(Fix::Queue(a)),
                    None if brew => c.fix(Fix::Queue(Action::tool(name))),
                    None => c.fix(Fix::Copy(format!("brew install {pkg}"))),
                }
            }
        };
        v.push(c.why(why));
    }
    v
}

fn apps() -> Vec<Check> {
    let g = "Apps and herdr";
    let mut v = Vec::new();
    let home = crate::config::home();
    let raycast = cfg!(target_os = "macos")
        && [
            "/Applications/Raycast.app".into(),
            home.join("Applications/Raycast.app"),
        ]
        .iter()
        .any(|p: &std::path::PathBuf| p.exists());
    if !cfg!(target_os = "macos") {
        // Raycast is a macOS app: nothing to look for here
    } else {
        v.push(if raycast {
        Check::new(g, "raycast", "Raycast", St::Ok, "installed · the extension is in step 10")
    } else {
        Check::new(g, "raycast", "Raycast", St::Info, "not installed (optional: sessions and approvals from Raycast)")
    }
    .why("Raycast's Aleph extension lists sessions, approves, prompts and shows a menu bar count. Optional."));
    }
    let hb = crate::herdr::bin();
    match proc::which(&hb) {
        None => {
            v.push(
                Check::new(g, "herdr", "herdr", St::Bad, "not installed here — herdr runs the sessions")
                    .fix(Fix::Queue(Action::HerdrLocal))
                    .why("herdr is the terminal multiplexer the agents run in, on every machine. This computer needs it too: it saves the machines and installs herdr on them."),
            );
        }
        Some(p) => {
            let o = cap(&hb, &["--version"]);
            v.push(
                Check::new(g, "herdr", "herdr", St::Ok, format!("{} · {}", crate::doctor::version(&o.text()), crate::doctor::tilde(&p)))
                    .why("herdr is the terminal multiplexer the agents run in. Machines get this same version (step 4)."),
            );
            let s = cap(&hb, &["status", "server"]);
            let t = s.text();
            v.push(if t.contains("not running") || !s.ok() {
                Check::new(g, "server", "herdr server", St::Info, "not running here — it starts with the first local session")
            } else {
                Check::new(g, "server", "herdr server", St::Ok, "running")
            }
            .why("This computer's own herdr server (the `local` machine). Remote machines have their own."));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_merge_by_id_and_keep_group_order() {
        let cfg = crate::config::Config::in_memory(&[]);
        let mut w = Wiz::new(
            cfg,
            Default::default(),
            std::env::temp_dir().join("x-no-state.json"),
            true,
        );
        msg(
            &mut w,
            vec![Check::new("Tools", "git", "git", St::Ok, "2.4")],
        );
        msg(
            &mut w,
            vec![Check::new("This Mac", "os", "system", St::Ok, "macOS")],
        );
        msg(
            &mut w,
            vec![Check::new("Tools", "git", "git", St::Warn, "not found")],
        );
        assert_eq!(w.pre.checks.len(), 2);
        assert_eq!(w.pre.checks[0].id, "os");
        assert_eq!(w.pre.checks[1].st, St::Warn);
        let rs = rows(&w);
        // every group has a heading, and a row while it's not looked at yet
        assert_eq!(
            rs.iter()
                .filter(|r| r.kind == super::super::rows::Kind::Head)
                .count(),
            GROUPS.len()
        );
    }
}
