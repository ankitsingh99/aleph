//! `aleph backup [FILE] [--all] [--json] | list | show FILE | restore FILE [--dry-run] [--yes]`
//!
//! What Aleph keeps about you, in one file to keep or carry to another
//! computer: the settings (`aleph.toml`, its profiles and the other `.toml`
//! files beside it; presets and workspaces are settings), the launch history
//! behind frecency, notes, session memory (what `aleph restore` brings back),
//! the queue and the setup wizard's answers. `--all` adds the Inbox and the
//! usage records.
//!
//! Never a secret: a secret setting's value is left out (a `secret:NAME`
//! reference stays, the Keychain keeps the value), and `secrets.toml`, device
//! tokens, push keys, logs, sockets and locks aren't read at all.
//!
//! A restore shows what changes and asks. It first saves what it's about to
//! replace as a backup of its own, and keeps this computer's secrets.

use crate::cli::Args;
use crate::config::{self, Config};
use crate::configcmd::is_secret;
use crate::out::*;
use crate::tomlcfg;
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use toml_edit::DocumentMut;

/// What a backup file says it is.
const FORMAT: u64 = 1;
/// The state files every backup carries (in `state_dir()`).
const STATE: &[&str] = &[
    "history.jsonl",
    "notes.json",
    "sessions.json",
    "queue.json",
    "setup.json",
];
/// And with `--all`: the Inbox.
const STATE_ALL: &[&str] = &[
    "events.jsonl",
    "inbox_dismissed",
    "inbox_marks",
    "inbox_seen",
];

/// `--all`: the usage store's own files (its rollup is rebuilt from them).
fn usage_file(name: &str) -> bool {
    ((name.starts_with("records-") || name.starts_with("steps-")) && name.ends_with(".tsv"))
        || matches!(
            name,
            "offsets.json" | "limits.json" | "accounts.json" | "projects.json" | "gens.json"
        )
}

const USAGE: &str = "usage: aleph backup [FILE] [--all] [--json] | list | show FILE | restore FILE [--dry-run] [--yes]";

/// One backup: when and where it was made, its files by their place
/// (`config/aleph.toml`, `state/notes.json`, `state/usage/…`), and the secret
/// settings each file had that it doesn't carry.
#[derive(Debug, Default, PartialEq)]
pub struct Bundle {
    pub made: u64,
    pub computer: String,
    pub version: String,
    pub files: BTreeMap<String, String>,
    pub left_out: BTreeMap<String, Vec<String>>,
}

impl Bundle {
    pub fn to_json(&self) -> Value {
        json!({
            "aleph_backup": FORMAT,
            "made": self.made,
            "computer": self.computer,
            "version": self.version,
            "left_out": self.left_out,
            "files": self.files,
        })
    }

    pub fn from_json(v: &Value) -> Result<Bundle, String> {
        match v.get("aleph_backup").and_then(Value::as_u64) {
            Some(FORMAT) => {}
            Some(n) => {
                return Err(format!(
                    "made by a newer aleph (format {n}): update this one first"
                ))
            }
            None => return Err("not an aleph backup".into()),
        }
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let mut b = Bundle {
            made: v.get("made").and_then(Value::as_u64).unwrap_or(0),
            computer: s("computer"),
            version: s("version"),
            ..Default::default()
        };
        if let Some(files) = v.get("files").and_then(Value::as_object) {
            for (k, t) in files {
                if let Some(t) = t.as_str() {
                    b.files.insert(k.clone(), t.to_string());
                }
            }
        }
        if let Some(lo) = v.get("left_out").and_then(Value::as_object) {
            for (k, keys) in lo {
                let keys = keys.as_array().into_iter().flatten();
                b.left_out.insert(
                    k.clone(),
                    keys.filter_map(|x| x.as_str().map(String::from)).collect(),
                );
            }
        }
        Ok(b)
    }

    pub fn bytes(&self) -> usize {
        self.files.values().map(String::len).sum()
    }
}

/// `text` (a settings file) without its secret values: the cleaned text and
/// the keys taken out. An error when it isn't TOML, or a secret couldn't be
/// taken out (an inline table): such a file isn't carried at all.
pub fn without_secrets(text: &str) -> Result<(String, Vec<String>), String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("not TOML ({e})"))?;
    let secret =
        |(k, v): &(String, String)| is_secret(k) && !v.is_empty() && !v.starts_with("secret:");
    let mut out = Vec::new();
    for (k, _) in tomlcfg::entries(&doc).iter().filter(|e| secret(e)) {
        tomlcfg::unset(&mut doc, k);
        out.push(k.clone());
    }
    if let Some((k, _)) = tomlcfg::entries(&doc).iter().find(|e| secret(e)) {
        return Err(format!(
            "{k} is a secret written where it can't be left out"
        ));
    }
    Ok((doc.to_string(), out))
}

