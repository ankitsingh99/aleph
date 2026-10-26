//! The pause: the panic key (`showcase.panic_key`, F12 by default) or
//! `aleph showcase panic [on|off]` blanks every window at once, and the
//! same again brings them back. It's a field in `state/showcase.json`
//! (`panic`), with the epoch moved on, so every process sees it within half
//! a second:
//!
//! * each dashboard (and the folder browser, the launch card, setup) draws
//!   only a starfield and the ℵ, and swallows every key but the panic key;
//! * the daemon answers — socket, HTTP, SSE — carry nothing real: every
//!   list empty, every word blank, every number 0, with a marker saying
//!   it's paused, so Raycast, the menu bar and the web console show their
//!   "be right back" cards;
//! * desktop banners wait (the phone's sinks don't).
//!
//! It works in every mode, showcase on or off, and changes nothing real:
//! sessions go on working, events are kept, nothing is sent or closed.

use super::super::state;
use crate::config::Config;
use crate::ui::{self, c};
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use std::time::Instant;

/// Is the pause on?
pub fn on() -> bool {
    state::panic()
}

/// Pause (or resume) every window; the menu bar is told at once when it
/// listens for that (`notify.raycast`).
pub fn set(on: bool) -> std::io::Result<()> {
    if cfg!(test) {
        // (a test pressing the panic key never touches the real state dir)
        return Err(std::io::Error::other("tests don't write the state file"));
    }
    state::update(|m: &mut Map<String, Value>| {
        m.insert("panic".into(), json!(on));
    })?;
    refresh_menu_bar();
    Ok(())
}

/// The panic key was pressed: the other way round.
pub fn toggle() {
    let _ = set(!on());
}

/// Raycast's menu bar draws again now rather than within 30 s — only where
/// it's asked to refresh on events (`notify.raycast`), off the caller's thread.
fn refresh_menu_bar() {
    if !cfg!(target_os = "macos") || cfg!(test) {
        return;
    }
    std::thread::spawn(|| {
        let cfg = Config::load();
        if !cfg.flag("notify.raycast") {
            return;
        }
        let link = cfg.get_or("notify.raycast.deeplink");
        if !link.is_empty() {
            let _ = crate::proc::cap("open", &["-g", &link], std::time::Duration::from_secs(5));
        }
    });
}

// ------------------------------------------------------------------ answers

/// What Aleph itself asks the daemon (its pid and status, a refresh,
/// tokens): answered as ever while paused, so nothing that manages the
/// daemon is misled (a pid of 0 would be everyone's).
pub fn internal(method: &str) -> bool {
    method.starts_with("daemon.") || method.starts_with("token.") || method == "hosts.refresh"
}

/// Fields that keep their words while paused: what a client needs to read
/// the answer at all (kinds, scopes, versions), and showcase's own markers.
fn keeps(k: &str) -> bool {
    matches!(
        k,
        "kind" | "type" | "scope" | "version" | "api" | "method" | "unit" | "showcase"
    )
}

/// An answer with nothing real left in it, its shape kept: lists empty,
/// strings blank, numbers 0 (flags and nulls as they were).
pub fn neutral(v: Value) -> Value {
    walk("", v)
}

fn walk(key: &str, v: Value) -> Value {
    // showcase's own marker, and an error (a client must still read it)
    if key == "showcase_safety" || key == "error" {
        return v;
    }
    match v {
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, x)| {
                    let x = walk(&k, x);
                    (k, x)
                })
                .collect(),
        ),
        Value::Array(_) => Value::Array(Vec::new()),
        Value::String(s) if keeps(key) => Value::String(s),
        Value::String(_) => Value::String(String::new()),
        Value::Number(n) if n.is_f64() => json!(0.0),
        Value::Number(_) => json!(0),
        other => other,
    }
}

// ------------------------------------------------------------------ the stars

