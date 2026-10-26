//! `aleph theme` — identity colours, in our views and in herdr's.
//!
//! * `show` (default): machine swatches and the session hues.
//! * `herdr [--apply]`: a sidebar config for the LOCAL herdr client that
//!   colours each session's glyph by its hue and each machine label by its
//!   machine colour. `--apply` writes it into `~/.config/herdr/config.toml`
//!   between `# >>> aleph >>>` markers (timestamped backup first), but
//!   only when no unmanaged `[ui.sidebar.agents…]` table exists; otherwise it
//!   saves `~/.config/aleph/herdr-sidebar.toml` and explains the merge.
//! * `machines [--apply]`: `[ui] accent = "<machine colour>"` in every host's
//!   own `~/.config/herdr/config.toml` (herdr renders on the server, so each
//!   machine gets its own accent), then `herdr --machine M server reload-config`.
//!
//! herdr's config is edited with a small line-based TOML editor (`toml`
//! below): it never creates a duplicate table, keeps comments, formatting and
//! CRLF line endings, and only touches the lines it has to.

use crate::cli::{self, Args};
use crate::config::{self, Config};
use crate::doctor::tilde;
use crate::out::*;
use crate::ui::{self, c};
use crate::{herdr, proc, sshcfg, theme};
use ratatui::prelude::*;
use serde_json::json;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::mpsc::channel;
use std::time::Duration;

pub const BEGIN: &str = crate::brand::RC_BEGIN;
pub const END: &str = crate::brand::RC_END;
const T_SSH: Duration = Duration::from_secs(25);

pub fn cmd(a: Args) -> ExitCode {
    let apply = a.rest.iter().any(|x| x == "--apply");
    let js = a.rest.iter().any(|x| x == "--json");
    let sub = a
        .rest
        .iter()
        .find(|x| !x.starts_with('-'))
        .map(String::as_str);
    match sub {
        None | Some("show") => show(&a.cfg, js),
        Some("herdr") | Some("sidebar") => sidebar_cmd(&a.cfg, apply),
        Some("machines") | Some("accent") => machines_cmd(&a, apply, js),
        Some(x) => {
            err(&format!("unknown theme command '{x}'"));
            hint("aleph theme [show | herdr [--apply] | machines [--apply]]");
            ExitCode::from(2)
        }
    }
}

// ------------------------------------------------------------------ show

fn show(cfg: &Config, js: bool) -> ExitCode {
    let hosts = cfg.hosts();
    if js {
        json(&json!({
            "machines": hosts.iter().map(|h| json!({
                "alias": h,
                "machine": machine_label(cfg, h),
                "color": theme::machine_hex(cfg, h),
                "name": cfg.host_get(h, "color").unwrap_or_default(),
            })).collect::<Vec<_>>(),
            "hues": theme::HUES.iter().map(|h| json!({
                "name": h.0, "glyph": h.1, "dark": format!("#{:06x}", h.2), "light": format!("#{:06x}", h.3),
            })).collect::<Vec<_>>(),
        }));
        return ExitCode::SUCCESS;
    }
    if !io::stdout().is_terminal() {
        for h in &hosts {
            println!("machine\t{h}\t{}", theme::machine_hex(cfg, h));
        }
        for h in theme::HUES {
            println!("hue\t{}\t#{:06x}\t{}", h.0, h.2, h.1);
        }
        return ExitCode::SUCCESS;
    }
    out(Line::from(Span::styled("Machines", ui::bold())));
    if hosts.is_empty() {
        out(Line::from(Span::styled(
            "  none yet — aleph host include <alias>",
            ui::muted(),
        )));
    }
    let w = hosts.iter().map(|h| h.chars().count()).max().unwrap_or(0);
    for h in &hosts {
        let col = theme::machine(cfg, h);
        out(Line::from(vec![
            Span::raw("  "),
            Span::styled("███", ui::fg(col)),
            Span::raw(" "),
            Span::styled(format!("{h:<w$}"), ui::fg(col).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(
                    "  {:<9} {}",
                    cfg.host_get(h, "color").unwrap_or_default(),
                    theme::machine_hex(cfg, h)
                ),
                ui::muted(),
            ),
        ]));
    }
    out(Line::from(""));
    out(Line::from(Span::styled("Session hues", ui::bold())));
    for row in theme::HUES.chunks(4) {
        let mut v = vec![Span::raw("  ")];
        for h in row {
            let col = theme::named(h.0).unwrap_or(c::MUTED());
            v.push(Span::styled(format!("{} ", h.1), ui::fg(col)));
            v.push(Span::styled(format!("{:<9}", h.0), ui::fg(col)));
        }
        out(Line::from(v));
    }
    out(Line::from(""));
    hint("aleph host color <alias> <name|#hex> · aleph theme herdr --apply · aleph theme machines --apply");
    ExitCode::SUCCESS
}

