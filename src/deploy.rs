//! `aleph daemon deploy <host> [--build host|cross] [--service] [--config copy|link|none]
//! [--with-secrets] [--src DIR] [--dry-run] [--yes]`
//!
//! A daemon of its own on another machine (a GPU box, a VPS), so the hub and
//! notifications carry on while this computer sleeps:
//!
//! 1. look at it (one read-only `sh -s`): OS and arch, cargo, an aleph
//!    already there, systemd and linger;
//! 2. build there (`--build host`, the default when it has cargo): this
//!    source as `git stash create` captures it (uncommitted changes too, the
//!    worktree untouched; else HEAD) goes over with `git archive`, then
//!    `nice -n 10 cargo build --release --locked` in `~/.cache/aleph/src`,
//!    cargo's progress on a spinner. Or build here (`--build cross`):
//!    `cargo zigbuild --release --target <arch>-unknown-linux-musl`, then the
//!    binary goes over ssh. Missing zig or cargo-zigbuild is explained, and
//!    nothing is installed for you;
//! 3. install it as `~/.local/bin/aleph`: a temp file, then `mv`;
//! 4. the settings: `copy` (this computer's, secrets left out unless
//!    `--with-secrets`; the machine's own name becomes `local` there; its old
//!    file kept as a backup), `link` (what to set, printed) or `none`;
//! 5. `--service`: `aleph daemon install` there, and linger — the command
//!    that keeps it running after logout is shown, never run;
//! 6. `aleph daemon status` there, and the one-hub rule: with daemons in two
//!    places, `hub.machine` decides which one runs automation (offered).
//!
//! Every ssh goes through `probe::sh` / `sshcfg::shell` (BatchMode, timeouts).
//! A link that drops mid-build says so; running it again carries on where
//! cargo stopped. `state/deploys.json` remembers what went where, for
//! `aleph update` and the doctor.

use crate::cli::Args;
use crate::config::{expand, state_dir};
use crate::configcmd::is_secret;
use crate::out::*;
use crate::probe;
use crate::proc::{self, sh_quote};
use crate::sshcfg;
use crate::ui;
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const T_LOOK: Duration = Duration::from_secs(30);
const T_UPLOAD: Duration = Duration::from_secs(600);
const T_BUILD: Duration = Duration::from_secs(45 * 60);
const T_STEP: Duration = Duration::from_secs(120);
/// Where the source goes on the machine (the build cache stays between deploys).
pub const SRC: &str = "$HOME/.cache/aleph/src";
/// Where the binary goes.
pub const BIN: &str = "$HOME/.local/bin/aleph";
/// A string only our binary holds (its launchd label): how a remote `aleph`
/// is told apart from another program of that name before anything runs it.
const MARK: &str = "dev.aleph.daemon";
/// For remote commands that only look: a first run there must not move a
/// claude-herdr setup over behind your back.
const LOOK: &str = "ALEPH_NO_MIGRATE=1";
/// `systemctl --user` over ssh wants the user's runtime dir.
const RUNTIME: &str = "[ -n \"${XDG_RUNTIME_DIR:-}\" ] || { d=/run/user/$(id -u); [ -d \"$d\" ] && export XDG_RUNTIME_DIR=\"$d\"; }";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    Host,
    Cross,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigMode {
    Copy,
    Link,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Opts {
    pub host: String,
    /// None: host when the machine has cargo, else cross
    pub build: Option<Build>,
    pub service: bool,
    /// None: copy when the machine has no settings yet, else keep them
    pub config: Option<ConfigMode>,
    pub with_secrets: bool,
    pub dry: bool,
    pub yes: bool,
    pub src: Option<String>,
}

/// (pure) The command line after `deploy`.
pub fn parse_opts(rest: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        host: String::new(),
        build: None,
        service: false,
        config: None,
        with_secrets: false,
        dry: false,
        yes: false,
        src: None,
    };
    let mut i = 0;
    let mut words = Vec::new();
    while i < rest.len() {
        let a = rest[i].as_str();
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a, None),
        };
        let value = |i: &mut usize| -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            *i += 1;
            rest.get(*i)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--build" => {
                o.build = Some(match value(&mut i)?.as_str() {
                    "host" => Build::Host,
                    "cross" => Build::Cross,
                    v => return Err(format!("--build {v}: host or cross")),
                })
            }
            "--config" => {
                o.config = Some(match value(&mut i)?.as_str() {
                    "copy" => ConfigMode::Copy,
                    "link" => ConfigMode::Link,
                    "none" => ConfigMode::None,
                    v => return Err(format!("--config {v}: copy, link or none")),
                })
            }
            "--src" => o.src = Some(value(&mut i)?),
            "--service" => o.service = true,
            "--with-secrets" => o.with_secrets = true,
            "--dry-run" => o.dry = true,
            "--yes" | "-y" => o.yes = true,
            "deploy" if words.is_empty() && o.host.is_empty() => {}
            f if f.starts_with('-') => return Err(format!("unknown flag {f}")),
            w => words.push(w.to_string()),
        }
        i += 1;
    }
    match words.as_slice() {
        [h] => o.host = h.clone(),
        [] => return Err("which machine? aleph daemon deploy <host>".into()),
        _ => return Err(format!("one machine at a time ({})", words.join(", "))),
    }
    if sshcfg::is_local(&o.host) {
        return Err("that's this computer — aleph daemon install runs the daemon here".into());
    }
    Ok(o)
}

// ------------------------------------------------------------------ the machine

/// What the machine is and has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Remote {
    /// `uname -s`: Linux, Darwin
    pub os: String,
    /// `uname -m`: x86_64, aarch64, arm64
    pub arch: String,
    pub home: String,
    pub user: String,
    /// `hostname -s`, lower case (what `hub.machine` compares)
    pub hostname: String,
    /// (path, `cargo --version`)
    pub cargo: Option<(String, String)>,
    /// an aleph there already: (path, its version)
    pub aleph: Option<(String, Option<String>)>,
    /// another program called aleph on its PATH (TeX's), never run
    pub other: Option<String>,
    /// `systemctl --user is-system-running` (None: no systemctl)
    pub systemd: Option<String>,
    pub linger: Option<bool>,
    /// it has settings of its own already
    pub config: bool,
    /// a build cache from an earlier deploy
    pub warm: bool,
    /// claude-herdr's files are there (the first aleph moves them over)
    pub legacy: bool,
}

/// The read-only look at a machine.
pub fn look_script() -> String {
    format!(
        r#"{RUNTIME}
printf 'ALEPH_OS\t%s\n' "$(uname -s 2>/dev/null)"
printf 'ALEPH_ARCH\t%s\n' "$(uname -m 2>/dev/null)"
printf 'ALEPH_HOME\t%s\n' "$HOME"
printf 'ALEPH_USER\t%s\n' "$(id -un 2>/dev/null)"
h=$(hostname -s 2>/dev/null) || h=$(uname -n 2>/dev/null | cut -d. -f1)
printf 'ALEPH_HOSTNAME\t%s\n' "$h"
c=$(command -v cargo 2>/dev/null) && printf 'ALEPH_CARGO\t%s\t%s\n' "$c" "$(cargo --version 2>/dev/null | head -n 1)"
for b in "{BIN}" "$(command -v aleph 2>/dev/null)"; do
  [ -n "$b" ] && [ -x "$b" ] || continue
  # only ours is run (another program can be called aleph: TeX's is)
  if LC_ALL=C grep -q {MARK} "$b" 2>/dev/null; then printf 'ALEPH_BIN\t%s\t%s\n' "$b" "$({LOOK} "$b" version --json </dev/null 2>/dev/null | tr -d '\n' | head -c 600)"; break; fi
  printf 'ALEPH_OTHER\t%s\n' "$b"
done
command -v systemctl >/dev/null 2>&1 && printf 'ALEPH_SYSTEMD\t%s\n' "$(systemctl --user is-system-running 2>&1 | head -n 1)"
command -v loginctl >/dev/null 2>&1 && printf 'ALEPH_LINGER\t%s\n' "$(loginctl show-user "$(id -un)" -p Linger 2>/dev/null)"
[ -f "${{XDG_CONFIG_HOME:-$HOME/.config}}/aleph/aleph.toml" ] && printf 'ALEPH_CONFIG\tyes\n'
[ -d "{SRC}/target" ] && printf 'ALEPH_WARM\tyes\n'
{{ [ -d "$HOME/.config/claude-herdr" ] || [ -f "${{XDG_CONFIG_HOME:-$HOME/.config}}/systemd/user/claude-herdr-watch.service" ]; }} && printf 'ALEPH_LEGACY\tyes\n'
printf 'ALEPH_END\n'
"#
    )
}

