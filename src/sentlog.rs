//! Prompts sent through Aleph, for the session journal (`journal.rs`, with
//! `journal.include` holding `prompts`). Aleph didn't keep them; this is a
//! small, pruned record:
//!
//! * `world_state()/sent.jsonl`, one `{t, host, pane, name, text}` a line;
//! * written where every prompt Aleph sends passes — `Herdr::prompt`,
//!   `Herdr::prompt_wait` and a new session's first prompt — so the TUI's
//!   send, broadcast and batch, `aleph send`, queue hand-outs, phone and web
//!   replies (the API), hand-offs and races are all in it;
//! * only while the journal includes prompts (the settings are looked at
//!   again at most once a minute);
//! * secrets masked first ([`scrub`]: `log::redact`'s tokens and secret
//!   URLs, API keys by their shape, `*_KEY=` / `*_TOKEN=` values, private
//!   key blocks, long opaque strings);
//! * kept [`KEEP`] (8 days) and at most [`CAP`] lines, pruned as it's written.
//!
//! Text typed straight into an agent in herdr never passes Aleph, so it isn't
//! here (the journal's header says so).

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long prompts are kept.
pub const KEEP: u64 = 8 * 86_400;
/// At most this many lines (the newest).
pub const CAP: usize = 2000;
/// The longest prompt kept (characters); the rest is cut.
const LONGEST: usize = 2000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sent {
    pub t: u64,
    pub host: String,
    pub pane: String,
    /// the session's name, when the sender knew it (else the journal finds it by pane)
    pub name: String,
    pub text: String,
}

impl Sent {
    fn to_json(&self) -> Value {
        json!({"t": self.t, "host": self.host, "pane": self.pane, "name": self.name, "text": self.text})
    }
    fn from_json(v: &Value) -> Option<Sent> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Sent {
            t: v.get("t").and_then(Value::as_u64)?,
            host: s("host"),
            pane: s("pane"),
            name: s("name"),
            text: s("text"),
        })
    }
}

pub fn file() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir()
            .join(format!("ch-sent-test-{}", std::process::id()))
            .join("sent.jsonl");
    }
    crate::config::world_state().join("sent.jsonl")
}

/// Does the journal include prompts (the settings, looked at once a minute)?
fn wanted() -> bool {
    if cfg!(test) {
        return false;
    }
    static AT: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut g = match AT.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if let Some((at, on)) = *g {
        if at.elapsed() < Duration::from_secs(60) {
            return on;
        }
    }
    let cfg = crate::config::Config::load();
    let on = crate::journal::Include::parse(&cfg.get_or("journal.include")).prompts;
    *g = Some((Instant::now(), on));
    on
}

/// A prompt Aleph just sent to `pane` on `host` (`name`: its session, when known).
pub fn record(host: &str, pane: &str, name: &str, text: &str) {
    record_at(crate::cache::now(), host, pane, name, text);
}

/// [`record`], sent at `t`.
pub fn record_at(t: u64, host: &str, pane: &str, name: &str, text: &str) {
    if text.trim().is_empty() || !wanted() {
        return;
    }
    let s = Sent {
        t,
        host: host.to_string(),
        pane: pane.to_string(),
        name: name.to_string(),
        text: cut(&scrub(text.trim()), LONGEST),
    };
    append_to(&file(), &s, crate::cache::now());
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut v: String = s.chars().take(n).collect();
    v.push('…');
    v
}

