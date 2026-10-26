//! A task queue per machine (`aleph queue`): prompts waiting for an agent.
//!
//! When a session on that machine is idle (or has finished), the next task is
//! sent to it — first in, first out, matching the task's folder and agent if it
//! names them. `--new` tasks get a fresh session instead. Dispatch runs in the
//! automation hub (`auto.rs`): in `aleph watch`, or in the dashboard when no
//! watch is running. Every task sent lands in the inbox.

use crate::cli::Args;
use crate::config::{write_atomic, Config};
use crate::fleet::Fleet;
use crate::herdr::{Herdr, Session};
use crate::out::{err, fail, hint, json, ok, out};
use crate::ui::{self, c};
use crate::{events, theme};
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Task {
    pub id: u64,
    pub host: String,
    pub prompt: String,
    /// only a session in (or under) this folder takes it
    pub dir: Option<String>,
    /// only this agent kind takes it
    pub agent: Option<String>,
    /// start a fresh session for it instead of reusing an idle one
    pub new: bool,
    /// only this session (by name) takes it: a message queued for one pane
    pub to: Option<String>,
    pub added: u64,
    /// queued | sent | failed
    pub state: String,
    pub session: Option<String>,
    pub at: Option<u64>,
    pub error: Option<String>,
    /// not before this time (unix seconds): `send --after 14:30`
    pub after: Option<u64>,
    /// queued while offline: the session's state_change_seq and status then.
    /// If it moved on before we're back (answered, prompted by someone else),
    /// the message is held rather than sent into a different moment.
    pub seq: Option<u64>,
    pub seen: Option<String>,
}

impl Task {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "id": self.id, "host": self.host, "prompt": self.prompt, "dir": self.dir,
            "agent": self.agent, "new": self.new, "to": self.to, "added": self.added, "state": self.state,
            "session": self.session, "at": self.at, "error": self.error, "after": self.after,
            "seq": self.seq, "seen": self.seen,
        })
    }
    fn from_json(v: &Value) -> Option<Task> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Task {
            id: v.get("id")?.as_u64()?,
            host: s("host")?,
            prompt: s("prompt")?,
            dir: s("dir"),
            agent: s("agent"),
            new: v.get("new").and_then(Value::as_bool).unwrap_or(false),
            to: s("to"),
            added: v.get("added").and_then(Value::as_u64).unwrap_or(0),
            state: s("state").unwrap_or_else(|| "queued".into()),
            session: s("session"),
            at: v.get("at").and_then(Value::as_u64),
            error: s("error"),
            after: v.get("after").and_then(Value::as_u64),
            seq: v.get("seq").and_then(Value::as_u64),
            seen: s("seen"),
        })
    }
}

pub fn file() -> PathBuf {
    crate::config::world_state().join("queue.json")
}

/// The file's shape: `{"v": 1, "tasks": […]}`; a bare list is the first one.
const VERSION: u64 = 1;

fn parse(text: &str) -> Vec<Task> {
    let v = serde_json::from_str::<Value>(text).ok();
    v.as_ref()
        .and_then(|v| v.as_array().or_else(|| v.get("tasks")?.as_array()))
        .map(|a| a.iter().filter_map(Task::from_json).collect())
        .unwrap_or_default()
}

pub fn load() -> Vec<Task> {
    std::fs::read_to_string(file())
        .map(|t| parse(&t))
        .unwrap_or_default()
}

