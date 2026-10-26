//! Showcase mode's capture and share, each part optional and set under
//! `showcase.*`:
//!
//! * **recordings** (`cast.rs`) — the palette's "Showcase: record" or the
//!   record key (`showcase.record_key`, ctrl+r) starts and stops an
//!   asciinema cast of the dashboard: the bytes as drawn, after the lens.
//!   A `● rec 0:42` chip sits in the header meanwhile; hand-offs (herdr, the
//!   folder browser, launch cards, prompts) pause it or stop it
//!   (`showcase.cast_handoff`). Stopping renders it as an MP4 and a GIF in
//!   the background (`video.rs`, drawn by `raster.rs`; `showcase.video`,
//!   `showcase.video_frame`), the .cast kept only with `showcase.cast_keep`;
//!   the dashboard says where they went (`showcase.cast_dir`). `aleph
//!   showcase casts` lists them, `aleph showcase render` makes them again.
//! * **posters** (`poster.rs`) — the palette's "Showcase: poster" saves the
//!   screen as an SVG card (and a PNG when a converter is already here);
//!   `aleph showcase snap` draws a view offscreen for one (`tui/snap.rs`).
//! * **your watermark** — `showcase.watermark`, set in the rule under the
//!   tabs (`showcase.watermark_on`: tui, raycast, web), never masked.
//! * **keycast names** (`names.rs`) — what a key did, beside its keycap.
//!
//! Nothing is captured while showcase mode is off (the screen would be the
//! real one) unless `showcase.capture_real` says so; a recording stops if
//! it goes off.

pub mod cast;
pub mod cmd;
pub mod names;
pub mod poster;
pub mod raster;
pub mod video;

use crate::config::Config;
use crate::ui::{self, c};
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// A recording runs (the writer's fast path).
static ON: AtomicBool = AtomicBool::new(false);
static REC: Mutex<Option<cast::Recorder>> = Mutex::new(None);
/// What to tell the dashboard: (ok, words). Toasts that stay up a while:
/// a recording's or a poster's path is there to be read.
static NOTES: Mutex<VecDeque<(bool, String)>> = Mutex::new(VecDeque::new());
/// Warnings to hold up (no ffmpeg here: only the .cast was saved).
static ALERTS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// The palette asked for a poster of the next frame.
static POSTER: AtomicBool = AtomicBool::new(false);

fn rec() -> MutexGuard<'static, Option<cast::Recorder>> {
    REC.lock().unwrap_or_else(|e| e.into_inner())
}

fn say(ok: bool, m: impl Into<String>) {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back((ok, m.into()));
    while q.len() > 4 {
        q.pop_front();
    }
}

/// The next thing to tell the dashboard (a render that finished since is
/// one).
pub fn note() -> Option<(bool, String)> {
    for (ok, m) in video::poll() {
        say(ok, m);
    }
    NOTES.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
}

fn alert(m: String) {
    let mut q = ALERTS.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(m);
    while q.len() > 2 {
        q.pop_front();
    }
}

/// The warnings to hold up, each handed to `f` (the dashboard's alert).
pub fn alerts(mut f: impl FnMut(String)) {
    let v: Vec<String> = ALERTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect();
    for m in v {
        f(m);
    }
}

/// Showcase's dress-up stands aside for a frame (a poster's): no keys just
/// pressed, no caption.
pub fn hush() {
    super::flair::hush();
}

// ------------------------------------------------------------------ where

/// A folder setting, `~` expanded. Left as it came on Linux, the desktop's
/// own folder (`XDG_VIDEOS_DIR`, `XDG_PICTURES_DIR`) with `Aleph` in it.
pub fn dir(cfg: &Config, key: &str) -> PathBuf {
    // unit tests never write into the real Movies or Pictures
    if cfg!(test) && cfg.get(key).is_none() {
        return std::env::temp_dir()
            .join(format!("aleph-test-{}", std::process::id()))
            .join(key);
    }
    if cfg!(target_os = "linux") && cfg.get(key).is_none() {
        let what = if key.ends_with("cast_dir") {
            "VIDEOS"
        } else {
            "PICTURES"
        };
        if let Some(p) = xdg_user_dir(what) {
            return p.join("Aleph");
        }
    }
    crate::config::expand(cfg.get_or(key).trim())
}