/// herdr's machine label for a fleet alias (what its `machine` token shows).
fn machine_label(cfg: &Config, alias: &str) -> String {
    cfg.host_get(alias, "machine")
        .or_else(|| {
            (cfg.get("ssh_host").as_deref() == Some(alias))
                .then(|| cfg.get("machine"))
                .flatten()
        })
        .unwrap_or_else(|| alias.to_string())
}

// ------------------------------------------------------------------ herdr sidebar

fn toml_str(s: &str) -> String {
    let mut o = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The sidebar rows: glyph coloured by hue, bold tab, state; the task title;
/// the machine label coloured by machine.
pub fn sidebar_rows(machines: &[(String, String)]) -> String {
    let mut glyphs: Vec<(&str, u32)> = Vec::new();
    for h in theme::HUES {
        if !glyphs.iter().any(|g| g.0 == h.1) {
            glyphs.push((h.1, h.2));
        }
    }
    let marks: Vec<String> = glyphs
        .iter()
        .take(16)
        .map(|(g, x)| format!("{{ equals = {}, fg = \"#{x:06x}\" }}", toml_str(g)))
        .collect();
    let mut seen: Vec<&str> = Vec::new();
    let mrules: Vec<String> = machines
        .iter()
        .filter(|(l, _)| {
            let fresh = !seen.contains(&l.as_str());
            seen.push(l);
            fresh
        })
        .take(16)
        .map(|(l, hex)| format!("{{ equals = {}, fg = {} }}", toml_str(l), toml_str(hex)))
        .collect();
    format!(
        "[\n  [\"state_icon\", {{ token = \"$mark\", rules = [{}] }}, {{ token = \"tab\", bold = true }}, \"state_text\"],\n  [\"terminal_title_stripped\"],\n  [{{ token = \"machine\", rules = [{}] }}, \"workspace\"],\n]",
        marks.join(", "),
        mrules.join(", ")
    )
}

/// The full managed block (markers included), `\n` line endings.
pub fn sidebar_block(machines: &[(String, String)]) -> String {
    let rows = sidebar_rows(machines);
    format!(
        "{BEGIN}\n# written by `aleph theme herdr --apply` — rerun it after changing machine colours\n[ui.sidebar.agents]\nrows = {rows}\n\n[ui.sidebar.agents.rows_by_agent]\nclaude = {rows}\n{END}\n"
    )
}

fn fleet_machines(cfg: &Config) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for h in cfg.hosts() {
        let hex = theme::machine_hex(cfg, &h);
        let label = machine_label(cfg, &h);
        if label != h {
            v.push((label, hex.clone()));
        }
        v.push((h, hex));
    }
    v
}

pub(crate) fn herdr_config() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| config::home().join(".config"))
        .join("herdr")
        .join("config.toml")
}

fn sidebar_cmd(cfg: &Config, apply: bool) -> ExitCode {
    let block = sidebar_block(&fleet_machines(cfg));
    if !apply {
        print!("{block}");
        if io::stdout().is_terminal() {
            hint("aleph theme herdr --apply writes this into ~/.config/herdr/config.toml");
        }
        return ExitCode::SUCCESS;
    }
    let path = herdr_config();
    let src = std::fs::read_to_string(&path).unwrap_or_default();
    match toml::set_block(&src, &block) {
        Ok(new) if new == src => {
            ok(&format!("{} already has the aleph sidebar", tilde(&path)));
            ExitCode::SUCCESS
        }
        Ok(new) => {
            if path.exists() {
                let bak = backup_path(&path);
                if let Err(e) = std::fs::copy(&path, &bak) {
                    return fail(&format!("couldn't back up {}: {e}", tilde(&path)));
                }
                say(Line::from(Span::styled(
                    format!("  backup {}", tilde(&bak)),
                    ui::faint(),
                )));
            }
            if let Err(e) = config::write_atomic(&path, new.as_bytes()) {
                return fail(&format!("couldn't write {}: {e}", tilde(&path)));
            }
            ok(&format!("herdr sidebar → {}", tilde(&path)));
            hint("reload herdr's config: herdr menu → reload config (or restart the client)");
            ExitCode::SUCCESS
        }
        Err(conflict) => {
            let side = config::Config::path()
                .parent()
                .map(|p| p.join("herdr-sidebar.toml"))
                .unwrap_or_else(|| PathBuf::from("herdr-sidebar.toml"));
            if let Err(e) = config::write_atomic(&side, block.as_bytes()) {
                return fail(&format!("couldn't write {}: {e}", tilde(&side)));
            }
            warn(&format!(
                "{} already defines {conflict} — left it untouched",
                tilde(&path)
            ));
            ok(&format!("saved the sidebar config to {}", tilde(&side)));
            hint("merge its rows into your [ui.sidebar.agents] table by hand, or remove yours and rerun aleph theme herdr --apply");
            ExitCode::from(1)
        }
    }
}