/// Append one line; past [`KEEP`] or [`CAP`], the file is rewritten
/// (atomically) without what's too old or too many.
fn append_to(path: &Path, s: &Sent, now: u64) {
    let line = s.to_json().to_string() + "\n";
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let oldest = existing
        .lines()
        .next()
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .and_then(|v| v.get("t").and_then(Value::as_u64));
    let lines = existing.lines().count();
    if lines >= CAP || oldest.is_some_and(|t| t + KEEP < now) {
        let mut keep: Vec<&str> = existing
            .lines()
            .filter(|l| {
                serde_json::from_str::<Value>(l)
                    .ok()
                    .and_then(|v| v.get("t").and_then(Value::as_u64))
                    .is_some_and(|t| t + KEEP >= now)
            })
            .collect();
        if keep.len() >= CAP {
            keep.drain(..keep.len() + 1 - CAP);
        }
        let mut body: String = keep.iter().map(|l| format!("{l}\n")).collect();
        body.push_str(&line);
        let _ = crate::config::write_atomic(path, body.as_bytes());
        return;
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Prompts sent in `[from, to)`, oldest first.
pub fn between(from: u64, to: u64) -> Vec<Sent> {
    read_from(&file(), from, to)
}

fn read_from(path: &Path, from: u64, to: u64) -> Vec<Sent> {
    let mut v: Vec<Sent> = std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| Sent::from_json(&v))
        .filter(|s| s.t >= from && s.t < to)
        .collect();
    v.sort_by_key(|s| s.t);
    v
}

// ------------------------------------------------------------------ secrets

/// Prefixes API keys and tokens start with (OpenAI / Anthropic, GitHub,
/// Slack, GitLab, Hugging Face, npm, PyPI, Stripe, Google).
const KEY_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "rk_live_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xapp-",
    "glpat-",
    "hf_",
    "npm_",
    "pypi-",
    "AIza",
];

/// Variable names whose value is a secret (`OPENAI_API_KEY=…`).
const SECRET_NAMES: &[&str] = &[
    "KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PASS",
    "AUTH",
    "CREDENTIAL",
];

/// What must never be written down, masked (pure — tested): `log::redact`
/// first (secret URLs, bearer tokens, `token=` / `password=`), then whole
/// private key blocks, then word by word: API keys by their shape, values of
/// variables named like secrets, JWTs, and long opaque strings that mix
/// upper case, lower case and digits (hex alone — a commit, a hash — stays).
pub fn scrub(s: &str) -> String {
    let s = private_keys(s);
    let s = crate::log::redact(&s);
    let mut out = String::with_capacity(s.len());
    for word in s.split_inclusive(char::is_whitespace) {
        let (w, ws) = match word.char_indices().last() {
            Some((i, c)) if c.is_whitespace() => (&word[..i], &word[i..]),
            _ => (word, ""),
        };
        out.push_str(&scrub_word(w));
        out.push_str(ws);
    }
    out
}

fn private_keys(s: &str) -> String {
    let mut s = s.to_string();
    while let Some(a) = s.find("-----BEGIN ") {
        let Some(k) = s[a..].find("PRIVATE KEY-----") else {
            break;
        };
        let end = s[a + k..]
            .find("-----END ")
            .and_then(|e| {
                let from = a + k + e;
                s[from..]
                    .find("PRIVATE KEY-----")
                    .map(|x| from + x + "PRIVATE KEY-----".len())
            })
            .unwrap_or(s.len());
        s.replace_range(a..end, "[private key]");
    }
    s
}

fn scrub_word(w: &str) -> String {
    // `NAME=value`, `--token=value`, `"key": "value"` keep their name
    if let Some(i) = w.find(['=', ':']) {
        let (name, value) = (&w[..i], &w[i + 1..]);
        let upper = name
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
            .to_uppercase();
        let named = !upper.is_empty() && upper.split(['_', '-']).any(|p| SECRET_NAMES.contains(&p));
        let v = value.trim_matches(|c: char| c == '"' || c == '\'' || c == ',');
        if named && v.len() >= 6 && !v.starts_with("//") && v != "…" {
            return format!("{}{}…", name, &w[i..=i]);
        }
        if !value.is_empty() && opaque(v) {
            return format!("{}{}…", name, &w[i..=i]);
        }
        return w.to_string();
    }
    let core = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
    if opaque(core) {
        return w.replace(core, "…");
    }
    w.to_string()
}

