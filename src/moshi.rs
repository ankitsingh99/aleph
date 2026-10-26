//! Moshi, the phone terminal for coding agents, reached through its own
//! `moshi-hook` CLI.
//!
//! What it gives the phone, for every machine and agent Aleph runs:
//! * **inbox messages** (`moshi-hook notify`): needs you, limits, machines
//!   down/up, queue results, finished work;
//! * **approvals** (`moshi-hook ask`): the agent's own options become buttons.
//!   The answer goes back through herdr as the matching key, only while the
//!   same question is still waiting;
//! * **live job rows** (`moshi-hook job`) for working sessions;
//! * **plan limits** (`moshi-hook usage`) and **recent folders** (`cwd-list`)
//!   for Aleph's own views;
//! * a persistent **`aleph` tmux session**. Moshi's session picker lists it,
//!   and Aleph runs its commands from that session's context, so tapping a card
//!   opens the dashboard.
//!
//! moshi-hook already hooks some agents natively on a machine (`status --json`
//! lists them). Those sessions get their cards from moshi-hook itself, and
//! Aleph only adds what it can't see. Everything here is best effort and runs
//! with timeouts; nothing talks to Moshi's servers directly.

use crate::config::Config;
use crate::proc;
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Source id on every card (lowercase letters, digits, `._-`).
pub const SOURCE: &str = "aleph";
/// The tmux session Moshi's picker opens.
pub const TMUX_SESSION: &str = "aleph";
const T_QUICK: Duration = Duration::from_secs(8);
const T_NOTIFY: Duration = Duration::from_secs(15);
const T_STRICT: Duration = Duration::from_secs(50);

/// `moshi-hook` on this machine: PATH, then where its installers put it.
pub fn bin() -> Option<PathBuf> {
    static B: OnceLock<Option<PathBuf>> = OnceLock::new();
    B.get_or_init(|| {
        proc::which("moshi-hook").or_else(|| {
            [
                "/opt/homebrew/bin/moshi-hook",
                "/usr/local/bin/moshi-hook",
                "~/.local/bin/moshi-hook",
            ]
            .iter()
            .map(|p| crate::config::expand(p))
            .find(|p| p.is_file())
        })
    })
    .clone()
}

/// What `moshi-hook status --json` + `probe --json` say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub version: String,
    pub paired: bool,
    pub running: bool,
    pub gateway: bool,
    pub e2ee: String,
    pub host_name: String,
    /// agents moshi-hook hooks natively here, with their state (`current`, …)
    pub hooks: Vec<(String, String)>,
}

impl Status {
    /// Agent ids (Aleph's) whose sessions get native Moshi cards here.
    pub fn native(&self) -> Vec<String> {
        self.hooks
            .iter()
            .filter(|(_, st)| st == "current")
            .map(|(t, _)| native_id(t).to_string())
            .collect()
    }
    pub fn ready(&self) -> bool {
        self.paired && self.running
    }
}

/// moshi-hook's target names → Aleph's agent ids.
fn native_id(t: &str) -> &str {
    match t {
        "antigravity" => "agy",
        "qoder" => "qodercli",
        other => other,
    }
}

