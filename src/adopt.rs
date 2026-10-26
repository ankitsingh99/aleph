//! Agents running outside herdr (a terminal tab of their own, where herdr
//! has no hold on them): stop one, or reopen it in herdr — the same agent, in
//! the same folder, continuing the same conversation with the options it was
//! started with. `x` / enter on its Fleet row, the palette, `aleph outside`.
//!
//! Reopening captures first, with one shell line on its machine:
//! * its folder (`/proc/<pid>/cwd`, else `lsof`);
//! * its command line (`/proc/<pid>/cmdline`, else `ps`);
//! * the session files it has open (`~/.claude/projects/…/<id>.jsonl`,
//!   Codex's `rollout-…-<id>.jsonl`), and for Claude Code the newest one in
//!   its folder's own directory.
//!
//! From that, the conversation's id (the open file, else the newest, else
//! what its usage logs say, else none: the agent's own "continue the latest")
//! and the options worth keeping (a list per agent: model, permission mode,
//! sandbox, folders it may use…). A prompt it was started with is left out:
//! it isn't sent twice. Then it's stopped (SIGTERM; nothing starts unless it
//! has gone, so two never write the same conversation) and started in herdr
//! with them.

use crate::cli::{self, Args};
use crate::config::{Config, Target};
use crate::herdr::Herdr;
use crate::out::*;
use ratatui::prelude::*;
use serde_json::json;
use std::process::ExitCode;
use std::time::Duration;

/// One agent process outside herdr.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub host: String,
    pub pid: String,
    /// registry id (claude, codex, …)
    pub agent: String,
    /// its command line as the process table has it
    pub cmd: String,
    /// its folder, when already known
    pub cwd: String,
}

/// What one look at the process found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capture {
    pub cwd: String,
    pub argv: Vec<String>,
    /// session files it has open
    pub open: Vec<String>,
    /// Claude Code: the newest session file of its folder
    pub newest: Option<String>,
    /// seconds since its conversation was last written (the open file, else
    /// the newest)
    pub idle: Option<u64>,
}

fn pid_ok(pid: &str) -> bool {
    !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit())
}

/// The shell line that captures a process (POSIX sh, no `'`).
pub fn capture_line(pid: &str, agent: &str) -> String {
    let mut s = format!(
        "p={pid}; LC_ALL=C; c=$(readlink /proc/$p/cwd 2>/dev/null); [ -n \"$c\" ] || c=$(lsof -a -d cwd -p $p -Fn 2>/dev/null | sed -n s/^n//p | head -n 1); printf \"cwd\\t%s\\n\" \"$c\"; if [ -r /proc/$p/cmdline ]; then printf \"argv\\t\"; tr \"\\000\" \"\\037\" < /proc/$p/cmdline; printf \"\\n\"; else printf \"args\\t%s\\n\" \"$(ps -o args= -p $p 2>/dev/null)\"; fi; o=$({{ ls -l /proc/$p/fd 2>/dev/null | sed -n \"s/.* -> //p\"; lsof -p $p -Fn 2>/dev/null | sed -n s/^n//p; }} | grep \"[.]jsonl$\" | sort -u); [ -n \"$o\" ] && printf \"%s\\n\" \"$o\" | sed \"s/^/open\\t/\"; n="
    );
    if agent == "claude" {
        // Claude Code keeps each folder's sessions in a directory named after it
        s.push_str("; d=$(printf %s \"$c\" | sed \"s/[^A-Za-z0-9]/-/g\"); n=$(ls -t \"$HOME/.claude/projects/$d\"/*.jsonl 2>/dev/null | head -n 1); [ -n \"$n\" ] && printf \"newest\\t%s\\n\" \"$n\"");
    }
    // how long ago its conversation was written: the open file, else the newest
    s.push_str("; f=; [ -n \"$o\" ] && f=$(ls -t $o 2>/dev/null | head -n 1); [ -n \"$f\" ] || f=$n; [ -n \"$f\" ] && t=$(stat -c %Y \"$f\" 2>/dev/null || stat -f %m \"$f\" 2>/dev/null) && [ -n \"$t\" ] && printf \"idle\\t%s\\n\" $(( $(date +%s) - t )); true");
    s
}

