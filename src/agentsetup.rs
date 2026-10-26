//! Setting up a machine's agents, one machine at a time: choose from every
//! agent Aleph knows (the ones already there ticked, with their versions),
//! install only the chosen ones, one after another with their output live,
//! then **validate** each (its binary is on your login PATH and `--version`
//! runs, through the PATH prelude) and **verify** it (a fresh probe finds
//! it; what herdr there knows about it). Integrations are offered at the
//! end, with consent.
//!
//! An agent that installs with npm on a machine without Node.js (or with
//! one too old) gets Node.js first, in the same run and under the same
//! yes: the current LTS from nodejs.org, sha256-checked, in your home
//! folder (`nodeinstall.rs`; `agents.node = off` says how instead). Each
//! machine's choice is its own — nothing picked on one is installed on
//! another.
//!
//! A failure never stops the rest. It says why in plain words ([`Why`]: no
//! Node.js or npm, the network, permission, sudo, a timeout, …), shows the
//! installer's last lines, keeps the whole log, and asks: retry, skip or
//! show the log. The summary lists what was installed, what failed (why,
//! and the fix) and what was skipped.
//!
//! `aleph host setup`, `aleph agents install` and the setup wizard's
//! hand-off all run [`run`]; the pure parts ([`defaults`], [`classify`],
//! [`why_blocked`], [`parse_check`], [`summary`]) are unit-tested.

use crate::agentinstall::{self, Plan};
use crate::agents::{self, Agent};
use crate::config::{self, Config};
use crate::icons;
use crate::out::*;
use crate::pick::{self, Item};
use crate::probe::{self, Found, HostProbe, IState};
use crate::ui::{self, c};
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

/// How long one agent's installer may run (`agents.install_timeout`, an hour
/// by default): on a slow link a clone plus a large download takes a while,
/// and one that asks a question is stopped sooner by whoever runs it (setup).
pub fn install_limit(cfg: &Config) -> Duration {
    Duration::from_secs(
        cfg.get("agents.install_timeout")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(3600)
            .max(60),
    )
}
/// The validation's `command -v` and `--version`.
const T_CHECK: Duration = Duration::from_secs(40);
/// Installer lines shown with a failure.
const TAIL: usize = 8;

/// `box3`, or `this machine` for `local`.
pub fn on(host: &str) -> String {
    if host == probe::LOCAL {
        "this machine".into()
    } else {
        host.to_string()
    }
}

fn named(a: &Agent, version: &str) -> String {
    if version.is_empty() {
        a.name.to_string()
    } else {
        format!("{} {version}", a.name)
    }
}

fn list(v: &[&str]) -> String {
    match v {
        [] => String::new(),
        [a] => a.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

// ------------------------------------------------------------------ choosing

/// One agent in the checklist.
#[derive(Debug, Clone)]
pub struct Choice {
    pub agent: &'static Agent,
    /// already installed there
    pub found: Option<Found>,
    pub plan: Plan,
    /// it installs with npm, and Node.js (LTS) goes on the machine first
    pub node: bool,
    /// ticked before you choose
    pub on: bool,
}

impl Choice {
    /// What the checklist says beside it.
    pub fn hint(&self) -> String {
        if let Some(f) = &self.found {
            return if f.version.is_empty() {
                "installed".into()
            } else {
                format!("installed {}", f.version)
            };
        }
        if self.node {
            return format!(
                "{} · Node.js LTS first",
                self.plan.command.clone().unwrap_or_default()
            );
        }
        match (why_blocked(&self.plan), &self.plan.command) {
            (Some(w), _) => w.short().to_string(),
            (None, Some(cmd)) => cmd.clone(),
            (None, None) => "no install command".into(),
        }
    }
}

/// It installs with npm and the machine has no (new enough) Node.js or npm.
pub fn needs_node(pl: &Plan) -> bool {
    matches!(why_blocked(pl), Some(Why::NoNode | Why::NoNpm))
}

/// …and Aleph will put Node.js there first (`agents.node` on, a fetcher
/// and an official build for the machine).
pub fn node_first(cfg: &Config, p: &HostProbe, pl: &Plan) -> bool {
    needs_node(pl) && crate::nodeinstall::enabled(cfg) && crate::nodeinstall::possible(p).is_ok()
}

/// What stops it installing here (None: it can — Node.js first if need be).
pub fn blocked(cfg: &Config, p: &HostProbe, pl: &Plan) -> Option<Why> {
    if node_first(cfg, p, pl) {
        return None;
    }
    why_blocked(pl)
}

/// What's ticked before you choose, and the sentence that says why (pure):
/// the agents already there; else the default agent and Claude Code /
/// Codex (two at most), when they can install there.
pub fn defaults(cfg: &Config, p: &HostProbe) -> (Vec<&'static str>, String) {
    let on = on(&p.host);
    let have = p.installed();
    if !have.is_empty() {
        let names: Vec<String> = have.iter().map(|(a, f)| named(a, &f.version)).collect();
        return (
            have.iter().map(|(a, _)| a.id).collect(),
            format!("Already on {on}: {} — ticked.", names.join(" · ")),
        );
    }
    let def = agents::default_agent_for(cfg, &p.host);
    let mut want: Vec<&'static Agent> = vec![def];
    for id in ["claude", "codex"] {
        if want.len() < 2 && !want.iter().any(|a| a.id == id) {
            if let Some(a) = agents::find(id) {
                want.push(a);
            }
        }
    }
    let mut ticked: Vec<&'static Agent> = Vec::new();
    let mut held: Vec<String> = Vec::new();
    let mut node = false;
    for a in want {
        let pl = agentinstall::plan(a, p);
        match blocked(cfg, p, &pl) {
            None => {
                node |= needs_node(&pl);
                ticked.push(a);
            }
            Some(w) => held.push(format!("{} {}", a.name, w.short())),
        }
    }
    let role = |a: &Agent| {
        if a.id == def.id {
            format!("{} (the default agent)", a.name)
        } else {
            a.name.to_string()
        }
    };
    let names: Vec<String> = ticked.iter().map(|a| role(a)).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut s = if ticked.is_empty() {
        format!("Nothing is installed on {on} yet, and nothing is ticked.")
    } else {
        format!("Nothing is installed on {on} yet: ticked {}.", list(&names))
    };
    if node {
        s.push_str(" Node.js (the current LTS) goes on first, for the npm ones.");
    }
    if !held.is_empty() {
        let h: Vec<&str> = held.iter().map(String::as_str).collect();
        s.push_str(&format!(" Not ticked: {} here.", list(&h)));
    }
    (ticked.iter().map(|a| a.id).collect(), s)
}

/// Every agent in the registry, for the checklist: the installed ones
/// first, then those that can install, then the ones that can't yet, then
/// the ones without a command; and the sentence about what's ticked.
pub fn choices(cfg: &Config, p: &HostProbe) -> (Vec<Choice>, String) {
    let (ticked, note) = defaults(cfg, p);
    let mut v: Vec<Choice> = agents::ALL
        .iter()
        .map(|a| {
            let plan = agentinstall::plan(a, p);
            Choice {
                agent: a,
                found: p.agent(a.id).cloned(),
                node: p.agent(a.id).is_none() && node_first(cfg, p, &plan),
                plan,
                on: ticked.contains(&a.id),
            }
        })
        .collect();
    v.sort_by_key(|ch| {
        if ch.found.is_some() {
            0
        } else if ch.plan.ready() || ch.node {
            1
        } else if ch.plan.command.is_some() {
            2
        } else {
            3
        }
    });
    (v, note)
}

// ------------------------------------------------------------------ why it failed

/// Why an install didn't work, in words a person can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// an npm install, and the machine has no (new enough) Node.js
    NoNode,
    /// Node.js without npm
    NoNpm,
    /// another prerequisite: curl, bash, uv, Homebrew
    Needs(String),
    /// npm's global folder needs root
    RootNpm,
    /// nothing we can run (where to read how)
    Manual(String),
    Network,
    Permission,
    Disk,
    /// the installer reached for sudo
    Sudo,
    Timeout,
    /// the ssh connection dropped (why)
    Connection(String),
    /// the installer exited with this status
    Exit(i32),
    /// it finished, but the agent isn't there
    Missing,
    /// it's there but doesn't run (the line that says so)
    Broken(String),
}

impl Why {
    /// Stable name (JSON).
    pub fn code(&self) -> &'static str {
        match self {
            Why::NoNode => "no_node",
            Why::NoNpm => "no_npm",
            Why::Needs(_) => "missing_tool",
            Why::RootNpm => "npm_needs_root",
            Why::Manual(_) => "no_command",
            Why::Network => "network",
            Why::Permission => "permission",
            Why::Disk => "disk_full",
            Why::Sudo => "sudo",
            Why::Timeout => "timeout",
            Why::Connection(_) => "connection",
            Why::Exit(_) => "exit_status",
            Why::Missing => "not_found_after",
            Why::Broken(_) => "doesnt_run",
        }
    }

    /// A few words (the checklist, the summary).
    pub fn short(&self) -> String {
        match self {
            Why::NoNode => "needs Node.js".into(),
            Why::NoNpm => "needs npm".into(),
            Why::Needs(t) => format!("needs {t}"),
            Why::RootNpm => "npm needs root".into(),
            Why::Manual(_) => "no install command".into(),
            Why::Network => "network error".into(),
            Why::Permission => "permission denied".into(),
            Why::Disk => "disk full".into(),
            Why::Sudo => "wanted sudo".into(),
            Why::Timeout => "timed out".into(),
            Why::Connection(_) => "connection lost".into(),
            Why::Exit(n) => format!("exit status {n}"),
            Why::Missing => "not there afterwards".into(),
            Why::Broken(_) => "doesn't run".into(),
        }
    }

    /// The whole sentence.
    pub fn text(&self, a: &Agent, host: &str) -> String {
        let on = on(host);
        let name = a.name;
        match self {
            Why::NoNode => {
                format!("{name} installs with npm, and {on} has no Node.js (18 or newer)")
            }
            Why::NoNpm => format!("{name} installs with npm, and {on} has Node.js but no npm"),
            Why::Needs(t) => format!("{name}'s installer needs {t}, which {on} doesn't have"),
            Why::RootNpm => {
                format!("npm's global folder on {on} needs root, and Aleph never runs sudo")
            }
            Why::Manual(how) => {
                format!("{name} has no install command Aleph can run — see {how}")
            }
            Why::Network => format!("a network error: {on} couldn't download {name}"),
            Why::Permission => {
                "permission denied: the installer couldn't write where it wanted to".into()
            }
            Why::Disk => format!("{on} ran out of disk space"),
            Why::Sudo => format!("{name}'s installer wants sudo on {on} — Aleph never runs sudo"),
            Why::Timeout => format!(
                "installing {name} took longer than {} minutes, so it was stopped (agents.install_timeout)",
                install_limit(&Config::load()).as_secs() / 60
            ),
            Why::Connection(e) => format!("lost the connection to {on} while installing ({e})"),
            Why::Exit(n) => format!("installing {name} on {on} failed (exit status {n})"),
            Why::Missing => {
                format!("the installer finished, but {name} isn't anywhere a shell on {on} looks")
            }
            Why::Broken(l) => format!("{name} is there, but it doesn't run: {}", ui::trunc(l, 80)),
        }
    }

    /// What to do about it.
    pub fn fix(&self, a: &Agent, host: &str, pl: Option<&Plan>) -> Option<String> {
        let on = on(host);
        let prereq = |what: &str| {
            pl.and_then(|pl| {
                pl.prereqs
                    .iter()
                    .find(|x| !x.ok && x.what.starts_with(what))
                    .and_then(|x| x.fix.clone())
            })
        };
        let yourself = || {
            pl.and_then(|pl| pl.command.as_deref()).map(|cmd| {
                format!(
                    "or install it yourself: ssh -t {host} {}",
                    crate::proc::sh_quote(cmd)
                )
            })
        };
        let s = match self {
            Why::NoNode => prereq("Node.js").unwrap_or_else(|| {
                format!("install Node.js 18 or newer on {on} — e.g. with nvm: nvm install --lts")
            }),
            Why::NoNpm => {
                prereq("npm").unwrap_or_else(|| "npm comes with Node.js (nvm installs both)".into())
            }
            Why::Needs(t) => prereq(t).unwrap_or_else(|| format!("install {t} on {on}")),
            Why::RootNpm => prereq("npm's global").unwrap_or_else(|| {
                "npm config set prefix ~/.npm-global, then put ~/.npm-global/bin on PATH (or use nvm)".into()
            }),
            Why::Manual(how) => format!("follow {how}, then aleph agents --refresh"),
            Why::Network => format!(
                "check {on}'s internet connection (a proxy, a firewall, DNS), then retry"
            ),
            Why::Permission | Why::Sudo => yourself()?.replacen("or install", "install", 1),
            Why::Disk => format!("free some space on {on}, then retry"),
            Why::Timeout => "retry on a better connection".into(),
            Why::Connection(_) => format!("aleph doctor --host {host} explains the link; then retry"),
            Why::Exit(_) => {
                let y = yourself().unwrap_or_default();
                format!("the installer's last lines say why{}", if y.is_empty() { String::new() } else { format!(" — {y}") })
            }
            Why::Missing => match pl.and_then(|pl| pl.lands_in.clone()) {
                Some(d) => format!("it should be in {d}: is that on the PATH there?"),
                None => format!("aleph --host {host} agents --refresh looks again"),
            },
            Why::Broken(_) => "installing it again usually fixes it: retry".into(),
        };
        let _ = a;
        Some(s)
    }
}