pub fn parse_status(status: &str, probe: &str) -> Status {
    let s: Value = serde_json::from_str(status).unwrap_or(Value::Null);
    let p: Value = serde_json::from_str(probe).unwrap_or(Value::Null);
    let st = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Status {
        version: st(&p, "version"),
        paired: s.get("paired").and_then(Value::as_bool).unwrap_or(false),
        running: p.get("running").and_then(Value::as_bool).unwrap_or(false),
        gateway: p.get("gateway").and_then(Value::as_bool).unwrap_or(false),
        e2ee: p
            .get("e2ee")
            .and_then(|e| e.get("policy"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        host_name: st(&s, "displayName"),
        hooks: s
            .get("hooks")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|h| (st(h, "target"), st(h, "status")))
                    .filter(|(t, _)| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// This machine's moshi-hook state (None = not installed). Cached for a minute.
pub fn status() -> Option<Status> {
    static C: std::sync::Mutex<Option<(Instant, Option<Status>)>> = std::sync::Mutex::new(None);
    let mut c = C.lock().unwrap();
    if let Some((at, s)) = c.as_ref() {
        if at.elapsed() < Duration::from_secs(60) {
            return s.clone();
        }
    }
    let s = bin().map(|b| {
        let b = b.to_string_lossy().into_owned();
        let st = proc::cap(&b, &["status", "--json"], T_QUICK).text();
        let pr = proc::cap(&b, &["probe", "--json"], T_QUICK).text();
        parse_status(&st, &pr)
    });
    *c = Some((Instant::now(), s.clone()));
    s
}

/// Is the bridge on? `moshi = auto` means on when moshi-hook is paired here.
pub fn enabled(cfg: &Config) -> bool {
    match cfg.get_or("moshi").as_str() {
        "off" | "false" | "0" => false,
        "on" | "true" | "1" => bin().is_some(),
        _ => status().is_some_and(|s| s.ready()),
    }
}

/// How Aleph treats one machine's sessions (`host.<a>.moshi`):
/// `full` (everything), `machine-only` (only what moshi-hook can't see: the
/// machine itself, limits, queue), `off`, or `auto` (full, except agents
/// moshi-hook hooks natively: on this computer, or on that machine when its
/// own moshi-hook does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Full,
    MachineOnly,
    Off,
}

pub fn mode(cfg: &Config, host: &str, agent: &str, native: &[String]) -> Mode {
    mode_with(cfg, host, agent, native, remote_hook)
}

/// `mode`, with "does moshi-hook on that machine hook this agent?" passed in
/// (tested).
fn mode_with(
    cfg: &Config,
    host: &str,
    agent: &str,
    native: &[String],
    hook: impl Fn(&str, &str) -> bool,
) -> Mode {
    match cfg.host_get(host, "moshi").as_deref() {
        Some("off") => Mode::Off,
        Some("machine-only") => Mode::MachineOnly,
        Some("full") => Mode::Full,
        _ if crate::sshcfg::is_local(host) && native.iter().any(|n| n == agent) => {
            Mode::MachineOnly
        }
        // that machine's own moshi-hook tells the phone about this agent;
        // Aleph sends what it can't see (down / up, limits)
        _ if !crate::sshcfg::is_local(host) && hook(host, agent) => Mode::MachineOnly,
        _ => Mode::Full,
    }
}

/// Does moshi-hook on that machine hook `agent` natively (from the machine's
/// last agent probe, kept a week)? Read at most every ten minutes per machine.
pub fn remote_hook(host: &str, agent: &str) -> bool {
    remote_native(host).iter().any(|a| a == agent)
}

/// The agents moshi-hook hooks natively on a machine (its last agent probe).
pub fn remote_native(host: &str) -> Vec<String> {
    type Seen = Vec<(String, Instant, Vec<String>)>;
    static S: std::sync::Mutex<Seen> = std::sync::Mutex::new(Vec::new());
    let mut g = S.lock().unwrap();
    if let Some((_, at, v)) = g.iter().find(|(h, _, _)| h == host) {
        if at.elapsed() < Duration::from_secs(600) {
            return v.clone();
        }
    }
    let v = crate::probe::cached(host, Duration::from_secs(7 * 86_400))
        .map(|p| p.moshi)
        .unwrap_or_default();
    g.retain(|(h, _, _)| h != host);
    g.push((host.to_string(), Instant::now(), v.clone()));
    v
}

/// The env that makes moshi-hook treat our calls as coming from the `aleph`
/// tmux session, so a tap opens it.
type Env = Vec<(String, String)>;

fn tmux_env() -> Env {
    static E: std::sync::Mutex<Option<(Instant, Env)>> = std::sync::Mutex::new(None);
    let mut c = E.lock().unwrap();
    if let Some((at, v)) = c.as_ref() {
        if at.elapsed() < Duration::from_secs(30) {
            return v.clone();
        }
    }
    let o = proc::cap(
        "tmux",
        &[
            "display-message",
            "-p",
            "-t",
            &format!("{TMUX_SESSION}:"),
            "#{socket_path},#{pid},#{session_id} #{pane_id}",
        ],
        Duration::from_secs(3),
    );
    let v = if o.ok() {
        parse_tmux(&o.text())
    } else {
        vec![]
    };
    *c = Some((Instant::now(), v.clone()));
    v
}

/// `socket,pid,$id %pane` → `TMUX` / `TMUX_PANE`.
pub fn parse_tmux(s: &str) -> Vec<(String, String)> {
    let line = s.lines().next().unwrap_or("").trim();
    let Some((tmux, pane)) = line.rsplit_once(' ') else {
        return vec![];
    };
    if !pane.starts_with('%') || tmux.matches(',').count() != 2 {
        return vec![];
    }
    vec![
        ("TMUX".into(), tmux.replace(",$", ",")),
        ("TMUX_PANE".into(), pane.into()),
    ]
}

fn hook(args: &[&str]) -> Option<Command> {
    let b = bin()?;
    let mut c = Command::new(b);
    c.args(args);
    // never pretend to be inside the agent's own herdr pane
    c.env_remove("HERDR_ENV").env_remove("HERDR_PANE_ID");
    for (k, v) in tmux_env() {
        c.env(k, v);
    }
    c.stdin(Stdio::null());
    Some(c)
}

/// One inbox card (and a push unless `silent`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Note {
    pub label: String,
    pub title: String,
    pub message: String,
    pub error: bool,
    pub silent: bool,
    /// wait for Moshi's server to take it, and fail when it doesn't (a
    /// best-effort send always "works", even with no phone to push to)
    pub strict: bool,
}

pub fn note_args(n: &Note) -> Vec<String> {
    let masked;
    let n = if crate::showcase::on() {
        masked = Note {
            label: crate::showcase::text(&n.label).into_owned(),
            title: crate::showcase::text(&n.title).into_owned(),
            message: crate::showcase::text(&n.message).into_owned(),
            ..n.clone()
        };
        &masked
    } else {
        n
    };
    let mut a = vec![
        "notify".to_string(),
        "--source".into(),
        SOURCE.into(),
        "--label".into(),
        if n.label.is_empty() {
            crate::brand::NAME.into()
        } else {
            n.label.clone()
        },
    ];
    if !n.title.is_empty() {
        a.push("--title".into());
        a.push(n.title.clone());
    }
    if n.error {
        a.push("--error".into());
    }
    if n.silent {
        a.push("--silent".into());
    }
    if n.strict {
        a.push("--strict".into());
    }
    a.push("--".into());
    a.push(if n.message.is_empty() {
        n.title.clone()
    } else {
        n.message.clone()
    });
    a
}

pub fn notify(n: &Note) -> Result<(), String> {
    if let Some(e) = crate::showcase::refused() {
        return Err(e);
    }
    let a = note_args(n);
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let mut c = hook(&refs).ok_or("moshi-hook isn't installed")?;
    // a strict send waits while the daemon retries (about 40 s)
    let t = if n.strict { T_STRICT } else { T_NOTIFY };
    let o = proc::run(&mut c, t, None).map_err(|e| e.to_string())?;
    if o.ok() {
        Ok(())
    } else if n.strict && o.reason().contains("unknown flag") {
        // an older moshi-hook: send it the best-effort way
        notify(&Note {
            strict: false,
            ..n.clone()
        })
    } else {
        Err(o
            .reason()
            .trim_start_matches("moshi-hook: ")
            .trim_start_matches("notification not sent: ")
            .to_string())
    }
}

/// A question for the phone.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub label: String,
    pub question: String,
    /// 2–6 buttons; empty = Approve / Deny
    pub options: Vec<String>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// yes/no question approved
    Approved,
    /// multiple choice: this option
    Chosen(String),
    Denied,
    TimedOut,
    /// daemon not running, not paired, offline
    Unreachable(String),
    Failed(String),
}

pub fn ask_args(q: &Ask) -> Vec<String> {
    let mut a = vec![
        "ask".to_string(),
        "--require-remote".into(),
        "--source".into(),
        SOURCE.into(),
        "--label".into(),
        q.label.clone(),
        "--timeout".into(),
        format!("{}s", q.timeout.as_secs().max(30)),
    ];
    for o in q.options.iter().take(6) {
        a.push("--option".into());
        a.push(o.clone());
    }
    a.push("--".into());
    a.push(q.question.clone());
    a
}

/// Map moshi-hook ask's exit status (and stdout) to an answer.
pub fn answer_of(code: Option<i32>, stdout: &str, stderr: &str, multi: bool) -> Answer {
    match code {
        Some(0) if multi => match stdout.trim() {
            "" => Answer::Failed("no option came back".into()),
            o => Answer::Chosen(o.to_string()),
        },
        Some(0) => Answer::Approved,
        // the app approved without picking one of our buttons: moshi-hook
        // cancels it and says why — not a "no"
        Some(_) if stderr.contains("without choosing an option") => Answer::Failed(
            stderr
                .lines()
                .find(|l| l.contains("without choosing"))
                .unwrap_or(stderr)
                .trim()
                .trim_start_matches("moshi-hook: ")
                .to_string(),
        ),
        // a question taken back (or dismissed) isn't a "no"
        Some(1) if stderr.to_ascii_lowercase().contains("cancel") => {
            Answer::Failed("cancelled".into())
        }
        Some(1) => Answer::Denied,
        Some(3) => Answer::TimedOut,
        Some(4) => Answer::Unreachable(stderr.trim().to_string()),
        Some(n) => Answer::Failed(format!("exit {n}: {}", stderr.trim())),
        None => Answer::Failed("killed".into()),
    }
}

/// Start an ask; returns the running child (the caller waits or kills it).
pub fn ask_spawn(q: &Ask) -> Result<std::process::Child, String> {
    let a = ask_args(q);
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let mut c = hook(&refs).ok_or("moshi-hook isn't installed")?;
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    c.spawn().map_err(|e| e.to_string())
}

/// Wait for a spawned ask (`cancel` kills it, e.g. when the question was
/// answered at a terminal meanwhile).
pub fn ask_wait(
    mut child: std::process::Child,
    multi: bool,
    cancel: &std::sync::atomic::AtomicBool,
    deadline: Duration,
) -> Answer {
    use std::io::Read;
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let mut out = String::new();
                let mut err = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut out);
                }
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return answer_of(st.code(), &out, &err, multi);
            }
            Ok(None) => {}
            Err(e) => return Answer::Failed(e.to_string()),
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) || t0.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return if t0.elapsed() > deadline {
                Answer::TimedOut
            } else {
                Answer::Failed("cancelled".into())
            };
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// A live job row: start → id.
pub fn job_start(label: &str, title: &str) -> Option<String> {
    let mut c = hook(&[
        "job", "start", "--source", SOURCE, "--label", label, "--", title,
    ])?;
    let o = proc::run(&mut c, T_NOTIFY, None).ok()?;
    let id = o.text().trim().to_string();
    (o.ok() && !id.is_empty()).then_some(id)
}

