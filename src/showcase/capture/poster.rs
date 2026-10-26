//! A poster: one frame of the dashboard (a ratatui buffer, after the lens)
//! as an SVG card in Aleph's Graphite + gold. Pure Rust, no dependencies:
//!
//! * text is monospace, every character placed in its own cell (an `x` per
//!   character), so columns line up whatever font the viewer has;
//! * box drawing (`─│╭╮╰╯├┤┬┴┼` and their heavy kin), block elements
//!   (`█▀▄▌▐`, the eighths, quadrants, shades) and Braille dots are drawn
//!   as shapes in cell geometry, not as glyphs — panels close, bars and
//!   charts are crisp at any zoom;
//! * colours are the theme's (the terminal's own background becomes the
//!   theme's base);
//! * the frame: `cockpit` — a graphite card with gold HUD corners, the ℵ
//!   mark, the view, the mode and your watermark; `plain` — the screen on
//!   its background, your watermark in the corner.
//!
//! A PNG comes only from a converter already on the computer
//! (`rsvg-convert`, or macOS's `qlmanage`); nothing is ever installed.

use crate::ui::c;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

/// A cell, in px.
pub const CW: f64 = 9.0;
pub const CH: f64 = 19.0;
const FONT_SIZE: f64 = 15.0;
/// the text's baseline in a cell
const BASELINE: f64 = 14.2;
const LIGHT: f64 = 1.3;
const HEAVY: f64 = 2.6;

const MONO: &str = "'JetBrains Mono','JetBrainsMono Nerd Font','SF Mono',Menlo,Monaco,'DejaVu Sans Mono','Symbols Nerd Font Mono',Consolas,monospace";
const SANS: &str =
    "Inter,-apple-system,BlinkMacSystemFont,'SF Pro Text','Helvetica Neue',Helvetica,Arial,sans-serif";

/// Aleph's mark (web/index.html's `#mark`), on a 100 × 100 box.
pub const MARK: &str = "M 19 7 C 30 4, 37 11, 41 17 L 78 67 C 82 74, 88 81, 94 84 L 94 91 L 68 91 C 64 87, 62 82, 58 77 L 14 29 C 7 22, 8 9, 19 7 Z M 62 8 L 86 5 C 91 9, 89 17, 84 22 C 77 30, 71 40, 64 54 L 51 40 C 57 31, 63 22, 66 17 C 66 13, 62 11, 62 8 Z M 33 38 L 47 50 C 38 62, 33 72, 33 81 C 33 86, 36 89, 41 89 L 41 94 L 8 94 L 8 89 C 15 89, 21 86, 23 81 C 25 70, 28 52, 33 38 Z";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Cockpit,
    Plain,
}

impl Frame {
    pub fn parse(s: &str) -> Option<Frame> {
        match s.trim() {
            "cockpit" | "" => Some(Frame::Cockpit),
            "plain" => Some(Frame::Plain),
            _ => None,
        }
    }
}

/// What the card says around the screen.
#[derive(Clone, Debug)]
pub struct Look {
    pub frame: Frame,
    /// the view (`Fleet`, `Monitor › Usage`)
    pub view: String,
    /// `masked`, `demo` or `real`
    pub mode: String,
    pub watermark: Option<String>,
    /// when (local, `2026-10-23 14:05`)
    pub when: String,
}

type Rgb = (u8, u8, u8);

/// The colours a poster resolves the screen's with.
#[derive(Clone, Debug)]
pub struct Ink {
    pub text: Rgb,
    pub base: Rgb,
    pub accent: Rgb,
    pub faint: Rgb,
    pub light: bool,
    /// the 16 named colours (black … white)
    pub ansi: [Rgb; 16],
}

fn rgb_of(c: Color, or: Rgb) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => or,
    }
}

impl Ink {
    /// The theme in use (`theme`, `accent`; graphite when NO_COLOR is set).
    pub fn now() -> Ink {
        Ink::of(c::pal())
    }

    pub fn of(p: &c::Pal) -> Ink {
        let dark = c::Pal::dark();
        let p = if p.mono { &dark } else { p };
        let g = |c: Color, d: Color| rgb_of(c, rgb_of(d, (0x88, 0x88, 0x88)));
        let text = g(p.text, dark.text);
        let base = g(p.base, dark.base);
        let (red, green, yellow, blue, mauve, teal, sky) = (
            g(p.red, dark.red),
            g(p.green, dark.green),
            g(p.yellow, dark.yellow),
            g(p.blue, dark.blue),
            g(p.mauve, dark.mauve),
            g(p.teal, dark.teal),
            g(p.sky, dark.sky),
        );
        let muted = g(p.muted, dark.muted);
        let faint = g(p.faint, dark.faint);
        let lift = |c: Rgb, k: f64| -> Rgb {
            let t = if p.light { 0.0 } else { 255.0 };
            let m = |x: u8| (x as f64 + (t - x as f64) * k).round() as u8;
            (m(c.0), m(c.1), m(c.2))
        };
        let black = if p.light {
            (0x20, 0x20, 0x20)
        } else {
            (0x26, 0x26, 0x26)
        };
        Ink {
            text,
            base,
            accent: g(p.accent, dark.accent),
            faint,
            light: p.light,
            ansi: [
                black,
                red,
                green,
                yellow,
                blue,
                mauve,
                teal,
                muted,
                faint,
                lift(red, 0.25),
                lift(green, 0.25),
                lift(yellow, 0.25),
                lift(blue, 0.25),
                lift(mauve, 0.25),
                sky,
                text,
            ],
        }
    }

