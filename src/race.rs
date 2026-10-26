//! Race mode: one prompt to 2–4 agents at once, each in a git worktree of its
//! own (`race/<id>-<agent>`), then their diffs side by side.
//!
//! `aleph race "prompt" [--agents claude,codex] [--host H] [--dir D]`, or the
//! palette's "Start a race…" on a session's machine and folder. The
//! dashboard's race view shows each agent's diff from where they all started
//! (the base commit, so commits count too), refreshed while it's open.
//! `aleph race ls | show ID` from the shell.
//!
//! Races are kept in `state/races.json`: id, prompt, machine, folder, base
//! commit, and each racer's session and worktree. Merging the winner is
//! yours: `show` prints the git line for it.

use crate::cli::{self, Args};
use crate::config::{Config, Target};
use crate::herdr::Herdr;
use crate::out::*;
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::process::ExitCode;
use std::time::Duration;

/// The most racers in one race (the view's columns).
pub const MAX: usize = 4;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Racer {
    pub agent: String,
    pub name: String,
    pub pane: String,
    pub dir: String,
    pub branch: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Race {
    pub id: String,
    pub t: u64,
    pub host: String,
    pub dir: String,
    pub base: String,
    pub prompt: String,
    pub racers: Vec<Racer>,
}

impl Race {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id, "t": self.t, "host": self.host, "dir": self.dir, "base": self.base, "prompt": self.prompt,
            "racers": self.racers.iter().map(|r| json!({"agent": r.agent, "name": r.name, "pane": r.pane, "dir": r.dir, "branch": r.branch})).collect::<Vec<_>>(),
        })
    }

    fn from_json(v: &Value) -> Option<Race> {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Race {
            id: v.get("id")?.as_str()?.to_string(),
            t: v.get("t").and_then(Value::as_u64).unwrap_or(0),
            host: s(v, "host"),
            dir: s(v, "dir"),
            base: s(v, "base"),
            prompt: s(v, "prompt"),
            racers: v
                .get("racers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|r| Racer {
                            agent: s(r, "agent"),
                            name: s(r, "name"),
                            pane: s(r, "pane"),
                            dir: s(r, "dir"),
                            branch: s(r, "branch"),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// `the prompt…` for a list.
    pub fn short(&self) -> String {
        crate::ui::trunc(self.prompt.lines().next().unwrap_or(""), 60)
    }
}

fn path() -> std::path::PathBuf {
    crate::config::world_state().join("races.json")
}

/// Every race, newest first.
pub fn all() -> Vec<Race> {
    let Ok(t) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
    let mut out: Vec<Race> = v
        .as_array()
        .map(|a| a.iter().filter_map(Race::from_json).collect())
        .unwrap_or_default();
    out.sort_by_key(|r| std::cmp::Reverse(r.t));
    out
}

pub fn get(id: &str) -> Option<Race> {
    all().into_iter().find(|r| r.id == id)
}

fn save(r: &Race) -> Result<(), String> {
    let mut v = all();
    v.retain(|x| x.id != r.id);
    v.insert(0, r.clone());
    // the last 30
    v.truncate(30);
    let j = Value::Array(v.iter().map(Race::to_json).collect());
    crate::config::write_atomic(&path(), j.to_string().as_bytes()).map_err(|e| e.to_string())
}

/// `1530-a`: the local time, and a letter when two start in a minute.
fn new_id(now: u64) -> String {
    let base = crate::ui::clock(now).replace(':', "");
    let used: Vec<String> = all().into_iter().map(|r| r.id).collect();
    for l in 'a'..='z' {
        let id = format!("{base}-{l}");
        if !used.contains(&id) {
            return id;
        }
    }
    format!("{base}-{now}")
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

/// The agents a race uses when none are named (`race.agents`).
pub fn default_agents(cfg: &Config) -> Vec<String> {
    let v: Vec<String> = cfg
        .get_or("race.agents")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if v.len() >= 2 {
        v
    } else {
        vec!["claude".into(), "codex".into()]
    }
}

/// Start a race: each agent in a worktree of its own, the same prompt.
pub fn start(
    cfg: &Config,
    t: &Target,
    dir: &str,
    prompt: &str,
    agents: &[String],
    say: &mut dyn FnMut(String),
) -> Result<Race, String> {
    if !(2..=MAX).contains(&agents.len()) {
        return Err(format!("a race takes 2 to {MAX} agents"));
    }
    if prompt.trim().is_empty() {
        return Err("a race needs a prompt".into());
    }
    let mut ids: Vec<&'static str> = Vec::new();
    for a in agents {
        let id = crate::agents::resolve(a)?;
        if id == crate::agents::SHELL {
            return Err("a plain shell can't race".into());
        }
        ids.push(id);
    }
    let base = shell(
        &t.ssh,
        &format!("cd {} && git rev-parse HEAD", crate::proc::sh_quote(dir)),
        20,
    )
    .map_err(|_| format!("{dir} on {} isn't in a git repository", t.ssh))?
    .trim()
    .to_string();
    let now = crate::cache::now();
    let id = new_id(now);
    let mut race = Race {
        id: id.clone(),
        t: now,
        host: t.ssh.clone(),
        dir: dir.to_string(),
        base,
        prompt: prompt.to_string(),
        racers: Vec::new(),
    };
    let mut failed = Vec::new();
    // two of the same agent get a number each
    let mut seen: Vec<&str> = Vec::new();
    for a in ids {
        let n = seen.iter().filter(|x| **x == a).count();
        seen.push(a);
        let branch = if n == 0 {
            format!("race/{id}-{a}")
        } else {
            format!("race/{id}-{a}{}", n + 1)
        };
        let mut o = cli::parse_new(&[], cfg)?.opts;
        o.agent = a.to_string();
        o.dir = Some(dir.as_bytes().to_vec());
        o.worktree = Some(branch.clone());
        o.prompt = Some(prompt.to_string());
        o.no_focus = true;
        cli::fill_names(&mut o);
        say(format!("starting {} in {branch}…", crate::agents::label(a)));
        match Herdr::new(t).new_session(&o, &mut |_, m| say(m)) {
            Ok(v) => {
                crate::launch::after_start(t, &o, &v);
                if let Some(s) = v.first() {
                    race.racers.push(Racer {
                        agent: a.to_string(),
                        name: s.name.clone(),
                        pane: s.pane.clone(),
                        dir: s.dir.clone(),
                        branch,
                    });
                }
            }
            Err(e) => failed.push(format!("{}: {e}", crate::agents::label(a))),
        }
    }
    if race.racers.is_empty() {
        return Err(failed.join("; "));
    }
    save(&race)?;
    crate::events::append(crate::events::Event {
        t: now,
        host: t.ssh.clone(),
        name: format!("race-{id}"),
        kind: crate::events::Kind::New,
        title: format!(
            "race {id}: {} on “{}”",
            race.racers
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()
                .join(" vs "),
            race.short()
        ),
        source: "aleph".into(),
        body: failed.join("\n"),
        ..Default::default()
    });
    Ok(race)
}

/// The shell line that prints a racer's diff from the base: the stat, a
/// mark, untracked files, a mark, the diff itself (cut at `max` lines).
pub fn diff_line(base: &str, dir: &str, max: usize) -> String {
    format!(
        "cd {} 2>/dev/null || exit 3; LC_ALL=C git diff --stat {b} 2>/dev/null | tail -n 40; echo @@aleph@@; LC_ALL=C git status --short --untracked-files=all 2>/dev/null | grep '^??' | head -n 20; echo @@aleph@@; LC_ALL=C git diff {b} 2>/dev/null | head -n {max}",
        crate::proc::sh_quote(dir),
        b = crate::proc::sh_quote(base)
    )
}

/// One racer's diff, split: (stat, untracked, diff).
pub fn diff_of(
    host: &str,
    base: &str,
    r: &Racer,
    max: usize,
) -> Result<(String, String, String), String> {
    let mut c = crate::sshcfg::shell(host, &diff_line(base, &r.dir, max));
    let o = crate::proc::run(&mut c, Duration::from_secs(30), None).map_err(|e| e.to_string())?;
    if o.code == Some(3) {
        return Err(format!("its worktree ({}) isn't there any more", r.dir));
    }
    if !o.ok() {
        return Err(o.reason());
    }
    let out = o.text();
    let mut p = out.splitn(3, "@@aleph@@\n");
    Ok((
        p.next().unwrap_or("").trim_end().to_string(),
        p.next().unwrap_or("").trim_end().to_string(),
        p.next().unwrap_or("").trim_end().to_string(),
    ))
}

pub fn cmd(a: Args) -> ExitCode {
    // the words, without the options' values
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < a.rest.len() {
        let x = &a.rest[i];
        if matches!(x.as_str(), "--agents" | "--host" | "--dir") {
            i += 2;
            continue;
        }
        if !x.starts_with('-') {
            pos.push(x.clone());
        }
        i += 1;
    }
    let opt = |k: &str| {
        a.rest
            .iter()
            .position(|x| x == k)
            .and_then(|i| a.rest.get(i + 1))
            .cloned()
    };
    let json_out = a.has("--json");
    match pos.first().map(String::as_str) {
        None => fail("usage: aleph race \"prompt\" [--agents claude,codex] [--host H] [--dir D] | ls | show ID"),
        Some("ls" | "list") => {
            let v = all();
            if json_out {
                json(&Value::Array(v.iter().map(Race::to_json).collect()));
                return ExitCode::SUCCESS;
            }
            if v.is_empty() {
                ok("no races yet");
                hint("aleph race \"the prompt\" --agents claude,codex --dir /srv/repo");
                return ExitCode::SUCCESS;
            }
            head("races");
            for r in &v {
                out(Line::from(vec![
                    Span::styled(format!("  {}  ", crate::ui::pad(&r.id, 8)), crate::ui::fg(crate::ui::c::ACCENT())),
                    Span::raw(format!("{}  ", crate::ui::pad(&r.short(), 50))),
                    Span::styled(
                        r.racers.iter().map(|x| x.name.clone()).collect::<Vec<_>>().join(" vs "),
                        crate::ui::muted(),
                    ),
                ]));
            }
            ExitCode::SUCCESS
        }
        Some("show") => {
            let Some(r) = pos.get(1).and_then(|id| get(id)) else {
                return fail("no such race (aleph race ls)");
            };
            head(&format!("race {} on {}: “{}”", r.id, r.host, r.short()));
            for x in &r.racers {
                out(Line::from(vec![
                    Span::styled(format!("  {}", crate::agents::label(&x.agent)), crate::ui::bold()),
                    Span::styled(format!("  {} · {}", x.name, x.branch), crate::ui::muted()),
                ]));
                match diff_of(&r.host, &r.base, x, 0) {
                    Ok((stat, untracked, _)) => {
                        let s = if stat.is_empty() { "    (no changes yet)".to_string() } else { stat };
                        for l in s.lines().chain(untracked.lines()) {
                            out(Line::from(Span::raw(format!("    {}", l.trim()))));
                        }
                    }
                    Err(e) => warn(&format!("{}: {e}", x.name)),
                }
            }
            hint(&format!(
                "keep one: git merge {} (in {}); the dashboard's race view shows the diffs side by side",
                r.racers.first().map(|x| x.branch.as_str()).unwrap_or("race/…"),
                r.dir
            ));
            ExitCode::SUCCESS
        }
        Some(_) => {
            let prompt = pos.join(" ");
            let agents: Vec<String> = opt("--agents")
                .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_else(|| default_agents(&a.cfg));
            let flag = opt("--host").or(a.flag.clone());
            let Some(t) = cli::target(&a.cfg, &flag) else {
                return cli::no_host();
            };
            let dir = match opt("--dir") {
                Some(d) => d,
                None if crate::sshcfg::is_local(&t.ssh) => std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                None => return fail("which repository? --dir /path/on/the/machine"),
            };
            let sp = (!json_out).then(|| Spinner::start(format!("Starting a race on {}", t.ssh)));
            let r = start(&a.cfg, &t, &dir, &prompt, &agents, &mut |m| {
                if let Some(sp) = &sp {
                    sp.set(m);
                }
            });
            if let Some(sp) = sp {
                sp.stop();
            }
            match r {
                Ok(r) => {
                    if json_out {
                        json(&r.to_json());
                        return ExitCode::SUCCESS;
                    }
                    ok(&format!(
                        "race {} started: {}",
                        r.id,
                        r.racers.iter().map(|x| format!("{} ({})", x.name, x.branch)).collect::<Vec<_>>().join(" vs ")
                    ));
                    hint(&format!("aleph race show {} · the dashboard: ctrl+k › Race", r.id));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn races_keep_their_racers_and_a_diff_splits_in_three() {
        let r = Race {
            id: "1530-a".into(),
            t: 5,
            host: "b31".into(),
            dir: "/srv/px".into(),
            base: "abc".into(),
            prompt: "make it fast\nand small".into(),
            racers: vec![Racer {
                agent: "codex".into(),
                name: "tune".into(),
                pane: "w1:p9".into(),
                dir: "/srv/px-race".into(),
                branch: "race/1530-a-codex".into(),
            }],
        };
        assert_eq!(Race::from_json(&r.to_json()).unwrap(), r);
        assert_eq!(r.short(), "make it fast");
        let l = diff_line("abc", "/srv/x y", 600);
        assert!(
            l.starts_with("cd '/srv/x y'")
                && l.contains("git diff --stat abc")
                && l.contains("head -n 600"),
            "{l}"
        );
        assert_eq!(
            default_agents(&Config::in_memory(&["race.agents=claude"])),
            ["claude", "codex"]
        );
    }
}