pub fn parse_capture(text: &str) -> Capture {
    let mut c = Capture::default();
    for l in text.lines() {
        let Some((k, v)) = l.split_once('\t') else {
            continue;
        };
        match k {
            "cwd" => c.cwd = v.trim().to_string(),
            "argv" => {
                c.argv = v
                    .split('\u{1f}')
                    .filter(|x| !x.is_empty())
                    .map(String::from)
                    .collect()
            }
            "args" if c.argv.is_empty() => {
                c.argv = v.split_whitespace().map(String::from).collect()
            }
            "open" => c.open.push(v.trim().to_string()),
            "newest" if !v.trim().is_empty() => c.newest = Some(v.trim().to_string()),
            "idle" => c.idle = v.trim().parse::<i64>().ok().map(|n| n.max(0) as u64),
            _ => {}
        }
    }
    c
}

fn uuidish(s: &str) -> bool {
    s.len() >= 8 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// The conversation's id in a session file's path, for this agent.
pub fn id_in_path(agent: &str, path: &str) -> Option<String> {
    let file = path.rsplit('/').next()?;
    let stem = file.strip_suffix(".jsonl")?;
    match agent {
        "claude" if path.contains("/.claude/projects/") && uuidish(stem) => Some(stem.to_string()),
        "codex" if path.contains("/.codex/sessions/") => {
            // rollout-2026-10-24T10-00-00-<uuid>
            let tail = stem.get(stem.len().saturating_sub(36)..)?;
            uuidish(tail).then(|| tail.to_string())
        }
        _ => None,
    }
}

/// The conversation to continue, and how it was found.
pub fn session_id(
    agent: &str,
    cap: Option<&Capture>,
    from_logs: Option<String>,
) -> Option<(String, &'static str)> {
    if let Some(c) = cap {
        if let Some(id) = c.open.iter().find_map(|p| id_in_path(agent, p)) {
            return Some((id, "the session file it has open"));
        }
        if let Some(id) = c.newest.as_deref().and_then(|p| id_in_path(agent, p)) {
            return Some((id, "its folder's newest session"));
        }
    }
    from_logs
        .filter(|i| !i.is_empty())
        .map(|i| (i, "its usage logs"))
}

/// Options worth keeping, per agent: (flag, takes a value).
fn keep_list(agent: &str) -> &'static [(&'static str, bool)] {
    match agent {
        "claude" => &[
            ("--model", true),
            ("--fallback-model", true),
            ("--permission-mode", true),
            ("--dangerously-skip-permissions", false),
            ("--add-dir", true),
            ("--allowedTools", true),
            ("--allowed-tools", true),
            ("--disallowedTools", true),
            ("--disallowed-tools", true),
            ("--mcp-config", true),
            ("--strict-mcp-config", false),
            ("--settings", true),
            ("--append-system-prompt", true),
            ("--verbose", false),
            ("--ide", false),
        ],
        "codex" => &[
            ("--model", true),
            ("-m", true),
            ("--sandbox", true),
            ("-s", true),
            ("--ask-for-approval", true),
            ("-a", true),
            ("--profile", true),
            ("-p", true),
            ("-c", true),
            ("--config", true),
            ("--full-auto", false),
            ("--dangerously-bypass-approvals-and-sandbox", false),
            ("--search", false),
            ("--oss", false),
        ],
        _ => &[],
    }
}

/// The options to start it with again: the ones on its agent's list, in the
/// order it had them (a prompt, resume flags and anything unknown left out).
pub fn kept_args(agent: &str, argv: &[String]) -> Vec<String> {
    let list = keep_list(agent);
    let mut out = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        let (name, inline) = match a.split_once('=') {
            Some((n, _)) if n.starts_with('-') => (n, true),
            _ => (a.as_str(), false),
        };
        if let Some((_, value)) = list.iter().find(|(f, _)| *f == name) {
            out.push(a.clone());
            if *value && !inline {
                if let Some(v) = argv.get(i + 1) {
                    out.push(v.clone());
                    i += 1;
                }
            }
        }
        i += 1;
    }
    out
}