/// The sidebar written into herdr's config.toml, without printing (Settings ›
/// Health's fix): a message saying what happened, Err when it couldn't.
pub fn apply_sidebar(cfg: &Config) -> Result<String, String> {
    let block = sidebar_block(&fleet_machines(cfg));
    let path = herdr_config();
    let src = std::fs::read_to_string(&path).unwrap_or_default();
    match toml::set_block(&src, &block) {
        Ok(new) if new == src => Ok(format!("{} already has the aleph sidebar", tilde(&path))),
        Ok(new) => {
            if path.exists() {
                std::fs::copy(&path, backup_path(&path))
                    .map_err(|e| format!("couldn't back up {}: {e}", tilde(&path)))?;
            }
            config::write_atomic(&path, new.as_bytes())
                .map_err(|e| format!("couldn't write {}: {e}", tilde(&path)))?;
            Ok(format!(
                "herdr sidebar → {} (reload herdr's config to see it)",
                tilde(&path)
            ))
        }
        Err(conflict) => Err(format!(
            "{} already defines {conflict} — left it alone (aleph theme herdr shows the block)",
            tilde(&path)
        )),
    }
}

/// `config.toml` → `config.toml.bak.20261021143005` (local time is not
/// available without libc; UTC keeps backups sortable).
fn backup_path(p: &std::path::Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(format!(".bak.{}", stamp(crate::cache::now())));
    PathBuf::from(s)
}