/// A plan that can't run, as a reason (pure).
pub fn why_blocked(pl: &Plan) -> Option<Why> {
    if pl.command.is_none() {
        let how = match &pl.method {
            agentinstall::Method::Manual(h) => h.clone(),
            _ => pl.agent.install.to_string(),
        };
        return Some(Why::Manual(how));
    }
    pl.blocker.as_ref()?;
    let first = pl.prereqs.iter().find(|x| !x.ok)?;
    Some(if first.what.starts_with("Node.js") {
        Why::NoNode
    } else if first.what == "npm" {
        Why::NoNpm
    } else if first.what.starts_with("npm's global") {
        Why::RootNpm
    } else {
        Why::Needs(first.what.clone())
    })
}

/// What an installer's output says went wrong (pure): None when it doesn't say.
pub fn classify(text: &str) -> Option<Why> {
    let t = text.to_lowercase();
    let any = |v: &[&str]| v.iter().any(|p| t.contains(p));
    if t.contains("ccsudo") {
        return Some(Why::Sudo);
    }
    if any(&[
        "node: not found",
        "node: command not found",
        "env: node: no such file",
        "env: 'node': no such file",
        "env: ‘node’: no such file",
        "node: no such file or directory",
    ]) {
        return Some(Why::NoNode);
    }
    if any(&["npm: not found", "npm: command not found"]) {
        return Some(Why::NoNpm);
    }
    if any(&[
        "could not resolve host",
        "enotfound",
        "eai_again",
        "getaddrinfo",
        "temporary failure in name resolution",
        "network is unreachable",
        "failed to connect",
        "connection refused",
        "econnrefused",
        "econnreset",
        "etimedout",
        "connection timed out",
        "curl: (6)",
        "curl: (7)",
        "curl: (28)",
        "curl: (35)",
        "unable to connect",
        "network request to",
        "ssl certificate problem",
    ]) {
        return Some(Why::Network);
    }
    if any(&["no space left on device", "enospc"]) {
        return Some(Why::Disk);
    }
    if any(&[
        "eacces",
        "permission denied",
        "operation not permitted",
        "read-only file system",
        "eperm",
    ]) {
        return Some(Why::Permission);
    }
    None
}

// ------------------------------------------------------------------ installing one

/// How a check after the install went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Ok,
    Warn,
    Bad,
    Info,
}

/// What an install reports while it runs.
pub enum Ev<'a> {
    /// what it's doing now (the spinner's words)
    Stage(String),
    /// a line of the installer's output
    Line(&'a str),
    /// a validation or verification result
    Check(Mark, String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Installed {
        version: String,
        path: String,
        /// it works, but: not on the PATH, shadowed, …
        note: Option<String>,
    },
    Already {
        version: String,
        path: String,
        note: Option<String>,
    },
    Failed {
        why: Why,
        fix: Option<String>,
        /// the installer's last lines
        tail: Vec<String>,
        /// its whole output
        log: Option<PathBuf>,
    },
    Skipped(String),
}

impl Outcome {
    pub fn failed(&self) -> bool {
        matches!(self, Outcome::Failed { .. })
    }
    pub fn status(&self) -> &'static str {
        match self {
            Outcome::Installed { .. } => "installed",
            Outcome::Already { .. } => "already",
            Outcome::Failed { .. } => "failed",
            Outcome::Skipped(_) => "skipped",
        }
    }
}

