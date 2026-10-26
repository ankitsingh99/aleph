//! Drawing helpers the celebrations and the HUD share — the splash's own
//! ideas (`tui/splash.rs`: smooth steps, colour ramps, hashes, braille
//! sub-cells) — and the one rule for whatever they put down: the foreground
//! only. A glyph goes only into a blank cell (one with no background of its
//! own); text the light passes over only changes colour.

use crate::ui::c;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

pub type Rgb = (u8, u8, u8);

/// 0 before `a`, 1 after `b`, smoothly between.
pub fn smooth(t: f64, a: f64, b: f64) -> f64 {
    let x = ((t - a) / (b - a)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub fn mix(a: Rgb, b: Rgb, k: f64) -> Rgb {
    let k = k.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * k).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Along a ramp of colours, 0 to 1.
pub fn ramp(stops: &[Rgb], f: f64) -> Rgb {
    let f = f.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let i = (f.floor() as usize).min(stops.len() - 2);
    mix(stops[i], stops[i + 1], f - i as f64)
}

pub fn hash(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A string's hash (a session's key: its own sparkle pattern).
pub fn hash_str(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// A braille cell drawn from `f` (a brightness at a point, in cells): each
/// of its eight dots is up when `f` there beats a Bayer threshold, so a
/// gradient comes out even. The glyph, and the brightest dot that's up.
pub fn braille_at(x: f64, y: f64, f: impl Fn(f64, f64) -> f64) -> Option<(char, f64)> {
    const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    const ORDER: [[f64; 4]; 2] = [[0.0, 6.0, 1.0, 7.0], [4.0, 2.0, 5.0, 3.0]];
    let (mut bits, mut top) = (0u32, 0f64);
    for (i, col) in BITS.iter().enumerate() {
        for (j, bit) in col.iter().enumerate() {
            let val = f(x - 0.25 + i as f64 * 0.5, y - 0.375 + j as f64 * 0.25);
            if val > (ORDER[i][j] + 0.5) / 8.0 {
                bits |= bit;
                top = top.max(val);
            }
        }
    }
    (bits != 0).then(|| (char::from_u32(0x2800 + bits).unwrap_or(' '), top))
}

/// A colour `k` of the way from the background's own darkness (or, on a
/// light terminal, paper) toward `rgb`: how light fades in and out here.
pub fn glow(rgb: Rgb, k: f64) -> Color {
    let pal = c::pal();
    if pal.mono {
        return Color::Reset;
    }
    let (from, to) = if pal.light {
        ((228, 224, 214), mix(rgb, (40, 30, 60), 0.45))
    } else {
        ((16, 16, 22), rgb)
    };
    let (r, g, b) = mix(from, to, k);
    Color::Rgb(r, g, b)
}

pub fn rgb_of(col: Color) -> Option<Rgb> {
    match col {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

/// The palette's colour as (r, g, b), or `or` (NO_COLOR, a named colour).
pub fn pal_rgb(col: Color, or: Rgb) -> Rgb {
    rgb_of(col).unwrap_or(or)
}

/// Nothing drawn at (x, y): a space with no background of its own (a pill's
/// padding isn't blank).
pub fn blank(buf: &Buffer, x: u16, y: u16) -> bool {
    buf.cell((x, y))
        .is_some_and(|c| !c.skip && c.symbol() == " " && c.bg == Color::Reset)
}

/// `sym` into a blank cell, in `fg`: false (and nothing done) when
/// something's already there.
pub fn put(buf: &mut Buffer, x: u16, y: u16, sym: &str, fg: Color) -> bool {
    if !blank(buf, x, y) {
        return false;
    }
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(sym).set_fg(fg);
    }
    true
}

/// Which cells of an area are open sky: blank, and so are the cells either
/// side of them (a single space between two words is the text's own), as
/// the frame was before an effect began drawing into it.
pub struct Sky {
    r: Rect,
    open: Vec<bool>,
}

impl Sky {
    pub fn of(buf: &Buffer, r: Rect) -> Sky {
        let r = r.intersection(buf.area);
        let side =
            |x: u16, y: u16| x < buf.area.left() || x >= buf.area.right() || blank(buf, x, y);
        let mut open = Vec::with_capacity(r.area() as usize);
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                open.push(
                    blank(buf, x, y) && side(x.wrapping_sub(1), y) && side(x.saturating_add(1), y),
                );
            }
        }
        Sky { r, open }
    }

    pub fn open(&self, x: u16, y: u16) -> bool {
        if !self.r.contains((x, y).into()) {
            return false;
        }
        let i = (y - self.r.y) as usize * self.r.width as usize + (x - self.r.x) as usize;
        self.open.get(i).copied().unwrap_or(false)
    }

    /// `sym` at (x, y) if it's open sky (and still blank).
    pub fn put(&self, buf: &mut Buffer, x: u16, y: u16, sym: &str, fg: Color) -> bool {
        self.open(x, y) && put(buf, x, y, sym, fg)
    }
}

/// What's drawn at (x, y) — never a blank — lit `k` of the way toward `rgb`
/// (its glyph, background and style stay).
pub fn tint(buf: &mut Buffer, x: u16, y: u16, rgb: Rgb, k: f64) {
    if k <= 0.02 || c::mono() {
        return;
    }
    let text = pal_rgb(c::TEXT(), (238, 238, 238));
    if let Some(cell) = buf.cell_mut((x, y)) {
        if cell.skip || cell.symbol() == " " {
            return;
        }
        let from = rgb_of(cell.fg).unwrap_or(text);
        let (r, g, b) = mix(from, rgb, k);
        cell.set_fg(Color::Rgb(r, g, b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    #[test]
    fn open_sky_isnt_the_space_between_two_words() {
        let mut b = Buffer::empty(Rect::new(0, 0, 20, 2));
        b.set_string(2, 0, "task of temper", Style::new());
        let sky = Sky::of(&b, b.area);
        assert!(!sky.open(6, 0), "the space in “task of”");
        assert!(sky.open(0, 0) && sky.open(18, 0) && sky.open(5, 1));
        assert!(!sky.open(1, 0), "right beside a word");
        assert!(!sky.open(30, 0));
        // what it says holds while the effect draws (a streak stays whole)
        for x in 0..20 {
            sky.put(&mut b, x, 1, "─", Color::Red);
        }
        assert_eq!(
            (0..20).map(|x| b[(x, 1)].symbol()).collect::<String>(),
            "─".repeat(20)
        );
    }

    #[test]
    fn glyphs_go_only_where_nothing_is() {
        let mut b = Buffer::empty(Rect::new(0, 0, 10, 2));
        b.set_string(0, 0, "ab", Style::new());
        b.set_string(4, 0, " ", Style::new().bg(Color::Blue));
        assert!(!put(&mut b, 0, 0, "✦", Color::Red));
        assert_eq!(b[(0, 0)].symbol(), "a");
        // a pill's padding is somebody's
        assert!(!put(&mut b, 4, 0, "✦", Color::Red));
        assert!(put(&mut b, 6, 0, "✦", Color::Red));
        assert_eq!(b[(6, 0)].symbol(), "✦");
        assert_eq!(b[(6, 0)].bg, Color::Reset);
        // off the buffer: nothing
        assert!(!put(&mut b, 40, 9, "✦", Color::Red));
        // a tint keeps the glyph and changes its colour
        tint(&mut b, 1, 0, (255, 0, 0), 1.0);
        assert_eq!(b[(1, 0)].symbol(), "b");
        assert_eq!(b[(1, 0)].fg, Color::Rgb(255, 0, 0));
        // and leaves blanks blank
        tint(&mut b, 8, 1, (255, 0, 0), 1.0);
        assert_eq!(b[(8, 1)].fg, Color::Reset);
    }

    #[test]
    fn ramps_and_braille() {
        assert_eq!(ramp(&[(0, 0, 0), (200, 100, 0)], 0.5), (100, 50, 0));
        assert_eq!(ramp(&[(0, 0, 0), (200, 100, 0)], 9.0), (200, 100, 0));
        assert!(braille_at(0.0, 0.0, |_, _| 0.0).is_none());
        let (ch, top) = braille_at(0.0, 0.0, |_, _| 1.0).unwrap();
        assert_eq!(ch, '⣿');
        assert_eq!(top, 1.0);
        assert_ne!(hash_str("b31|w1:p1"), hash_str("b31|w1:p2"));
        assert_eq!(smooth(5.0, 0.0, 1.0), 1.0);
    }
}