/// What reopening it will do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub found: Found,
    pub dir: String,
    pub id: Option<String>,
    /// how the id was found (or why there's none)
    pub how: String,
    /// everything after the agent's executable
    pub args: Vec<String>,
}

impl Plan {
    /// `claude --model opus --resume 3f2a…` (what herdr will run).
    pub fn command(&self) -> String {
        let exe = crate::agents::find(&self.found.agent)
            .and_then(|a| a.exes.first().copied())
            .unwrap_or(self.found.agent.as_str());
        let mut v = vec![exe.to_string()];
        v.extend(self.args.iter().map(|a| crate::proc::sh_quote(a)));
        v.join(" ")
    }
}

pub fn plan(found: &Found, cap: Option<&Capture>, from_logs: Option<String>) -> Plan {
    let argv: Vec<String> = match cap {
        Some(c) if !c.argv.is_empty() => c.argv.clone(),
        _ => found.cmd.split_whitespace().map(String::from).collect(),
    };
    let kept = kept_args(&found.agent, argv.get(1..).unwrap_or(&[]));
    let dir = cap
        .map(|c| c.cwd.clone())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| found.cwd.clone());
    let sid = session_id(&found.agent, cap, from_logs);
    let resume = crate::agents::find(&found.agent)
        .map(|a| a.resume_for(sid.as_ref().map(|x| x.0.as_str())))
        .unwrap_or_default();
    let how = match &sid {
        Some((_, how)) => format!("the conversation from {how}"),
        None if resume.is_empty() => "a new conversation (it can't continue one)".into(),
        None => "its most recent conversation in that folder".into(),
    };
    // Codex: options before its `resume` subcommand
    let args = if found.agent == "codex" {
        kept.into_iter().chain(resume).collect()
    } else {
        kept.into_iter().chain(resume).collect::<Vec<_>>()
    };
    Plan {
        found: found.clone(),
        dir,
        id: sid.map(|x| x.0),
        how,
        args,
    }
}

fn shell(host: &str, line: &str, secs: u64) -> Result<String, String> {
    let mut c = crate::sshcfg::shell(host, line);
    let o = crate::proc::run(&mut c, Duration::from_secs(secs), None).map_err(|e| e.to_string())?;
    if o.ok() {
        Ok(o.text())
    } else {
        Err(o.reason())
    }
}

pub fn capture(host: &str, pid: &str, agent: &str) -> Result<Capture, String> {
    if !pid_ok(pid) {
        return Err(format!("{pid}: not a process id"));
    }
    shell(host, &capture_line(pid, agent), 20).map(|t| parse_capture(&t))
}

/// SIGTERM, then wait (up to 10 s) for it to go: true when it has.
pub fn stop(host: &str, pid: &str) -> Result<bool, String> {
    if !pid_ok(pid) {
        return Err(format!("{pid}: not a process id"));
    }
    let line = format!(
        "kill -TERM {pid} 2>/dev/null || {{ echo gone; exit 0; }}; i=0; while kill -0 {pid} 2>/dev/null && [ $i -lt 50 ]; do sleep 0.2; i=$((i+1)); done; kill -0 {pid} 2>/dev/null && echo running || echo gone"
    );
    shell(host, &line, 20).map(|t| t.trim().ends_with("gone"))
}