/// What the validation script found (pure parse of its `CCV` lines).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    /// `command -v` with your login shell's PATH
    pub login: Option<String>,
    /// … and with a fresh interactive shell's (rc files)
    pub shell: Option<String>,
    /// (the one that ran, its exit status, its first lines)
    pub ran: Option<(String, i32, String)>,
}

/// The validation for one executable: where `command -v` finds it (login
/// PATH, then an interactive shell's), and its `--version` (15 s at most).
/// Runs after the PATH prelude (`probe::sh`): `P` is the login PATH, `O`
/// the one ssh gave us.
pub fn check_script(exe: &str) -> String {
    let e = crate::proc::sh_quote(exe);
    format!(
        r#"LC_ALL=C; export LC_ALL
E={e}
# the fake world searches no system folders (CCH_SYS_DIRS=): nor does this
if [ -n "${{CCH_SYS_DIRS+x}}" ] && [ -z "$CCH_SYS_DIRS" ]; then PATH=$(printf '%s\n' "$PATH" | tr ':' '\n' | grep -v -x -e /opt/homebrew/bin -e /usr/local/bin | paste -sd: -); fi
L=$(PATH=${{P:-$PATH}}; command -v "$E" 2>/dev/null | grep '^/' | head -n 1)
S=$( ( PATH=${{O:-$PATH}}; export PATH; exec "${{SHELL:-/bin/sh}}" -lic "command -v $E" ) </dev/null 2>/dev/null | grep '^/' | tail -n 1)
printf 'CCV\tlogin\t%s\n' "$L"
printf 'CCV\tshell\t%s\n' "$S"
X=${{L:-$S}}
[ -n "$X" ] || X=$(command -v "$E" 2>/dev/null | grep '^/' | head -n 1)
if [ -n "$X" ]; then
  D=$(mktemp 2>/dev/null) || D=/tmp/cch-check.$$
  ( PATH="${{X%/*}}:$PATH"; export PATH; exec "$X" --version ) </dev/null >"$D" 2>&1 & vp=$!
  ( sleep 15; kill -9 $vp ) >/dev/null 2>&1 & vk=$!
  wait $vp; rc=$?; kill $vk 2>/dev/null
  printf 'CCV\trun\t%s\t%s\t%s\n' "$X" "$rc" "$(tr -d '\r' <"$D" | grep . | head -n 2 | tr '\t\n' '  ')"
  rm -f "$D"
fi
"#
    )
}

/// The validation's answer (pure).
pub fn parse_check(text: &str) -> Checked {
    let mut c = Checked::default();
    for l in text.lines() {
        let f: Vec<&str> = l.splitn(5, '\t').collect();
        if f.first() != Some(&"CCV") || f.len() < 3 {
            continue;
        }
        let v = f[2].trim();
        match f[1] {
            "login" if !v.is_empty() => c.login = Some(v.to_string()),
            "shell" if !v.is_empty() => c.shell = Some(v.to_string()),
            "run" if !v.is_empty() => {
                let rc = f.get(3).and_then(|x| x.trim().parse().ok()).unwrap_or(-1);
                let out = f.get(4).map(|x| x.trim().to_string()).unwrap_or_default();
                c.ran = Some((v.to_string(), rc, out));
            }
            _ => {}
        }
    }
    c
}

/// `~/.local/state/aleph/installs/<host>-<agent>-<time>.log` (the newest 40 kept).
fn save_log(host: &str, id: &str, body: &str) -> Option<PathBuf> {
    let dir = config::state_dir().join("installs");
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let f = dir.join(format!("{safe}-{id}-{}.log", crate::cache::now()));
    // (the exit-status marker is Aleph's own; the last line says it in words)
    let body: String = body
        .lines()
        .filter(|l| !l.starts_with("CCX\t"))
        .map(|l| format!("{l}\n"))
        .collect();
    config::write_atomic(&f, body.as_bytes()).ok()?;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        v.sort();
        let n = v.len().saturating_sub(40);
        for (_, p) in v.into_iter().take(n) {
            let _ = std::fs::remove_file(p);
        }
    }
    Some(f)
}

fn tail_of(text: &str) -> Vec<String> {
    let v: Vec<String> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.starts_with("CCX\t") && !l.contains("CCSUDO"))
        .map(|l| ui::trunc(l, 160))
        .collect();
    v[v.len().saturating_sub(TAIL)..].to_vec()
}

/// After an install (or for one that's there): validate, then verify with a
/// fresh probe. The outcome, and that probe.
fn confirm(
    p: &HostProbe,
    ag: &'static Agent,
    pl: Option<&Plan>,
    out: &str,
    log: Option<PathBuf>,
    fresh: bool,
    ev: &mut dyn FnMut(Ev),
) -> (Outcome, Option<HostProbe>) {
    let on = on(&p.host);
    let exe = ag.exe();
    ev(Ev::Stage(format!("Checking that {} runs on {on}", ag.name)));
    let ck = probe::sh(&p.host, &check_script(exe), T_CHECK)
        .map(|o| parse_check(&o.text()))
        .unwrap_or_default();
    let q = if fresh {
        ev(Ev::Stage(format!("Looking at {on} again")));
        Some(probe::run(&p.host))
    } else {
        None
    };
    let now = q.as_ref().filter(|q| q.ok()).unwrap_or(p);
    let f = now.agent(ag.id).cloned();
    // validate: the login PATH, and --version
    match (&ck.login, &ck.shell) {
        (Some(l), _) => ev(Ev::Check(
            Mark::Ok,
            format!("on your login PATH · {}", now.tilde(l)),
        )),
        (None, Some(s)) => ev(Ev::Check(
            Mark::Ok,
            format!("on your shell's PATH (rc files) · {}", now.tilde(s)),
        )),
        (None, None) => {
            if let Some(f) = &f {
                ev(Ev::Check(
                    Mark::Warn,
                    format!(
                        "not on your login PATH — it's in {}; herdr types `{exe}` into a shell",
                        now.tilde(f.dir())
                    ),
                ))
            }
        }
    }
    match &ck.ran {
        Some((_, 0, line)) => ev(Ev::Check(
            Mark::Ok,
            format!("{exe} --version · {}", ui::trunc(line, 60)),
        )),
        Some((_, rc, line)) => ev(Ev::Check(
            Mark::Bad,
            format!(
                "{exe} --version failed ({}) · {}",
                if *rc == 137 {
                    "it hung".to_string()
                } else {
                    format!("status {rc}")
                },
                ui::trunc(line, 60)
            ),
        )),
        None => {}
    }
    // verify: the probe found it; what herdr there makes of it
    let Some(f) = f else {
        let why = now
            .skipped
            .iter()
            .find(|s| s.id == ag.id && s.broken)
            .map(|s| Why::Broken(s.why.clone()))
            .or_else(|| match &ck.ran {
                Some((_, rc, line)) if *rc != 0 => Some(Why::Broken(line.clone())),
                _ => None,
            })
            .or_else(|| classify(out))
            .unwrap_or(Why::Missing);
        let fix = why.fix(ag, &p.host, pl);
        return (
            Outcome::Failed {
                why,
                fix,
                tail: tail_of(out),
                log,
            },
            q,
        );
    };
    match now.herdr_version() {
        Some(hv) => ev(Ev::Check(
            Mark::Ok,
            format!("herdr {hv} starts it · {}", probe::start_of(Some(ag))),
        )),
        None => ev(Ev::Check(
            Mark::Warn,
            format!(
                "herdr isn't on {on} yet — sessions need it (aleph host setup {})",
                p.host
            ),
        )),
    }
    if let Some(name) = ag.integration {
        match now.integ(name).map(|i| &i.state) {
            Some(IState::Current(_)) => ev(Ev::Check(
                Mark::Ok,
                format!("herdr's {name} integration is installed"),
            )),
            Some(IState::Missing) | Some(IState::Outdated { .. }) => ev(Ev::Check(
                Mark::Info,
                format!("herdr's {name} integration isn't installed yet — offered at the end"),
            )),
            _ => {}
        }
    }
    let note = if let Some(s) = &f.shadowed_by {
        Some(format!(
            "{} comes first on the PATH there and isn't {} — a shell runs that one",
            now.tilde(s),
            ag.name
        ))
    } else if f.on_path == Some(false) || (ck.login.is_none() && ck.shell.is_none()) {
        Some(format!(
            "{} isn't on your shell's PATH on {on} — herdr types `{exe}` into a shell, so add it to PATH (e.g. in ~/.profile)",
            now.tilde(f.dir())
        ))
    } else {
        None
    };
    (
        Outcome::Installed {
            version: f.version.clone(),
            path: f.path.clone(),
            note,
        },
        q,
    )
}