/// (pure) `aleph version --json`, or the banner of an older aleph.
pub fn parse_version(text: &str) -> Option<String> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return v.get("version").and_then(Value::as_str).map(String::from);
    }
    t.split_whitespace()
        .find(|w| {
            w.starts_with(|c: char| c.is_ascii_digit())
                && w.contains('.')
                && w.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
        .map(String::from)
}

/// (pure) What [`look_script`] printed.
pub fn parse_remote(text: &str) -> Result<Remote, String> {
    let mut r = Remote::default();
    let mut end = false;
    for l in text.lines() {
        let mut f = l.split('\t');
        let k = f.next().unwrap_or("");
        let a = f.next().unwrap_or("").trim().to_string();
        let b = f.next().unwrap_or("").trim().to_string();
        match k {
            "ALEPH_OS" => r.os = a,
            "ALEPH_ARCH" => r.arch = a,
            "ALEPH_HOME" => r.home = a,
            "ALEPH_USER" => r.user = a,
            "ALEPH_HOSTNAME" => r.hostname = a.to_lowercase(),
            "ALEPH_CARGO" if !a.is_empty() => r.cargo = Some((a, b)),
            "ALEPH_BIN" if !a.is_empty() => r.aleph = Some((a, parse_version(&b))),
            "ALEPH_OTHER" if !a.is_empty() => r.other = Some(a),
            "ALEPH_SYSTEMD" => r.systemd = Some(a),
            "ALEPH_LINGER" => r.linger = crate::service::parse_linger(&a),
            "ALEPH_CONFIG" => r.config = true,
            "ALEPH_WARM" => r.warm = true,
            "ALEPH_LEGACY" => r.legacy = true,
            "ALEPH_END" => end = true,
            _ => {}
        }
    }
    if !end || r.os.is_empty() {
        return Err("the machine's answer was cut short".into());
    }
    Ok(r)
}

impl Remote {
    pub fn is_linux(&self) -> bool {
        self.os == "Linux"
    }
    /// systemd answers for this user there.
    pub fn has_systemd(&self) -> bool {
        self.systemd.as_deref().is_some_and(|s| {
            matches!(
                s,
                "running" | "degraded" | "starting" | "initializing" | "maintenance"
            )
        })
    }
}

/// (pure) The Rust target for a Linux machine's `uname -m`.
pub fn musl_triple(arch: &str) -> Option<&'static str> {
    match arch.trim() {
        "x86_64" | "amd64" => Some("x86_64-unknown-linux-musl"),
        "aarch64" | "arm64" | "armv8l" => Some("aarch64-unknown-linux-musl"),
        _ => None,
    }
}

// ------------------------------------------------------------------ here

/// Tools a cross build needs on this computer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Local {
    pub zigbuild: bool,
    pub zig: bool,
    /// `rustup target list --installed` (None: no rustup)
    pub targets: Option<Vec<String>>,
}

pub fn local_tools() -> Local {
    let targets = proc::which("rustup").map(|_| {
        proc::cap(
            "rustup",
            &["target", "list", "--installed"],
            Duration::from_secs(20),
        )
        .text()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
    });
    Local {
        zigbuild: proc::which("cargo-zigbuild").is_some(),
        zig: proc::which("zig").is_some(),
        targets,
    }
}

/// (pure) How to get what a cross build needs (never run for you).
pub fn cross_needs(l: &Local, triple: &str) -> Vec<String> {
    let mut v = Vec::new();
    if !l.zig {
        v.push(if cfg!(target_os = "macos") {
            "zig: brew install zig   (or pip install ziglang)".to_string()
        } else {
            "zig: https://ziglang.org/download — or pip install ziglang".to_string()
        });
    }
    if !l.zigbuild {
        v.push("cargo-zigbuild: cargo install --locked cargo-zigbuild".into());
    }
    if let Some(t) = &l.targets {
        if !t.iter().any(|x| x == triple) {
            v.push(format!("the Rust target: rustup target add {triple}"));
        }
    }
    v
}

/// The checkout of aleph to build: `--src`, the git checkout you're in,
/// or the one this binary was built from.
pub fn source_dir(flag: Option<&str>) -> Result<PathBuf, String> {
    let ours = |d: &Path| {
        std::fs::read_to_string(d.join("Cargo.toml")).is_ok_and(|t| t.contains("name = \"aleph\""))
            && d.join("src/main.rs").is_file()
    };
    if let Some(f) = flag {
        let p = expand(f);
        return if ours(&p) {
            Ok(p)
        } else {
            Err(format!(
                "{f} isn't a checkout of aleph (no Cargo.toml for it there)"
            ))
        };
    }
    let top = proc::cap(
        "git",
        &["rev-parse", "--show-toplevel"],
        Duration::from_secs(10),
    );
    if top.ok() {
        let p = PathBuf::from(top.text().trim());
        if ours(&p) {
            return Ok(p);
        }
    }
    // a build from your own checkout (install.sh) remembers where it is; a
    // release names no folder of the computer that built it
    if let Some(m) = option_env!("ALEPH_SRC").map(PathBuf::from) {
        if ours(&m) {
            return Ok(m);
        }
    }
    Err("no checkout of aleph to build from — run it from yours, or pass --src DIR".into())
}

/// (pure) The `version = "…"` of a Cargo.toml's package.
pub fn crate_version(toml: &str) -> Option<String> {
    let mut in_pkg = false;
    for l in toml.lines().map(str::trim) {
        if l.starts_with('[') {
            in_pkg = l == "[package]";
            continue;
        }
        if in_pkg {
            if let Some(v) = l.strip_prefix("version") {
                let v = v.trim_start().strip_prefix('=')?.trim().trim_matches('"');
                return Some(v.to_string());
            }
        }
    }
    None
}

fn git(src: &Path, args: &[&str], t: Duration) -> proc::Out {
    let mut c = std::process::Command::new("git");
    c.arg("-C").arg(src).args(args);
    proc::run(&mut c, t, None).unwrap_or_else(|e| proc::Out {
        code: None,
        stderr: format!("git: {e}"),
        ..Default::default()
    })
}

// ------------------------------------------------------------------ the plan

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub build: Build,
    /// cross: the target
    pub triple: Option<String>,
    /// the service can be installed there
    pub service: bool,
    /// worth saying
    pub notes: Vec<String>,
}

