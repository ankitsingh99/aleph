//! The prompt library: saved prompts with variables, insertable wherever you
//! write one (the send box, the queue, a new session's first prompt) with
//! ctrl+p or as `@name`, and from the shell (`aleph send x @review`).
//!
//! They're settings, `prompt.<name>` (`[prompts]` in aleph.toml), so a
//! profile or a backup carries them. In the text, `{dir}`, `{folder}`,
//! `{host}`, `{session}`, `{agent}`, `{task}` (the session's title), `{date}`
//! and `{time}` fill themselves in from the session; any other `{word}` is
//! asked for when you use it (or `--var word=…` from the shell). review,
//! tests, commit, handoff and explain are starters until you save your own
//! by those names.

use crate::cli::Args;
use crate::config::Config;
use crate::out::*;
use ratatui::prelude::*;
use serde_json::json;
use std::process::ExitCode;

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub name: String,
    pub text: String,
    pub starter: bool,
}

const STARTERS: &[(&str, &str)] = &[
    (
        "review",
        "Review the changes in {folder} since the last commit: bugs first, then anything risky or unclear. Don't change any files yet.",
    ),
    (
        "tests",
        "Run the test suite in {folder}, fix what fails, and tell me what you changed and why.",
    ),
    (
        "commit",
        "Write a commit message for what you changed, in this repo's style, and commit it.",
    ),
    (
        "handoff",
        "Summarise this session for whoever takes it over: the task, what's done, the decisions made and why, what's left, and anything half-done.",
    ),
    (
        "explain",
        "Explain how {topic} works in this codebase: where it lives, how the pieces connect, and anything surprising.",
    ),
];

/// Every prompt: yours (by name), then the starters you haven't replaced.
pub fn all(cfg: &Config) -> Vec<Prompt> {
    let mut v: Vec<Prompt> = cfg
        .entries()
        .into_iter()
        .filter_map(|(k, text)| {
            let name = k.strip_prefix("prompt.")?;
            (!name.is_empty() && !name.contains('.')).then(|| Prompt {
                name: name.to_string(),
                text,
                starter: false,
            })
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    for (n, t) in STARTERS {
        if !v.iter().any(|p| p.name == *n) {
            v.push(Prompt {
                name: n.to_string(),
                text: t.to_string(),
                starter: true,
            });
        }
    }
    v
}

pub fn get(cfg: &Config, name: &str) -> Option<Prompt> {
    let name = name.trim_start_matches('@');
    all(cfg).into_iter().find(|p| p.name == name)
}

/// A prompt's name from what you'd type: `Fix the ticket` → `fix-the-ticket`.
pub fn slug(s: &str) -> String {
    crate::workspace::slug(s)
}

/// The variables in a text, in order, each once: `{topic}`, `{ticket_id}`.
pub fn vars(text: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        let n = after
            .bytes()
            .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        if n > 0 && after.as_bytes().get(n) == Some(&b'}') && !after.as_bytes()[0].is_ascii_digit()
        {
            let name = &after[..n];
            if !v.iter().any(|x| x == name) {
                v.push(name.to_string());
            }
            rest = &after[n + 1..];
        } else {
            rest = after;
        }
    }
    v
}

/// What a prompt's session is: the values the automatic variables take.
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    pub host: String,
    pub session: String,
    pub agent: String,
    pub dir: String,
    pub task: String,
}

impl Ctx {
    pub fn of(s: &crate::herdr::Session) -> Ctx {
        Ctx {
            host: s.host.clone(),
            session: s.name.clone(),
            agent: crate::agents::find(&s.agent)
                .map(|a| a.name.to_string())
                .unwrap_or_else(|| s.agent.clone()),
            dir: s.cwd.clone(),
            task: s.title.clone(),
        }
    }

    /// An automatic variable's value (None: not one, or nothing to say).
    pub fn value(&self, var: &str) -> Option<String> {
        let now = crate::cache::now();
        let v = match var {
            "dir" => self.dir.clone(),
            "folder" => self
                .dir
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string(),
            "host" | "machine" => self.host.clone(),
            "session" => self.session.clone(),
            "agent" => self.agent.clone(),
            "task" => self.task.clone(),
            "date" => {
                let d = crate::ui::local_day(now);
                let (y, m, day) = crate::ui::civil(d);
                format!("{y:04}-{m:02}-{day:02}")
            }
            "time" => crate::ui::clock(now),
            _ => return None,
        };
        (!v.is_empty()).then_some(v)
    }
}

/// The variables a text still needs from you, given its session.
pub fn asks(text: &str, ctx: &Ctx) -> Vec<String> {
    vars(text)
        .into_iter()
        .filter(|v| ctx.value(v).is_none())
        .collect()
}

/// The text with every variable filled: the session's, then `given`.
pub fn fill(text: &str, ctx: &Ctx, given: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for v in vars(text) {
        let val = given
            .iter()
            .find(|(k, _)| *k == v)
            .map(|(_, x)| x.clone())
            .or_else(|| ctx.value(&v));
        if let Some(x) = val {
            out = out.replace(&format!("{{{v}}}"), &x);
        }
    }
    out
}