/// Read-modify-write under a lock file, so watch, the dashboard and the CLI
/// never lose each other's changes.
pub fn update<R>(f: impl FnOnce(&mut Vec<Task>) -> R) -> R {
    let lock = file().with_extension("lock");
    let _ = std::fs::create_dir_all(crate::config::world_state());
    for _ in 0..40 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(_) => break,
            Err(_) => {
                let stale = std::fs::metadata(&lock)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|e| e > Duration::from_secs(10));
                if stale {
                    let _ = std::fs::remove_file(&lock);
                } else {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
    let mut v = load();
    let r = f(&mut v);
    // keep finished tasks for a day, so `queue ls` can show what happened
    let now = crate::cache::now();
    v.retain(|t| t.state == "queued" || t.at.is_none_or(|at| now.saturating_sub(at) < 86_400));
    let data =
        json!({"v": VERSION, "tasks": v.iter().map(Task::to_json).collect::<Vec<_>>()}).to_string();
    let _ = write_atomic(&file(), data.as_bytes());
    let _ = std::fs::remove_file(&lock);
    r
}

/// Queued tasks for `host` — cheap enough to call every frame (re-read only
/// when the file changes).
pub fn count(host: &str) -> usize {
    cached(|v| {
        v.iter()
            .filter(|t| t.host == host && t.state == "queued")
            .count()
    })
}

/// Run `f` over the queue, re-read only when the file changed (per frame).
fn cached<R>(f: impl FnOnce(&[Task]) -> R) -> R {
    static CACHE: Mutex<Option<(Option<SystemTime>, Vec<Task>)>> = Mutex::new(None);
    let mtime = std::fs::metadata(file()).and_then(|m| m.modified()).ok();
    let mut g = CACHE.lock().unwrap();
    if g.as_ref().is_none_or(|(t, _)| *t != mtime) {
        *g = Some((mtime, load()));
    }
    f(g.as_ref().map(|(_, v)| v.as_slice()).unwrap_or(&[]))
}

pub fn add(t: Task) -> u64 {
    update(|v| {
        let id = v.iter().map(|x| x.id).max().unwrap_or(0) + 1;
        v.push(Task {
            id,
            added: crate::cache::now(),
            state: "queued".into(),
            ..t
        });
        id
    })
}

/// Does session `s` fit task `t`?
pub fn fits(t: &Task, s: &Session) -> bool {
    s.host == t.host
        && matches!(s.status.as_str(), "idle" | "done")
        && t.to
            .as_deref()
            .is_none_or(|n| s.name == n || s.display() == n)
        && t.agent
            .as_deref()
            .is_none_or(|a| crate::agents::same(&s.agent, a))
        && t.dir.as_deref().is_none_or(|d| {
            s.cwd == d || s.cwd.starts_with(&format!("{}/", d.trim_end_matches('/')))
        })
}

/// What dispatch decided for one task.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// send it to this session (host|pane key)
    Send(u64, String),
    /// start a new session for it
    Start(u64),
    /// queued offline for a moment that has passed: hold it (why)
    Hold(u64, String),
}

/// A message queued offline for a session that has moved on since: why to
/// hold it (None: send). One queued while it worked was waiting for it to
/// finish anyway.
pub fn stale(t: &Task, s: &Session) -> Option<String> {
    let q = t.seq?;
    if t.seen.as_deref() == Some("working") || s.seq == 0 || q == 0 || s.seq == q {
        return None;
    }
    Some(format!(
        "{} changed while you were offline (it was {} then) — aleph queue send {} sends it anyway",
        s.display(),
        t.seen.as_deref().unwrap_or("idle"),
        t.id
    ))
}

/// One step per machine: the oldest queued task that can go somewhere now.
/// `busy` holds session keys that were just given a task (not idle yet in the
/// fleet's view). Pure — the hub carries the plans out.
pub fn plan(tasks: &[Task], sessions: &[&Session], busy: &[String]) -> Vec<Plan> {
    let mut out = Vec::new();
    let mut hosts_done: Vec<&str> = Vec::new();
    let mut taken: Vec<String> = busy.to_vec();
    for t in tasks.iter().filter(|t| t.state == "queued") {
        if hosts_done.contains(&t.host.as_str()) {
            continue;
        }
        if t.new {
            out.push(Plan::Start(t.id));
            hosts_done.push(&t.host);
            continue;
        }
        if let Some(s) = sessions
            .iter()
            .find(|s| fits(t, s) && !taken.contains(&s.key()))
        {
            if let Some(why) = stale(t, s) {
                out.push(Plan::Hold(t.id, why));
                continue;
            }
            taken.push(s.key());
            out.push(Plan::Send(t.id, s.key()));
            hosts_done.push(&t.host);
        }
    }
    out
}

