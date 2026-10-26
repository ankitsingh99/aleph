//! Bringing a machine into the fleet (`aleph host include/set/add`, `aleph hub`).
//!
//! One pass that checks everything first (key login, the remote herdr, the
//! agents installed there, whether herdr has the machine saved), prints it as
//! an aligned checklist, then asks a single question that says exactly what
//! will happen, so herdr's own install prompt is never a surprise.

use crate::config::Config;
use crate::out::{err, hint, ok, say, yes, Spinner};
use crate::ui::{self, c};
use crate::{agents, doctor, herdr, proc, sshcfg};
use ratatui::prelude::*;
use std::time::Duration;

/// What we learned about a machine before touching anything.
#[derive(Debug, Default, Clone)]
pub struct Facts {
    pub os: String,
    pub remote_herdr: Option<String>,
    pub remote_path: String,
    /// registry agents on the machine: (display name, version)
    pub agents: Vec<(String, String)>,
    pub local_herdr: Option<String>,
    pub saved: bool,
}

/// What `herdr machine add` will have to do on the remote side.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Ready,
    Save,
    Install(String),
    Upgrade { from: String, to: String },
}

pub fn plan(f: &Facts) -> Plan {
    if f.saved {
        return Plan::Ready;
    }
    match (&f.remote_herdr, &f.local_herdr) {
        (None, Some(l)) => Plan::Install(l.clone()),
        (Some(r), Some(l)) if r != l => Plan::Upgrade {
            from: r.clone(),
            to: l.clone(),
        },
        _ => Plan::Save,
    }
}

fn row(label: &str, mark: Span<'static>, text: String, note: &str) {
    let mut s = vec![
        Span::raw("  "),
        Span::styled(ui::pad(label, 8), ui::muted()),
        mark,
        Span::raw(" "),
        Span::raw(text),
    ];
    if !note.is_empty() {
        s.push(Span::styled(format!("  {note}"), ui::faint()));
    }
    say(Line::from(s));
}
fn good() -> Span<'static> {
    Span::styled("✓", ui::fg(c::GREEN()))
}
fn todo() -> Span<'static> {
    Span::styled("○", ui::fg(c::YELLOW()))
}
fn bad() -> Span<'static> {
    Span::styled("✗", ui::fg(c::RED()))
}

