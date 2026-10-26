//! Aleph's API: one table of methods for every front end — the daemon's
//! socket (`aleph api`, `ssh vps aleph api --stdio`), the HTTP API and web
//! console, Raycast, and the dashboard.
//!
//! Each method names the least scope a caller needs: `read` (look),
//! `reply` (answer, send, queue) or `control` (start, stop, settings). The
//! local socket is this user's own (0600), so it gets `control`. Callers
//! from elsewhere (`Ctx::remote`) are also held to `remote.allow` /
//! `remote.deny` for every session they act on, and audited in the Inbox.

use crate::config::{Config, Target};
use crate::daemon::Shared;
use crate::events::{self, Event, Kind};
use crate::herdr::{self, Herdr, Session};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Read,
    Reply,
    Control,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Reply => "reply",
            Scope::Control => "control",
        }
    }
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "read" => Some(Scope::Read),
            "reply" => Some(Scope::Reply),
            "control" => Some(Scope::Control),
            _ => None,
        }
    }
}

pub struct Method {
    pub name: &'static str,
    pub scope: Scope,
    pub params: &'static str,
    pub help: &'static str,
}

const fn m(name: &'static str, scope: Scope, params: &'static str, help: &'static str) -> Method {
    Method {
        name,
        scope,
        params,
        help,
    }
}

use Scope::{Control, Read, Reply};

pub const METHODS: &[Method] = &[
    m("daemon.ping", Read, "", "is it up (version, pid)"),
    m(
        "daemon.status",
        Read,
        "",
        "version, pid, uptime, whether it runs the hub, machines",
    ),
    m("api.methods", Read, "", "this list"),
    m(
        "state.snapshot",
        Read,
        "",
        "every machine and its sessions (fleet.json's shape)",
    ),
    m(
        "events.subscribe",
        Read,
        "{state?: bool, kinds?: [kind], since?: t, topics?: [topic] | \"all\"}",
        "stays open: {event} and {state} lines as things happen, and {changed: topic} when inbox, queue, notes, usage, agents, services, doctor, config or history moved",
    ),
    m("session.list", Read, "{host?}", "sessions, flat"),
    m(
        "screen.read",
        Read,
        "{session, lines?, ansi?: bool}",
        "a session's recent output (and its question, when it asks one); ansi: aleph peek --json --ansi's answer (colours, the keys that answer yes, a limit on screen) from a read that never scrolls",
    ),
    m(
        "stream.open",
        Read,
        "{session, lines?}",
        "stays open: {screen} lines as a session's screen changes, {ended} when it does",
    ),
    m(
        "inbox.list",
        Read,
        "{n?, host?, kind?, unread?}",
        "recent events, newest first",
    ),
    m(
        "inbox.read",
        Reply,
        "{t?}",
        "mark read up to t (default: now)",
    ),
    m(
        "inbox.dismiss",
        Reply,
        "{ids?: [id], kinds?: [kind], ended?: bool, delete?: bool}",
        "archive events (delete: for good) — by id, or every one of those kinds and/or from sessions that have ended",
    ),
    m(
        "session.send",
        Reply,
        "{session, text, wait?, until?: [state], timeout_s?}",
        "prompt a session",
    ),
    m(
        "session.keys",
        Reply,
        "{session, keys: [key]}",
        "send keys (1, enter, esc, up, …)",
    ),
    m(
        "session.approve",
        Reply,
        "{session}",
        "answer its question with the yes option",
    ),
    m(
        "session.deny",
        Reply,
        "{session}",
        "answer its question with the no option (or esc)",
    ),
    m("queue.list", Read, "", "queued tasks"),
    m(
        "queue.add",
        Reply,
        "{host, prompt, to?, dir?, agent?, new?, after?}",
        "queue a task (after: idle | 30m | 14:30)",
    ),
    m("queue.rm", Reply, "{id}", "drop a queued task"),
    m(
        "session.new",
        Control,
        "{host, agent?, dir?, name?, prompt?, args?: [arg], worktree?}",
        "start a session",
    ),
    m(
        "session.close",
        Control,
        "{session, worktree?, now?}",
        "stop it and close its pane after close.grace (5s; the agent runs until then): {pending, deadline} — session.close.undo keeps it; now: at once",
    ),
    m(
        "session.close.undo",
        Control,
        "{session?}",
        "keep a session whose close is waiting out its grace (no session: the last close asked)",
    ),
    m("session.rename", Control, "{session, name}", "rename it"),
    m("preset.list", Read, "", "presets"),
    m(
        "preset.show",
        Read,
        "{name}",
        "one preset, and where it would start",
    ),
    m("preset.launch", Control, "{name, reuse?}", "start a preset"),
    m("hosts.list", Read, "", "the machines in the fleet"),
    m(
        "agents.list",
        Read,
        "{host?, local?: bool, refresh?: bool}",
        "the agents on each machine (aleph agents --json's shape), from what the daemon keeps; refresh: look again now",
    ),
    m(
        "services.list",
        Read,
        "{host?}",
        "LLM servers and AI gateways on each machine (aleph services --json's shape), rescanned while someone asks",
    ),
    m(
        "doctor.get",
        Read,
        "{host?, refresh?: bool}",
        "the last Doctor report (aleph doctor --json's shape, with at and running); an old one starts a new run, its sections arriving as they finish (topic doctor)",
    ),
    m("notes.list", Read, "", "every session note"),
    m("note.set", Reply, "{session, text?}", "a session's note (no text: clear it)"),
    m("history.list", Read, "{n?}", "the sessions you started, newest first"),
    m("recent.list", Read, "{host?}", "recent folders, best first"),
    m(
        "config.list",
        Read,
        "",
        "every setting and its value (secret values need the control scope)",
    ),
    m("usage.sources", Read, "{host?}", "what the analytics read on each machine"),
    m(
        "usage.now",
        Read,
        "",
        "the daemon's usage summary, refreshed every minute: today, week and 30 days (tokens, $ priced as API usage); by agent, machine and account; quota left per account (plan, monthly price); each machine's freshness",
    ),
    m(
        "daemon.facts",
        Read,
        "",
        "the daemon as its service manager sees it (aleph daemon status --json)",
    ),
    m("tokens.list", Control, "", "the paired devices"),
    m("queue.clear", Control, "{host?}", "drop the queued tasks (one machine's)"),
    m(
        "notify.mute",
        Reply,
        "{session, off?: bool}",
        "a session's notifications muted for an hour (off: unmuted)",
    ),
    m(
        "hosts.refresh",
        Reply,
        "{host, what?, herdr?, agents?, installed?, failed?, probed?}",
        "a machine changed (it was just set up): look at it again now — its sessions, herdr's version, its agents",
    ),
    m(
        "hosts.start_herdr",
        Control,
        "{host}",
        "start herdr's server on a machine where it stopped (the sessions Aleph remembers there come back too)",
    ),
    m(
        "stats.get",
        Read,
        "{host?, brief?: bool, procs?: bool}",
        "a machine's live stats (every machine's without host), from a stream the daemon keeps while asked (aleph stats --json's shape; brief: just what a row shows; procs: false leaves the process lists out)",
    ),
    m(
        "usage.query",
        Read,
        "{range?, by?, split?, compare?: bool, host?, agent?, account?, model?, project?}",
        "tokens, requests, prompts, time and cost (range: 24h | 3d | 7d | 30d | 6mo | all | today | this-week | this-month | A..B; by and split: host | agent | account | model | effort | project | day | hour | session; filters: a string, a comma list or an array; account: an id, email or nickname; compare: the same length before as previous)",
    ),
    m(
        "usage.stats",
        Read,
        "{range?, host?, agent?, account?, model?, project?, top?}",
        "per prompt and per session: cost, time, tokens and requests as distributions (n, mean, p50, p90, p99, max, histogram buckets), the costliest prompts and sessions, what no prompt holds",
    ),
    m(
        "usage.explore",
        Read,
        "{range?, split?, host?, agent?, account?, model?, project?}",
        "what Usage's screens show beyond the totals: the cost broken up by split (account by default; agent, host, model, project), the days stacked by it, this period against the one before, a day's tokens, requests, prompts, active hours and cache hits, the model mix, the week's hours, the last 12 weeks, the tools, the plans, the stats and the insights",
    ),
    m(
        "timeline.recent",
        Read,
        "{n?}",
        "the latest agent conversations with a timeline (host, agent, session, last activity; its project, account, prompts, requests, tokens and $ from the usage store)",
    ),
    m(
        "timeline.get",
        Read,
        "{session?: id | prefix | last, host?, fold?: smart|open|folded, open?: {row id: bool}, running?: bool}",
        "one agent conversation as a tree: its lanes (subagents), prompts, turns, tool calls, compactions and errors, each with time, tokens and $ (figures: them as one line); rows folded as asked, running: its last prompt still open (names and numbers from its logs, never what was said)",
    ),
    m(
        "timeline.words",
        Read,
        "{session, host?}",
        "a conversation's words, read now from its machine and never kept: each prompt's first line and each tool call's target (a file, a command), secrets masked; keyed like timeline.get's rows (key); Claude Code and Codex",
    ),
    m(
        "usage.projects",
        Read,
        "{range?}",
        "every project (a repo wherever it was worked on: its remote, else its first commit; worktrees and subfolders folded; analytics.projects.<name> rules) with its machines, folders and usage (30 days by default)",
    ),
    m(
        "usage.accounts",
        Read,
        "",
        "every account the agents were signed in to (agent, label, kind, plan, email, machines) and its last 30 days",
    ),
    m("net.status", Read, "", "this computer's connection"),
    m(
        "config.get",
        Read,
        "{key}",
        "a setting (secrets need control)",
    ),
    m("config.set", Control, "{key, value}", "change a setting"),
    m("config.unset", Control, "{key}", "back to the default"),
    // a showcase build's presenter sync (showcase.sync): what the presenter's
    // dashboard shows, for Raycast and the web console to follow
    #[cfg(feature = "showcase")]
    m(
        "presenter.get",
        Read,
        "",
        "what the presenter's dashboard shows: its view, the machine and the session selected (on: false while showcase.sync is off); topic presenter",
    ),
    #[cfg(feature = "showcase")]
    m(
        "presenter.set",
        Control,
        "{view, host?, pane_id?, name?} | {on: false}",
        "a dashboard on this computer says what it shows (presenter sync)",
    ),
];