pub fn job_update(id: &str, progress: Option<u8>, message: &str) {
    let p = progress.map(|p| p.min(100).to_string());
    let mut a = vec!["job", "update", id, "--message", message];
    if let Some(p) = &p {
        a.push("--progress");
        a.push(p);
    }
    if let Some(mut c) = hook(&a) {
        let _ = proc::run(&mut c, T_NOTIFY, None);
    }
}

pub fn job_end(id: &str, error: bool, message: &str) {
    let mut a = vec!["job", "end", id, "--message", message];
    if error {
        a.push("--error");
    }
    if let Some(mut c) = hook(&a) {
        let _ = proc::run(&mut c, T_NOTIFY, None);
    }
}

/// Plan-limit snapshots per agent account (`moshi-hook usage`): the rings in
/// Moshi's Usage view, for Aleph's analytics too.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub agent: String,
    pub account: String,
    pub label: String,
    pub used: f64,
    pub resets_at: String,
}

pub fn parse_usage(s: &str) -> Vec<Window> {
    let v: Value = serde_json::from_str(s).unwrap_or(Value::Null);
    let mut out = Vec::new();
    for acct in v.as_array().into_iter().flatten() {
        let st = |k: &str| {
            acct.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        for w in acct
            .get("windows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            out.push(Window {
                agent: st("agent"),
                account: st("accountLabel"),
                label: w.get("label").and_then(Value::as_str).unwrap_or("").into(),
                used: w
                    .get("usedPercentage")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                resets_at: w
                    .get("resetsAt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
            });
        }
    }
    out
}