    /// A cell's colour (None: the terminal's own — no paint).
    pub fn resolve(&self, c: Color) -> Option<Rgb> {
        let i = match c {
            Color::Reset => return None,
            Color::Rgb(r, g, b) => return Some((r, g, b)),
            Color::Indexed(n) => return Some(self.indexed(n)),
            Color::Black => 0,
            Color::Red => 1,
            Color::Green => 2,
            Color::Yellow => 3,
            Color::Blue => 4,
            Color::Magenta => 5,
            Color::Cyan => 6,
            Color::Gray => 7,
            Color::DarkGray => 8,
            Color::LightRed => 9,
            Color::LightGreen => 10,
            Color::LightYellow => 11,
            Color::LightBlue => 12,
            Color::LightMagenta => 13,
            Color::LightCyan => 14,
            Color::White => 15,
        };
        Some(self.ansi[i])
    }

    /// xterm's 256: the 16, a 6×6×6 cube, 24 greys.
    fn indexed(&self, n: u8) -> Rgb {
        match n {
            0..=15 => self.ansi[n as usize],
            16..=231 => {
                let k = n - 16;
                let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                (lv(k / 36), lv(k / 6 % 6), lv(k % 6))
            }
            _ => {
                let v = 8 + (n - 232) * 10;
                (v, v, v)
            }
        }
    }

    /// `#rrggbb` for the 16 (a cast's theme).
    pub fn palette(&self) -> Vec<String> {
        self.ansi.iter().map(|c| hex(*c)).collect()
    }
}

pub fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Text for SVG: `& < >` and quotes escaped, control characters gone.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() => {}
            c => o.push(c),
        }
    }
    o
}

fn n(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

// ------------------------------------------------------------------ shapes

/// A box-drawing character's arms: (up, right, down, left), 1 light, 2 heavy.
fn arms(ch: char) -> Option<[u8; 4]> {
    Some(match ch {
        '─' => [0, 1, 0, 1],
        '━' => [0, 2, 0, 2],
        '│' => [1, 0, 1, 0],
        '┃' => [2, 0, 2, 0],
        '┌' => [0, 1, 1, 0],
        '┏' => [0, 2, 2, 0],
        '┐' => [0, 0, 1, 1],
        '┓' => [0, 0, 2, 2],
        '└' => [1, 1, 0, 0],
        '┗' => [2, 2, 0, 0],
        '┘' => [1, 0, 0, 1],
        '┛' => [2, 0, 0, 2],
        '├' => [1, 1, 1, 0],
        '┣' => [2, 2, 2, 0],
        '┝' => [1, 2, 1, 0],
        '┤' => [1, 0, 1, 1],
        '┫' => [2, 0, 2, 2],
        '┥' => [1, 0, 1, 2],
        '┬' => [0, 1, 1, 1],
        '┳' => [0, 2, 2, 2],
        '┯' => [0, 2, 1, 2],
        '┴' => [1, 1, 0, 1],
        '┻' => [2, 2, 0, 2],
        '┷' => [1, 2, 0, 2],
        '┼' => [1, 1, 1, 1],
        '╋' => [2, 2, 2, 2],
        '┿' => [1, 2, 1, 2],
        '╂' => [2, 1, 2, 1],
        '╴' => [0, 0, 0, 1],
        '╵' => [1, 0, 0, 0],
        '╶' => [0, 1, 0, 0],
        '╷' => [0, 0, 1, 0],
        '╸' => [0, 0, 0, 2],
        '╹' => [2, 0, 0, 0],
        '╺' => [0, 2, 0, 0],
        '╻' => [0, 0, 2, 0],
        '╼' => [0, 2, 0, 1],
        '╾' => [0, 1, 0, 2],
        _ => return None,
    })
}

/// A rounded corner: (the arc's path in a cell at the origin).
fn arc(ch: char) -> Option<String> {
    let (cx, cy, r) = (CW / 2.0, CH / 2.0, CW / 2.0);
    Some(match ch {
        '╭' => format!(
            "M{} {}H{}A{r} {r} 0 0 0 {cx} {}V{}",
            n(CW),
            n(cy),
            n(cx + r),
            n(cy + r),
            n(CH)
        ),
        '╮' => format!(
            "M0 {}H{}A{r} {r} 0 0 1 {cx} {}V{}",
            n(cy),
            n(cx - r),
            n(cy + r),
            n(CH)
        ),
        '╯' => format!(
            "M{cx} 0V{}A{r} {r} 0 0 1 {} {}H0",
            n(cy - r),
            n(cx - r),
            n(cy)
        ),
        '╰' => format!(
            "M{cx} 0V{}A{r} {r} 0 0 0 {} {}H{}",
            n(cy - r),
            n(cx + r),
            n(cy),
            n(CW)
        ),
        _ => return None,
    })
}

/// A block element: its rectangles as fractions of the cell (x0, y0, x1,
/// y1), and how opaque (the shades).
fn block(ch: char) -> Option<(Vec<[f64; 4]>, f64)> {
    let full = |v: Vec<[f64; 4]>| Some((v, 1.0));
    const UL: [f64; 4] = [0.0, 0.0, 0.5, 0.5];
    const UR: [f64; 4] = [0.5, 0.0, 1.0, 0.5];
    const LL: [f64; 4] = [0.0, 0.5, 0.5, 1.0];
    const LR: [f64; 4] = [0.5, 0.5, 1.0, 1.0];
    match ch {
        '█' => full(vec![[0.0, 0.0, 1.0, 1.0]]),
        '▀' => full(vec![[0.0, 0.0, 1.0, 0.5]]),
        '▔' => full(vec![[0.0, 0.0, 1.0, 0.125]]),
        '▕' => full(vec![[0.875, 0.0, 1.0, 1.0]]),
        '▐' => full(vec![[0.5, 0.0, 1.0, 1.0]]),
        // the lower eighths: ▁ … ▇ (▄ is four of them)
        '\u{2581}'..='\u{2587}' => {
            let k = (ch as u32 - 0x2580) as f64;
            full(vec![[0.0, 1.0 - k / 8.0, 1.0, 1.0]])
        }
        // the left eighths: ▉ (seven) … ▏ (one); ▌ is four
        '\u{2589}'..='\u{258f}' => {
            let k = (0x2590 - ch as u32) as f64;
            full(vec![[0.0, 0.0, k / 8.0, 1.0]])
        }
        '░' => Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.25)),
        '▒' => Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.5)),
        '▓' => Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.75)),
        '▖' => full(vec![LL]),
        '▗' => full(vec![LR]),
        '▘' => full(vec![UL]),
        '▝' => full(vec![UR]),
        '▙' => full(vec![UL, LL, LR]),
        '▚' => full(vec![UL, LR]),
        '▛' => full(vec![UL, UR, LL]),
        '▜' => full(vec![UL, UR, LR]),
        '▞' => full(vec![UR, LL]),
        '▟' => full(vec![UR, LL, LR]),
        _ => None,
    }
}