/// Install one agent on the probed machine, reporting as it goes. A fresh
/// probe comes back when one was taken (after an install).
pub fn install(
    p: &HostProbe,
    ag: &'static Agent,
    ev: &mut dyn FnMut(Ev),
) -> (Outcome, Option<HostProbe>) {
    if let Some(f) = p.agent(ag.id) {
        let (o, q) = confirm(p, ag, None, "", None, false, ev);
        let note = match o {
            Outcome::Installed { note, .. } => note,
            _ => None,
        };
        return (
            Outcome::Already {
                version: f.version.clone(),
                path: f.path.clone(),
                note,
            },
            q,
        );
    }
    let pl = agentinstall::plan(ag, p);
    if let Some(why) = why_blocked(&pl) {
        let fix = why.fix(ag, &p.host, Some(&pl));
        return (
            Outcome::Failed {
                why,
                fix,
                tail: vec![],
                log: None,
            },
            None,
        );
    }
    let Some(script) = agentinstall::script(&pl) else {
        return (
            Outcome::Failed {
                why: Why::Manual(ag.install.to_string()),
                fix: None,
                tail: vec![],
                log: None,
            },
            None,
        );
    };
    let on = on(&p.host);
    ev(Ev::Stage(format!("Installing {} on {on}", ag.name)));
    let mut text = String::new();
    let r = probe::sh_stream(&p.host, &script, install_limit(&Config::load()), |l| {
        text.push_str(l);
        text.push('\n');
        if !l.starts_with("CCX\t") && !l.contains("CCSUDO") {
            ev(Ev::Line(l));
        }
    });
    let cmd = pl.command.clone().unwrap_or_default();
    let fail = |why: Why, text: &str, log: Option<PathBuf>| {
        let fix = why.fix(ag, &p.host, Some(&pl));
        (
            Outcome::Failed {
                why,
                fix,
                tail: tail_of(text),
                log,
            },
            None,
        )
    };
    let o = match r {
        Ok(o) => o,
        Err(e) => {
            let log = save_log(&p.host, ag.id, &format!("$ {cmd}\n{text}\n[{e}]\n"));
            return fail(Why::Connection(e), &text, log);
        }
    };
    let (rc, sudo) = agentinstall::outcome(&o.text());
    let log = save_log(
        &p.host,
        ag.id,
        &format!(
            "$ {cmd}\n{text}{}\n[{}]\n",
            if o.stderr.trim().is_empty() {
                String::new()
            } else {
                format!("{}\n", o.stderr.trim_end())
            },
            match rc {
                Some(n) => format!("exit status {n}"),
                None => format!("no exit status: {}", o.reason()),
            }
        ),
    );
    if o.timed_out {
        return fail(Why::Timeout, &text, log);
    }
    if sudo {
        return fail(Why::Sudo, &text, log);
    }
    match rc {
        Some(0) => {}
        Some(n) => return fail(classify(&text).unwrap_or(Why::Exit(n)), &text, log),
        None => return fail(Why::Connection(o.reason()), &text, log),
    }
    confirm(p, ag, Some(&pl), &text, log, true, ev)
}

// ------------------------------------------------------------------ the flow

#[derive(Debug, Clone, Copy, Default)]
pub struct Opts {
    /// installs: don't ask (the integrations still are, on a terminal)
    pub yes: bool,
    /// show the plan, run nothing
    pub dry_run: bool,
    /// herdr's integrations for them: Some(true) install, Some(false)
    /// don't, None ask on a terminal
    pub integrations: Option<bool>,
    /// Node.js first for the npm ones that need it: None follows
    /// `agents.node`
    pub node: Option<bool>,
}

/// How putting Node.js on the machine went.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeRes {
    Installed {
        version: String,
        path: String,
        npm: String,
    },
    Failed {
        code: &'static str,
        text: String,
        fix: String,
        log: Option<PathBuf>,
    },
    Skipped(String),
}

impl NodeRes {
    fn to_json(&self) -> Value {
        match self {
            NodeRes::Installed { version, path, npm } => {
                json!({"status": "installed", "version": version, "path": path, "npm": npm})
            }
            NodeRes::Failed {
                code,
                text,
                fix,
                log,
            } => json!({
                "status": "failed", "why": code, "text": text, "fix": fix,
                "log": log.as_ref().map(|l| l.display().to_string()),
            }),
            NodeRes::Skipped(w) => json!({"status": "skipped", "text": w}),
        }
    }
}

/// How a machine's agent setup went.
pub struct Report {
    pub host: String,
    pub results: Vec<(&'static Agent, Outcome)>,
    /// Node.js, when the npm ones needed it
    pub node: Option<NodeRes>,
    /// the machine as it is now
    pub probe: HostProbe,
    /// the checklist was left, or the installs declined
    pub declined: bool,
}

impl Report {
    fn new(host: &str, p: HostProbe) -> Report {
        Report {
            host: host.to_string(),
            results: vec![],
            node: None,
            probe: p,
            declined: false,
        }
    }
    /// Nothing failed (and something was chosen).
    pub fn ok(&self) -> bool {
        !self.declined && !self.results.iter().any(|(_, o)| o.failed())
    }
    pub fn count(&self, status: &str) -> usize {
        self.results
            .iter()
            .filter(|(_, o)| o.status() == status)
            .count()
    }
    pub fn to_json(&self) -> Value {
        let results: Vec<Value> = self
            .results
            .iter()
            .map(|(a, o)| {
                let mut v = json!({"id": a.id, "name": a.name, "status": o.status()});
                match o {
                    Outcome::Installed {
                        version,
                        path,
                        note,
                    }
                    | Outcome::Already {
                        version,
                        path,
                        note,
                    } => {
                        v["version"] = json!(version);
                        v["path"] = json!(path);
                        v["note"] = json!(note);
                    }
                    Outcome::Failed {
                        why,
                        fix,
                        tail,
                        log,
                    } => {
                        v["why"] = json!(why.code());
                        v["text"] = json!(why.text(a, &self.host));
                        v["fix"] = json!(fix);
                        v["tail"] = json!(tail);
                        v["log"] = json!(log.as_ref().map(|l| l.display().to_string()));
                    }
                    Outcome::Skipped(w) => v["text"] = json!(w),
                }
                v
            })
            .collect();
        json!({
            "host": self.host,
            "declined": self.declined,
            "node": self.node.as_ref().map(NodeRes::to_json),
            "results": results,
        })
    }
}

/// Node.js's mark (its hexagon, in its green).
fn node_glyph() -> Span<'static> {
    if c::mono() {
        Span::styled("⬢", ui::bold())
    } else {
        Span::styled(
            "⬢",
            Style::new()
                .fg(Color::Rgb(95, 160, 78))
                .add_modifier(Modifier::BOLD),
        )
    }
}

fn major(v: &str) -> u64 {
    v.trim_start_matches(['v', 'V'])
        .split('.')
        .next()
        .and_then(|x| x.parse().ok())
        .unwrap_or(0)
}