/// A star's place: a well-mixed hash of the cell.
fn hash(x: u16, y: u16) -> u64 {
    let mut z = ((x as u64) << 32 | y as u64).wrapping_add(0x51_7cc1_b727_220a);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

const STARS: [&str; 5] = ["·", "✦", "˚", "⋆", "∙"];

/// Milliseconds since the first paused frame (the twinkle's clock).
fn clock() -> u128 {
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis()
}

/// The frame, paused: nothing of it left, only stars, the ℵ and "be right
/// back" (and, faint at the bottom, how to come back). Foreground only.
pub fn blank(buf: &mut Buffer, keys: &[String]) {
    draw(buf, keys, clock());
}

pub fn draw(buf: &mut Buffer, keys: &[String], ms: u128) {
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
    let tick = (ms / 700) as u64;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let h = hash(x, y);
            if !h.is_multiple_of(37) {
                continue;
            }
            let glyph = STARS[((h >> 8) % STARS.len() as u64) as usize];
            // most stars faint; now and then one burns gold for a moment
            let st = if c::mono() {
                Style::new().add_modifier(Modifier::DIM)
            } else if (tick + (h >> 20)).is_multiple_of(13) {
                ui::fg(c::ACCENT())
            } else if (h >> 12).is_multiple_of(5) {
                ui::fg(c::MUTED())
            } else {
                ui::fg(c::FAINT())
            };
            buf[(x, y)].set_symbol(glyph).set_style(st);
        }
    }
    if area.height < 3 || area.width < 16 {
        return;
    }
    let mid = area.y + area.height / 2;
    let centre = |buf: &mut Buffer, y: u16, l: Line| {
        let w = ui::line_width(&l) as u16;
        if w <= area.width {
            let x = area.x + (area.width - w) / 2;
            // a clear stretch around the words, so no star sits in them
            for cx in x.saturating_sub(2)..(x + w + 2).min(area.right()) {
                buf[(cx, y)].reset();
            }
            buf.set_line(x, y, &l, w);
        }
    };
    let mark = if c::mono() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD)
    };
    centre(
        buf,
        mid.saturating_sub(1),
        Line::from(Span::styled(crate::brand::MARK, mark)),
    );
    centre(
        buf,
        mid + 1,
        Line::from(Span::styled(
            "be right back",
            ui::muted().add_modifier(Modifier::ITALIC),
        )),
    );
    if area.height >= 8 {
        let how = match keys.first() {
            Some(k) => format!("paused · {} brings it back", pretty(k)),
            None => "paused · aleph showcase panic off brings it back".to_string(),
        };
        centre(
            buf,
            area.bottom() - 1,
            Line::from(Span::styled(how, ui::faint())),
        );
    }
}

/// A key as the pause names it: `F12`, `⌃4`, `⌥P`.
pub fn pretty(k: &str) -> String {
    let (mut pre, mut rest) = (String::new(), k);
    loop {
        if let Some(r) = rest.strip_prefix("ctrl+") {
            pre.push('⌃');
            rest = r;
        } else if let Some(r) = rest.strip_prefix("alt+") {
            pre.push('⌥');
            rest = r;
        } else {
            break;
        }
    }
    let fkey =
        rest.len() > 1 && rest.starts_with('f') && rest[1..].chars().all(|c| c.is_ascii_digit());
    let base = if fkey || !pre.is_empty() {
        rest.to_uppercase()
    } else {
        rest.to_string()
    };
    pre + &base
}

/// `aleph showcase panic [on|off|status]` (none: the other way round).
pub fn cmd(arg: Option<&str>) -> std::process::ExitCode {
    use crate::out::{err, hint, ok};
    let to = match arg.unwrap_or("toggle") {
        "on" => true,
        "off" => false,
        "toggle" => !on(),
        "status" => {
            if on() {
                ok("paused: every window shows the stars");
            } else {
                ok("not paused");
            }
            return std::process::ExitCode::SUCCESS;
        }
        other => {
            err(&format!(
                "unknown: aleph showcase panic {other} (on | off | status; none: the other way round)"
            ));
            return std::process::ExitCode::from(2);
        }
    };
    match set(to) {
        Ok(()) if to => {
            ok("paused: every window shows the stars — dashboards, Raycast, the menu bar, the web console");
            let keys = super::settings().panic_keys.clone();
            hint(&match keys.first() {
                Some(k) => {
                    format!("{k} in a dashboard, or aleph showcase panic off, brings them back")
                }
                None => "aleph showcase panic off brings them back".to_string(),
            });
            std::process::ExitCode::SUCCESS
        }
        Ok(()) => {
            ok("back: every window shows what it did");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            err(&format!("couldn't switch the pause: {e}"));
            std::process::ExitCode::from(1)
        }
    }
}