pub fn usage() -> Vec<Window> {
    let Some(b) = bin() else { return vec![] };
    let o = proc::cap(&b.to_string_lossy(), &["usage"], Duration::from_secs(30));
    if o.ok() {
        parse_usage(&o.text())
    } else {
        vec![]
    }
}

/// Recent agent folders on this machine as moshi-hook sees them (Claude
/// transcripts, Codex rollouts, Cursor projects — also sessions started
/// outside Aleph), cached for five minutes. Empty without moshi-hook.
pub fn recent_folders() -> Vec<String> {
    static C: std::sync::Mutex<Option<(Instant, Vec<String>)>> = std::sync::Mutex::new(None);
    let mut c = C.lock().unwrap();
    if let Some((at, v)) = c.as_ref() {
        if at.elapsed() < Duration::from_secs(300) {
            return v.clone();
        }
    }
    let v = cwd_list(20);
    *c = Some((Instant::now(), v.clone()));
    v
}

/// Recent agent folders on this machine, most recent first.
pub fn cwd_list(limit: usize) -> Vec<String> {
    let Some(b) = bin() else { return vec![] };
    let l = limit.to_string();
    let o = proc::cap(
        &b.to_string_lossy(),
        &["cwd-list", "--json", "--limit", &l],
        T_QUICK,
    );
    let v: Value = serde_json::from_str(&o.text()).unwrap_or(Value::Null);
    let arr = v
        .as_array()
        .cloned()
        .or_else(|| v.get("entries").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    arr.iter()
        .filter_map(|e| {
            e.as_str()
                .map(String::from)
                .or_else(|| e.get("cwd").and_then(Value::as_str).map(String::from))
                .or_else(|| e.get("path").and_then(Value::as_str).map(String::from))
        })
        .collect()
}

// ------------------------------------------------------------------ questions

/// The agent's own choices in a question (`❯ 1. Yes`, `2. No, and tell …`):
/// (number, label). Empty when it isn't a numbered menu.
pub fn options(detection: &str) -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = Vec::new();
    for l in detection.lines() {
        let t = l
            .trim_start()
            .trim_start_matches(['❯', '›', '>', '→', '▸'])
            .trim_start();
        let Some((n, rest)) = t.split_once(['.', ')']) else {
            continue;
        };
        let Ok(n) = n.trim().parse::<u32>() else {
            continue;
        };
        let label = rest.trim();
        if label.is_empty() || v.iter().any(|(m, _)| *m == n) {
            continue;
        }
        v.push((n, label.to_string()));
    }
    if v.len() >= 2 {
        v
    } else {
        vec![]
    }
}

/// Button text for an option: `1 · Yes` (short; numbered so the answer maps back).
pub fn button(n: u32, label: &str) -> String {
    format!("{n} · {}", crate::ui::trunc(label, 40))
}

/// The option number a button stands for.
pub fn button_number(b: &str) -> Option<u32> {
    b.split_once(" · ").and_then(|(n, _)| n.trim().parse().ok())
}

/// The question itself: the lines before the options (the ask's body).
pub fn question_text(detection: &str) -> String {
    // a line of one drawing character (a box's border, a preview's edge)
    let rule = |l: &str, set: &str| {
        let t = l.trim();
        t.chars().count() >= 8 && t.chars().all(|c| set.contains(c))
    };
    let mut above: Vec<&str> = Vec::new();
    for l in detection.lines() {
        let t = l
            .trim_start()
            .trim_start_matches(['❯', '›', '>', '→', '▸'])
            .trim_start();
        if t.split_once(['.', ')'])
            .is_some_and(|(n, _)| n.trim().parse::<u32>().is_ok())
        {
            break;
        }
        above.push(l);
    }
    // the question's box starts at its top border (above it: the agent's
    // banner, your prompt, its work); with no border, its last few lines
    let start = match above.iter().rposition(|l| rule(l, "─━═")) {
        Some(i) => i + 1,
        None => above.len().saturating_sub(8),
    };
    let mut lines: Vec<&str> = Vec::new();
    // between dashed edges: a command (keep it) or a file's contents (its
    // first lines are enough)
    let mut preview: Option<usize> = None;
    for l in &above[start..] {
        if rule(l, "╌┄┈-") {
            preview = if preview.is_some() { None } else { Some(0) };
            continue;
        }
        let t = l.trim();
        if t.is_empty() || t.chars().all(|c| "─━│┃╭╮╰╯═-".contains(c)) || t.starts_with("Tip:")
        {
            continue;
        }
        if let Some(n) = preview.as_mut() {
            *n += 1;
            if *n > 3 {
                if *n == 4 {
                    lines.push("…");
                }
                continue;
            }
        }
        lines.push(t);
    }
    let s = lines.join("\n");
    if s.chars().count() > 600 {
        crate::ui::trunc(&s, 600)
    } else {
        s
    }
}

