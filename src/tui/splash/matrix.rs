//! `matrix`: inside the matrix. Digital rain pours down the screen —
//! columns of katakana, digits and letters, each falling at its own speed,
//! a white-green glyph at its head and a trail fading behind it, its glyphs
//! churning. Your machines' names surface in the code, lit for a moment.
//! Then déjà vu: a band of the rain repeats itself (the same rows twice,
//! and then again), and a wave of light ripples across the columns. The
//! code slows and freezes, the columns hanging mid-fall; then it decodes:
//! the columns round the middle lock into ALEPH, written in lit code, while
//! the rest drains away. With the logo they lock into the ℵ instead, which
//! turns to gold, and the rain runs on through it while it is held.

use super::scene::{ismooth, Frame, GATHER_S};
use super::*;

type Rgb = (u8, u8, u8);

pub(super) const TITLE: &str = "the code beneath it all";
/// Its colours on the way into the ℵ: the code's pale green, then gold.
pub(super) const COLS: [Rgb; 2] = [(150, 255, 176), (255, 214, 120)];

/// Phosphor, a trail's tail to its head: deep green, green, bright green,
/// pale green, white-green.
const RAIN: [Rgb; 5] = [
    (0, 80, 34),
    (0, 160, 62),
    (40, 225, 102),
    (150, 255, 176),
    (232, 255, 238),
];
const HEAD: Rgb = (240, 255, 244);

/// The acts, as shares of the scene: the names surfacing; déjà vu (the
/// band repeating itself, twice, and the wave across the columns); the
/// freeze; the columns locking into the word (or the ℵ) while the rest
/// drains away.
const NAMES: (f64, f64) = (0.06, 0.44);
const BAND: [(f64, f64); 2] = [(0.42, 0.455), (0.48, 0.505)];
const WAVE: (f64, f64) = (0.46, 0.6);
const FREEZE: (f64, f64) = (0.56, 0.71);
const LOCK: (f64, f64) = (0.73, 0.93);
const DRAIN: (f64, f64) = (0.73, 0.98);
/// How long a column's lock takes to run down it, and a drained glyph to
/// fall out of sight.
const SWEEP: f64 = 0.05;
const FALL: f64 = 0.12;

/// ALEPH, five pixels by seven a letter.
const FONT: [[&str; 7]; 5] = [
    [
        ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
    ],
    [
        "#....", "#....", "#....", "#....", "#....", "#....", "#####",
    ],
    [
        "#####", "#....", "#....", "####.", "#....", "#....", "#####",
    ],
    [
        "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
    ],
    [
        "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
    ],
];
/// The word's width in pixels: five letters and the gaps between them.
const WORD: i32 = 5 * 5 + 4;

/// The rain's clock: running, slowing (closed form), stopped.
fn clock(u: f64) -> f64 {
    u - ismooth(u, FREEZE.0, FREEZE.1)
}

/// The rows the rain falls through (the wordmark and the console below).
fn rows(f: &Frame) -> i32 {
    (f.h as i32 - 3).max(1)
}

/// A cell's own key, for hashing.
fn key(x: i32, y: i32) -> u64 {
    (x as u32 as u64) << 20 | y as u32 as u64
}

/// One glyph of the code: halfwidth katakana mostly, digits, a few letters
/// and signs (all one column).
fn glyph(h: u64) -> char {
    const SIGNS: &[u8] = b"ZXRTKMAEL:.=*+<>|";
    let r = h % 100;
    let i = (h >> 8) as u32;
    if r < 66 {
        char::from_u32(0xFF66 + i % 56).unwrap_or('0')
    } else if r < 88 {
        char::from(b'0' + (i % 10) as u8)
    } else {
        char::from(SIGNS[(i as usize) % SIGNS.len()])
    }
}

/// One drop's way down a column: its speed (rows a scene), its trail's
/// length, its cycle (the screen, the trail, a gap), where in it the scene
/// finds it, how fast its glyphs churn.
#[derive(Clone, Copy)]
struct Lane {
    v: f64,
    len: f64,
    period: f64,
    phase: f64,
    churn: f64,
}