pub fn method(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|m| m.name == name)
}

pub fn methods_json() -> Value {
    Value::Array(
        METHODS
            .iter()
            .map(|m| json!({"name": m.name, "scope": m.scope.as_str(), "params": m.params, "help": m.help}))
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Error {
        Error {
            code,
            message: message.into(),
        }
    }
}

fn bad(msg: impl Into<String>) -> Error {
    Error::new("invalid", msg)
}

fn failed(msg: impl Into<String>) -> Error {
    Error::new("failed", msg)
}

type R = Result<Value, Error>;

/// Who's calling, with what rights.
pub struct Ctx<'a> {
    pub cfg: &'a Config,
    /// the daemon's state (None: this process has no daemon loop)
    pub shared: Option<&'a Shared>,
    pub scope: Scope,
    /// from another device (web, phone, Raycast over HTTP): held to
    /// `remote.allow` / `remote.deny` and audited
    pub remote: bool,
    /// for the audit line: `web · pixel`, `socket`, …
    pub who: String,
}

impl<'a> Ctx<'a> {
    /// This user on this computer (the daemon's socket).
    pub fn local(cfg: &'a Config, shared: Option<&'a Shared>) -> Ctx<'a> {
        Ctx {
            cfg,
            shared,
            scope: Scope::Control,
            remote: false,
            who: "socket".into(),
        }
    }
}