/// `@name` as the whole of a prompt, expanded (Err: what's missing).
pub fn expand(
    cfg: &Config,
    text: &str,
    ctx: &Ctx,
    given: &[(String, String)],
) -> Option<Result<String, String>> {
    let t = text.trim();
    let name = t.strip_prefix('@')?;
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let p = get(cfg, name)?;
    let missing: Vec<String> = asks(&p.text, ctx)
        .into_iter()
        .filter(|v| !given.iter().any(|(k, _)| k == v))
        .collect();
    Some(if missing.is_empty() {
        Ok(fill(&p.text, ctx, given))
    } else {
        Err(format!(
            "@{name} needs {}: --var {}=…",
            missing
                .iter()
                .map(|v| format!("{{{v}}}"))
                .collect::<Vec<_>>()
                .join(", "),
            missing[0]
        ))
    })
}

pub fn cmd(mut a: Args) -> ExitCode {
    let pos: Vec<String> = a.positional().iter().map(|s| s.to_string()).collect();
    let json_out = a.has("--json");
    match pos.first().map(String::as_str) {
        None | Some("ls" | "list") => {
            let all = all(&a.cfg);
            if json_out {
                json(&json!(all
                    .iter()
                    .map(|p| json!({"name": p.name, "text": p.text, "vars": vars(&p.text), "starter": p.starter}))
                    .collect::<Vec<_>>()));
                return ExitCode::SUCCESS;
            }
            head("prompts  ·  ctrl+p or @name where you write one");
            for p in &all {
                out(Line::from(vec![
                    Span::styled(
                        format!("  @{}  ", crate::ui::pad(&p.name, 12)),
                        crate::ui::fg(crate::ui::c::ACCENT()),
                    ),
                    Span::raw(crate::ui::trunc(&p.text, 90)),
                    Span::styled(if p.starter { "  starter" } else { "" }, crate::ui::faint()),
                ]));
            }
            hint("aleph prompt add NAME \"text with {vars}\" · aleph send b31:hunt @review");
            ExitCode::SUCCESS
        }
        Some("show") => match pos.get(1).and_then(|n| get(&a.cfg, n)) {
            Some(p) => {
                raw(&p.text);
                ExitCode::SUCCESS
            }
            None => fail("no such prompt (aleph prompt ls)"),
        },
        Some("add" | "set") => {
            let (Some(n), true) = (pos.get(1), pos.len() > 2) else {
                return fail("usage: aleph prompt add NAME TEXT…");
            };
            let name = slug(n);
            if name.is_empty() {
                return fail("a prompt's name needs a letter or digit");
            }
            let text = pos[2..].join(" ");
            match a.cfg.set(&format!("prompt.{name}"), &text) {
                Ok(_) => {
                    ok(&format!("@{name} saved"));
                    let v = vars(&text);
                    if !v.is_empty() {
                        hint(&format!(
                            "its variables: {}",
                            v.iter()
                                .map(|x| format!("{{{x}}}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        ));
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e.to_string()),
            }
        }
        Some("rm") => match pos.get(1) {
            Some(n) => {
                let k = format!("prompt.{}", n.trim_start_matches('@'));
                if a.cfg.get_raw(&k).is_none() {
                    return fail(&format!("no saved prompt {n}"));
                }
                match a.cfg.unset(&k) {
                    Ok(_) => {
                        ok(&format!("{n} deleted"));
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(&e.to_string()),
                }
            }
            None => fail("usage: aleph prompt rm NAME"),
        },
        Some(_) => fail("usage: aleph prompt [ls] | show NAME | add NAME TEXT… | rm NAME"),
    }
}

/// `--var k=v` pairs on a command line.
pub fn given(rest: &[String]) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--var" {
            if let Some((k, x)) = rest.get(i + 1).and_then(|s| s.split_once('=')) {
                v.push((k.to_string(), x.to_string()));
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_fill_from_the_session_and_ask_for_the_rest() {
        assert_eq!(
            vars("fix {ticket} in {dir}, then {ticket} again; {} {9x} {Up}"),
            ["ticket", "dir"]
        );
        let ctx = Ctx {
            host: "b31".into(),
            session: "hunt".into(),
            agent: "Claude Code".into(),
            dir: "/srv/px/".into(),
            task: String::new(),
        };
        assert_eq!(
            asks("explain {topic} in {folder} on {host} ({task})", &ctx),
            ["topic", "task"]
        );
        assert_eq!(
            fill(
                "{folder} on {host}: {topic}",
                &ctx,
                &[("topic".into(), "LUTs".into())]
            ),
            "px on b31: LUTs"
        );
        let cfg = Config::in_memory(&["prompt.fix=Fix {ticket} in {folder}"]);
        assert_eq!(
            expand(&cfg, "@fix", &ctx, &[("ticket".into(), "VD-7".into())])
                .unwrap()
                .unwrap(),
            "Fix VD-7 in px"
        );
        assert!(expand(&cfg, "@fix", &ctx, &[])
            .unwrap()
            .unwrap_err()
            .contains("{ticket}"));
        // starters, and not every @ is a prompt
        assert!(expand(&cfg, "@review", &ctx, &[]).unwrap().is_ok());
        assert!(expand(&cfg, "@nope", &ctx, &[]).is_none());
        assert!(expand(&cfg, "@fix this please", &ctx, &[]).is_none());
        assert!(expand(&cfg, "plain text", &ctx, &[]).is_none());
        assert_eq!(
            given(&["--var".into(), "a=b c".into(), "x".into()]),
            [("a".to_string(), "b c".to_string())]
        );
    }
}
