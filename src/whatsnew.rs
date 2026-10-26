//! What's new: `docs/whats-new.toml`, built in, is a short note in plain words
//! per deploy (newest first) with that deploy's commit subjects under it. A
//! showcase build adds `docs/whats-new-showcase.toml`'s notes to the same
//! entries. The dashboard shows the entries newer than the last one you saw,
//! once, after an update (`ui.whats_new`); `aleph whats-new` prints them.

use crate::cli::Args;
use crate::out::*;
use ratatui::prelude::*;
use serde_json::json;
use std::process::ExitCode;
use std::sync::OnceLock;
use toml_edit::{DocumentMut, Item};

const SRC: &str = include_str!("../docs/whats-new.toml");

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub id: String,
    pub date: String,
    pub title: String,
    pub notes: Vec<String>,
    pub commits: Vec<String>,
}

fn strings(t: &toml_edit::Table, k: &str) -> Vec<String> {
    t.get(k)
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// The entries in a file's text, in its order (newest first).
pub fn parse(src: &str) -> Vec<Entry> {
    let Ok(doc) = src.parse::<DocumentMut>() else {
        return Vec::new();
    };
    let Some(arr) = doc.get("entry").and_then(Item::as_array_of_tables) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|t| {
            let s = |k: &str| t.get(k).and_then(Item::as_str).unwrap_or("").to_string();
            let id = s("id");
            (!id.is_empty()).then(|| Entry {
                id,
                date: s("date"),
                title: s("title"),
                notes: strings(t, "notes"),
                commits: strings(t, "commits"),
            })
        })
        .collect()
}

/// `extra`'s notes and commits added to the entries with the same id.
pub fn merge(mut base: Vec<Entry>, extra: Vec<Entry>) -> Vec<Entry> {
    for x in extra {
        if let Some(e) = base.iter_mut().find(|e| e.id == x.id) {
            e.notes.extend(x.notes);
            e.commits.extend(x.commits);
        }
    }
    base
}

/// Every entry this build knows, newest first.
pub fn all() -> &'static [Entry] {
    static ALL: OnceLock<Vec<Entry>> = OnceLock::new();
    ALL.get_or_init(|| merge(parse(SRC), parse(crate::showcase::whats_new())))
}

fn seen_path() -> std::path::PathBuf {
    crate::config::state_dir().join("whats-new-seen")
}

/// The newest entry you've seen (None: never).
pub fn seen() -> Option<String> {
    std::fs::read_to_string(seen_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn mark_seen() {
    if let Some(e) = all().first() {
        let _ = crate::config::write_atomic(&seen_path(), e.id.as_bytes());
    }
}

/// The entries newer than `seen`: none seen yet (a first run) is only the
/// newest, and so is a `seen` this build doesn't list (it's from another one).
pub fn unseen<'a>(all: &'a [Entry], seen: Option<&str>) -> Vec<&'a Entry> {
    match seen {
        Some(s) => match all.iter().position(|e| e.id == s) {
            Some(i) => all[..i].iter().collect(),
            None => all.iter().take(1).collect(),
        },
        None => all.iter().take(1).collect(),
    }
}

/// What the dashboard shows after an update: the unseen entries, when the
/// card is on.
pub fn to_show(cfg: &crate::config::Config) -> Vec<Entry> {
    if cfg.get_or("ui.whats_new") == "off" {
        return Vec::new();
    }
    unseen(all(), seen().as_deref())
        .into_iter()
        .cloned()
        .collect()
}

pub fn cmd(a: Args) -> ExitCode {
    let n: usize = a
        .rest
        .iter()
        .position(|x| x == "-n")
        .and_then(|i| a.rest.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(if a.has("--all") { usize::MAX } else { 3 });
    let list: Vec<&Entry> = all().iter().take(n).collect();
    if a.has("--json") {
        json(&json!(list
            .iter()
            .map(|e| json!({"id": e.id, "date": e.date, "title": e.title, "notes": e.notes, "commits": e.commits}))
            .collect::<Vec<_>>()));
        return ExitCode::SUCCESS;
    }
    if list.is_empty() {
        ok("nothing new written down in this build");
        return ExitCode::SUCCESS;
    }
    let width = crossterm::terminal::size()
        .map(|(w, _)| w)
        .unwrap_or(100)
        .clamp(40, 110) as usize;
    let more = a.has("--commits");
    for (i, e) in list.iter().enumerate() {
        if i > 0 {
            out(Line::raw(""));
        }
        out(Line::from(vec![
            Span::styled("ℵ ", crate::ui::fg(crate::ui::c::ACCENT())),
            Span::styled(e.title.clone(), crate::ui::bold()),
            Span::styled(format!("  ·  {}", e.date), crate::ui::faint()),
        ]));
        for n in &e.notes {
            for (j, l) in crate::ui::wrap(n, width - 4).into_iter().enumerate() {
                out(Line::from(vec![
                    Span::styled(
                        if j == 0 { "  • " } else { "    " },
                        crate::ui::fg(crate::ui::c::ACCENT()),
                    ),
                    Span::raw(l),
                ]));
            }
        }
        if more {
            for c in &e.commits {
                out(Line::from(Span::styled(
                    format!("    · {}", crate::ui::trunc(c, width - 6)),
                    crate::ui::faint(),
                )));
            }
        }
    }
    if !more {
        hint("--commits lists each deploy's commits · --all every entry");
    }
    mark_seen();
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_placeholder_is_left_in_either_file() {
        // tools/whats-new.sh starts an entry with "…" for the notes to write
        let show = include_str!("../docs/whats-new-showcase.toml");
        for (file, text) in [("whats-new.toml", SRC), ("whats-new-showcase.toml", show)] {
            for e in parse(text) {
                for n in &e.notes {
                    assert!(
                        n.trim() != "…" && !n.trim().is_empty(),
                        "{file}: {}'s notes still say …",
                        e.id
                    );
                }
            }
        }
    }

    #[test]
    fn the_built_in_notes_parse() {
        let a = parse(SRC);
        assert!(!a.is_empty(), "docs/whats-new.toml has no entries");
        for e in &a {
            assert!(
                !e.title.is_empty() && !e.notes.is_empty(),
                "{}: a title and notes",
                e.id
            );
            assert!(
                !e.notes.iter().any(|n| n.contains('…') && n.len() < 6),
                "{}: notes left as …",
                e.id
            );
        }
        let mut ids: Vec<&str> = a.iter().map(|e| e.id.as_str()).collect();
        ids.dedup();
        assert_eq!(ids.len(), a.len(), "ids are unique");
    }

    #[test]
    fn only_what_you_havent_seen() {
        let e = |id: &str| Entry {
            id: id.into(),
            ..Default::default()
        };
        let all = vec![e("c"), e("b"), e("a")];
        let ids = |v: Vec<&Entry>| v.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(unseen(&all, Some("a"))), ["c", "b"]);
        assert_eq!(ids(unseen(&all, Some("c"))), Vec::<String>::new());
        // a first run, or a mark from another build: just the newest
        assert_eq!(ids(unseen(&all, None)), ["c"]);
        assert_eq!(ids(unseen(&all, Some("zz"))), ["c"]);
        // showcase notes join their entry
        let m = merge(
            vec![Entry {
                id: "c".into(),
                notes: vec!["x".into()],
                ..Default::default()
            }],
            vec![
                Entry {
                    id: "c".into(),
                    notes: vec!["y".into()],
                    ..Default::default()
                },
                e("nope"),
            ],
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].notes, ["x", "y"]);
    }
}
