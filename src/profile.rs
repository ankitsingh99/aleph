//! Settings profiles: `aleph profile [ls] | use NAME | off | show NAME |
//! set NAME KEY VALUE | unset NAME KEY | save NAME KEY… | edit NAME | rm NAME`.
//!
//! A profile is `profiles/<name>.toml` beside `aleph.toml`. It holds only the
//! settings it names and is layered over yours (the top layer,
//! `Source::Profile`), so turning it off brings yours back untouched.
//!
//! The profile in use is `state/profile.json`, so the daemon, every dashboard
//! and every command run with it; `ALEPH_PROFILE=<name>` still picks one for
//! one shell. A profile's own `[profile]` table says what it's for (`about`)
//! and can turn showcase mode on (`showcase = "masked"` or `"demo"`, in a
//! build that has it); `off` puts the mode back as it was.
//!
//! demo, travel and work are starters: the first `use` writes one out for
//! you to change.

use crate::cli::Args;
use crate::config::{self, Config};
use crate::out::*;
use crate::tomlcfg;
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;
use toml_edit::DocumentMut;

const USAGE: &str = "usage: aleph profile [ls] | use NAME | off | show NAME | set NAME KEY VALUE | unset NAME KEY | save NAME KEY… | edit NAME | rm NAME [--yes]";

#[cfg(feature = "showcase")]
const DEMO_ABOUT: &str = "showing Aleph to people: showcase mode over your fleet, the cockpit look, money hidden, no update card, notifications that don't quote agents";
#[cfg(not(feature = "showcase"))]
const DEMO_ABOUT: &str = "showing Aleph to people: the cockpit look, no update card, notifications that don't quote agents";
#[cfg(feature = "showcase")]
const DEMO_MODE: Option<&str> = Some("masked");
#[cfg(not(feature = "showcase"))]
const DEMO_MODE: Option<&str> = None;

/// A starter: (name, what it's for, its settings, a showcase mode).
pub type Starter = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
    Option<&'static str>,
);

/// The starters.
pub const STARTERS: &[Starter] = &[
    (
        "demo",
        DEMO_ABOUT,
        &[
            ("ui.splash", "cockpit"),
            ("ui.whats_new", "off"),
            ("notify.include_text", "off"),
            #[cfg(feature = "showcase")]
            ("showcase.money", "hidden"),
        ],
        DEMO_MODE,
    ),
    (
        "travel",
        "a weak or metered link: slower polls and stats, no LLM-server scans, usage read every 10 minutes, glyphs for logos",
        &[
            ("refresh", "2"),
            ("stats_interval", "5"),
            ("services", "off"),
            ("analytics.interval", "600"),
            ("ui.logos", "glyphs"),
        ],
        None,
    ),
    (
        "work",
        "a day at the desk: straight to the fleet (no splash), whole messages in notifications",
        &[("ui.splash", "off"), ("notify.include_text", "full")],
        None,
    ),
];

fn valid(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{name}: a profile's name is lowercase letters, digits, - and _ (up to 32)"
        ))
    }
}

pub fn dir() -> PathBuf {
    Config::dir().join("profiles")
}

pub fn path_of(name: &str) -> PathBuf {
    dir().join(format!("{name}.toml"))
}

fn state_path() -> PathBuf {
    config::state_dir().join("profile.json")
}

/// What `state/profile.json` says: the profile in use, and the showcase
/// mode from before it (to put back).
#[derive(Debug, Default, PartialEq)]
pub struct Active {
    pub name: String,
    pub showcase_before: Option<String>,
    pub since: u64,
}

