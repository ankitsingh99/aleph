//! Watch rules: tell me when a session's output says something.
//!
//! ```toml
//! [watch.tests]
//! match = "FAILED|Traceback"   # what to look for
//! regex = true                 # a regular expression (else plain text)
//! hosts = "b31,gpu*"           # which machines (globs; default all)
//! agents = "claude,codex"      # which agents (default all)
//! source = "recent"            # recent output (default) or the visible screen
//! ```
//!
//! herdr does the matching — `pane.output_matched` subscriptions through the
//! event listener, one per rule per machine, over the panes the rule
//! covers — and a match lands in the Inbox as a `match` event, which the
//! notification routes send on (to the desktop by default, like questions).
//! `aleph notify watch` lists, adds and removes rules.

use crate::config::Config;
use serde_json::{json, Value};

/// One rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    pub pattern: String,
    pub regex: bool,
    pub hosts: Vec<String>,
    pub agents: Vec<String>,
    /// `recent` or `visible`
    pub source: String,
}

impl Rule {
    pub fn applies(&self, host: &str, agent: &str) -> bool {
        let ok = |list: &[String], v: &str| {
            list.is_empty() || list.iter().any(|g| crate::permit::glob(g, v))
        };
        ok(&self.hosts, host) && ok(&self.agents, agent)
    }
}

fn list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty() && x != "*")
        .collect()
}

/// The rules in the settings (`watch.<name>.match` and friends).
pub fn rules(cfg: &Config) -> Vec<Rule> {
    let mut names: Vec<String> = cfg
        .with_prefix("watch.")
        .into_iter()
        .filter_map(|(k, _)| k.strip_suffix(".match").map(str::to_string))
        .filter(|n| !n.is_empty() && !n.contains('.'))
        .collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|n| {
            let get = |f: &str| cfg.get(&format!("watch.{n}.{f}"));
            let pattern = get("match").filter(|m| !m.is_empty())?;
            Some(Rule {
                regex: get("regex")
                    .is_some_and(|v| matches!(v.as_str(), "true" | "on" | "1" | "yes")),
                hosts: list(&get("hosts").unwrap_or_default()),
                agents: list(&get("agents").unwrap_or_default()),
                source: match get("source").as_deref() {
                    Some("visible") => "visible".into(),
                    _ => "recent".into(),
                },
                name: n,
                pattern,
            })
        })
        .collect()
}

/// One subscription the listener keeps: a rule and the panes it covers on a
/// machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub rule: String,
    pub pattern: String,
    pub regex: bool,
    pub source: String,
    pub panes: Vec<String>,
}

/// The subscriptions for a machine's sessions (pane, agent) (pure — tested).
pub fn plans(rules: &[Rule], host: &str, sessions: &[(String, String)]) -> Vec<Plan> {
    rules
        .iter()
        .filter_map(|r| {
            let mut panes: Vec<String> = sessions
                .iter()
                .filter(|(_, agent)| r.applies(host, agent))
                .map(|(p, _)| p.clone())
                .collect();
            panes.sort();
            (!panes.is_empty()).then(|| Plan {
                rule: r.name.clone(),
                pattern: r.pattern.clone(),
                regex: r.regex,
                source: r.source.clone(),
                panes,
            })
        })
        .collect()
}

/// The `events.subscribe` line for one plan (pure — tested).
pub fn request(id: &str, p: &Plan) -> String {
    let m = json!({"type": if p.regex { "regex" } else { "substring" }, "value": p.pattern});
    let subs: Vec<Value> = p
        .panes
        .iter()
        .map(|pane| {
            json!({"type": "pane.output_matched", "pane_id": pane, "source": p.source,
                   "match": m, "lines": 40, "strip_ansi": true})
        })
        .collect();
    let mut s = json!({"id": id, "method": "events.subscribe", "params": {"subscriptions": subs}})
        .to_string();
    s.push('\n');
    s
}