/// Carry out one plan (blocking: run it off the UI thread).
pub fn run(
    p: &Plan,
    fleet_sessions: &[Session],
    target_of: impl Fn(&str) -> crate::config::Target,
) {
    let id = match p {
        Plan::Send(id, _) | Plan::Start(id) => *id,
        Plan::Hold(id, why) => {
            hold(*id, why);
            return;
        }
    };
    let Some(t) = load()
        .into_iter()
        .find(|t| t.id == id && t.state == "queued")
    else {
        return;
    };
    let tgt = target_of(&t.host);
    let h = Herdr::new(&tgt);
    let res: Result<String, String> = match p {
        Plan::Send(_, key) => match fleet_sessions.iter().find(|s| s.key() == *key) {
            Some(s) => h.prompt(&s.pane_id, &t.prompt).map(|_| s.name.clone()),
            None => Err("the session went away".into()),
        },
        Plan::Hold(..) => return,
        Plan::Start(_) => {
            let cfg = Config::load();
            // that machine's defaults (host.<alias>.agent, .args)
            let mut o = match crate::cli::parse_new(&["--host".into(), t.host.clone()], &cfg) {
                Ok(x) => x.opts,
                Err(e) => {
                    finish(id, Err(e), &t);
                    return;
                }
            };
            o.dir = t
                .dir
                .as_deref()
                .map(|d| crate::path::normalize(d.as_bytes()));
            if let Some(a) = &t.agent {
                o.agent = a.clone();
            }
            o.prompt = Some(t.prompt.clone());
            crate::cli::fill_names(&mut o);
            h.new_session(&o, &mut |_, _| {}).and_then(|v| {
                crate::launch::after_start(&tgt, &o, &v);
                v.first()
                    .map(|s| s.name.clone())
                    .ok_or_else(|| "nothing started".into())
            })
        }
    };
    finish(id, res, &t);
}

/// Hold a queued task (it says why), and say so in the Inbox.
fn hold(id: u64, why: &str) {
    let t = update(|v| {
        let t = v.iter_mut().find(|t| t.id == id && t.state == "queued")?;
        t.state = "held".into();
        t.error = Some(why.to_string());
        Some(t.clone())
    });
    if let Some(t) = t {
        crate::events::append(crate::events::Event {
            t: crate::cache::now(),
            host: t.host.clone(),
            name: t.to.clone().unwrap_or_default(),
            kind: crate::events::Kind::Task,
            title: format!("held #{id}: {why}"),
            ..Default::default()
        });
    }
}

/// Queued messages for one session, oldest first.
pub fn for_session(host: &str, name: &str) -> Vec<Task> {
    cached(|v| {
        v.iter()
            .filter(|t| t.host == host && t.state == "queued" && t.to.as_deref() == Some(name))
            .cloned()
            .collect()
    })
}

/// Drop a queued task; false if it was already sent or gone.
pub fn remove(id: u64) -> bool {
    update(|v| {
        let n = v.len();
        v.retain(|t| !(t.id == id && t.state == "queued"));
        n != v.len()
    })
}

/// Send a queued task to `s` right now, skipping the wait (blocking).
pub fn send_now(id: u64, s: &Session, tgt: &crate::config::Target) -> Result<String, String> {
    let t = load()
        .into_iter()
        .find(|t| t.id == id && t.state == "queued")
        .ok_or("it was already sent")?;
    let res = Herdr::new(tgt)
        .prompt(&s.pane_id, &t.prompt)
        .map(|_| s.name.clone());
    finish(id, res.clone(), &t);
    res.map(|n| format!("sent to {n}"))
}

fn finish(id: u64, res: Result<String, String>, t: &Task) {
    let now = crate::cache::now();
    update(|v| {
        if let Some(x) = v.iter_mut().find(|x| x.id == id) {
            x.at = Some(now);
            match &res {
                Ok(name) => {
                    x.state = "sent".into();
                    x.session = Some(name.clone());
                }
                Err(e) => {
                    x.state = "failed".into();
                    x.error = Some(e.clone());
                }
            }
        }
    });
    events::append(events::Event {
        t: now,
        host: t.host.clone(),
        name: res.clone().unwrap_or_default(),
        agent: t.agent.clone().unwrap_or_default(),
        kind: events::Kind::Task,
        title: match &res {
            Ok(_) => ui::trunc(&t.prompt, 120),
            Err(e) => format!("task #{id} failed: {e}"),
        },
        ..Default::default()
    });
}

