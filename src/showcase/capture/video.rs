//! A recording as video: stopping one leaves an MP4 and a GIF in
//! `showcase.cast_dir` (`showcase.video`), the .cast kept only when
//! `showcase.cast_keep` says so — or when they couldn't be made.
//!
//! * **replay** — the cast through `vt100` on a fixed 30 fps timeline,
//!   pauses longer than its `idle_time_limit` cut short the way asciinema
//!   plays them, `"r"` resizes honoured; dim text (which vt100 doesn't keep)
//!   is folded into its colour on the way in;
//! * **frames** — each one drawn (`raster.rs`) in `showcase.video_frame`:
//!   `cockpit`, the screen in a card like the poster's at 1920×1080, or
//!   `plain`, the terminal edge to edge at twice the poster's cell size;
//! * **encoding** — by the ffmpeg already on this computer (nothing is
//!   installed): raw RGB on its stdin → H.264 MP4, then the GIF from the MP4
//!   (a palette first, then the frames through it);
//! * **after a stop** — `aleph showcase render <cast> --after-stop` runs on
//!   its own (its own session, a lower priority), so quitting the dashboard
//!   doesn't stop it; a status file says how it went, and the dashboard,
//!   while it's up, says where the files went.

use super::poster::{self, Frame, Ink};
use super::raster::{self, Canvas, Cells, Clip, Fonts, Look, Rgb};
use super::{clock, shown};
use crate::config::Config;
use serde_json::{json, Value};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The MP4's frame rate, and the GIF's.
pub const FPS: u32 = 30;
pub const GIF_FPS: u32 = 15;
/// The GIF's widest.
pub const GIF_WIDTH: u32 = 1280;
/// The last screen held this long at the end.
const HOLD: f64 = 1.0;
/// The cockpit's size.
pub const COCKPIT: (usize, usize) = (1920, 1080);
/// The plain frame's widest, and tallest.
const PLAIN_MAX: (i32, i32) = (3840, 2160);
/// A line's height, per px of font.
const LINE: f32 = 1.25;

/// What's said when there's no ffmpeg.
pub fn no_ffmpeg() -> String {
    let how = if cfg!(target_os = "macos") {
        "brew install ffmpeg"
    } else {
        "install ffmpeg"
    };
    format!("ffmpeg isn't on this computer — only the .cast was saved ({how} for MP4 and GIF)")
}

/// The ffmpeg on PATH.
pub fn ffmpeg() -> Option<PathBuf> {
    crate::proc::which("ffmpeg")
}

/// `showcase.video` (`mp4,gif`, `all`, `all,-gif`, `off`): (an MP4, a GIF).
pub fn outputs(s: &str) -> (bool, bool) {
    let words: Vec<String> = s
        .split(',')
        .map(|w| w.trim().to_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    let every = words
        .iter()
        .any(|w| matches!(w.as_str(), "all" | "any" | "on"))
        || (!words.is_empty() && words.iter().all(|w| w.starts_with('-')));
    let has = |o: &str| {
        (every && !words.iter().any(|w| w.strip_prefix('-') == Some(o)))
            || words.iter().any(|w| w == o)
    };
    (has("mp4"), has("gif"))
}

/// `MP4 + GIF`, `MP4`, `GIF`.
pub fn what((mp4, gif): (bool, bool)) -> &'static str {
    match (mp4, gif) {
        (true, true) => "MP4 + GIF",
        (true, false) => "MP4",
        _ => "GIF",
    }
}

// ------------------------------------------------------------------ the cast

/// The player's colours: default text and background, the 16.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub fg: Rgb,
    pub bg: Rgb,
    pub pal: [Rgb; 16],
}

impl Theme {
    pub fn of(ink: &Ink) -> Theme {
        Theme {
            fg: ink.text,
            bg: ink.base,
            pal: ink.ansi,
        }
    }

    /// A vt100 colour (`dflt`: what Default is here).
    pub fn rgb(&self, c: vt100::Color, dflt: Rgb) -> Rgb {
        match c {
            vt100::Color::Default => dflt,
            vt100::Color::Rgb(r, g, b) => (r, g, b),
            vt100::Color::Idx(n) => match n {
                0..=15 => self.pal[n as usize],
                16..=231 => {
                    let k = n - 16;
                    let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                    (lv(k / 36), lv(k / 6 % 6), lv(k % 6))
                }
                _ => {
                    let v = 8 + (n - 232) * 10;
                    (v, v, v)
                }
            },
        }
    }
}

/// An event, on the recording's clock.
#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    Out(f64, String),
    Size(f64, u16, u16),
}