/// `XDG_<WHAT>_DIR`: the environment, else `~/.config/user-dirs.dirs`
/// (`XDG_VIDEOS_DIR="$HOME/Videos"`). None for the home folder itself.
fn xdg_user_dir(what: &str) -> Option<PathBuf> {
    let var = format!("XDG_{what}_DIR");
    let raw = std::env::var(&var).ok().or_else(|| {
        let cfgdir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::config::home().join(".config"));
        let t = std::fs::read_to_string(cfgdir.join("user-dirs.dirs")).ok()?;
        user_dirs(&t, &var)
    })?;
    let home = crate::config::home();
    let p = match raw.strip_prefix("$HOME") {
        Some(r) => home.join(r.trim_start_matches('/')),
        None => PathBuf::from(&raw),
    };
    (p.is_absolute() && p != home).then_some(p)
}

/// `KEY="value"` from a user-dirs.dirs file.
fn user_dirs(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.trim().split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// A path as a toast or a list shows it: under the home folder without the
/// `~/` (so the lens has no home path to take for a private one), else whole.
pub fn shown(p: &Path) -> String {
    match p.strip_prefix(crate::config::home()) {
        Ok(r) if !r.as_os_str().is_empty() => r.display().to_string(),
        _ => p.display().to_string(),
    }
}

/// Local time: (year, month, day, hour, minute, second).
fn local(t: u64) -> (i64, u32, u32, i64, i64, i64) {
    let s = t as i64 + ui::utc_offset();
    let (y, m, d) = ui::civil(s.div_euclid(86_400));
    let r = s.rem_euclid(86_400);
    (y, m, d, r / 3600, r % 3600 / 60, r % 60)
}

/// `20261023-140512`, for file names.
pub fn file_stamp(t: u64) -> String {
    let (y, m, d, h, mi, s) = local(t);
    format!("{y}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// `2026-10-23 14:05`.
pub fn when(t: u64) -> String {
    let (y, m, d, h, mi, _) = local(t);
    format!("{y}-{m:02}-{d:02} {h:02}:{mi:02}")
}

/// `0:42`, `12:05`, `1:02:03`.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A view's name as a file name's part: `monitor-usage`.
fn slug(view: &str) -> String {
    let s: String = view
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let parts: Vec<&str> = s.split('-').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        "screen".into()
    } else {
        parts.join("-")
    }
}

// ------------------------------------------------------------------ may we

/// May the screen be captured now? While showcase mode masks it — or
/// when `showcase.capture_real` allows the real one.
pub fn allowed(cfg: &Config) -> Result<(), String> {
    if super::on() || cfg.flag("showcase.capture_real") {
        return Ok(());
    }
    Err("showcase is off: a capture would show your real names and folders — switch to masked or the demo world first (or set showcase.capture_real)".into())
}

// ------------------------------------------------------------------ recording

pub fn tee(b: &[u8]) {
    if ON.load(Ordering::Relaxed) {
        if let Some(r) = rec().as_mut() {
            r.bytes(b);
        }
    }
}

pub fn tee_flush() {
    if ON.load(Ordering::Relaxed) {
        if let Some(r) = rec().as_mut() {
            r.flush();
        }
    }
}

/// How long the one running has gone, and where it goes.
pub fn recording() -> Option<(u64, PathBuf)> {
    if !ON.load(Ordering::Relaxed) {
        return None;
    }
    rec().as_ref().map(|r| (r.now() as u64, r.path.clone()))
}

/// The player's theme: the dashboard's own colours.
fn theme() -> (String, String, Vec<String>) {
    let ink = poster::Ink::now();
    (poster::hex(ink.text), poster::hex(ink.base), ink.palette())
}

/// Start one (in `showcase.cast_dir`): what to say.
pub fn start(cfg: &Config) -> Result<String, String> {
    allowed(cfg)?;
    if ON.load(Ordering::Relaxed) {
        return Err("a recording is running already".into());
    }
    let d = dir(cfg, "showcase.cast_dir");
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", shown(&d)))?;
    let path = d.join(format!("aleph-{}.cast", file_stamp(crate::cache::now())));
    let wm = cfg.get_or("showcase.watermark");
    let title = match wm.trim() {
        "" => format!("{} — {}", crate::brand::NAME, super::mode_name()),
        w => format!("{} — {w}", crate::brand::NAME),
    };
    let opts = cast::Opts {
        title,
        idle: cfg.num("showcase.cast_idle") as f64,
        theme: Some(theme()),
        // for its video: the mode (the card's chip) and the accent
        env: vec![
            ("ALEPH_SHOWCASE".into(), super::mode_name().into()),
            (
                "ALEPH_ACCENT".into(),
                poster::hex(poster::Ink::now().accent),
            ),
        ],
    };
    let r = cast::Recorder::create(&path, opts).map_err(|e| format!("{}: {e}", shown(&path)))?;
    *rec() = Some(r);
    ON.store(true, Ordering::Relaxed);
    let key = cfg.get_or("showcase.record_key");
    let stop = match key.trim() {
        "" | "off" | "none" => "the palette stops it".to_string(),
        k => format!("{k} stops it"),
    };
    Ok(format!("● recording → {} ({stop})", shown(&path)))
}

/// Stop it: what to say (None: none ran). Its MP4 and GIF start rendering
/// in the background; with no ffmpeg here a warning is held up (or, on the
/// way out, said here: `quitting`).
pub fn stop(cfg: &Config, quitting: bool) -> Option<String> {
    let r = rec().take()?;
    ON.store(false, Ordering::Relaxed);
    Some(match r.finish() {
        Some((p, secs)) => {
            let (m, warn) = video::stopped(cfg, &p, secs, quitting);
            if let Some(w) = warn {
                alert(w);
            }
            m
        }
        None => "the recording stopped before its first frame: nothing saved".into(),
    })
}

/// Start, or stop the one running; what happened goes to the dashboard
/// (a start says nothing while the header's chip shows it).
pub fn toggle(cfg: &Config) {
    if let Some(m) = stop(cfg, false) {
        say(true, m);
        return;
    }
    match start(cfg) {
        Ok(m) if !cfg.flag("showcase.cast_chip") => say(true, m),
        Ok(_) => {}
        Err(e) => say(false, e),
    }
}

/// Is `k` the record key (`showcase.record_key`, a comma list)?
pub fn is_record_key(cfg: &Config, k: &str) -> bool {
    let want = cfg.get_or("showcase.record_key");
    !k.is_empty()
        && want
            .split(',')
            .map(str::trim)
            .any(|w| w == k && !matches!(w, "off" | "none"))
}

pub fn key(cfg: &Config, k: &str) -> bool {
    if !is_record_key(cfg, k) {
        return false;
    }
    toggle(cfg);
    true
}

pub fn away(cfg: &Config, quit: bool) -> Option<String> {
    if !ON.load(Ordering::Relaxed) {
        return None;
    }
    if quit {
        return stop(cfg, true);
    }
    if cfg.get_or("showcase.cast_handoff") == "stop" {
        // said once the dashboard is back
        if let Some(m) = stop(cfg, false) {
            say(true, m);
        }
        return None;
    }
    if let Some(r) = rec().as_mut() {
        r.pause("away");
    }
    None
}

pub fn back() {
    if ON.load(Ordering::Relaxed) {
        if let Some(r) = rec().as_mut() {
            r.resume();
        }
    }
}

/// `● rec 0:42` after the wordmark (and its chip), the dot beating.
pub fn rec_chip(cfg: &Config, left: &mut Vec<Span<'static>>) {
    if !cfg.flag("showcase.cast_chip") || allowed(cfg).is_err() {
        return;
    }
    let Some((secs, _)) = recording() else {
        return;
    };
    let beat = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() / 600 % 2 == 0)
        .unwrap_or(true);
    let (dot, txt) = if c::mono() {
        let s = Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD);
        (s, s)
    } else {
        let bg = Style::new().bg(c::SEL());
        (
            bg.fg(if beat { c::RED() } else { c::FAINT() }),
            bg.fg(c::TEXT()).add_modifier(Modifier::BOLD),
        )
    };
    let at = left.len().saturating_sub(1).max(1).min(left.len());
    left.insert(at, Span::styled(format!("rec {} ", clock(secs)), txt));
    left.insert(at, Span::styled(" ● ", dot));
    left.insert(at, Span::raw("  "));
}