/// A stable fingerprint of a question, to tell "still the same question".
pub fn fingerprint(detection: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    detection
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: &str = "\
 Bash command
   pytest -x tests/
   Run the test suite

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for pytest commands in /srv/px
   3. No, and tell Claude what to do differently (esc)
";

    #[test]
    fn options_and_question_come_from_the_agents_menu() {
        let o = options(Q);
        assert_eq!(o.len(), 3);
        assert_eq!(o[0], (1, "Yes".to_string()));
        assert!(o[2].1.starts_with("No, and tell Claude"));
        let q = question_text(Q);
        assert!(q.starts_with("Bash command\npytest -x tests/"));
        assert!(q.ends_with("Do you want to proceed?"));
        assert_eq!(button_number(&button(2, &o[1].1)), Some(2));
        assert!(options("Continue? (y/n)").is_empty());
    }

    #[test]
    fn the_question_without_the_screen_around_it() {
        // a whole screen: banner, prompt, then the permission box with a preview
        let screen = "\
 ▐▛███▛█   Claude Code v2.1.308
▝▜██████▀  Opus 5.5 · Claude Max
❯ Create the file /tmp/x/demo.txt containing hi.
⏺ Write(/tmp/x/demo.txt)
───────────────────────────────────────────────────────────
 Create file
 /tmp/x/demo.txt
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
  1 hi
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Do you want to create demo.txt?
 ❯ 1. Yes
   2. Yes, and switch to accept edits
   3. No
";
        assert_eq!(
            question_text(screen),
            "Create file\n/tmp/x/demo.txt\n1 hi\nDo you want to create demo.txt?"
        );
        // a command between dashed edges stays; a tip and a long file don't
        let bash = "\
───────────────────────────────────────────────────────────
 Bash command
 Tip: auto mode handles these prompts for you
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
   curl -sI https://example.org | head -1
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Fetch example.org headers
 Do you want to proceed?
 ❯ 1. Yes
   4. No
";
        assert_eq!(
            question_text(bash),
            "Bash command\ncurl -sI https://example.org | head -1\nFetch example.org headers\nDo you want to proceed?"
        );
        let long = format!(
            "{}\n Create file\n{}\n{}\n{}\n Do you want to create a.txt?\n 1. Yes\n",
            "─".repeat(20),
            "╌".repeat(20),
            (1..=9)
                .map(|i| format!("  {i} line"))
                .collect::<Vec<_>>()
                .join("\n"),
            "╌".repeat(20)
        );
        assert_eq!(
            question_text(&long),
            "Create file\n1 line\n2 line\n3 line\n…\nDo you want to create a.txt?"
        );
    }

    #[test]
    fn fingerprints_ignore_spacing_only() {
        assert_eq!(fingerprint("a  b\nc"), fingerprint("a b c"));
        assert_ne!(fingerprint("a b"), fingerprint("a c"));
    }

    #[test]
    fn status_and_native_agents() {
        let st = r#"{"displayName":"Mac","paired":true,"hooks":[{"target":"claude","status":"current"},{"target":"gemini","status":"not_found"},{"target":"antigravity","status":"current"}]}"#;
        let pr = r#"{"installed":true,"running":true,"gateway":true,"version":"0.4.35","e2ee":{"policy":"off"}}"#;
        let s = parse_status(st, pr);
        assert!(s.ready() && s.gateway);
        assert_eq!(s.version, "0.4.35");
        assert_eq!(s.native(), vec!["claude", "agy"]);
        assert!(!parse_status("", "").ready());
    }

    #[test]
    fn modes_skip_what_moshi_sees_natively() {
        let c = Config::in_memory(&["host.b31.moshi=machine-only", "host.gpu2.moshi=off"]);
        let native = vec!["claude".to_string()];
        let none = |_: &str, _: &str| false;
        let m = |h, a| mode_with(&c, h, a, &native, none);
        assert_eq!(m("local", "claude"), Mode::MachineOnly);
        assert_eq!(m("local", "gemini"), Mode::Full);
        assert_eq!(m("dex42", "claude"), Mode::Full);
        assert_eq!(m("b31", "codex"), Mode::MachineOnly);
        assert_eq!(m("gpu2", "codex"), Mode::Off);
        // moshi-hook on that machine hooks codex: only machine news from us
        // for codex there, unless its setting says otherwise
        let hook = |h: &str, a: &str| (h == "dex42" || h == "gpu2") && a == "codex";
        assert_eq!(
            mode_with(&c, "dex42", "codex", &native, hook),
            Mode::MachineOnly
        );
        assert_eq!(mode_with(&c, "dex42", "claude", &native, hook), Mode::Full);
        assert_eq!(mode_with(&c, "gpu2", "codex", &native, hook), Mode::Off);
        assert_eq!(mode_with(&c, "local", "gemini", &native, hook), Mode::Full);
    }

    #[test]
    fn ask_args_and_answers() {
        let a = ask_args(&Ask {
            label: "Aleph · b31 · heron".into(),
            question: "Run pytest?".into(),
            options: vec!["1 · Yes".into(), "3 · No".into()],
            timeout: Duration::from_secs(1800),
        });
        assert_eq!(&a[..2], &["ask", "--require-remote"]);
        assert!(a.windows(2).any(|w| w == ["--timeout", "1800s"]));
        assert_eq!(a.iter().filter(|x| *x == "--option").count(), 2);
        assert_eq!(a.last().unwrap(), "Run pytest?");
        assert_eq!(
            answer_of(Some(0), "1 · Yes\n", "", true),
            Answer::Chosen("1 · Yes".into())
        );
        assert_eq!(answer_of(Some(0), "", "", false), Answer::Approved);
        assert_eq!(answer_of(Some(1), "", "", true), Answer::Denied);
        assert!(matches!(
            answer_of(Some(1), "", "moshi-hook: approved without choosing an option (update the Moshi app); treating as cancelled\n", true),
            Answer::Failed(m) if m.starts_with("approved without choosing")
        ));
        // taken back is not a "no"
        assert_eq!(
            answer_of(Some(1), "", "moshi-hook: cancelled\n", false),
            Answer::Failed("cancelled".into())
        );
        assert_eq!(answer_of(Some(3), "", "", true), Answer::TimedOut);
        assert!(matches!(
            answer_of(Some(4), "", "offline", true),
            Answer::Unreachable(_)
        ));
    }

    #[test]
    fn notes_and_tmux_context() {
        let a = note_args(&Note {
            label: "Aleph · b31".into(),
            title: "heron needs you".into(),
            message: "Run pytest?".into(),
            error: false,
            silent: true,
            strict: true,
        });
        assert!(a.contains(&"--strict".to_string()));
        assert_eq!(a[0], "notify");
        assert!(a.contains(&"--silent".to_string()) && !a.contains(&"--error".to_string()));
        assert_eq!(a.last().unwrap(), "Run pytest?");
        assert_eq!(
            parse_tmux("/private/tmp/tmux-501/default,4242,$3 %7\n"),
            vec![
                (
                    "TMUX".to_string(),
                    "/private/tmp/tmux-501/default,4242,3".to_string()
                ),
                ("TMUX_PANE".to_string(), "%7".to_string())
            ]
        );
        assert!(parse_tmux("no server running").is_empty());
    }

    #[test]
    fn usage_windows() {
        let s = r#"[{"accountLabel":"Pro","agent":"codex","windows":[{"label":"7d","usedPercentage":8,"resetsAt":"2026-10-28T11:24:00Z"}]},{"agent":"claude","windows":[{"label":"5h","usedPercentage":62.5,"resetsAt":"x"},{"label":"7d","usedPercentage":30,"resetsAt":"y"}]}]"#;
        let w = parse_usage(s);
        assert_eq!(w.len(), 3);
        assert_eq!(
            (w[0].agent.as_str(), w[0].label.as_str(), w[0].used),
            ("codex", "7d", 8.0)
        );
        assert_eq!(w[1].used, 62.5);
        assert!(parse_usage("nope").is_empty());
    }
}

