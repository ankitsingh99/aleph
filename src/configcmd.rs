//! `aleph config [path | check | schema | edit | export | import | diff | sources | undo]`
//!
//! `aleph.toml` as a declarative, shareable file: validate it (with a
//! did-you-mean for typos), give editors a JSON Schema, export it (secrets
//! masked) for someone else, import theirs after showing exactly what changes,
//! see where each value comes from (include / main / profile / default), and
//! step back to the previous version.

use crate::cli::Args;
use crate::config::{self, Config, Kind, Source, KNOWN};
use crate::out::*;
use crate::tomlcfg;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const SECRET_WORDS: &[&str] = &[
    "ntfy", "slack", "webhook", "token", "secret", "password", "apikey",
];

pub(crate) fn is_secret(key: &str) -> bool {
    SECRET_WORDS.iter().any(|w| key.contains(w)) || crate::secrets::SECRET_KEYS.contains(&key)
}

pub fn cmd(a: Args) -> ExitCode {
    let sub = a.rest.first().map(String::as_str).unwrap_or("path");
    let rest: Vec<String> = a.rest.iter().skip(1).cloned().collect();
    let has = |f: &str| rest.iter().any(|x| x == f);
    match sub {
        "path" => {
            for (what, p) in a.cfg.files() {
                say(ratatui::text::Line::raw(format!("{what:<8} {}", p.display())));
            }
            if !a.cfg.is_toml() {
                hint("this is the old key=value format — aleph.toml replaces it on the next run without ALEPH_CONFIG");
            }
            ExitCode::SUCCESS
        }
        "check" => check(&a.cfg),
        // every setting as Markdown (docs/config.md is this)
        "docs" => {
            print!("{}", config::docs_markdown());
            ExitCode::SUCCESS
        }
        "schema" => {
            json(&schema());
            ExitCode::SUCCESS
        }
        "edit" => edit(&a.cfg),
        "export" => {
            print!("{}", export(&a.cfg, has("--with-secrets")));
            ExitCode::SUCCESS
        }
        "import" => match rest.iter().find(|x| !x.starts_with('-')) {
            Some(src) => import(a.cfg, src, has("--dry-run"), has("--yes")),
            None => fail("usage: aleph config import <file.toml | https://…> [--dry-run] [--yes]"),
        },
        "diff" => match rest.iter().find(|x| !x.starts_with('-')) {
            Some(src) => match read_entries(src) {
                Ok(theirs) => {
                    let n = print_diff(&a.cfg.entries(), &theirs);
                    if n == 0 {
                        ok("no differences");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            },
            None => fail("usage: aleph config diff <file.toml | https://…>"),
        },
        "sources" => sources(&a.cfg, rest.first().map(String::as_str)),
        "undo" => undo(a.cfg, has("--yes")),
        _ => fail("usage: aleph config [path | check | schema | edit | export [--with-secrets] | import <src> [--dry-run] | diff <src> | sources [key] | undo]"),
    }
}

// ------------------------------------------------------------------ check

/// Every key path in a TOML document, with the line it's on.
fn key_paths(text: &str) -> Vec<(Vec<String>, usize)> {
    let mut out = Vec::new();
    let mut table: Vec<String> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with('[') {
            let inner = l.trim_start_matches('[').trim_end_matches(']').trim();
            table = split_key(inner);
            continue;
        }
        if let Some((k, _)) = l.split_once('=') {
            let mut p = table.clone();
            p.extend(split_key(k.trim()));
            out.push((p, i + 1));
        }
    }
    out
}

/// `a."b.c".d` → [a, b.c, d]
fn split_key(k: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for c in k.chars() {
        match c {
            '"' => q = !q,
            '.' if !q => {
                v.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        v.push(cur.trim().to_string());
    }
    v
}

fn lev(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The closest known TOML path to a misspelled one.
pub fn suggest(path: &str) -> Option<String> {
    tomlcfg::PATHS
        .iter()
        .map(|(_, p)| (*p, lev(path, p)))
        .filter(|(_, d)| *d <= 3)
        .min_by_key(|(_, d)| *d)
        .map(|(p, _)| p.to_string())
}

/// Problems in a settings file: (line, message, is_error).
pub fn problems(text: &str) -> Vec<(usize, String, bool)> {
    let mut v = Vec::new();
    let doc = match text.parse::<toml_edit::DocumentMut>() {
        Ok(d) => d,
        Err(e) => {
            v.push((0, format!("not valid TOML: {e}"), true));
            return v;
        }
    };
    for (path, line) in key_paths(text) {
        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
        if refs == ["include"] {
            continue;
        }
        // presets have their own shape (panes, vars): checked below
        if matches!(refs.first(), Some(&"presets") | Some(&"preset")) {
            continue;
        }
        // notification routes: [[notify.route]] tables (router.rs reads them)
        if refs.len() > 2 && refs[0] == "notify" && refs[1] == "route" {
            continue;
        }
        // a profile's own table (aleph profile): what it's for, its showcase mode
        if refs.first() == Some(&"profile") {
            continue;
        }
        // a showcase build's settings, in a file this build reads too
        if refs.first() == Some(&"showcase") && !crate::showcase::built() {
            v.push((
                line,
                format!("`{}` is for a showcase build", path.join(".")),
                false,
            ));
            continue;
        }
        if refs.first() == Some(&"legacy") {
            v.push((
                line,
                format!("`{}` is kept but Aleph doesn't use it", path.join(".")),
                false,
            ));
            continue;
        }
        if tomlcfg::from_path(&refs).is_none() {
            let dotted = path.join(".");
            let hint = suggest(&dotted)
                .map(|s| format!(" — did you mean `{s}`?"))
                .unwrap_or_default();
            v.push((line, format!("unknown setting `{dotted}`{hint}"), true));
        }
    }
    for (k, val) in tomlcfg::entries(&doc) {
        if let Err(e) = config::check(&k, &val) {
            let line = key_paths(text)
                .into_iter()
                .find(|(p, _)| *p == tomlcfg::to_path(&k))
                .map(|(_, l)| l)
                .unwrap_or(0);
            v.push((line, e, true));
        }
    }
    let (_, bad) = crate::presets::from_doc(&doc, "");
    for b in bad {
        v.push((0, b, true));
    }
    v.sort_by_key(|x| x.0);
    v
}

fn check(cfg: &Config) -> ExitCode {
    let mut bad = 0;
    for (what, p) in cfg.files() {
        let Ok(text) = std::fs::read_to_string(&p) else {
            if what == "main" {
                ok(&format!(
                    "{} doesn't exist yet — every setting is at its default",
                    p.display()
                ));
            }
            continue;
        };
        if p.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let ps = problems(&text);
        if ps.is_empty() {
            ok(&format!("{} ({what})", crate::doctor::tilde(&p)));
        }
        for (line, msg, error) in ps {
            let at = if line > 0 {
                format!("{}:{line}", crate::doctor::tilde(&p))
            } else {
                crate::doctor::tilde(&p)
            };
            if error {
                bad += 1;
                err(&format!("{at}: {msg}"));
            } else {
                warn(&format!("{at}: {msg}"));
            }
        }
    }
    if bad > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

// ------------------------------------------------------------------ schema

fn kind_schema(k: Kind, default: &str, help: &str) -> Value {
    let mut v = match k {
        Kind::Bool => json!({"type": "boolean"}),
        Kind::Num => json!({"type": "number", "minimum": 0}),
        Kind::Choice(c) => json!({"type": "string", "enum": c}),
        // a comma list of these
        Kind::List(c) => json!({"type": "string", "examples": [c.join(",")]}),
        Kind::Text => json!({"type": "string"}),
    };
    v["description"] = json!(help);
    if !default.is_empty() {
        v["default"] = match k {
            Kind::Bool => json!(matches!(default, "true" | "on" | "1" | "yes")),
            Kind::Num => default
                .parse::<f64>()
                .map(|x| json!(x))
                .unwrap_or(json!(default)),
            _ => json!(default),
        };
    }
    v
}

/// A JSON Schema for `aleph.toml` (taplo / VS Code completion and checks).
pub fn schema() -> Value {
    let mut root = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "aleph.toml",
        "description": "Aleph's settings (aleph settings lists them all)",
        "type": "object",
        "properties": {
            "include": {"type": "array", "items": {"type": "string"},
                "description": "more settings files, lower priority than this one (globs allowed)"}
        }
    });
    for k in KNOWN {
        let p = tomlcfg::to_path(k.key);
        let mut node = &mut root;
        for part in &p[..p.len() - 1] {
            if node.get("properties").is_none() {
                node["properties"] = json!({});
                node["type"] = json!("object");
            }
            if node["properties"].get(part).is_none() {
                node["properties"][part] = json!({"type": "object", "properties": {}});
            }
            node = &mut node["properties"][part];
        }
        let mut s = kind_schema(k.kind, k.default, k.help);
        if matches!(
            k.key,
            "hosts" | "remote.allow" | "remote.deny" | "analytics.include" | "analytics.exclude"
        ) {
            s = json!({"type": "array", "items": {"type": "string"}, "description": k.help});
        }
        node["properties"][p.last().expect("path")] = s;
    }
    let host_fields = json!({
        "type": "object",
        "properties": {
            "color": {"type": "string", "description": "machine colour: a palette name or #rrggbb"},
            "machine": {"type": "string", "description": "herdr machine label, when it differs"},
            "dir": {"type": "string", "description": "start folder for the browser and new sessions"},
            "args": {"type": "string", "description": "default agent arguments there"},
            "agent": {"type": "string", "description": "default agent there"},
            "moshi": {"type": "string", "enum": ["auto", "full", "machine-only", "off"]},
            "session": {"type": "string", "description": "herdr session name on that machine"},
            "tags": {"type": "array", "items": {"type": "string"}},
            "mac": {"type": "string", "description": "for Wake-on-LAN"}
        }
    });
    root["properties"]["hosts"] = json!({"type": "object", "description": "one table per machine", "additionalProperties": host_fields});
    root["properties"]["presets"] = json!({"type": "object", "description": "saved starting points (aleph preset)", "additionalProperties": {"type": "object"}});
    root["properties"]["keys"] = json!({"type": "object", "description": "dashboard key overrides: action = \"key[,key]\"", "additionalProperties": {"type": "string"}});
    root["properties"]["actions"] = json!({"type": "object", "description": "custom keys: key = \"label|command\"", "additionalProperties": {"type": "string"}});
    if root["properties"]["analytics"].get("properties").is_some() {
        root["properties"]["analytics"]["properties"]["prices"] = json!({"type": "object", "description": "$ per million tokens per model: model = \"input,output[,cache write 5m,cache write 1h,cache read]\"", "additionalProperties": {"type": "string"}});
    }
    if root["properties"]["automation"].get("properties").is_some() {
        root["properties"]["automation"]["properties"]["rules"] = json!({"type": "object", "description": "auto-approve rules: name = \"pattern\"", "additionalProperties": {"type": "string"}});
    }
    root
}

// ------------------------------------------------------------------ edit

fn edit(cfg: &Config) -> ExitCode {
    let p = cfg.path.clone();
    if !p.exists() {
        let _ = config::write_private(&p, tomlcfg::render(&cfg.entries()).as_bytes());
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let before = std::fs::read_to_string(&p).unwrap_or_default();
    let st = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&p)
        .status();
    if st.is_err() {
        return fail(&format!("couldn't start {editor}"));
    }
    let after = std::fs::read_to_string(&p).unwrap_or_default();
    if after == before {
        ok("no changes");
        return ExitCode::SUCCESS;
    }
    // keep the previous version for `aleph config undo`
    let dir = config::state_dir().join("config-history");
    let _ = config::write_private(
        &dir.join(format!("aleph.toml.{}.edit", crate::cache::now())),
        before.as_bytes(),
    );
    let ps = problems(&after);
    let errors = ps.iter().filter(|x| x.2).count();
    for (line, msg, error) in &ps {
        let at = format!("line {line}");
        if *error {
            err(&format!("{at}: {msg}"));
        } else {
            warn(&format!("{at}: {msg}"));
        }
    }
    if errors > 0 {
        hint("aleph config undo puts the previous version back");
        return ExitCode::from(1);
    }
    ok("saved and checked");
    ExitCode::SUCCESS
}

// ------------------------------------------------------------------ export / import

pub fn export(cfg: &Config, with_secrets: bool) -> String {
    let entries: Vec<(String, String)> = cfg
        .entries()
        .into_iter()
        .map(|(k, v)| {
            // a `secret:NAME` reference is safe to share; the value isn't
            if !with_secrets && is_secret(&k) && !v.starts_with("secret:") {
                (k, "<secret: set it with aleph settings set>".to_string())
            } else {
                (k, v)
            }
        })
        .collect();
    tomlcfg::render(&entries)
}

/// Settings in a TOML file or URL, as flat entries.
pub(crate) fn read_entries(src: &str) -> Result<Vec<(String, String)>, String> {
    let text = if src.starts_with("https://") || src.starts_with("http://") {
        let o = crate::proc::cap("curl", &["-fsSL", "-m", "20", src], Duration::from_secs(25));
        if !o.ok() {
            return Err(format!("couldn't fetch {src}: {}", o.reason()));
        }
        o.text()
    } else {
        std::fs::read_to_string(config::expand(src)).map_err(|e| format!("{src}: {e}"))?
    };
    let doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{src}: {e}"))?;
    Ok(tomlcfg::entries(&doc))
}

/// Print what would change going from `ours` to `theirs`; returns the count.
fn print_diff(ours: &[(String, String)], theirs: &[(String, String)]) -> usize {
    use ratatui::prelude::*;
    let mut n = 0;
    for (k, v) in theirs {
        match ours.iter().find(|(x, _)| x == k) {
            Some((_, old)) if old == v => {}
            Some((_, old)) => {
                n += 1;
                let (o, nv) = if is_secret(k) {
                    ("…".into(), "…".into())
                } else {
                    (old.clone(), v.clone())
                };
                say(Line::from(vec![
                    Span::styled("  ~ ", crate::ui::fg(crate::ui::c::YELLOW())),
                    Span::raw(format!("{k}: {o} → {nv}")),
                ]));
            }
            None => {
                n += 1;
                let nv = if is_secret(k) {
                    "…".into()
                } else {
                    v.clone()
                };
                say(Line::from(vec![
                    Span::styled("  + ", crate::ui::fg(crate::ui::c::GREEN())),
                    Span::raw(format!("{k} = {nv}")),
                ]));
            }
        }
    }
    n
}

/// What importing `theirs` changes: (key, old value or None, new value).
pub(crate) fn changes(
    ours: &[(String, String)],
    theirs: &[(String, String)],
) -> Vec<(String, Option<String>, String)> {
    theirs
        .iter()
        .filter_map(|(k, v)| match ours.iter().find(|(x, _)| x == k) {
            Some((_, old)) if old == v => None,
            Some((_, old)) => Some((k.clone(), Some(old.clone()), v.clone())),
            None => Some((k.clone(), None, v.clone())),
        })
        .collect()
}

/// Keys an import should call out: they run commands, answer questions or
/// let phones act.
pub(crate) fn risky(k: &str) -> bool {
    k.starts_with("hook.")
        || k.starts_with("action.")
        || k.starts_with("approve.")
        || k == "auto_approve"
        || k.starts_with("remote.")
}

fn import(mut cfg: Config, src: &str, dry: bool, yes_flag: bool) -> ExitCode {
    let theirs = match read_entries(src) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    // a shared file must not run anything on its own: show what it changes
    let n = print_diff(&cfg.entries(), &theirs);
    if n == 0 {
        ok("nothing to change");
        return ExitCode::SUCCESS;
    }
    let risky: Vec<&String> = theirs.iter().map(|(k, _)| k).filter(|k| risky(k)).collect();
    if !risky.is_empty() {
        warn(&format!(
            "it also sets commands, approvals or remote control: {} — read them before saying yes",
            risky
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if dry {
        hint("nothing changed (--dry-run)");
        return ExitCode::SUCCESS;
    }
    if !yes_flag && !yes(&format!("Apply these {n} changes?")) {
        return ExitCode::from(1);
    }
    for (k, v) in &theirs {
        if cfg.get(k).as_deref() == Some(v.as_str()) {
            continue;
        }
        if let Err(e) = cfg.set(k, v) {
            return fail(&format!("{k}: {e}"));
        }
    }
    ok(&format!(
        "{n} settings changed (aleph config undo steps back)"
    ));
    ExitCode::SUCCESS
}

/// `$EDITOR` on the settings file from the dashboard: a version is kept for
/// undo first, and an edit with errors is put back at once (the rejected
/// text is kept next to the history). Ok: what happened, in words.
pub(crate) fn edit_checked(cfg: &Config) -> Result<String, String> {
    let p = cfg.path.clone();
    if !p.exists() {
        let _ = config::write_private(&p, tomlcfg::render(&cfg.entries()).as_bytes());
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let before = std::fs::read_to_string(&p).unwrap_or_default();
    let st = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&p)
        .status();
    if !st.is_ok_and(|s| s.success()) {
        return Err(format!(
            "{editor} didn't finish cleanly — nothing was checked"
        ));
    }
    let after = std::fs::read_to_string(&p).unwrap_or_default();
    if after == before {
        return Ok("no changes".into());
    }
    let hist = config::state_dir().join("config-history");
    let _ = config::write_private(
        &hist.join(format!("aleph.toml.{}.edit", crate::cache::now())),
        before.as_bytes(),
    );
    let ps = problems(&after);
    if let Some((line, msg, _)) = ps.iter().find(|x| x.2) {
        let rejected = config::state_dir().join("aleph.toml.rejected");
        let _ = config::write_private(&rejected, after.as_bytes());
        let _ = config::write_private(&p, before.as_bytes());
        return Err(format!(
            "line {line}: {msg} — put back as it was; your edit is in {}",
            crate::doctor::tilde(&rejected)
        ));
    }
    Ok(match ps.first() {
        Some((line, msg, _)) => format!("saved · line {line}: {msg}"),
        None => "saved and checked".into(),
    })
}

/// Undo the last change to the settings (`aleph config undo`, quietly).
pub(crate) fn undo_last(cfg: &Config) -> Result<String, String> {
    let prev = config::history_latest().ok_or("no earlier version kept yet")?;
    let data = std::fs::read(&prev).map_err(|e| format!("{}: {e}", prev.display()))?;
    if let Ok(cur) = std::fs::read(&cfg.path) {
        let _ = config::write_private(
            &config::state_dir()
                .join("config-history")
                .join(format!("aleph.toml.{}.undone", crate::cache::now())),
            &cur,
        );
    }
    config::write_private(&cfg.path, &data).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&prev);
    Ok("the previous version is back".into())
}

// ------------------------------------------------------------------ sources / undo

fn source_word(cfg: &Config, k: &str) -> String {
    match cfg.source(k) {
        Some(Source::Main) => "main".into(),
        Some(Source::Include(p)) => format!("include {}", crate::doctor::tilde(&p)),
        Some(Source::Profile(p)) => format!("profile {}", crate::doctor::tilde(&p)),
        None => "default".into(),
    }
}

fn sources(cfg: &Config, key: Option<&str>) -> ExitCode {
    let mut keys: Vec<String> = KNOWN.iter().map(|k| k.key.to_string()).collect();
    for (k, _) in cfg.entries() {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    if let Some(k) = key {
        keys.retain(|x| x == k);
        if keys.is_empty() {
            return fail(&format!("no setting {k}"));
        }
    }
    let w = keys.iter().map(|k| k.len()).max().unwrap_or(10).min(28);
    for k in keys {
        let v = cfg.get_raw(&k).unwrap_or_else(|| cfg.get_or(&k));
        let shown = if is_secret(&k) && !v.is_empty() && !v.starts_with("secret:") {
            "…".to_string()
        } else {
            v
        };
        println!(
            "{:<w$}  {:<30}  {}",
            k,
            crate::ui::trunc(&shown, 30),
            source_word(cfg, &k)
        );
    }
    ExitCode::SUCCESS
}

fn undo(cfg: Config, yes_flag: bool) -> ExitCode {
    let Some(prev) = config::history_latest() else {
        return fail("no earlier version kept yet");
    };
    let target: PathBuf = cfg.path.clone();
    if !yes_flag
        && !yes(&format!(
            "Put back the version from {} into {}?",
            crate::doctor::tilde(&prev),
            crate::doctor::tilde(&target)
        ))
    {
        return ExitCode::from(1);
    }
    restore(&prev, &target)
}

fn restore(prev: &Path, target: &Path) -> ExitCode {
    let Ok(data) = std::fs::read(prev) else {
        return fail(&format!("couldn't read {}", prev.display()));
    };
    if let Ok(cur) = std::fs::read(target) {
        let _ = config::write_private(
            &config::state_dir()
                .join("config-history")
                .join(format!("aleph.toml.{}.undone", crate::cache::now())),
            &cur,
        );
    }
    match config::write_private(target, &data) {
        Ok(()) => {
            let _ = std::fs::remove_file(prev);
            ok("restored the previous version");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_name_the_line_and_suggest() {
        let t = "[ui]\nscale = \"large\"\nscal = \"x\"\n[fleet]\nrefresh = \"fast\"\n[legacy]\nold = 1\n";
        let p = problems(t);
        assert!(
            p.iter()
                .any(|(l, m, e)| *l == 3 && *e && m.contains("did you mean `ui.scale`")),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|(l, m, e)| *l == 5 && *e && m.contains("bad value for refresh")),
            "{p:?}"
        );
        assert!(p.iter().any(|(l, _, e)| *l == 7 && !*e), "{p:?}");
        assert!(problems("[ui\n")
            .iter()
            .any(|(_, m, e)| *e && m.contains("not valid TOML")));
        assert!(problems("[ui]\nscale = \"large\"\n").is_empty());
    }

    #[test]
    fn schema_has_every_section() {
        let s = schema();
        assert_eq!(
            s["properties"]["ui"]["properties"]["scale"]["enum"][0],
            "compact"
        );
        assert_eq!(
            s["properties"]["automation"]["properties"]["auto_approve"]["type"],
            "boolean"
        );
        assert!(
            s["properties"]["hosts"]["additionalProperties"]["properties"]["color"].is_object()
        );
        assert_eq!(
            s["properties"]["fleet"]["properties"]["machines"]["type"],
            "array"
        );
    }

    #[test]
    fn export_masks_secrets() {
        let c = Config::in_memory(&["notify.ntfy=https://ntfy.sh/secret", "ui_scale=large"]);
        let e = export(&c, false);
        assert!(
            !e.contains("ntfy.sh/secret") && e.contains("scale = \"large\""),
            "{e}"
        );
        assert!(export(&c, true).contains("ntfy.sh/secret"));
    }
}