// ------------------------------------------------------------------ the frame

pub fn finish(cfg: &Config, buf: &mut Buffer, view: &str) {
    // (safety: while paused, nothing over the stars — and a recording or a
    // poster takes the stars)
    if super::on() && !super::paused() {
        // the keycast staged during the draw, into the masked frame
        super::flair::lay_keys(buf);
        ribbon(cfg, buf);
    }
    if ON.load(Ordering::Relaxed) {
        if allowed(cfg).is_err() {
            // showcase went off: the real screen isn't recorded
            if let Some(m) = stop(cfg, false) {
                say(true, format!("{m} (showcase went off)"));
            }
        } else if let Some(r) = rec().as_mut() {
            r.frame(buf);
        }
    }
    if POSTER.swap(false, Ordering::Relaxed) {
        poster_later(cfg, buf.clone(), view);
    }
}

/// The poster of a frame, written off the UI thread (its PNG can take a
/// second); what came of it goes to the dashboard.
fn poster_later(cfg: &Config, buf: Buffer, view: &str) {
    let look = look(cfg, view, None);
    let d = dir(cfg, "showcase.poster_dir");
    let how = cfg.get_or("showcase.poster_png");
    std::thread::spawn(move || match save_poster(&buf, &look, &d, &how) {
        Ok((svg, png)) => say(true, saved(&svg, &png)),
        Err(e) => say(false, format!("poster: {e}")),
    });
}