impl Ev {
    pub fn t(&self) -> f64 {
        match self {
            Ev::Out(t, _) | Ev::Size(t, _, _) => *t,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cast {
    pub cols: u16,
    pub rows: u16,
    /// when it began (unix)
    pub at: u64,
    /// the longest pause played (0: every pause as it was)
    pub idle: f64,
    pub title: String,
    pub theme: Option<Theme>,
    /// showcase's mode then (`masked`, `demo`, `real`), and the accent
    pub mode: Option<String>,
    pub accent: Option<Rgb>,
    pub events: Vec<Ev>,
}

/// `WxH` → (columns, rows).
fn size(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.trim().split_once('x')?;
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// A cast's text (asciinema v2) read: its header and events (markers and
/// input left out).
pub fn parse(text: &str) -> Result<Cast, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let h: Value = lines
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .ok_or("not an asciinema cast (no header)")?;
    if h.get("version").and_then(Value::as_u64) != Some(2) {
        return Err("not an asciinema v2 cast".into());
    }
    let dim = |k: &str| {
        h.get(k)
            .and_then(Value::as_u64)
            .filter(|n| (1..=1000).contains(n))
    };
    let (Some(cols), Some(rows)) = (dim("width"), dim("height")) else {
        return Err("the cast's header has no size".into());
    };
    let theme = h.get("theme").and_then(|t| {
        let fg = raster::parse_hex(t.get("fg")?.as_str()?)?;
        let bg = raster::parse_hex(t.get("bg")?.as_str()?)?;
        let cols: Vec<Rgb> = t
            .get("palette")?
            .as_str()?
            .split(':')
            .filter_map(raster::parse_hex)
            .collect();
        let mut pal = [(0, 0, 0); 16];
        for (i, c) in cols.iter().cycle().take(16).enumerate() {
            pal[i] = *c;
        }
        (cols.len() == 8 || cols.len() == 16).then_some(Theme { fg, bg, pal })
    });
    let env = |k: &str| {
        h.get("env")
            .and_then(|e| e.get(k))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let mut events = Vec::new();
    for l in lines {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        let (Some(t), Some(kind), Some(data)) = (
            v.get(0).and_then(Value::as_f64),
            v.get(1).and_then(Value::as_str),
            v.get(2).and_then(Value::as_str),
        ) else {
            continue;
        };
        match kind {
            "o" => events.push(Ev::Out(t.max(0.0), data.to_string())),
            "r" => {
                if let Some((w, h)) = size(data) {
                    events.push(Ev::Size(t.max(0.0), w, h));
                }
            }
            _ => {}
        }
    }
    Ok(Cast {
        cols: cols as u16,
        rows: rows as u16,
        at: h.get("timestamp").and_then(Value::as_u64).unwrap_or(0),
        idle: h
            .get("idle_time_limit")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        title: h
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        theme,
        mode: env("ALEPH_SHOWCASE").filter(|m| matches!(m.as_str(), "masked" | "demo" | "real")),
        accent: env("ALEPH_ACCENT").and_then(|a| raster::parse_hex(&a)),
        events,
    })
}

/// The events' times as played: a pause longer than `idle` cut to it.
pub fn played(events: &[Ev], idle: f64) -> Vec<f64> {
    let (mut prev, mut now) = (0.0f64, 0.0f64);
    events
        .iter()
        .map(|e| {
            let d = (e.t() - prev).max(0.0);
            prev = e.t();
            now += if idle > 0.0 { d.min(idle) } else { d };
            now
        })
        .collect()
}

impl Cast {
    /// The mode, from the header (the title's, in casts made before it was
    /// written down).
    pub fn mode(&self) -> Option<String> {
        self.mode.clone().or_else(|| {
            let (_, m) = self.title.rsplit_once(" — ")?;
            matches!(m, "masked" | "demo").then(|| m.to_string())
        })
    }

    /// The watermark it was made with (`Aleph — yours`).
    pub fn watermark(&self) -> Option<String> {
        let (_, w) = self.title.rsplit_once(" — ")?;
        (!matches!(w, "masked" | "demo" | "real") && !w.trim().is_empty())
            .then(|| w.trim().to_string())
    }

    /// The biggest screen it has.
    pub fn biggest(&self) -> (u16, u16) {
        self.events
            .iter()
            .fold((self.cols, self.rows), |(w, h), e| match e {
                Ev::Size(_, c, r) => (w.max(*c), h.max(*r)),
                _ => (w, h),
            })
    }
}

// ------------------------------------------------------------------ dim

/// A colour as SGR sets it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sgr {
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

impl Sgr {
    fn vt(self) -> vt100::Color {
        match self {
            Sgr::Default => vt100::Color::Default,
            Sgr::Idx(n) => vt100::Color::Idx(n),
            Sgr::Rgb(r, g, b) => vt100::Color::Rgb(r, g, b),
        }
    }

    /// The SGR that sets it as the foreground.
    fn fg_code(self) -> String {
        match self {
            Sgr::Default => "39".into(),
            Sgr::Idx(n) => format!("38;5;{n}"),
            Sgr::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
        }
    }
}

/// vt100 keeps no dim (SGR 2): this follows the stream's colours and
/// intensity and, while text is dim, sets its colour to the dimmed one
/// (blended toward its background, as the poster's opacity does).
#[derive(Debug)]
pub struct Dim {
    theme: Theme,
    fg: Sgr,
    bg: Sgr,
    dim: bool,
    /// an escape cut off at the end of the last chunk
    carry: String,
}

impl Dim {
    pub fn new(theme: Theme) -> Dim {
        Dim {
            theme,
            fg: Sgr::Default,
            bg: Sgr::Default,
            dim: false,
            carry: String::new(),
        }
    }

    /// The dimmed foreground now.
    fn dimmed(&self) -> Rgb {
        let fg = self.theme.rgb(self.fg.vt(), self.theme.fg);
        let bg = self.theme.rgb(self.bg.vt(), self.theme.bg);
        raster::mix(bg, fg, 0.55)
    }

    /// Follow one SGR's parameters.
    fn follow(&mut self, params: &str) {
        let p: Vec<u16> = params
            .split([';', ':'])
            .map(|x| x.parse().unwrap_or(0))
            .collect();
        let mut i = 0;
        while i < p.len() {
            let n = p[i];
            let colour = |i: &mut usize| -> Option<Sgr> {
                match p.get(*i + 1) {
                    Some(5) => {
                        let v = *p.get(*i + 2)? as u8;
                        *i += 2;
                        Some(Sgr::Idx(v))
                    }
                    Some(2) => {
                        let (r, g, b) = (*p.get(*i + 2)?, *p.get(*i + 3)?, *p.get(*i + 4)?);
                        *i += 4;
                        Some(Sgr::Rgb(r as u8, g as u8, b as u8))
                    }
                    _ => None,
                }
            };
            match n {
                0 => {
                    self.fg = Sgr::Default;
                    self.bg = Sgr::Default;
                    self.dim = false;
                }
                2 => self.dim = true,
                22 => self.dim = false,
                30..=37 => self.fg = Sgr::Idx((n - 30) as u8),
                90..=97 => self.fg = Sgr::Idx((n - 90 + 8) as u8),
                39 => self.fg = Sgr::Default,
                40..=47 => self.bg = Sgr::Idx((n - 40) as u8),
                100..=107 => self.bg = Sgr::Idx((n - 100 + 8) as u8),
                49 => self.bg = Sgr::Default,
                38 => {
                    if let Some(c) = colour(&mut i) {
                        self.fg = c;
                    }
                }
                48 => {
                    if let Some(c) = colour(&mut i) {
                        self.bg = c;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// A chunk of output on its way to vt100.
    pub fn pass(&mut self, s: &str) -> String {
        let text = std::mem::take(&mut self.carry) + s;
        let mut out = String::with_capacity(text.len() + 16);
        let mut rest = text.as_str();
        while let Some(at) = rest.find("\x1b[") {
            out.push_str(&rest[..at]);
            let seq = &rest[at + 2..];
            // parameters, intermediates, then the final byte
            let Some(end) = seq.find(|c: char| ('\x40'..='\x7e').contains(&c)) else {
                // cut off: the rest waits for the next chunk
                self.carry = rest[at..].to_string();
                return out;
            };
            let whole = &rest[at..at + 2 + end + 1];
            out.push_str(whole);
            if seq.as_bytes()[end] == b'm' && !seq[..end].starts_with(['?', '>', '<', '=']) {
                let was = self.dim;
                let params = if seq[..end].is_empty() {
                    "0"
                } else {
                    &seq[..end]
                };
                self.follow(params);
                if self.dim {
                    let (r, g, b) = self.dimmed();
                    out.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
                } else if was {
                    out.push_str(&format!("\x1b[{}m", self.fg.fg_code()));
                }
            }
            rest = &rest[at + 2 + end + 1..];
        }
        // a lone ESC at the very end may start a sequence
        if let Some(stripped) = rest.strip_suffix('\x1b') {
            out.push_str(stripped);
            self.carry = "\x1b".into();
        } else {
            out.push_str(rest);
        }
        out
    }
}

// ------------------------------------------------------------------ layout

/// Where the grid goes in a frame, and its cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub w: usize,
    pub h: usize,
    /// the grid's top left
    pub gx: i32,
    pub gy: i32,
    pub cw: i32,
    pub ch: i32,
    pub px: f32,
}

/// The cockpit card's measures around the grid: page margin, padding, title
/// bar, the gap under it, the footer.
const CARD: (i32, i32, i32, i32, i32) = (28, 22, 50, 12, 34);

/// Cells for a font size: (width, height).
fn cell_of(px: f32, adv: f32) -> (i32, i32) {
    (
        ((px * adv).round() as i32).max(1),
        ((px * LINE).round() as i32).max(2),
    )
}

/// The biggest font whose grid fits `room` (px, at most 32).
fn fit(cols: u16, rows: u16, room: (i32, i32), adv: f32) -> (i32, i32, f32) {
    let mut q = 32 * 4;
    while q > 4 * 4 {
        let px = q as f32 / 4.0;
        let (cw, ch) = cell_of(px, adv);
        if cols as i32 * cw <= room.0 && rows as i32 * ch <= room.1 {
            return (cw, ch, px.min((cw as f32 + 0.15) / adv));
        }
        q -= 1;
    }
    let (cw, ch) = cell_of(4.0, adv);
    (cw, ch, 4.0)
}

/// The cockpit: the grid in a card, the card centred at 1920×1080.
pub fn cockpit(cols: u16, rows: u16, adv: f32) -> Layout {
    let (m, p, tb, gap, fb) = CARD;
    let (w, h) = (COCKPIT.0 as i32, COCKPIT.1 as i32);
    let room = (w - 2 * (m + p), h - 2 * m - tb - gap - fb);
    let (cw, ch, px) = fit(cols, rows, room, adv);
    let (gw, gh) = (cols as i32 * cw, rows as i32 * ch);
    let (card_w, card_h) = (gw + 2 * p, tb + gap + gh + fb);
    let (x0, y0) = ((w - card_w) / 2, (h - card_h) / 2);
    Layout {
        w: w as usize,
        h: h as usize,
        gx: x0 + p,
        gy: y0 + tb + gap,
        cw,
        ch,
        px,
    }
}

/// Plain: the terminal edge to edge, its cells twice the poster's (smaller
/// when that's wider than 3840 or taller than 2160), the size even.
pub fn plain(cols: u16, rows: u16, adv: f32) -> Layout {
    let (mut cw, mut ch) = ((poster::CW * 2.0) as i32, (poster::CH * 2.0) as i32);
    if cols as i32 * cw > PLAIN_MAX.0 || rows as i32 * ch > PLAIN_MAX.1 {
        let k = (PLAIN_MAX.0 as f32 / (cols as i32 * cw) as f32)
            .min(PLAIN_MAX.1 as f32 / (rows as i32 * ch) as f32);
        cw = ((cw as f32 * k).floor() as i32).max(1);
        ch = ((cw as f32 * poster::CH as f32 / poster::CW as f32).round() as i32)
            .min(PLAIN_MAX.1 / rows.max(1) as i32)
            .max(2);
    }
    let even = |v: i32| (v + 1) & !1;
    let px = ((cw as f32 + 0.15) / adv).min(ch as f32 / LINE);
    Layout {
        w: even(cols as i32 * cw) as usize,
        h: even(rows as i32 * ch) as usize,
        gx: 0,
        gy: 0,
        cw,
        ch,
        px,
    }
}

// ------------------------------------------------------------------ the card

/// What the cockpit card says.
#[derive(Clone, Debug)]
pub struct Card {
    pub mode: Option<String>,
    pub watermark: Option<String>,
    /// when it was recorded, and how long it plays
    pub when: String,
    pub length: String,
}

/// Text in the mono font at `px`, from (x, baseline): its width.
#[allow(clippy::too_many_arguments)]
fn text(
    cv: &mut Canvas,
    f: &mut Fonts,
    (x, base): (i32, i32),
    s: &str,
    px: f32,
    style: u8,
    c: Rgb,
) -> i32 {
    let adv = (px * f.adv).round() as i32;
    let lh = (px * LINE).round() as i32;
    let mut at = x;
    for ch in s.chars() {
        if ch != ' ' {
            if let Some(g) = f.glyph(ch, style, px, adv, lh) {
                let clip = Clip::new(at - 2, base - lh, at + adv + 2, base + lh);
                cv.mask(at + g.dx, base + g.top, g.w, &g.cov, c, clip);
                if g.fake_bold {
                    cv.mask(at + g.dx + 1, base + g.top, g.w, &g.cov, c, clip);
                }
            }
        }
        at += adv;
    }
    at - x
}

fn text_w(f: &Fonts, s: &str, px: f32) -> i32 {
    s.chars().count() as i32 * (px * f.adv).round() as i32
}

/// A box blur of a mask, three times over (about a Gaussian of `r`).
fn blur(m: &mut [f32], w: usize, h: usize, r: usize) {
    // one pass along lines `n` long, `stride` apart within a line, a line
    // starting every `step`: a running sum over [i − r, i + r]
    let pass =
        |from: &[f32], to: &mut [f32], n: usize, stride: usize, lines: usize, step: usize| {
            let k = (2 * r + 1) as f32;
            for l in 0..lines {
                let base = l * step;
                let mut acc: f32 = (0..r.min(n)).map(|i| from[base + i * stride]).sum();
                for i in 0..n {
                    if i + r < n {
                        acc += from[base + (i + r) * stride];
                    }
                    if i > r {
                        acc -= from[base + (i - r - 1) * stride];
                    }
                    to[base + i * stride] = acc / k;
                }
            }
        };
    let mut tmp = vec![0f32; m.len()];
    for _ in 0..3 {
        pass(m, &mut tmp, w, 1, h, w);
        pass(&tmp, m, h, w, w, 1);
    }
}

/// A 4×4 ordered dither (−0.5 … 0.5): a gradient without bands.
fn bayer(x: usize, y: usize) -> f32 {
    const B: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    B[y % 4][x % 4] as f32 / 16.0 - 0.47
}

/// The cockpit's page and card, the grid's place left in the base colour.
pub fn card(l: &Layout, cols: u16, rows: u16, look: &Card, ink: &Ink, f: &mut Fonts) -> Canvas {
    let (_, p, tb, gap, fb) = CARD;
    let (w, h) = (l.w, l.h);
    let (gw, gh) = (cols as i32 * l.cw, rows as i32 * l.ch);
    let (cx, cy) = (l.gx - p, l.gy - tb - gap);
    let (cw, chh) = (gw + 2 * p, tb + gap + gh + fb);
    let (black, white) = ((0u8, 0u8, 0u8), (255u8, 255u8, 255u8));
    let (pg0, pg1, shade) = if ink.light {
        (
            raster::mix(ink.base, black, 0.09),
            raster::mix(ink.base, white, 0.4),
            0.18,
        )
    } else {
        (
            raster::mix(ink.base, black, 0.4),
            raster::mix(ink.base, white, 0.05),
            0.6,
        )
    };
    // the page: a diagonal gradient, the accent's glow top left and faintly
    // bottom right
    let mut cv = Canvas::new(w, h, pg0);
    let acc = ink.accent;
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let t = (fx + fy) / 2.0;
            let mut c = [0f32; 3];
            let (a0, a1, ac) = (
                [pg0.0 as f32, pg0.1 as f32, pg0.2 as f32],
                [pg1.0 as f32, pg1.1 as f32, pg1.2 as f32],
                [acc.0 as f32, acc.1 as f32, acc.2 as f32],
            );
            let g1 =
                (1.0 - (((fx - 0.1) / 0.8).powi(2) + (fy / 0.8).powi(2)).sqrt()).max(0.0) * 0.2;
            let g2 = (1.0 - (((fx - 1.0) / 0.6).powi(2) + ((fy - 1.0) / 0.6).powi(2)).sqrt())
                .max(0.0)
                * 0.09;
            let d = bayer(x, y);
            for i in 0..3 {
                let v = a0[i] + (a1[i] - a0[i]) * t;
                let v = v + (ac[i] - v) * g1;
                let v = v + (ac[i] - v) * g2;
                c[i] = (v + d).round().clamp(0.0, 255.0);
            }
            let k = (y * w + x) * 3;
            cv.px[k] = c[0] as u8;
            cv.px[k + 1] = c[1] as u8;
            cv.px[k + 2] = c[2] as u8;
        }
    }
    // the card's shadow, soft
    let r = 15usize;
    let (sx0, sy0) = (
        (cx + 6 - 3 * r as i32).max(0) as usize,
        (cy + 14 - 3 * r as i32).max(0) as usize,
    );
    let (sx1, sy1) = (
        ((cx + cw - 6 + 3 * r as i32) as usize).min(w),
        ((cy + 14 + chh + 3 * r as i32) as usize).min(h),
    );
    if sx1 > sx0 && sy1 > sy0 {
        let (mw, mh) = (sx1 - sx0, sy1 - sy0);
        let mut mask = Canvas::new(mw, mh, black);
        let ox = |v: i32| (v - sx0 as i32) as f32;
        let oy = |v: i32| (v - sy0 as i32) as f32;
        mask.rrect(
            ox(cx + 6),
            oy(cy + 14),
            ox(cx + cw - 6),
            oy(cy + 14 + chh),
            16.0,
            white,
            1.0,
        );
        let mut a: Vec<f32> = mask
            .px
            .chunks_exact(3)
            .map(|p| p[0] as f32 / 255.0)
            .collect();
        blur(&mut a, mw, mh, r);
        for y in 0..mh {
            for x in 0..mw {
                let k = a[y * mw + x] * shade;
                if k > 0.002 {
                    let i = ((sy0 + y) * w + sx0 + x) * 3;
                    for c in &mut cv.px[i..i + 3] {
                        *c = (*c as f32 * (1.0 - k)).round() as u8;
                    }
                }
            }
        }
    }
    // the card, its edge a line of the accent
    let (fx0, fy0, fx1, fy1) = (cx as f32, cy as f32, (cx + cw) as f32, (cy + chh) as f32);
    cv.rrect(
        fx0,
        fy0,
        fx1,
        fy1,
        16.0,
        raster::mix(ink.base, acc, 0.38),
        1.0,
    );
    cv.rrect(
        fx0 + 1.0,
        fy0 + 1.0,
        fx1 - 1.0,
        fy1 - 1.0,
        15.0,
        ink.base,
        1.0,
    );
    // the title bar: ℵ aleph  ✦ showcase                     your watermark
    let ty = cy + 32;
    let mark = raster::path(poster::MARK, 0.25, ((cx + p) as f32, cy as f32 + 12.5));
    let all = cv.all();
    cv.fill(&mark, acc, 1.0, all);
    let mut x = cx + p + 36;
    x += text(&mut cv, f, (x, ty), "aleph", 18.0, 1, ink.text) + 16;
    let chip = match look.mode.as_deref() {
        Some("masked") => Some("✦ showcase"),
        Some("demo") => Some("✦ demo"),
        _ => None,
    };
    if let Some(chip) = chip {
        let tw = text_w(f, chip, 12.0);
        let (pw, ph) = (tw + 20, 22);
        let (px0, py0) = (x as f32, (ty - 16) as f32);
        cv.rrect(px0, py0, px0 + pw as f32, py0 + ph as f32, 11.0, acc, 1.0);
        let on = if ink.light { white } else { (0x1b, 0x14, 0x05) };
        text(&mut cv, f, (x + 10, ty - 1), chip, 12.0, 1, on);
    }
    if let Some(wm) = &look.watermark {
        let tw = text_w(f, wm, 15.0);
        text(&mut cv, f, (cx + cw - p - tw, ty - 1), wm, 15.0, 2, acc);
    }
    // the rule under it, fading at both ends
    let (rx0, rx1) = (cx + p, cx + cw - p);
    for xx in rx0..rx1 {
        let t = (xx - rx0) as f32 / (rx1 - rx0).max(1) as f32;
        let k = 0.65 * (t / 0.18).min(1.0).min((1.0 - t) / 0.18);
        cv.blend(xx, cy + tb, xx + 1, cy + tb + 1, acc, k);
    }
    // the footer: size, when, how long · what the data is
    let fy = cy + chh - 13;
    let left = format!("{cols} × {rows} · {} · {}", look.when, look.length);
    text(&mut cv, f, (cx + p, fy), &left, 12.0, 0, ink.faint);
    let about = poster::about(look.mode.as_deref().unwrap_or(""));
    if !about.is_empty() {
        let tw = text_w(f, about, 12.0);
        text(
            &mut cv,
            f,
            (cx + cw - p - tw, fy),
            about,
            12.0,
            0,
            ink.faint,
        );
    }
    // HUD corners, just outside the card
    let (o, len) = (10, 24);
    let (x0, y0, x1, y1) = (cx - o, cy - o, cx + cw + o, cy + chh + o);
    let gold = |cv: &mut Canvas, a: (i32, i32), b: (i32, i32)| {
        let (l, r) = (a.0.min(b.0) - 1, a.0.max(b.0) + 1);
        let (t, btm) = (a.1.min(b.1) - 1, a.1.max(b.1) + 1);
        cv.blend(l, t, r, btm, acc, 0.85);
    };
    for (corner, dx, dy) in [
        ((x0, y0), 1, 1),
        ((x1, y0), -1, 1),
        ((x1, y1), -1, -1),
        ((x0, y1), 1, -1),
    ] {
        gold(&mut cv, corner, (corner.0 + dx * len, corner.1));
        gold(
            &mut cv,
            (corner.0, corner.1 + dy * 2),
            (corner.0, corner.1 + dy * len),
        );
    }
    // the grid's place: the screen's own background
    cv.rect(l.gx, l.gy, l.gx + gw, l.gy + gh, ink.base);
    cv
}

// ------------------------------------------------------------------ frames

/// How a run went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    pub frames: u64,
    pub secs: f64,
    pub w: usize,
    pub h: usize,
}

/// How many frames `secs` plays as.
fn frames_for(secs: f64) -> u64 {
    ((secs * FPS as f64 - 1e-6).ceil() as u64).max(1)
}

/// The cell at (row, col) as it's drawn (None: the right half of a wide one).
fn look_of(c: &vt100::Cell, th: &Theme, last: bool) -> Option<Look> {
    if c.is_wide_continuation() {
        return None;
    }
    let (mut fg, mut bg) = (th.rgb(c.fgcolor(), th.fg), th.rgb(c.bgcolor(), th.bg));
    if c.inverse() {
        std::mem::swap(&mut fg, &mut bg);
    }
    Some(Look {
        text: c.contents(),
        fg,
        bg,
        bold: c.bold(),
        italic: c.italic(),
        underline: c.underline(),
        cells: if c.is_wide() && !last { 2 } else { 1 },
    })
}

/// Every frame of `cast`, in `frame`, handed to `sink` in turn (`on`: frames
/// done, of how many).
pub fn play(
    cast: &Cast,
    frame: Frame,
    look: &Card,
    ink: &Ink,
    fonts: &mut Fonts,
    sink: &mut dyn FnMut(&Canvas) -> io::Result<()>,
    on: &mut dyn FnMut(u64, u64),
) -> io::Result<Played> {
    let th = Theme::of(ink);
    let times = played(&cast.events, cast.idle);
    let secs = times.last().copied().unwrap_or(0.0) + HOLD;
    let n = frames_for(secs);
    let (mut cols, mut rows) = (cast.cols, cast.rows);
    let big = cast.biggest();
    let adv = fonts.adv;
    let lay = |c: u16, r: u16| match frame {
        Frame::Cockpit => cockpit(c, r, adv),
        // one size for the whole video: the biggest screen's
        Frame::Plain => plain(big.0, big.1, adv),
    };
    let mut l = lay(cols, rows);
    let (vw, vh) = (l.w, l.h);
    let mut back = match frame {
        Frame::Cockpit => card(&l, cols, rows, look, ink, fonts),
        Frame::Plain => Canvas::new(vw, vh, th.bg),
    };
    let mut cells = Cells::new(l.cw, l.ch, l.px, fonts);
    let mut cv = back.clone();
    let mut vt = vt100::Parser::new(rows, cols, 0);
    let mut dim = Dim::new(th);
    let mut prev: Vec<Option<Option<Look>>> = vec![None; cols as usize * rows as usize];
    let mut k = 0usize;
    for i in 0..n {
        let until = (i + 1) as f64 / FPS as f64;
        let mut dirty = i == 0;
        while k < cast.events.len() && times[k] < until {
            match &cast.events[k] {
                Ev::Out(_, d) => vt.process(dim.pass(d).as_bytes()),
                Ev::Size(_, c, r) => {
                    (cols, rows) = (*c, *r);
                    vt.set_size(rows, cols);
                    l = lay(cols, rows);
                    if frame == Frame::Cockpit {
                        back = card(&l, cols, rows, look, ink, fonts);
                    }
                    cells = Cells::new(l.cw, l.ch, l.px, fonts);
                    cv = back.clone();
                    prev = vec![None; cols as usize * rows as usize];
                }
            }
            dirty = true;
            k += 1;
        }
        if dirty {
            let s = vt.screen();
            for r in 0..rows {
                for c in 0..cols {
                    let Some(cell) = s.cell(r, c) else {
                        continue;
                    };
                    let now = look_of(cell, &th, c + 1 == cols);
                    let slot = &mut prev[r as usize * cols as usize + c as usize];
                    if slot.as_ref() == Some(&now) {
                        continue;
                    }
                    // (a wide one's right half is drawn with it)
                    if let Some(lk) = &now {
                        let at = (l.gx + c as i32 * l.cw, l.gy + r as i32 * l.ch);
                        raster::paint(&mut cv, fonts, &cells, at, lk);
                    }
                    *slot = Some(now);
                }
            }
        }
        sink(&cv)?;
        on(i + 1, n);
    }
    Ok(Played {
        frames: n,
        secs,
        w: vw,
        h: vh,
    })
}

// ------------------------------------------------------------------ ffmpeg

/// ffmpeg's arguments for the MP4: raw RGB frames on stdin → H.264 (BT.709,
/// as players take HD video to be), the index first so it plays at once.
pub fn mp4_args(w: usize, h: usize, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = [
        "-hide_banner",
        "-nostats",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(format!("{w}x{h}"));
    a.extend(
        [
            "-framerate",
            &FPS.to_string(),
            "-i",
            "-",
            "-an",
            "-vf",
            "scale=out_color_matrix=bt709:out_range=tv,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "18",
            "-preset",
            "veryfast",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    a.push(out.display().to_string());
    a
}

/// The GIF's frames from the MP4: `GIF_FPS`, at most `GIF_WIDTH` wide, scaled
/// with lanczos.
fn gif_scale() -> String {
    format!("fps={GIF_FPS},scale=w='min({GIF_WIDTH},iw)':h=-2:flags=lanczos")
}

/// ffmpeg's arguments for the GIF's palette (pass one) and the GIF through
/// it (pass two).
pub fn gif_args(mp4: &Path, palette: &Path, out: &Path) -> (Vec<String>, Vec<String>) {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let mut one = s(&["-hide_banner", "-nostats", "-loglevel", "error", "-y", "-i"]);
    one.push(mp4.display().to_string());
    one.push("-vf".into());
    one.push(format!("{},palettegen=max_colors=256", gif_scale()));
    one.push(palette.display().to_string());
    let mut two = s(&["-hide_banner", "-nostats", "-loglevel", "error", "-y", "-i"]);
    two.push(mp4.display().to_string());
    two.push("-i".into());
    two.push(palette.display().to_string());
    two.push("-lavfi".into());
    two.push(format!(
        "{}[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle",
        gif_scale()
    ));
    two.extend(s(&["-loop", "0", "-f", "gif"]));
    two.push(out.display().to_string());
    (one, two)
}

/// How long ffmpeg may take over `frames` (generous: a slow machine, a big frame).
fn deadline(frames: u64, per: f64) -> Duration {
    Duration::from_secs_f64(120.0 + frames as f64 * per)
}

/// A file beside `p` while it's being made: `.name.part`.
fn part(p: &Path) -> PathBuf {
    let name = p
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    p.with_file_name(format!(".{name}.part"))
}

// ------------------------------------------------------------------ a render

/// What to make of a cast.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub cast: PathBuf,
    pub mp4: bool,
    pub gif: bool,
    pub frame: Frame,
    /// `showcase.video_font`
    pub font: String,
    pub keep_cast: bool,
}

impl Job {
    /// The settings' job for `cast`.
    pub fn of(cfg: &Config, cast: &Path) -> Job {
        let (mp4, gif) = outputs(&cfg.get_or("showcase.video"));
        Job {
            cast: cast.to_path_buf(),
            mp4,
            gif,
            frame: Frame::parse(&cfg.get_or("showcase.video_frame")).unwrap_or(Frame::Cockpit),
            font: cfg.get_or("showcase.video_font"),
            keep_cast: cfg.flag("showcase.cast_keep"),
        }
    }
}

/// What a render made.
#[derive(Clone, Debug, PartialEq)]
pub struct Made {
    pub mp4: Option<PathBuf>,
    pub gif: Option<PathBuf>,
    /// the cast is still there
    pub cast_kept: bool,
    pub width: usize,
    pub height: usize,
    pub frames: u64,
    /// how long it plays
    pub secs: f64,
    /// how long the render took
    pub took: f64,
    /// the font it was drawn with
    pub font: String,
}

/// Where a render is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    Frames(u64, u64),
    Gif,
}

/// The look a cast's frames take: its own colours, the accent it was made
/// with (else this one's), and what the card says.
pub fn dress(cast: &Cast) -> (Ink, Card) {
    let mut ink = Ink::now();
    if let Some(t) = &cast.theme {
        ink.text = t.fg;
        ink.base = t.bg;
        ink.ansi = t.pal;
        ink.faint = t.pal[8];
        let lum = |c: Rgb| 0.2126 * c.0 as f32 + 0.7152 * c.1 as f32 + 0.0722 * c.2 as f32;
        ink.light = lum(t.bg) > 140.0;
    }
    if let Some(a) = cast.accent {
        ink.accent = a;
    }
    let secs = played(&cast.events, cast.idle)
        .last()
        .copied()
        .unwrap_or(0.0);
    let card = Card {
        mode: cast.mode(),
        watermark: cast.watermark(),
        when: super::when(cast.at),
        length: clock(secs.round() as u64),
    };
    (ink, card)
}

/// Make the job's MP4 and GIF beside its cast with `ffmpeg`; the cast goes
/// unless it's kept. Nothing is left half made.
pub fn render(job: &Job, ffmpeg: &Path, on: &mut dyn FnMut(Stage)) -> Result<Made, String> {
    let t0 = Instant::now();
    if !job.mp4 && !job.gif {
        return Err("nothing to make (showcase.video is off)".into());
    }
    let text =
        std::fs::read_to_string(&job.cast).map_err(|e| format!("{}: {e}", shown(&job.cast)))?;
    let cast = parse(&text).map_err(|e| format!("{}: {e}", shown(&job.cast)))?;
    let spec = raster::find(&job.font)?;
    let mut fonts = Fonts::load(&spec)?;
    let (ink, card) = dress(&cast);
    let mp4 = job.cast.with_extension("mp4");
    let gif = job.cast.with_extension("gif");
    // the MP4 first (the GIF is made from it); without one wanted, a
    // temporary one
    let tmp_dir = std::env::temp_dir().join(format!("aleph-render-{}", std::process::id()));
    let mp4_part = if job.mp4 {
        part(&mp4)
    } else {
        let _ = std::fs::create_dir_all(&tmp_dir);
        tmp_dir.join("video.mp4")
    };
    let clean = || {
        let _ = std::fs::remove_file(part(&mp4));
        let _ = std::fs::remove_file(part(&gif));
        let _ = std::fs::remove_dir_all(&tmp_dir);
    };
    let big = cast.biggest();
    let l = match job.frame {
        Frame::Cockpit => cockpit(cast.cols, cast.rows, fonts.adv),
        Frame::Plain => plain(big.0, big.1, fonts.adv),
    };
    let (w, h) = (l.w, l.h);
    let n_guess = frames_for(
        played(&cast.events, cast.idle)
            .last()
            .copied()
            .unwrap_or(0.0)
            + HOLD,
    );
    let mut cmd = Command::new(ffmpeg);
    cmd.args(mp4_args(w, h, &mp4_part));
    let mut result: Option<io::Result<Played>> = None;
    let r = crate::proc::feed(&mut cmd, deadline(n_guess, 0.25), |stdin| {
        let mut sink = |c: &Canvas| stdin.write_all(&c.px);
        let mut tick = |i: u64, n: u64| {
            if i.is_multiple_of(15) || i == n {
                on(Stage::Frames(i, n));
            }
        };
        let p = play(
            &cast, job.frame, &card, &ink, &mut fonts, &mut sink, &mut tick,
        );
        let ok = p
            .as_ref()
            .map(|_| ())
            .map_err(|e| io::Error::new(e.kind(), e.to_string()));
        result = Some(p);
        ok
    });
    let ran = match (r, result) {
        (Ok((o, _)), Some(Ok(p))) if o.ok() => p,
        (Ok((o, wrote)), _) => {
            clean();
            let why = if o.timed_out {
                "ffmpeg took too long".to_string()
            } else if !o.stderr.trim().is_empty() {
                format!("ffmpeg: {}", o.reason())
            } else if let Err(e) = wrote {
                format!("writing the frames: {e}")
            } else {
                format!("ffmpeg: {}", o.reason())
            };
            return Err(why);
        }
        (Err(e), _) => {
            clean();
            return Err(format!("ffmpeg: {e}"));
        }
    };
    let mut made = Made {
        mp4: None,
        gif: None,
        cast_kept: true,
        width: ran.w,
        height: ran.h,
        frames: ran.frames,
        secs: ran.secs,
        took: 0.0,
        font: fonts.what.clone(),
    };
    if job.mp4 {
        std::fs::rename(&mp4_part, &mp4).map_err(|e| {
            clean();
            format!("{}: {e}", shown(&mp4))
        })?;
        made.mp4 = Some(mp4.clone());
    }
    if job.gif {
        on(Stage::Gif);
        let src = if job.mp4 {
            mp4.clone()
        } else {
            mp4_part.clone()
        };
        let _ = std::fs::create_dir_all(&tmp_dir);
        let palette = tmp_dir.join("palette.png");
        let (one, two) = gif_args(&src, &palette, &part(&gif));
        let t = deadline(ran.frames, 0.1);
        let run = |args: Vec<String>| -> Result<(), String> {
            match crate::proc::run(Command::new(ffmpeg).args(args), t, None) {
                Ok(o) if o.ok() => Ok(()),
                Ok(o) => Err(format!("ffmpeg (GIF): {}", o.reason())),
                Err(e) => Err(format!("ffmpeg (GIF): {e}")),
            }
        };
        let g = run(one).and_then(|_| run(two)).and_then(|_| {
            std::fs::rename(part(&gif), &gif).map_err(|e| format!("{}: {e}", shown(&gif)))
        });
        if let Err(e) = g {
            clean();
            // the MP4 made is kept; the cast too
            if made.mp4.is_some() {
                return Err(format!("{e} (the MP4 is made: {})", shown(&mp4)));
            }
            return Err(e);
        }
        made.gif = Some(gif.clone());
    }
    clean();
    if !job.keep_cast && std::fs::remove_file(&job.cast).is_ok() {
        made.cast_kept = false;
    }
    made.took = t0.elapsed().as_secs_f64();
    Ok(made)
}

/// What a render says in its status file (and `--json`).
pub fn status(job: &Job, r: &Result<Made, String>) -> Value {
    match r {
        Ok(m) => json!({
            "ok": true,
            "cast": job.cast,
            "cast_kept": m.cast_kept,
            "mp4": m.mp4,
            "gif": m.gif,
            "width": m.width,
            "height": m.height,
            "frames": m.frames,
            "seconds": (m.secs * 10.0).round() / 10.0,
            "took": (m.took * 10.0).round() / 10.0,
            "font": m.font,
        }),
        Err(e) => {
            json!({"ok": false, "cast": job.cast, "cast_kept": job.cast.exists(), "error": e})
        }
    }
}

// ------------------------------------------------------------------ after a stop

/// What stopping a recording leads to.
#[derive(Clone, Debug, PartialEq)]
pub enum After {
    /// the cast alone (`showcase.video` off)
    Cast,
    /// no ffmpeg here: the cast alone, and a warning
    NoFfmpeg,
    /// a render to start: what it makes
    Render(&'static str),
}

pub fn after(cfg: &Config, has_ffmpeg: bool) -> After {
    let o = outputs(&cfg.get_or("showcase.video"));
    if !o.0 && !o.1 {
        After::Cast
    } else if !has_ffmpeg {
        After::NoFfmpeg
    } else {
        After::Render(what(o))
    }
}

/// A render started after a stop, the dashboard waiting to say how it went.
struct Pending {
    child: Child,
    cast: PathBuf,
    status: PathBuf,
    what: &'static str,
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static LAST_LOOK: Mutex<Option<Instant>> = Mutex::new(None);

/// Where a render after a stop says how it went.
fn status_dir() -> PathBuf {
    crate::config::cache_dir().join("renders")
}

/// `aleph showcase render <cast> --after-stop --status <file>`, on its own:
/// its own session (a closing terminal, a ctrl+c and the dashboard's quit
/// don't reach it), a lower priority, nothing on the screen.
fn spawn(cast: &Path, status: &Path) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let mut c = Command::new(exe);
    c.args(["showcase", "render"])
        .arg(cast)
        .args(["--after-stop", "--status"])
        .arg(status)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid and nice are async-signal-safe; nothing else runs
    // between fork and exec.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            let _ = libc::nice(10);
            Ok(())
        });
    }
    c.spawn()
}

/// A recording was saved at `cast` (`secs` long): start its render, or say
/// why not. (what to say, a warning to hold up)
pub fn stopped(cfg: &Config, cast: &Path, secs: f64, quitting: bool) -> (String, Option<String>) {
    let rec = format!("recorded {} → {}", clock(secs.round() as u64), shown(cast));
    // never a process (or a which) from a unit test
    let has = cfg!(test) || ffmpeg().is_some();
    match after(cfg, has) {
        After::Cast => (rec, None),
        After::NoFfmpeg if quitting => (
            format!("{rec} — {}", no_ffmpeg().replacen(" — ", ", so ", 1)),
            None,
        ),
        After::NoFfmpeg => (rec, Some(no_ffmpeg())),
        After::Render(what) => {
            if cfg!(test) {
                return (format!("{rec} · rendering {what}…"), None);
            }
            let d = status_dir();
            let _ = std::fs::create_dir_all(&d);
            prune(&d);
            let stem = cast
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let status = d.join(format!("{stem}.json"));
            let _ = std::fs::remove_file(&status);
            match spawn(cast, &status) {
                Ok(child) => {
                    PENDING
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(Pending {
                            child,
                            cast: cast.to_path_buf(),
                            status,
                            what,
                        });
                    if quitting {
                        (
                            format!("{rec} · rendering {what} in the background (aleph showcase casts lists them)"),
                            None,
                        )
                    } else {
                        (format!("{rec} · rendering {what}…"), None)
                    }
                }
                Err(e) => (format!("{rec} (the {what} render didn't start: {e})"), None),
            }
        }
    }
}

/// Status files a week old go.
fn prune(d: &Path) {
    let week = Duration::from_secs(7 * 86_400);
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|a| a > week);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// `MP4 + GIF → Movies/Aleph/aleph-….mp4 + .gif`, or why not.
pub fn told(v: &Value, what: &str, cast: &Path) -> (bool, String) {
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        let p = |k: &str| v.get(k).and_then(Value::as_str).map(PathBuf::from);
        let (mp4, gif) = (p("mp4"), p("gif"));
        let first = mp4
            .clone()
            .or(gif.clone())
            .unwrap_or_else(|| cast.to_path_buf());
        let more = match (&mp4, &gif) {
            (Some(_), Some(_)) => " + .gif",
            _ => "",
        };
        let kept = if v.get("cast_kept").and_then(Value::as_bool) == Some(true) {
            " (the .cast is kept)"
        } else {
            ""
        };
        return (true, format!("{what} → {}{more}{kept}", shown(&first)));
    }
    let e = v
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("it stopped");
    (
        false,
        format!("no {what}: {e} — the .cast is kept → {}", shown(cast)),
    )
}

/// Renders started from this dashboard that finished since: what to say.
pub fn poll() -> Vec<(bool, String)> {
    {
        let mut last = LAST_LOOK.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
            return Vec::new();
        }
        *last = Some(Instant::now());
    }
    let mut v = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if v.is_empty() {
        return Vec::new();
    }
    let mut said = Vec::new();
    v.retain_mut(|p| {
        let done = match p.child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => true,
        };
        if !done {
            return true;
        }
        let st = std::fs::read_to_string(&p.status)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .unwrap_or_else(
                || json!({"ok": false, "error": "the render stopped before it was done"}),
            );
        let _ = std::fs::remove_file(&p.status);
        said.push(told(&st, p.what, &p.cast));
        false
    });
    said
}

// ------------------------------------------------------------------ what's in a file

/// An MP4's length and picture size (from its `moov`: `mvhd`, the video
/// track's `tkhd`).
pub fn mp4_info(path: &Path) -> Option<(f64, u32, u32)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut at = 0u64;
    let mut moov: Option<Vec<u8>> = None;
    while at + 8 <= len {
        f.seek(SeekFrom::Start(at)).ok()?;
        let mut h = [0u8; 16];
        f.read_exact(&mut h[..8]).ok()?;
        let mut size = u32::from_be_bytes(h[0..4].try_into().ok()?) as u64;
        let mut head = 8;
        if size == 1 {
            f.read_exact(&mut h[8..16]).ok()?;
            size = u64::from_be_bytes(h[8..16].try_into().ok()?);
            head = 16;
        } else if size == 0 {
            size = len - at;
        }
        if size < head {
            return None;
        }
        if &h[4..8] == b"moov" {
            let n = (size - head).min(64 << 20) as usize;
            let mut b = vec![0u8; n];
            f.read_exact(&mut b).ok()?;
            moov = Some(b);
            break;
        }
        at += size;
    }
    let moov = moov?;
    let be32 = |b: &[u8], o: usize| -> Option<u32> {
        Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
    };
    let be64 = |b: &[u8], o: usize| -> Option<u64> {
        Some(u64::from_be_bytes(b.get(o..o + 8)?.try_into().ok()?))
    };
    // children of a box's body: (type, body)
    let kids = |b: &[u8]| -> Vec<([u8; 4], Vec<u8>)> {
        let mut v = Vec::new();
        let mut i = 0usize;
        while i + 8 <= b.len() {
            let size = be32(b, i).unwrap_or(0) as usize;
            if size < 8 || i + size > b.len() {
                break;
            }
            let mut t = [0u8; 4];
            t.copy_from_slice(&b[i + 4..i + 8]);
            v.push((t, b[i + 8..i + size].to_vec()));
            i += size;
        }
        v
    };
    let mut secs = None;
    let mut wh = None;
    for (t, body) in kids(&moov) {
        if &t == b"mvhd" {
            let (scale, dur) = if body.first() == Some(&1) {
                (be32(&body, 20)?, be64(&body, 24)?)
            } else {
                (be32(&body, 12)?, be32(&body, 16)? as u64)
            };
            if scale > 0 {
                secs = Some(dur as f64 / scale as f64);
            }
        }
        if &t == b"trak" && wh.is_none() {
            for (t2, b2) in kids(&body) {
                if &t2 == b"tkhd" {
                    let o = if b2.first() == Some(&1) { 88 } else { 76 };
                    let (w, h) = (be32(&b2, o)? >> 16, be32(&b2, o + 4)? >> 16);
                    if w > 0 && h > 0 {
                        wh = Some((w, h));
                    }
                }
            }
        }
    }
    let (w, h) = wh.unwrap_or((0, 0));
    Some((secs?, w, h))
}