// ------------------------------------------------------------------ aleph moshi

use crate::cli::Args;
use crate::out::*;
use std::process::ExitCode;

/// Can a phone reach this computer's shell? (sshd, mosh-server, Tailscale)
#[derive(Clone, Debug, Default)]
pub struct Shell {
    pub sshd: bool,
    pub mosh: Option<String>,
    pub tailscale: Option<String>,
    pub user: String,
}

pub fn shell_access() -> Shell {
    let sshd = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 22)),
        Duration::from_millis(800),
    )
    .is_ok();
    // what a non-interactive ssh command (mosh's server start) finds
    let mosh = proc::cap(
        "zsh",
        &["-c", "command -v mosh-server"],
        Duration::from_secs(5),
    )
    .text()
    .lines()
    .next()
    .map(str::trim)
    .filter(|p| p.starts_with('/'))
    .map(String::from)
    .or_else(|| proc::which("mosh-server").map(|p| p.to_string_lossy().into_owned()));
    let ts = proc::cap("tailscale", &["ip", "-4"], Duration::from_secs(5));
    let tailscale = ts
        .ok()
        .then(|| ts.text().lines().next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty());
    Shell {
        sshd,
        mosh,
        tailscale,
        user: std::env::var("USER").unwrap_or_default(),
    }
}

fn tmux_running() -> bool {
    proc::cap(
        "tmux",
        &["has-session", "-t", TMUX_SESSION],
        Duration::from_secs(3),
    )
    .ok()
}

pub fn cmd(a: Args) -> ExitCode {
    let sub = a.rest.first().map(String::as_str).unwrap_or("status");
    let has = |f: &str| a.rest.iter().any(|x| x == f);
    match sub {
        "status" | "--json" => status_cmd(&a.cfg, has("--json")),
        "install" | "setup" => install(a),
        "uninstall" | "off" => uninstall(a),
        "test" => test(&a.cfg, has("--ask")),
        "usage" => usage_cmd(has("--json")),
        "sync" => sync(a),
        _ => fail(
            "usage: aleph moshi [status | install | test [--ask] | usage | uninstall] [--json]",
        ),
    }
}