/// A Braille pattern's dots: (column 0–1, row 0–3).
fn braille(ch: char) -> Option<Vec<(u8, u8)>> {
    let v = ch as u32;
    if !(0x2800..=0x28ff).contains(&v) {
        return None;
    }
    let bits = (v - 0x2800) as u8;
    const AT: [(u8, u8); 8] = [
        (0, 0),
        (0, 1),
        (0, 2),
        (1, 0),
        (1, 1),
        (1, 2),
        (0, 3),
        (1, 3),
    ];
    Some(
        (0..8)
            .filter(|i| bits & (1 << i) != 0)
            .map(|i| AT[i as usize])
            .collect(),
    )
}

// ------------------------------------------------------------------ the screen

/// How a cell looks once its colours are resolved.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Pen {
    fg: Rgb,
    bold: bool,
    italic: bool,
    dim: bool,
}

/// Merged straight lines: (colour, thickness in tenths, dim) → (the fixed
/// coordinate, from, to), in tenths of a px.
type Lines = BTreeMap<(Rgb, u32, bool), Vec<(i64, i64, i64)>>;

fn tenth(v: f64) -> i64 {
    (v * 10.0).round() as i64
}

fn merged(mut v: Vec<(i64, i64, i64)>) -> Vec<(i64, i64, i64)> {
    v.sort();
    let mut out: Vec<(i64, i64, i64)> = Vec::new();
    for (k, a, b) in v {
        match out.last_mut() {
            Some((k2, _, b2)) if *k2 == k && a <= *b2 => *b2 = (*b2).max(b),
            _ => out.push((k, a, b)),
        }
    }
    out
}