// ------------------------------------------------------------------ CLI

fn usage() -> ExitCode {
    fail("usage: aleph queue [ls] | add [host] \"prompt\" [--dir /abs] [--agent A] [--to name] [--new] | rm <id> | send <id> | clear [host]")
}

pub fn cmd(a: Args) -> ExitCode {
    let p = a.positional();
    match p.first().copied().unwrap_or("ls") {
        "ls" | "list" => ls(&a),
        "add" => {
            let mut dir = None;
            let mut agent = None;
            let mut words: Vec<String> = Vec::new();
            let mut to: Option<String> = None;
            let mut i = 1;
            while i < a.rest.len() {
                match a.rest[i].as_str() {
                    "--dir" | "-d" => {
                        dir = a.rest.get(i + 1).cloned();
                        i += 1;
                    }
                    "--agent" => {
                        agent = a.rest.get(i + 1).cloned();
                        i += 1;
                    }
                    "--to" => {
                        to = a.rest.get(i + 1).cloned();
                        i += 1;
                    }
                    "--new" | "--json" => {}
                    w => words.push(w.to_string()),
                }
                i += 1;
            }
            // `add b31 "text"` or `add "text"` (default machine / --host)
            let fleet = a.cfg.fleet();
            // `--to b31:name` names the machine too
            let to_host = to
                .as_deref()
                .and_then(|t| t.split_once(':'))
                .map(|(h, _)| h.to_string());
            let to = to.map(|t| t.split_once(':').map_or(t.clone(), |(_, n)| n.to_string()));
            let (host, prompt) = match words.split_first() {
                _ if to_host.is_some() => (to_host.clone().unwrap_or_default(), words.join(" ")),
                Some((h, rest)) if fleet.contains(h) && !rest.is_empty() => {
                    (h.clone(), rest.join(" "))
                }
                _ => match a.flag.clone().or_else(|| a.cfg.get("ssh_host")) {
                    Some(h) => (h, words.join(" ")),
                    None => return crate::cli::no_host(),
                },
            };
            if prompt.trim().is_empty() {
                return usage();
            }
            if let Some(d) = &dir {
                if !d.starts_with('/') {
                    return fail("--dir must be an absolute path on that machine");
                }
            }
            let agent = match agent.map(|x| crate::agents::resolve(&x)) {
                Some(Ok(x)) => Some(x.to_string()),
                Some(Err(e)) => return fail(&e),
                None => None,
            };
            let new = a.has("--new");
            let id = add(Task {
                host: host.clone(),
                prompt,
                dir,
                agent,
                new,
                to: to.clone(),
                ..Default::default()
            });
            if a.has("--json") {
                json(&json!({"id": id, "host": host}));
            } else {
                ok(&format!(
                    "task #{id} queued on {host} — {}",
                    match &to {
                        _ if new => "it gets a new session".to_string(),
                        Some(n) => format!("{n} gets it once it's done with what it's doing"),
                        None => "the next idle session there takes it".to_string(),
                    }
                ));
                if !watch_running(&a.cfg) {
                    hint("tasks are handed out while aleph watch (or the dashboard) runs — aleph watch --install");
                }
            }
            ExitCode::SUCCESS
        }
        // a held one goes after all (no second look at the session)
        "send" => {
            let Some(id) = p
                .get(1)
                .and_then(|x| x.trim_start_matches('#').parse::<u64>().ok())
            else {
                return usage();
            };
            let ok_ = update(|v| {
                let Some(t) = v
                    .iter_mut()
                    .find(|t| t.id == id && matches!(t.state.as_str(), "held" | "queued"))
                else {
                    return false;
                };
                t.state = "queued".into();
                t.seq = None;
                t.seen = None;
                t.error = None;
                true
            });
            if ok_ {
                ok(&format!("task #{id} goes as soon as its session is free"));
                ExitCode::SUCCESS
            } else {
                err(&format!("no held or queued task #{id}"));
                ExitCode::from(1)
            }
        }
        "rm" | "remove" => {
            let Some(id) = p
                .get(1)
                .and_then(|x| x.trim_start_matches('#').parse::<u64>().ok())
            else {
                return usage();
            };
            if remove(id) {
                ok(&format!("task #{id} removed"));
                ExitCode::SUCCESS
            } else {
                err(&format!("no queued task #{id}"));
                ExitCode::from(1)
            }
        }
        "clear" => {
            let host = p.get(1).map(|s| s.to_string());
            let n = update(|v| {
                let n = v.len();
                v.retain(|t| t.state != "queued" || host.as_deref().is_some_and(|h| h != t.host));
                n - v.len()
            });
            ok(&format!(
                "cleared {n} queued task{}",
                if n == 1 { "" } else { "s" }
            ));
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

/// Is a `aleph watch` keeping the fleet (and the queue) moving right now?
/// Is a daemon running here (it answers on its socket; or, from a daemon
/// without one, its status.json is fresh)?
pub fn watch_running(cfg: &Config) -> bool {
    if crate::daemon::other_running() {
        return true;
    }
    let iv = cfg.num("watch_interval").max(5);
    std::fs::metadata(crate::config::world_cache().join("status.json"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|e| e < Duration::from_secs(iv * 3 + 5))
}

fn ls(a: &Args) -> ExitCode {
    let v = load();
    if a.has("--json") {
        json(&Value::Array(v.iter().map(Task::to_json).collect()));
        return ExitCode::SUCCESS;
    }
    if !std::io::stdout().is_terminal() {
        for t in &v {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                t.id,
                t.host,
                t.state,
                t.session.clone().unwrap_or_default(),
                t.dir.clone().unwrap_or_default(),
                t.prompt
            );
        }
        return ExitCode::SUCCESS;
    }
    if v.is_empty() {
        ok("the queue is empty");
        hint("aleph queue add <host> \"run the full test suite and fix what fails\"");
        return ExitCode::SUCCESS;
    }
    let now = crate::cache::now();
    for t in &v {
        let col = theme::machine(&a.cfg, &t.host);
        let (g, sc) = match t.state.as_str() {
            "queued" => ("≡ queued", c::BLUE()),
            "sent" => ("✓ sent", c::GREEN()),
            _ => ("✗ failed", c::RED()),
        };
        let mut l = vec![
            Span::styled(format!("  #{:<3} ", t.id), ui::faint()),
            Span::styled("▌", ui::fg(col)),
            Span::styled(ui::pad(&t.host, 8), ui::fg(col)),
            Span::styled(ui::pad(g, 10), ui::fg(sc)),
            Span::styled(
                ui::pad(&ui::ago(now.saturating_sub(t.at.unwrap_or(t.added))), 9),
                ui::faint(),
            ),
            Span::raw(ui::trunc(&t.prompt, 60)),
        ];
        match (&t.session, &t.to) {
            (Some(s), _) => l.push(Span::styled(format!("  → {s}"), ui::muted())),
            (None, Some(n)) => l.push(Span::styled(format!("  for {n}"), ui::muted())),
            _ => {}
        }
        if let Some(e) = &t.error {
            l.push(Span::styled(format!("  {e}"), ui::fg(c::RED())));
        }
        if t.new {
            l.push(Span::styled("  (new session)", ui::faint()));
        }
        if let Some(at) = t.after.filter(|at| *at > now && t.state == "queued") {
            l.push(Span::styled(
                format!("  after {}", ui::clock(at)),
                ui::fg(c::YELLOW()),
            ));
        }
        out(Line::from(l));
    }
    ExitCode::SUCCESS
}

/// `--after` → the time a task becomes due: `30m` / `2h` / `1d` from `now`,
/// or the next local `14:30` (`off` = the UTC offset). `idle` / `done` are
/// no time at all: the session takes it when it's free.
pub fn parse_after(s: &str, now: u64, off: i64) -> Result<Option<u64>, String> {
    let s = s.trim();
    if matches!(s, "idle" | "done" | "free") {
        return Ok(None);
    }
    if let Some((h, m)) = s.split_once(':') {
        let (Ok(h), Ok(m)) = (h.parse::<i64>(), m.parse::<i64>()) else {
            return Err(format!("--after {s}: use HH:MM"));
        };
        if !(0..24).contains(&h) || !(0..60).contains(&m) {
            return Err(format!("--after {s}: use HH:MM"));
        }
        let local = now as i64 + off;
        let midnight = local.div_euclid(86_400) * 86_400;
        let mut t = midnight + h * 3600 + m * 60;
        if t <= local {
            t += 86_400;
        }
        return Ok(Some((t - off) as u64));
    }
    match crate::log::parse_duration(s) {
        Some(d) => Ok(Some(now + d.as_secs())),
        None => Err(format!(
            "--after {s}: idle, a duration (30m, 2h) or a time (14:30)"
        )),
    }
}

/// Tasks the fleet could take right now (for the hub); ones with a later
/// `after` wait.
pub fn plans_for(fleet: &Fleet, busy: &[String]) -> Vec<Plan> {
    let now = crate::cache::now();
    let tasks: Vec<Task> = load()
        .into_iter()
        .filter(|t| t.after.is_none_or(|a| a <= now))
        .collect();
    if !tasks.iter().any(|t| t.state == "queued") {
        return vec![];
    }
    let sessions: Vec<&Session> = fleet
        .hosts
        .iter()
        .filter(|h| h.conn == crate::fleet::Conn::Up)
        .flat_map(|h| h.sessions.iter())
        .collect();
    plan(&tasks, &sessions, busy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_times() {
        // 2026-10-22 10:00 UTC; this computer at +05:30 (15:30 local)
        let now = 1_792_663_200;
        let off = 19_800;
        assert_eq!(parse_after("idle", now, off), Ok(None));
        assert_eq!(parse_after("30m", now, off), Ok(Some(now + 1800)));
        assert_eq!(parse_after("2h", now, off), Ok(Some(now + 7200)));
        // later today (16:00 local = 10:30 UTC)
        assert_eq!(parse_after("16:00", now, off), Ok(Some(now + 1800)));
        // already past today: tomorrow (09:00 local)
        assert_eq!(parse_after("09:00", now, off), Ok(Some(now + 63_000)));
        assert!(parse_after("25:00", now, off).is_err());
        assert!(parse_after("soon", now, off).is_err());
    }

    fn s(host: &str, pane: &str, status: &str, cwd: &str, agent: &str) -> Session {
        Session {
            host: host.into(),
            pane_id: pane.into(),
            status: status.into(),
            cwd: cwd.into(),
            agent: agent.into(),
            name: pane.into(),
            ..Default::default()
        }
    }
    fn t(id: u64, host: &str, dir: Option<&str>, agent: Option<&str>, new: bool) -> Task {
        Task {
            id,
            host: host.into(),
            prompt: format!("task {id}"),
            dir: dir.map(String::from),
            agent: agent.map(String::from),
            new,
            state: "queued".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_message_for_one_session_waits_for_it() {
        let a = s("b31", "p1", "idle", "/srv", "claude");
        let b = s("b31", "p2", "working", "/srv", "claude");
        let mut m = t(1, "b31", None, None, false);
        m.to = Some("p2".into());
        // p1 is idle but the message is for p2, which is busy
        assert!(plan(&[m.clone()], &[&a, &b], &[]).is_empty());
        let b = s("b31", "p2", "done", "/srv", "claude");
        assert_eq!(plan(&[m], &[&a, &b], &[]), vec![Plan::Send(1, b.key())]);
    }

    #[test]
    fn fifo_one_per_machine_matching_folder_and_agent() {
        let a = s("b31", "p1", "idle", "/srv/px", "claude");
        let b = s("b31", "p2", "working", "/srv/x", "claude");
        let c = s("gpu2", "p3", "done", "/home/u/train", "codex");
        let ss = vec![&a, &b, &c];
        let tasks = vec![
            t(1, "b31", Some("/srv/x"), None, false), // only p2 fits and it's busy
            t(2, "b31", Some("/srv"), None, false),   // p1 under /srv
            t(3, "b31", None, None, false),           // b31 already got one this step
            t(4, "gpu2", None, Some("claude"), false), // p3 is codex
            t(5, "gpu2", None, Some("codex"), false),
        ];
        assert_eq!(
            plan(&tasks, &ss, &[]),
            vec![
                Plan::Send(2, "b31|p1".into()),
                Plan::Send(5, "gpu2|p3".into())
            ]
        );
        // a session that was just given a task isn't idle yet
        assert_eq!(
            plan(&tasks, &ss, &["b31|p1".into()]),
            vec![Plan::Send(5, "gpu2|p3".into())]
        );
        // --new tasks start a session
        assert_eq!(
            plan(&[t(9, "b31", None, None, true)], &ss, &[]),
            vec![Plan::Start(9)]
        );
        // nothing queued, nothing planned
        let mut sent = t(1, "b31", None, None, false);
        sent.state = "sent".into();
        assert!(plan(&[sent], &ss, &[]).is_empty());
    }

    #[test]
    fn offline_messages_wait_for_the_moment_they_were_written_for() {
        let mut a = s("b31", "p1", "idle", "/srv", "claude");
        a.seq = 7;
        let ss = vec![&a];
        // queued offline while it was asking (seq 5): it has moved on → held
        let mut q = t(1, "b31", None, None, false);
        q.seq = Some(5);
        q.seen = Some("asking".into());
        match &plan(std::slice::from_ref(&q), &ss, &[])[..] {
            [Plan::Hold(1, why)] => assert!(why.contains("aleph queue send 1"), "{why}"),
            p => panic!("{p:?}"),
        }
        // still the same moment → sent
        q.seq = Some(7);
        assert_eq!(
            plan(&[q.clone()], &ss, &[]),
            vec![Plan::Send(1, "b31|p1".into())]
        );
        // queued while it worked: finishing was the point → sent
        q.seq = Some(5);
        q.seen = Some("working".into());
        assert_eq!(
            plan(&[q.clone()], &ss, &[]),
            vec![Plan::Send(1, "b31|p1".into())]
        );
        // queued online (no seq) → sent
        q.seq = None;
        assert_eq!(plan(&[q], &ss, &[]), vec![Plan::Send(1, "b31|p1".into())]);
    }

    #[test]
    fn folder_match_is_by_path_component() {
        let task = t(1, "h", Some("/srv/px"), None, false);
        assert!(fits(&task, &s("h", "p", "idle", "/srv/px", "")));
        assert!(fits(&task, &s("h", "p", "idle", "/srv/px/src", "")));
        assert!(!fits(&task, &s("h", "p", "idle", "/srv/vd2", "")));
        // agents match by id, whatever herdr calls them
        let task = t(2, "h", None, Some("opencode"), false);
        assert!(fits(&task, &s("h", "p", "idle", "/x", "OpenCode")));
        assert!(!fits(&task, &s("h", "p", "idle", "/x", "claude")));
    }

    #[test]
    fn the_file_is_versioned_and_old_ones_still_read() {
        let one = t(3, "b31", None, None, false);
        let old = Value::Array(vec![one.to_json()]).to_string();
        let new = json!({"v": VERSION, "tasks": [one.to_json()]}).to_string();
        assert_eq!(parse(&old), vec![one.clone()]);
        assert_eq!(parse(&new), vec![one]);
        assert!(parse("{not json").is_empty());
    }

    #[test]
    fn json_roundtrip() {
        let mut x = t(7, "b31", Some("/d"), Some("codex"), true);
        x.session = Some("ignite".into());
        assert_eq!(Task::from_json(&x.to_json()), Some(x));
        assert!(parse("garbage").is_empty());
    }
}