pub fn active() -> Option<Active> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(state_path()).ok()?).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    valid(&name).ok()?;
    Some(Active {
        name,
        showcase_before: v
            .get("showcase_before")
            .and_then(Value::as_str)
            .map(String::from),
        since: v.get("since").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// The profile every process layers on: `ALEPH_PROFILE`, else the one in use.
pub fn active_name() -> Option<String> {
    crate::brand::env("PROFILE")
        .filter(|n| !n.is_empty())
        .or_else(|| active().map(|a| a.name))
}

/// The stamp a running process compares to notice a switch.
pub fn state_file() -> PathBuf {
    state_path()
}

/// A starter's file: its comment, its settings, its `[profile]` table.
pub fn starter_text(name: &str) -> Option<String> {
    let (n, about, sets, mode) = STARTERS.iter().find(|s| s.0 == name)?;
    let mut doc = DocumentMut::new();
    let mut t = toml_edit::Table::new();
    t.insert("about", toml_edit::value(*about));
    if let Some(m) = mode {
        t.insert("showcase", toml_edit::value(*m));
    }
    doc.insert("profile", toml_edit::Item::Table(t));
    for (k, v) in *sets {
        tomlcfg::set(&mut doc, k, v);
    }
    Some(format!(
        "# The {n} profile (aleph profile use {n}): only these settings, over yours.\n# Change it with aleph profile set {n} KEY VALUE, or edit this file.\n\n{doc}"
    ))
}

/// A profile's file, parsed: its settings, what it's for, its showcase mode.
pub struct Profile {
    pub name: String,
    pub about: String,
    pub showcase: Option<String>,
    pub settings: Vec<(String, String)>,
}

pub fn read(name: &str) -> Result<Profile, String> {
    valid(name)?;
    let p = path_of(name);
    let text = std::fs::read_to_string(&p).map_err(|_| format!("no profile {name}"))?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{}: not TOML ({e})", crate::doctor::tilde(&p)))?;
    let meta = |k: &str| {
        doc.get("profile")
            .and_then(|t| t.get(k))
            .and_then(|v| v.as_str())
            .map(String::from)
    };
    Ok(Profile {
        name: name.to_string(),
        about: meta("about").unwrap_or_default(),
        showcase: meta("showcase").filter(|m| matches!(m.as_str(), "masked" | "demo" | "off")),
        settings: tomlcfg::entries(&doc),
    })
}

/// Every profile: the files, then the starters not written out yet.
pub fn names() -> Vec<(String, bool)> {
    let mut v: Vec<(String, bool)> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "toml"))
                .then(|| p.file_stem()?.to_str().map(String::from))
                .flatten()
        })
        .filter(|n| valid(n).is_ok())
        .map(|n| (n, true))
        .collect();
    v.sort();
    for (s, ..) in STARTERS {
        if !v.iter().any(|(n, _)| n == s) {
            v.push((s.to_string(), false));
        }
    }
    v
}

fn write_state(a: &Active) -> Result<(), String> {
    let v = json!({"name": a.name, "showcase_before": a.showcase_before, "since": a.since});
    config::write_atomic(&state_path(), v.to_string().as_bytes()).map_err(|e| e.to_string())
}

/// Turn `name` on (writing a starter out first). The message says what it did.
pub fn use_profile(name: &str) -> Result<String, String> {
    valid(name)?;
    let p = path_of(name);
    let mut wrote = false;
    if !p.exists() {
        let Some(t) = starter_text(name) else {
            return Err(format!(
                "no profile {name}: aleph profile save {name} KEY… makes one from your settings, aleph profile ls lists them"
            ));
        };
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        config::write_atomic(&p, t.as_bytes()).map_err(|e| e.to_string())?;
        wrote = true;
    }
    let prof = read(name)?;
    for (k, v) in &prof.settings {
        if let Err(e) = config::check(k, v) {
            return Err(format!("{}: {k}: {e}", crate::doctor::tilde(&p)));
        }
    }
    let before = active();
    // the showcase mode from before the first profile, put back by `off`
    let mut showcase_before = before.as_ref().and_then(|a| a.showcase_before.clone());
    let mut said_mode = String::new();
    if let Some(m) = &prof.showcase {
        if crate::showcase::built() {
            let now = crate::showcase::mode_name();
            let now = if now == "real" {
                "off".to_string()
            } else {
                now.to_string()
            };
            if showcase_before.is_none() {
                showcase_before = Some(now.clone());
            }
            if *m != now {
                crate::showcase::switch(m)?;
                said_mode = format!(", showcase {m}");
            }
        }
    }
    write_state(&Active {
        name: name.to_string(),
        showcase_before,
        since: crate::cache::now(),
    })?;
    let n = prof.settings.len();
    Ok(format!(
        "profile {name} on: {n} setting{} over yours{said_mode}{}",
        if n == 1 { "" } else { "s" },
        if wrote {
            " (a starter, written out to change)"
        } else {
            ""
        }
    ))
}

