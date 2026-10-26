//! Notes and tags on sessions: `aleph note <session> "text #tag"`, or `N` in
//! Fleet, Focus and Inbox. Kept in `state/notes.json` by machine and session
//! name, so they outlive the session: Inbox shows them on its rows and
//! searches them, Fleet and Focus show them next to the session.

use crate::config::write_atomic;
use serde_json::{json, Map, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Note {
    pub text: String,
    /// `#tags` in the text, without the `#`
    pub tags: Vec<String>,
    pub t: u64,
}

pub fn file() -> std::path::PathBuf {
    crate::config::world_state().join("notes.json")
}

fn key(host: &str, name: &str) -> String {
    format!("{host}|{name}")
}

/// `#tags` in a note, in order, once each.
pub fn tags_of(text: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for w in text.split_whitespace() {
        if let Some(t) = w.strip_prefix('#') {
            let t: String = t
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
                .collect::<String>()
                .trim_end_matches('.')
                .to_lowercase();
            if !t.is_empty() && !v.contains(&t) {
                v.push(t);
            }
        }
    }
    v
}

/// (checked at, the file's mtime then, its notes)
type Cache = Option<(Instant, Option<SystemTime>, Map<String, Value>)>;
static CACHE: Mutex<Cache> = Mutex::new(None);

/// Look at every note (the file is checked for changes at most once a second;
/// searches call this per row).
fn with<R>(f: impl FnOnce(&Map<String, Value>) -> R) -> R {
    let mut g = CACHE.lock().unwrap();
    let fresh = g
        .as_ref()
        .is_some_and(|(at, _, _)| at.elapsed() < Duration::from_secs(1));
    if !fresh {
        let p = file();
        let m = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        match g.as_mut() {
            Some((at, t, _)) if *t == m => *at = Instant::now(),
            _ => {
                let v: Map<String, Value> = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                *g = Some((Instant::now(), m, v));
            }
        }
    }
    f(g.as_ref().map(|x| &x.2).unwrap_or(&Map::new()))
}

fn load() -> Map<String, Value> {
    with(|m| m.clone())
}

fn parse(v: &Value) -> Option<Note> {
    let text = v.get("text")?.as_str()?.to_string();
    Some(Note {
        tags: tags_of(&text),
        text,
        t: v.get("t").and_then(Value::as_u64).unwrap_or(0),
    })
}

pub fn get(host: &str, name: &str) -> Option<Note> {
    if name.is_empty() {
        return None;
    }
    with(|m| m.get(&key(host, name)).and_then(parse))
}

/// Set (or, with empty text, remove) a session's note.
pub fn set(host: &str, name: &str, text: &str) -> std::io::Result<()> {
    let mut m = load();
    let k = key(host, name);
    let text = text.trim();
    if text.is_empty() {
        m.remove(&k);
    } else {
        m.insert(k, json!({"text": text, "t": crate::cache::now()}));
    }
    write_atomic(&file(), Value::Object(m).to_string().as_bytes())?;
    *CACHE.lock().unwrap() = None;
    Ok(())
}

/// What a search should see of a session's note: its text (tags included).
pub fn search_text(host: &str, name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    with(|m| {
        m.get(&key(host, name))
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    })
}

/// Every note, newest first: (host, name, note).
pub fn all() -> Vec<(String, String, Note)> {
    let mut v: Vec<(String, String, Note)> = load()
        .iter()
        .filter_map(|(k, x)| {
            let (h, n) = k.split_once('|')?;
            Some((h.to_string(), n.to_string(), parse(x)?))
        })
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.2.t));
    v
}

/// `aleph note [<session> [text… | --clear]]`
pub fn cmd(a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::*;
    let p = a.positional();
    let Some(q) = p.first() else {
        let v = all();
        if a.has("--json") {
            json(&Value::Array(
                v.iter()
                    .map(|(h, n, x)| json!({"host": h, "name": n, "text": x.text, "tags": x.tags, "t": x.t}))
                    .collect(),
            ));
            return std::process::ExitCode::SUCCESS;
        }
        if v.is_empty() {
            ok("no notes yet");
            hint("aleph note <session> \"waiting on the data refresh #blocked\"");
        }
        for (h, n, x) in v {
            println!("{h}:{n}\t{}", x.text);
        }
        return std::process::ExitCode::SUCCESS;
    };
    // a running session (host:name or name), or host:name of one that ended
    let (host, name) = match crate::cli::find_quiet(&a.cfg, &a.flag, q) {
        Some((_, s)) => (s.host.clone(), s.name.clone()),
        None => match q.split_once(':') {
            Some((h, n)) => (h.to_string(), n.to_string()),
            None => {
                return fail(&format!(
                    "no session '{q}' (use host:name for one that ended)"
                ))
            }
        },
    };
    if p.len() == 1 && !a.has("--clear") {
        return match get(&host, &name) {
            Some(n) => {
                println!("{}", n.text);
                std::process::ExitCode::SUCCESS
            }
            None => fail(&format!("no note on {host}:{name}")),
        };
    }
    let text = if a.has("--clear") {
        String::new()
    } else {
        p[1..].join(" ")
    };
    match set(&host, &name, &text) {
        Ok(()) if text.is_empty() => {
            ok(&format!("cleared the note on {host}:{name}"));
            std::process::ExitCode::SUCCESS
        }
        Ok(()) => {
            ok(&format!("noted on {host}:{name}"));
            std::process::ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("couldn't save: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_from_text() {
        assert_eq!(
            tags_of("waiting on #data-refresh, then #ship. #Ship again"),
            vec!["data-refresh", "ship"]
        );
        assert!(tags_of("no tags # here").is_empty());
    }

    #[test]
    fn notes_round_trip() {
        let host = format!("t{}", std::process::id());
        set(&host, "ignite", "check the #lut fit").unwrap();
        let n = get(&host, "ignite").unwrap();
        assert_eq!(
            (n.text.as_str(), n.tags.clone()),
            ("check the #lut fit", vec!["lut".to_string()])
        );
        assert!(search_text(&host, "ignite").contains("lut"));
        set(&host, "ignite", "").unwrap();
        assert!(get(&host, "ignite").is_none());
    }
}