/// (pure) Host or cross, and whether each step can happen.
pub fn plan(r: &Remote, o: &Opts, l: &Local) -> Result<Plan, String> {
    let build = o.build.unwrap_or(if r.cargo.is_some() {
        Build::Host
    } else {
        Build::Cross
    });
    let mut p = Plan {
        build,
        triple: None,
        service: o.service,
        notes: Vec::new(),
    };
    match build {
        Build::Host => {
            if r.cargo.is_none() {
                return Err(format!(
                    "{} has no cargo — --build cross builds it here instead (or install Rust there: https://rustup.rs)",
                    o.host
                ));
            }
        }
        Build::Cross => {
            if !r.is_linux() {
                return Err(format!(
                    "cross builds are for Linux machines, and {} runs {} — use --build host",
                    o.host, r.os
                ));
            }
            let Some(t) = musl_triple(&r.arch) else {
                return Err(format!(
                    "no Linux (musl) target for {}'s {} — use --build host",
                    o.host, r.arch
                ));
            };
            let need = cross_needs(l, t);
            if !need.is_empty() {
                return Err(format!(
                    "a cross build needs these here first (Aleph installs nothing for you):\n  {}",
                    need.join("\n  ")
                ));
            }
            p.triple = Some(t.to_string());
        }
    }
    if o.service && r.is_linux() && !r.has_systemd() {
        p.service = false;
        p.notes.push(format!(
            "{} has no systemd for your user ({}): no service there — run `aleph daemon run` under your own supervisor",
            o.host,
            r.systemd.as_deref().unwrap_or("no systemctl")
        ));
    }
    if r.legacy {
        p.notes.push(format!(
            "claude-herdr's files are on {}: the first aleph run there moves them over (its watch service too)",
            o.host
        ));
    }
    Ok(p)
}

// ------------------------------------------------------------------ the settings

/// The settings a machine gets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemoteConfig {
    /// flat settings, as they'll be written
    pub entries: Vec<(String, String)>,
    /// secrets that stay here
    pub left_out: Vec<String>,
    /// `secret:NAME` references: (setting, name)
    pub refs: Vec<(String, String)>,
    /// what changed on the way, said in the plan
    pub changes: Vec<String>,
}

impl RemoteConfig {
    pub fn toml(&self) -> String {
        crate::tomlcfg::render(&self.entries)
    }
}

/// (pure) This computer's settings, as `host` should have them: secrets out
/// (unless `with_secrets`), and its own name becoming `local` there (a
/// machine reads its own herdr directly; its name here may not exist there).
pub fn remote_config(entries: &[(String, String)], host: &str, with_secrets: bool) -> RemoteConfig {
    let mut rc = RemoteConfig::default();
    let mut moved = false;
    for (k, v) in entries {
        if let Some(name) = v.strip_prefix("secret:") {
            rc.refs.push((k.clone(), name.to_string()));
            rc.entries.push((k.clone(), v.clone()));
            continue;
        }
        if is_secret(k) && !with_secrets {
            rc.left_out.push(k.clone());
            continue;
        }
        match k.as_str() {
            "hosts" => {
                let list: Vec<String> = v
                    .split(',')
                    .map(str::trim)
                    .filter(|h| !h.is_empty())
                    .map(|h| {
                        if h == host {
                            moved = true;
                            crate::sshcfg::LOCAL.to_string()
                        } else {
                            h.to_string()
                        }
                    })
                    .collect();
                rc.entries.push((k.clone(), list.join(",")));
            }
            "ssh_host" if v == host => {
                moved = true;
                rc.entries
                    .push((k.clone(), crate::sshcfg::LOCAL.to_string()));
            }
            // the herdr machine label for the default host: that's local there
            "machine" if entries.iter().any(|(x, y)| x == "ssh_host" && y == host) => {}
            "local" => {}
            _ => rc.entries.push((k.clone(), v.clone())),
        }
    }
    if moved {
        rc.changes.push(format!(
            "{host} becomes `local` there: the daemon reads its own herdr directly"
        ));
    }
    rc.entries.push(("local".into(), "true".into()));
    rc
}

/// (pure) What happens to the settings: `--config`, else copy when the
/// machine has none yet and keep what it has otherwise (a redeploy that
/// only brings a new binary mustn't undo what was set there).
pub fn config_mode(asked: Option<ConfigMode>, has_own: bool) -> ConfigMode {
    asked.unwrap_or(if has_own {
        ConfigMode::None
    } else {
        ConfigMode::Copy
    })
}

// ------------------------------------------------------------------ one hub

#[derive(Clone, Debug, PartialEq)]
pub enum HubAdvice {
    /// nothing to change
    Fine(String),
    /// worth asking: set hub.machine to the machine
    Ask(String),
    /// set to something that's neither
    Odd(String),
}

/// (pure) `hub.machine` with a daemon on `remote` too. `local_daemon`: this
/// computer runs one.
pub fn hub_advice(hub: &str, this: &str, remote: &str, local_daemon: bool) -> HubAdvice {
    let h = hub.trim().to_lowercase();
    if h == remote {
        return HubAdvice::Fine(format!("{remote} runs automation (hub.machine)"));
    }
    if !h.is_empty() && h == this {
        return HubAdvice::Fine(format!(
            "this computer runs automation (hub.machine = {h}); {remote}'s daemon only watches"
        ));
    }
    if h.is_empty() || h == "local" {
        return HubAdvice::Ask(if local_daemon {
            format!("this computer runs a daemon too: with hub.machine {} both would run automation — double notifications, double approvals", if h.is_empty() { "empty" } else { "= local" })
        } else {
            format!("with hub.machine {}, {remote}'s daemon and this computer's dashboard (while it's open) would both run automation", if h.is_empty() { "empty" } else { "= local" })
        });
    }
    HubAdvice::Odd(format!(
        "hub.machine = {h} is neither this computer ({this}) nor {remote}: automation runs there only"
    ))
}

// ------------------------------------------------------------------ deploys.json

fn deploys_path() -> PathBuf {
    state_dir().join("deploys.json")
}