/// The screen as SVG elements, at the origin, `w`×`h` cells of `CW`×`CH`.
pub fn screen(buf: &Buffer, ink: &Ink) -> String {
    let a = buf.area;
    let mut bg = String::new();
    let mut shapes = String::new();
    let mut hl: Lines = BTreeMap::new();
    let mut vl: Lines = BTreeMap::new();
    let mut arcs: BTreeMap<(Rgb, bool), String> = BTreeMap::new();
    let mut dots: BTreeMap<(Rgb, bool), String> = BTreeMap::new();
    let mut text: BTreeMap<(u16, Pen), Vec<(u16, char)>> = BTreeMap::new();
    let mut lone = String::new();
    let mut marks: Vec<(Rgb, f64, f64, f64)> = Vec::new();
    for (ry, y) in (a.top()..a.bottom()).enumerate() {
        let top = ry as f64 * CH;
        let mut skip = 0usize;
        // this row's backgrounds, merged: (from col, to col, colour)
        let mut runs: Vec<(u16, u16, Rgb)> = Vec::new();
        // full-width blocks, merged along the row: (from, to, char, colour, opacity, dim)
        let mut bars: Vec<(u16, u16, char, Rgb, f64, bool)> = Vec::new();
        for (rx, x) in (a.left()..a.right()).enumerate() {
            let cell = &buf[(x, y)];
            let rx = rx as u16;
            let m = cell.modifier;
            let (mut fg, mut bgc) = (ink.resolve(cell.fg), ink.resolve(cell.bg));
            if m.contains(Modifier::REVERSED) {
                (fg, bgc) = (bgc.or(Some(ink.base)), fg.or(Some(ink.text)));
            }
            if let Some(b) = bgc {
                match runs.last_mut() {
                    Some((_, to, c)) if *to == rx && *c == b => *to = rx + 1,
                    _ => runs.push((rx, rx + 1, b)),
                }
            }
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = cell.symbol();
            skip = sym.width().saturating_sub(1);
            if cell.skip || m.contains(Modifier::HIDDEN) {
                continue;
            }
            let fg = fg.unwrap_or(ink.text);
            let dim = m.contains(Modifier::DIM);
            let left = rx as f64 * CW;
            let mut cs = sym.chars();
            let (Some(ch), None) = (cs.next(), cs.next()) else {
                if !sym.trim().is_empty() {
                    // a grapheme of several characters: on its own
                    let _ = write!(
                        lone,
                        "<text x=\"{}\" y=\"{}\" fill=\"{}\"{}>{}</text>",
                        n(left),
                        n(top + BASELINE),
                        hex(fg),
                        if dim { " fill-opacity=\".55\"" } else { "" },
                        esc(sym)
                    );
                }
                continue;
            };
            if m.contains(Modifier::UNDERLINED) {
                marks.push((fg, left, top + CH - 2.6, CW * sym.width().max(1) as f64));
            }
            if m.contains(Modifier::CROSSED_OUT) {
                marks.push((fg, left, top + CH * 0.55, CW * sym.width().max(1) as f64));
            }
            if ch == ' ' || ch == '\u{a0}' || ch == '\0' || ch == '\u{2800}' {
                continue;
            }
            if let Some(w) = arms(ch) {
                let (cx, cy) = (left + CW / 2.0, top + CH / 2.0);
                let th = |k: u8| match k {
                    1 => LIGHT,
                    2 => HEAVY,
                    _ => 0.0,
                };
                let vmax = th(w[0]).max(th(w[2]));
                let hmax = th(w[1]).max(th(w[3]));
                // each arm from the centre (past it by half the crossing
                // line, so corners close) to the cell's edge
                let put = |lines: &mut Lines, k: u8, fixed: f64, a0: f64, a1: f64| {
                    if k > 0 {
                        lines
                            .entry((fg, (th(k) * 10.0) as u32, dim))
                            .or_default()
                            .push((tenth(fixed), tenth(a0.min(a1)), tenth(a0.max(a1))));
                    }
                };
                put(&mut vl, w[0], cx, top, cy + hmax / 2.0);
                put(&mut hl, w[1], cy, cx - vmax / 2.0, left + CW);
                put(&mut vl, w[2], cx, cy - hmax / 2.0, top + CH);
                put(&mut hl, w[3], cy, left, cx + vmax / 2.0);
                continue;
            }
            if let Some(d) = arc(ch) {
                let _ = write!(
                    arcs.entry((fg, dim)).or_default(),
                    "<path transform=\"translate({} {})\" d=\"{d}\"/>",
                    n(left),
                    n(top)
                );
                continue;
            }
            if let Some((rects, op)) = block(ch) {
                let wide = rects.len() == 1 && rects[0][0] == 0.0 && rects[0][2] == 1.0;
                if wide {
                    match bars.last_mut() {
                        Some((_, to, c2, f2, o2, d2))
                            if *to == rx && *c2 == ch && *f2 == fg && *o2 == op && *d2 == dim =>
                        {
                            *to = rx + 1
                        }
                        _ => bars.push((rx, rx + 1, ch, fg, op, dim)),
                    }
                } else {
                    for r in rects {
                        let _ = write!(
                            shapes,
                            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"{}/>",
                            n(left + r[0] * CW),
                            n(top + r[1] * CH),
                            n((r[2] - r[0]) * CW),
                            n((r[3] - r[1]) * CH),
                            hex(fg),
                            opacity(op, dim)
                        );
                    }
                }
                continue;
            }
            if let Some(ds) = braille(ch) {
                let e = dots.entry((fg, dim)).or_default();
                for (col, row) in ds {
                    let _ = write!(
                        e,
                        "<circle cx=\"{}\" cy=\"{}\" r=\"1.25\"/>",
                        n(left + CW * (0.25 + 0.5 * col as f64)),
                        n(top + CH * (0.125 + 0.25 * row as f64))
                    );
                }
                continue;
            }
            let pen = Pen {
                fg,
                bold: m.contains(Modifier::BOLD),
                italic: m.contains(Modifier::ITALIC),
                dim,
            };
            if (ch as u32) > 0xffff {
                // outside the BMP: on its own (viewers count those differently)
                let _ = write!(
                    lone,
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\"{}{}{}>{}</text>",
                    n(left),
                    n(top + BASELINE),
                    hex(fg),
                    if pen.bold { " font-weight=\"700\"" } else { "" },
                    if pen.italic {
                        " font-style=\"italic\""
                    } else {
                        ""
                    },
                    if dim { " fill-opacity=\".55\"" } else { "" },
                    esc(&ch.to_string())
                );
                continue;
            }
            text.entry((ry as u16, pen)).or_default().push((rx, ch));
        }
        for (from, to, c) in runs {
            let _ = write!(
                bg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                n(from as f64 * CW),
                n(top),
                n((to - from) as f64 * CW),
                n(CH),
                hex(c)
            );
        }
        for (from, to, ch, fg, op, dim) in bars {
            if let Some((rects, _)) = block(ch) {
                let r = rects[0];
                let _ = write!(
                    shapes,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"{}/>",
                    n(from as f64 * CW),
                    n(top + r[1] * CH),
                    n((to - from) as f64 * CW),
                    n((r[3] - r[1]) * CH),
                    hex(fg),
                    opacity(op, dim)
                );
            }
        }
    }
    let mut out = String::new();
    let _ = write!(out, "<g>{bg}</g><g>{shapes}</g>");
    for (horiz, lines) in [(true, hl), (false, vl)] {
        for ((col, th, dim), segs) in lines {
            let mut d = String::new();
            for (k, a0, a1) in merged(segs) {
                let (k, a0, a1) = (k as f64 / 10.0, a0 as f64 / 10.0, a1 as f64 / 10.0);
                if horiz {
                    let _ = write!(d, "M{} {}H{}", n(a0), n(k), n(a1));
                } else {
                    let _ = write!(d, "M{} {}V{}", n(k), n(a0), n(a1));
                }
            }
            let _ = write!(
                out,
                "<path d=\"{d}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}/>",
                hex(col),
                n(th as f64 / 10.0),
                if dim { " stroke-opacity=\".55\"" } else { "" }
            );
        }
    }
    for ((col, dim), paths) in arcs {
        let _ = write!(
            out,
            "<g fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}>{paths}</g>",
            hex(col),
            n(LIGHT),
            if dim { " stroke-opacity=\".55\"" } else { "" }
        );
    }
    for ((col, dim), cs) in dots {
        let _ = write!(
            out,
            "<g fill=\"{}\"{}>{cs}</g>",
            hex(col),
            if dim { " fill-opacity=\".55\"" } else { "" }
        );
    }
    for (col, x, y, w) in marks {
        let _ = write!(
            out,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"1\" fill=\"{}\"/>",
            n(x),
            n(y),
            n(w),
            hex(col)
        );
    }
    let _ = write!(out, "<g font-family=\"{MONO}\" font-size=\"{FONT_SIZE}\">");
    for ((row, pen), cs) in text {
        let xs: Vec<String> = cs.iter().map(|(x, _)| n(*x as f64 * CW)).collect();
        let s: String = cs.iter().map(|(_, c)| *c).collect();
        let _ = write!(
            out,
            "<text x=\"{}\" y=\"{}\" fill=\"{}\"{}{}{}>{}</text>",
            xs.join(" "),
            n(row as f64 * CH + BASELINE),
            hex(pen.fg),
            if pen.bold { " font-weight=\"700\"" } else { "" },
            if pen.italic {
                " font-style=\"italic\""
            } else {
                ""
            },
            if pen.dim { " fill-opacity=\".55\"" } else { "" },
            esc(&s)
        );
    }
    out.push_str(&lone);
    out.push_str("</g>");
    out
}