/// Stop it and start it again in herdr; the new session's name.
pub fn reopen(
    cfg: &Config,
    t: &Target,
    p: &Plan,
    say: &mut dyn FnMut(String),
) -> Result<String, String> {
    if p.dir.is_empty() {
        return Err("its folder couldn't be found, so it can't be reopened there".into());
    }
    let label = crate::agents::label(&p.found.agent).to_string();
    say(format!("stopping {label} (pid {})…", p.found.pid));
    if !stop(&t.ssh, &p.found.pid)? {
        return Err(format!(
            "{label} (pid {}) didn't exit within 10 s: nothing was started, so two don't write the same conversation",
            p.found.pid
        ));
    }
    let mut o = cli::parse_new(&[], cfg)?.opts;
    o.agent = p.found.agent.clone();
    o.dir = Some(p.dir.as_bytes().to_vec());
    o.args = p.args.clone();
    o.no_focus = true;
    cli::fill_names(&mut o);
    say(format!("starting {label} in herdr…"));
    let v = Herdr::new(t).new_session(&o, &mut |_, m| say(m))?;
    crate::launch::after_start(t, &o, &v);
    let name = v.first().map(|s| s.name.clone()).unwrap_or_default();
    crate::events::append(crate::events::Event {
        t: crate::cache::now(),
        host: t.ssh.clone(),
        name: name.clone(),
        kind: crate::events::Kind::Remote,
        title: format!(
            "{name}: {label} reopened in herdr from a terminal (pid {}), with {}",
            p.found.pid, p.how
        ),
        agent: p.found.agent.clone(),
        source: "aleph".into(),
        body: p.command(),
        ..Default::default()
    });
    Ok(name)
}

/// The agents outside herdr on these machines, from one sample each.
pub fn list(hosts: &[String]) -> Vec<(String, crate::telemetry::AgentProc)> {
    crate::statscmd::sample(hosts, Duration::from_secs(25))
        .into_iter()
        .flat_map(|(h, st)| {
            st.agent_list
                .into_iter()
                .filter(|a| !a.in_herdr)
                .map(move |a| (h.clone(), a))
        })
        .collect()
}

fn find_one(cfg: &Config, q: &str) -> Result<Found, String> {
    let (host, pid) = q
        .rsplit_once(':')
        .ok_or_else(|| format!("{q}: machine:pid (aleph outside ls lists them)"))?;
    let all = list(&[host.to_string()]);
    let (_, a) = all.into_iter().find(|(_, a)| a.pid == pid).ok_or_else(|| {
        format!("no agent outside herdr with pid {pid} on {host} (aleph outside ls)")
    })?;
    let _ = cfg;
    Ok(Found {
        host: host.to_string(),
        pid: a.pid,
        agent: a.agent,
        cmd: a.cmd,
        cwd: String::new(),
    })
}

/// One agent outside herdr, as `aleph outside ls` shows it.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub host: String,
    pub p: crate::telemetry::AgentProc,
    /// its folder, conversation and last write (none: its machine didn't say)
    pub cap: Option<Capture>,
}

impl Row {
    /// Working (its CPU busy, or its conversation written in the last 30 s)
    /// or idle, and since when (pure).
    pub fn status(&self) -> (bool, String) {
        let idle = self.cap.as_ref().and_then(|c| c.idle);
        if self.p.cpu >= 0.05 || idle.is_some_and(|i| i < 30) {
            return (true, "working".into());
        }
        match idle {
            Some(i) => (false, format!("idle {}", crate::ui::dur(i))),
            None => (false, "idle".into()),
        }
    }

    pub fn session(&self) -> Option<String> {
        session_id(&self.p.agent, self.cap.as_ref(), None).map(|(id, _)| id)
    }

    pub fn folder(&self) -> String {
        self.cap
            .as_ref()
            .map(|c| crate::herdr::short_path(&c.cwd, &self.p.user))
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| "—".into())
    }
}

