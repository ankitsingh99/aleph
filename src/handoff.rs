//! Hand a session's work to another agent: `aleph handoff <session> [--to
//! AGENT] [--summary agent|aleph] [--json]`, or `h` on a session in Fleet.
//!
//! A hand-off note says what the task is, what's done, the decisions made,
//! what's left and the diff so far. It starts a new session of the other agent
//! in the same folder, with the note as its first prompt. The old session
//! stays as it is.
//!
//! The note comes from one of two places (`handoff.summary`):
//! * **agent** (the default): the session's own agent writes it, asked with
//!   the prompt library's `handoff`, to a file in the machine's
//!   `~/.cache/aleph/handoff/`, never in the repo;
//! * **aleph**: Aleph builds it with no agent at all, from the task's title,
//!   the end of the agent's screen and git (status, diff stat, recent
//!   commits).
//!
//! With `handoff.fallback` on (the default), an agent that's at its usage
//! limit, or doesn't write the note in time, gets Aleph's note instead.

use crate::cli::{self, Args};
use crate::config::{Config, Target};
use crate::herdr::{Herdr, Session};
use crate::out::*;
use serde_json::json;
use std::process::ExitCode;
use std::time::Duration;

/// How long the agent has to write its note.
const AGENT_WAIT: Duration = Duration::from_secs(6 * 60);

/// The agent to hand to when none is named: Codex from Claude Code, else
/// Claude Code (or `handoff.to`).
pub fn default_to(cfg: &Config, from: &str) -> String {
    match cfg.get_or("handoff.to").as_str() {
        "" if from == "claude" => "codex".into(),
        "" => "claude".into(),
        x => x.to_string(),
    }
}

/// Is it at its plan's usage limit (so it can't write a note)?
pub fn at_limit(s: &Session) -> bool {
    s.limit.is_some() || s.status == "limit"
}

/// What Aleph knows without asking the agent: a shell line that prints git's
/// view of the folder (`LC_ALL=C`, nothing written).
fn git_line(dir: &str) -> String {
    format!(
        "{}cd {} 2>/dev/null || exit 0; LC_ALL=C git rev-parse --abbrev-ref HEAD 2>/dev/null | sed 's/^/branch: /'; LC_ALL=C git status --short 2>/dev/null | head -n 40; echo @@aleph@@; LC_ALL=C git diff --stat 2>/dev/null | tail -n 25; echo @@aleph@@; LC_ALL=C git log --oneline -n 8 2>/dev/null",
        crate::chan::PATH_PRELUDE,
        crate::proc::sh_quote(dir)
    )
}

/// The note, written by Aleph: the task, git, and the end of the screen.
pub fn aleph_note(s: &Session, git: &str, screen: &str) -> String {
    let mut parts: Vec<&str> = git.splitn(3, "@@aleph@@\n").collect();
    while parts.len() < 3 {
        parts.push("");
    }
    let (status, stat, log) = (parts[0].trim(), parts[1].trim(), parts[2].trim());
    let mut n = format!(
        "# Hand-off from {} ({})\n\nTask: {}\nFolder: {}\n",
        s.name,
        crate::agents::label(&s.agent),
        if s.title.trim().is_empty() {
            "(no title)"
        } else {
            s.title.trim()
        },
        s.cwd
    );
    if !status.is_empty() {
        n.push_str(&format!("\n## Changed files (git status)\n{status}\n"));
    }
    if !stat.is_empty() {
        n.push_str(&format!("\n## The diff so far (git diff --stat)\n{stat}\n"));
    }
    if !log.is_empty() {
        n.push_str(&format!("\n## Recent commits\n{log}\n"));
    }
    let tail: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = &tail[tail.len().saturating_sub(40)..];
    if !tail.is_empty() {
        n.push_str(&format!(
            "\n## The end of its screen\n```\n{}\n```\n",
            tail.join("\n")
        ));
    }
    n
}

/// The new agent's first prompt: who it's taking over from, and the note.
pub fn intro(s: &Session, note: &str, by: &str) -> String {
    format!(
        "You're taking over a task from another coding agent ({} in session {}), in this folder. Its hand-off note ({by}) is below. Read it, look at the current state of the code (git status, git diff), then carry on with what's left. Ask me if anything in it is unclear before changing a lot.\n\n{note}",
        crate::agents::label(&s.agent),
        s.name
    )
}