/// Put Node.js `r` on the machine (output live), then look again: it must
/// be there, and the one a shell finds. The result, and the new look.
fn node_install(
    p: &HostProbe,
    r: &crate::nodeinstall::Release,
    plat: &str,
) -> (NodeRes, Option<HostProbe>) {
    let on = on(&p.host);
    let sp = Spinner::start(format!("Installing {} on {on}", r.label()));
    let mut text = String::new();
    let res = probe::sh_stream(
        &p.host,
        &crate::nodeinstall::install_script(r, plat),
        install_limit(&Config::load()),
        |l| {
            text.push_str(l);
            text.push('\n');
            if !l.starts_with("CCN\t") {
                let l = format!("  │ {}", ui::trunc(l, 160));
                sp.print(|| say(Line::from(Span::styled(l, ui::faint()))));
            }
        },
    );
    sp.stop();
    let log = save_log(
        &p.host,
        "node",
        &format!("$ install {} ({plat})\n{text}", r.label()),
    );
    let fail = |code: &'static str, t: String, fix: String| {
        (
            NodeRes::Failed {
                code,
                text: t,
                fix,
                log: log.clone(),
            },
            None,
        )
    };
    let o = match res {
        Ok(o) => o,
        Err(e) => {
            return fail(
                "connection",
                format!("lost the connection to {on} while installing Node.js ({e})"),
                format!(
                    "aleph doctor --host {} explains the link; then retry",
                    p.host
                ),
            )
        }
    };
    if o.timed_out {
        return fail(
            "timeout",
            format!("installing Node.js on {on} took too long, so it was stopped"),
            "retry on a better connection".into(),
        );
    }
    match crate::nodeinstall::parse_install(&o.text()) {
        Some(Ok((v, path, npm))) => {
            say(check_line(
                Mark::Ok,
                "sha256 matches nodejs.org's SHASUMS256.txt",
            ));
            let sp = Spinner::start(format!("Looking at {on} again"));
            let q = probe::run(&p.host);
            sp.stop();
            say(check_line(
                Mark::Ok,
                &format!("node {v} · {}", q.tilde(&path)),
            ));
            if !npm.is_empty() {
                say(check_line(
                    Mark::Ok,
                    &format!("npm {npm} · its global folder is yours (no sudo)"),
                ));
            }
            let first = q.tool("node").map(|t| (t.path.clone(), t.version.clone()));
            if let Some((fp, fv)) = first.filter(|(_, fv)| !fv.is_empty() && major(fv) < 18) {
                return fail(
                    "older_first",
                    format!(
                        "Node.js {v} is in ~/.local/bin, but an older one comes first on the PATH there ({} v{fv})",
                        q.tilde(&fp)
                    ),
                    "put ~/.local/bin first on your PATH (e.g. in ~/.profile), then retry".into(),
                );
            }
            (
                NodeRes::Installed {
                    version: v,
                    path,
                    npm,
                },
                Some(q),
            )
        }
        Some(Err((step, detail))) => {
            let (code, t, fix): (&'static str, String, String) = match (step.as_str(), classify(&text)) {
                (_, Some(Why::Disk)) => (
                    "disk_full",
                    format!("{on} ran out of disk space installing Node.js"),
                    format!("free some space on {on}, then retry"),
                ),
                ("download", _) => (
                    "network",
                    format!("a network error: {on} couldn't download Node.js from nodejs.org"),
                    format!("check {on}'s internet connection (a proxy, a firewall, DNS), then retry"),
                ),
                ("checksum", _) => (
                    "checksum",
                    format!("the download didn't match nodejs.org's checksum ({detail}) — nothing was installed"),
                    "retry (a proxy that rewrites downloads does this)".into(),
                ),
                ("nosha", _) => (
                    "missing_tool",
                    format!("{on} has neither sha256sum nor shasum to check the download"),
                    "install coreutils (sha256sum), then retry".into(),
                ),
                (_, Some(Why::Permission)) => (
                    "permission",
                    format!("permission denied unpacking Node.js on {on} ({detail})"),
                    format!("check who owns ~/{} there", crate::nodeinstall::HOME_DIR),
                ),
                _ => (
                    "unpack",
                    format!("couldn't unpack Node.js on {on} ({detail})"),
                    "the log says why".into(),
                ),
            };
            fail(code, t, fix)
        }
        None => fail(
            "exit_status",
            format!("installing Node.js on {on} stopped ({})", o.reason()),
            "the log says why".into(),
        ),
    }
}

fn print_node(p: &HostProbe, n: &NodeRes) {
    match n {
        NodeRes::Installed { version, path, .. } => ok(&format!(
            "Node.js {version} installed on {} · {}",
            on(&p.host),
            p.tilde(path)
        )),
        NodeRes::Failed { text, fix, log, .. } => {
            err(text);
            hint(fix);
            if let Some(l) = log {
                say(Line::from(Span::styled(
                    format!("  log: {}", crate::doctor::tilde(l)),
                    ui::faint(),
                )));
            }
        }
        NodeRes::Skipped(w) => say(Line::from(vec![
            Span::styled("○ ", ui::fg(c::YELLOW())),
            Span::raw(format!("Node.js · {w}")),
        ])),
    }
}

/// Retry, skip or show the log (a typed answer; no terminal: skip).
enum Next {
    Retry,
    Skip,
    Log,
}

fn next_step(has_log: bool) -> Next {
    if !io::stdin().is_terminal() {
        return Next::Skip;
    }
    let a = ask(
        "Retry, skip, or show the log?",
        if has_log { "r / s / l" } else { "r / s" },
    );
    match a.trim().to_lowercase().chars().next() {
        Some('r') => Next::Retry,
        Some('l') if has_log => Next::Log,
        _ => Next::Skip,
    }
}

fn check_line(m: Mark, text: &str) -> Line<'static> {
    let (g, col) = match m {
        Mark::Ok => ("✓", c::GREEN()),
        Mark::Warn => ("!", c::YELLOW()),
        Mark::Bad => ("✗", c::RED()),
        Mark::Info => ("·", c::MUTED()),
    };
    Line::from(vec![
        Span::styled(
            format!("    {g} "),
            ui::fg(col).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            text.to_string(),
            if m == Mark::Info {
                ui::muted()
            } else {
                Style::new()
            },
        ),
    ])
}

/// One install with a spinner, its output dimmed under it.
fn attempt(p: &HostProbe, ag: &'static Agent) -> (Outcome, Option<HostProbe>) {
    let sp = Spinner::start(format!("Installing {} on {}", ag.name, on(&p.host)));
    let r = install(p, ag, &mut |e| match e {
        Ev::Stage(s) => sp.set(s),
        Ev::Line(l) => {
            let l = format!("  │ {}", ui::trunc(l, 160));
            sp.print(|| say(Line::from(Span::styled(l, ui::faint()))));
        }
        Ev::Check(m, t) => sp.print(|| say(check_line(m, &t))),
    });
    sp.stop();
    r
}

/// Print how one install went (its result line, and for a failure the why,
/// the last lines and the fix).
fn print_outcome(p: &HostProbe, ag: &'static Agent, o: &Outcome) {
    let on = on(&p.host);
    match o {
        Outcome::Installed {
            version,
            path,
            note,
        } => {
            ok(&format!(
                "{} installed on {on} · {}",
                named(ag, version),
                p.tilde(path)
            ));
            if let Some(n) = note {
                warn(n);
            }
        }
        Outcome::Already {
            version,
            path,
            note,
        } => {
            ok(&format!(
                "{} is already installed on {on} · {}",
                named(ag, version),
                p.tilde(path)
            ));
            if let Some(n) = note {
                warn(n);
            }
        }
        // (its last lines are right above: they streamed as it ran)
        Outcome::Failed { why, fix, log, .. } => {
            err(&why.text(ag, &p.host));
            if let Some(f) = fix {
                hint(f);
            }
            if let Some(l) = log {
                say(Line::from(Span::styled(
                    format!("  log: {}", crate::doctor::tilde(l)),
                    ui::faint(),
                )));
            }
        }
        Outcome::Skipped(w) => say(Line::from(vec![
            Span::styled("○ ", ui::fg(c::YELLOW())),
            Span::raw(format!("{} · {w}", ag.name)),
        ])),
    }
}

fn show_log(f: &std::path::Path) {
    let text = std::fs::read_to_string(f).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(200);
    say(Line::from(Span::styled(
        format!("── {} ", crate::doctor::tilde(f)),
        ui::muted(),
    )));
    if from > 0 {
        say(Line::from(Span::styled(
            format!("  … {from} earlier lines"),
            ui::faint(),
        )));
    }
    for l in &lines[from..] {
        say(Line::from(Span::styled(format!("  {l}"), ui::faint())));
    }
}