/// Call one method.
pub fn call(ctx: &Ctx, name: &str, p: &Value) -> R {
    // showcase mode: a masked machine name sent back is the real one again
    let back;
    let p = if crate::showcase::on() && p.get("host").and_then(Value::as_str).is_some() {
        let mut v = p.clone();
        let h = v["host"].as_str().unwrap_or("").to_string();
        v["host"] = json!(crate::showcase::unmask(&h, &ctx.cfg.fleet()).into_owned());
        back = v;
        &back
    } else {
        p
    };
    let Some(m) = method(name) else {
        return Err(Error::new("unknown", format!("no such method: {name}")));
    };
    if ctx.scope < m.scope {
        return Err(Error::new(
            "forbidden",
            format!("{name} needs the {} scope", m.scope.as_str()),
        ));
    }
    match name {
        "daemon.ping" => Ok(
            json!({"pong": true, "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id()}),
        ),
        "daemon.status" => Ok(status(ctx)),
        "api.methods" => Ok(methods_json()),
        "state.snapshot" => Ok(snapshot(ctx)),
        "events.subscribe" | "stream.open" => Err(bad(format!(
            "{name} needs its own connection (it stays open)"
        ))),
        "session.list" => {
            let host = p.get("host").and_then(Value::as_str);
            Ok(Value::Array(
                sessions(ctx)
                    .iter()
                    .filter(|s| host.is_none_or(|h| s.host == h))
                    .map(session_json)
                    .collect(),
            ))
        }
        "screen.read" => {
            let (t, s) = resolve(ctx, p)?;
            let n = p
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .clamp(1, 2000) as usize;
            let h = Herdr::new(&t);
            // ansi: aleph peek --json --ansi's answer (colours, the keys that
            // answer yes, a usage limit on screen) — a read that never scrolls
            if p.get("ansi").and_then(Value::as_bool).unwrap_or(false) {
                let (v, _, _) = crate::statscmd::peek_json(&h, s, n, true).map_err(failed)?;
                return Ok(v);
            }
            let text = h.read_pane(&s.pane_id, n).map_err(failed)?;
            // showcase: redacted or hidden on the way out (`showcase.screens`)
            let text = crate::showcase::screen_text(&text);
            let mut v = json!({"host": s.host, "name": s.name, "status": s.status, "text": text});
            if s.blocked() {
                if let Ok(q) = h.read(&s.pane_id, "detection", 0, false) {
                    v["question"] = json!(crate::showcase::screen_text(q.trim_end()));
                }
            }
            Ok(v)
        }
        "inbox.list" => Ok(inbox_list(p)),
        "inbox.read" => {
            let t = p
                .get("t")
                .and_then(Value::as_u64)
                .unwrap_or_else(crate::cache::now);
            events::set_seen(t);
            Ok(json!({"seen": t}))
        }
        "inbox.dismiss" => inbox_dismiss(ctx, p),
        "session.send" => send(ctx, p),
        "session.keys" => {
            let (t, s) = act_on(ctx, p)?;
            let keys: Vec<String> = p
                .get("keys")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("keys: a list, like [\"1\"] or [\"enter\"]"))?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            if keys.is_empty() {
                return Err(bad("keys: at least one"));
            }
            let k: Vec<&str> = keys.iter().map(String::as_str).collect();
            Herdr::new(&t).send_keys(&s.pane_id, &k).map_err(failed)?;
            audit(ctx, &s, &format!("keys {}", keys.join(" ")), "");
            Ok(json!({"sent": keys}))
        }
        "session.approve" => {
            let (t, s) = act_on(ctx, p)?;
            if !s.blocked() {
                return Err(failed(format!(
                    "{} isn't asking anything right now ({})",
                    s.display(),
                    s.status
                )));
            }
            let h = Herdr::new(&t);
            if ctx.remote {
                let q = h
                    .read(&s.pane_id, "detection", 0, false)
                    .unwrap_or_default();
                if let Some(d) = crate::permit::terminal_only(ctx.cfg, &q) {
                    return Err(Error::new(
                        "forbidden",
                        format!("that question is on the approve_deny list ({d}): answer it at a terminal"),
                    ));
                }
            }
            let keys = h.approve(&s.pane_id).map_err(failed)?;
            audit(ctx, &s, "approved", &keys.join(" "));
            Ok(json!({"host": s.host, "name": s.name, "keys": keys}))
        }
        "session.deny" => {
            let (t, s) = act_on(ctx, p)?;
            if !s.blocked() {
                return Err(failed(format!(
                    "{} isn't asking anything right now ({})",
                    s.display(),
                    s.status
                )));
            }
            let h = Herdr::new(&t);
            let q = h
                .read(&s.pane_id, "detection", 0, false)
                .unwrap_or_default();
            let keys = crate::actions::deny_keys(&q);
            let k: Vec<&str> = keys.iter().map(String::as_str).collect();
            h.send_keys(&s.pane_id, &k).map_err(failed)?;
            audit(ctx, &s, "denied", &keys.join(" "));
            Ok(json!({"host": s.host, "name": s.name, "keys": keys}))
        }
        "queue.list" => Ok(Value::Array(
            crate::queue::load()
                .iter()
                .map(crate::queue::Task::to_json)
                .collect(),
        )),
        "queue.add" => queue_add(ctx, p),
        "queue.rm" => {
            let id = p
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad("id: a number"))?;
            Ok(json!({"removed": crate::queue::remove(id)}))
        }
        "session.new" => new_session(ctx, p),
        "session.close" => {
            let (t, s) = act_on(ctx, p)?;
            // after the grace: the hub carries it out (`closing.rs`)
            let g = crate::closing::grace(ctx.cfg);
            if p.get("now").and_then(Value::as_bool) != Some(true) && !g.is_zero() {
                let wt = p.get("worktree").and_then(Value::as_bool) == Some(true);
                let by = format!("api ({})", ctx.who);
                let e = crate::closing::request(std::slice::from_ref(&s), g, wt, &by).remove(0);
                audit(ctx, &s, &format!("closing in {}", crate::closing::grace_text(g)), "");
                return Ok(crate::closing::pending_json(&e, g));
            }
            let _ = crate::closing::undo(crate::closing::Sel::Session(&s.host, &s.pane_id));
            let h = Herdr::new(&t);
            h.close_pane(&s.pane_id).map_err(failed)?;
            crate::restore::mark_closed(&t.ssh, &s.name);
            let mut out = json!({"closed": true, "host": s.host, "name": s.name});
            if p.get("worktree").and_then(Value::as_bool) == Some(true) {
                out["worktree"] = match h.remove_worktree(&s.workspace_id) {
                    Ok(()) => json!("removed"),
                    Err(e) => json!(format!("kept: {e}")),
                };
            }
            audit(ctx, &s, "closed", "");
            Ok(out)
        }
        "session.close.undo" => close_undo(ctx, p),
        "session.rename" => {
            let (t, s) = act_on(ctx, p)?;
            let new = str_of(p, "name")?;
            if !crate::names::valid(new) {
                return Err(bad(
                    "names are lowercase letters, digits, - and _ (start with a letter, ≤ 32)",
                ));
            }
            let h = Herdr::new(&t);
            let label = crate::cli::renamed_label(&s.label, new);
            h.rename_tab(&s.tab_id, &label).map_err(failed)?;
            let _ = h.rename_agent(&s.pane_id, new);
            let _ = h.report_tokens(&s.pane_id, &[("name", new)]);
            audit(ctx, &s, &format!("renamed to {new}"), "");
            Ok(json!({"label": label}))
        }
        "preset.list" => Ok(Value::Array(
            crate::presets::all(ctx.cfg)
                .iter()
                .map(crate::presets::to_json)
                .collect(),
        )),
        "preset.show" => {
            let name = str_of(p, "name")?;
            let pr = crate::presets::get(ctx.cfg, name)
                .ok_or_else(|| Error::new("not_found", format!("no preset {name}")))?;
            Ok(crate::presets::to_json(&pr))
        }
        "preset.launch" => preset_launch(ctx, p),
        "hosts.list" => Ok(hosts(ctx)),
        "hosts.refresh" => hosts_refresh(ctx, p),
        "agents.list" => {
            let mut hosts = match p.get("host").and_then(Value::as_str) {
                Some(h) => vec![h.to_string()],
                None => ctx.cfg.fleet(),
            };
            if p.get("local").and_then(Value::as_bool).unwrap_or(false)
                && !hosts.iter().any(|h| crate::sshcfg::is_local(h))
            {
                hosts.push(crate::sshcfg::LOCAL.into());
            }
            let refresh = p.get("refresh").and_then(Value::as_bool).unwrap_or(false);
            Ok(crate::keep::agents(&hosts, refresh))
        }
        "services.list" => {
            let hosts = match p.get("host").and_then(Value::as_str) {
                Some(h) => vec![h.to_string()],
                None => ctx.cfg.fleet(),
            };
            Ok(crate::keep::services(ctx.cfg, &hosts))
        }
        "doctor.get" => Ok(crate::keep::doctor(
            ctx.cfg,
            p.get("host").and_then(Value::as_str),
            p.get("refresh").and_then(Value::as_bool).unwrap_or(false),
        )),
        "notes.list" => Ok(Value::Array(
            crate::notes::all()
                .iter()
                .map(|(h, n, x)| json!({"host": h, "name": n, "text": x.text, "tags": x.tags, "t": x.t}))
                .collect(),
        )),
        "note.set" => {
            let (_, s) = resolve(ctx, p)?;
            let text = p.get("text").and_then(Value::as_str).unwrap_or("");
            crate::notes::set(&s.host, &s.name, text)
                .map_err(|e| failed(format!("couldn't keep the note: {e}")))?;
            Ok(json!({"host": s.host, "name": s.name, "text": text}))
        }
        "history.list" => {
            let n = p.get("n").and_then(Value::as_u64).unwrap_or(30).min(5000) as usize;
            Ok(Value::Array(
                crate::history::load(n).iter().map(|e| e.to_json()).collect(),
            ))
        }
        "recent.list" => Ok(Value::Array(
            crate::history::recent_dirs(p.get("host").and_then(Value::as_str))
                .iter()
                .map(|r| json!({"host": r.host, "dir": r.dir, "count": r.count, "last": r.last, "score": r.score}))
                .collect(),
        )),
        "config.list" => {
            let mut v = crate::cli::settings_json(ctx.cfg);
            if ctx.scope < Scope::Control {
                if let Some(o) = v.as_object_mut() {
                    for (k, x) in o.iter_mut() {
                        let secret = crate::configcmd::is_secret(k)
                            && x.as_str().is_some_and(|s| !s.is_empty() && !s.starts_with("secret:"));
                        if secret {
                            *x = json!("••••");
                        }
                    }
                }
            }
            Ok(v)
        }
        "usage.sources" => {
            let hosts = match p.get("host").and_then(Value::as_str) {
                Some(h) => vec![h.to_string()],
                None => ctx.cfg.fleet(),
            };
            Ok(crate::usage::cmd::sources_json(ctx.cfg, &hosts))
        }
        "daemon.facts" => Ok(crate::keep::facts(ctx.cfg)),
        "usage.now" => Ok(crate::usage::now::get(ctx.cfg)),
        "usage.accounts" => Ok(crate::usage::cmd::accounts_json(ctx.cfg)),
        "timeline.recent" => {
            let n = p.get("n").and_then(Value::as_u64).unwrap_or(20).min(500) as usize;
            Ok(json!(crate::timeline::recent_json(ctx.cfg, n)))
        }
        "timeline.words" => {
            if ctx.cfg.get_or("timeline.words") == "off" {
                return Ok(json!({}));
            }
            let q = p.get("session").and_then(Value::as_str).unwrap_or("last");
            let host = p.get("host").and_then(Value::as_str);
            let hit = crate::timeline::recent(1000)
                .into_iter()
                .find(|(h, _, s, _)| (q == "last" || s.starts_with(q)) && host.is_none_or(|x| x == h))
                .ok_or_else(|| bad(format!("no timeline for {q}")))?;
            let w = crate::usage::steps::words(&hit.0, &hit.1, &hit.2).map_err(bad)?;
            Ok(json!(w))
        }
        "timeline.get" => {
            let q = p.get("session").and_then(Value::as_str).unwrap_or("last");
            let host = p.get("host").and_then(Value::as_str);
            let rec = crate::timeline::recent(1000);
            let hit = rec
                .iter()
                .find(|(h, _, s, _)| (q == "last" || s.starts_with(q)) && host.is_none_or(|x| x == h))
                .cloned()
                .ok_or_else(|| bad(format!("no timeline for {q} (timeline.recent lists them)")))?;
            let t = crate::timeline::load(ctx.cfg, &hit.0, &hit.1, &hit.2);
            let fold = crate::timeline::Fold::parse(
                p.get("fold")
                    .and_then(Value::as_str)
                    .unwrap_or(&ctx.cfg.get_or("timeline.fold")),
            );
            let open: std::collections::HashMap<String, bool> = p
                .get("open")
                .and_then(Value::as_object)
                .map(|o| o.iter().filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b))).collect())
                .unwrap_or_default();
            let running = p.get("running").and_then(Value::as_bool).unwrap_or(false);
            let rows = crate::timeline::rows(&t, fold, &open, running);
            Ok(crate::timeline::to_json(&t, &rows))
        }
        "usage.projects" => {
            let range = match p.get("range").and_then(Value::as_str) {
                Some(r) => crate::usage::Range::parse(r).ok_or_else(|| bad(format!("range: {r}?")))?,
                None => crate::usage::Range::days(30),
            };
            Ok(crate::usage::cmd::projects_json(ctx.cfg, range))
        }
        "usage.explore" => {
            let (range, f) = usage_scope(ctx, p)?;
            let split = match p.get("split").and_then(Value::as_str) {
                Some(x) => crate::usage::By::parse(x)
                    .filter(|b| *b != crate::usage::By::Total)
                    .ok_or_else(|| bad(format!("split: {x}? (account, agent, host, model, project)")))?,
                None => crate::usage::By::Account,
            };
            Ok(crate::usage::cmd::explore_json(ctx.cfg, range, &f, split))
        }
        "usage.stats" => {
            let (range, f) = usage_scope(ctx, p)?;
            let top = p.get("top").and_then(Value::as_u64).unwrap_or(10).min(100) as usize;
            let mut v = crate::usage::cmd::stats_json(&crate::usage::stats(ctx.cfg, range, &f, top));
            v["range"] = json!(range.label());
            Ok(v)
        }
        "tokens.list" => Ok(crate::devices::tokens_json()),
        "queue.clear" => {
            let host = p.get("host").and_then(Value::as_str).map(str::to_string);
            let n = crate::queue::update(|v| {
                let n = v.len();
                v.retain(|t| t.state != "queued" || host.as_deref().is_some_and(|h| h != t.host));
                n - v.len()
            });
            Ok(json!({"cleared": n}))
        }
        "notify.mute" => {
            let (_, s) = resolve(ctx, p)?;
            let off = p.get("off").and_then(Value::as_bool).unwrap_or(false);
            let on = crate::router::mute(&s.host, &s.name, if off { 0 } else { 3600 });
            Ok(json!({"host": s.host, "name": s.name, "muted": on}))
        }
        "hosts.start_herdr" => {
            let host = str_of(p, "host")?.to_string();
            if !ctx.cfg.fleet().contains(&host) {
                return Err(Error::new("not_found", format!("no machine {host}")));
            }
            let m = crate::doctor::start_herdr(ctx.cfg, &host).map_err(failed)?;
            if let Some(sh) = ctx.shared {
                sh.ask_refresh(&host);
            }
            Ok(json!({"host": host, "message": m}))
        }
        "usage.query" => usage(ctx, p),
        "stats.get" => Ok(stats(ctx, p)),
        "net.status" => {
            let t = crate::net::target(ctx.cfg);
            let rtt = crate::net::probe(&t, Duration::from_millis(1500));
            Ok(json!({
                "target": t,
                "online": rtt.is_some(),
                "rtt_ms": rtt.map(|d| d.as_millis() as u64),
                "route": crate::net::default_route(),
            }))
        }
        "config.get" => {
            let key = str_of(p, "key")?;
            let v = ctx.cfg.get(key);
            let secret = crate::configcmd::is_secret(key)
                && v.as_deref().is_some_and(|v| !v.starts_with("secret:"));
            if secret && ctx.scope < Scope::Control {
                return Err(Error::new(
                    "forbidden",
                    format!("{key} is a secret: it needs the control scope"),
                ));
            }
            Ok(
                json!({"key": key, "value": v, "default": crate::config::known(key).map(|k| k.default)}),
            )
        }
        "config.set" => {
            let key = str_of(p, "key")?;
            let val = match p.get("value") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => return Err(bad("value: the new value")),
                Some(v) => v.to_string(),
            };
            let mut c = ctx.cfg.clone();
            c.set(key, &val).map_err(|e| failed(e.to_string()))?;
            Ok(json!({"key": key, "value": val}))
        }
        "config.unset" => {
            let key = str_of(p, "key")?;
            let mut c = ctx.cfg.clone();
            c.unset(key).map_err(|e| failed(e.to_string()))?;
            Ok(json!({"key": key, "value": Value::Null}))
        }
        // a showcase build's own methods (presenter sync)
        _ => match crate::showcase::method(ctx.cfg, name, p, ctx.remote) {
            Some(r) => r.map_err(failed),
            None => Err(Error::new("unknown", format!("no such method: {name}"))),
        },
    }
}