/// Back to your own settings (and showcase mode as it was).
pub fn off() -> Result<String, String> {
    let Some(a) = active() else {
        return Ok("no profile in use: these are your own settings".into());
    };
    std::fs::remove_file(state_path()).map_err(|e| e.to_string())?;
    let mut said = String::new();
    if let Some(m) = a.showcase_before.filter(|_| crate::showcase::built()) {
        let now = crate::showcase::mode_name();
        let now = if now == "real" { "off" } else { now };
        if m != now {
            crate::showcase::switch(&m)?;
            said = format!(", showcase {m} again");
        }
    }
    Ok(format!("profile {} off: your own settings{said}", a.name))
}

fn status(cfg: &Config, json_out: bool) -> ExitCode {
    let act = active_name();
    let env = crate::brand::env("PROFILE").filter(|n| !n.is_empty());
    let list = names();
    if json_out {
        json(&json!({
            "active": act,
            "from_env": env.is_some(),
            "profiles": list.iter().map(|(n, written)| {
                let p = read(n).ok();
                json!({
                    "name": n,
                    "written": written,
                    "about": p.as_ref().map(|p| p.about.clone()).unwrap_or_else(|| STARTERS.iter().find(|s| s.0 == n).map(|s| s.1.to_string()).unwrap_or_default()),
                    "settings": p.as_ref().map(|p| p.settings.len()).unwrap_or(0),
                    "showcase": p.and_then(|p| p.showcase),
                })
            }).collect::<Vec<_>>(),
        }));
        return ExitCode::SUCCESS;
    }
    match &act {
        Some(n) if env.is_some() => head(&format!("profile {n} (ALEPH_PROFILE, this shell only)")),
        Some(n) => head(&format!("profile {n} in use")),
        None => head("no profile in use: your own settings"),
    }
    for (n, written) in &list {
        let p = read(n).ok();
        let about = p
            .as_ref()
            .map(|p| p.about.clone())
            .filter(|a| !a.is_empty())
            .or_else(|| STARTERS.iter().find(|s| s.0 == n).map(|s| s.1.to_string()))
            .unwrap_or_default();
        let what = match &p {
            Some(p) => format!(
                "{} setting{}",
                p.settings.len(),
                if p.settings.len() == 1 { "" } else { "s" }
            ),
            None if !written => "starter".into(),
            None => "unreadable".into(),
        };
        let on = act.as_deref() == Some(n.as_str());
        out(Line::from(vec![
            Span::styled(
                if on { "  ● " } else { "    " },
                crate::ui::fg(crate::ui::c::ACCENT()),
            ),
            Span::styled(
                format!("{}  ", crate::ui::pad(n, 12)),
                if on { crate::ui::bold() } else { Style::new() },
            ),
            Span::styled(
                format!("{}  ", crate::ui::pad(&what, 11)),
                crate::ui::muted(),
            ),
            Span::styled(crate::ui::trunc(&about, 90), crate::ui::faint()),
        ]));
    }
    let _ = cfg;
    hint(if act.is_some() {
        "aleph profile off · aleph profile show NAME"
    } else {
        "aleph profile use NAME · aleph profile save NAME KEY… makes one from your settings"
    });
    ExitCode::SUCCESS
}