/// `theirs` (a settings file from a backup) with this computer's own secret
/// values from `ours` put back wherever `theirs` doesn't set one.
pub fn with_our_secrets(theirs: &str, ours: Option<&str>) -> String {
    let (Ok(mut doc), Some(Ok(o))) = (
        theirs.parse::<DocumentMut>(),
        ours.map(str::parse::<DocumentMut>),
    ) else {
        return theirs.to_string();
    };
    let have = tomlcfg::entries(&doc);
    for (k, v) in tomlcfg::entries(&o) {
        if is_secret(&k) && !v.is_empty() && !have.iter().any(|(x, _)| *x == k) {
            tomlcfg::set(&mut doc, &k, &v);
        }
    }
    doc.to_string()
}

/// The `.toml` files in the config folder and one folder down (profiles/).
fn tomls(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return v;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&p) {
                v.extend(
                    sub.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "toml")),
                );
            }
        } else if p.extension().is_some_and(|x| x == "toml") {
            v.push(p);
        }
    }
    v.sort();
    v
}

/// Everything a backup carries, read from this computer now.
pub fn collect(all: bool) -> (Bundle, Vec<String>) {
    let mut b = Bundle {
        made: crate::cache::now(),
        computer: crate::auto::this_machine(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ..Default::default()
    };
    let mut skipped = Vec::new();
    let cdir = Config::dir();
    for p in tomls(&cdir) {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(rel) = p.strip_prefix(&cdir) else {
            continue;
        };
        let rel = format!("config/{}", rel.to_string_lossy());
        match without_secrets(&text) {
            Ok((clean, gone)) => {
                if !gone.is_empty() {
                    b.left_out.insert(rel.clone(), gone);
                }
                b.files.insert(rel, clean);
            }
            Err(e) => skipped.push(format!("{rel}: {e}")),
        }
    }
    let sdir = config::state_dir();
    let extra: &[&str] = if all { STATE_ALL } else { &[] };
    for n in STATE.iter().chain(extra) {
        if let Ok(t) = std::fs::read_to_string(sdir.join(n)) {
            b.files.insert(format!("state/{n}"), t);
        }
    }
    if all {
        if let Ok(rd) = std::fs::read_dir(sdir.join("usage")) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if usage_file(&name) {
                    if let Ok(t) = std::fs::read_to_string(e.path()) {
                        b.files.insert(format!("state/usage/{name}"), t);
                    }
                }
            }
        }
    }
    (b, skipped)
}

/// Where a backup's file goes on this computer: only the places a backup
/// writes (never outside the config and state folders, never a secret,
/// socket or log), else None.
pub fn target(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        ["config", f] | ["config", _, f] if f.ends_with(".toml") && *f != "secrets.toml" => {
            Some(Config::dir().join(&rel["config/".len()..]))
        }
        ["state", f] if STATE.contains(f) || STATE_ALL.contains(f) => {
            Some(config::state_dir().join(f))
        }
        ["state", "usage", f] if usage_file(f) => Some(config::state_dir().join("usage").join(f)),
        _ => None,
    }
}

/// `$XDG_DATA_HOME/aleph/backups`, where backups go unless you name a file.
pub fn dir() -> PathBuf {
    config::data_dir().join("backups")
}

/// `20261024-1105`, local time.
fn stamp(t: u64) -> String {
    let s = t as i64 + crate::ui::utc_offset();
    let (y, m, d) = crate::ui::civil(s.div_euclid(86_400));
    let r = s.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}", r / 3600, r % 3600 / 60)
}

fn write(path: &Path, b: &Bundle) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let text = serde_json::to_string_pretty(&b.to_json()).unwrap_or_default();
    config::write_private(path, text.as_bytes())
}

fn read(file: &str) -> Result<Bundle, String> {
    let p = config::expand(file);
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{file}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{file}: not JSON ({e})"))?;
    Bundle::from_json(&v).map_err(|e| format!("{file}: {e}"))
}