/// The summary (pure): Node.js when it was needed, then one line per
/// agent, aligned.
pub fn summary(
    host: &str,
    results: &[(&'static Agent, Outcome)],
    node: Option<&NodeRes>,
    p: &HostProbe,
) -> Vec<Line<'static>> {
    let n = |s: &str| results.iter().filter(|(_, o)| o.status() == s).count();
    let (inst, fail, skip) = (n("installed") + n("already"), n("failed"), n("skipped"));
    let mut head = vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled(format!("Agents on {}", on(host)), ui::bold()),
        Span::styled(format!("  {inst} installed"), ui::fg(c::GREEN())),
    ];
    if fail > 0 {
        head.push(Span::styled(format!(" · {fail} failed"), ui::fg(c::RED())));
    }
    if skip > 0 {
        head.push(Span::styled(format!(" · {skip} skipped"), ui::muted()));
    }
    let mut out = vec![Line::from(head)];
    let wn = results
        .iter()
        .map(|(a, _)| ui::width(a.name))
        .chain(node.map(|_| 7))
        .max()
        .unwrap_or(8);
    if let Some(n) = node {
        let (g, col, mid, rest) = match n {
            NodeRes::Installed { version, path, .. } => {
                ("✓", c::GREEN(), format!("{version} (LTS)"), p.tilde(path))
            }
            NodeRes::Failed { code, fix, .. } => {
                ("✗", c::RED(), code.replace('_', " "), fix.clone())
            }
            NodeRes::Skipped(w) => ("○", c::MUTED(), "skipped".into(), w.clone()),
        };
        out.push(Line::from(vec![
            Span::styled(format!("  {g} "), ui::fg(col).add_modifier(Modifier::BOLD)),
            node_glyph(),
            Span::raw(format!(" {}  ", ui::pad("Node.js", wn))),
            Span::styled(ui::pad(&mid, 22), ui::fg(col)),
            Span::styled(format!("  {rest}"), ui::faint()),
        ]));
    }
    for (a, o) in results {
        let (g, col, mid, rest): (&str, Color, String, String) = match o {
            Outcome::Installed {
                version,
                path,
                note,
            }
            | Outcome::Already {
                version,
                path,
                note,
            } => (
                if note.is_some() { "!" } else { "✓" },
                if note.is_some() {
                    c::YELLOW()
                } else {
                    c::GREEN()
                },
                if matches!(o, Outcome::Already { .. }) {
                    format!("{version} (already)")
                } else {
                    version.clone()
                },
                match note {
                    Some(_) => format!("{} · not on PATH", p.tilde(path)),
                    None => p.tilde(path),
                },
            ),
            Outcome::Failed { why, fix, .. } => {
                ("✗", c::RED(), why.short(), fix.clone().unwrap_or_default())
            }
            Outcome::Skipped(w) => ("○", c::MUTED(), "skipped".into(), w.clone()),
        };
        out.push(Line::from(vec![
            Span::styled(format!("  {g} "), ui::fg(col).add_modifier(Modifier::BOLD)),
            icons::agent(a.id),
            Span::raw(format!(" {}  ", ui::pad(a.name, wn))),
            Span::styled(ui::pad(&mid, 22), ui::fg(col)),
            Span::styled(format!("  {rest}"), ui::faint()),
        ]));
    }
    out
}

/// Node.js for the npm ones: the release to install (or why it can't).
type NodePlan = Result<(crate::nodeinstall::Release, String), String>;

/// The plans, before asking: Node.js first when the npm ones need it, then
/// the exact command for each (or why it can't).
fn print_plans(cfg: &Config, p: &HostProbe, todo: &[&'static Agent], node: Option<&NodePlan>) {
    if let ([one], None) = (todo, node) {
        crate::agentcmd::print_install_plan(cfg, p, &agentinstall::plan(one, p));
        return;
    }
    say(Line::from(vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled(
            if todo.len() == 1 {
                format!("Install {} on {}", todo[0].name, on(&p.host))
            } else {
                format!("Install {} agents on {}", todo.len(), on(&p.host))
            },
            ui::bold(),
        ),
        Span::styled(
            "  · one after another; a failure never stops the rest",
            ui::faint(),
        ),
    ]));
    let wn = todo
        .iter()
        .map(|a| ui::width(a.name))
        .chain(node.map(|_| 7))
        .max()
        .unwrap_or(8);
    let need: Vec<&str> = todo
        .iter()
        .filter(|a| needs_node(&agentinstall::plan(a, p)))
        .map(|a| a.name)
        .collect();
    match node {
        Some(Ok((r, plat))) => say(Line::from(vec![
            Span::raw("  "),
            node_glyph(),
            Span::raw(format!(" {}  ", ui::pad("Node.js", wn))),
            Span::styled(format!("{} {}", r.version, plat), ui::bold()),
            Span::styled(
                format!(
                    "  the current LTS (\"{}\"), from nodejs.org · sha256-checked · into ~/{} · no sudo · for {}",
                    r.lts,
                    crate::nodeinstall::HOME_DIR,
                    list(&need)
                ),
                ui::faint(),
            ),
        ])),
        Some(Err(e)) => say(Line::from(vec![
            Span::raw("  "),
            node_glyph(),
            Span::raw(format!(" {}  ", ui::pad("Node.js", wn))),
            Span::styled(format!("✗ can't install it: {e}"), ui::fg(c::RED())),
        ])),
        None => {}
    }
    let node_ok = matches!(node, Some(Ok(_)));
    for a in todo {
        let pl = agentinstall::plan(a, p);
        let mut l = vec![
            Span::raw("  "),
            icons::agent(a.id),
            Span::raw(format!(" {}  ", ui::pad(a.name, wn))),
        ];
        match why_blocked(&pl).filter(|_| !(node_ok && needs_node(&pl))) {
            Some(w) => {
                l.push(Span::styled(format!("✗ {}", w.short()), ui::fg(c::RED())));
                if let Some(f) = w.fix(a, &p.host, Some(&pl)) {
                    l.push(Span::styled(
                        format!("  {}", ui::trunc(&f, 90)),
                        ui::faint(),
                    ));
                }
            }
            None => {
                l.push(Span::styled("$ ", ui::faint()));
                l.push(Span::styled(
                    pl.command.clone().unwrap_or_default(),
                    ui::bold(),
                ));
                if needs_node(&pl) {
                    l.push(Span::styled("  · after Node.js", ui::faint()));
                }
            }
        }
        say(Line::from(l));
    }
}

/// Retry, skip or the log after a failure; true: retry.
fn again(log: Option<&std::path::Path>) -> bool {
    loop {
        match next_step(log.is_some()) {
            Next::Retry => return true,
            Next::Log => {
                if let Some(l) = log {
                    show_log(l);
                }
            }
            Next::Skip => return false,
        }
    }
}

/// Look at the machine again (after a fix, before a retry).
fn relook(host: &str, cur: &mut HostProbe) {
    let sp = Spinner::start(format!("Looking at {} again", on(host)));
    let q = probe::run(host);
    sp.stop();
    if q.ok() {
        *cur = q;
    }
}