/// Column `x`'s drops (every other column carries rain, as in the film).
fn lanes(f: &Frame, rows: f64, x: i32) -> [Option<Lane>; 2] {
    if x % 2 != 0 {
        return [None, None];
    }
    let base = rows.max(12.0);
    let mk = |lane: u64| {
        let h = hash(f.seed ^ 0x6d61_7472, (x as u64) << 2 | lane);
        if lane == 1 && unit(h, 0) < 0.45 {
            return None;
        }
        let len = (rows * (0.2 + 0.5 * unit(h, 2).powf(0.9))).max(4.0);
        let gap = rows * (0.25 + 1.1 * unit(h, 3));
        let period = rows + len + gap;
        Some(Lane {
            v: base * (2.4 + 3.6 * unit(h, 1)),
            len,
            period,
            phase: unit(h, 4) * period,
            churn: 6.0 + 30.0 * unit(h, 5),
        })
    };
    [mk(0), mk(1)]
}

/// The rain at `(x, y)` by the clock `tau`: its glyph, how far along its
/// trail (1 the head, toward 0 its tail), whether it's a head. The drops
/// are all over the screen from the first moment, their trails growing
/// behind them as they fall.
fn cell(
    f: &Frame,
    cols: &[[Option<Lane>; 2]],
    x: i32,
    y: i32,
    tau: f64,
) -> Option<(char, f64, bool)> {
    if x < 0 || y < 0 || x as usize >= cols.len() {
        return None;
    }
    let mut best: Option<(char, f64, bool)> = None;
    for (li, lane) in cols[x as usize].iter().enumerate() {
        let Some(l) = lane else { continue };
        let fell = l.v * tau;
        let s = l.phase + fell;
        let cyc = (s / l.period).floor();
        let head = (s - cyc * l.period).floor() - 1.0;
        let d = head - y as f64;
        if d < 0.0 || d >= l.len.min(fell + 1.0) {
            continue;
        }
        let q = 1.0 - d / l.len;
        if best.is_some_and(|b| b.1 >= q) {
            continue;
        }
        let c = (cyc as u64) << 1 | li as u64;
        let k = key(x, y) ^ c << 44;
        let g = if d < 1.0 {
            // the head writes a new glyph every moment
            glyph(hash(f.seed ^ 0x4ead, k ^ ((tau * 160.0) as u64) << 52))
        } else {
            let own = unit(f.seed ^ 0x0c4e, k);
            let ep = (tau * l.churn * (0.25 + 1.75 * own) + own * 7.0).floor() as u64;
            glyph(hash(f.seed ^ 0x9a1f, k ^ ep << 52))
        };
        best = Some((g, q, d < 1.0));
    }
    best
}

/// What the code resolves into at the end: ALEPH written large (each of its
/// pixels a few glyphs), or with the logo the ℵ's cells.
struct Code {
    /// the cells, and a grid of them (`w` wide, from row 0)
    cells: Vec<(i32, i32)>,
    grid: Vec<bool>,
    w: i32,
    /// its box
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
}

/// The word's pixel (columns, rows), as large as the screen allows.
fn scale(f: &Frame) -> (i32, i32) {
    let rows = rows(f);
    let py = match rows {
        r if r >= 45 => 3,
        r if r >= 22 => 2,
        _ => 1,
    };
    let px = ((f.w * 0.8 / WORD as f64).floor() as i32).clamp(1, 2 * py);
    (px, py)
}

/// Where the word goes: its top left, and its pixel's size.
fn word(f: &Frame) -> (i32, i32, i32, i32) {
    let (px, py) = scale(f);
    (
        f.cx as i32 - WORD * px / 2,
        f.cy as i32 - 7 * py / 2,
        px,
        py,
    )
}