pub fn cmd(a: Args) -> ExitCode {
    let pos = a.positional();
    let json_out = a.has("--json");
    match pos.first().copied() {
        Some("list" | "ls") => list(json_out),
        Some("show") => match pos.get(1) {
            Some(f) => show(f, json_out),
            None => fail("usage: aleph backup show FILE"),
        },
        Some("restore") => match pos.get(1) {
            Some(f) => restore(f, a.has("--dry-run"), a.has("--yes")),
            None => fail("usage: aleph backup restore FILE [--dry-run] [--yes]"),
        },
        Some("help") => {
            raw(USAGE);
            ExitCode::SUCCESS
        }
        file => make(file, a.has("--all"), json_out),
    }
}

fn make(file: Option<&str>, all: bool, json_out: bool) -> ExitCode {
    let (b, skipped) = collect(all);
    let path = match file {
        Some(f) => config::expand(f),
        None => dir().join(format!("aleph-{}-{}.json", b.computer, stamp(b.made))),
    };
    if let Err(e) = write(&path, &b) {
        return fail(&format!("couldn't write {}: {e}", path.display()));
    }
    let left: Vec<&String> = b.left_out.values().flatten().collect();
    if json_out {
        json(&json!({
            "path": path,
            "files": b.files.keys().collect::<Vec<_>>(),
            "bytes": b.bytes(),
            "left_out": left,
            "skipped": skipped,
        }));
        return ExitCode::SUCCESS;
    }
    ok(&format!(
        "backed up {} files ({}) → {}",
        b.files.len(),
        crate::ui::bytes(b.bytes() as f64),
        crate::doctor::tilde(&path)
    ));
    if !left.is_empty() {
        let names: Vec<&str> = left.iter().map(|s| s.as_str()).collect();
        hint(&format!(
            "{} secret{} left out ({}): set them again on another computer, or keep them as secret:NAME",
            left.len(),
            if left.len() == 1 { "" } else { "s" },
            names.join(", ")
        ));
    }
    for s in &skipped {
        warn(&format!("not carried: {s}"));
    }
    if !all {
        hint("--all also carries the Inbox and the usage records");
    }
    ExitCode::SUCCESS
}

fn list(json_out: bool) -> ExitCode {
    let mut v: Vec<(PathBuf, u64, u64)> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let t = m
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs();
            Some((e.path(), m.len(), t))
        })
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.2));
    if json_out {
        json(
            &json!({"dir": dir(), "backups": v.iter().map(|(p, n, t)| json!({"path": p, "bytes": n, "at": t})).collect::<Vec<_>>()}),
        );
        return ExitCode::SUCCESS;
    }
    if v.is_empty() {
        ok(&format!(
            "no backups yet in {}",
            crate::doctor::tilde(&dir())
        ));
        hint("aleph backup makes one");
        return ExitCode::SUCCESS;
    }
    head(&format!("backups  ·  {}", crate::doctor::tilde(&dir())));
    let now = crate::cache::now();
    for (p, n, t) in &v {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        out(Line::from(vec![
            Span::raw(format!("  {}  ", crate::ui::pad(&name, 40))),
            Span::styled(
                format!("{:>8}  ", crate::ui::bytes(*n as f64)),
                crate::ui::muted(),
            ),
            Span::styled(crate::ui::ago(now.saturating_sub(*t)), crate::ui::faint()),
        ]));
    }
    ExitCode::SUCCESS
}

fn show(file: &str, json_out: bool) -> ExitCode {
    let b = match read(file) {
        Ok(b) => b,
        Err(e) => return fail(&e),
    };
    if json_out {
        let mut v = b.to_json();
        // what's in it, not the contents
        v["files"] = json!(b
            .files
            .iter()
            .map(|(k, t)| json!({"file": k, "bytes": t.len()}))
            .collect::<Vec<_>>());
        json(&v);
        return ExitCode::SUCCESS;
    }
    head(&format!(
        "backup from {} · aleph {} · {}",
        b.computer,
        b.version,
        crate::ui::ago(crate::cache::now().saturating_sub(b.made))
    ));
    for (k, t) in &b.files {
        let place = if target(k).is_some() {
            ""
        } else {
            "  (not restored: not a place backups write)"
        };
        out(Line::from(vec![
            Span::raw(format!("  {}  ", crate::ui::pad(k, 34))),
            Span::styled(
                format!("{:>8}", crate::ui::bytes(t.len() as f64)),
                crate::ui::muted(),
            ),
            Span::styled(place.to_string(), crate::ui::faint()),
        ]));
    }
    for (k, keys) in &b.left_out {
        hint(&format!("{k}: left out {}", keys.join(", ")));
    }
    ExitCode::SUCCESS
}

/// What restoring one file does.
#[derive(Debug, PartialEq)]
pub enum Change {
    New,
    Same,
    /// lines that are only in one of them
    Differs(usize),
}