/// A token by its looks: a known key prefix with a long tail, a JWT, or a
/// long run of letters and digits in both cases.
fn opaque(w: &str) -> bool {
    let tail_ok = |rest: &str| {
        rest.len() >= 16
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    if KEY_PREFIXES
        .iter()
        .any(|p| w.strip_prefix(p).is_some_and(tail_ok))
    {
        return true;
    }
    // AWS access key ids
    if w.len() == 20
        && (w.starts_with("AKIA") || w.starts_with("ASIA"))
        && w.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return true;
    }
    // a JWT: three base64url parts, the first `eyJ`
    if w.starts_with("eyJ") && w.split('.').count() == 3 && w.len() >= 30 {
        return true;
    }
    w.len() >= 32
        && w.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '_' || c == '-' || c == '='
        })
        && w.chars().any(|c| c.is_ascii_uppercase())
        && w.chars().any(|c| c.is_ascii_lowercase())
        && w.chars().any(|c| c.is_ascii_digit())
        // a path or a long word-like name isn't one
        && !w.contains('/')
        && w.chars().filter(|c| c.is_ascii_digit()).count() >= 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_written_down() {
        let s = scrub("use sk-ant-api03-AbCdEf1234567890abcdefXYZ to call it");
        assert_eq!(s, "use … to call it");
        assert_eq!(
            scrub("export OPENAI_API_KEY=abc123def456 && run"),
            "export OPENAI_API_KEY=… && run"
        );
        assert_eq!(
            scrub("push with ghp_aaaaBBBBccccDDDDeeeeFFFF0000"),
            "push with …"
        );
        assert_eq!(scrub("AWS AKIAIOSFODNN7EXAMPLE key"), "AWS … key");
        assert!(scrub("token: eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcDEF123ghi").contains('…'));
        let pem = "here:\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\n-----END OPENSSH PRIVATE KEY-----\nthanks";
        assert_eq!(scrub(pem), "here:\n[private key]\nthanks");
        assert!(!scrub("notify https://ntfy.sh/my-secret-topic").contains("my-secret-topic"));
        assert_eq!(scrub("password=hunter2hunter2 please"), "password=… please");
        // what isn't a secret stays: words, paths, commits, numbers
        for ok in [
            "Fix the flaky test in tests/net.rs",
            "revert 3f2a9c1e5b7d4a6f8e0c2b4d6f8a0c2e4b6d8f0a",
            "run cargo test --release -j 8",
            "see /home/u/work/AlephSessionJournalWriterV2Final/src",
            "the key: idempotency matters",
        ] {
            assert_eq!(scrub(ok), ok);
        }
    }

    #[test]
    fn kept_a_while_then_pruned() {
        let p = std::env::temp_dir()
            .join(format!("ch-sent-prune-{}", std::process::id()))
            .join("sent.jsonl");
        let _ = std::fs::remove_file(&p);
        let s = |t: u64, text: &str| Sent {
            t,
            host: "b31".into(),
            pane: "w1:p2".into(),
            name: "ignite".into(),
            text: text.into(),
        };
        let now = 100 * 86_400;
        append_to(&p, &s(now - 10 * 86_400, "old"), now - 10 * 86_400);
        append_to(&p, &s(now - 9 * 86_400, "older"), now - 9 * 86_400);
        assert_eq!(read_from(&p, 0, u64::MAX).len(), 2);
        // the next one prunes what's past KEEP
        append_to(&p, &s(now - 60, "fix it"), now - 60);
        append_to(&p, &s(now, "and test it"), now);
        let v = read_from(&p, 0, u64::MAX);
        assert_eq!(
            v.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(),
            vec!["fix it", "and test it"]
        );
        // a window: [from, to)
        assert_eq!(read_from(&p, now - 60, now).len(), 1);
        let _ = std::fs::remove_file(&p);
    }
}