fn status_cmd(cfg: &Config, js: bool) -> ExitCode {
    let st = status();
    let sh = shell_access();
    let tmux = tmux_running();
    if js {
        json(&serde_json::json!({
            "installed": bin().is_some(),
            "bin": bin().map(|b| b.to_string_lossy().into_owned()),
            "version": st.as_ref().map(|s| s.version.clone()),
            "paired": st.as_ref().is_some_and(|s| s.paired),
            "running": st.as_ref().is_some_and(|s| s.running),
            "gateway": st.as_ref().is_some_and(|s| s.gateway),
            "e2ee": st.as_ref().map(|s| s.e2ee.clone()),
            "native_agents": st.as_ref().map(|s| s.native()).unwrap_or_default(),
            "bridge": enabled(cfg),
            "events": cfg.get_or("moshi.events"),
            "approvals": cfg.flag("moshi.approvals"),
            "jobs": cfg.flag("moshi.jobs"),
            "session": {"wanted": cfg.flag("moshi.session"), "running": tmux, "name": TMUX_SESSION},
            "remote_allow": cfg.get_or("remote.allow"),
            "shell": {"sshd": sh.sshd, "mosh_server": sh.mosh, "tailscale_ip": sh.tailscale, "user": sh.user},
        }));
        return ExitCode::SUCCESS;
    }
    let Some(b) = bin() else {
        warn("moshi-hook isn't installed on this computer");
        hint("install Moshi's host tools (https://getmoshi.app), pair with `moshi-hook pair`, then: aleph moshi install");
        return ExitCode::from(1);
    };
    let st = st.unwrap_or_default();
    head("Moshi");
    let row = |k: &str, ok_: bool, v: String| {
        if ok_ {
            ok(&format!("{k:<12} {v}"))
        } else {
            warn(&format!("{k:<12} {v}"))
        }
    };
    row(
        "moshi-hook",
        true,
        format!("{} · {}", st.version, crate::doctor::tilde(&b)),
    );
    row(
        "paired",
        st.paired,
        if st.paired {
            format!("as {}", st.host_name)
        } else {
            "no — moshi-hook pair --token <from the Moshi app>".into()
        },
    );
    row(
        "daemon",
        st.running,
        if st.running {
            "running".into()
        } else {
            "not running — moshi-hook service install".into()
        },
    );
    row(
        "e2ee",
        st.e2ee == "on",
        if st.e2ee.is_empty() {
            "unknown".into()
        } else {
            format!(
                "{} (moshi-hook e2ee enable seals approvals end to end)",
                st.e2ee
            )
        },
    );
    let native = st.native();
    row(
        "native",
        true,
        if native.is_empty() {
            "no agent hooks here".into()
        } else {
            format!(
                "{} — moshi-hook's own cards; Aleph adds the rest",
                native.join(", ")
            )
        },
    );
    head("Aleph → Moshi");
    row(
        "bridge",
        enabled(cfg),
        format!(
            "moshi={} · events: {}",
            cfg.get_or("moshi"),
            cfg.get_or("moshi.events")
        ),
    );
    let allow = cfg.get_or("remote.allow");
    row(
        "approvals",
        cfg.flag("moshi.approvals") && !allow.is_empty(),
        if allow.is_empty() {
            "off — remote.allow is empty (notifications only)".into()
        } else {
            format!("on for {allow} (never for approve_deny questions)")
        },
    );
    row(
        "jobs",
        cfg.flag("moshi.jobs"),
        if cfg.flag("moshi.jobs") {
            "working sessions show as live rows".into()
        } else {
            "off".into()
        },
    );
    row(
        "session",
        !cfg.flag("moshi.session") || tmux,
        match (cfg.flag("moshi.session"), tmux) {
            (true, true) => {
                format!("tmux `{TMUX_SESSION}` is running — pick it in Moshi's session list")
            }
            (true, false) => {
                format!("tmux `{TMUX_SESSION}` isn't running yet (the daemon starts it)")
            }
            _ => "off".into(),
        },
    );
    head("Shell from the phone");
    row(
        "ssh",
        sh.sshd,
        if sh.sshd {
            format!("Remote Login is on · user {}", sh.user)
        } else {
            "Remote Login is off — moshi-hook host enable-ssh".into()
        },
    );
    row(
        "mosh",
        sh.mosh.is_some(),
        sh.mosh
            .clone()
            .unwrap_or_else(|| "mosh-server not found for ssh commands — brew install mosh".into()),
    );
    row(
        "tailscale",
        sh.tailscale.is_some(),
        sh.tailscale
            .clone()
            .map(|ip| format!("{ip} (reach this Mac from anywhere on your tailnet)"))
            .unwrap_or_else(|| "not connected".into()),
    );
    ExitCode::SUCCESS
}