pub fn change(ours: Option<&str>, theirs: &str) -> Change {
    match ours {
        None => Change::New,
        Some(o) if o == theirs => Change::Same,
        Some(o) => {
            let mut count: BTreeMap<&str, i64> = BTreeMap::new();
            for l in o.lines() {
                *count.entry(l).or_default() += 1;
            }
            for l in theirs.lines() {
                *count.entry(l).or_default() -= 1;
            }
            Change::Differs(
                count
                    .values()
                    .map(|n| n.unsigned_abs() as usize)
                    .sum::<usize>()
                    .max(1),
            )
        }
    }
}

/// The settings a restored settings file changes: `+ k = v`, `~ k: a → b`,
/// `− k` (secrets as …).
fn settings_lines(ours: Option<&str>, theirs: &str) -> Vec<Line<'static>> {
    let parse = |t: &str| {
        t.parse::<DocumentMut>()
            .map(|d| tomlcfg::entries(&d))
            .unwrap_or_default()
    };
    let (o, t) = (ours.map(parse).unwrap_or_default(), parse(theirs));
    let shown = |k: &str, v: &str| {
        if is_secret(k) {
            "…".to_string()
        } else {
            v.to_string()
        }
    };
    let mut v = Vec::new();
    for (k, old, new) in crate::configcmd::changes(&o, &t) {
        let (mark, col, text) = match old {
            Some(old) => (
                "~",
                crate::ui::c::YELLOW(),
                format!("{k}: {} → {}", shown(&k, &old), shown(&k, &new)),
            ),
            None => (
                "+",
                crate::ui::c::GREEN(),
                format!("{k} = {}", shown(&k, &new)),
            ),
        };
        v.push(Line::from(vec![
            Span::styled(format!("      {mark} "), crate::ui::fg(col)),
            Span::raw(text),
        ]));
    }
    for (k, _) in o.iter().filter(|(k, _)| !t.iter().any(|(x, _)| x == k)) {
        if is_secret(k) {
            continue; // kept: a restore puts this computer's secrets back
        }
        v.push(Line::from(vec![
            Span::styled("      − ", crate::ui::fg(crate::ui::c::RED())),
            Span::raw(k.clone()),
        ]));
    }
    v
}