/// `YYYYmmddHHMMSS` (UTC) for a unix time.
pub fn stamp(t: u64) -> String {
    let (days, rem) = ((t / 86400) as i64, t % 86400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

// ------------------------------------------------------------------ per-machine accents

const REMOTE_FILE: &str = r#"f="$HOME/.config/herdr/config.toml""#;

fn machines_cmd(a: &Args, apply: bool, js: bool) -> ExitCode {
    let hosts: Vec<String> = match &a.flag {
        Some(h) => vec![h.clone()],
        None => a.cfg.hosts(),
    };
    if hosts.is_empty() {
        return cli::no_host();
    }
    let plan: Vec<(String, String, String)> = hosts
        .iter()
        .map(|h| {
            (
                h.clone(),
                machine_label(&a.cfg, h),
                theme::machine_hex(&a.cfg, h),
            )
        })
        .collect();
    if !apply {
        if js {
            json(&json!(plan
                .iter()
                .map(|(h, m, x)| json!({"alias": h, "machine": m, "accent": x}))
                .collect::<Vec<_>>()));
            return ExitCode::SUCCESS;
        }
        for (h, _, hex) in &plan {
            let col = ui::parse_color(hex).unwrap_or(c::MUTED());
            out(Line::from(vec![
                Span::styled("  ███ ", ui::fg(col)),
                Span::styled(h.clone(), ui::fg(col).add_modifier(Modifier::BOLD)),
                Span::styled(format!("  [ui] accent = \"{hex}\""), ui::muted()),
            ]));
        }
        hint("aleph theme machines --apply sets it in each machine's ~/.config/herdr/config.toml");
        return ExitCode::SUCCESS;
    }
    let sp = Spinner::start(format!("Setting herdr accents on {}", hosts.join(", ")));
    let (tx, rx) = channel();
    for (h, m, hex) in plan.clone() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let r = apply_accent(&h, &m, &hex);
            let _ = tx.send((h, hex, r));
        });
    }
    drop(tx);
    let mut res: Vec<(String, String, Result<String, String>)> = rx.iter().collect();
    sp.stop();
    res.sort_by_key(|r| hosts.iter().position(|h| *h == r.0));
    let mut bad = 0;
    let mut rows = Vec::new();
    for (h, hex, r) in res {
        match &r {
            Ok(note) => ok(&format!("{h}  accent {hex}  {note}")),
            Err(e) => {
                bad += 1;
                err(&format!("{h}: {e}"));
            }
        }
        rows.push(json!({"alias": h, "accent": hex, "ok": r.is_ok(), "note": r.as_ref().map_or_else(|e| e.clone(), |n| n.clone())}));
    }
    if js {
        json(&json!(rows));
    }
    if bad > 0 {
        hint("aleph doctor shows why a machine can't be reached");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn ssh_sh(host: &str, script: &str, input: Option<&[u8]>) -> Result<proc::Out, String> {
    proc::run(
        Command::new(sshcfg::ssh_bin())
            .args(sshcfg::batch())
            .arg("-T")
            .arg(host)
            .arg(format!("sh -c '{script}'")),
        T_SSH,
        input,
    )
    .map_err(|e| format!("can't run ssh: {e}"))
}

/// Read, edit and write back one machine's herdr config, then reload it.
fn apply_accent(host: &str, machine: &str, hex: &str) -> Result<String, String> {
    let read = format!(
        r#"{REMOTE_FILE}; if [ -f "$f" ]; then echo CH-EXISTS; cat "$f"; else echo CH-MISSING; fi"#
    );
    let o = ssh_sh(host, &read, None)?;
    if !o.ok() {
        let p = sshcfg::Probe {
            fail: sshcfg::classify(&o),
            detail: o.reason(),
        };
        return Err(p.explain());
    }
    let raw = o.text();
    let (exists, src) = if let Some(r) = raw.strip_prefix("CH-EXISTS\n") {
        (true, r.to_string())
    } else if raw.starts_with("CH-MISSING") {
        (false, String::new())
    } else {
        return Err("unexpected reply while reading ~/.config/herdr/config.toml".into());
    };
    let new = toml::set_ui_accent(&src, hex)?;
    if new == src {
        return Ok("(already set)".into());
    }
    let ts = stamp(crate::cache::now());
    let write = format!(
        r#"{REMOTE_FILE}; mkdir -p "$HOME/.config/herdr" && {{ [ -f "$f" ] && cp -p "$f" "$f.bak.{ts}" || true; }} && cat > "$f.ch-tmp.$$" && mv "$f.ch-tmp.$$" "$f""#
    );
    let o = ssh_sh(host, &write, Some(new.as_bytes()))?;
    if !o.ok() {
        return Err(format!("couldn't write the config: {}", o.reason()));
    }
    let mut note = if exists {
        format!("(backup config.toml.bak.{ts})")
    } else {
        "(new config.toml)".into()
    };
    let r = proc::cap(
        &herdr::bin(),
        &["--machine", machine, "server", "reload-config"],
        Duration::from_secs(20),
    );
    if !r.ok() {
        note.push_str(" · not reloaded (herdr server not running?) — it applies on next start");
    }
    Ok(note)
}

// ------------------------------------------------------------------ TOML editor

/// A minimal line-based TOML editor: enough structure (tables, keys,
/// multi-line values and strings, comments) to edit herdr's config without
/// rewriting it.
pub mod toml {
    use super::{BEGIN, END};

    #[derive(Clone, Debug, PartialEq)]
    pub enum Kind {
        /// blank or comment-only
        Blank,
        /// `[name]` or `[[name]]`
        Header {
            name: String,
            array: bool,
        },
        /// `key = …` (key normalised: no quotes/spaces, dotted)
        Key {
            key: String,
        },
        /// continuation of a multi-line value or string
        Cont,
        Other,
    }

    #[derive(Clone, Debug)]
    pub struct Ln<'a> {
        /// the line without its ending
        pub text: &'a str,
        /// `\n`, `\r\n` or empty (last line)
        pub end: &'a str,
        pub kind: Kind,
        /// the table this line belongs to ("" = root)
        pub table: String,
    }

    #[derive(Default)]
    struct St {
        depth: i32,
        /// inside `"""` or `'''`
        ml: Option<&'static str>,
    }

    /// Scan `s` (a value or continuation) updating bracket depth and
    /// multi-line string state; stops at a comment.
    fn scan(s: &str, st: &mut St) {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if let Some(q) = st.ml {
                if b[i..].starts_with(q.as_bytes()) {
                    st.ml = None;
                    i += 3;
                    // `""""` / `''''`: up to two quotes may belong to the string
                    while i < b.len() && b[i] == q.as_bytes()[0] {
                        i += 1;
                    }
                } else {
                    i += if b[i] == b'\\' && q == "\"\"\"" { 2 } else { 1 };
                }
                continue;
            }
            match b[i] {
                b'#' => return,
                b'"' | b'\'' => {
                    let q = b[i];
                    if b[i..].starts_with(b"\"\"\"") || b[i..].starts_with(b"'''") {
                        st.ml = Some(if q == b'"' { "\"\"\"" } else { "'''" });
                        i += 3;
                        continue;
                    }
                    i += 1;
                    while i < b.len() && b[i] != q {
                        i += if q == b'"' && b[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                }
                b'[' | b'{' => {
                    st.depth += 1;
                    i += 1;
                }
                b']' | b'}' => {
                    st.depth -= 1;
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }

    /// `"ui" . 'sidebar'` → `ui.sidebar`.
    pub fn norm_key(k: &str) -> String {
        k.chars()
            .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
            .collect()
    }

    /// Index of the first `ch` outside quotes.
    fn find_unquoted(s: &str, ch: u8) -> Option<usize> {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'"' | b'\'' => {
                    let q = b[i];
                    i += 1;
                    while i < b.len() && b[i] != q {
                        i += if q == b'"' && b[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                }
                x if x == ch => return Some(i),
                _ => i += 1,
            }
        }
        None
    }

    pub fn lines(src: &str) -> Vec<Ln<'_>> {
        let mut out = Vec::new();
        let mut st = St::default();
        let mut table = String::new();
        for raw in src.split_inclusive('\n') {
            let (text, end) = if let Some(t) = raw.strip_suffix("\r\n") {
                (t, "\r\n")
            } else if let Some(t) = raw.strip_suffix('\n') {
                (t, "\n")
            } else {
                (raw, "")
            };
            let t = text.trim_start();
            let kind = if st.ml.is_some() || st.depth > 0 {
                scan(text, &mut st);
                Kind::Cont
            } else if t.is_empty() || t.starts_with('#') {
                Kind::Blank
            } else if t.starts_with('[') {
                let array = t.starts_with("[[");
                let inner = &t[if array { 2 } else { 1 }..];
                let close = find_unquoted(inner, b']').unwrap_or(inner.len());
                let name = norm_key(&inner[..close]);
                table = name.clone();
                Kind::Header { name, array }
            } else if let Some(eq) = find_unquoted(t, b'=') {
                scan(&t[eq + 1..], &mut st);
                Kind::Key {
                    key: norm_key(&t[..eq]),
                }
            } else {
                Kind::Other
            };
            out.push(Ln {
                text,
                end,
                kind,
                table: table.clone(),
            });
        }
        out
    }

    /// The line ending the file mostly uses.
    pub fn eol(src: &str) -> &'static str {
        let crlf = src.matches("\r\n").count();
        if crlf > 0 && crlf * 2 >= src.matches('\n').count() {
            "\r\n"
        } else {
            "\n"
        }
    }

    fn join(v: &[(String, String)]) -> String {
        v.iter().map(|(t, e)| format!("{t}{e}")).collect()
    }

    /// Replace a key's value on one line, keeping the key, spacing and any
    /// trailing comment.
    fn replace_value(text: &str, value: &str) -> String {
        let Some(eq) = find_unquoted(text, b'=') else {
            return text.to_string();
        };
        let after = &text[eq + 1..];
        let ws = after.len() - after.trim_start().len();
        let v = &after[ws..];
        let vend = match v.as_bytes().first() {
            Some(q @ (b'"' | b'\'')) => {
                let b = v.as_bytes();
                let mut i = 1;
                while i < b.len() && b[i] != *q {
                    i += if *q == b'"' && b[i] == b'\\' { 2 } else { 1 };
                }
                (i + 1).min(v.len())
            }
            _ => find_unquoted(v, b'#').map_or(v.len(), |i| v[..i].trim_end().len()),
        };
        format!("{}{}{value}{}", &text[..eq + 1], &after[..ws], &v[vend..])
    }

    /// Set `[ui] accent = "<hex>"`, never creating a duplicate `[ui]` table.
    pub fn set_ui_accent(src: &str, hex: &str) -> Result<String, String> {
        let value = format!("\"{}\"", hex.replace(['"', '\\'], ""));
        let nl = eol(src);
        let ls = lines(src);
        let mut v: Vec<(String, String)> = ls
            .iter()
            .map(|l| (l.text.to_string(), l.end.to_string()))
            .collect();
        let is_key = |l: &Ln, table: &str, key: &str| {
            l.table == table && matches!(&l.kind, Kind::Key { key: k } if k == key)
        };
        // every place an accent can already live: `[ui] accent`, root `ui.accent`
        let hits: Vec<usize> = ls
            .iter()
            .enumerate()
            .filter(|(_, l)| is_key(l, "ui", "accent") || is_key(l, "", "ui.accent"))
            .map(|(i, _)| i)
            .collect();
        if !hits.is_empty() {
            for i in hits {
                v[i].0 = replace_value(&v[i].0, &value);
            }
            return Ok(join(&v));
        }
        if ls.iter().any(|l| is_key(l, "", "ui")) {
            return Err(
                "its [ui] is an inline table (ui = { … }) — set accent there by hand".into(),
            );
        }
        let header = ls
            .iter()
            .position(|l| matches!(&l.kind, Kind::Header { name, array: false } if name == "ui"));
        let fix_end = |v: &mut Vec<(String, String)>, i: usize| {
            if v[i].1.is_empty() {
                v[i].1 = nl.to_string();
            }
        };
        if let Some(h) = header {
            fix_end(&mut v, h);
            v.insert(h + 1, (format!("accent = {value}"), nl.to_string()));
            return Ok(join(&v));
        }
        // root dotted keys (`ui.theme = …`) define the table already
        if let Some(last) = ls.iter().rposition(|l| {
            l.table.is_empty() && matches!(&l.kind, Kind::Key { key } if key.starts_with("ui."))
        }) {
            let mut j = last + 1;
            while j < ls.len() && ls[j].kind == Kind::Cont {
                j += 1;
            }
            fix_end(&mut v, j - 1);
            v.insert(j, (format!("ui.accent = {value}"), nl.to_string()));
            return Ok(join(&v));
        }
        let block = [
            ("[ui]".to_string(), nl.to_string()),
            (format!("accent = {value}"), nl.to_string()),
        ];
        // keep [ui] ahead of its sub-tables ([ui.sidebar], …)
        if let Some(sub) = ls
            .iter()
            .position(|l| matches!(&l.kind, Kind::Header { name, .. } if name.starts_with("ui.")))
        {
            let mut at = sub;
            // comments directly above the sub-table belong to it
            while at > 0 && ls[at - 1].kind == Kind::Blank && !ls[at - 1].text.trim().is_empty() {
                at -= 1;
            }
            let mut ins = block.to_vec();
            ins.push((String::new(), nl.to_string()));
            v.splice(at..at, ins);
            return Ok(join(&v));
        }
        if let Some(last) = v.last_mut() {
            if last.1.is_empty() {
                last.1 = nl.to_string();
            }
        }
        if v.iter().any(|(t, _)| !t.trim().is_empty()) {
            v.push((String::new(), nl.to_string()));
        }
        v.extend(block);
        Ok(join(&v))
    }

    /// Install (or refresh) our managed block. `block` uses `\n`; it's
    /// converted to the file's line endings. Err = the unmanaged table or key
    /// that would conflict with it.
    pub fn set_block(src: &str, block: &str) -> Result<String, String> {
        let nl = eol(src);
        let ls = lines(src);
        // our block, or the one written before the rename (claude-herdr)
        let is_begin = |t: &str| t == BEGIN || t == crate::brand::LEGACY_RC_BEGIN;
        let is_end = |t: &str| t == END || t == crate::brand::LEGACY_RC_END;
        let begin = ls.iter().position(|l| is_begin(l.text.trim()));
        let end = begin.and_then(|b| {
            ls.iter()
                .skip(b)
                .position(|l| is_end(l.text.trim()))
                .map(|e| e + b)
        });
        let inside = |i: usize| matches!((begin, end), (Some(b), Some(e)) if i >= b && i <= e);
        for (i, l) in ls.iter().enumerate() {
            if inside(i) {
                continue;
            }
            let hit = match &l.kind {
                Kind::Header { name, .. } => {
                    name == "ui.sidebar.agents" || name.starts_with("ui.sidebar.agents.")
                }
                Kind::Key { key } => {
                    let full = if l.table.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{key}", l.table)
                    };
                    full == "ui.sidebar.agents" || full.starts_with("ui.sidebar.agents.")
                }
                _ => false,
            };
            if hit {
                return Err(match &l.kind {
                    Kind::Header { name, .. } => format!("[{name}]"),
                    _ => format!("`{}`", l.text.trim()),
                });
            }
        }
        let body: String = block.lines().map(|l| format!("{l}{nl}")).collect();
        let mut v: Vec<(String, String)> = ls
            .iter()
            .map(|l| (l.text.to_string(), l.end.to_string()))
            .collect();
        if let (Some(b), Some(e)) = (begin, end) {
            let tail = v[e].1.clone();
            let mut s = join(&v[..b]);
            s.push_str(&body);
            if tail.is_empty() {
                // the old block ended the file without a newline; keep it that way
                s.truncate(s.len() - nl.len());
            }
            s.push_str(&join(&v[e + 1..]));
            return Ok(s);
        }
        if let Some(last) = v.last_mut() {
            if last.1.is_empty() {
                last.1 = nl.to_string();
            }
        }
        let mut s = join(&v);
        if !s.trim().is_empty() {
            s.push_str(nl);
        }
        s.push_str(&body);
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::toml::*;
    use super::*;

    fn count_ui(s: &str) -> usize {
        lines(s)
            .iter()
            .filter(|l| matches!(&l.kind, Kind::Header { name, array: false } if name == "ui"))
            .count()
    }

    #[test]
    fn accent_empty_and_missing_table() {
        assert_eq!(
            set_ui_accent("", "#89b4fa").unwrap(),
            "[ui]\naccent = \"#89b4fa\"\n"
        );
        let src = "# my herdr\n[keys]\nprefix = \"ctrl+b\"";
        let out = set_ui_accent(src, "#89b4fa").unwrap();
        assert_eq!(
            out,
            "# my herdr\n[keys]\nprefix = \"ctrl+b\"\n\n[ui]\naccent = \"#89b4fa\"\n"
        );
        // idempotent, and never a second [ui]
        assert_eq!(set_ui_accent(&out, "#89b4fa").unwrap(), out);
        let again = set_ui_accent(&out, "#cba6f7").unwrap();
        assert_eq!(count_ui(&again), 1);
        assert!(again.contains("accent = \"#cba6f7\"") && !again.contains("89b4fa"));
    }

    #[test]
    fn accent_existing_table_and_key() {
        let src = "[ui]\ntheme = \"dark\"   # keep me\n\n[keys]\nx = 1\n";
        assert_eq!(
            set_ui_accent(src, "#a6e3a1").unwrap(),
            "[ui]\naccent = \"#a6e3a1\"\ntheme = \"dark\"   # keep me\n\n[keys]\nx = 1\n"
        );
        let src = "[ui]  # appearance\naccent   =  \"#000000\"  # brand\nfoo = 1\n";
        assert_eq!(
            set_ui_accent(src, "#a6e3a1").unwrap(),
            "[ui]  # appearance\naccent   =  \"#a6e3a1\"  # brand\nfoo = 1\n"
        );
        // literal strings, quoted keys, bare (invalid) values
        assert_eq!(
            set_ui_accent("[ui]\n\"accent\" = '#000'\n", "#111111").unwrap(),
            "[ui]\n\"accent\" = \"#111111\"\n"
        );
        assert_eq!(
            set_ui_accent("[ui]\naccent = red # x\n", "#111111").unwrap(),
            "[ui]\naccent = \"#111111\" # x\n"
        );
        // `[ ui ]` with spaces is the same table
        let out = set_ui_accent("[ ui ]\nfoo = 1", "#111111").unwrap();
        assert_eq!(out, "[ ui ]\naccent = \"#111111\"\nfoo = 1");
        assert_eq!(count_ui(&out), 1);
        // header without a trailing newline
        assert_eq!(
            set_ui_accent("[ui]", "#111111").unwrap(),
            "[ui]\naccent = \"#111111\"\n"
        );
    }

    #[test]
    fn accent_ignores_other_tables_and_values() {
        // accent in another table is not ours
        let src = "[theme.custom]\naccent = \"#ffffff\"\n";
        let out = set_ui_accent(src, "#111111").unwrap();
        assert!(out.starts_with(src) && out.ends_with("[ui]\naccent = \"#111111\"\n"));
        // a multi-line array whose lines start with `[` isn't a table header
        let src = "[ui.sidebar.agents]\nrows = [\n  [\"state_icon\"],\n  [\"ui\"],\n]\n";
        let out = set_ui_accent(src, "#111111").unwrap();
        assert_eq!(
            out,
            format!("[ui]\naccent = \"#111111\"\n\n{src}"),
            "[ui] goes before its sub-tables"
        );
        // ...nor is `[ui]` inside a multi-line string
        let src = "[help]\ntext = \"\"\"\n[ui]\naccent = \"#000\"\n\"\"\"\n";
        let out = set_ui_accent(src, "#111111").unwrap();
        assert!(out.starts_with(src), "multi-line string left alone: {out}");
        assert!(out.ends_with("\n[ui]\naccent = \"#111111\"\n"));
        let src = "[help]\ntext = '''\n[ui]\n'''\n";
        assert!(set_ui_accent(src, "#111111").unwrap().starts_with(src));
        // `#` inside a string isn't a comment; brackets in strings don't nest
        let src = "[ui]\nlabel = \"a # b [\"\naccent = \"#000000\"\n[x]\n";
        assert_eq!(
            set_ui_accent(src, "#111111").unwrap(),
            "[ui]\nlabel = \"a # b [\"\naccent = \"#111111\"\n[x]\n"
        );
    }

    #[test]
    fn accent_sub_tables_dotted_and_inline() {
        // comments directly above a sub-table stay with it
        let src = "a = 1\n\n# sidebar\n[ui.sidebar]\nwidth = 30\n";
        assert_eq!(
            set_ui_accent(src, "#111111").unwrap(),
            "a = 1\n\n[ui]\naccent = \"#111111\"\n\n# sidebar\n[ui.sidebar]\nwidth = 30\n"
        );
        // root dotted keys already define [ui]
        let src = "ui.theme = \"dark\"\n[keys]\n";
        assert_eq!(
            set_ui_accent(src, "#111111").unwrap(),
            "ui.theme = \"dark\"\nui.accent = \"#111111\"\n[keys]\n"
        );
        let src = "ui.accent = \"#000\"\n";
        assert_eq!(
            set_ui_accent(src, "#111111").unwrap(),
            "ui.accent = \"#111111\"\n"
        );
        assert!(set_ui_accent("ui = { accent = \"#000\" }\n", "#111111").is_err());
        // [[ui]] (array of tables) is not [ui]
        let ls = lines("[[ui]]\n");
        assert_eq!(
            ls[0].kind,
            Kind::Header {
                name: "ui".into(),
                array: true
            }
        );
    }

    #[test]
    fn accent_keeps_crlf_and_comments() {
        let src = "# herdr\r\n[keys]\r\nprefix = \"ctrl+b\" # yes\r\n";
        let out = set_ui_accent(src, "#111111").unwrap();
        assert_eq!(
            out,
            "# herdr\r\n[keys]\r\nprefix = \"ctrl+b\" # yes\r\n\r\n[ui]\r\naccent = \"#111111\"\r\n"
        );
        let src = "[ui]\r\nfoo = 1\r\n";
        assert_eq!(
            set_ui_accent(src, "#111111").unwrap(),
            "[ui]\r\naccent = \"#111111\"\r\nfoo = 1\r\n"
        );
        assert_eq!(eol("a\nb\n"), "\n");
        assert_eq!(eol("a\r\nb\r\n"), "\r\n");
    }

    #[test]
    fn line_scanner() {
        let ls = lines("x = 1\n[a . \"b\"] # c\nk = [\n 1,\n]\ny = 2");
        assert_eq!(ls[0].kind, Kind::Key { key: "x".into() });
        assert_eq!(ls[0].table, "");
        assert_eq!(
            ls[1].kind,
            Kind::Header {
                name: "a.b".into(),
                array: false
            }
        );
        assert_eq!(ls[3].kind, Kind::Cont);
        assert_eq!(ls[4].kind, Kind::Cont);
        assert_eq!(
            (&ls[5].kind, ls[5].table.as_str(), ls[5].end),
            (&Kind::Key { key: "y".into() }, "a.b", "")
        );
    }

    #[test]
    fn sidebar_block_insert_replace_conflict() {
        let m = vec![("b31".to_string(), "#89b4fa".to_string())];
        let block = sidebar_block(&m);
        assert!(block.starts_with(BEGIN) && block.trim_end().ends_with(END));
        assert!(block.contains("{ equals = \"b31\", fg = \"#89b4fa\" }"));
        assert!(block.contains("{ equals = \"◆\", fg = \"#fab387\" }"));
        assert!(block.contains("[\"terminal_title_stripped\"]"));
        assert!(block.contains("claude = ["));
        // the generated rows parse as one table each with our scanner
        let ls = lines(&block);
        let headers: Vec<&Kind> = ls
            .iter()
            .map(|l| &l.kind)
            .filter(|k| matches!(k, Kind::Header { .. }))
            .collect();
        assert_eq!(headers.len(), 2);

        let src = "[ui]\naccent = \"#000\"\n";
        let out = set_block(src, &block).unwrap();
        assert_eq!(out, format!("{src}\n{block}"));
        assert_eq!(set_block(&out, &block).unwrap(), out, "idempotent");
        // refresh in place, keeping what follows
        let old = format!("{src}\n{BEGIN}\nold\n{END}\n[keys]\nx = 1\n");
        let out = set_block(&old, &block).unwrap();
        assert_eq!(out, format!("{src}\n{block}[keys]\nx = 1\n"));
        // CRLF files get CRLF
        let out = set_block("a = 1\r\n", &block).unwrap();
        assert!(out.starts_with("a = 1\r\n\r\n# >>> aleph >>>\r\n"));
        assert!(!out.replace("\r\n", "").contains('\n'));
        // unmanaged sidebar tables or dotted keys → conflict
        assert!(set_block("[ui.sidebar.agents]\nrows = []\n", &block).is_err());
        assert!(set_block("[ui.sidebar.agents.rows_by_agent]\n", &block).is_err());
        assert!(set_block("[ui.sidebar]\nagents.rows = []\n", &block).is_err());
        assert!(set_block("[ui]\nsidebar.agents = {}\n", &block).is_err());
        assert!(set_block("[ui.sidebar]\nwidth = 3\n", &block).is_ok());
        assert_eq!(set_block("", &block).unwrap(), block);
    }

    #[test]
    fn accent_unicode_and_odd_inputs() {
        // multi-byte text in strings, comments and multi-line strings never panics
        let src = "[ui]\nname = \"★ ünï\" # ✻ ✶\nmsg = \"\"\"\n◆ ● \\\" ▲\n\"\"\"\naccent = '●'\n";
        let out = set_ui_accent(src, "#111111").unwrap();
        assert!(out.contains("accent = \"#111111\"") && out.contains("★ ünï\" # ✻ ✶"));
        // two (invalid) [ui] tables: edit, never add a third
        let src = "[ui]\na = 1\n[ui]\nb = 2\n";
        assert_eq!(count_ui(&set_ui_accent(src, "#111111").unwrap()), 2);
        // whitespace-only file
        assert_eq!(
            set_ui_accent("\n\n", "#111111").unwrap(),
            "\n\n[ui]\naccent = \"#111111\"\n"
        );
        // repeated machine labels give one rule each
        let rows = sidebar_rows(&[("b31".into(), "#1".into()), ("b31".into(), "#2".into())]);
        assert_eq!(rows.matches("equals = \"b31\"").count(), 1);
    }

    #[test]
    fn stamps() {
        assert_eq!(stamp(0), "19700101000000");
        assert_eq!(stamp(1_700_000_000), "20231114221320");
        assert_eq!(stamp(951_782_400), "20000229000000");
    }
}
