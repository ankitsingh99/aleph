//! Session history: one JSON line per session started, in
//! `$XDG_STATE_HOME/aleph/history.jsonl`. Feeds `aleph history`, the
//! History view, recent folders (frecency) in the browser and palette, and
//! name picking (recently used names are avoided).

use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

const KEEP: usize = 5000;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Entry {
    pub t: u64,
    pub host: String,
    pub dir: String,
    pub name: String,
    pub agent: String,
    pub args: Vec<String>,
    pub prompt: Option<String>,
    pub worktree: bool,
}

/// The shell command that starts this again.
pub fn command(e: &Entry) -> String {
    let q = crate::proc::sh_quote;
    let mut v = vec![crate::doctor::me().to_string()];
    if !e.host.is_empty() {
        v.push(format!("--host {}", q(&e.host)));
    }
    v.push("new".into());
    if !e.dir.is_empty() {
        v.push(format!("--dir {}", q(&e.dir)));
    }
    if !e.agent.is_empty() {
        v.push(format!("--agent {}", q(&e.agent)));
    }
    if e.worktree {
        v.push("--worktree".into());
    }
    if let Some(p) = e.prompt.as_deref().filter(|p| !p.is_empty()) {
        v.push(format!("-m {}", q(p)));
    }
    if !e.args.is_empty() {
        v.push("--".into());
        v.extend(e.args.iter().map(|a| q(a)));
    }
    v.join(" ")
}

pub fn file() -> PathBuf {
    crate::brand::env_os("HISTORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::world_state().join("history.jsonl"))
}

impl Entry {
    pub fn to_json(&self) -> Value {
        json!({
            "t": self.t, "host": self.host, "dir": self.dir, "name": self.name,
            "agent": self.agent, "args": self.args, "prompt": self.prompt, "worktree": self.worktree,
        })
    }
    fn from_json(v: &Value) -> Option<Entry> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Entry {
            t: v.get("t")?.as_u64()?,
            host: s("host"),
            dir: s("dir"),
            name: s("name"),
            agent: s("agent"),
            args: v
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            prompt: v.get("prompt").and_then(Value::as_str).map(String::from),
            worktree: v.get("worktree").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

pub fn append(e: &Entry) {
    let f = file();
    if let Some(d) = f.parent() {
        let _ = fs::create_dir_all(d);
    }
    if let Ok(mut fh) = fs::OpenOptions::new().create(true).append(true).open(&f) {
        let _ = writeln!(fh, "{}", e.to_json());
    }
    // trim occasionally (cheap check on size)
    if fs::metadata(&f)
        .map(|m| m.len() > 2_000_000)
        .unwrap_or(false)
    {
        let mut all = load_all();
        all.reverse(); // oldest first
        let keep = all.split_off(all.len().saturating_sub(KEEP));
        let mut s = String::new();
        for e in keep {
            s.push_str(&e.to_json().to_string());
            s.push('\n');
        }
        let _ = crate::config::write_atomic(&f, s.as_bytes());
    }
}

/// Every entry, newest first.
pub fn load_all() -> Vec<Entry> {
    let Ok(fh) = fs::File::open(file()) else {
        return vec![];
    };
    let mut v: Vec<Entry> = BufReader::new(fh)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .filter_map(|v| Entry::from_json(&v))
        .collect();
    v.reverse();
    v
}

pub fn load(limit: usize) -> Vec<Entry> {
    let mut v = load_all();
    v.truncate(limit);
    v
}

/// A launch's id (the Inbox's archive marks it by this).
pub fn id(e: &Entry) -> String {
    format!("launch|{}|{}|{}", e.t, e.host, e.name)
}

/// Forget launches for good (the Inbox's delete): the file is written back
/// without them, so "start again" no longer knows them. How many went.
pub fn remove(ids: &[String]) -> usize {
    let all = load_all();
    let n = all.iter().filter(|e| ids.contains(&id(e))).count();
    if n > 0 {
        let mut s = String::new();
        // load_all is newest first; the file is oldest first
        for e in all.iter().rev().filter(|e| !ids.contains(&id(e))) {
            s.push_str(&e.to_json().to_string());
            s.push('\n');
        }
        let _ = crate::config::write_atomic(&file(), s.as_bytes());
    }
    n
}

/// Names used in the last `days` days.
pub fn recent_names(days: u64) -> Vec<String> {
    let cutoff = crate::cache::now().saturating_sub(days * 86400);
    let mut out: Vec<String> = Vec::new();
    for e in load_all().into_iter().take_while(|e| e.t >= cutoff) {
        if !out.contains(&e.name) {
            out.push(e.name);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recent {
    pub host: String,
    pub dir: String,
    pub score: f64,
    pub count: usize,
    pub last: u64,
}

/// Folders by frecency (recent + frequent), optionally for one host.
pub fn recent_dirs(host: Option<&str>) -> Vec<Recent> {
    rank(&load_all(), host, crate::cache::now())
}

/// Pure (tested): frecency over `entries`.
pub fn rank(entries: &[Entry], host: Option<&str>, now: u64) -> Vec<Recent> {
    let mut out: Vec<Recent> = Vec::new();
    for e in entries {
        if e.dir.is_empty() || host.is_some_and(|h| h != e.host) {
            continue;
        }
        let age = now.saturating_sub(e.t);
        let w = match age {
            0..=3599 => 8.0,
            3600..=86399 => 4.0,
            86400..=604799 => 2.0,
            604800..=2591999 => 1.0,
            _ => 0.4,
        };
        match out.iter_mut().find(|r| r.host == e.host && r.dir == e.dir) {
            Some(r) => {
                r.score += w;
                r.count += 1;
                r.last = r.last.max(e.t);
            }
            None => out.push(Recent {
                host: e.host.clone(),
                dir: e.dir.clone(),
                score: w,
                count: 1,
                last: e.t,
            }),
        }
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.last.cmp(&a.last))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frecency_prefers_recent_and_frequent() {
        let now = 10_000_000;
        let e = |t: u64, d: &str| Entry {
            t,
            host: "b31".into(),
            dir: d.into(),
            name: "x".into(),
            ..Default::default()
        };
        let v = vec![
            e(now - 60, "/a"),
            e(now - 90 * 86400, "/old"),
            e(now - 90 * 86400, "/old"),
            e(now - 2 * 86400, "/b"),
            e(now - 3 * 86400, "/b"),
        ];
        let r = rank(&v, Some("b31"), now);
        assert_eq!(r[0].dir, "/a");
        assert_eq!(r[1].dir, "/b");
        assert_eq!(r[1].count, 2);
        assert!(rank(&v, Some("gpu2"), now).is_empty());
    }

    #[test]
    fn json_roundtrip() {
        let e = Entry {
            t: 5,
            host: "h".into(),
            dir: "/d".into(),
            name: "n".into(),
            agent: "codex".into(),
            args: vec!["--x".into()],
            prompt: Some("p".into()),
            worktree: true,
        };
        assert_eq!(Entry::from_json(&e.to_json()).unwrap(), e);
    }
}