/// What a poster's card says: the view, the mode, your watermark, the time.
pub fn look(cfg: &Config, view: &str, frame: Option<poster::Frame>) -> poster::Look {
    poster::Look {
        frame: frame.unwrap_or_else(|| {
            poster::Frame::parse(&cfg.get_or("showcase.poster_frame"))
                .unwrap_or(poster::Frame::Cockpit)
        }),
        view: view.to_string(),
        mode: super::mode_name().to_string(),
        watermark: mark_text(cfg),
        when: when(crate::cache::now()),
    }
}

/// The SVG (and maybe a PNG) of `buf` in `d`: their paths, or what went
/// wrong with the PNG.
pub fn save_poster(
    buf: &Buffer,
    look: &poster::Look,
    d: &Path,
    how: &str,
) -> Result<(PathBuf, poster::Png), String> {
    let name = format!(
        "aleph-{}-{}.svg",
        slug(&look.view),
        file_stamp(crate::cache::now())
    );
    std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", shown(d)))?;
    save_poster_at(buf, look, &d.join(name), how)
}

pub fn save_poster_at(
    buf: &Buffer,
    look: &poster::Look,
    path: &Path,
    how: &str,
) -> Result<(PathBuf, poster::Png), String> {
    let (svg, w, _) = poster::svg(buf, look, &poster::Ink::now());
    crate::config::write_atomic(path, svg.as_bytes())
        .map_err(|e| format!("{}: {e}", shown(path)))?;
    Ok((path.to_path_buf(), poster::png(path, how, w)))
}

/// `poster → Pictures/Aleph/aleph-fleet-….svg (+ PNG, by rsvg-convert)`.
pub fn saved(svg: &Path, png: &poster::Png) -> String {
    let tail = match png {
        Ok(Some((_, tool))) => format!(" + PNG ({tool})"),
        Ok(None) => String::new(),
        Err(e) => format!(" (no PNG: {e})"),
    };
    format!("poster → {}{tail}", shown(svg))
}

// ------------------------------------------------------------------ the palette

pub fn items(cfg: &Config) -> Vec<(&'static str, String, String)> {
    let key = cfg.get_or("showcase.record_key");
    let k = match key.trim() {
        "" | "off" | "none" => String::new(),
        k => format!(" · {k}"),
    };
    let rec = match recording() {
        Some((secs, p)) => (
            "record",
            "Showcase: stop recording".to_string(),
            format!("{} so far → {}{k}", clock(secs), shown(&p)),
        ),
        None => (
            "record",
            "Showcase: record".to_string(),
            format!(
                "this screen as {}, as drawn (masked) → {}{k}",
                match video::outputs(&cfg.get_or("showcase.video")) {
                    (true, true) => "an MP4 and a GIF",
                    (true, false) => "an MP4",
                    (false, true) => "a GIF",
                    _ => "an asciinema cast",
                },
                shown(&dir(cfg, "showcase.cast_dir"))
            ),
        ),
    };
    let png = if cfg.get_or("showcase.poster_png") == "off" {
        ""
    } else {
        " (and a PNG)"
    };
    vec![
        rec,
        (
            "poster",
            "Showcase: poster".to_string(),
            format!(
                "this screen as an SVG card{png} → {}",
                shown(&dir(cfg, "showcase.poster_dir"))
            ),
        ),
    ]
}