fn str_of<'v>(p: &'v Value, k: &str) -> Result<&'v str, Error> {
    p.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad(format!("{k}: missing")))
}

fn status(ctx: &Ctx) -> Value {
    let mut v = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "socket": crate::daemon::sock_path(),
        "hosts": ctx.cfg.fleet(),
        "hub_holder": crate::auto::HubLock::holder(),
    });
    if let Some(sh) = ctx.shared {
        v["kind"] = json!(sh.kind);
        v["started_at"] = json!(sh.started);
        v["uptime_s"] = json!(sh.uptime().as_secs());
        v["hub"] = json!(sh.hub());
        v["gen"] = json!(sh.generation());
        v["http"] = json!(sh.http());
    }
    v
}

/// `inbox.dismiss`: by ids, or by kinds and/or "from ended sessions" (a
/// session no longer on a machine that answers), archived or deleted.
fn inbox_dismiss(ctx: &Ctx, p: &Value) -> R {
    let list = |k: &str| -> Vec<String> {
        p.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut ids = list("ids");
    let kinds = list("kinds");
    let ended = p.get("ended").and_then(Value::as_bool).unwrap_or(false);
    if ids.is_empty() && kinds.is_empty() && !ended {
        return Err(bad("ids, kinds or ended"));
    }
    if !kinds.is_empty() || ended {
        let snap = snapshot(ctx);
        let live = sessions_of(&snap);
        let up = |h: &str| {
            snap.get("hosts")
                .and_then(Value::as_array)
                .is_some_and(|a| {
                    a.iter().any(|x| {
                        x.get("alias").and_then(Value::as_str) == Some(h)
                            && x.get("reachable").and_then(Value::as_bool) == Some(true)
                    })
                })
        };
        let dismissed = events::dismissed();
        for e in events::load_inbox(events::CAP) {
            if dismissed.contains(&events::key(&e)) {
                continue;
            }
            let gone = !e.name.is_empty()
                && up(&e.host)
                && !live.iter().any(|s| {
                    s.host == e.host
                        && (s.name == e.name || (!e.pane.is_empty() && s.pane_id == e.pane))
                });
            if kinds.iter().any(|k| k == e.kind.as_str()) || (ended && gone) {
                ids.push(e.id());
            }
        }
    }
    if p.get("delete").and_then(Value::as_bool).unwrap_or(false) {
        let evs = events::load(events::CAP);
        let keys: std::collections::HashSet<String> = evs
            .iter()
            .filter(|e| ids.contains(&e.id()))
            .map(events::key)
            .collect();
        return Ok(json!({"deleted": events::remove(&keys)}));
    }
    Ok(json!({"dismissed": events::dismiss_ids(&ids)}))
}

fn snapshot(ctx: &Ctx) -> Value {
    let mut v = match ctx.shared {
        Some(sh) => {
            let (g, mut v) = sh.get();
            v["gen"] = json!(g);
            v
        }
        None => crate::cli::fleet_now(ctx.cfg, &None, "Looking at every machine").to_json(ctx.cfg),
    };
    // showcase mode says so, as the subscribers' snapshots do (masked on
    // the way out like every answer)
    crate::showcase::mark(&mut v);
    v
}

/// Sessions in a fleet snapshot.
pub fn sessions_of(snap: &Value) -> Vec<Session> {
    snap.get("hosts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|h| {
            h.get("sessions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(crate::fleet::session_from_json)
        .collect()
}

fn sessions(ctx: &Ctx) -> Vec<Session> {
    sessions_of(&snapshot(ctx))
}

fn session_json(s: &Session) -> Value {
    let mut v = crate::fleet::session_to_json(s);
    v["display"] = json!(s.display());
    v
}

/// The session `params.session` names (`name`, `host:name`, or a pane id):
/// from the daemon's snapshot, or that machine asked fresh when the snapshot
/// doesn't have it.
pub fn resolve(ctx: &Ctx, p: &Value) -> Result<(Target, Session), Error> {
    let all = sessions(ctx);
    // showcase mode: a client that saw the masked name sends it back
    let q = crate::showcase::unmask(str_of(p, "session")?, &selectors(&all)).into_owned();
    let q = q.as_str();
    let found = herdr::pick(&all, q)
        .or_else(|| all.iter().find(|s| s.pane_id == q))
        .cloned();
    let s = match found {
        Some(s) => s,
        None => {
            // maybe it just started: ask that machine (or all of them)
            let flag = q.split_once(':').map(|(h, _)| h.to_string());
            let (_, s) = crate::cli::find_quiet(ctx.cfg, &flag, q)
                .ok_or_else(|| Error::new("not_found", format!("no session {q}")))?;
            s
        }
    };
    let t = ctx.cfg.target_for(&s.host, herdr::machine_for);
    Ok((t, s))
}

/// Every way a session can be named (`host:name`, `host:pane`, its name): what
/// a masked selector is matched against.
fn selectors(all: &[Session]) -> Vec<String> {
    if !crate::showcase::on() {
        return Vec::new();
    }
    all.iter()
        .flat_map(|s| {
            [
                format!("{}:{}", s.host, s.name),
                format!("{}:{}", s.host, s.pane_id),
                format!("{}:{}", s.host, s.display()),
                s.name.clone(),
                s.display().to_string(),
            ]
        })
        .collect()
}

/// `resolve`, then the remote-control check for callers from elsewhere.
fn act_on(ctx: &Ctx, p: &Value) -> Result<(Target, Session), Error> {
    let (t, s) = resolve(ctx, p)?;
    allowed(ctx, &s.host, &s.agent)?;
    Ok((t, s))
}

fn allowed(ctx: &Ctx, host: &str, agent: &str) -> Result<(), Error> {
    if ctx.remote && !crate::permit::allowed(ctx.cfg, host, agent) {
        return Err(Error::new(
            "forbidden",
            format!("remote control isn't on for {host}:{agent} (remote.allow)"),
        ));
    }
    Ok(())
}

/// Remote actions go in the Inbox (`remote` events).
fn audit(ctx: &Ctx, s: &Session, what: &str, detail: &str) {
    if !ctx.remote {
        return;
    }
    events::append(Event {
        t: crate::cache::now(),
        host: s.host.clone(),
        name: s.name.clone(),
        kind: Kind::Remote,
        title: format!("{what} from {}", ctx.who),
        agent: s.agent.clone(),
        source: "aleph".into(),
        body: detail.to_string(),
        ..Default::default()
    });
}

/// `session.close.undo`: keep it (it was in its grace). Named by what the
/// client called it (`b31:ignite`, a pane id) — the session may have gone
/// from the snapshot.
fn close_undo(ctx: &Ctx, p: &Value) -> R {
    use crate::closing::{undo, Entry, Sel};
    let q = p.get("session").and_then(Value::as_str).map(|q| {
        // showcase mode: a client that saw the masked name sends it back
        crate::showcase::unmask(q, &selectors(&sessions(ctx))).into_owned()
    });
    let sel = match &q {
        Some(q) => Sel::Query(q),
        None => Sel::Latest,
    };
    // (remote callers: only sessions remote control covers)
    if ctx.remote {
        for e in crate::closing::load()
            .iter()
            .filter(|e| q.as_deref().is_none_or(|q| e.matches(q)))
        {
            allowed(ctx, &e.host, &e.agent)?;
        }
    }
    let (kept, late) = undo(sel);
    if kept.is_empty() {
        return Err(match (late.first(), &q) {
            (Some(e), _) => failed(format!("too late: {} is closing already", e.display)),
            (None, Some(q)) => Error::new("not_found", format!("{q} isn't closing")),
            (None, None) => Error::new("not_found", "nothing is closing"),
        });
    }
    if ctx.remote {
        for e in &kept {
            events::append(Event {
                t: crate::cache::now(),
                host: e.host.clone(),
                name: e.name.clone(),
                kind: Kind::Remote,
                title: format!("kept (close undone) from {}", ctx.who),
                agent: e.agent.clone(),
                source: "aleph".into(),
                ..Default::default()
            });
        }
    }
    Ok(json!({
        "kept": kept.iter().map(Entry::to_json).collect::<Vec<_>>(),
        "names": kept.iter().map(|e| e.display.clone()).collect::<Vec<_>>(),
    }))
}

fn send(ctx: &Ctx, p: &Value) -> R {
    let (t, s) = act_on(ctx, p)?;
    let text = str_of(p, "text")?;
    let h = Herdr::new(&t);
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(false) || p.get("until").is_some();
    audit(ctx, &s, "sent a message", &crate::ui::trunc(text, 140));
    if wait {
        let until: Vec<String> = p
            .get("until")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let timeout = p
            .get("timeout_s")
            .and_then(Value::as_u64)
            .map(Duration::from_secs);
        let st = h
            .prompt_wait(&s.pane_id, text, &until, timeout)
            .map_err(failed)?;
        return Ok(json!({"sent": true, "state": st}));
    }
    h.prompt(&s.pane_id, text).map_err(failed)?;
    Ok(json!({"sent": true}))
}

fn queue_add(ctx: &Ctx, p: &Value) -> R {
    let host = str_of(p, "host")?.to_string();
    let prompt = str_of(p, "prompt")?.to_string();
    let to = p.get("to").and_then(Value::as_str).map(str::to_string);
    let agent = p.get("agent").and_then(Value::as_str).map(str::to_string);
    if ctx.remote {
        // a remote queue entry acts on that agent (or any agent) there
        allowed(ctx, &host, agent.as_deref().unwrap_or("*"))?;
    }
    let after = match p.get("after").and_then(Value::as_str) {
        Some(w) => crate::queue::parse_after(w, crate::cache::now(), crate::ui::utc_offset())
            .map_err(bad)?,
        None => None,
    };
    let id = crate::queue::add(crate::queue::Task {
        host: host.clone(),
        prompt,
        to,
        dir: p.get("dir").and_then(Value::as_str).map(str::to_string),
        agent,
        new: p.get("new").and_then(Value::as_bool).unwrap_or(false),
        after,
        ..Default::default()
    });
    Ok(json!({"id": id}))
}

fn new_session(ctx: &Ctx, p: &Value) -> R {
    let host = str_of(p, "host")?;
    let mut rest: Vec<String> = Vec::new();
    for (k, flag) in [
        ("dir", "--dir"),
        ("name", "--name"),
        ("agent", "--agent"),
        ("prompt", "-m"),
    ] {
        if let Some(v) = p.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()) {
            rest.push(flag.into());
            rest.push(v.into());
        }
    }
    if p.get("worktree").and_then(Value::as_bool) == Some(true) {
        rest.push("--worktree".into());
    }
    if let Some(a) = p.get("args").and_then(Value::as_array) {
        rest.push("--".into());
        rest.extend(a.iter().filter_map(Value::as_str).map(str::to_string));
    }
    let mut na = crate::cli::parse_new(&rest, ctx.cfg).map_err(bad)?;
    allowed(ctx, host, &na.opts.agent_id())?;
    let t = ctx.cfg.target_for(host, herdr::machine_for);
    crate::cli::fill_names(&mut na.opts);
    let v = Herdr::new(&t)
        .new_session(&na.opts, &mut |_, _| {})
        .map_err(failed)?;
    crate::launch::after_start(&t, &na.opts, &v);
    if ctx.remote {
        if let Some(s) = v.first() {
            events::append(Event {
                t: crate::cache::now(),
                host: t.ssh.clone(),
                name: s.name.clone(),
                kind: Kind::Remote,
                title: format!("started from {}", ctx.who),
                agent: na.opts.agent_id(),
                source: "aleph".into(),
                ..Default::default()
            });
        }
    }
    Ok(json!({
        "host": t.ssh,
        "agent": na.opts.agent_id(),
        "sessions": v.iter().map(|s| json!({"name": s.name, "label": s.label, "pane_id": s.pane, "dir": s.dir})).collect::<Vec<_>>(),
        "name": v.first().map(|s| s.name.clone()),
        "pane_id": v.first().map(|s| s.pane.clone()),
    }))
}

fn preset_launch(ctx: &Ctx, p: &Value) -> R {
    let name = str_of(p, "name")?;
    let pr = crate::presets::get(ctx.cfg, name)
        .ok_or_else(|| Error::new("not_found", format!("no preset {name}")))?;
    let tags = crate::presets::fleet_tags(ctx.cfg);
    let plan = crate::presets::plan(&pr, &tags, ctx.cfg.get("ssh_host").as_deref(), &[]);
    if plan.launches.is_empty() {
        return Err(failed(format!("@{name}: nothing to start")));
    }
    if ctx.remote {
        for l in &plan.launches {
            allowed(ctx, &l.host, l.agent.as_deref().unwrap_or("*"))?;
        }
    }
    let reuse = p.get("reuse").and_then(Value::as_bool).unwrap_or(true);
    let (started, errors) = crate::presets::launch(ctx.cfg, &plan, reuse, &mut |_| {});
    Ok(json!({
        "started": started.iter().map(|(h, n, pane)| json!({"host": h, "name": n, "pane_id": pane})).collect::<Vec<_>>(),
        "errors": errors,
    }))
}

/// `hosts.refresh`: a machine changed. Drop what's cached about it, leave
/// the note for dashboards (`refresh.rs`), and have this process's loop look
/// at it again (the daemon's fleet, or a dashboard's).
fn hosts_refresh(ctx: &Ctx, p: &Value) -> R {
    let host = str_of(p, "host")?.to_string();
    // the setting may be newer than this process (a machine just added)
    let cfg = Config::load();
    let known = cfg.fleet().contains(&host)
        || ctx.cfg.fleet().contains(&host)
        || crate::sshcfg::aliases().contains(&host);
    if !known {
        return Err(Error::new("not_found", format!("no machine {host}")));
    }
    let mut n =
        crate::refresh::Note::from_json(p).unwrap_or_else(|| crate::refresh::Note::bare(&host));
    n.host = host.clone();
    if n.at == 0 {
        n.at = crate::refresh::now_ms();
    }
    crate::refresh::forget(&host, !n.probed);
    crate::keep::forget_doctor(&host);
    crate::refresh::mark(&n).map_err(|e| failed(format!("couldn't leave the note: {e}")))?;
    if let Some(sh) = ctx.shared {
        sh.ask_refresh(&host);
    }
    Ok(json!({
        "host": host,
        "by": ctx.shared.map(|s| s.kind).unwrap_or("aleph"),
        "in_fleet": cfg.fleet().contains(&host),
    }))
}

fn hosts(ctx: &Ctx) -> Value {
    let snap = snapshot(ctx);
    let live = |a: &str| {
        snap.get("hosts")
            .and_then(Value::as_array)
            .and_then(|v| {
                v.iter()
                    .find(|h| h.get("alias").and_then(Value::as_str) == Some(a))
            })
            .cloned()
    };
    Value::Array(
        ctx.cfg
            .fleet()
            .iter()
            .map(|a| {
                let h = live(a);
                let g = |k: &str| h.as_ref().and_then(|h| h.get(k)).cloned().unwrap_or(Value::Null);
                json!({
                    "alias": a,
                    "color": crate::theme::machine_hex(ctx.cfg, a),
                    "reachable": g("reachable"),
                    "latency_ms": g("latency_ms"),
                    "herdr_version": g("herdr_version"),
                    "sessions": h.as_ref().and_then(|h| h.get("sessions")).and_then(Value::as_array).map(Vec::len).unwrap_or(0),
                })
            })
            .collect(),
    )
}

/// `stats.get`: the asked machine's (or every machine's) latest live sample,
/// in `aleph stats --json`'s shape; the samplers run while someone asks.
fn stats(ctx: &Ctx, p: &Value) -> Value {
    let hosts: Vec<String> = match p.get("host").and_then(Value::as_str) {
        Some(h) => vec![h.to_string()],
        None => ctx.cfg.fleet(),
    };
    // every machine at once: each first ask waits for its own first sample
    let got: Vec<(String, crate::telemetry::Stats)> = std::thread::scope(|sc| {
        let hs: Vec<_> = hosts
            .iter()
            .map(|h| sc.spawn(move || (h.clone(), crate::livestats::get(h))))
            .collect();
        hs.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let brief = p.get("brief").and_then(Value::as_bool).unwrap_or(false);
    let procs = p.get("procs").and_then(Value::as_bool).unwrap_or(true);
    json!({
        "generated_at": crate::cache::now(),
        "hosts": got
            .iter()
            .map(|(h, s)| {
                let mut v = crate::statscmd::stats_json(h, &crate::theme::machine_hex(ctx.cfg, h), s);
                trim_stats(&mut v, brief, procs);
                v
            })
            .collect::<Vec<_>>(),
    })
}

/// A smaller stats answer: `brief` keeps what a list row shows (load, memory,
/// the GPUs' use); without `procs`, the process lists go.
fn trim_stats(v: &mut Value, brief: bool, procs: bool) {
    let Some(o) = v.as_object_mut() else {
        return;
    };
    if brief {
        const KEEP: &[&str] = &[
            "alias",
            "color",
            "reachable",
            "error",
            "os",
            "platform",
            "cpu",
            "load",
            "mem_used",
            "mem_total",
            "gpus",
        ];
        o.retain(|k, _| KEEP.contains(&k.as_str()));
        if let Some(gs) = o.get_mut("gpus").and_then(Value::as_array_mut) {
            for g in gs.iter_mut().filter_map(Value::as_object_mut) {
                g.retain(|k, _| {
                    ["index", "name", "vendor", "util", "mem_used", "mem_total"]
                        .contains(&k.as_str())
                });
            }
        }
    } else if !procs {
        o.remove("top");
        o.remove("gpu_procs");
    }
}

fn usage(ctx: &Ctx, p: &Value) -> R {
    use crate::usage::By;
    let (range, f) = usage_scope(ctx, p)?;
    let by = match p.get("by").and_then(Value::as_str) {
        Some(b) => By::parse(b).ok_or_else(|| {
            bad(format!(
                "by: {b}? (host, agent, account, model, effort, project, day, hour, session)"
            ))
        })?,
        None => By::Total,
    };
    let srows = crate::usage::rows(ctx.cfg, range, &f);
    let rows = crate::usage::group(&srows, by);
    let mut v = json!({
        "range": range.label(),
        "by": by.name(),
        "rows": rows.iter().map(|r| crate::usage::cmd::row_json(by, r)).collect::<Vec<_>>(),
        "limits": crate::usage::store::limits(),
    });
    // split: each row's own breakdown (agent → accounts, …)
    if let Some(t) = p.get("split").and_then(Value::as_str) {
        let then = By::parse(t).ok_or_else(|| bad(format!("split: {t}?")))?;
        v["split"] = json!(crate::usage::split(&srows, by, then)
            .iter()
            .map(|(g, sub)| json!({"key": g.key, "rows": sub.iter().map(|r| crate::usage::cmd::row_json(then, r)).collect::<Vec<_>>()}))
            .collect::<Vec<_>>());
    }
    // compare: the same length just before
    if p.get("compare").and_then(Value::as_bool).unwrap_or(false) {
        let before = crate::usage::group(&crate::usage::rows(ctx.cfg, range.prev(), &f), By::Total);
        v["previous"] = before
            .first()
            .map(|r| crate::usage::cmd::row_json(By::Total, r))
            .unwrap_or(Value::Null);
    }
    Ok(v)
}

/// A usage call's range and filter: `range` (24h, 7d, 6mo, this-week,
/// 2026-09-01..2026-10-15, …), and `host` / `agent` / `account` / `model` /
/// `project` (a folder), each a string (a comma list) or an array.
fn usage_scope(ctx: &Ctx, p: &Value) -> Result<(crate::usage::Range, crate::usage::Filter), Error> {
    use crate::usage::Range;
    let range = match p.get("range").and_then(Value::as_str) {
        Some(r) => Range::parse(r).ok_or_else(|| {
            bad(format!(
                "range: {r}? (24h, 3d, 7d, 30d, 6mo, all, today, this-week, this-month, 2026-09-01..2026-10-15)"
            ))
        })?,
        None => Range::days(7),
    };
    let l = |k: &str| -> Vec<String> {
        match p.get(k) {
            Some(Value::String(s)) => crate::usage::list_of(s),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(Value::as_str)
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    };
    let f = crate::usage::filter_of(
        ctx.cfg,
        &l("host"),
        &l("agent"),
        &l("account"),
        &l("model"),
        &l("project"),
    )
    .map_err(bad)?;
    Ok((range, f))
}

fn inbox_list(p: &Value) -> Value {
    let n = p.get("n").and_then(Value::as_u64).unwrap_or(30).min(2000) as usize;
    let host = p.get("host").and_then(Value::as_str);
    let kind = p.get("kind").and_then(Value::as_str);
    let unread_only = p.get("unread").and_then(Value::as_bool).unwrap_or(false);
    let seen = events::seen();
    let gone = events::dismissed();
    Value::Array(
        events::load_inbox(events::CAP)
            .into_iter()
            .filter(|e| !gone.contains(&events::key(e)))
            .filter(|e| host.is_none_or(|h| e.host == h))
            .filter(|e| kind.is_none_or(|k| e.kind.as_str() == k))
            .filter(|e| !unread_only || e.t > seen)
            .take(n)
            .map(|e| {
                let mut v = e.to_json();
                v["unread"] = json!(e.t > seen);
                v
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn stats_answers_trimmed() {
        let full = || {
            json!({"alias": "b31", "reachable": true, "cpu": 0.2, "cores": [0.1], "top": [{"pid": 1}],
                   "gpu_procs": [], "disks": [], "gpus": [{"index": 0, "util": 0.5, "temp": 40, "power": 90}]})
        };
        let mut v = full();
        trim_stats(&mut v, true, true);
        assert_eq!(
            v,
            json!({"alias": "b31", "reachable": true, "cpu": 0.2, "gpus": [{"index": 0, "util": 0.5}]})
        );
        let mut v = full();
        trim_stats(&mut v, false, false);
        assert!(v.get("top").is_none() && v.get("gpu_procs").is_none() && v.get("cores").is_some());
        assert!(METHODS.iter().any(|m| m.name == "stats.get"));
    }

    use super::*;

    fn cfg() -> Config {
        Config::in_memory(&["hosts=b31,gpu2", "remote.allow=b31:claude"])
    }

    #[test]
    fn every_method_is_listed_once() {
        let mut names: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
        assert!(method("session.send").is_some_and(|m| m.scope == Scope::Reply));
        assert!(method("config.set").is_some_and(|m| m.scope == Scope::Control));
    }

    #[test]
    fn scopes_are_enforced() {
        let c = cfg();
        let mut ctx = Ctx::local(&c, None);
        ctx.scope = Scope::Read;
        let e = call(
            &ctx,
            "config.set",
            &json!({"key": "theme", "value": "dark"}),
        )
        .unwrap_err();
        assert_eq!(e.code, "forbidden");
        assert!(e.message.contains("control"));
        assert_eq!(call(&ctx, "daemon.ping", &json!({})).unwrap()["pong"], true);
        assert_eq!(call(&ctx, "nope", &json!({})).unwrap_err().code, "unknown");
    }

    #[test]
    fn remote_callers_need_remote_allow() {
        let c = cfg();
        let mut ctx = Ctx::local(&c, None);
        ctx.remote = true;
        assert!(allowed(&ctx, "b31", "claude").is_ok());
        assert_eq!(
            allowed(&ctx, "gpu2", "claude").unwrap_err().code,
            "forbidden"
        );
        assert_eq!(allowed(&ctx, "b31", "codex").unwrap_err().code, "forbidden");
        ctx.remote = false;
        assert!(allowed(&ctx, "gpu2", "codex").is_ok());
    }

    #[test]
    fn snapshot_sessions() {
        let snap = json!({"hosts": [{"alias": "b31", "sessions": [
            {"host": "b31", "name": "temper", "pane_id": "w1:p2", "agent": "claude", "status": "blocked"},
            {"host": "b31", "name": "", "pane_id": ""},
        ]}]});
        let v = sessions_of(&snap);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "temper");
        assert!(v[0].blocked());
    }

    #[test]
    fn secrets_need_control() {
        let c = Config::in_memory(&["notify.ntfy=https://ntfy.sh/abc", "theme=dark"]);
        let mut ctx = Ctx::local(&c, None);
        ctx.scope = Scope::Read;
        assert_eq!(
            call(&ctx, "config.get", &json!({"key": "theme"})).unwrap()["value"],
            "dark"
        );
        assert_eq!(
            call(&ctx, "config.get", &json!({"key": "notify.ntfy"}))
                .unwrap_err()
                .code,
            "forbidden"
        );
        ctx.scope = Scope::Control;
        assert!(call(&ctx, "config.get", &json!({"key": "notify.ntfy"})).is_ok());
    }
}