fn show(cfg: &Config, name: &str, json_out: bool) -> ExitCode {
    let prof = match read(name) {
        Ok(p) => p,
        Err(_) if starter_text(name).is_some() => {
            hint(&format!(
                "{name} is a starter: aleph profile use {name} writes it out"
            ));
            return ExitCode::SUCCESS;
        }
        Err(e) => return fail(&e),
    };
    // your own value of each, without any profile
    let own = |k: &str| {
        cfg.get_own(k).unwrap_or_else(|| {
            config::known(k)
                .map(|k| k.default.to_string())
                .unwrap_or_default()
        })
    };
    if json_out {
        json(&json!({
            "name": prof.name, "about": prof.about, "showcase": prof.showcase,
            "settings": prof.settings.iter().map(|(k, v)| json!({"key": k, "value": v, "yours": own(k)})).collect::<Vec<_>>(),
        }));
        return ExitCode::SUCCESS;
    }
    head(&format!(
        "profile {name}  ·  {}",
        crate::doctor::tilde(&path_of(name))
    ));
    if !prof.about.is_empty() {
        out(Line::from(Span::styled(
            format!("  {}", prof.about),
            crate::ui::faint(),
        )));
    }
    if let Some(m) = &prof.showcase {
        out(Line::from(vec![
            Span::raw(format!("  {}  ", crate::ui::pad("showcase", 24))),
            Span::styled(m.clone(), crate::ui::fg(crate::ui::c::ACCENT())),
        ]));
    }
    for (k, v) in &prof.settings {
        let mine = own(k);
        let shown = |x: &str| {
            if crate::configcmd::is_secret(k) && !x.is_empty() {
                "…".to_string()
            } else if x.is_empty() {
                "–".to_string()
            } else {
                x.to_string()
            }
        };
        out(Line::from(vec![
            Span::raw(format!("  {}  ", crate::ui::pad(k, 24))),
            Span::styled(
                format!("{}  ", crate::ui::pad(&shown(v), 14)),
                crate::ui::fg(crate::ui::c::ACCENT()),
            ),
            Span::styled(
                if mine == *v {
                    "(the same as yours)".to_string()
                } else {
                    format!("yours: {}", shown(&mine))
                },
                crate::ui::faint(),
            ),
        ]));
    }
    ExitCode::SUCCESS
}

fn edit_file(name: &str, f: impl FnOnce(&mut DocumentMut) -> Result<String, String>) -> ExitCode {
    if let Err(e) = valid(name) {
        return fail(&e);
    }
    let p = path_of(name);
    let text = std::fs::read_to_string(&p).ok().or_else(|| starter_text(name)).unwrap_or_else(|| {
        format!("# The {name} profile (aleph profile use {name}): only these settings, over yours.\n\n")
    });
    let mut doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => return fail(&format!("{}: not TOML ({e})", crate::doctor::tilde(&p))),
    };
    let msg = match f(&mut doc) {
        Ok(m) => m,
        Err(e) => return fail(&e),
    };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = config::write_atomic(&p, doc.to_string().as_bytes()) {
        return fail(&e.to_string());
    }
    ok(&msg);
    if active_name().as_deref() == Some(name) {
        hint("in use now: running dashboards and the daemon pick it up within seconds");
    }
    ExitCode::SUCCESS
}