/// Every machine a daemon was deployed to: host → {version, rev, build, at, hostname, service}.
pub fn deploys() -> serde_json::Map<String, Value> {
    std::fs::read_to_string(deploys_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn remember(host: &str, v: Value) {
    let mut m = deploys();
    m.insert(host.to_string(), v);
    let _ = crate::config::write_atomic(&deploys_path(), Value::Object(m).to_string().as_bytes());
}

/// The aleph version on each machine (`version --json`, in parallel).
pub fn remote_versions(hosts: &[String]) -> Vec<(String, Result<String, String>)> {
    // only an aleph that's ours is run (TeX has one too)
    let script = format!(
        "for b in \"{BIN}\" \"$(command -v aleph 2>/dev/null)\"; do [ -x \"$b\" ] && LC_ALL=C grep -q {MARK} \"$b\" 2>/dev/null && {{ {LOOK} \"$b\" version --json </dev/null 2>/dev/null; exit; }}; done; exit 1"
    );
    let handles: Vec<_> = hosts
        .iter()
        .map(|h| {
            let (h, s) = (h.clone(), script.clone());
            std::thread::spawn(move || {
                let r = probe::sh(&h, &s, Duration::from_secs(20)).and_then(|o| {
                    parse_version(&o.text()).ok_or_else(|| {
                        if o.ok() {
                            "no aleph there".to_string()
                        } else {
                            o.reason()
                        }
                    })
                });
                (h, r)
            })
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

/// (pure) A deployed daemon's version next to this one: `same`, `behind`,
/// `ahead` or `unreachable`.
pub fn compare(ours: &str, theirs: &Result<String, String>) -> &'static str {
    match theirs {
        Err(_) => "unreachable",
        Ok(v) => match crate::update::cmp_versions(v, ours) {
            std::cmp::Ordering::Equal => "same",
            std::cmp::Ordering::Less => "behind",
            std::cmp::Ordering::Greater => "ahead",
        },
    }
}

// ------------------------------------------------------------------ inline scripts

/// A one-line POSIX sh script for `ssh host <cmd>` whose stdin carries data:
/// `sh -c '…'`, so sh runs it whatever the login shell is (fish too). The
/// script itself has no `'` or `!`.
pub fn sh_line(script: &str) -> String {
    debug_assert!(!script.contains('\'') && !script.contains('!'), "{script}");
    format!("sh -c '{script}'")
}

/// The source (a tar.gz on stdin) into `~/.cache/aleph/src`, swapped in
/// whole; the build cache (`target/`) moves over to the new tree.
pub fn upload_script(rev: &str) -> String {
    let rev: String = rev.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    sh_line(&format!(
        "D=\"{SRC}\"; N=\"$D.new.$$\"; rm -rf \"$N\" && mkdir -p \"$N\" && tar -xzf - -C \"$N\" && {{ test -d \"$D/target\" && mv \"$D/target\" \"$N/target\"; true; }} && rm -rf \"$D\" && mv \"$N\" \"$D\" && echo {rev} > \"$D/.aleph-rev\" && echo ALEPH_UPLOADED"
    ))
}

/// The binary (on stdin) to `~/.local/bin/aleph`: a temp file, then mv.
pub fn put_script() -> String {
    sh_line(&format!(
        "mkdir -p \"$HOME/.local/bin\" || exit 1; T=\"$HOME/.local/bin/.aleph.new.$$\"; cat > \"$T\" && chmod 755 \"$T\" && mv -f \"$T\" \"{BIN}\" || {{ rm -f \"$T\"; exit 1; }}; {LOOK} \"{BIN}\" version --json"
    ))
}

/// The settings (TOML on stdin) to `~/.config/aleph/aleph.toml`, the old file
/// kept as `aleph.toml.bak-<time>`; prints where they went.
pub fn write_config_script() -> String {
    sh_line(
        "C=\"${XDG_CONFIG_HOME:-$HOME/.config}/aleph\"; mkdir -p \"$C\" && chmod 700 \"$C\" && umask 077 && { test -f \"$C/aleph.toml\" && cp -p \"$C/aleph.toml\" \"$C/aleph.toml.bak-$(date +%Y%m%d-%H%M%S)\"; true; } && cat > \"$C/aleph.toml.new\" && mv -f \"$C/aleph.toml.new\" \"$C/aleph.toml\" && echo \"$C/aleph.toml\"",
    )
}

/// A secret's value (on stdin) into the machine's own secret store.
pub fn secret_script(name: &str) -> Option<String> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    ok.then(|| sh_line(&format!("{LOOK} \"{BIN}\" secret set {name} -")))
}

// ------------------------------------------------------------------ the command

fn row(label: &str, text: impl Into<String>) {
    say(Line::from(vec![
        Span::styled(format!("  {}", ui::pad(label, 9)), ui::faint()),
        Span::raw(" "),
        Span::raw(text.into()),
    ]));
}

fn note(text: &str) {
    say(Line::from(Span::styled(
        format!("  {} {text}", " ".repeat(9)),
        ui::muted(),
    )));
}

/// An ssh link that died under us (rather than a command that failed).
fn dropped(o: &proc::Out) -> bool {
    o.timed_out
        || (o.code == Some(255)
            && [
                "Connection",
                "connection",
                "Broken pipe",
                "timed out",
                "closed by remote host",
            ]
            .iter()
            .any(|w| o.stderr.contains(w)))
}

fn link_dropped(host: &str, what: &str) -> ExitCode {
    err(&format!(
        "the connection to {host} dropped during the {what}"
    ));
    hint("run the same command again: it carries on where it stopped (cargo keeps its build cache there)");
    ExitCode::from(1)
}

/// `aleph daemon deploy …`.
pub fn cmd(a: Args) -> ExitCode {
    let o = match parse_opts(a.rest) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let cfg = a.cfg;
    let src = match source_dir(o.src.as_deref()) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let version = std::fs::read_to_string(src.join("Cargo.toml"))
        .ok()
        .and_then(|t| crate_version(&t))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
    let host = o.host.clone();

    // 1. look
    let sp = Spinner::start(format!("Looking at {host}"));
    let look = probe::sh(&host, &look_script(), T_LOOK);
    sp.stop();
    let r = match look {
        Ok(out) if out.ok() => match parse_remote(&out.text()) {
            Ok(r) => r,
            Err(e) => return fail(&format!("{host}: {e}")),
        },
        Ok(out) => {
            err(&format!("{host}: {}", out.reason()));
            hint(&format!(
                "aleph doctor --host {host} shows what's wrong with the connection"
            ));
            return ExitCode::from(1);
        }
        Err(e) => return fail(&format!("{host}: {e}")),
    };
    let cross = match o.build {
        Some(Build::Cross) => true,
        Some(Build::Host) => false,
        None => r.cargo.is_none(),
    };
    let local = if cross {
        local_tools()
    } else {
        Local::default()
    };
    let p = match plan(&r, &o, &local) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let dirty = git(
        &src,
        &["status", "--porcelain", "--untracked-files=no"],
        Duration::from_secs(20),
    )
    .text()
    .lines()
    .count();
    // git stash create leaves untracked files out: say which stay behind
    let untracked: Vec<String> = git(
        &src,
        &["ls-files", "--others", "--exclude-standard"],
        Duration::from_secs(20),
    )
    .text()
    .lines()
    .map(String::from)
    .collect();
    let mode = config_mode(o.config, r.config);
    let local_daemon = crate::daemonctl::ping().is_some()
        || crate::service::Spec::daemon(&crate::service::exe())
            .file(crate::service::Init::here())
            .exists();
    let hub = hub_advice(
        &cfg.get("hub.machine").unwrap_or_default(),
        &crate::auto::this_machine(),
        &r.hostname,
        local_daemon,
    );
    let rc0 = remote_config(&cfg.entries(), &host, o.with_secrets);

    // the plan
    head(&format!(
        "Deploy aleph {version} to {host} — {} {}, as {}",
        r.os, r.arch, r.user
    ));
    match p.build {
        Build::Host => {
            let (_, cv) = r.cargo.clone().unwrap_or_default();
            // `cargo 1.98.0 (abc123 2026-09-01)` → `cargo 1.98.0`
            let cv: Vec<&str> = cv.split_whitespace().take(2).collect();
            row(
                "build",
                format!(
                    "on {host} ({}){}",
                    if cv.is_empty() {
                        "cargo".into()
                    } else {
                        cv.join(" ")
                    },
                    if r.warm {
                        " · its build cache from last time"
                    } else {
                        " · the first build takes a few minutes"
                    }
                ),
            );
            row(
                "source",
                format!(
                    "{}{}",
                    crate::doctor::tilde(&src),
                    if dirty > 0 {
                        format!(" · {dirty} changed file{} included (git stash create; your worktree stays as it is)", if dirty == 1 { "" } else { "s" })
                    } else {
                        " · HEAD".into()
                    }
                ),
            );
            row(
                "upload",
                format!("git archive | ssh {host} → ~/.cache/aleph/src"),
            );
            row(
                "compile",
                "nice -n 10 cargo build --release --locked, there",
            );
        }
        Build::Cross => {
            let t = p.triple.clone().unwrap_or_default();
            row(
                "build",
                format!("here: cargo zigbuild --release --locked --target {t}"),
            );
            row(
                "source",
                format!("{} (its working tree)", crate::doctor::tilde(&src)),
            );
            row("upload", format!("the binary, over ssh to {host}"));
        }
    }
    row(
        "install",
        format!(
            "~/.local/bin/aleph (a temp file, then mv){}",
            match (&r.aleph, &r.other) {
                (Some((_, Some(v))), _) => format!(" · replaces aleph {v}"),
                (Some((path, None)), _) => format!(" · replaces {path}"),
                // another program's aleph stays; the service runs ours by its full path
                (None, Some(o)) => format!(
                    " · {o} is another program (left alone; ~/.local/bin comes first in a login shell)"
                ),
                (None, None) => String::new(),
            }
        ),
    );
    match mode {
        ConfigMode::Copy => {
            row(
                "settings",
                format!(
                    "this computer's → ~/.config/aleph/aleph.toml there{}",
                    if r.config {
                        " (its own kept as a backup)"
                    } else {
                        ""
                    }
                ),
            );
            for c in &rc0.changes {
                note(c);
            }
            if !rc0.left_out.is_empty() {
                note(&format!(
                    "left out (secrets): {} — --with-secrets copies them",
                    rc0.left_out.join(", ")
                ));
            }
            if !rc0.refs.is_empty() {
                note(&format!(
                    "{}: {}",
                    rc0.refs
                        .iter()
                        .map(|(_, n)| format!("secret:{n}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if o.with_secrets {
                        "copied into its secret store (over stdin)"
                    } else {
                        "references only — --with-secrets copies the values into its secret store"
                    }
                ));
            }
        }
        ConfigMode::Link => row("settings", "printed for you to set there (--config link)"),
        ConfigMode::None if o.config.is_none() => row(
            "settings",
            "it has its own: kept (--config copy replaces them with this computer's, keeping a backup)",
        ),
        ConfigMode::None => row("settings", "left as they are (--config none)"),
    }
    if p.service {
        row(
            "service",
            format!(
                "aleph daemon install there (systemd --user aleph.service){}",
                match r.linger {
                    Some(false) => " · linger is off there: you'll get the command for it",
                    _ => "",
                }
            ),
        );
    } else {
        row(
            "service",
            "none — --service installs it (systemd --user) there",
        );
    }
    row("then", format!("aleph daemon status on {host}"));
    match &hub {
        HubAdvice::Fine(m) => row("hub", m.clone()),
        HubAdvice::Ask(m) => {
            row("hub", m.clone());
            note(&format!(
                "you'll be asked to let {host} run it (hub.machine = {})",
                r.hostname
            ));
        }
        HubAdvice::Odd(m) => row("hub", m.clone()),
    }
    for n in &p.notes {
        warn(n);
    }
    if p.build == Build::Host && !untracked.is_empty() {
        warn(&format!(
            "{} untracked file{} stay{} behind (git stash create sends tracked files only; git add what the build needs): {}",
            untracked.len(),
            if untracked.len() == 1 { "" } else { "s" },
            if untracked.len() == 1 { "s" } else { "" },
            ui::trunc(&untracked.join(", "), 100)
        ));
    }
    if o.dry {
        hint("nothing done (--dry-run)");
        return ExitCode::SUCCESS;
    }
    if !o.yes {
        if !std::io::stdin().is_terminal() {
            return fail("aleph daemon deploy asks first — run it in a terminal, or pass --yes (--dry-run shows the plan)");
        }
        if !yes("Deploy?") {
            return ExitCode::from(1);
        }
    }

    // 2–3. build and install
    let (got, rev) = match p.build {
        Build::Host => match host_build(&host, &src) {
            Ok(x) => x,
            Err(code) => return code,
        },
        Build::Cross => match cross_build(&host, &src, p.triple.as_deref().unwrap_or("")) {
            Ok(x) => x,
            Err(code) => return code,
        },
    };
    match &got {
        Some(v) if *v == version => ok(&format!("{host} has aleph {v} in ~/.local/bin")),
        Some(v) => warn(&format!(
            "{host} answers aleph {v}, the source says {version}"
        )),
        None => warn(&format!(
            "installed, but ~/.local/bin/aleph on {host} didn't say its version"
        )),
    }

    // the one hub, before the settings go over (they carry hub.machine)
    let mut cfg = cfg;
    let mut set_hub_there = false;
    if let HubAdvice::Ask(why) = &hub {
        say(Line::from(Span::styled(format!("  {why}"), ui::muted())));
        if o.yes
            || yes(&format!(
                "Let {host} run automation — auto-approve, the queue, notifications (hub.machine = {} here and there)?",
                r.hostname
            ))
        {
            match cfg.set("hub.machine", &r.hostname) {
                Ok(()) => {
                    ok(&format!("hub.machine = {} here", r.hostname));
                    set_hub_there = true;
                }
                Err(e) => warn(&format!("couldn't set hub.machine: {e}")),
            }
        } else {
            hint(&format!("aleph settings set hub.machine <the hostname -s of the one that should> — here and on {host}"));
        }
    }

    // 4. settings
    match mode {
        ConfigMode::Copy => {
            let rc = remote_config(&cfg.entries(), &host, o.with_secrets);
            if let Err(code) = copy_config(&host, &r.home, &rc, o.with_secrets) {
                return code;
            }
        }
        ConfigMode::Link => {
            let rc = remote_config(&cfg.entries(), &host, false);
            print_link(&host, &rc);
            if set_hub_there {
                set_remote(&host, "hub.machine", &r.hostname);
            }
        }
        ConfigMode::None => {
            if set_hub_there {
                set_remote(&host, "hub.machine", &r.hostname);
            }
        }
    }

    // 5. the service
    if p.service {
        service_there(&host, &r, o.yes);
    }

    // 6. how it is there
    let st = status_there(&host);
    remember(
        &host,
        json!({
            "version": got.clone().unwrap_or(version.clone()),
            "rev": rev,
            "build": if p.build == Build::Host { "host" } else { "cross" },
            "at": crate::cache::now(),
            "hostname": r.hostname,
            "service": p.service,
            "path": "~/.local/bin/aleph",
        }),
    );
    if !p.service {
        hint(&format!(
            "to run it at login there: aleph daemon deploy {host} --service (or: ssh {host} ~/.local/bin/aleph daemon install)"
        ));
    }
    if st.is_none() && p.service {
        hint(&format!(
            "ssh {host} ~/.local/bin/aleph daemon logs --service shows why"
        ));
    }
    ExitCode::SUCCESS
}

/// Count of packages in a Cargo.lock (cargo's progress has no total off a tty).
fn lock_packages(src: &Path) -> usize {
    std::fs::read_to_string(src.join("Cargo.lock"))
        .map(|t| t.matches("[[package]]").count())
        .unwrap_or(0)
}

/// `cargo` output → spinner text: `37/~92 · Compiling ratatui v0.29.0`.
fn progress(n: usize, total: usize, line: &str) -> String {
    let l = ui::trunc(line.trim(), 60);
    if total > 0 {
        format!("{n}/~{total} · {l}")
    } else {
        l
    }
}

/// The source over, `cargo build` there, the binary into place: (version, rev).
fn host_build(host: &str, src: &Path) -> Result<(Option<String>, String), ExitCode> {
    // what to send: the worktree as git stash create sees it, else HEAD
    let stash = git(src, &["stash", "create"], Duration::from_secs(30));
    let mut rev = stash.text().trim().to_string();
    let dirty = !rev.is_empty();
    if rev.is_empty() {
        rev = git(src, &["rev-parse", "HEAD"], Duration::from_secs(10))
            .text()
            .trim()
            .to_string();
    }
    if rev.is_empty() {
        err(&format!(
            "couldn't read the source's git revision ({})",
            stash.reason()
        ));
        return Err(ExitCode::from(1));
    }
    let short = rev[..rev.len().min(10)].to_string();
    let sp = Spinner::start(format!("Packing the source ({short})"));
    let arch = git(
        src,
        &["archive", "--format=tar.gz", &rev],
        Duration::from_secs(120),
    );
    if !arch.ok() || arch.stdout.is_empty() {
        sp.stop();
        err(&format!("git archive: {}", arch.reason()));
        return Err(ExitCode::from(1));
    }
    sp.set(format!(
        "Sending the source to {host} ({})",
        ui::bytes(arch.stdout.len() as f64)
    ));
    let up = proc::run(
        &mut sshcfg::shell(host, &upload_script(&rev)),
        T_UPLOAD,
        Some(&arch.stdout),
    );
    sp.stop();
    match up {
        Ok(o) if o.ok() && o.text().contains("ALEPH_UPLOADED") => ok(&format!(
            "sent the source ({short}{}, {}) to {host}:~/.cache/aleph/src",
            if dirty { ", with your changes" } else { "" },
            ui::bytes(arch.stdout.len() as f64)
        )),
        Ok(o) if dropped(&o) => return Err(link_dropped(host, "upload")),
        Ok(o) => {
            err(&format!("{host}: unpacking the source: {}", o.reason()));
            return Err(ExitCode::from(1));
        }
        Err(e) => {
            err(&format!("ssh: {e}"));
            return Err(ExitCode::from(1));
        }
    }
    // build there, cargo's lines on the spinner
    let total = lock_packages(src);
    let script = format!(
        "unset CARGO_TARGET_DIR; cd \"{SRC}\" || exit 1; nice -n 10 cargo build --release --locked 2>&1"
    );
    let sp = Spinner::start(format!("Building on {host}"));
    let mut n = 0;
    let mut tail: Vec<String> = Vec::new();
    let b = probe::sh_stream(host, &script, T_BUILD, |l| {
        if l.trim_start().starts_with("Compiling ") {
            n += 1;
        }
        tail.push(l.to_string());
        if tail.len() > 40 {
            tail.remove(0);
        }
        sp.set(format!("Building on {host} · {}", progress(n, total, l)));
    });
    sp.stop();
    match b {
        Ok(o) if o.ok() => ok(&format!("built on {host} ({n} crates compiled)")),
        Ok(o) if dropped(&o) => return Err(link_dropped(host, "build")),
        Ok(o) => {
            for l in tail.iter().rev().take(15).rev() {
                say(Line::from(Span::styled(format!("    {l}"), ui::faint())));
            }
            err(&format!("cargo build failed on {host} ({})", o.reason()));
            return Err(ExitCode::from(1));
        }
        Err(e) => {
            err(&format!("ssh: {e}"));
            return Err(ExitCode::from(1));
        }
    }
    let install = format!(
        "B=\"{SRC}/target/release/aleph\"; [ -x \"$B\" ] || {{ echo \"no binary at $B\" >&2; exit 1; }}; mkdir -p \"$HOME/.local/bin\" || exit 1; T=\"$HOME/.local/bin/.aleph.new.$$\"; cp \"$B\" \"$T\" && chmod 755 \"$T\" && mv -f \"$T\" \"{BIN}\" || {{ rm -f \"$T\"; exit 1; }}; {LOOK} \"{BIN}\" version --json"
    );
    match probe::sh(host, &install, T_STEP) {
        Ok(o) if o.ok() => Ok((parse_version(&o.text()), rev)),
        Ok(o) if dropped(&o) => Err(link_dropped(host, "install")),
        Ok(o) => {
            err(&format!(
                "{host}: installing ~/.local/bin/aleph: {}",
                o.reason()
            ));
            Err(ExitCode::from(1))
        }
        Err(e) => {
            err(&format!("ssh: {e}"));
            Err(ExitCode::from(1))
        }
    }
}

/// `cargo zigbuild` here, the binary over ssh: (version, rev).
fn cross_build(host: &str, src: &Path, triple: &str) -> Result<(Option<String>, String), ExitCode> {
    let head = git(
        src,
        &["rev-parse", "--short=10", "HEAD"],
        Duration::from_secs(10),
    )
    .text()
    .trim()
    .to_string();
    let dirty = !git(
        src,
        &["status", "--porcelain", "--untracked-files=no"],
        Duration::from_secs(20),
    )
    .text()
    .trim()
    .is_empty();
    let rev = if dirty {
        format!("{head}+changes")
    } else {
        head
    };
    let total = lock_packages(src);
    let sp = Spinner::start(format!("Building for {triple} here"));
    let mut n = 0;
    let mut tail: Vec<String> = Vec::new();
    let mut c = std::process::Command::new("cargo");
    c.current_dir(src)
        .args(["zigbuild", "--release", "--locked", "--target", triple]);
    let b = proc::stream(&mut c, T_BUILD, None, |l| {
        if l.trim_start().starts_with("Compiling ") {
            n += 1;
        }
        tail.push(l.to_string());
        if tail.len() > 40 {
            tail.remove(0);
        }
        sp.set(format!("Building for {triple} · {}", progress(n, total, l)));
    });
    sp.stop();
    match b {
        Ok(o) if o.ok() => ok(&format!("built for {triple} ({n} crates compiled)")),
        Ok(o) => {
            for l in tail.iter().rev().take(15).rev() {
                say(Line::from(Span::styled(format!("    {l}"), ui::faint())));
            }
            err(&format!("cargo zigbuild failed ({})", o.reason()));
            return Err(ExitCode::from(1));
        }
        Err(e) => {
            err(&format!("cargo: {e}"));
            return Err(ExitCode::from(1));
        }
    }
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { src.join(p) })
        .unwrap_or_else(|| src.join("target"));
    let bin = target_dir
        .join(triple)
        .join("release")
        .join(crate::brand::BIN);
    let Ok(bytes) = std::fs::read(&bin) else {
        err(&format!("no binary at {}", bin.display()));
        return Err(ExitCode::from(1));
    };
    let sp = Spinner::start(format!(
        "Sending it to {host} ({})",
        ui::bytes(bytes.len() as f64)
    ));
    let r = proc::run(
        &mut sshcfg::shell(host, &put_script()),
        T_UPLOAD,
        Some(&bytes),
    );
    sp.stop();
    match r {
        Ok(o) if o.ok() => Ok((parse_version(&o.text()), rev)),
        Ok(o) if dropped(&o) => Err(link_dropped(host, "upload")),
        Ok(o) => {
            err(&format!(
                "{host}: installing ~/.local/bin/aleph: {}",
                o.reason()
            ));
            Err(ExitCode::from(1))
        }
        Err(e) => {
            err(&format!("ssh: {e}"));
            Err(ExitCode::from(1))
        }
    }
}

/// Write the settings there (the old file kept), then the secrets (stdin).
fn copy_config(
    host: &str,
    home: &str,
    rc: &RemoteConfig,
    with_secrets: bool,
) -> Result<(), ExitCode> {
    match proc::run(
        &mut sshcfg::shell(host, &write_config_script()),
        T_STEP,
        Some(rc.toml().as_bytes()),
    ) {
        Ok(o) if o.ok() => {
            let p = o.text().trim().to_string();
            let p = match p.strip_prefix(home).filter(|_| !home.is_empty()) {
                Some(r) => format!("~{r}"),
                None => p,
            };
            ok(&format!("settings → {host}:{p}"));
        }
        Ok(o) if dropped(&o) => return Err(link_dropped(host, "settings copy")),
        Ok(o) => {
            err(&format!("{host}: writing the settings: {}", o.reason()));
            return Err(ExitCode::from(1));
        }
        Err(e) => {
            err(&format!("ssh: {e}"));
            return Err(ExitCode::from(1));
        }
    }
    if with_secrets {
        for (k, name) in &rc.refs {
            let Some(v) = crate::secrets::get(name) else {
                warn(&format!(
                    "secret:{name} ({k}) isn't stored here — not copied"
                ));
                continue;
            };
            let Some(cmd) = secret_script(name) else {
                warn(&format!(
                    "secret:{name}: a name with odd characters — set it there by hand"
                ));
                continue;
            };
            match proc::run(&mut sshcfg::shell(host, &cmd), T_STEP, Some(v.as_bytes())) {
                Ok(o) if o.ok() => ok(&format!("secret:{name} → {host}'s secret store")),
                Ok(o) => warn(&format!("secret:{name}: {}", o.reason())),
                Err(e) => warn(&format!("secret:{name}: {e}")),
            }
        }
    }
    Ok(())
}

/// `--config link`: what to set there, by hand.
fn print_link(host: &str, rc: &RemoteConfig) {
    head(&format!(
        "Settings for {host} (run these there, or deploy with --config copy):"
    ));
    for (k, v) in &rc.entries {
        out(Line::from(Span::styled(
            format!("  aleph settings set {k} {}", sh_quote(v)),
            ui::muted(),
        )));
    }
    for (_, name) in &rc.refs {
        out(Line::from(Span::styled(
            format!("  aleph secret set {name}"),
            ui::muted(),
        )));
    }
}

fn set_remote(host: &str, key: &str, value: &str) {
    let cmd = format!(
        "{LOOK} \"{BIN}\" settings set {} {}",
        sh_quote(key),
        sh_quote(value)
    );
    match probe::sh(host, &cmd, T_STEP) {
        Ok(o) if o.ok() => ok(&format!("{key} = {value} on {host}")),
        Ok(o) => warn(&format!("{host}: {key}: {}", o.reason())),
        Err(e) => warn(&format!("{host}: {e}")),
    }
}

/// `aleph daemon install` there, then linger.
fn service_there(host: &str, r: &Remote, assume: bool) {
    let sp = Spinner::start(format!("Installing the service on {host}"));
    let cmd = format!("{RUNTIME}; \"{BIN}\" daemon install 2>&1");
    let res = probe::sh(host, &cmd, T_STEP);
    sp.stop();
    match res {
        Ok(o) => {
            for l in o.text().lines().filter(|l| !l.trim().is_empty()) {
                say(Line::from(Span::styled(format!("    {l}"), ui::faint())));
            }
            if o.ok() {
                ok(&format!(
                    "{host} runs the daemon at login (systemd --user aleph.service)"
                ));
            } else {
                warn(&format!("{host}: aleph daemon install: {}", o.reason()));
            }
        }
        Err(e) => warn(&format!("{host}: {e}")),
    }
    if r.linger == Some(false) {
        warn(&format!(
            "linger is off on {host}: its daemon stops whenever you log out of it"
        ));
        let user = if r.user.is_empty() { "$USER" } else { &r.user };
        let show = assume
            || yes(&format!(
                "Show the command that keeps it running (it needs you, and maybe sudo, on {host})?"
            ));
        if show {
            out(Line::from(vec![
                Span::styled("  $ ", ui::faint()),
                Span::styled(
                    format!("ssh -t {host} sudo loginctl enable-linger {user}"),
                    ui::bold(),
                ),
            ]));
            hint("Aleph never runs it for you; without sudo, `loginctl enable-linger` alone works where polkit allows it");
        }
    }
}

/// `aleph daemon status --json` there, said in a line; None when no daemon answers.
fn status_there(host: &str) -> Option<Value> {
    let cmd = format!("{RUNTIME}; {LOOK} \"{BIN}\" daemon status --json 2>/dev/null");
    let o = probe::sh(host, &cmd, Duration::from_secs(40)).ok()?;
    let v: Value = serde_json::from_slice(&o.stdout).ok()?;
    let svc = &v["service"];
    let svc_text = match (svc["installed"].as_bool(), svc["running"].as_bool()) {
        (Some(true), Some(true)) => {
            format!("service {} running", svc["name"].as_str().unwrap_or(""))
        }
        (Some(true), _) => format!("service {} not running", svc["name"].as_str().unwrap_or("")),
        _ => "no service".to_string(),
    };
    if v["running"].as_bool() == Some(true) {
        ok(&format!(
            "{host}: the daemon runs aleph {} (pid {}) · {svc_text}",
            v["version"].as_str().unwrap_or("?"),
            v["pid"].as_u64().unwrap_or(0)
        ));
        if v["version_match"].as_bool() == Some(false) {
            hint("it picks up the new binary by itself within seconds (it re-execs when its binary changes)");
        }
        Some(v)
    } else {
        say(Line::from(Span::styled(
            format!("  {host}: no daemon answers yet · {svc_text}"),
            ui::muted(),
        )));
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn linux() -> Remote {
        Remote {
            os: "Linux".into(),
            arch: "x86_64".into(),
            home: "/home/selene".into(),
            user: "selene".into(),
            hostname: "b31".into(),
            cargo: Some((
                "/home/selene/.cargo/bin/cargo".into(),
                "cargo 1.98.0".into(),
            )),
            systemd: Some("running".into()),
            linger: Some(false),
            ..Default::default()
        }
    }

    fn opts() -> Opts {
        parse_opts(&s(&["deploy", "b31"])).unwrap()
    }

    #[test]
    fn reads_the_command_line() {
        let o = parse_opts(&s(&[
            "deploy",
            "b31",
            "--build",
            "cross",
            "--service",
            "--config=link",
            "--with-secrets",
            "--dry-run",
            "--yes",
            "--src",
            "~/aleph",
        ]))
        .unwrap();
        assert_eq!(o.host, "b31");
        assert_eq!(o.build, Some(Build::Cross));
        assert!(o.service && o.with_secrets && o.dry && o.yes);
        assert_eq!(o.config, Some(ConfigMode::Link));
        assert_eq!(o.src.as_deref(), Some("~/aleph"));
        assert_eq!(opts().config, None);
        assert!(parse_opts(&s(&["deploy"]))
            .unwrap_err()
            .contains("which machine"));
        assert!(parse_opts(&s(&["deploy", "b31", "gpu2"])).is_err());
        assert!(parse_opts(&s(&["deploy", "b31", "--build", "fast"])).is_err());
        assert!(parse_opts(&s(&["deploy", "local"]))
            .unwrap_err()
            .contains("daemon install"));
        assert!(parse_opts(&s(&["deploy", "b31", "--bogus"])).is_err());
    }

    #[test]
    fn reads_the_machine() {
        let t = "junk from a login banner\nALEPH_OS\tLinux\nALEPH_ARCH\taarch64\nALEPH_HOME\t/home/n\nALEPH_USER\tn\nALEPH_HOSTNAME\tB31\nALEPH_CARGO\t/home/n/.cargo/bin/cargo\tcargo 1.98.0 (abc 2026-09-01)\nALEPH_BIN\t/home/n/.local/bin/aleph\t{\"name\":\"aleph\",\"version\":\"1.9.0\"}\nALEPH_SYSTEMD\tdegraded\nALEPH_LINGER\tLinger=yes\nALEPH_CONFIG\tyes\nALEPH_WARM\tyes\nALEPH_END\n";
        let r = parse_remote(t).unwrap();
        assert_eq!(
            (r.os.as_str(), r.arch.as_str(), r.hostname.as_str()),
            ("Linux", "aarch64", "b31")
        );
        assert_eq!(r.cargo.as_ref().unwrap().1, "cargo 1.98.0 (abc 2026-09-01)");
        assert_eq!(
            r.aleph,
            Some(("/home/n/.local/bin/aleph".into(), Some("1.9.0".into())))
        );
        assert!(r.has_systemd() && r.config && r.warm && !r.legacy);
        assert_eq!(r.linger, Some(true));
        // cut short (a dropped link)
        assert!(parse_remote("ALEPH_OS\tLinux\n").is_err());
        // no cargo, no systemd
        let r = parse_remote("ALEPH_OS\tLinux\nALEPH_ARCH\tx86_64\nALEPH_SYSTEMD\tFailed to connect to bus\nALEPH_END\n").unwrap();
        assert!(r.cargo.is_none() && !r.has_systemd());
        // TeX's aleph on the PATH: noted, never taken for ours
        let r = parse_remote("ALEPH_OS\tLinux\nALEPH_OTHER\t/usr/bin/aleph\nALEPH_END\n").unwrap();
        assert!(r.aleph.is_none());
        assert_eq!(r.other.as_deref(), Some("/usr/bin/aleph"));
        assert_eq!(
            parse_version("ℵ aleph 1.8.2\n  herdr 0.9.5"),
            Some("1.8.2".into())
        );
    }

    #[test]
    fn host_or_cross() {
        // cargo there: host
        let p = plan(&linux(), &opts(), &Local::default()).unwrap();
        assert_eq!(p.build, Build::Host);
        assert!(p.triple.is_none());
        // no cargo there: cross, for its arch
        let mut r = linux();
        r.cargo = None;
        let tools = Local {
            zigbuild: true,
            zig: true,
            targets: Some(s(&[
                "x86_64-unknown-linux-musl",
                "aarch64-unknown-linux-musl",
            ])),
        };
        let p = plan(&r, &opts(), &tools).unwrap();
        assert_eq!(p.build, Build::Cross);
        assert_eq!(p.triple.as_deref(), Some("x86_64-unknown-linux-musl"));
        r.arch = "aarch64".into();
        assert_eq!(
            plan(&r, &opts(), &tools).unwrap().triple.as_deref(),
            Some("aarch64-unknown-linux-musl")
        );
        r.arch = "riscv64".into();
        assert!(plan(&r, &opts(), &tools).unwrap_err().contains("riscv64"));
        // asked for host without cargo there
        let mut o = opts();
        o.build = Some(Build::Host);
        let mut r = linux();
        r.cargo = None;
        assert!(plan(&r, &o, &tools).unwrap_err().contains("--build cross"));
        // cross to a Mac: no
        let mut r = linux();
        r.os = "Darwin".into();
        let mut o = opts();
        o.build = Some(Build::Cross);
        assert!(plan(&r, &o, &tools).unwrap_err().contains("--build host"));
        // missing zig / cargo-zigbuild / the target: explained, never installed
        let e = plan(
            &linux(),
            &o,
            &Local {
                zigbuild: false,
                zig: false,
                targets: Some(vec![]),
            },
        )
        .unwrap_err();
        assert!(
            e.contains("zig")
                && e.contains("cargo install --locked cargo-zigbuild")
                && e.contains("rustup target add x86_64-unknown-linux-musl"),
            "{e}"
        );
        assert!(e.contains("installs nothing"));
        // no rustup: can't tell, so it doesn't ask for the target
        assert!(plan(
            &linux(),
            &o,
            &Local {
                zigbuild: true,
                zig: true,
                targets: None
            }
        )
        .is_ok());
    }

    #[test]
    fn service_needs_systemd_and_notes_legacy() {
        let mut o = opts();
        o.service = true;
        assert!(plan(&linux(), &o, &Local::default()).unwrap().service);
        let mut r = linux();
        r.systemd = Some("offline".into());
        let p = plan(&r, &o, &Local::default()).unwrap();
        assert!(!p.service && p.notes[0].contains("no systemd"));
        let mut r = linux();
        r.legacy = true;
        assert!(plan(&r, &opts(), &Local::default()).unwrap().notes[0].contains("claude-herdr"));
        assert_eq!(musl_triple("amd64"), Some("x86_64-unknown-linux-musl"));
        assert_eq!(musl_triple("mips"), None);
    }

    #[test]
    fn the_settings_a_machine_gets() {
        let e: Vec<(String, String)> = [
            ("ssh_host", "b31"),
            ("machine", "b31"),
            ("hosts", "b31,gpu2"),
            ("local", "false"),
            ("notify.ntfy", "https://ntfy.sh/secret-topic"),
            ("notify.slack", "secret:slack"),
            ("hub.machine", "b31"),
            ("theme", "dark"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let rc = remote_config(&e, "b31", false);
        let get = |k: &str| {
            rc.entries
                .iter()
                .find(|(x, _)| x == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("hosts"), Some("local,gpu2"));
        assert_eq!(get("ssh_host"), Some("local"));
        assert_eq!(get("machine"), None);
        assert_eq!(get("local"), Some("true"));
        assert_eq!(get("notify.ntfy"), None, "a secret stays here");
        assert_eq!(rc.left_out, vec!["notify.ntfy"]);
        assert_eq!(
            get("notify.slack"),
            Some("secret:slack"),
            "a reference is fine"
        );
        assert_eq!(
            rc.refs,
            vec![("notify.slack".to_string(), "slack".to_string())]
        );
        assert_eq!(get("hub.machine"), Some("b31"));
        assert_eq!(get("theme"), Some("dark"));
        assert_eq!(rc.changes.len(), 1);
        let t = rc.toml();
        assert!(t.contains("[fleet]") && !t.contains("secret-topic"), "{t}");
        // with secrets
        let rc = remote_config(&e, "b31", true);
        assert!(rc.left_out.is_empty());
        assert!(rc.toml().contains("secret-topic"));
        // another machine's name stays
        let rc = remote_config(&e, "gpu2", false);
        assert_eq!(
            rc.entries.iter().find(|(k, _)| k == "hosts").unwrap().1,
            "b31,local"
        );
        assert_eq!(
            rc.entries.iter().find(|(k, _)| k == "ssh_host").unwrap().1,
            "b31"
        );
    }

    #[test]
    fn settings_are_copied_once_unless_asked() {
        assert_eq!(config_mode(None, false), ConfigMode::Copy);
        assert_eq!(config_mode(None, true), ConfigMode::None);
        assert_eq!(config_mode(Some(ConfigMode::Copy), true), ConfigMode::Copy);
        assert_eq!(config_mode(Some(ConfigMode::Link), false), ConfigMode::Link);
    }

    #[test]
    fn one_hub() {
        assert!(
            matches!(hub_advice("", "mbp", "b31", true), HubAdvice::Ask(m) if m.contains("this computer runs a daemon too"))
        );
        assert!(
            matches!(hub_advice("", "mbp", "b31", false), HubAdvice::Ask(m) if m.contains("dashboard"))
        );
        assert!(matches!(
            hub_advice("local", "mbp", "b31", true),
            HubAdvice::Ask(_)
        ));
        assert!(
            matches!(hub_advice("B31", "mbp", "b31", true), HubAdvice::Fine(m) if m.contains("b31 runs automation"))
        );
        assert!(
            matches!(hub_advice("mbp", "mbp", "b31", true), HubAdvice::Fine(m) if m.contains("only watches"))
        );
        assert!(matches!(
            hub_advice("dex42", "mbp", "b31", true),
            HubAdvice::Odd(_)
        ));
    }

    #[test]
    fn versions_next_to_ours() {
        assert_eq!(compare("2.0.0", &Ok("2.0.0".into())), "same");
        assert_eq!(compare("2.0.0", &Ok("1.9.3".into())), "behind");
        assert_eq!(compare("2.0.0", &Ok("2.1.0".into())), "ahead");
        assert_eq!(compare("2.0.0", &Err("timed out".into())), "unreachable");
    }

    #[test]
    fn small_parsers() {
        assert_eq!(crate_version("[package]\nname = \"aleph\"\nversion = \"2.0.0\"\n[dependencies]\nversion = \"9\"\n"), Some("2.0.0".into()));
        assert_eq!(crate_version("[dependencies]\nx = \"1\"\n"), None);
        assert_eq!(
            progress(3, 90, "   Compiling serde v1.0.228"),
            "3/~90 · Compiling serde v1.0.228"
        );
        assert_eq!(progress(0, 0, "Finished"), "Finished");
        let o = proc::Out {
            code: Some(255),
            stderr: "Connection to b31 closed by remote host.".into(),
            ..Default::default()
        };
        assert!(dropped(&o));
        let o = proc::Out {
            code: Some(101),
            stderr: "error[E0425]".into(),
            ..Default::default()
        };
        assert!(!dropped(&o));
        assert!(look_script().contains("ALEPH_END"));
    }

    #[test]
    fn inline_scripts_survive_any_login_shell() {
        for s in [
            upload_script("3f2a1b9c"),
            put_script(),
            write_config_script(),
            secret_script("ntfy-main").unwrap(),
        ] {
            assert!(s.starts_with("sh -c '") && s.ends_with('\''), "{s}");
            let inner = &s[7..s.len() - 1];
            assert!(!inner.contains('\'') && !inner.contains('!'), "{inner}");
        }
        assert!(upload_script("3f2a; rm -rf /").contains("echo 3f2armrf "));
        assert!(upload_script("abc").contains("mv \"$D/target\" \"$N/target\""));
        assert!(secret_script("a b").is_none() && secret_script("x'y").is_none());
    }
}