impl Code {
    fn of(f: &Frame) -> Code {
        let cells: Vec<(i32, i32)> = if f.opts.logo {
            let (_, _, _, _, cells) = logo_box(f.w, f.h, f.cx, f.cy);
            cells
                .into_iter()
                .filter(|c| c.2.count_ones() >= 3)
                .map(|c| (c.0, c.1))
                .collect()
        } else {
            let (x0, y0, px, py) = word(f);
            let mut v = Vec::new();
            for (l, letter) in FONT.iter().enumerate() {
                for (r, line) in letter.iter().enumerate() {
                    for (c, b) in line.bytes().enumerate() {
                        if b != b'#' {
                            continue;
                        }
                        let x = x0 + (l as i32 * 6 + c as i32) * px;
                        let y = y0 + r as i32 * py;
                        for (i, j) in (0..px).flat_map(|i| (0..py).map(move |j| (i, j))) {
                            v.push((x + i, y + j));
                        }
                    }
                }
            }
            v
        };
        let w = f.w as i32;
        let rows = rows(f);
        let mut grid = vec![false; (w * rows).max(0) as usize];
        let (mut x0, mut x1, mut y0, mut y1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        let cells: Vec<(i32, i32)> = cells
            .into_iter()
            .filter(|&(x, y)| x >= 0 && y >= 0 && x < w && y < rows)
            .collect();
        for &(x, y) in &cells {
            grid[(y * w + x) as usize] = true;
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
        Code {
            cells,
            grid,
            w,
            x0,
            x1,
            y0,
            y1,
        }
    }

    fn has(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.w
            && self
                .grid
                .get((y * self.w + x) as usize)
                .copied()
                .unwrap_or(false)
    }

    /// When column `x` locks (the middle first, outward).
    fn lock_at(&self, f: &Frame, x: i32) -> f64 {
        let half = ((self.x1 - self.x0) as f64 / 2.0).max(1.0);
        let side = ((x as f64 - f.cx).abs() / half).min(1.0);
        let jit = unit(f.seed ^ 0x10c, x as u64);
        LOCK.0 + (LOCK.1 - LOCK.0 - SWEEP) * (0.72 * side + 0.28 * jit)
    }

    /// The row the lock has run down to in column `x`.
    fn head(&self, f: &Frame, x: i32) -> f64 {
        let s = (f.u() - self.lock_at(f, x)) / SWEEP;
        self.y0 as f64 - 1.0 + s * (self.y1 - self.y0 + 3) as f64
    }

    /// Has the lock reached `(x, y)`?
    fn locked(&self, f: &Frame, x: i32, y: i32) -> bool {
        self.has(x, y) && f.u() >= LOCK.0 && y as f64 <= self.head(f, x)
    }
}

pub(super) fn draw(cv: &mut Canvas, f: &Frame) {
    let fade = f.fade();
    if fade <= 0.0 {
        return;
    }
    let rows = rows(f);
    let cols: Vec<[Option<Lane>; 2]> = (0..f.w as i32).map(|x| lanes(f, rows as f64, x)).collect();
    let code = Code::of(f);
    rain(cv, f, &cols, &code, fade);
    names(cv, f, fade);
    lock(cv, f, &code, fade);
}

/// The band that repeats itself in déjà vu: its first row, how many, and
/// how far it slips sideways; while it's on.
fn band(f: &Frame) -> Option<(i32, i32, i32)> {
    let u = f.u();
    let rows = rows(f);
    let n = (rows / 4).max(2);
    let b0 = (rows as f64 * (0.42 + 0.12 * unit(f.seed, 0xba0d))) as i32;
    BAND.iter()
        .enumerate()
        .find(|(_, (a, b))| (*a..*b).contains(&u))
        .map(|(i, _)| (b0.min(rows - n).max(n), n, if i == 0 { 1 } else { -1 }))
}

/// The wave of light across the columns at `(x, y)`, 0–1.
fn wave(f: &Frame, x: i32, y: i32) -> f64 {
    let u = f.u();
    let on = window(u, WAVE.0, WAVE.0 + 0.012, WAVE.1 - 0.015, WAVE.1);
    if on <= 0.0 {
        return 0.0;
    }
    let q = (u - WAVE.0) / (WAVE.1 - WAVE.0);
    let wid = f.w * 0.07 + 4.0;
    // it ripples: a little ahead in some rows, behind in others
    let xw = -wid * 2.0 + (f.w + wid * 4.0) * q + 2.5 * (y as f64 * 0.45 + q * 9.0).sin();
    on * (-((x as f64 - xw) / wid).powi(2)).exp()
}

/// When column `x`'s glyphs let go (the outer columns first) and how far
/// one at row `y` has fallen since (rows), with what's left of its light.
fn drained(f: &Frame, x: i32, y: i32, rows: f64) -> (f64, f64) {
    let u = f.u();
    if u <= DRAIN.0 {
        return (0.0, 1.0);
    }
    let side = ((x as f64 - f.cx).abs() / (f.w / 2.0)).min(1.0);
    let room = DRAIN.1 - DRAIN.0 - FALL - 0.03;
    let rel = DRAIN.0 + room * (1.0 - side) + 0.02 * unit(f.seed ^ 0xd7, x as u64);
    let s = u - rel - 0.01 * unit(f.seed ^ 0xd8, key(x, y));
    if s <= 0.0 {
        return (0.0, 1.0);
    }
    let g = (rows + 2.0) / (FALL * FALL);
    (g * s * s, 1.0 - 0.55 * smooth(s, 0.0, FALL))
}

/// The rain: every column's drops, déjà vu's band and wave, frozen, then
/// draining away round the code as it locks.
fn rain(cv: &mut Canvas, f: &Frame, cols: &[[Option<Lane>; 2]], code: &Code, fade: f64) {
    let rows = rows(f);
    let tau = clock(f.u());
    let band = band(f);
    let frozen = f.act(FREEZE.1 - 0.04, FREEZE.1);
    for x in 0..f.w as i32 {
        for y in 0..rows {
            let (sx, sy, slip) = match band {
                Some((b0, n, s)) if (b0..b0 + n).contains(&y) => (x - s, y - n, true),
                _ => (x, y, false),
            };
            let lit = wave(f, x, y);
            // the wave shows the code that's there between the drops too
            let ghost = || {
                (lit > 0.1 && sx % 2 == 0).then(|| {
                    let ep = (tau * 40.0) as u64;
                    let g = glyph(hash(f.seed ^ 0x9405, key(sx, sy) ^ ep << 40));
                    (g, -1.0, false)
                })
            };
            let Some((g, q, head)) = cell(f, cols, sx, sy, tau).or_else(ghost) else {
                continue;
            };
            // the code's own cells hang where they are until the lock takes them
            let target = code.has(x, y);
            if target && code.locked(f, x, y) {
                continue;
            }
            let (dy, left) = if target {
                (0.0, 1.0)
            } else {
                drained(f, x, y, rows as f64)
            };
            let yy = y + dy.round() as i32;
            if yy >= rows {
                continue;
            }
            let own = unit(f.seed ^ 0x5a, key(sx, sy) ^ ((tau * 30.0) as u64) << 40);
            let (mut col, mut k) = if q < 0.0 {
                (RAIN[2], lit * 0.7)
            } else if head {
                (HEAD, 1.0)
            } else if own < 0.05 {
                // a glyph flaring now and then
                (RAIN[3], 0.95)
            } else {
                (
                    ramp(&RAIN, 0.1 + 0.78 * q.powf(1.3)),
                    0.3 + 0.68 * q.powf(1.1),
                )
            };
            // frozen, it holds its breath: the dim code brighter, all of it held
            col = mix(col, RAIN[3], 0.25 * frozen * (1.0 - q.max(0.0)));
            k = (k * (1.0 + 0.3 * frozen)).min(1.0);
            if slip {
                // the band that's been here before: brighter, paler
                col = mix(col, HEAD, 0.35);
                k = (k * 1.25 + 0.1).min(1.0);
            }
            col = mix(col, HEAD, lit * 0.8);
            k = (k + lit * 0.6).min(1.0) * left * fade;
            let mut b = [0u8; 4];
            cv.put_with(x, yy, g.encode_utf8(&mut b), col, k, head || k > 0.85);
        }
    }
}

/// Your machines' names surfacing in the rain: decoded letter by letter
/// out of churning glyphs, lit for a moment, then gone back into the code.
fn names(cv: &mut Canvas, f: &Frame, fade: f64) {
    let fleet = &f.info.fleet;
    let m = fleet.len().min(6);
    let u = f.u();
    if m == 0 || !(NAMES.0..NAMES.1 + 0.08).contains(&u) {
        return;
    }
    let rows = rows(f) as f64;
    let slot = (NAMES.1 - NAMES.0) / m as f64;
    let dur = (slot * 1.5).clamp(0.08, 0.17);
    for (i, mach) in fleet.iter().take(m).enumerate() {
        let a = NAMES.0 + slot * (i as f64 + 0.5) - dur / 2.0;
        let on = window(u, a, a + 0.012, a + dur - 0.03, a + dur) * fade;
        if on <= 0.0 {
            continue;
        }
        let name: String = mach.name.chars().take(18).collect();
        let n = name.chars().count() as i32;
        let i = i as u64;
        let frac = ((i as f64 + 0.5) / m as f64 + 0.18 * (unit(f.seed, 0x4a0 + i) - 0.5))
            .clamp(0.05, 0.95);
        let y = (1.0 + frac * (rows - 3.0)).round() as i32;
        let x = (2.0 + unit(f.seed, 0x4b0 + i) * (f.w - n as f64 - 6.0)).round() as i32;
        // a letter decoded every few moments, out of churning glyphs
        let step = (dur * 0.35 / n.max(1) as f64).min(0.006);
        let done = (((u - a - 0.008) / step).floor().max(0.0) as i32).min(n);
        cv.clear(x - 1, y);
        cv.clear(x + n, y);
        let prefix: String = name.chars().take(done as usize).collect();
        cv.text(x, y, &prefix, mix(RAIN[3], HEAD, 0.6), on);
        if done > 0 {
            // the newest letter flashes white
            let last: String = name.chars().nth(done as usize - 1).into_iter().collect();
            if u - a - 0.008 - (done - 1) as f64 * step < step * 2.0 {
                cv.put_with(x + done - 1, y, &last, HEAD, on, true);
            }
        }
        for j in done..n {
            let g = glyph(hash(
                f.seed ^ 0x6e,
                key(x + j, y) ^ ((u * 900.0) as u64) << 40,
            ));
            let mut b = [0u8; 4];
            cv.put_with(x + j, y, g.encode_utf8(&mut b), RAIN[3], on * 0.9, true);
        }
    }
}

/// The columns locking into the code: a bright head running down each
/// column of it (the middle first), the cells it passes lit and held.
fn lock(cv: &mut Canvas, f: &Frame, code: &Code, fade: f64) {
    let u = f.u();
    if u < LOCK.0 || code.cells.is_empty() {
        return;
    }
    // strokes a glyph thin (a small screen's) read better without it
    let glow = f.opts.logo || scale(f).0 > 1;
    for x in code.x0 - 1..=code.x1 + 1 {
        let hy = code.head(f, x.clamp(code.x0, code.x1));
        if hy < code.y0 as f64 - 1.0 {
            continue;
        }
        let mut any = false;
        for y in code.y0 - 1..=code.y1 + 1 {
            if y as f64 > hy {
                continue;
            }
            let mut b = [0u8; 4];
            if !code.has(x, y) {
                // a faint glow of code round the strokes
                let near = code.has(x - 1, y)
                    || code.has(x + 1, y)
                    || code.has(x, y - 1)
                    || code.has(x, y + 1);
                if near && glow && unit(f.seed ^ 0x6a11, key(x, y)) < 0.35 {
                    let g = glyph(hash(f.seed ^ 0x6a10, key(x, y)));
                    cv.put(x, y, g.encode_utf8(&mut b), RAIN[1], 0.32 * fade);
                }
                continue;
            }
            any = true;
            let fresh = (1.0 - (hy - y as f64) / 4.0).max(0.0);
            let (g, col) = lit(f, code, x, y, u, fresh);
            cv.put_with(x, y, g.encode_utf8(&mut b), col, fade, true);
        }
        // the head itself, while it runs through
        let hi = hy.floor() as i32;
        if any && (code.y0..=code.y1 + 1).contains(&hi) {
            let g = glyph(hash(f.seed ^ 0x4e, key(x, hi) ^ ((u * 700.0) as u64) << 40));
            let mut b = [0u8; 4];
            cv.put_with(x, hi, g.encode_utf8(&mut b), HEAD, fade, true);
        }
    }
}

/// One cell of the lit code at `u` (`fresh`: the lock just passed it):
/// its glyph, now and then turning over or flaring, and its colour,
/// brighter toward the top of the strokes.
fn lit(f: &Frame, code: &Code, x: i32, y: i32, u: f64, fresh: f64) -> (char, Rgb) {
    let own = unit(f.seed ^ 0xa1e, key(x, y));
    let ep = (u * (4.0 + 10.0 * own) + own * 5.0).floor() as u64;
    let g = glyph(hash(f.seed ^ 0xa1f, key(x, y) ^ ep << 48));
    if unit(f.seed ^ 0xa20, key(x, y) ^ ep << 48) < 0.07 {
        return (g, HEAD);
    }
    let tall = (code.y1 - code.y0).max(1) as f64;
    let base = ramp(&RAIN, 0.86 - 0.24 * (y - code.y0) as f64 / tall);
    (g, mix(base, HEAD, fresh))
}

/// Where its particles are as the ℵ gathers: the code it locked into.
pub(super) fn sources(f: &Frame) -> Vec<(f64, f64)> {
    Code::of(f)
        .cells
        .into_iter()
        .map(|(x, y)| (x as f64, y as f64))
        .collect()
}

/// With the logo, after the scene: while the ℵ gathers, the code it locked
/// into stays lit wherever the gold hasn't landed yet (so it turns to gold
/// in place, the sparks swirling round it); held, the rain runs on through
/// it.
pub(super) fn paint(cv: &mut Canvas, f: &Frame) {
    let done = f.len + GATHER_S;
    if f.t < f.len || f.t >= done + f.opts.hold {
        return;
    }
    if f.t < done {
        transmute(cv, f);
    } else {
        through(cv, f);
    }
}

/// Is the ℵ's gold there at `(x, y)` (a block or its braille edge)?
fn gold(cv: &Canvas, x: i32, y: i32) -> bool {
    !cv.blank(x, y)
        && cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
            .symbol()
            .chars()
            .next()
            .is_some_and(|c| c == '█' || ('\u{2801}'..='\u{28FF}').contains(&c))
}

/// The code ℵ, lit where the gold hasn't landed yet.
fn transmute(cv: &mut Canvas, f: &Frame) {
    let code = Code::of(f);
    // its clock runs on past the scene, so the code keeps turning over
    let u = f.t / f.len;
    let k = 1.0 - 0.35 * smooth(f.t, f.len, f.len + GATHER_S);
    for &(x, y) in &code.cells {
        if gold(cv, x, y) {
            continue;
        }
        let (g, col) = lit(f, &code, x, y, u, 0.0);
        let mut b = [0u8; 4];
        cv.put_with(x, y, g.encode_utf8(&mut b), col, k, true);
    }
}

/// The ℵ held, the rain running on through it: each column of the mark
/// lit green-white where a drop's head passes, fading behind it, the gold
/// showing between the drops.
fn through(cv: &mut Canvas, f: &Frame) {
    let done = f.len + GATHER_S;
    let end = done + f.opts.hold;
    let on = smooth(f.t, done, done + 120.0) * (1.0 - smooth(f.t, end - 120.0, end));
    let (x0, y0, lc, lr, cells) = logo_box(f.w, f.h, f.cx, f.cy);
    // the hold's own clock, in shares of the scene (it runs on)
    let tau = (f.t - done) / f.len;
    for (x, y, _) in cells {
        if !gold(cv, x, y) {
            continue;
        }
        let h = hash(f.seed ^ 0xd0, x as u64);
        let len = lr as f64 * (0.25 + 0.3 * unit(h, 1));
        let period = lr as f64 + len + lr as f64 * (0.5 + 1.5 * unit(h, 2));
        let v = lr as f64 * (9.0 + 6.0 * unit(h, 3));
        let s = (v * tau + unit(h, 4) * period).rem_euclid(period);
        let d = (y0 as f64 - 1.0 + s).floor() - y as f64;
        if d < 0.0 || d >= len {
            continue;
        }
        let q = 1.0 - d / len;
        let sym = cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
            .symbol()
            .to_string();
        let fpos = ((x - x0) as f64 / lc as f64 + (y - y0) as f64 / lr as f64) / 2.0;
        let code = if d < 1.0 {
            HEAD
        } else {
            ramp(&RAIN, 0.4 + 0.5 * q)
        };
        cv.put_with(
            x,
            y,
            &sym,
            mix(ramp(&GOLD, fpos), code, on * (0.2 + 0.75 * q)),
            1.0,
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::scene;
    use super::*;

    fn info(names: &[&str]) -> Info {
        Info {
            machines: names.len(),
            sessions: 2,
            needs: 0,
            fleet: names
                .iter()
                .map(|n| Mach {
                    name: n.to_string(),
                    rgb: (120, 200, 255),
                    up: true,
                    sessions: vec![],
                })
                .collect(),
        }
    }

    /// A moment of the scene (ms) on a dark 120×32 screen, and its frame.
    fn shot(ms: u64, logo: bool, info: &Info) -> (Buffer, Frame<'_>) {
        let f = scene::at(120, 32, ms, logo, info);
        (scene::dark(Kind::Matrix, &f), f)
    }

    fn light(b: &Buffer, x: i32, y: i32) -> u32 {
        match b[(x as u16, y as u16)].fg {
            Color::Rgb(r, g, bl) => r as u32 + g as u32 + bl as u32,
            _ => 0,
        }
    }

    fn row(b: &Buffer, y: i32) -> String {
        (0..b.area.width)
            .map(|x| b[(x, y as u16)].symbol())
            .collect()
    }

    #[test]
    fn its_glyphs_are_katakana_digits_and_letters_one_column_each() {
        let (mut kana, mut digits) = (0, 0);
        for i in 0..2000u64 {
            let g = glyph(hash(7, i));
            assert_eq!(ui::width(&g.to_string()), 1, "{g:?}");
            kana += ('\u{FF66}'..='\u{FF9D}').contains(&g) as usize;
            digits += g.is_ascii_digit() as usize;
        }
        assert!(kana > 1100 && digits > 300, "{kana} kana, {digits} digits");
    }

    #[test]
    fn a_columns_head_is_brighter_than_its_tail() {
        let info = info(&[]);
        let (b, f) = shot(600, false, &info);
        let rows = rows(&f);
        let tau = clock(f.u());
        let mut seen = 0;
        for x in (0..120).step_by(2) {
            let [Some(l), None] = lanes(&f, rows as f64, x) else {
                continue;
            };
            let fell = l.v * tau;
            let head = (l.phase + fell).rem_euclid(l.period).floor() as i32 - 1;
            let len = l.len.min(fell + 1.0).ceil() as i32;
            if len < 6 || head < len || head >= rows - 1 {
                continue;
            }
            let fg = b[(x as u16, head as u16)].fg;
            assert_eq!(fg, Color::Rgb(HEAD.0, HEAD.1, HEAD.2), "column {x}");
            let tail = head - len + 2;
            assert!(
                light(&b, x, head) > light(&b, x, tail) + 150,
                "column {x}: head {head}, tail {tail}"
            );
            // and nothing just below it
            assert_eq!(b[(x as u16, head as u16 + 1)].symbol(), " ", "column {x}");
            seen += 1;
        }
        assert!(seen >= 5, "{seen} columns");
    }

    #[test]
    fn a_machines_name_surfaces_as_a_horizontal_run() {
        let info = info(&["normandy", "serenity"]);
        for name in ["normandy", "serenity"] {
            let found = (0..1500).step_by(15).any(|ms| {
                let (b, f) = shot(ms, false, &info);
                (0..rows(&f)).any(|y| row(&b, y).contains(name))
            });
            assert!(found, "{name} never surfaced");
        }
        // and gone again by déjà vu
        let (b, f) = shot(1500, false, &info);
        assert!(!(0..rows(&f)).any(|y| row(&b, y).contains("normandy")));
    }

    #[test]
    fn deja_vu_repeats_a_band_of_rows() {
        let info = info(&[]);
        let ms = (scene::SCENE_MS as f64 * (BAND[0].0 + 0.01)) as u64;
        let (b, f) = shot(ms, false, &info);
        let (b0, n, s) = band(&f).expect("the band is on");
        let (mut same, mut lit) = (0, 0);
        for y in b0..b0 + n {
            for x in 2..118 {
                let sym = b[(x as u16, y as u16)].symbol();
                if sym != " " {
                    lit += 1;
                    same += (sym == b[((x - s) as u16, (y - n) as u16)].symbol()) as usize;
                }
            }
        }
        assert!(lit > 40 && same == lit, "{same} of {lit} repeat");
        // between its two times it's gone: the rain back in its own columns
        let (b, f) = shot((scene::SCENE_MS as f64 * 0.467) as u64, false, &info);
        assert!(band(&f).is_none());
        let odd = (b0..b0 + n)
            .flat_map(|y| (1..119).step_by(2).map(move |x| (x, y)))
            .filter(|&(x, y)| b[(x as u16, y as u16)].symbol() != " " && wave(&f, x, y) < 0.1)
            .count();
        assert_eq!(odd, 0);
    }

    #[test]
    fn frozen_the_columns_hang_still() {
        let info = info(&[]);
        let at = |u: f64| shot((scene::SCENE_MS as f64 * u) as u64, false, &info);
        let rain = |b: &Buffer, f: &Frame| (0..rows(f)).map(|y| row(b, y)).collect::<Vec<_>>();
        let ((a, fa), (b, fb)) = (at(0.3), at(0.32));
        assert_ne!(rain(&a, &fa), rain(&b, &fb), "falling");
        let ((a, fa), (b, fb)) = (at(FREEZE.1 + 0.002), at(LOCK.0 - 0.002));
        assert_eq!(rain(&a, &fa), rain(&b, &fb), "frozen");
        let lit = rain(&a, &fa).concat().chars().filter(|c| *c != ' ').count();
        assert!(lit > 300, "{lit}");
    }

    #[test]
    fn without_the_logo_it_decodes_into_aleph_and_the_rest_drains_away() {
        let info = info(&["normandy"]);
        let (b, f) = shot(scene::SCENE_MS - 1, false, &info);
        let (x0, y0, px, py) = word(&f);
        let bold = |x: i32, y: i32| {
            let c = &b[(x as u16, y as u16)];
            c.symbol() != " " && c.modifier.contains(Modifier::BOLD)
        };
        for (l, letter) in FONT.iter().enumerate() {
            for (r, line) in letter.iter().enumerate() {
                for (c, ch) in line.bytes().enumerate() {
                    let x = x0 + (l as i32 * 6 + c as i32) * px;
                    let y = y0 + r as i32 * py;
                    for (i, j) in (0..px).flat_map(|i| (0..py).map(move |j| (i, j))) {
                        assert_eq!(bold(x + i, y + j), ch == b'#', "letter {l} at {r},{c}");
                    }
                }
            }
        }
        // round the middle of the screen
        assert!((x0 + WORD * px / 2 - f.cx as i32).abs() <= 1);
        assert!((y0 + 7 * py / 2 - f.cy as i32).abs() <= 1);
        // and the rest of the rain gone
        for y in 0..rows(&f) {
            for x in 0..120 {
                let near =
                    (x0 - 1..=x0 + WORD * px).contains(&x) && (y0 - 1..=y0 + 7 * py).contains(&y);
                assert!(near || b[(x as u16, y as u16)].symbol() == " ", "({x},{y})");
            }
        }
    }

    #[test]
    fn with_the_logo_the_code_locks_into_the_mark_and_turns_to_gold() {
        let info = info(&["normandy"]);
        let green = |b: &Buffer, x: i32, y: i32| match b[(x as u16, y as u16)].fg {
            Color::Rgb(r, g, _) => g as i32 > r as i32 + 40,
            _ => false,
        };
        // the scene's last moment: the ℵ in lit code
        let (b, f) = shot(scene::SCENE_MS - 1, true, &info);
        let code = Code::of(&f);
        assert!(code.cells.len() > 60);
        let lit = code.cells.iter().filter(|&&(x, y)| green(&b, x, y)).count();
        assert!(
            lit * 10 >= code.cells.len() * 9,
            "{lit} of {}",
            code.cells.len()
        );
        // half gathered: gold where it has landed, the code lit where not
        let ms = (scene::SCENE_MS as f64 + scene::GATHER_S * 0.7) as u64;
        let (b, _) = shot(ms, true, &info);
        let solid = code
            .cells
            .iter()
            .filter(|&&(x, y)| b[(x as u16, y as u16)].symbol() == "█")
            .count();
        let still = code.cells.iter().filter(|&&(x, y)| green(&b, x, y)).count();
        assert!(solid > 20 && still > 20, "{solid} gold, {still} code");
        // held: gold, the rain running on through it
        let ms = (scene::SCENE_MS as f64 + scene::GATHER_S + 300.0) as u64;
        let (b, _) = shot(ms, true, &info);
        let (mut rain, mut gold) = (0, 0);
        for y in 0..32 {
            for x in 0..120 {
                if b[(x as u16, y as u16)].symbol() == "█" {
                    if green(&b, x, y) {
                        rain += 1;
                    } else {
                        gold += 1;
                    }
                }
            }
        }
        assert!(rain >= 3 && gold > rain * 3, "{rain} lit, {gold} gold");
    }

    #[test]
    fn any_fleet_none_one_or_many_draws_on_any_screen() {
        let long: Vec<String> = (0..12)
            .map(|i| format!("a-machine-with-a-name-longer-than-a-small-screen-{i}"))
            .collect();
        let long: Vec<&str> = long.iter().map(String::as_str).collect();
        for info in [info(&[]), info(&["normandy"]), info(&long)] {
            for (w, h) in [(50, 14), (200, 60)] {
                for u in [0.05, 0.2, 0.44, 0.65, 0.8, 1.0] {
                    let ms = (scene::SCENE_MS as f64 * u) as u64;
                    for logo in [false, true] {
                        let f = scene::at(w, h, ms, logo, &info);
                        let b = scene::dark(Kind::Matrix, &f);
                        let lit = (0..h)
                            .flat_map(|y| (0..w).map(move |x| (x, y)))
                            .filter(|&p| b[p].symbol() != " ")
                            .count();
                        assert!(lit > 10, "{w}×{h} at {u}: {lit}");
                    }
                }
            }
        }
    }
}