pub fn cmd(a: Args) -> ExitCode {
    let pos = a.positional();
    let json_out = a.has("--json");
    let arg = |i: usize| pos.get(i).map(|s| s.to_string());
    match pos.first().copied() {
        None | Some("ls" | "list" | "status") => status(&a.cfg, json_out),
        Some("use") => match arg(1) {
            Some(n) => match use_profile(&n) {
                Ok(m) => {
                    ok(&m);
                    hint("aleph profile off brings your own settings back · aleph profile show to see them");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            },
            None => fail("usage: aleph profile use NAME"),
        },
        Some("off") => match off() {
            Ok(m) => {
                ok(&m);
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        Some("show") => match arg(1).or_else(active_name) {
            Some(n) => show(&a.cfg, &n, json_out),
            None => fail("usage: aleph profile show NAME"),
        },
        Some("set") => match (arg(1), arg(2), pos.get(3..).map(|r| r.join(" "))) {
            (Some(n), Some(k), Some(v)) if !v.is_empty() => {
                if config::known(&k).is_none()
                    && !k.starts_with("host.")
                    && !k.starts_with("preset.")
                    && !k.starts_with("workspace.")
                {
                    return fail(&format!("{k}: not a setting (aleph settings lists them)"));
                }
                if let Err(e) = config::check(&k, &v) {
                    return fail(&format!("{k}: {e}"));
                }
                edit_file(&n, |doc| {
                    tomlcfg::set(doc, &k, &v);
                    Ok(format!("profile {n}: {k} = {v}"))
                })
            }
            _ => fail("usage: aleph profile set NAME KEY VALUE"),
        },
        Some("unset") => match (arg(1), arg(2)) {
            (Some(n), Some(k)) => edit_file(&n, |doc| {
                if tomlcfg::unset(doc, &k) {
                    Ok(format!("profile {n}: {k} is yours again"))
                } else {
                    Err(format!("profile {n} doesn't set {k}"))
                }
            }),
            _ => fail("usage: aleph profile unset NAME KEY"),
        },
        Some("save") => match arg(1) {
            Some(n) if pos.len() > 2 => {
                let keys: Vec<String> = pos[2..].iter().map(|s| s.to_string()).collect();
                let cfg = a.cfg.clone();
                edit_file(&n, |doc| {
                    let mut done = Vec::new();
                    for k in &keys {
                        let Some(v) = cfg
                            .get_raw(k)
                            .or_else(|| config::known(k).map(|x| x.default.to_string()))
                        else {
                            return Err(format!("{k}: not a setting"));
                        };
                        tomlcfg::set(doc, k, &v);
                        done.push(k.as_str());
                    }
                    Ok(format!(
                        "profile {n}: saved {} as they are now",
                        done.join(", ")
                    ))
                })
            }
            _ => {
                fail("usage: aleph profile save NAME KEY… (the settings to keep, as they are now)")
            }
        },
        Some("edit") => match arg(1) {
            Some(n) => {
                if let Err(e) = valid(&n) {
                    return fail(&e);
                }
                let p = path_of(&n);
                if !p.exists() {
                    let t = starter_text(&n).unwrap_or_else(|| format!("# The {n} profile: only these settings, over yours.\n\n[profile]\nabout = \"\"\n"));
                    if let Some(d) = p.parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    let _ = config::write_atomic(&p, t.as_bytes());
                }
                let ed = std::env::var("VISUAL")
                    .or_else(|_| std::env::var("EDITOR"))
                    .unwrap_or_else(|_| "vi".into());
                let st = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!(
                        "{ed} {}",
                        crate::proc::sh_quote(&p.to_string_lossy())
                    ))
                    .status();
                if !st.is_ok_and(|s| s.success()) {
                    return fail(&format!("{ed} didn't finish cleanly"));
                }
                match read(&n) {
                    Ok(p) => {
                        ok(&format!("profile {n}: {} settings", p.settings.len()));
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(&e),
                }
            }
            None => fail("usage: aleph profile edit NAME"),
        },
        Some("rm") => match arg(1) {
            Some(n) => {
                if let Err(e) = valid(&n) {
                    return fail(&e);
                }
                let p = path_of(&n);
                if !p.exists() {
                    return fail(&format!("no profile {n}"));
                }
                if !a.has("--yes")
                    && !yes(&format!(
                        "delete the {n} profile ({})?",
                        crate::doctor::tilde(&p)
                    ))
                {
                    hint("kept");
                    return ExitCode::from(1);
                }
                if active().is_some_and(|x| x.name == n) {
                    if let Ok(m) = off() {
                        ok(&m);
                    }
                }
                match std::fs::remove_file(&p) {
                    Ok(()) => {
                        ok(&format!("profile {n} deleted"));
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(&e.to_string()),
                }
            }
            None => fail("usage: aleph profile rm NAME [--yes]"),
        },
        Some(_) => {
            raw(USAGE);
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_are_valid_settings() {
        for (n, about, sets, _) in STARTERS {
            let t = starter_text(n).unwrap();
            let doc: DocumentMut = t.parse().unwrap();
            let es = tomlcfg::entries(&doc);
            assert!(!about.is_empty());
            for (k, v) in *sets {
                // a showcase setting is a setting only in a build that has it
                if k.starts_with("showcase.") && !cfg!(feature = "showcase") {
                    continue;
                }
                assert!(
                    es.iter().any(|(x, y)| x == k && y == v),
                    "{n}: {k} = {v} in\n{t}"
                );
                assert!(config::check(k, v).is_ok(), "{n}: {k} = {v}");
            }
            // the [profile] table isn't a setting
            assert!(!es.iter().any(|(k, _)| k.starts_with("profile")), "{es:?}");
        }
        assert!(
            valid("demo-2").is_ok()
                && valid("Demo").is_err()
                && valid("../x").is_err()
                && valid("").is_err()
        );
    }
}