/// The line an `output_matched` event says matched.
pub fn matched_line(data: &Value) -> String {
    data.get("matched_line")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            data.pointer("/read/text")
                .and_then(Value::as_str)
                .and_then(|t| t.lines().rev().find(|l| !l.trim().is_empty()))
                .map(str::to_string)
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// `aleph notify watch [ls | add <name> <text> [--regex] [--host H] [--agent A] [--visible] | rm <name>]`
pub fn cmd(mut a: crate::cli::Args, p: &[String]) -> std::process::ExitCode {
    use crate::out::{err, hint, ok};
    use std::process::ExitCode;
    let flag_val = |k: &str| {
        a.rest
            .iter()
            .position(|x| x == k)
            .and_then(|i| a.rest.get(i + 1))
            .cloned()
    };
    match p.get(1).map(String::as_str) {
        None | Some("ls") | Some("list") => {
            let rs = rules(&a.cfg);
            if a.has("--json") {
                crate::out::json(&Value::Array(
                    rs.iter()
                        .map(|r| json!({"name": r.name, "match": r.pattern, "regex": r.regex, "hosts": r.hosts, "agents": r.agents, "source": r.source}))
                        .collect(),
                ));
                return ExitCode::SUCCESS;
            }
            if rs.is_empty() {
                ok("no watch rules");
                hint("aleph notify watch add tests 'FAILED' — a match lands in the Inbox and notifies you");
                return ExitCode::SUCCESS;
            }
            for r in &rs {
                println!(
                    "  {:<14} {}{}  {}{}",
                    r.name,
                    if r.regex { "/" } else { "\"" },
                    r.pattern,
                    if r.hosts.is_empty() {
                        "every machine".to_string()
                    } else {
                        r.hosts.join(",")
                    },
                    if r.agents.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", r.agents.join(","))
                    }
                );
            }
            ExitCode::SUCCESS
        }
        Some("add") => {
            let (Some(name), Some(text)) = (p.get(2), p.get(3)) else {
                return crate::out::fail(
                    "usage: aleph notify watch add <name> <text> [--regex] [--host H] [--agent A] [--visible]",
                );
            };
            if !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return crate::out::fail("rule names are letters, digits, - and _");
            }
            let k = |f: &str| format!("watch.{name}.{f}");
            let hosts = flag_val("--host");
            let agents = flag_val("--agent");
            let mut r = a.cfg.set(&k("match"), text);
            if r.is_ok() && a.has("--regex") {
                r = a.cfg.set(&k("regex"), "true");
            }
            if r.is_ok() {
                if let Some(h) = hosts {
                    r = a.cfg.set(&k("hosts"), &h);
                }
            }
            if r.is_ok() {
                if let Some(ag) = agents {
                    r = a.cfg.set(&k("agents"), &ag);
                }
            }
            if r.is_ok() && a.has("--visible") {
                r = a.cfg.set(&k("source"), "visible");
            }
            match r {
                Ok(()) => {
                    ok(&format!("watching for {text:?} ({name})"));
                    hint("the daemon (or the dashboard) picks it up within a minute; matches land in the Inbox");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    err(&e.to_string());
                    ExitCode::from(1)
                }
            }
        }
        Some("rm") | Some("remove") => {
            let Some(name) = p.get(2) else {
                return crate::out::fail("usage: aleph notify watch rm <name>");
            };
            match a.cfg.unset_prefix(&format!("watch.{name}.")) {
                Ok(()) => {
                    ok(&format!("removed the watch rule {name}"));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    err(&e.to_string());
                    ExitCode::from(1)
                }
            }
        }
        Some(x) => crate::out::fail(&format!("aleph notify watch {x}: ls, add or rm")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_from_settings() {
        let c = Config::in_memory(&[
            "watch.tests.match=FAILED|Traceback",
            "watch.tests.regex=true",
            "watch.tests.hosts=b31, gpu*",
            "watch.done.match=All done",
            "watch.done.agents=codex",
            "watch.bad.regex=true",
        ]);
        let rs = rules(&c);
        assert_eq!(rs.len(), 2);
        let t = rs.iter().find(|r| r.name == "tests").unwrap();
        assert!(t.regex && t.hosts == ["b31", "gpu*"]);
        assert!(t.applies("gpu2", "claude") && !t.applies("dex42", "claude"));
        let d = rs.iter().find(|r| r.name == "done").unwrap();
        assert!(!d.regex && d.applies("dex42", "codex") && !d.applies("dex42", "claude"));
        let ps = plans(
            &rs,
            "b31",
            &[
                ("w1:p2".into(), "claude".into()),
                ("w1:p3".into(), "codex".into()),
            ],
        );
        assert_eq!(ps.len(), 2);
        let tp = ps.iter().find(|p| p.rule == "tests").unwrap();
        assert_eq!(tp.panes, vec!["w1:p2", "w1:p3"]);
        let dp = ps.iter().find(|p| p.rule == "done").unwrap();
        assert_eq!(dp.panes, vec!["w1:p3"]);
        let line = request("w1", tp);
        let v: Value = serde_json::from_str(&line).unwrap();
        let s = &v["params"]["subscriptions"][0];
        assert_eq!(s["type"], "pane.output_matched");
        assert_eq!(s["match"]["type"], "regex");
        assert_eq!(s["match"]["value"], "FAILED|Traceback");
        assert_eq!(s["source"], "recent");
        assert_eq!(
            matched_line(&json!({"matched_line": " 1 FAILED  "})),
            "1 FAILED"
        );
        assert_eq!(
            matched_line(&json!({"read": {"text": "a\nE assert x\n\n"}})),
            "E assert x"
        );
    }
}