fn opacity(op: f64, dim: bool) -> String {
    let o = op * if dim { 0.55 } else { 1.0 };
    if o >= 0.999 {
        String::new()
    } else {
        format!(" fill-opacity=\"{}\"", n(o))
    }
}

// ------------------------------------------------------------------ the card

/// Roughly how wide sans text is (px), to place what follows it.
fn sans_w(s: &str, size: f64, bold: bool) -> f64 {
    s.chars().count() as f64 * size * if bold { 0.6 } else { 0.55 }
}

/// What the footer says about the data shown.
pub fn about(mode: &str) -> &'static str {
    match mode {
        "masked" => "names, folders and addresses are stand-ins",
        "demo" => "the demo world: a made-up fleet",
        _ => "",
    }
}

/// The poster: the screen in its frame. Its size in px with it.
pub fn svg(buf: &Buffer, look: &Look, ink: &Ink) -> (String, f64, f64) {
    let (cols, rows) = (buf.area.width as f64, buf.area.height as f64);
    let (gw, gh) = (cols * CW, rows * CH);
    let screen = screen(buf, ink);
    let (acc, base, text, faint) = (
        hex(ink.accent),
        hex(ink.base),
        hex(ink.text),
        hex(ink.faint),
    );
    let label = format!("Aleph — {}", look.view);
    let mut s = String::new();
    match look.frame {
        Frame::Cockpit => {
            let (m, p, tb, gap, fb) = (40.0, 22.0, 50.0, 12.0, 34.0);
            let (cw, ch) = (gw + 2.0 * p, tb + gap + gh + fb);
            let (w, h) = (cw + 2.0 * m, ch + 2.0 * m);
            let (cx, cy) = (m, m);
            let (gx, gy) = (cx + p, cy + tb + gap);
            // the page in the screen's own base, deeper (graphite, or the
            // cockpit's midnight while showcase.palette has it)
            let mix = |(r, g, b): Rgb, t: f64, k: f64| -> String {
                let m = |x: u8| (x as f64 + (t - x as f64) * k).round() as u8;
                hex((m(r), m(g), m(b)))
            };
            let (pg0, pg1, shadow) = if ink.light {
                (mix(ink.base, 0.0, 0.09), mix(ink.base, 255.0, 0.4), "0.18")
            } else {
                (mix(ink.base, 0.0, 0.4), mix(ink.base, 255.0, 0.05), "0.6")
            };
            let _ = write!(
                s,
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"{}\">",
                n(w), n(h), n(w), n(h), esc(&label)
            );
            let _ = write!(s, "<title>{}</title>", esc(&label));
            let _ = write!(
                s,
                "<defs>\
                 <linearGradient id=\"al-page\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{pg0}\"/><stop offset=\"1\" stop-color=\"{pg1}\"/></linearGradient>\
                 <radialGradient id=\"al-glow\" cx=\"0.1\" cy=\"0\" r=\"0.8\"><stop offset=\"0\" stop-color=\"{acc}\" stop-opacity=\"0.2\"/><stop offset=\"1\" stop-color=\"{acc}\" stop-opacity=\"0\"/></radialGradient>\
                 <radialGradient id=\"al-glow2\" cx=\"1\" cy=\"1\" r=\"0.6\"><stop offset=\"0\" stop-color=\"{acc}\" stop-opacity=\"0.09\"/><stop offset=\"1\" stop-color=\"{acc}\" stop-opacity=\"0\"/></radialGradient>\
                 <linearGradient id=\"al-rule\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\"><stop offset=\"0\" stop-color=\"{acc}\" stop-opacity=\"0\"/><stop offset=\"0.18\" stop-color=\"{acc}\" stop-opacity=\"0.65\"/><stop offset=\"0.82\" stop-color=\"{acc}\" stop-opacity=\"0.65\"/><stop offset=\"1\" stop-color=\"{acc}\" stop-opacity=\"0\"/></linearGradient>\
                 <filter id=\"al-shadow\" x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"130%\"><feGaussianBlur stdDeviation=\"16\"/></filter>\
                 <clipPath id=\"al-screen\"><rect width=\"{}\" height=\"{}\"/></clipPath>\
                 </defs>",
                n(gw), n(gh)
            );
            let _ = write!(
                s,
                "<rect width=\"{}\" height=\"{}\" fill=\"url(#al-page)\"/><rect width=\"{}\" height=\"{}\" fill=\"url(#al-glow)\"/><rect width=\"{}\" height=\"{}\" fill=\"url(#al-glow2)\"/>",
                n(w), n(h), n(w), n(h), n(w), n(h)
            );
            // the card, on its shadow
            let _ = write!(
                s,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"16\" fill=\"#000\" opacity=\"{shadow}\" filter=\"url(#al-shadow)\"/>\
                 <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"16\" fill=\"{base}\" stroke=\"{acc}\" stroke-opacity=\"0.38\"/>",
                n(cx + 6.0), n(cy + 14.0), n(cw - 12.0), n(ch),
                n(cx + 0.5), n(cy + 0.5), n(cw - 1.0), n(ch - 1.0)
            );
            // the title bar: ℵ aleph › view   ✦ showcase          watermark
            let ty = cy + 32.0;
            let _ = write!(
                s,
                "<g transform=\"translate({} {}) scale(0.25)\"><path fill=\"{acc}\" d=\"{MARK}\"/></g>",
                n(cx + p),
                n(cy + 12.5)
            );
            let mut x = cx + p + 36.0;
            let _ = write!(
                s,
                "<g font-family=\"{SANS}\"><text x=\"{}\" y=\"{}\" font-size=\"18\" font-weight=\"700\" fill=\"{text}\">aleph</text>",
                n(x), n(ty)
            );
            x += sans_w("aleph", 18.0, true) + 10.0;
            let _ = write!(
                s,
                "<text x=\"{}\" y=\"{}\" font-size=\"18\" fill=\"{faint}\">›</text>",
                n(x),
                n(ty)
            );
            x += 16.0;
            let _ = write!(
                s,
                "<text x=\"{}\" y=\"{}\" font-size=\"16\" font-weight=\"600\" fill=\"{acc}\">{}</text>",
                n(x),
                n(ty - 0.5),
                esc(&look.view)
            );
            x += sans_w(&look.view, 16.0, true) + 14.0;
            let chip = match look.mode.as_str() {
                "masked" => Some("✦ showcase"),
                "demo" => Some("✦ demo"),
                _ => None,
            };
            if let Some(chip) = chip {
                let w = sans_w(chip, 12.0, true) + 18.0;
                let ink_c = if ink.light { "#ffffff" } else { "#1b1405" };
                let _ = write!(
                    s,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"22\" rx=\"11\" fill=\"{acc}\"/><text x=\"{}\" y=\"{}\" font-size=\"12\" font-weight=\"700\" fill=\"{ink_c}\" text-anchor=\"middle\">{}</text>",
                    n(x), n(ty - 16.0), n(w), n(x + w / 2.0), n(ty - 1.0), esc(chip)
                );
            }
            if let Some(wm) = &look.watermark {
                let _ = write!(
                    s,
                    "<text x=\"{}\" y=\"{}\" font-size=\"15\" font-style=\"italic\" fill=\"{acc}\" text-anchor=\"end\">{}</text>",
                    n(cx + cw - p),
                    n(ty - 1.0),
                    esc(wm)
                );
            }
            s.push_str("</g>");
            let _ = write!(
                s,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"1\" fill=\"url(#al-rule)\"/>",
                n(cx + p),
                n(cy + tb),
                n(cw - 2.0 * p)
            );
            // the screen
            let _ = write!(
                s,
                "<g transform=\"translate({} {})\" clip-path=\"url(#al-screen)\">{screen}</g>",
                n(gx),
                n(gy)
            );
            // the footer: size and time · what the data is
            let fy = cy + ch - 13.0;
            let _ = write!(
                s,
                "<g font-family=\"{MONO}\" font-size=\"12\" fill=\"{faint}\"><text x=\"{}\" y=\"{}\">{} × {} · {}</text><text x=\"{}\" y=\"{}\" text-anchor=\"end\">{}</text></g>",
                n(cx + p), n(fy), cols, rows, esc(&look.when),
                n(cx + cw - p), n(fy), about(&look.mode)
            );
            // HUD corners, just outside the card
            let (o, l) = (10.0, 24.0);
            let (x0, y0, x1, y1) = (cx - o, cy - o, cx + cw + o, cy + ch + o);
            let _ = write!(
                s,
                "<path fill=\"none\" stroke=\"{acc}\" stroke-width=\"2\" stroke-opacity=\"0.85\" stroke-linecap=\"round\" d=\"M{} {}V{}H{}M{} {}H{}V{}M{} {}V{}H{}M{} {}H{}V{}\"/>",
                n(x0), n(y0 + l), n(y0), n(x0 + l),
                n(x1 - l), n(y0), n(x1), n(y0 + l),
                n(x1), n(y1 - l), n(y1), n(x1 - l),
                n(x0 + l), n(y1), n(x0), n(y1 - l)
            );
            s.push_str("</svg>");
            (s, w, h)
        }
        Frame::Plain => {
            let p = 18.0;
            let foot = if look.watermark.is_some() { 22.0 } else { 0.0 };
            let (w, h) = (gw + 2.0 * p, gh + 2.0 * p + foot);
            let _ = write!(
                s,
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"{}\"><title>{}</title>\
                 <defs><clipPath id=\"al-screen\"><rect width=\"{}\" height=\"{}\"/></clipPath></defs>\
                 <rect width=\"{}\" height=\"{}\" rx=\"12\" fill=\"{base}\"/>",
                n(w), n(h), n(w), n(h), esc(&label), esc(&label), n(gw), n(gh), n(w), n(h)
            );
            let _ = write!(
                s,
                "<g transform=\"translate({p} {p})\" clip-path=\"url(#al-screen)\">{screen}</g>"
            );
            if let Some(wm) = &look.watermark {
                let _ = write!(
                    s,
                    "<text x=\"{}\" y=\"{}\" font-family=\"{SANS}\" font-size=\"12\" font-style=\"italic\" fill=\"{acc}\" text-anchor=\"end\">✦ {}</text>",
                    n(w - p),
                    n(h - 13.0),
                    esc(wm)
                );
            }
            s.push_str("</svg>");
            (s, w, h)
        }
    }
}