pub fn run(cfg: &Config, id: &str) {
    match (id, allowed(cfg)) {
        ("record", _) => toggle(cfg),
        ("poster", Ok(())) => {
            // the next frame, without the keys just pressed or a caption
            hush();
            POSTER.store(true, Ordering::Relaxed);
        }
        ("poster", Err(e)) => say(false, e),
        _ => say(false, format!("no capture {id}")),
    }
}

// ------------------------------------------------------------------ watermark

/// The watermark (control characters out, at most 60 characters), if set.
pub fn mark_text(cfg: &Config) -> Option<String> {
    let w: String = cfg
        .get_or("showcase.watermark")
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect();
    let w = w.trim();
    (!w.is_empty()).then(|| w.to_string())
}

/// The watermark, if it's shown in `place` (tui, raycast, web).
pub fn watermark(cfg: &Config, place: &str) -> Option<String> {
    let w = mark_text(cfg)?;
    let on = cfg.get_or("showcase.watermark_on");
    on.split(',')
        .map(str::trim)
        .any(|x| x.eq_ignore_ascii_case(place) || x.eq_ignore_ascii_case("all"))
        .then_some(w)
}

/// `/api/v1/me`'s `watermark` (the web console's banner shows it).
pub fn stamp(cfg: &Config, v: &mut Value) {
    if let (Some(w), true) = (watermark(cfg, "web"), v.is_object()) {
        v["watermark"] = Value::from(w);
    }
}

/// Is this row the rule under the tabs? It runs across as line, end to end
/// (the active tab's heavy stretch, the HUD's brackets and readouts set
/// into it).
fn is_rule(buf: &Buffer, y: u16) -> bool {
    let a = buf.area;
    let line = |x: u16| matches!(buf[(x, y)].symbol(), "─" | "━" | "┏" | "┓");
    let n = (a.left()..a.right()).filter(|x| line(*x)).count();
    a.width > 0 && line(a.left()) && line(a.right() - 1) && n * 2 >= a.width as usize
}

/// The rule's plain stretches (light line, nothing set into it): (from, to).
fn plain_runs(buf: &Buffer, y: u16) -> Vec<(u16, u16)> {
    let a = buf.area;
    let mut v: Vec<(u16, u16)> = Vec::new();
    for x in a.left()..a.right() {
        let c = &buf[(x, y)];
        if c.symbol() == "─" && c.bg == Color::Reset && !c.skip {
            match v.last_mut() {
                Some((_, to)) if *to == x => *to = x + 1,
                _ => v.push((x, x + 1)),
            }
        }
    }
    v
}