/// Each machine's agents captured, one shell line a machine, in parallel
/// (a machine that doesn't answer: rows without a capture).
pub fn rows(v: Vec<(String, crate::telemetry::AgentProc)>) -> Vec<Row> {
    let mut hosts: Vec<String> = v.iter().map(|(h, _)| h.clone()).collect();
    hosts.dedup();
    let caps: Vec<(String, Vec<(String, Capture)>)> = std::thread::scope(|sc| {
        let jobs: Vec<_> = hosts
            .iter()
            .map(|h| {
                let mine: Vec<(String, String)> = v
                    .iter()
                    .filter(|(x, p)| x == h && pid_ok(&p.pid))
                    .map(|(_, p)| (p.pid.clone(), p.agent.clone()))
                    .collect();
                sc.spawn(move || (h.clone(), capture_all(h, &mine)))
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    });
    v.into_iter()
        .map(|(host, p)| {
            let cap = caps
                .iter()
                .find(|(h, _)| *h == host)
                .and_then(|(_, c)| c.iter().find(|(pid, _)| *pid == p.pid))
                .map(|(_, c)| c.clone());
            Row { host, p, cap }
        })
        .collect()
}

/// Every one of `procs` (pid, agent) on `host` captured in one shell line.
fn capture_all(host: &str, procs: &[(String, String)]) -> Vec<(String, Capture)> {
    if procs.is_empty() {
        return Vec::new();
    }
    let line: Vec<String> = procs
        .iter()
        .map(|(pid, agent)| {
            format!(
                "printf \"pid\\t%s\\n\" {pid}; ( {} )",
                capture_line(pid, agent)
            )
        })
        .collect();
    shell(host, &line.join("; "), 20)
        .map(|t| split_captures(&t))
        .unwrap_or_default()
}

/// `pid\t…` lines start each process's capture (pure — tested).
pub fn split_captures(text: &str) -> Vec<(String, Capture)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for l in text.lines() {
        if let Some(pid) = l.strip_prefix("pid\t") {
            v.push((pid.trim().to_string(), String::new()));
        } else if let Some((_, t)) = v.last_mut() {
            t.push_str(l);
            t.push('\n');
        }
    }
    v.into_iter().map(|(p, t)| (p, parse_capture(&t))).collect()
}

/// The human table: a title, then a bordered table of every agent (its
/// machine in its colour, working or idle and since when, CPU, memory,
/// folder, conversation, command), as wide as `width` allows (pure — tested).
fn table(cfg: &Config, v: &[Row], width: usize) -> Vec<Line<'static>> {
    use crate::out::Col;
    use crate::ui::{self, c};
    let mut hosts: Vec<&str> = v.iter().map(|r| r.host.as_str()).collect();
    hosts.dedup();
    let working = v.iter().filter(|r| r.status().0).count();
    let mut lines = vec![Line::from(vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled("agents outside herdr", ui::bold()),
        Span::styled(
            format!(
                "  ·  {} on {}{}",
                v.len(),
                if hosts.len() == 1 {
                    hosts[0].to_string()
                } else {
                    format!("{} machines", hosts.len())
                },
                if working > 0 {
                    format!("  ·  {working} working")
                } else {
                    String::new()
                }
            ),
            ui::faint(),
        ),
    ])];
    let cols = [
        Col::new("machine"),
        Col::new("pid"),
        Col::new("agent"),
        Col::new("status"),
        Col::new("up").right(),
        Col::new("cpu").right(),
        Col::new("memory").right(),
        Col::new("folder").flex(),
        Col::new("conversation").flex().optional(),
        Col::new("command").flex().optional(),
    ];
    let rows: Vec<Vec<Span<'static>>> = v
        .iter()
        .map(|r| {
            let col = crate::theme::machine(cfg, &r.host);
            let (busy, st) = r.status();
            vec![
                Span::styled(
                    r.host.clone(),
                    Style::new().fg(col).add_modifier(Modifier::BOLD),
                ),
                Span::styled(r.p.pid.clone(), ui::fg(c::ACCENT())),
                Span::raw(crate::agents::label(&r.p.agent)),
                if busy {
                    Span::styled(format!("● {st}"), ui::fg(c::WORKING()))
                } else {
                    Span::styled(format!("○ {st}"), ui::muted())
                },
                Span::raw(r.p.age.map_or("—".to_string(), ui::dur)),
                Span::raw(format!("{:.0}%", r.p.cpu * 100.0)),
                Span::raw(if r.p.rss > 0.0 {
                    ui::bytes(r.p.rss)
                } else {
                    "—".into()
                }),
                Span::raw(r.folder()),
                Span::styled(r.session().unwrap_or_else(|| "—".into()), ui::muted()),
                Span::styled(r.p.cmd.clone(), ui::faint()),
            ]
        })
        .collect();
    lines.extend(crate::out::table(&cols, &rows, width));
    lines
}

const USAGE: &str =
    "usage: aleph outside [ls] | stop MACHINE:PID [--yes] | reopen MACHINE:PID [--yes] [--json]";