// ------------------------------------------------------------------ PNG

/// What came of a poster's PNG: Some((it, the tool that made it)), None
/// (no converter, or off), or why it failed.
pub type Png = Result<Option<(PathBuf, &'static str)>, String>;

/// A PNG beside the SVG at twice its size, by a converter already on this
/// computer (`how`: auto, rsvg-convert, qlmanage, off): Some((the PNG, the
/// tool)), None when there's none (or off). Blocking: off the UI thread.
pub fn png(svg: &Path, how: &str, width: f64) -> Png {
    let out = svg.with_extension("png");
    let tools: &[&'static str] = match how.trim() {
        "off" | "none" | "" => &[],
        "rsvg-convert" => &["rsvg-convert"],
        "qlmanage" => &["qlmanage"],
        _ => &["rsvg-convert", "qlmanage"],
    };
    let picked = how.trim() == "rsvg-convert" || how.trim() == "qlmanage";
    for &t in tools {
        if t == "qlmanage" && !cfg!(target_os = "macos") {
            continue;
        }
        if crate::proc::which(t).is_none() {
            if picked {
                return Err(format!("{t} isn't on this computer (showcase.poster_png)"));
            }
            continue;
        }
        let r = match t {
            "rsvg-convert" => crate::proc::run(
                std::process::Command::new(t)
                    .args(["--zoom", "2", "--format", "png", "--output"])
                    .arg(&out)
                    .arg(svg),
                Duration::from_secs(30),
                None,
            ),
            _ => {
                // qlmanage draws an SVG into a square and cuts off the rest:
                // hand it a square copy with the poster centred, then crop
                // its picture back to the poster's own shape (sips, macOS's)
                let tmp = svg.with_extension("qlmanage");
                let _ = std::fs::create_dir_all(&tmp);
                let text = std::fs::read_to_string(svg).unwrap_or_default();
                let (copy, w, h) = match squared(&text) {
                    Some((sq, w, h)) => {
                        let p = tmp.join("poster.svg");
                        let _ = std::fs::write(&p, sq);
                        (p, w, h)
                    }
                    None => (svg.to_path_buf(), width, width),
                };
                let side = (w.max(h) * 2.0).round() as u64;
                let r = crate::proc::run(
                    std::process::Command::new(t)
                        .args(["-t", "-s", &side.to_string(), "-o"])
                        .arg(&tmp)
                        .arg(&copy),
                    Duration::from_secs(30),
                    None,
                );
                let made = copy
                    .file_name()
                    .map(|f| tmp.join(format!("{}.png", f.to_string_lossy())));
                let moved = made.is_some_and(|m| std::fs::rename(m, &out).is_ok());
                let _ = std::fs::remove_dir_all(&tmp);
                if moved && (w - h).abs() > 0.5 {
                    let (pw, ph) = ((w * 2.0).round() as u64, (h * 2.0).round() as u64);
                    let _ = crate::proc::run(
                        std::process::Command::new("sips")
                            .args(["--cropToHeightWidth", &ph.to_string(), &pw.to_string()])
                            .arg(&out),
                        Duration::from_secs(30),
                        None,
                    );
                }
                match r {
                    Ok(o) if !moved => Err(std::io::Error::other(format!(
                        "qlmanage made no picture ({})",
                        o.text().trim()
                    ))),
                    r => r,
                }
            }
        };
        return match r {
            Ok(o) if o.ok() && out.exists() => Ok(Some((out, t))),
            Ok(o) => Err(format!("{t}: {}", o.text().trim())),
            Err(e) => Err(format!("{t}: {e}")),
        };
    }
    Ok(None)
}

/// The poster as a square SVG (for qlmanage), the poster centred on its
/// own page colour; with its width and height. None if its size can't be read.
fn squared(svg: &str) -> Option<(String, f64, f64)> {
    let open_end = svg.find('>')?;
    let root = &svg[..open_end];
    let attr = |name: &str| -> Option<f64> {
        let at = root.find(&format!(" {name}=\""))? + name.len() + 3;
        root[at..].split('"').next()?.parse().ok()
    };
    let (w, h) = (attr("width")?, attr("height")?);
    let inner = svg[open_end + 1..].trim_end().strip_suffix("</svg>")?;
    let s = w.max(h);
    let (x, y) = ((s - w) / 2.0, (s - h) / 2.0);
    Some((
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{s}\" height=\"{s}\" viewBox=\"0 0 {s} {s}\">\
             <rect width=\"{s}\" height=\"{s}\" fill=\"#060811\"/>\
             <svg x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">{inner}</svg></svg>"
        ),
        w,
        h,
    ))
}