/// What a hand-off did.
pub struct Done {
    pub by: &'static str,
    pub to: String,
    pub started: Vec<crate::herdr::Started>,
    /// why the agent's note wasn't used, when it wasn't
    pub fell_back: Option<String>,
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

/// Ask the agent for its note, written to a file Aleph reads back.
fn agent_note(
    cfg: &Config,
    t: &Target,
    s: &Session,
    say: &mut dyn FnMut(String),
) -> Result<String, String> {
    let home = shell(&s.host, "printf %s \"$HOME\"", 20)?;
    let home = home.trim();
    if home.is_empty() {
        return Err("couldn't find the machine's home folder".into());
    }
    let path = format!(
        "{home}/.cache/aleph/handoff/{}-{}.md",
        s.name,
        crate::cache::now()
    );
    let ask = crate::prompts::get(cfg, "handoff")
        .map(|p| p.text)
        .unwrap_or_default();
    let text = format!(
        "{ask}\n\nWrite it as Markdown to {path} (create the folder; don't put it in the repo), then say you're done. Don't change anything else."
    );
    say(format!("asking {} for its hand-off note…", s.name));
    let until = vec!["done".to_string(), "idle".to_string()];
    Herdr::new(t).prompt_wait(&s.pane_id, &text, &until, Some(AGENT_WAIT))?;
    let note = shell(
        &s.host,
        &format!("cat {}", crate::proc::sh_quote(&path)),
        20,
    )
    .map_err(|_| format!("{} didn't write its note to {path}", s.name))?;
    if note.trim().is_empty() {
        return Err(format!("{} left its note empty", s.name));
    }
    Ok(note)
}

/// Hand `s` to `to`: the note (the agent's, or Aleph's), then the new session.
pub fn run(
    cfg: &Config,
    t: &Target,
    s: &Session,
    to: &str,
    by_agent: bool,
    say: &mut dyn FnMut(String),
) -> Result<Done, String> {
    let to = crate::agents::resolve(to)?;
    if to == crate::agents::SHELL {
        return Err("a plain shell can't take over a task".into());
    }
    let fallback = cfg.flag("handoff.fallback");
    let mut fell_back = None;
    let mut note = None;
    if by_agent {
        if at_limit(s) {
            let why = format!("{} is at its usage limit", s.name);
            if !fallback {
                return Err(format!("{why} (handoff.fallback is off)"));
            }
            fell_back = Some(why);
        } else {
            match agent_note(cfg, t, s, say) {
                Ok(n) => note = Some(n),
                Err(e) if fallback => fell_back = Some(e),
                Err(e) => return Err(e),
            }
        }
    }
    let by = if note.is_some() {
        "its own"
    } else {
        "built by Aleph"
    };
    let note = match note {
        Some(n) => n,
        None => {
            say("writing the hand-off note from git and its screen…".into());
            let git = shell(&s.host, &git_line(&s.cwd), 30).unwrap_or_default();
            let screen = Herdr::new(t).read_pane(&s.pane_id, 80).unwrap_or_default();
            aleph_note(s, &git, &screen)
        }
    };
    let mut o = cli::parse_new(&[], cfg)?.opts;
    o.agent = to.to_string();
    if !s.cwd.is_empty() {
        o.dir = Some(s.cwd.as_bytes().to_vec());
    }
    o.prompt = Some(intro(s, &note, by));
    o.no_focus = true;
    cli::fill_names(&mut o);
    say(format!("starting {}…", crate::agents::label(to)));
    let started = Herdr::new(t).new_session(&o, &mut |_, m| say(m))?;
    crate::launch::after_start(t, &o, &started);
    let new_name = started.first().map(|x| x.name.clone()).unwrap_or_default();
    // on the new session: it's where the work goes on (and two hand-offs
    // of one session are two events, not one twice)
    crate::events::append(crate::events::Event {
        t: crate::cache::now(),
        host: s.host.clone(),
        name: new_name.clone(),
        kind: crate::events::Kind::Remote,
        title: format!(
            "{new_name} took over from {} ({}), with {} hand-off note",
            s.name,
            crate::agents::label(&s.agent),
            if by == "its own" {
                "its own"
            } else {
                "Aleph's"
            }
        ),
        agent: to.to_string(),
        source: "aleph".into(),
        body: note.chars().take(2000).collect(),
        ..Default::default()
    });
    Ok(Done {
        by: if by == "its own" { "agent" } else { "aleph" },
        to: to.to_string(),
        started,
        fell_back,
    })
}

pub fn cmd(a: Args) -> ExitCode {
    let pos = a.positional();
    let Some(q) = pos.first() else {
        return fail(
            "usage: aleph handoff <session> [--to AGENT] [--summary agent|aleph] [--json]",
        );
    };
    let opt = |k: &str| {
        a.rest
            .iter()
            .position(|x| x == k)
            .and_then(|i| a.rest.get(i + 1))
            .cloned()
    };
    let (t, s) = match cli::find(&a.cfg, &a.flag, q) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let to = opt("--to").unwrap_or_else(|| default_to(&a.cfg, &s.agent));
    let by_agent = match opt("--summary")
        .unwrap_or_else(|| a.cfg.get_or("handoff.summary"))
        .as_str()
    {
        "aleph" => false,
        "agent" | "" => true,
        x => return fail(&format!("--summary {x}: agent or aleph")),
    };
    let quiet = a.has("--json");
    let sp = (!quiet).then(|| Spinner::start(format!("Handing {} over", s.display())));
    let r = run(&a.cfg, &t, &s, &to, by_agent, &mut |m| {
        if let Some(sp) = &sp {
            sp.set(m);
        }
    });
    if let Some(sp) = sp {
        sp.stop();
    }
    match r {
        Ok(d) => {
            let new = d
                .started
                .first()
                .map(|x| x.name.clone())
                .unwrap_or_default();
            if quiet {
                json(
                    &json!({"from": s.name, "host": s.host, "to": d.to, "session": new, "note_by": d.by, "fell_back": d.fell_back}),
                );
                return ExitCode::SUCCESS;
            }
            if let Some(why) = &d.fell_back {
                warn(&format!("{why}: Aleph wrote the hand-off note instead"));
            }
            ok(&format!(
                "{} took over from {} on {} ({} note)",
                new,
                s.name,
                s.host,
                if d.by == "agent" {
                    "its own"
                } else {
                    "Aleph's"
                }
            ));
            hint(&format!(
                "{} is still there: aleph close {}:{} when you're done with it",
                s.name, s.host, s.name
            ));
            ExitCode::SUCCESS
        }
        Err(e) => {
            crate::events::append(crate::events::Event {
                t: crate::cache::now(),
                host: s.host.clone(),
                name: s.name.clone(),
                kind: crate::events::Kind::Error,
                title: format!("couldn't hand {} over: {e}", s.name),
                agent: s.agent.clone(),
                source: "aleph".into(),
                ..Default::default()
            });
            fail(&format!("couldn't hand {} over: {e}", s.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alephs_note_has_the_task_the_diff_and_the_screen() {
        let s = Session {
            host: "b31".into(),
            name: "ignite".into(),
            agent: "claude".into(),
            cwd: "/srv/px".into(),
            title: "Profile the layout engine".into(),
            ..Default::default()
        };
        // a diff stat's histogram has dashes of its own
        let git = "branch: feat/lut\n M src/a.rs\n@@aleph@@\n src/a.rs | 12 +++---\n 1 file changed\n@@aleph@@\nabc123 fit the LUT\n";
        let n = aleph_note(&s, git, "line\n\n⏺ SIMD is 2.3× faster\n");
        assert!(
            n.contains("Task: Profile the layout engine") && n.contains("branch: feat/lut"),
            "{n}"
        );
        assert!(
            n.contains("## The diff so far") && n.contains("1 file changed"),
            "{n}"
        );
        assert!(
            n.contains("## Recent commits\nabc123") && n.contains("SIMD is 2.3× faster"),
            "{n}"
        );
        // no git: no empty sections
        let n = aleph_note(&s, "", "");
        assert!(!n.contains("##"), "{n}");
        let i = intro(&s, "the note", "its own");
        assert!(i.contains("Claude Code in session ignite") && i.ends_with("the note"));
        let cfg = Config::in_memory(&[]);
        assert_eq!(default_to(&cfg, "claude"), "codex");
        assert_eq!(default_to(&cfg, "codex"), "claude");
        assert!(at_limit(&Session {
            limit: Some(String::new()),
            ..Default::default()
        }));
    }
}
