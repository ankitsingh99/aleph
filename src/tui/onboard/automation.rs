//! Step 8, Automation: what Aleph does on its own. Auto-approve answers a
//! question with its own yes option when a rule matches it (`approve.<name>
//! = pattern`) and nothing on the deny list does (`approve_deny`: rm -rf,
//! sudo, git push, … always wait for you); self-heal brings a machine's
//! herdr back; after a usage limit a session can carry on by itself. A
//! question can be tried against the rules as they'll be.

use super::rows::Row;
use super::{Aside, Ev, Hand, Wiz};
use crate::auto::pmatch;
use crate::ui::{self, c};
use ratatui::prelude::*;

#[derive(Default)]
pub(crate) struct State {
    /// the last question tried, and what the rules said
    pub tried: Option<(String, Result<String, String>)>,
}

/// Rules that are safe for most people.
pub(crate) const STARTERS: &[(&str, &str)] = &[
    ("tests", "*pytest*"),
    ("cargo-test", "*cargo test*"),
    ("npm-test", "*npm test*"),
    ("git-status", "*git status*"),
    ("git-diff", "*git diff*"),
];

/// The rules as they'll be: (name, pattern), the plan's changes included.
pub(crate) fn rules(w: &Wiz) -> Vec<(String, String)> {
    let mut names: Vec<String> = w
        .cfg
        .with_prefix("approve.")
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    for s in &w.plan.sets {
        if let Some(n) = s.key.strip_prefix("approve.") {
            if !names.iter().any(|x| x == n) {
                names.push(n.to_string());
            }
        }
    }
    names
        .into_iter()
        .map(|n| {
            let v = w.value(&format!("approve.{n}"));
            (n, v)
        })
        .filter(|(_, v)| !v.trim().is_empty())
        .collect()
}

fn deny(w: &Wiz) -> Vec<String> {
    super::split(&w.value("approve_deny"))
}