/// Choose (unless `ids` says which), install, validate, verify, summarise,
/// then offer the integrations. Printed; asks on a terminal.
pub fn run(cfg: &Config, p: HostProbe, ids: Option<Vec<&'static str>>, o: Opts) -> Report {
    let host = p.host.clone();
    let on = on(&host);
    let mut rep = Report::new(&host, p.clone());
    let chosen: Vec<&'static Agent> = match ids {
        Some(v) => v.iter().filter_map(|id| agents::find(id)).collect(),
        None => {
            let (ch, note) = choices(cfg, &p);
            say(Line::from(vec![
                Span::styled("⏺ ", ui::fg(c::ACCENT())),
                Span::styled(format!("Agents for {on}"), ui::bold()),
                Span::styled(
                    format!(
                        "  · {} that Aleph knows; nothing installs until you say so",
                        ch.len()
                    ),
                    ui::faint(),
                ),
            ]));
            say(Line::from(Span::styled(format!("  {note}"), ui::muted())));
            let items: Vec<Item> = ch
                .iter()
                .map(|x| Item::new(x.agent.name, x.hint(), x.on).icon(icons::agent(x.agent.id)))
                .collect();
            match pick::multi(&format!("Agents for {on}"), &items) {
                Some(idx) => idx.iter().map(|&i| ch[i].agent).collect(),
                None => {
                    hint(&format!(
                        "nothing installed — aleph host setup {host} asks again"
                    ));
                    rep.declined = true;
                    return rep;
                }
            }
        }
    };
    let mut cur = p;
    let (have, todo): (Vec<&'static Agent>, Vec<&'static Agent>) =
        chosen.iter().partition(|a| cur.agent(a.id).is_some());
    for a in &have {
        let (out, _) = install(&cur, a, &mut |_| {});
        print_outcome(&cur, a, &out);
        rep.results.push((a, out));
    }
    if todo.is_empty() {
        if chosen.is_empty() {
            say(Line::from(Span::styled(
                "  nothing chosen — nothing to install",
                ui::muted(),
            )));
        }
    } else {
        // the npm ones on a machine without Node.js: the current LTS first
        let need_node = todo
            .iter()
            .any(|a| needs_node(&agentinstall::plan(a, &cur)));
        let node_plan: Option<NodePlan> =
            (need_node && o.node.unwrap_or_else(|| crate::nodeinstall::enabled(cfg))).then(|| {
                let sp = Spinner::start(format!("Looking up Node.js's current LTS for {on}"));
                let r = probe::sh(
                    &host,
                    &crate::nodeinstall::look_script(),
                    Duration::from_secs(60),
                )
                .map_err(|e| format!("couldn't look ({e})"))
                .and_then(|o| crate::nodeinstall::plan(&crate::nodeinstall::parse_look(&o.text())));
                sp.stop();
                r
            });
        let node_ok = matches!(node_plan, Some(Ok(_)));
        print_plans(cfg, &cur, &todo, node_plan.as_ref());
        if o.dry_run {
            hint("--dry-run: nothing was run");
            rep.node = node_plan.as_ref().map(|np| match np {
                Ok(_) => NodeRes::Skipped("dry run".into()),
                Err(e) => NodeRes::Failed {
                    code: "unavailable",
                    text: e.clone(),
                    fix: "install Node.js 18 or newer yourself — e.g. with nvm: nvm install --lts"
                        .into(),
                    log: None,
                },
            });
            for a in &todo {
                let pl = agentinstall::plan(a, &cur);
                let out = match why_blocked(&pl).filter(|_| !(node_ok && needs_node(&pl))) {
                    Some(why) => Outcome::Failed {
                        fix: why.fix(a, &host, Some(&pl)),
                        why,
                        tail: vec![],
                        log: None,
                    },
                    None => Outcome::Skipped("dry run".into()),
                };
                rep.results.push((a, out));
            }
            rep.probe = cur;
            return rep;
        }
        let mut ready: Vec<String> = Vec::new();
        if let Some(Ok((r, _))) = &node_plan {
            ready.push(format!("Node.js {} (LTS)", r.version));
        }
        ready.extend(
            todo.iter()
                .filter(|a| {
                    let pl = agentinstall::plan(a, &cur);
                    why_blocked(&pl).is_none() || (node_ok && needs_node(&pl))
                })
                .map(|a| a.name.to_string()),
        );
        let names: Vec<&str> = ready.iter().map(String::as_str).collect();
        if !ready.is_empty() && !o.yes && !yes(&format!("Install {} on {on}?", list(&names))) {
            hint(&format!(
                "aleph agents install {} --host {host} does it later",
                todo.iter().map(|a| a.id).collect::<Vec<_>>().join(" ")
            ));
            for a in &todo {
                rep.results.push((a, Outcome::Skipped("not now".into())));
            }
            rep.declined = true;
            rep.probe = cur;
            return rep;
        }
        // Node.js first: retry / skip / the log, like an agent
        // (a few words, for the npm ones' fix: the full story is above)
        let mut node_failed: Option<String> = match &node_plan {
            Some(Err(e)) if e.contains("nodejs.org") => Some("nodejs.org was out of reach".into()),
            Some(Err(e)) => Some(ui::trunc(e, 60)),
            _ => None,
        };
        if let Some(Ok((r, plat))) = &node_plan {
            say(Line::raw(""));
            say(Line::from(vec![
                Span::styled(
                    "[Node.js] ",
                    ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
                ),
                node_glyph(),
                Span::styled(format!(" {}", r.label()), ui::bold()),
                Span::styled(format!(" on {on}"), ui::muted()),
            ]));
            let res = loop {
                let (res, q) = node_install(&cur, r, plat);
                if let Some(q) = q.filter(|q| q.ok()) {
                    cur = q;
                }
                print_node(&cur, &res);
                let NodeRes::Failed { log, .. } = &res else {
                    break res;
                };
                if !again(log.as_deref()) {
                    break res;
                }
                relook(&host, &mut cur);
            };
            if let NodeRes::Failed { code, .. } = &res {
                node_failed = Some(match *code {
                    "network" => "its download failed".to_string(),
                    "checksum" => "its download didn't match the checksum".to_string(),
                    c => c.replace('_', " "),
                });
            }
            rep.node = Some(res);
        } else if let Some(Err(e)) = &node_plan {
            rep.node = Some(NodeRes::Failed {
                code: "unavailable",
                text: format!("Node.js can't be installed on {on}: {e}"),
                fix: "install Node.js 18 or newer yourself — e.g. with nvm: nvm install --lts"
                    .into(),
                log: None,
            });
        }
        let n = todo.len();
        for (i, a) in todo.iter().enumerate() {
            if n > 1 {
                say(Line::raw(""));
                say(Line::from(vec![
                    Span::styled(
                        format!("[{}/{n}] ", i + 1),
                        ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
                    ),
                    icons::agent(a.id),
                    Span::styled(format!(" {}", a.name), ui::bold()),
                    Span::styled(format!(" on {on}"), ui::muted()),
                ]));
            }
            let out = loop {
                let (mut out, q) = attempt(&cur, a);
                if let Some(q) = q.filter(|q| q.ok()) {
                    cur = q;
                }
                // it needed the Node.js that couldn't go on: say that
                if let (
                    Outcome::Failed {
                        why: Why::NoNode | Why::NoNpm,
                        fix,
                        ..
                    },
                    Some(nf),
                ) = (&mut out, &node_failed)
                {
                    *fix = Some(format!(
                        "Node.js couldn't be installed first ({nf}): retry, or install Node.js 18 or newer on {on} yourself — nvm install --lts"
                    ));
                }
                print_outcome(&cur, a, &out);
                let Outcome::Failed { log, .. } = &out else {
                    break out;
                };
                if !again(log.as_deref()) {
                    break out;
                }
                // they may have fixed something meanwhile: look again first
                relook(&host, &mut cur);
            };
            rep.results.push((a, out));
        }
    }
    if rep.results.len() > 1 || rep.node.is_some() {
        say(Line::raw(""));
        for l in summary(&host, &rep.results, rep.node.as_ref(), &cur) {
            say(l);
        }
    }
    // herdr's integrations for the ones that are there now (opt-in)
    let there: Vec<String> = rep
        .results
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Installed { .. } | Outcome::Already { .. }))
        .map(|(a, _)| a.id.to_string())
        .collect();
    if !there.is_empty() && cur.herdr.is_some() {
        let steps = crate::agentcmd::plan(std::slice::from_ref(&cur), &there);
        if !steps.is_empty() {
            say(Line::raw(""));
            let go = match o.integrations {
                Some(v) => Some(v),
                None if io::stdin().is_terminal() => None,
                None => Some(false),
            };
            let done = match go {
                Some(false) => {
                    hint(&format!(
                        "aleph --host {host} agents setup {} lets herdr restore their sessions after a restart",
                        there.join(" ")
                    ));
                    false
                }
                Some(true) => {
                    crate::agentcmd::setup_flow(cfg, std::slice::from_ref(&cur), &there, true)
                        .is_some()
                }
                None => crate::agentcmd::setup_flow(cfg, std::slice::from_ref(&cur), &there, false)
                    .is_some(),
            };
            if done {
                let sp = Spinner::start(format!("Looking at {on} again"));
                let q = probe::run(&host);
                sp.stop();
                if q.ok() {
                    cur = q;
                }
            }
        }
    }
    rep.probe = cur;
    rep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_of(text: &str) -> HostProbe {
        probe::parse("box3", text)
    }

    fn cfg() -> Config {
        Config::in_memory(&["hosts=box3", "local=false"])
    }

    const BARE: &str =
        "CCO\tLinux x86_64\nCCT\tcurl\t/usr/bin/curl\nCCT\tbash\t/bin/bash\nCCW\t/home/t\n";
    const NODE: &str = "CCO\tLinux x86_64\nCCT\tcurl\t/usr/bin/curl\nCCT\tbash\t/bin/bash\nCCT\tnpm\t/usr/bin/npm\t/home/t/.npm-global\tw\nCCT\tnode\t/usr/bin/node\tv22.1.0\nCCW\t/home/t\n";

    #[test]
    fn defaults_tick_what_is_there() {
        let p = probe_of("CCA\tcodex\t/b/codex\t0.46.0\nCCO\tLinux\n");
        let (t, note) = defaults(&cfg(), &p);
        assert_eq!(t, vec!["codex"]);
        assert!(note.contains("Already on box3: Codex 0.46.0"), "{note}");
    }

    #[test]
    fn defaults_on_a_bare_machine_say_what_they_picked() {
        // Node.js there: the default agent and Codex
        let (t, note) = defaults(&cfg(), &probe_of(NODE));
        assert_eq!(t, vec!["claude", "codex"]);
        assert!(
            note.contains("ticked Claude Code (the default agent) and Codex"),
            "{note}"
        );
        // no Node.js, but it can go on (curl, an official build): Codex too,
        // Node.js first, and that's said
        let (t, note) = defaults(&cfg(), &probe_of(BARE));
        assert_eq!(t, vec!["claude", "codex"]);
        assert!(
            note.contains("Node.js (the current LTS) goes on first"),
            "{note}"
        );
        // agents.node = off: Codex is held back, and that's said
        let off = Config::in_memory(&["hosts=box3", "local=false", "agents.node=off"]);
        let (t, note) = defaults(&off, &probe_of(BARE));
        assert_eq!(t, vec!["claude"]);
        assert!(note.contains("Not ticked: Codex needs Node.js"), "{note}");
        // nothing to download it with: the same
        let nocurl = BARE.replace("CCT\tcurl\t/usr/bin/curl\n", "");
        assert!(!defaults(&cfg(), &probe_of(&nocurl)).0.contains(&"codex"));
        // another default agent
        let c = Config::in_memory(&["hosts=box3", "local=false", "agent=gemini"]);
        let (t, _) = defaults(&c, &probe_of(NODE));
        assert_eq!(t, vec!["gemini", "claude"]);
    }

    #[test]
    fn choices_list_every_agent_installed_first() {
        let p = probe_of(&format!("CCA\tcodex\t/b/codex\t0.46.0\n{BARE}"));
        let (v, _) = choices(&cfg(), &p);
        assert_eq!(v.len(), agents::ALL.len());
        assert_eq!(v[0].agent.id, "codex");
        assert_eq!(v[0].hint(), "installed 0.46.0");
        assert!(v[0].on);
        let claude = v.iter().find(|x| x.agent.id == "claude").unwrap();
        assert_eq!(
            claude.hint(),
            "curl -fsSL https://claude.ai/install.sh | bash"
        );
        assert!(!claude.on);
        let gem = v.iter().find(|x| x.agent.id == "gemini").unwrap();
        assert_eq!(
            gem.hint(),
            "npm i -g @google/gemini-cli · Node.js LTS first"
        );
        let off = Config::in_memory(&["hosts=box3", "local=false", "agents.node=off"]);
        let (v, _) = choices(&off, &p);
        let gem = v.iter().find(|x| x.agent.id == "gemini").unwrap();
        assert_eq!(gem.hint(), "needs Node.js");
        // every agent in the registry has a command to run
        assert!(v
            .iter()
            .all(|x| !matches!(x.plan.method, agentinstall::Method::Manual(_))));
    }

    #[test]
    fn blocked_plans_say_why() {
        let ag = |id| agents::find(id).unwrap();
        assert_eq!(
            why_blocked(&agentinstall::plan(ag("codex"), &probe_of(BARE))),
            Some(Why::NoNode)
        );
        assert_eq!(
            why_blocked(&agentinstall::plan(ag("codex"), &probe_of(NODE))),
            None
        );
        assert!(matches!(
            why_blocked(&agentinstall::plan(agents::tests::web_only(), &probe_of(NODE))),
            Some(Why::Manual(u)) if u.starts_with("https://")
        ));
        assert_eq!(
            why_blocked(&agentinstall::plan(ag("kimi"), &probe_of(NODE))),
            Some(Why::Needs("uv".into()))
        );
        let root = NODE.replace("\tw\n", "\tro\n");
        assert_eq!(
            why_blocked(&agentinstall::plan(ag("codex"), &probe_of(&root))),
            Some(Why::RootNpm)
        );
        // the fix comes from the plan's prerequisite
        let pl = agentinstall::plan(ag("codex"), &probe_of(BARE));
        let fix = Why::NoNode.fix(ag("codex"), "box3", Some(&pl)).unwrap();
        assert!(fix.contains("nvm install --lts"), "{fix}");
    }

    #[test]
    fn installer_output_is_classified() {
        assert_eq!(
            classify("npm error code ENOTFOUND\nnpm error network request to https://registry.npmjs.org/x failed"),
            Some(Why::Network)
        );
        assert_eq!(
            classify("curl: (6) Could not resolve host: cursor.com"),
            Some(Why::Network)
        );
        assert_eq!(
            classify("npm error EACCES: permission denied, mkdir '/usr/lib/node_modules'"),
            Some(Why::Permission)
        );
        assert_eq!(classify("CCSUDO /tmp/x/sudo npm i -g x"), Some(Why::Sudo));
        assert_eq!(
            classify("/usr/bin/env: 'node': No such file or directory"),
            Some(Why::NoNode)
        );
        assert_eq!(classify("sh: 1: npm: not found"), Some(Why::NoNpm));
        assert_eq!(
            classify("write error: No space left on device"),
            Some(Why::Disk)
        );
        assert_eq!(classify("added 1 package in 1s"), None);
        // every reason reads as a sentence and has a code and a fix
        let a = agents::find("codex").unwrap();
        for w in [
            Why::NoNode,
            Why::NoNpm,
            Why::Needs("uv".into()),
            Why::RootNpm,
            Why::Manual("https://x".into()),
            Why::Network,
            Why::Disk,
            Why::Timeout,
            Why::Connection("closed".into()),
            Why::Exit(1),
            Why::Missing,
            Why::Broken("segfault".into()),
        ] {
            assert!(!w.text(a, "box3").is_empty() && !w.code().is_empty() && !w.short().is_empty());
            assert!(w.fix(a, "box3", None).is_some(), "{w:?}");
        }
    }

    #[test]
    fn validation_is_parsed() {
        let c = parse_check(
            "junk\nCCV\tlogin\t/home/t/.local/bin/claude\nCCV\tshell\t\nCCV\trun\t/home/t/.local/bin/claude\t0\t2.1.200 (Claude Code) \n",
        );
        assert_eq!(c.login.as_deref(), Some("/home/t/.local/bin/claude"));
        assert_eq!(c.shell, None);
        assert_eq!(
            c.ran,
            Some((
                "/home/t/.local/bin/claude".into(),
                0,
                "2.1.200 (Claude Code)".into()
            ))
        );
        let c = parse_check("CCV\tlogin\t\nCCV\tshell\t\n");
        assert_eq!(c, Checked::default());
        // the script is POSIX sh that names the executable once, quoted
        let s = check_script("claude");
        assert!(s.contains("E=claude\n") && s.contains("--version"));
    }

    #[test]
    fn the_summary_lines_up() {
        crate::ui::c::init("dark", None);
        let p = probe_of(BARE);
        let ag = |id| agents::find(id).unwrap();
        let res = vec![
            (
                ag("claude"),
                Outcome::Installed {
                    version: "2.1.200".into(),
                    path: "/home/t/.local/bin/claude".into(),
                    note: None,
                },
            ),
            (
                ag("codex"),
                Outcome::Failed {
                    why: Why::NoNode,
                    fix: Some("install Node.js".into()),
                    tail: vec![],
                    log: None,
                },
            ),
            (ag("gemini"), Outcome::Skipped("not now".into())),
        ];
        let text: Vec<String> = summary("box3", &res, None, &p)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert!(
            text[0].contains("Agents on box3  1 installed · 1 failed · 1 skipped"),
            "{text:?}"
        );
        assert!(
            text[1].contains("✓")
                && text[1].contains("Claude Code")
                && text[1].contains("~/.local/bin/claude")
        );
        assert!(
            text[2].contains("✗")
                && text[2].contains("needs Node.js")
                && text[2].contains("install Node.js")
        );
        assert!(text[3].contains("○") && text[3].contains("skipped"));
        // the columns line up
        let col = |s: &str, w: &str| s.find(w).unwrap();
        assert_eq!(col(&text[1], "2.1.200"), col(&text[2], "needs Node.js"));
        // Node.js, when it was needed, comes first
        let node = NodeRes::Installed {
            version: "v24.11.1".into(),
            path: "/home/t/.local/bin/node".into(),
            npm: "11.6.1".into(),
        };
        let with: Vec<String> = summary("box3", &res, Some(&node), &p)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert!(
            with[1].contains("Node.js") && with[1].contains("v24.11.1 (LTS)"),
            "{with:?}"
        );
        let r = Report {
            host: "box3".into(),
            results: res,
            node: Some(node),
            probe: p,
            declined: false,
        };
        assert!(!r.ok());
        let j = r.to_json();
        assert_eq!(j["results"][1]["why"], "no_node");
        assert_eq!(j["node"]["version"], "v24.11.1");
        assert_eq!(j["results"][0]["status"], "installed");
    }
}