fn install(a: Args) -> ExitCode {
    let mut cfg = a.cfg;
    let Some(b) = bin() else {
        err("moshi-hook isn't installed on this computer");
        hint("install Moshi's host tools (https://getmoshi.app) and pair this Mac from the Moshi app, then rerun");
        return ExitCode::from(1);
    };
    let b = b.to_string_lossy().into_owned();
    let st = status().unwrap_or_default();
    if !st.paired {
        err("this Mac isn't paired with Moshi yet");
        hint("in the Moshi app: Settings → Hooks → copy the token, then: moshi-hook pair --token <token>");
        return ExitCode::from(1);
    }
    if !st.running {
        let o = proc::cap(&b, &["service", "install"], Duration::from_secs(30));
        if o.ok() {
            ok("moshi-hook's background service is installed and running");
        } else {
            warn(&format!("moshi-hook service install: {}", o.reason()));
        }
    }
    let mut set = |k: &str, v: &str| {
        if let Err(e) = cfg.set(k, v) {
            warn(&format!("couldn't set {k}: {e}"));
        }
    };
    set("moshi", "on");
    set("moshi.approvals", "true");
    set("moshi.jobs", "true");
    set("moshi.session", "true");
    let allow_was_empty = cfg.get("remote.allow").is_none();
    if allow_was_empty {
        let _ = cfg.set("remote.allow", "*:*");
    }
    ok("Aleph → Moshi is on: fleet cards, phone approvals and live jobs");
    if allow_was_empty {
        hint("approvals from the phone are on for every machine and agent (remote.allow=*:*) — narrow it with: aleph settings set remote.allow b31:*,*:claude");
    }
    match crate::moshibridge::ensure_tmux_session() {
        Ok(true) => ok(&format!(
            "started tmux session `{TMUX_SESSION}` — Moshi lists it when you connect"
        )),
        Ok(false) => ok(&format!("tmux session `{TMUX_SESSION}` is already running")),
        Err(e) => warn(&format!(
            "couldn't start the tmux session: {e} (brew install tmux)"
        )),
    }
    let fleet = cfg.fleet();
    let n = Note {
        label: crate::brand::NAME.into(),
        title: format!("{} is connected", crate::brand::NAME),
        message: format!(
            "Cards, approvals and live jobs from {} will arrive here. Open the `{TMUX_SESSION}` session for the dashboard.",
            if fleet.is_empty() { "your machines".to_string() } else { fleet.join(", ") }
        ),
        strict: true,
        ..Default::default()
    };
    match notify(&n) {
        Ok(()) => ok("sent a hello card to Moshi"),
        Err(e) => warn(&format!("couldn't send a test card: {e}")),
    }
    let sh = shell_access();
    if !sh.sshd {
        warn("Remote Login (ssh) is off, so the phone can't open a shell here — moshi-hook host enable-ssh");
    }
    if sh.mosh.is_none() {
        warn("mosh-server isn't reachable for ssh commands — brew install mosh");
    }
    status_cmd(&cfg, false)
}

fn uninstall(a: Args) -> ExitCode {
    let mut cfg = a.cfg;
    let _ = cfg.set("moshi", "off");
    if tmux_running() {
        let _ = proc::cap(
            "tmux",
            &["kill-session", "-t", TMUX_SESSION],
            Duration::from_secs(5),
        );
    }
    ok("Aleph → Moshi is off (moshi-hook itself and its agent hooks are untouched)");
    ExitCode::SUCCESS
}

fn test(cfg: &Config, ask: bool) -> ExitCode {
    if bin().is_none() {
        return fail("moshi-hook isn't installed on this computer");
    }
    if !ask {
        return match notify(&Note {
            label: crate::brand::NAME.into(),
            title: format!("{} test", crate::brand::NAME),
            message: "If you can read this, Aleph reaches Moshi.".into(),
            strict: true,
            ..Default::default()
        }) {
            Ok(()) => {
                ok("sent — check the Moshi inbox");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&format!("couldn't send: {e}")),
        };
    }
    let choices = cfg.flag("moshi.choices");
    let q = Ask {
        label: crate::brand::NAME.into(),
        question: "Aleph test — tap a button (nothing is sent anywhere)".into(),
        options: if choices {
            vec!["1 · Looks good".into(), "2 · Something's off".into()]
        } else {
            vec![]
        },
        timeout: Duration::from_secs(300),
    };
    let sp = Spinner::start("Waiting for your answer on the phone (5 min)".into());
    let r = ask_spawn(&q).map(|c| {
        ask_wait(
            c,
            true,
            &std::sync::atomic::AtomicBool::new(false),
            Duration::from_secs(320),
        )
    });
    sp.stop();
    match r {
        Ok(Answer::Chosen(o)) => {
            ok(&format!("you picked: {o}"));
            ExitCode::SUCCESS
        }
        Ok(Answer::Approved) | Ok(Answer::Denied) => {
            ok("your answer came back from the phone");
            ExitCode::SUCCESS
        }
        Ok(a) => fail(&format!("no answer: {a:?}")),
        Err(e) => fail(&e),
    }
}

fn usage_cmd(js: bool) -> ExitCode {
    let w = usage();
    if js {
        json(&serde_json::json!(w
            .iter()
            .map(|w| serde_json::json!({"agent": w.agent, "account": w.account, "window": w.label, "used": w.used, "resets_at": w.resets_at}))
            .collect::<Vec<_>>()));
        return ExitCode::SUCCESS;
    }
    if w.is_empty() {
        warn("no plan-limit snapshots (moshi-hook usage found none)");
        return ExitCode::SUCCESS;
    }
    head("Plan limits (from moshi-hook usage)");
    for x in &w {
        say(ratatui::text::Line::raw(format!(
            "  {:<8} {:<6} {:>5.1}%  resets {}  {}",
            x.agent, x.label, x.used, x.resets_at, x.account
        )));
    }
    ExitCode::SUCCESS
}

/// One pass of the bridge, then wait for answers (scripts and tests).
fn sync(a: Args) -> ExitCode {
    let mut f = crate::fleet::Fleet::new(&a.cfg, a.flag.as_deref());
    if f.hosts.is_empty() {
        return crate::cli::no_host();
    }
    f.poll_blocking(Duration::from_secs(45));
    let mut b = crate::moshibridge::Bridge::new();
    // asked for now: what's waiting goes now
    b.grace = Some(0);
    b.tick(&a.cfg, &f, &[], &[]);
    b.drain(Duration::from_secs(60));
    ok("moshi sync: done");
    ExitCode::SUCCESS
}