/// Pure: what the rules do with `q` (the deny list first).
pub(crate) fn decide(
    rules: &[(String, String)],
    deny: &[String],
    q: &str,
) -> Result<String, String> {
    if let Some(d) = deny.iter().find(|d| pmatch(d, q)) {
        return Err(format!("never: it matches the deny list ({d})"));
    }
    match rules.iter().find(|(_, p)| pmatch(p, q)) {
        Some((n, p)) => Ok(format!("approved by rule {n} ({p})")),
        None => Err("no rule matches — it waits for you".into()),
    }
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let mut v = vec![Row::head("Auto-approve")];
    v.push(
        Row::toggle("auto_approve", "auto-approve", w.flag("auto_approve"), w.pending("auto_approve"))
            .note("answers questions a rule below matches, with their own yes")
            .why("When a session asks a question that one of your rules matches, the hub answers it with the question's own yes option (never a blind 1). Every answer is in the Inbox. Runs in the daemon, or a dashboard while no daemon runs."),
    );
    let rs = rules(w);
    for (n, p) in &rs {
        let key = format!("approve.{n}");
        v.push(
            Row::input(key.clone(), n.clone(), p.clone(), "a pattern with *", w.pending(&key))
                .why("A rule: questions containing this pattern (* matches anything, case doesn't matter) are approved. Empty removes it."),
        );
    }
    if rs.is_empty() {
        v.push(Row::info("No rules yet."));
    }
    v.push(Row::button("rule-add", "+ add a rule…"));
    if STARTERS
        .iter()
        .any(|(n, _)| !rs.iter().any(|(x, _)| x == n))
    {
        v.push(Row::button(
            "starters",
            "add the usual safe ones (tests, git status, git diff)",
        ));
    }
    v.push(
        Row::input("approve_deny", "never", w.value("approve_deny"), "patterns, comma-separated", w.pending("approve_deny"))
            .why("Questions matching any of these are never answered automatically — not by a rule, not from a phone. They wait for you at a terminal."),
    );
    v.push(Row::button("try", "try a question…").why("Type (or paste) a question an agent might ask: setup says what the rules, as they'll be, would do with it."));
    if let Some((q, r)) = &w.au.tried {
        let (col, t) = match r {
            Ok(t) => (c::GREEN(), format!("✓ {t}")),
            Err(t) => (c::YELLOW(), format!("○ {t}")),
        };
        v.push(Row::line(vec![
            Span::raw("    "),
            Span::styled(format!("“{}” ", ui::trunc(q, 30)), ui::faint()),
            Span::styled(t, ui::fg(col)),
        ]));
    }
    v.push(Row::head("Self-heal"));
    v.push(
        Row::toggle("auto_heal", "bring herdr back", w.flag("auto_heal"), w.pending("auto_heal"))
            .note("restarts a machine's herdr server when it stops, then restores sessions")
            .why("When a machine answers but its herdr server is gone (a reboot, a crash), the hub starts it again and brings back the sessions it remembers (aleph restore --yes)."),
    );
    v.push(Row::head("After a usage limit"));
    v.push(Row::info("Ticked agents carry on by themselves once their limit resets, each with its own message; the rest wait for you."));
    let globs = split_globs(&w.value("limits.auto_resume"));
    let all = globs.iter().any(|g| g == "*:*");
    v.push(
        Row::pick("resume:*", "every agent", all)
            .note("now and later")
            .why("limits.auto_resume = *:* — every agent on every machine, including ones you add later."),
    );
    for a in resume_agents(w) {
        let on = all || globs.iter().any(|g| g == &format!("*:{}", a.id));
        v.push(
            Row::pick(format!("resume:{}", a.id), a.name, on)
                .note(if on { "carries on" } else { "waits for you" })
                .why("Its sessions that stopped at the plan's usage limit get their message once the limit resets (limits.auto_resume, host:agent globs)."),
        );
        if on {
            let key = format!("resume.{}", a.id);
            v.push(Row::input(
                &key,
                format!("  {} says", a.name),
                w.value(&key),
                "its own message (empty: the one below)",
                w.pending(&key),
            ));
        }
    }
    let other: Vec<&String> = globs
        .iter()
        .filter(|g| *g != "*:*" && !(g.starts_with("*:") && crate::agents::find(&g[2..]).is_some()))
        .collect();
    if !other.is_empty() {
        v.push(Row::info(format!(
            "Also: {} (limits.auto_resume, kept as it is)",
            other
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    v.push(Row::input(
        "limits.resume_prompt",
        "resume with",
        w.value("limits.resume_prompt"),
        "what an agent without its own message is told",
        w.pending("limits.resume_prompt"),
    ));
    v.push(Row::head("Queue"));
    v.push(Row::info("Messages queued with aleph send --after (or Q in Fleet) go out when the session is free — from the daemon, or a dashboard while none runs (step 9)."));
    v
}

/// `*:claude, b31:codex` → its globs.
fn split_globs(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The agents the checklist offers: your default agent, Claude Code and
/// Codex, and any on your machines, queued, or named in the setting already.
fn resume_agents(w: &Wiz) -> Vec<&'static crate::agents::Agent> {
    // your default agent, Claude Code and Codex always; then the rest you have
    let mut ids: Vec<String> = vec![w.value("agent"), "claude".into(), "codex".into()];
    for p in w.probes.values() {
        for (a, _) in p.installed() {
            ids.push(a.id.to_string());
        }
    }
    for (_, a) in &w.plan.actions {
        if let super::Action::AgentInstall { agent, .. } = a {
            ids.push(agent.clone());
        }
    }
    for g in split_globs(&w.value("limits.auto_resume")) {
        if let Some(id) = g.strip_prefix("*:") {
            ids.push(id.to_string());
        }
    }
    crate::agents::ALL
        .iter()
        .filter(|a| ids.iter().any(|i| i == a.id))
        .collect()
}

/// Tick or untick `id` (`*` = every agent) in the globs (pure).
pub(crate) fn toggle_resume(cur: &str, id: &str) -> String {
    let mut g = split_globs(cur);
    let me = format!("*:{id}");
    if g.contains(&me) {
        g.retain(|x| *x != me);
    } else {
        if id == "*" {
            // every agent: the per-agent ones are in it
            g.retain(|x| !(x.starts_with("*:") && crate::agents::find(&x[2..]).is_some()));
        }
        g.push(me);
    }
    // unticking one under every agent: the others stay ticked
    g.join(",")
}

pub(crate) fn aside(w: &Wiz, _width: u16) -> Option<Aside> {
    let d = deny(w);
    let mut lines = vec![Line::from(Span::styled(
        "Always waits for you:",
        ui::bold(),
    ))];
    for p in d.iter().take(12) {
        lines.push(Line::from(Span::styled(
            format!("  {p}"),
            ui::fg(c::YELLOW()),
        )));
    }
    let rs = rules(w);
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        if w.flag("auto_approve") {
            format!(
                "{} rule{} answer on their own",
                rs.len(),
                if rs.len() == 1 { "" } else { "s" }
            )
        } else {
            "auto-approve is off: rules wait".into()
        },
        ui::muted(),
    )));
    Some(Aside {
        title: "Deny list".into(),
        lines,
    })
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Button(id) if id == "rule-add" => {
            w.edit(
                "New rule",
                "name=pattern, e.g. tests=*pytest* (* matches anything)",
                "",
                "rule",
            );
        }
        Ev::Input(id, t) if id == "rule" => {
            let Some((n, p)) = t.split_once('=') else {
                if !t.is_empty() {
                    w.toast(true, "name=pattern, e.g. tests=*pytest*");
                }
                return None;
            };
            let n: String = n
                .trim()
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            if n.is_empty() || p.trim().is_empty() {
                return None;
            }
            w.set(&format!("approve.{n}"), p.trim());
        }
        Ev::Button(id) if id == "starters" => {
            let rs = rules(w);
            for (n, p) in STARTERS {
                if !rs.iter().any(|(x, _)| x == n) {
                    w.set(&format!("approve.{n}"), p);
                }
            }
        }
        Ev::Button(id) if id == "try" => {
            w.edit(
                "Try a question",
                "a question as an agent would ask it, e.g. Do you want to run cargo test?",
                "",
                "try",
            );
        }
        Ev::Input(id, q) if id == "try" => {
            if !q.is_empty() {
                let r = decide(&rules(w), &deny(w), &q);
                w.au.tried = Some((q, r));
            }
        }
        Ev::Pick(id) if id.starts_with("resume:") => {
            let a = id.trim_start_matches("resume:");
            let cur = w.value("limits.auto_resume");
            let mut next = toggle_resume(&cur, a);
            // under every agent, unticking one ticks the rest by name
            if a != "*" && split_globs(&cur).iter().any(|g| g == "*:*") {
                let rest: Vec<String> = resume_agents(w)
                    .iter()
                    .filter(|x| x.id != a)
                    .map(|x| format!("*:{}", x.id))
                    .collect();
                let mut g = split_globs(&cur);
                g.retain(|x| x != "*:*");
                g.extend(rest);
                next = g.join(",");
            }
            w.set("limits.auto_resume", &next);
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resume_checklist_edits_the_globs() {
        assert_eq!(toggle_resume("", "claude"), "*:claude");
        assert_eq!(toggle_resume("*:claude", "codex"), "*:claude,*:codex");
        assert_eq!(toggle_resume("*:claude,*:codex", "claude"), "*:codex");
        // every agent takes over the ones by name; other globs stay
        assert_eq!(toggle_resume("b31:claude,*:codex", "*"), "b31:claude,*:*");
        assert_eq!(toggle_resume("*:*", "*"), "");
    }

    #[test]
    fn the_deny_list_always_wins() {
        let rules = vec![
            ("tests".to_string(), "*pytest*".to_string()),
            ("all".to_string(), "*".to_string()),
        ];
        let deny: Vec<String> = crate::auto::DEFAULT_DENY
            .split(',')
            .map(String::from)
            .collect();
        assert!(decide(&rules, &deny, "Run pytest -q?")
            .unwrap()
            .contains("tests"));
        assert!(decide(&rules, &deny, "Run sudo pytest?")
            .unwrap_err()
            .contains("deny list"));
        assert!(decide(&[], &deny, "Run pytest?")
            .unwrap_err()
            .contains("no rule"));
    }
}