/// A GIF's length (its frames' delays) and size.
pub fn gif_info(path: &Path) -> Option<(f64, u32, u32)> {
    let b = std::fs::read(path).ok()?;
    if !b.starts_with(b"GIF8") || b.len() < 13 {
        return None;
    }
    let (w, h) = (
        u16::from_le_bytes([b[6], b[7]]) as u32,
        u16::from_le_bytes([b[8], b[9]]) as u32,
    );
    let mut i = 13
        + if b[10] & 0x80 != 0 {
            3 << ((b[10] & 7) + 1)
        } else {
            0
        };
    let mut cs = 0u64;
    let skip = |i: &mut usize| {
        while *i < b.len() {
            let n = b[*i] as usize;
            *i += 1;
            if n == 0 {
                break;
            }
            *i += n;
        }
    };
    while i < b.len() {
        match b[i] {
            0x21 => {
                if b.get(i + 1) == Some(&0xf9) && i + 6 < b.len() {
                    cs += u16::from_le_bytes([b[i + 4], b[i + 5]]) as u64;
                }
                i += 2;
                skip(&mut i);
            }
            0x2c => {
                let packed = *b.get(i + 9)?;
                i += 10;
                if packed & 0x80 != 0 {
                    i += 3 << ((packed & 7) + 1);
                }
                i += 1; // LZW minimum code size
                skip(&mut i);
            }
            _ => break,
        }
    }
    Some((cs as f64 / 100.0, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = r##"{"version":2,"width":20,"height":3,"timestamp":1792782174,"idle_time_limit":2.0,"title":"Aleph — masked","theme":{"fg":"#eeeeee","bg":"#101010","palette":"#000000:#ff0000:#00ff00:#ffff00:#0000ff:#ff00ff:#00ffff:#cccccc:#808080:#ff8080:#80ff80:#ffff80:#8080ff:#ff80ff:#80ffff:#ffffff"},"env":{"TERM":"xterm-256color","ALEPH_SHOWCASE":"demo","ALEPH_ACCENT":"#e2b451"}}"##;

    fn cast(events: &[&str]) -> String {
        let mut s = HEAD.to_string() + "\n";
        for e in events {
            s.push_str(e);
            s.push('\n');
        }
        s
    }

    #[test]
    fn a_cast_is_read_and_its_pauses_cut_short() {
        let c = parse(&cast(&[
            r#"[0.1,"o","\u001b[2J\u001b[Hhello"]"#,
            r#"[0.5,"m","away"]"#,
            r#"[9.5,"r","30x4"]"#,
            r#"[10.0,"o","!"]"#,
            "not json",
        ]))
        .unwrap();
        assert_eq!((c.cols, c.rows, c.at), (20, 3, 1792782174));
        assert_eq!(c.events.len(), 3);
        assert_eq!(c.events[1], Ev::Size(9.5, 30, 4));
        assert_eq!(c.mode().as_deref(), Some("demo"));
        assert_eq!(c.accent, Some((0xe2, 0xb4, 0x51)));
        assert_eq!(c.theme.unwrap().pal[1], (255, 0, 0));
        assert_eq!(c.biggest(), (30, 4));
        // 0.1, then 9.4 later cut to 2, then 0.5
        let t = played(&c.events, c.idle);
        assert!(
            (t[0] - 0.1).abs() < 1e-9 && (t[1] - 2.1).abs() < 1e-9 && (t[2] - 2.6).abs() < 1e-9,
            "{t:?}"
        );
        assert_eq!(played(&c.events, 0.0)[2], 10.0);
        assert!(parse("{\"version\":1}").is_err());
        assert!(parse("").is_err());
        // the title says the mode (or the watermark) in older casts
        let mut old = c.clone();
        old.mode = None;
        old.title = "Aleph — masked".into();
        assert_eq!(old.mode().as_deref(), Some("masked"));
        assert_eq!(old.watermark(), None);
        old.title = "Aleph — made by alex".into();
        assert_eq!(
            (old.mode(), old.watermark().as_deref()),
            (None, Some("made by alex"))
        );
    }

    fn theme() -> Theme {
        let mut pal = [(0, 0, 0); 16];
        pal[1] = (200, 0, 0);
        Theme {
            fg: (200, 200, 200),
            bg: (0, 0, 0),
            pal,
        }
    }

    #[test]
    fn dim_text_gets_its_dimmed_colour() {
        let mut d = Dim::new(theme());
        // dim on: the default colour, dimmed, is set after it
        let s = d.pass("a\x1b[2mb");
        assert_eq!(s, "a\x1b[2m\x1b[38;2;110;110;110mb");
        // a colour while dim: dimmed too
        let s = d.pass("\x1b[38;2;100;0;200mc");
        assert!(s.ends_with("\x1b[38;2;55;0;110mc"), "{s:?}");
        // intensity back to normal: the colour as it was set
        let s = d.pass("\x1b[22md");
        assert_eq!(s, "\x1b[22m\x1b[38;2;100;0;200md");
        // a reset: nothing to add
        assert_eq!(d.pass("\x1b[0me"), "\x1b[0me");
        // an escape cut in two waits for its end
        assert_eq!(d.pass("f\x1b[3"), "f");
        assert_eq!(d.pass("1;2mg"), "\x1b[31;2m\x1b[38;2;110;0;0mg");
        assert_eq!(d.pass("\x1b"), "");
        assert_eq!(d.pass("[mh"), "\x1b[m\x1b[39mh");
        // other sequences pass as they are
        assert_eq!(d.pass("\x1b[?25l\x1b[1;1H"), "\x1b[?25l\x1b[1;1H");
        // through vt100: the cell has the dimmed colour
        let mut vt = vt100::Parser::new(1, 10, 0);
        let mut d = Dim::new(theme());
        vt.process(d.pass("\x1b[31;2mx\x1b[22my").as_bytes());
        let th = theme();
        let c = vt.screen().cell(0, 0).unwrap();
        assert_eq!(th.rgb(c.fgcolor(), th.fg), (110, 0, 0));
        let c = vt.screen().cell(0, 1).unwrap();
        assert_eq!(th.rgb(c.fgcolor(), th.fg), (200, 0, 0));
    }

    #[test]
    fn layouts_fit_and_stay_even() {
        let c = cockpit(226, 60, 0.602);
        assert_eq!((c.w, c.h), COCKPIT);
        assert!(
            226 * c.cw <= 1920 - 2 * (CARD.0 + CARD.1) && 60 * c.ch <= 1080,
            "{c:?}"
        );
        assert!(c.gx > 0 && c.gy > 0 && c.ch >= 2 * c.cw - 1, "{c:?}");
        // a smaller screen: bigger letters
        let s = cockpit(100, 30, 0.602);
        assert!(s.px > c.px && s.cw > c.cw, "{s:?}");
        // plain: twice the poster's cells, even
        let p = plain(120, 33, 0.602);
        assert_eq!((p.cw, p.ch), (18, 38));
        assert_eq!((p.w, p.h), (2160, 1254));
        let odd = plain(101, 3, 0.602);
        assert_eq!(odd.w % 2, 0);
        // too wide: capped at 3840, still even
        let wide = plain(300, 60, 0.602);
        assert!(
            wide.w <= 3840 && wide.w.is_multiple_of(2) && wide.h.is_multiple_of(2),
            "{wide:?}"
        );
        assert!(wide.cw < 18 && wide.ch > wide.cw);
    }

    #[test]
    fn ffmpeg_is_asked_for_h264_and_a_two_pass_gif() {
        let a = mp4_args(1920, 1080, Path::new("/x/.a.mp4.part"));
        let s = a.join(" ");
        assert!(
            s.contains("-f rawvideo -pix_fmt rgb24 -s 1920x1080 -framerate 30 -i -"),
            "{s}"
        );
        assert!(
            s.contains("-c:v libx264 -pix_fmt yuv420p -crf 18 -preset veryfast"),
            "{s}"
        );
        assert!(
            s.contains("-movflags +faststart -f mp4 /x/.a.mp4.part"),
            "{s}"
        );
        assert!(s.contains("out_color_matrix=bt709"));
        let (one, two) = gif_args(
            Path::new("/x/a.mp4"),
            Path::new("/t/p.png"),
            Path::new("/x/.a.gif.part"),
        );
        let (one, two) = (one.join(" "), two.join(" "));
        assert!(
            one.contains(
                "-i /x/a.mp4 -vf fps=15,scale=w='min(1280,iw)':h=-2:flags=lanczos,palettegen"
            ),
            "{one}"
        );
        assert!(one.ends_with("/t/p.png"));
        assert!(
            two.contains("-i /x/a.mp4 -i /t/p.png -lavfi fps=15"),
            "{two}"
        );
        assert!(two.contains("[v][1:v]paletteuse=dither=bayer"), "{two}");
        assert!(two.ends_with("-loop 0 -f gif /x/.a.gif.part"), "{two}");
        assert_eq!(part(Path::new("/m/a.mp4")), PathBuf::from("/m/.a.mp4.part"));
        assert!(deadline(900, 0.25) >= Duration::from_secs(300));
    }

    #[test]
    fn what_a_stop_leads_to() {
        let c = Config::in_memory(&[]);
        assert_eq!(outputs(&c.get_or("showcase.video")), (true, true));
        assert_eq!(after(&c, true), After::Render("MP4 + GIF"));
        assert_eq!(after(&c, false), After::NoFfmpeg);
        let gif = Config::in_memory(&["showcase.video=gif"]);
        assert_eq!(after(&gif, true), After::Render("GIF"));
        let off = Config::in_memory(&["showcase.video=off"]);
        assert_eq!(after(&off, false), After::Cast);
        assert_eq!(outputs(" MP4 , gif "), (true, true));
        assert_eq!(outputs("webm"), (false, false));
        // what Settings' checklist saves, and all-but-some
        assert_eq!(outputs("all"), (true, true));
        assert_eq!(outputs("off"), (false, false));
        assert_eq!(outputs("all,-gif"), (true, false));
        assert_eq!(outputs("-mp4"), (false, true));
        assert!(crate::config::check("showcase.video", "all, -gif").is_ok());
        assert!(crate::config::check("showcase.video", "webm").is_err());
        assert!(no_ffmpeg().starts_with("ffmpeg isn't on this computer — only the .cast was saved"));
        // a unit test never starts a process: it says what would happen
        let (m, warn) = stopped(&c, Path::new("/tmp/a.cast"), 42.0, false);
        assert!(
            m.starts_with("recorded 0:42 → ") && m.contains("rendering MP4 + GIF"),
            "{m}"
        );
        assert!(warn.is_none());
        let j = Job::of(&c, Path::new("/m/a.cast"));
        assert_eq!(
            (j.mp4, j.gif, j.frame, j.keep_cast),
            (true, true, Frame::Cockpit, false)
        );
        let k = Job::of(
            &Config::in_memory(&["showcase.cast_keep=true", "showcase.video_frame=plain"]),
            Path::new("/m/a.cast"),
        );
        assert_eq!((k.frame, k.keep_cast), (Frame::Plain, true));
    }

    #[test]
    fn a_finished_render_says_where_its_files_went() {
        let home = crate::config::home();
        let cast = home.join("Movies/Aleph/a.cast");
        let v = json!({"ok": true, "mp4": home.join("Movies/Aleph/a.mp4"), "gif": home.join("Movies/Aleph/a.gif"), "cast_kept": false});
        assert_eq!(
            told(&v, "MP4 + GIF", &cast),
            (true, "MP4 + GIF → Movies/Aleph/a.mp4 + .gif".into())
        );
        let kept = json!({"ok": true, "gif": home.join("Movies/Aleph/a.gif"), "cast_kept": true});
        assert_eq!(
            told(&kept, "GIF", &cast).1,
            "GIF → Movies/Aleph/a.gif (the .cast is kept)"
        );
        let bad = json!({"ok": false, "error": "ffmpeg: boom"});
        let (ok, m) = told(&bad, "MP4", &cast);
        assert!(
            !ok && m.contains("ffmpeg: boom") && m.contains("kept → Movies/Aleph/a.cast"),
            "{m}"
        );
    }

    #[test]
    fn media_files_tell_their_length_and_size() {
        let d = std::env::temp_dir().join(format!("aleph-media-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        // an MP4's boxes: ftyp, then moov with mvhd (600 per s, 4.5 s) and a
        // trak whose tkhd says 1920×1080
        let bx = |t: &[u8], body: &[u8]| -> Vec<u8> {
            let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(t);
            v.extend_from_slice(body);
            v
        };
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&600u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&2700u32.to_be_bytes());
        let mut tkhd = vec![0u8; 84];
        tkhd[76..80].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        let trak = bx(b"trak", &bx(b"tkhd", &tkhd));
        let moov = bx(b"moov", &[bx(b"mvhd", &mvhd), trak].concat());
        let f = d.join("a.mp4");
        std::fs::write(&f, [bx(b"ftyp", b"isom"), moov].concat()).unwrap();
        assert_eq!(mp4_info(&f), Some((4.5, 1920, 1080)));
        std::fs::write(d.join("b.mp4"), b"nope").unwrap();
        assert_eq!(mp4_info(&d.join("b.mp4")), None);
        // a GIF: two frames of 0.5 s, 4×2
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&[4, 0, 2, 0, 0x80, 0, 0]);
        g.extend_from_slice(&[0; 6]);
        for _ in 0..2 {
            g.extend_from_slice(&[0x21, 0xf9, 4, 0, 50, 0, 0, 0]);
            g.extend_from_slice(&[0x2c, 0, 0, 0, 0, 4, 0, 2, 0, 0, 2, 2, 0x4c, 0x01, 0]);
        }
        g.push(0x3b);
        std::fs::write(d.join("c.gif"), &g).unwrap();
        assert_eq!(gif_info(&d.join("c.gif")), Some((1.0, 4, 2)));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A cast played to frames, without ffmpeg: the first frame shows the
    /// text where its cells are, a resize is honoured, the last screen held.
    #[test]
    fn a_cast_plays_to_frames() {
        let Ok(spec) = raster::find("") else {
            return;
        };
        let mut fonts = Fonts::load(&spec).unwrap();
        let c = parse(&cast(&[
            r#"[0.0,"o","\u001b[H\u001b[2J\u001b[38;2;226;180;81m███ gold\u001b[0m"]"#,
            r#"[0.5,"o","\u001b[2;1H\u001b[44m  \u001b[0m"]"#,
            r#"[5.0,"r","24x3"]"#,
            r#"[5.1,"o","\u001b[3;1H\u001b[7m inv \u001b[0m"]"#,
        ]))
        .unwrap();
        let (ink, card_look) = dress(&c);
        let mut frames: Vec<Canvas> = Vec::new();
        let mut sink = |cv: &Canvas| {
            frames.push(cv.clone());
            Ok(())
        };
        let p = play(
            &c,
            Frame::Plain,
            &card_look,
            &ink,
            &mut fonts,
            &mut sink,
            &mut |_, _| {},
        )
        .unwrap();
        // 0.5 s, the 4.5 s pause cut to 2, 0.1, then the hold: 3.6 s at 30 fps
        assert_eq!(p.frames, 108);
        assert_eq!(frames.len(), 108);
        let l = plain(24, 3, fonts.adv);
        assert_eq!((p.w, p.h), (l.w, l.h));
        let f0 = &frames[0];
        // the first cell: a full block in gold
        assert_eq!(f0.at(5, 5), (226, 180, 81));
        // the background elsewhere is the theme's
        assert_eq!(f0.at(l.cw as usize * 15, 5), (0x10, 0x10, 0x10));
        // the blue cells come with the second event
        let blue = c.theme.unwrap().pal[4];
        assert_ne!(f0.at(5, l.ch as usize + 5), blue);
        let last = frames.last().unwrap();
        assert_eq!(last.at(5, l.ch as usize + 5), blue);
        // inverse: the text colour behind
        assert_eq!(last.at(2, 2 * l.ch as usize + 2), (0xee, 0xee, 0xee));
        // a cockpit frame: 1920×1080, the gold mark top left of the card
        let mut first: Option<Canvas> = None;
        let mut sink = |cv: &Canvas| {
            if first.is_none() {
                first = Some(cv.clone());
            }
            Ok(())
        };
        let q = play(
            &c,
            Frame::Cockpit,
            &card_look,
            &ink,
            &mut fonts,
            &mut sink,
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!((q.w, q.h), COCKPIT);
        let f = first.unwrap();
        let l = cockpit(20, 3, fonts.adv);
        assert_eq!(f.at(l.gx as usize + 2, l.gy as usize + 2), (226, 180, 81));
    }
}