/// `Claude Code 2.1.200 · Codex 0.46.0` (a version only where one was reported).
pub fn agent_list(v: &[(String, String)]) -> String {
    v.iter()
        .map(|(n, ver)| {
            if ver.is_empty() {
                n.clone()
            } else {
                format!("{n} {ver}")
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `/home/u/.local/bin/herdr` → `~/.local/bin/herdr` (the remote $HOME isn't known here).
pub fn tilde_remote(p: &str) -> String {
    for root in ["/home/", "/Users/"] {
        if let Some(rest) = p.strip_prefix(root) {
            if let Some((_, tail)) = rest.split_once('/') {
                return format!("~/{tail}");
            }
        }
    }
    if let Some(tail) = p.strip_prefix("/root/") {
        return format!("~/{tail}");
    }
    p.to_string()
}

fn rule(text: &str) {
    let w = 60usize.saturating_sub(text.chars().count() + 4);
    say(Line::from(vec![
        Span::styled("── ", ui::faint()),
        Span::styled(text.to_string(), ui::muted()),
        Span::styled(format!(" {}", "─".repeat(w)), ui::faint()),
    ]));
}

/// How bringing a machine in went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Onboard {
    /// herdr is there and the machine is saved in herdr
    Ready,
    /// it answers, but herdr isn't set up there (declined, or it failed)
    NotReady,
    /// no key login (or it doesn't answer): nothing else can be done
    NoLogin,
}

/// Check, explain, ask once, then hand over to `herdr machine add`.
/// Returns true when the machine ends up usable.
pub fn run(cfg: &mut Config, alias: &str, machine: &str) -> bool {
    if let Some(e) = crate::showcase::refused() {
        crate::out::err(&e);
        return false;
    }
    run_with(cfg, alias, machine, false) == Onboard::Ready
}

/// [`run`]; `assume_yes` answers its question (`aleph host setup --yes`).
pub fn run_with(cfg: &mut Config, alias: &str, machine: &str, assume_yes: bool) -> Onboard {
    if sshcfg::is_local(alias) {
        return if run_local(cfg) {
            Onboard::Ready
        } else {
            Onboard::NotReady
        };
    }
    let sp = Spinner::start(format!("Checking key login to {alias}"));
    let mut probe = sshcfg::probe(alias);
    sp.stop();
    if !probe.ok() {
        row("ssh", bad(), probe.explain(), "");
        if probe.fail == Some(sshcfg::Fail::Auth)
            && yes("Set up key login now? (one password prompt)")
        {
            if let Err(e) = sshcfg::setup_key(alias) {
                err(&e);
            }
            probe = sshcfg::probe(alias);
        }
        if !probe.ok() {
            hint(&match probe.fail {
                Some(sshcfg::Fail::Auth) => {
                    format!("aleph host key {alias} sets up key login, then run this again")
                }
                _ => format!("aleph doctor --host {alias} explains what's wrong"),
            });
            return Onboard::NoLogin;
        }
    }
    row("ssh", good(), "key login works".into(), "");

    let herdr_local = proc::which(&herdr::bin()).is_some();
    let sp = Spinner::start(format!("Looking at {alias}"));
    let (info, _) = doctor::remote(alias);
    let local = herdr_local
        .then(|| proc::cap(&herdr::bin(), &["--version"], Duration::from_secs(10)))
        .filter(|o| o.ok())
        .map(|o| doctor::version(&o.text()));
    sp.stop();
    if let Some(e) = &info.error {
        row("remote", bad(), e.clone(), "");
        return Onboard::NoLogin;
    }
    let f = Facts {
        os: info.os.clone(),
        remote_herdr: info.herdr.as_ref().map(|h| h.1.clone()),
        remote_path: info.herdr.as_ref().map(|h| h.0.clone()).unwrap_or_default(),
        agents: info
            .installed()
            .iter()
            .map(|(a, f)| (a.name.to_string(), f.version.clone()))
            .collect(),
        local_herdr: local,
        saved: herdr_local && herdr::has_machine(machine),
    };
    if !f.os.is_empty() {
        row("system", good(), f.os.clone(), "");
    }
    let def = agents::default_agent_for(cfg, alias);
    if f.agents.is_empty() {
        row(
            "agents",
            todo(),
            "none of herdr's agents is on the PATH".into(),
            &format!("{} (the default): {}", def.name, def.install_hint()),
        );
    } else {
        let note = if f.agents.iter().any(|(n, _)| n == def.name) {
            String::new()
        } else {
            format!(
                "the default, {}, isn't there — aleph settings set agent <id>",
                def.name
            )
        };
        row("agents", good(), agent_list(&f.agents), &note);
    }
    let p = plan(&f);
    match (&f.remote_herdr, &p) {
        (Some(v), Plan::Upgrade { to, .. }) => row(
            "herdr",
            todo(),
            format!("{v} · this Mac has {to}"),
            "versions must match",
        ),
        (Some(v), _) => row("herdr", good(), v.clone(), &tilde_remote(&f.remote_path)),
        (None, _) => row("herdr", todo(), "not installed".into(), ""),
    }
    if !herdr_local {
        row(
            "saved",
            bad(),
            "herdr isn't installed on this Mac".into(),
            "",
        );
        hint("install herdr locally (https://herdr.dev), then run this again");
        return Onboard::NotReady;
    }
    if p == Plan::Ready {
        row("saved", good(), format!("herdr machine '{machine}'"), "");
        hint(&format!(
            "aleph --host {alias} new starts a session there · claude opens the dashboard"
        ));
        return Onboard::Ready;
    }
    row("saved", todo(), "not a herdr machine yet".into(), "");
    say(Line::raw(""));

    let q = match &p {
        Plan::Install(v) => format!("Install herdr {v} on {alias} and save it in herdr?"),
        Plan::Upgrade { from, to } => {
            format!("Update herdr on {alias} ({from} → {to}) and save it in herdr?")
        }
        _ => format!("Save {alias} in herdr?"),
    };
    if !assume_yes && !yes(&q) {
        hint(&format!("herdr machine add {alias} does it later"));
        return Onboard::NotReady;
    }
    if matches!(p, Plan::Install(_) | Plan::Upgrade { .. }) {
        say(Line::from(Span::styled(
            "  herdr will confirm the download once more — answer Y. It goes to ~/.local/bin on the remote.",
            ui::faint(),
        )));
    }
    rule(&format!("herdr machine add {alias}"));
    let r = proc::interactive(&herdr::bin(), &["machine", "add", alias]);
    rule("back in aleph");
    match r {
        Ok(true) => {
            let m = herdr::machine_for(alias);
            if cfg.get("ssh_host").as_deref() == Some(alias) {
                let _ = cfg.set("machine", &m);
            } else if m != alias {
                let _ = cfg.set(&format!("host.{alias}.machine"), &m);
            }
            crate::chan::forget(alias);
            ok(&format!("{alias} is ready — saved in herdr as '{m}'"));
            Onboard::Ready
        }
        Ok(false) => {
            err(&format!("herdr machine add {alias} didn't finish"));
            hint(&format!("run it again with: herdr machine add {alias}"));
            Onboard::NotReady
        }
        Err(e) => {
            err(&format!("couldn't run herdr: {e}"));
            Onboard::NotReady
        }
    }
}

/// `local` is this computer's own herdr: nothing to log in to or save.
fn run_local(cfg: &mut Config) -> bool {
    let Some(p) = proc::which(&herdr::bin()) else {
        row("herdr", bad(), "not installed on this computer".into(), "");
        hint("install it from https://herdr.dev");
        return false;
    };
    let v = proc::cap(&herdr::bin(), &["--version"], Duration::from_secs(10));
    row(
        "herdr",
        good(),
        doctor::version(&v.text()),
        &doctor::tilde(&p),
    );
    let o = proc::cap(
        &herdr::bin(),
        &["status", "server"],
        Duration::from_secs(10),
    );
    if o.text().contains("not running") {
        row(
            "server",
            todo(),
            "not running".into(),
            "starts with the first session",
        );
    } else {
        row("server", good(), "running".into(), "");
    }
    if !cfg.flag("local") {
        let _ = cfg.set("local", "true");
    }
    hint("aleph local <agent> opens one here (claude, codex, opencode, … or shell) · aleph agents --local lists what's installed");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(remote: Option<&str>, local: Option<&str>, saved: bool) -> Facts {
        Facts {
            remote_herdr: remote.map(str::to_string),
            local_herdr: local.map(str::to_string),
            saved,
            ..Default::default()
        }
    }

    #[test]
    fn plans() {
        assert_eq!(plan(&f(Some("0.9.5"), Some("0.9.5"), true)), Plan::Ready);
        assert_eq!(plan(&f(Some("0.9.5"), Some("0.9.5"), false)), Plan::Save);
        assert_eq!(
            plan(&f(None, Some("0.9.5"), false)),
            Plan::Install("0.9.5".into())
        );
        assert_eq!(
            plan(&f(Some("0.7.5"), Some("0.9.5"), false)),
            Plan::Upgrade {
                from: "0.7.5".into(),
                to: "0.9.5".into()
            }
        );
        assert_eq!(plan(&f(Some("0.7.5"), None, false)), Plan::Save);
    }

    #[test]
    fn agents_row() {
        let v = vec![
            ("Claude Code".to_string(), "2.1.200".to_string()),
            ("Codex".to_string(), String::new()),
        ];
        assert_eq!(agent_list(&v), "Claude Code 2.1.200 · Codex");
        assert_eq!(agent_list(&[]), "");
    }

    #[test]
    fn remote_tilde() {
        assert_eq!(
            tilde_remote("/home/selene/.local/bin/herdr"),
            "~/.local/bin/herdr"
        );
        assert_eq!(tilde_remote("/root/bin/herdr"), "~/bin/herdr");
        assert_eq!(tilde_remote("/usr/bin/herdr"), "/usr/bin/herdr");
    }
}