#[cfg(test)]
mod tests {
    use super::Frame;
    #[test]
    fn a_square_copy_centres_the_poster_for_qlmanage() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\" viewBox=\"0 0 200 100\"><rect width=\"200\" height=\"100\"/></svg>";
        let (sq, w, h) = squared(svg).unwrap();
        assert_eq!((w, h), (200.0, 100.0));
        assert!(
            sq.contains("width=\"200\" height=\"200\" viewBox=\"0 0 200 200\""),
            "{sq}"
        );
        assert!(
            sq.contains("<svg x=\"0\" y=\"50\" width=\"200\" height=\"100\""),
            "{sq}"
        );
        assert!(
            sq.contains("<rect width=\"200\" height=\"100\"/></svg></svg>"),
            "{sq}"
        );
        assert!(squared("not an svg").is_none());
    }

    /// A real conversion, to look at (`POSTER_SVG=file.svg cargo test
    /// --features showcase poster_png_is_whole -- --ignored`).
    #[test]
    #[ignore]
    fn poster_png_is_whole() {
        let Some(p) = std::env::var_os("POSTER_SVG") else {
            return;
        };
        let r = png(std::path::Path::new(&p), "auto", 0.0);
        println!("{r:?}");
        assert!(matches!(r, Ok(Some(_))));
    }

    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::prelude::*;
    use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
    use ratatui::Terminal;

    fn ink() -> Ink {
        Ink::of(&c::Pal::dark())
    }

    fn panel() -> Buffer {
        let mut t = Terminal::new(TestBackend::new(30, 6)).unwrap();
        t.draw(|f| {
            let b = Block::new()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" ℵ Fleet ")
                .border_style(Style::new().fg(Color::Rgb(0x3a, 0x3a, 0x3a)));
            let inner = b.inner(f.area());
            f.render_widget(b, f.area());
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled("b31 ", Style::new().fg(Color::Rgb(0xe2, 0xb4, 0x51)).bold()),
                        Span::styled("<working> & ", Style::new().fg(Color::Blue)),
                        Span::styled("████▌", Style::new().fg(Color::Green)),
                    ]),
                    Line::from(Span::styled(
                        " needs you ",
                        Style::new().add_modifier(Modifier::REVERSED),
                    )),
                    Line::from("⣿⡇ 漢字 🚀"),
                ]),
                inner,
            );
        })
        .unwrap();
        t.backend().buffer().clone()
    }

    #[test]
    fn the_screen_becomes_shapes_and_placed_text() {
        let s = screen(&panel(), &ink());
        // text is escaped, each character in its cell
        assert!(s.contains("&lt;working&gt;"), "{s}");
        assert!(s.contains("&amp;"));
        assert!(!s.contains("<working>"));
        // the gold, bold
        assert!(s.contains("fill=\"#e2b451\" font-weight=\"700\""), "{s}");
        // the rounded corners as arcs, the borders as merged lines
        assert!(s.matches("<path transform=").count() == 4, "{s}");
        assert!(s.contains("stroke=\"#3a3a3a\""));
        // a bar of four █ is one rectangle, the ▌ half a cell
        assert!(
            s.contains(&format!(
                "width=\"{}\" height=\"{}\" fill=\"#46a758\"",
                n(4.0 * CW),
                n(CH)
            )),
            "{s}"
        );
        assert!(s.contains(&format!(
            "width=\"{}\" height=\"{}\" fill=\"#46a758\"",
            n(CW / 2.0),
            n(CH)
        )));
        // reversed: the text colour behind, the base in front
        assert!(
            s.contains("fill=\"#eeeeee\"/>"),
            "a background run in the text colour"
        );
        assert!(s.contains("fill=\"#111111\">needs"), "{s}");
        // Braille as dots: ⣿ eight, ⡇ four
        assert_eq!(s.matches("<circle").count(), 12);
        // wide characters keep their place; the astral one stands alone
        assert!(s.contains("漢字"));
        assert!(s.contains(">🚀</text>"));
        // the terminal's own background is never painted on the screen
        assert!(!s.contains("fill=\"#111111\"/>"));
    }

    #[test]
    fn x_positions_follow_the_cells() {
        let mut t = Terminal::new(TestBackend::new(10, 1)).unwrap();
        t.draw(|f| f.render_widget(Paragraph::new("ab  c"), f.area()))
            .unwrap();
        let s = screen(t.backend().buffer(), &ink());
        // the spaces aren't characters of the run, their cells still count
        assert!(
            s.contains(&format!("x=\"0 {} {}\"", n(CW), n(4.0 * CW))),
            "{s}"
        );
        assert!(s.contains(">abc</text>"));
    }

    #[test]
    fn frames_carry_the_mark_the_view_and_the_watermark() {
        let b = panel();
        let mut look = Look {
            frame: Frame::Cockpit,
            view: "Fleet".into(),
            mode: "masked".into(),
            watermark: Some("made by <me>".into()),
            when: "2026-10-23 14:05".into(),
        };
        let (s, w, h) = svg(&b, &look, &ink());
        assert!(s.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(s.ends_with("</svg>"));
        assert!(s.contains(MARK));
        assert!(s.contains(">Fleet</text>") && s.contains("✦ showcase"));
        assert!(s.contains("made by &lt;me&gt;"));
        assert!(s.contains("stand-ins"));
        assert!(s.contains("30 × 6 · 2026-10-23 14:05"));
        assert!(w > 30.0 * CW && h > 6.0 * CH);
        // balanced: every opened tag closes
        assert_eq!(s.matches("<g").count(), s.matches("</g>").count());
        assert_eq!(s.matches("<text").count(), s.matches("</text>").count());
        look.frame = Frame::Plain;
        look.mode = "demo".into();
        look.watermark = None;
        let (p, pw, ph) = svg(&b, &look, &ink());
        assert!(!p.contains(MARK) && !p.contains("showcase"));
        assert_eq!((pw, ph), (30.0 * CW + 36.0, 6.0 * CH + 36.0));
        assert_eq!(Frame::parse("plain"), Some(Frame::Plain));
        assert_eq!(Frame::parse("fancy"), None);
    }

    #[test]
    fn colours_resolve_like_a_terminal() {
        let i = ink();
        assert_eq!(i.resolve(Color::Reset), None);
        assert_eq!(i.resolve(Color::Rgb(1, 2, 3)), Some((1, 2, 3)));
        assert_eq!(i.resolve(Color::Red), Some((0xe5, 0x48, 0x4d)));
        assert_eq!(i.resolve(Color::Indexed(16)), Some((0, 0, 0)));
        assert_eq!(i.resolve(Color::Indexed(231)), Some((255, 255, 255)));
        assert_eq!(i.resolve(Color::Indexed(232)), Some((8, 8, 8)));
        assert_eq!(i.palette().len(), 16);
        // NO_COLOR's palette poses as graphite
        let m = Ink::of(&c::Pal::mono());
        assert_eq!(m.base, (0x11, 0x11, 0x11));
        assert_eq!(braille('\u{2800}').unwrap().len(), 0);
        assert_eq!(block('▁').unwrap().0[0][1], 1.0 - 1.0 / 8.0);
        assert_eq!(block('▉').unwrap().0[0][2], 7.0 / 8.0);
        assert_eq!(block('▏').unwrap().0[0][2], 1.0 / 8.0);
    }

    #[test]
    fn no_converter_no_png() {
        let d = std::env::temp_dir().join(format!("aleph-poster-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let f = d.join("x.svg");
        std::fs::write(&f, "<svg/>").unwrap();
        assert_eq!(png(&f, "off", 100.0), Ok(None));
    }
}
