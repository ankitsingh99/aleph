//! Plan limits asked of the agent itself, where it can say. A log only holds
//! the last snapshot a session saw, which goes stale (and misses a reset);
//! Codex's app server answers `account/rateLimits/read` with the numbers the
//! ChatGPT usage page shows, and `account/read` with whose account it is. One
//! short JSON-RPC exchange with the official binary over the machine's shell
//! (ssh, or `sh -c` here), run beside the log read, once per Codex home;
//! nothing is kept but the numbers, the plan, the account's email and a hash
//! of its id (one account on two machines is one account). No token is read
//! or asked for: Codex itself holds its sign-in.

use super::parse::{self, Limit, Window};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

/// The requests: introduce ourselves, then ask for the limits (answered as id 2).
const CODEX_ASK: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"aleph","version":""#,
    env!("CARGO_PKG_VERSION"),
    r#""}}}"#,
    "\n",
    r#"{"jsonrpc":"2.0","method":"initialized"}"#,
    "\n",
    r#"{"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read"}"#,
    "\n",
    r#"{"jsonrpc":"2.0","id":3,"method":"account/read","params":{"refreshToken":false}}"#,
    "\n"
);

/// What Codex's app server said: its plan windows, and whose account (its
/// email and plan, when `account/read` gave them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asked {
    pub limit: Option<Limit>,
    pub email: String,
    pub plan: String,
}

/// Codex's plan windows and account on `host` (`home`: its `CODEX_HOME`, None
/// the default), live (None: no codex there, not logged in, no answer in time).
pub fn codex(host: &str, home: Option<&str>, timeout: Duration) -> Option<Asked> {
    if let Some(l) = crate::showcase::codex_limits(host) {
        return l.map(|l| Asked {
            plan: l.plan.clone(),
            limit: Some(l),
            ..Default::default()
        });
    }
    let env = match home {
        Some(h) if !h.contains('\'') && !h.chars().any(char::is_control) => {
            format!("CODEX_HOME='{h}' ")
        }
        Some(_) => return None,
        None => String::new(),
    };
    let cmd = format!(
        "{}; command -v codex >/dev/null 2>&1 || exit 0; {env}exec codex app-server",
        crate::chan::PATH_PRELUDE
    );
    let mut c = crate::sshcfg::shell(host, &cmd);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = c.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    let out = child.stdout.take()?;
    let _ = stdin.write_all(CODEX_ASK.as_bytes());
    let _ = stdin.flush();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut n = 0;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if line.contains(r#""id":2"#) || line.contains(r#""id":3"#) {
                let _ = tx.send(line);
                n += 1;
                if n == 2 {
                    break;
                }
            }
        }
    });
    let t0 = std::time::Instant::now();
    let (mut limits, mut account) = (None, None);
    while limits.is_none() || account.is_none() {
        let Some(left) = timeout.checked_sub(t0.elapsed()) else {
            break;
        };
        let Ok(line) = rx.recv_timeout(left) else {
            break;
        };
        if line.contains(r#""id":2"#) {
            limits = Some(line);
        } else {
            account = Some(line);
        }
    }
    // stdin stays open until the answers are in: the server stops at its end
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let limit = limits.and_then(|l| codex_limits(&l, crate::cache::now()));
    let (email, plan) = account.as_deref().map(codex_account).unwrap_or_default();
    (limit.is_some() || !email.is_empty()).then(|| Asked {
        plan: if plan.is_empty() {
            limit.as_ref().map(|l| l.plan.clone()).unwrap_or_default()
        } else {
            plan
        },
        limit,
        email,
    })
}

/// The answer to `account/read`: the account's email and plan (either may be
/// missing: an API key has neither). Read by key wherever they are, so a
/// newer shape still gives them.
pub fn codex_account(line: &str) -> (String, String) {
    fn find(v: &Value, k: &str) -> Option<String> {
        match v {
            Value::Object(m) => m
                .get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| m.values().find_map(|x| find(x, k))),
            Value::Array(a) => a.iter().find_map(|x| find(x, k)),
            _ => None,
        }
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Default::default();
    };
    let Some(r) = v.get("result") else {
        return Default::default();
    };
    let email = find(r, "email")
        .filter(|e| e.contains('@') && !e.chars().any(char::is_control))
        .unwrap_or_default();
    (email, find(r, "planType").unwrap_or_default())
}

/// The answer to `account/rateLimits/read` as a snapshot taken `now`.
pub fn codex_limits(line: &str, now: u64) -> Option<Limit> {
    let v: Value = serde_json::from_str(line).ok()?;
    let r = v.get("result")?;
    let rl = r.get("rateLimits")?;
    let windows: Vec<Window> = ["primary", "secondary"]
        .iter()
        .filter_map(|k| {
            let w = rl.get(*k)?;
            Some(Window {
                used_percent: w.get("usedPercent")?.as_f64()?,
                minutes: w
                    .get("windowDurationMins")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                resets_at: w.get("resetsAt").and_then(Value::as_u64).map(|t| {
                    if t > 100_000_000_000 {
                        t / 1000
                    } else {
                        t
                    }
                }),
            })
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    let s = |x: Option<&Value>| x.and_then(Value::as_str).unwrap_or("").to_string();
    let account = s(r.get("accountId"));
    Some(Limit {
        ts: now,
        id: s(rl.get("limitId")),
        plan: s(rl.get("planType")),
        windows,
        source: "live".into(),
        account: if account.is_empty() {
            String::new()
        } else {
            parse::hash(&["codex", &account])
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_answer() {
        let line = r#"{"id":2,"result":{"ordinaryUsageAllowed":true,"rateLimits":{"limitId":"codex","primary":{"usedPercent":1,"windowDurationMins":10080,"resetsAt":1793296560},"secondary":null,"planType":"prolite"},"accountId":"0e1d2c3b-4a59-4687-9a1b-2c3d4e5f6a7b"}}"#;
        let l = codex_limits(line, 100).unwrap();
        assert_eq!(
            (l.ts, l.id.as_str(), l.plan.as_str(), l.source.as_str()),
            (100, "codex", "prolite", "live")
        );
        assert_eq!(
            l.windows,
            [Window {
                used_percent: 1.0,
                minutes: 10080,
                resets_at: Some(1_793_296_560)
            }]
        );
        assert_eq!(l.account.len(), 16, "the account as a hash, never its id");
        assert!(!l.account.contains("bcef"));
        assert!(codex_limits(r#"{"id":2,"error":{"message":"not logged in"}}"#, 1).is_none());
        assert!(codex_limits(r#"{"id":2,"result":{"rateLimits":{"primary":null}}}"#, 1).is_none());
    }

    #[test]
    fn whose_account_it_is() {
        let (e, p) = codex_account(
            r#"{"id":3,"result":{"account":{"type":"chatgpt","email":"me@chatgpt.dev","planType":"pro"},"requiresOpenaiAuth":true}}"#,
        );
        assert_eq!((e.as_str(), p.as_str()), ("me@chatgpt.dev", "pro"));
        assert_eq!(
            codex_account(r#"{"id":3,"result":{"account":{"type":"apiKey"}}}"#),
            Default::default()
        );
        assert_eq!(
            codex_account(r#"{"id":3,"error":{"message":"no"}}"#),
            Default::default()
        );
    }

    #[test]
    fn the_ask_is_four_lines() {
        assert_eq!(CODEX_ASK.lines().count(), 4);
        for l in CODEX_ASK.lines() {
            serde_json::from_str::<Value>(l).unwrap();
        }
    }
}