/// Your watermark set into the rule under the tabs, at the right end of its
/// last plain stretch (left of the HUD's readouts when they're there):
/// `──┤ ✦ yours ├──`. Only ever over the rule's own light line — never the
/// active tab's, never what's set into it, never content; drawn after the
/// lens, so it takes no room a name needs. No stretch long enough: none.
pub fn ribbon(cfg: &Config, buf: &mut Buffer) {
    let Some(w) = watermark(cfg, "tui") else {
        return;
    };
    let a = buf.area;
    let Some(y) = (a.top()..a.bottom().min(a.top() + 4)).find(|y| is_rule(buf, *y)) else {
        return;
    };
    let text = format!(" ✦ {w} ");
    let tw = ui::width(&text) as u16 + 2;
    // two of the stretch's cells kept on each side
    let Some((_, to)) = plain_runs(buf, y)
        .into_iter()
        .rev()
        .find(|(from, to)| to - from >= tw + 4)
    else {
        return;
    };
    let x0 = to - 2 - tw;
    let (line, star, words) = if c::mono() {
        (Style::new(), Style::new(), Style::new())
    } else {
        (
            ui::fg(c::LINE()),
            ui::fg(c::ACCENT()),
            ui::muted().add_modifier(Modifier::ITALIC),
        )
    };
    let spans = vec![
        Span::styled("┤", line),
        Span::styled(" ✦ ", star),
        Span::styled(format!("{w} "), words),
        Span::styled("├", line),
    ];
    buf.set_line(x0, y, &Line::from(spans), tw);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn cfg(lines: &[&str]) -> Config {
        Config::in_memory(lines)
    }

    #[test]
    fn folders_times_and_names() {
        assert_eq!(clock(42), "0:42");
        assert_eq!(clock(725), "12:05");
        assert_eq!(clock(3723), "1:02:03");
        assert_eq!(slug("Monitor › Usage"), "monitor-usage");
        assert_eq!(slug("✦"), "screen");
        assert_eq!(file_stamp(0).len(), "19700101-000000".len());
        assert_eq!(
            user_dirs("# x\nXDG_VIDEOS_DIR=\"$HOME/Vidéos\"\n", "XDG_VIDEOS_DIR").as_deref(),
            Some("$HOME/Vidéos")
        );
        assert_eq!(user_dirs("", "XDG_VIDEOS_DIR"), None);
        let home = crate::config::home();
        assert_eq!(
            shown(&home.join("Movies/Aleph/a.cast")),
            "Movies/Aleph/a.cast"
        );
        assert_eq!(shown(Path::new("/tmp/x.cast")), "/tmp/x.cast");
        let c = cfg(&["showcase.cast_dir=/tmp/casts"]);
        assert_eq!(dir(&c, "showcase.cast_dir"), PathBuf::from("/tmp/casts"));
    }

    #[test]
    fn nothing_is_captured_while_showcase_is_off() {
        crate::showcase::force("off");
        let c = cfg(&[]);
        assert!(allowed(&c).is_err());
        assert!(start(&c).is_err());
        assert!(allowed(&cfg(&["showcase.capture_real=true"])).is_ok());
        crate::showcase::force("masked");
        assert!(allowed(&c).is_ok());
        crate::showcase::force("off");
    }

    #[test]
    fn the_record_key_is_the_setting() {
        let c = cfg(&[]);
        assert!(is_record_key(&c, "ctrl+r"));
        assert!(!is_record_key(&c, "r") && !is_record_key(&c, ""));
        let none = cfg(&["showcase.record_key=off"]);
        assert!(!is_record_key(&none, "ctrl+r"));
        let two = cfg(&["showcase.record_key=f9, ctrl+r"]);
        assert!(is_record_key(&two, "f9") && is_record_key(&two, "ctrl+r"));
    }

    #[test]
    fn the_watermark_goes_where_it_is_asked_and_never_over_content() {
        let c = cfg(&["showcase.watermark=  alex.dev\u{7}  "]);
        assert_eq!(mark_text(&c).as_deref(), Some("alex.dev"));
        for p in ["tui", "raycast", "web"] {
            assert!(watermark(&c, p).is_some(), "{p}");
        }
        let only = cfg(&["showcase.watermark=x", "showcase.watermark_on=web"]);
        assert!(watermark(&only, "tui").is_none() && watermark(&only, "web").is_some());
        assert!(watermark(&cfg(&[]), "tui").is_none());
        let mut v = serde_json::json!({"name": "phone"});
        stamp(&c, &mut v);
        assert_eq!(v["watermark"], "alex.dev");

        // the ribbon: in the rule's plain stretch only
        let draw = |rule: &str| {
            let mut t = Terminal::new(TestBackend::new(60, 4)).unwrap();
            t.draw(|f| {
                let b = f.buffer_mut();
                b.set_string(0, 0, "content content", Style::new());
                b.set_string(0, 1, rule, Style::new());
                ribbon(&c, b);
            })
            .unwrap();
            let b = t.backend().buffer().clone();
            (0..60)
                .map(|x| b[(x, 1)].symbol().to_string())
                .collect::<String>()
        };
        let plain = "━━━━━━━━".to_string() + &"─".repeat(52);
        let row = draw(&plain);
        assert!(row.contains("┤ ✦ alex.dev ├"), "{row}");
        assert!(row.starts_with("━━━━━━━━") && row.ends_with("──"), "{row}");
        // something set into the rule at its end (the HUD's readouts): beside
        // it, never over it
        let busy = "─".repeat(30) + "┤ ⟡ 2/2 up ├" + &"─".repeat(17) + "┓";
        let row = draw(&busy);
        assert!(
            row.contains("┤ ⟡ 2/2 up ├") && row.contains("alex.dev"),
            "{row}"
        );
        assert!(row.find("alex").unwrap() < row.find('⟡').unwrap(), "{row}");
        // the active tab's heavy line is never covered
        let heavy = "─".repeat(10) + &"━".repeat(50);
        assert!(!draw(&heavy).contains("alex"));
        // a row of words isn't the rule
        let words = "─".to_string() + &"x".repeat(58) + "─";
        assert!(!draw(&words).contains("alex"));
    }
}