fn restore(file: &str, dry: bool, yes_all: bool) -> ExitCode {
    let b = match read(file) {
        Ok(b) => b,
        Err(e) => return fail(&e),
    };
    head(&format!(
        "restore the backup from {} (aleph {}, {})",
        b.computer,
        b.version,
        crate::ui::ago(crate::cache::now().saturating_sub(b.made))
    ));
    // what goes where, as it will be written
    let mut plan: Vec<(String, PathBuf, String)> = Vec::new();
    let mut refused = Vec::new();
    for (rel, text) in &b.files {
        let Some(path) = target(rel) else {
            refused.push(rel.clone());
            continue;
        };
        let ours = std::fs::read_to_string(&path).ok();
        let text = if rel.starts_with("config/") {
            with_our_secrets(text, ours.as_deref())
        } else {
            text.clone()
        };
        plan.push((rel.clone(), path, text));
    }
    let mut todo = 0;
    for (rel, path, text) in &plan {
        let ours = std::fs::read_to_string(path).ok();
        let (mark, col, what) = match change(ours.as_deref(), text) {
            Change::Same => ("=", crate::ui::c::FAINT(), "the same".to_string()),
            Change::New => ("+", crate::ui::c::GREEN(), "new here".to_string()),
            Change::Differs(n) => (
                "~",
                crate::ui::c::YELLOW(),
                format!("{n} line{} differ", if n == 1 { "" } else { "s" }),
            ),
        };
        if mark != "=" {
            todo += 1;
        }
        say(Line::from(vec![
            Span::styled(format!("  {mark} "), crate::ui::fg(col)),
            Span::raw(format!("{}  ", crate::ui::pad(rel, 30))),
            Span::styled(what, crate::ui::muted()),
        ]));
        if mark != "=" && rel.starts_with("config/") {
            for l in settings_lines(ours.as_deref(), text) {
                say(l);
            }
        }
    }
    for r in &refused {
        warn(&format!("{r}: not restored (not a place backups write)"));
    }
    if todo == 0 {
        ok("nothing to change: this computer already has all of it");
        return ExitCode::SUCCESS;
    }
    if dry {
        hint("--dry-run: nothing written");
        return ExitCode::SUCCESS;
    }
    if !yes_all
        && !yes(&format!(
            "restore {todo} file{}?",
            if todo == 1 { "" } else { "s" }
        ))
    {
        hint("nothing written");
        return ExitCode::from(1);
    }
    // what it replaces, first, so a restore can be taken back
    let touches_all = plan.iter().any(|(r, _, _)| {
        r.starts_with("state/usage/") || STATE_ALL.iter().any(|f| r == &format!("state/{f}"))
    });
    let (before, _) = collect(touches_all);
    let undo = dir().join(format!("before-restore-{}.json", stamp(before.made)));
    if let Err(e) = write(&undo, &before) {
        return fail(&format!(
            "couldn't save what's here first ({}): nothing written",
            e
        ));
    }
    let mut failed = 0;
    for (rel, path, text) in &plan {
        if std::fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let r = if rel.starts_with("config/") {
            config::write_atomic(path, text.as_bytes())
        } else {
            config::write_private(path, text.as_bytes())
        };
        if let Err(e) = r {
            failed += 1;
            err(&format!("{rel}: {e}"));
        }
    }
    if failed > 0 {
        warn(&format!(
            "{failed} file{} not written",
            if failed == 1 { "" } else { "s" }
        ));
    } else {
        ok(&format!(
            "restored {todo} file{}",
            if todo == 1 { "" } else { "s" }
        ));
    }
    hint(&format!(
        "what was here before: aleph backup restore {}",
        crate::doctor::tilde(&undo)
    ));
    hint("a running daemon picks it up after aleph daemon restart");
    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_left_out_and_put_back() {
        let text = "[notify]\nntfy = \"https://ntfy.sh/s3cret\"\nslack = \"secret:SLACK\"\n\n[ui]\nsplash = \"cockpit\"\n";
        let (clean, gone) = without_secrets(text).unwrap();
        assert_eq!(gone, vec!["notify.ntfy".to_string()]);
        assert!(!clean.contains("s3cret"), "{clean}");
        // a reference is safe to carry; other settings stay as written
        assert!(
            clean.contains("secret:SLACK") && clean.contains("cockpit"),
            "{clean}"
        );
        // restored here: this computer's own value comes back
        let ours = "[notify]\nntfy = \"https://ntfy.sh/mine\"\n";
        let back = with_our_secrets(&clean, Some(ours));
        assert!(
            back.contains("ntfy.sh/mine") && back.contains("cockpit"),
            "{back}"
        );
        // one the backup sets itself isn't overwritten
        assert!(
            with_our_secrets(&clean, Some("[notify]\nslack = \"x\"\n")).contains("secret:SLACK")
        );
        // a secret where it can't be taken out: the file isn't carried
        assert!(without_secrets("notify = { ntfy = \"https://ntfy.sh/s3cret\" }\n").is_err());
        assert!(without_secrets("not = [toml").is_err());
    }

    #[test]
    fn a_backup_writes_only_its_own_places() {
        assert!(target("config/aleph.toml").is_some());
        assert!(target("config/profiles/demo.toml").is_some());
        assert!(target("state/notes.json").is_some());
        assert!(target("state/usage/records-2026-10.tsv").is_some());
        for bad in [
            "config/secrets.toml",
            "config/../../.ssh/config",
            "/etc/passwd",
            "state/secrets.toml",
            "state/daemon.sock",
            "state/devices.json",
            "state/usage/../../x.tsv",
            "config/a/b/c.toml",
            "config/aleph.sh",
            "other/notes.json",
        ] {
            assert!(target(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn bundles_round_trip_and_say_what_changes() {
        let mut b = Bundle {
            made: 1_761_300_000,
            computer: "mac".into(),
            version: "2.0.0".into(),
            ..Default::default()
        };
        b.files.insert("state/notes.json".into(), "{}".into());
        b.left_out
            .insert("config/aleph.toml".into(), vec!["notify.ntfy".into()]);
        assert_eq!(Bundle::from_json(&b.to_json()).unwrap(), b);
        assert!(Bundle::from_json(&json!({"aleph_backup": 9}))
            .unwrap_err()
            .contains("newer"));
        assert!(Bundle::from_json(&json!({"x": 1})).is_err());
        assert_eq!(change(None, "a"), Change::New);
        assert_eq!(change(Some("a\nb"), "a\nb"), Change::Same);
        assert_eq!(change(Some("a\nb"), "a\nc"), Change::Differs(2));
    }

    #[test]
    fn local_stamps() {
        // the offset is this computer's; the shape is fixed
        let s = stamp(1_761_300_000);
        assert_eq!(s.len(), 13);
        assert_eq!(&s[8..9], "-");
    }
}