pub fn cmd(a: Args) -> ExitCode {
    let pos = a.positional();
    let json_out = a.has("--json");
    match pos.first().copied() {
        None | Some("ls" | "list") => {
            let hosts = a.cfg.fleet();
            let sp = (!json_out).then(|| Spinner::start("Looking for agents outside herdr".into()));
            let v = rows(list(&hosts));
            if let Some(sp) = sp {
                sp.stop();
            }
            if json_out {
                json(&json!(v
                    .iter()
                    .map(|r| json!({
                        "host": r.host,
                        "pid": r.p.pid,
                        "agent": r.p.agent,
                        "user": r.p.user,
                        "working": r.status().0,
                        "status": r.status().1,
                        "age": r.p.age,
                        "cpu": r.p.cpu,
                        "rss": r.p.rss,
                        "cwd": r.cap.as_ref().map(|c| c.cwd.clone()),
                        "idle": r.cap.as_ref().and_then(|c| c.idle),
                        "session": r.session(),
                        "cmd": r.p.cmd,
                    }))
                    .collect::<Vec<_>>()));
                return ExitCode::SUCCESS;
            }
            // piped: one agent a line, tab-separated
            if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                for r in &v {
                    raw(&format!(
                        "{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.0}\t{}\t{}\t{}",
                        r.host,
                        r.p.pid,
                        r.p.agent,
                        r.status().1,
                        r.p.age.map(|a| a.to_string()).unwrap_or_default(),
                        r.p.cpu,
                        r.p.rss,
                        r.cap.as_ref().map(|c| c.cwd.as_str()).unwrap_or_default(),
                        r.session().unwrap_or_default(),
                        r.p.cmd
                    ));
                }
                return ExitCode::SUCCESS;
            }
            if v.is_empty() {
                ok("no agents outside herdr: every one runs in herdr");
                return ExitCode::SUCCESS;
            }
            let width = crossterm::terminal::size().map_or(120, |(w, _)| w as usize);
            for l in table(&a.cfg, &v, width.clamp(60, 220)) {
                out(l);
            }
            hint("aleph outside reopen MACHINE:PID — the same agent in herdr, continuing its conversation · stop MACHINE:PID");
            ExitCode::SUCCESS
        }
        Some("stop") => {
            let Some(q) = pos.get(1) else {
                return fail(USAGE);
            };
            let f = match find_one(&a.cfg, q) {
                Ok(f) => f,
                Err(e) => return fail(&e),
            };
            let label = crate::agents::label(&f.agent);
            if !a.has("--yes")
                && !yes(&format!(
                    "stop {label} (pid {}) on {}? Its terminal stays; there's no undo",
                    f.pid, f.host
                ))
            {
                hint("left running");
                return ExitCode::from(1);
            }
            match stop(&f.host, &f.pid) {
                Ok(true) => {
                    ok(&format!("stopped {label} (pid {}) on {}", f.pid, f.host));
                    ExitCode::SUCCESS
                }
                Ok(false) => fail(&format!(
                    "{label} (pid {}) is still running 10 s later",
                    f.pid
                )),
                Err(e) => fail(&e),
            }
        }
        Some("reopen") => {
            let Some(q) = pos.get(1) else {
                return fail(USAGE);
            };
            let f = match find_one(&a.cfg, q) {
                Ok(f) => f,
                Err(e) => return fail(&e),
            };
            let cap = capture(&f.host, &f.pid, &f.agent).ok();
            let p = plan(&f, cap.as_ref(), None);
            let label = crate::agents::label(&f.agent);
            if !json_out {
                head(&format!(
                    "reopen {label} (pid {}) on {} in herdr",
                    f.pid, f.host
                ));
                say(Line::from(Span::raw(format!("  folder   {}", p.dir))));
                say(Line::from(Span::raw(format!("  runs     {}", p.command()))));
                say(Line::from(Span::styled(
                    format!("  with     {}", p.how),
                    crate::ui::muted(),
                )));
            }
            if !a.has("--yes") && !yes("stop it and start it in herdr like that?") {
                hint("left as it is");
                return ExitCode::from(1);
            }
            let Some(t) = cli::target(&a.cfg, &Some(f.host.clone())) else {
                return cli::no_host();
            };
            match reopen(&a.cfg, &t, &p, &mut |_| {}) {
                Ok(name) => {
                    if json_out {
                        json(
                            &json!({"host": f.host, "pid": f.pid, "session": name, "dir": p.dir, "command": p.command(), "id": p.id}),
                        );
                    } else {
                        ok(&format!("{label} runs in herdr now as {name}, {}", p.how));
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        Some(_) => fail(USAGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_table_is_bordered_with_status_and_fits_the_width() {
        let cfg = Config::in_memory(&[]);
        let p = |pid: &str, agent: &str, age: Option<u64>, cpu: f64, cmd: &str| {
            crate::telemetry::AgentProc {
                agent: agent.into(),
                pid: pid.into(),
                user: "selene".into(),
                age,
                cmd: cmd.into(),
                in_herdr: false,
                cpu,
                rss: 512.0 * 1048576.0,
            }
        };
        let cap = |idle: Option<u64>| {
            Capture {
            cwd: "/home/selene/ox_color_check".into(),
            newest: Some("/home/selene/.claude/projects/-home-selene-px/3f2a9c1e-0b7d-4c55-9a61-2e8f0c4d7a10.jsonl".into()),
            idle,
            ..Default::default()
        }
        };
        let v = vec![
            Row {
                host: "b31".into(),
                p: p(
                    "2066101",
                    "claude",
                    Some(11520),
                    0.0,
                    "claude --resume 3f2a9c1e",
                ),
                cap: Some(cap(Some(600))),
            },
            Row {
                host: "b31".into(),
                p: p("2066150", "claude", Some(60), 0.0, "claude"),
                cap: Some(cap(Some(4))),
            },
            Row {
                host: "gpu2".into(),
                p: p("88", "codex", None, 0.4, &"codex ".repeat(40)),
                cap: None,
            },
        ];
        let text = |w: usize| -> Vec<String> {
            table(&cfg, &v, w)
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        let t = text(200);
        assert!(
            t[0].contains("agents outside herdr")
                && t[0].contains("3 on 2 machines")
                && t[0].contains("2 working"),
            "{t:#?}"
        );
        // rounded borders, the heads, a rule under them, every row as wide
        assert!(t[1].starts_with('╭') && t[1].ends_with('╮'), "{t:#?}");
        assert!(
            t[2].contains("machine") && t[2].contains("status") && t[2].contains("conversation")
        );
        assert!(t[3].starts_with('├') && t.last().unwrap().starts_with('╰'));
        let w0 = crate::ui::width(&t[1]);
        assert!(t[1..].iter().all(|l| crate::ui::width(l) == w0), "{t:#?}");
        // idle 10m; written 4 s ago: working; busy CPU: working
        assert!(
            t[4].contains("○ idle 10m") && t[4].contains("~/ox_color_check"),
            "{t:#?}"
        );
        assert!(t[4].contains("3f2a9c1e-0b7d"), "{t:#?}");
        assert!(
            t[5].contains("● working") && t[6].contains("● working"),
            "{t:#?}"
        );
        assert!(t[6].contains("40%") && t[6].contains("—"), "{t:#?}");
        // narrow: the command, then the conversation, give way; never wider
        let t = text(100);
        assert!(t.iter().all(|l| crate::ui::width(l) <= 100), "{t:#?}");
        assert!(
            !t[2].contains("command") && t[2].contains("folder"),
            "{t:#?}"
        );
    }

    #[test]
    fn captures_split_per_process() {
        let t = "pid\t11\ncwd\t/srv/a\nidle\t42\npid\t12\ncwd\t/srv/b\nopen\t/x/.claude/projects/-srv-b/aaaa1111-0000-0000-0000-000000000000.jsonl\n";
        let v = split_captures(t);
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].0.as_str(), v[0].1.cwd.as_str(), v[0].1.idle),
            ("11", "/srv/a", Some(42))
        );
        assert_eq!(
            (v[1].0.as_str(), v[1].1.open.len(), v[1].1.idle),
            ("12", 1, None)
        );
    }

    #[test]
    fn a_capture_reads_the_folder_argv_and_open_sessions() {
        let t = "cwd\t/srv/px\nargv\tnode\u{1f}/opt/claude/cli.js\u{1f}--model\u{1f}opus\u{1f}--dangerously-skip-permissions\u{1f}fix the LUT\u{1f}\nopen\t/home/n/.claude/projects/-srv-px/3f2a9c1e-0b7d-4c55-9a61-2e8f0c4d7a10.jsonl\nnewest\t/home/n/.claude/projects/-srv-px/aaaa1111-0000-0000-0000-000000000000.jsonl\n";
        let c = parse_capture(t);
        assert_eq!(c.cwd, "/srv/px");
        assert_eq!(c.argv.len(), 6);
        let f = Found {
            host: "b31".into(),
            pid: "4242".into(),
            agent: "claude".into(),
            ..Default::default()
        };
        let p = plan(&f, Some(&c), Some("from-logs".into()));
        // the open file wins; the prompt isn't sent twice; the options stay
        assert_eq!(
            p.id.as_deref(),
            Some("3f2a9c1e-0b7d-4c55-9a61-2e8f0c4d7a10")
        );
        assert_eq!(
            p.args,
            [
                "--model",
                "opus",
                "--dangerously-skip-permissions",
                "--resume",
                "3f2a9c1e-0b7d-4c55-9a61-2e8f0c4d7a10"
            ]
        );
        assert!(p.how.contains("open"), "{}", p.how);
        assert_eq!(p.command(), "claude --model opus --dangerously-skip-permissions --resume 3f2a9c1e-0b7d-4c55-9a61-2e8f0c4d7a10");
        // nothing open: the newest of its folder; nothing at all: the logs, else continue
        let c2 = Capture {
            open: vec![],
            ..c.clone()
        };
        assert_eq!(
            plan(&f, Some(&c2), None).id.as_deref(),
            Some("aaaa1111-0000-0000-0000-000000000000")
        );
        let c3 = Capture {
            open: vec![],
            newest: None,
            ..c.clone()
        };
        assert_eq!(
            plan(&f, Some(&c3), Some("abcdef12".into())).id.as_deref(),
            Some("abcdef12")
        );
        let p4 = plan(&f, Some(&c3), None);
        assert_eq!(
            (p4.id, p4.args.last().map(String::as_str)),
            (None, Some("--continue"))
        );
    }

    /// By hand, on this computer: the capture line reads a live process (this
    /// test's own) — its folder and its command line. `cargo test
    /// capture_reads_a_live_process -- --ignored`
    #[test]
    #[ignore]
    fn capture_reads_a_live_process() {
        let me = std::process::id().to_string();
        let o = crate::proc::cap(
            "sh",
            &["-c", &capture_line(&me, "claude")],
            Duration::from_secs(20),
        );
        let c = parse_capture(&o.text());
        let here = std::env::current_dir().unwrap();
        let here = std::fs::canonicalize(here).unwrap();
        assert_eq!(
            std::fs::canonicalize(&c.cwd).ok().as_deref(),
            Some(here.as_path()),
            "{}",
            o.text()
        );
        assert!(!c.argv.is_empty(), "{}", o.text());
    }

    #[test]
    fn codex_keeps_its_options_before_resume() {
        let f = Found {
            host: "b31".into(),
            pid: "77".into(),
            agent: "codex".into(),
            cmd: "codex -m gpt-5 --full-auto -c model_reasoning_effort=high do it".into(),
            cwd: "/srv/x".into(),
        };
        let c = Capture {
            cwd: "/srv/x".into(),
            open: vec!["/home/n/.codex/sessions/2026/10/24/rollout-2026-10-24T10-00-00-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl".into()],
            ..Default::default()
        };
        let p = plan(&f, Some(&c), None);
        assert_eq!(
            p.args,
            [
                "-m",
                "gpt-5",
                "--full-auto",
                "-c",
                "model_reasoning_effort=high",
                "resume",
                "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"
            ]
        );
        assert!(capture_line("77", "codex").starts_with("p=77;"));
        assert!(!capture_line("77", "codex").contains('\''));
        assert!(capture_line("77", "claude").contains(".claude/projects"));
    }
}
