//! A theme's living details in the dashboard (`crate::look`): the header's
//! ornament (cyberpunk's data readout), a glitch as the view changes, and
//! with `ui.ambience = rich` its ambience in empty space (Tokyo Night's city
//! rain). Each frame is a pure function of the time; `ui.ambience = off`,
//! `ui.motion = off` and NO_COLOR keep it all still (`reduced`: the ornament
//! alone, still).

use super::motion::{self, Motion};
use super::Dash;
use crate::look::{Ambience, Drift, Level, Ornament, Particles};
use crate::ui::{self, c};
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the glitch between views lasts.
pub(crate) const GLITCH: Duration = Duration::from_millis(180);

/// When the view last changed (the glitch's start).
static SWITCHED: Mutex<Option<Instant>> = Mutex::new(None);

/// The view just changed.
pub(crate) fn switched() {
    if let Ok(mut g) = SWITCHED.lock() {
        *g = Some(Instant::now());
    }
}

fn hash(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^ (x >> 29)
}

fn rgb(c: Color, or: (u8, u8, u8)) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => or,
    }
}

/// `a` toward `b` by `k` (0 → 1).
fn mix(a: Color, b: Color, k: f64) -> Color {
    let (a, b) = (rgb(a, (128, 128, 128)), rgb(b, (0, 0, 0)));
    let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * k.clamp(0.0, 1.0)).round() as u8;
    Color::Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// The header's ornament after the wordmark, `ms` since the dashboard
/// opened (pure — tested): cyberpunk's `▮ 4FA2·19C0`, its cursor blinking and
/// its readout ticking over (still with `ui.motion = reduced`).
pub(crate) fn ornament(ms: u128, motion: Motion) -> Vec<Span<'static>> {
    ornament_of(&crate::look::get(), ms, motion)
}

/// `ornament` for a given look (pure — tested).
fn ornament_of(lk: &crate::look::Look, ms: u128, motion: Motion) -> Vec<Span<'static>> {
    if lk.level == Level::Off || motion == Motion::Off {
        return Vec::new();
    }
    match lk.ornament {
        Ornament::None => Vec::new(),
        Ornament::Hex => {
            let still = motion == Motion::Reduced;
            let tick = if still { 0 } else { (ms / 420) as u64 };
            let h = hash(0xa1e9, tick);
            let cursor = if still || (ms / 530).is_multiple_of(2) {
                "▮"
            } else {
                " "
            };
            vec![
                Span::styled(cursor.to_string(), ui::fg(c::EDGE())),
                Span::styled(
                    format!(" {:04X}·{:04X} ", h & 0xffff, (h >> 16) & 0xffff),
                    ui::fg(c::EDGE_DIM()),
                ),
                Span::raw("  "),
            ]
        }
        // a radio telescope's trace scrolling left, a peak every `period`
        Ornament::Trace(period) => {
            let still = motion == Motion::Reduced;
            let step = 70u128;
            let t = if still { 0 } else { ms / step };
            let gap = (period / step).max(4);
            let trace: String = (0..12u128)
                .map(|i| match (t + i) % gap {
                    0 => "▇",
                    1 => "▃",
                    n if n == gap - 1 => "▂",
                    _ => "▁",
                })
                .collect();
            vec![
                Span::styled(trace, ui::fg(c::EDGE())),
                Span::styled(
                    format!(" P {:.3} s  ", period as f64 / 1000.0),
                    ui::fg(c::EDGE_DIM()),
                ),
            ]
        }
        Ornament::Cycle(frames, each, ink) => {
            let still = motion == Motion::Reduced;
            let i = if still {
                0
            } else {
                (ms / each.max(1)) as usize
            };
            vec![
                Span::styled(frames[i % frames.len()].to_string(), ui::fg(ink.color())),
                Span::raw("  "),
            ]
        }
    }
}

/// After a view change, its glitch over the frame just drawn (pure in
/// `since` and `seed` — tested): bands of rows torn sideways, tinted the
/// theme's red one way and its cyan the other, a little static.
pub(crate) fn glitch(buf: &mut Buffer, area: Rect, since: Duration, seed: u64) {
    if since >= GLITCH || area.width < 4 {
        return;
    }
    let left = 1.0 - since.as_secs_f64() / GLITCH.as_secs_f64();
    let bands = 2 + (hash(seed, 1) % 3) as u16;
    for b in 0..bands {
        let h = hash(seed, 10 + b as u64);
        let y0 = area.y + (h % area.height.max(1) as u64) as u16;
        let rows = 1 + (h >> 8) % 3;
        let dx = ((h >> 16) % 5) as i32 - 2;
        let tint = if dx < 0 { c::EDGE() } else { c::WORKING() };
        for y in y0..(y0 + rows as u16).min(area.bottom()) {
            let row: Vec<_> = (area.left()..area.right())
                .map(|x| buf.cell((x, y)).cloned().unwrap_or_default())
                .collect();
            for (i, x) in (area.left()..area.right()).enumerate() {
                let from = (i as i32 - dx).rem_euclid(row.len() as i32) as usize;
                if let Some(cl) = buf.cell_mut((x, y)) {
                    *cl = row[from].clone();
                    if cl.symbol() != " " {
                        cl.fg = mix(cl.fg, tint, 0.55 * left);
                    } else if hash(seed ^ y as u64, x as u64).is_multiple_of(23) {
                        cl.set_symbol("░");
                        cl.fg = mix(c::FAINT(), tint, 0.5);
                    }
                }
            }
        }
    }
}

/// A theme's ambience in the empty cells of `area`, `ms` in (pure —
/// tested): its particles falling (Tokyo Night's city rain, the matrix's code,
/// Nord's snow, dust falling into the horizon), rising (Gruvbox's embers,
/// Dracula's mist, Catppuccin's bubbles) or twinkling in place (stars), each
/// column at its own pace, faint against the ground. Only open space is
/// touched: never text, never the gap between two words, never a cell with
/// a background of its own, never an agent's screen (`keep_out`: its blank
/// cells are the agent's).
pub(crate) fn particles(buf: &mut Buffer, area: Rect, ms: u128, p: &Particles, keep_out: &[Rect]) {
    let h = area.height as u64;
    if h < 3 || p.glyphs.is_empty() {
        return;
    }
    let base = c::BASE();
    let open = |buf: &Buffer, x: u16, y: u16| {
        if keep_out.iter().any(|r| r.contains(Position::new(x, y))) {
            return false;
        }
        // open space: the terminal's background, or the theme's ground
        let free = |cl: Option<&ratatui::buffer::Cell>| {
            cl.is_none_or(|cl| cl.symbol() == " " && (cl.bg == Color::Reset || cl.bg == base))
        };
        free(buf.cell((x, y)))
            && free(x.checked_sub(1).and_then(|l| buf.cell((l, y))))
            && free(buf.cell((x + 1, y)))
    };
    let glyph = |k: u64, i: u64, tick: u64| -> &'static str {
        match (i, p.glyphs.len()) {
            (0, _) | (_, 1) => p.glyphs[0],
            (_, n) => p.glyphs[1 + (hash(k ^ tick, i) % (n as u64 - 1)) as usize],
        }
    };
    for x in area.left()..area.right() {
        let k = hash(0x5a1d, x as u64);
        if !k.is_multiple_of(p.every) {
            continue;
        }
        let rare = (k >> 40).is_multiple_of(7);
        let col = p.inks[usize::from(rare)].color();
        let spread = |lo: u64, hi: u64, at: u32| lo + (k >> at) % (hi - lo + 1);
        let len = spread(p.len.0, p.len.1, 16);
        let tick = (ms / 130) as u64;
        if p.drift == Drift::Still {
            // a star: its own place in the column, breathing — in four steps
            // that fade into the ground and back, never blinking on or off
            let y = area.y + ((k >> 8) % h) as u16;
            let phase = (ms as f64 / (900.0 + ((k >> 20) % 900) as f64)) + (k % 97) as f64;
            let glow = ((0.5 + 0.5 * phase.sin()) * 4.0).round() / 4.0;
            if glow > 0.0 && open(buf, x, y) {
                if let Some(cl) = buf.cell_mut((x, y)) {
                    cl.set_symbol(p.glyphs[((k >> 28) % p.glyphs.len() as u64) as usize]);
                    cl.fg = mix(col, base, 0.92 - 0.37 * glow);
                }
            }
            continue;
        }
        let speed = spread(p.speed.0.max(1), p.speed.1.max(1), 8);
        let span = h + len + 6 + (k >> 24) % 12;
        let head = ((ms as u64 * speed / 1000) + (k >> 32)) % span;
        for i in 0..len {
            let Some(y) = head.checked_sub(i) else { break };
            if y >= h {
                continue;
            }
            // rising: the same fall, upside down
            let y = if p.drift == Drift::Up { h - 1 - y } else { y };
            let yy = area.y + y as u16;
            if !open(buf, x, yy) {
                continue;
            }
            let Some(cl) = buf.cell_mut((x, yy)) else {
                continue;
            };
            let fade = 0.62 + 0.3 * i as f64 / len as f64;
            cl.set_symbol(glyph(k, i, tick));
            cl.fg = mix(col, base, fade);
        }
    }
}

/// The frame's living details, after it's drawn and before the theme's
/// ground (`ui::finish`): the glitch after a view change, the ambience in
/// empty space. Nothing over a popup.
pub(crate) fn frame(d: &Dash, buf: &mut Buffer, body: Rect) {
    let screens = ui::agent_screens();
    let lk = crate::look::get();
    let m = motion::of(d);
    if lk.level == Level::Off || m != Motion::On {
        return;
    }
    let quiet = !matches!(d.mode, super::Mode::Normal);
    if let (Level::Rich, false, Ambience::Particles(p)) = (lk.level, quiet, lk.ambience) {
        particles(buf, body, d.t0.elapsed().as_millis(), &p, &screens);
    }
    if lk.glitch {
        let at = SWITCHED.lock().ok().and_then(|g| *g);
        if let Some(at) = at {
            let seed = at.duration_since(d.t0).as_millis() as u64;
            let area = buf.area;
            glitch(buf, area, at.elapsed(), seed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &Buffer) -> String {
        let a = b.area;
        (a.top()..a.bottom())
            .map(|y| {
                (a.left()..a.right())
                    .map(|x| b[(x, y)].symbol().to_string())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    #[test]
    fn the_glitch_tears_a_few_rows_then_is_gone() {
        let area = Rect::new(0, 0, 40, 12);
        let mut b = Buffer::empty(area);
        for y in 0..12 {
            b.set_string(
                0,
                y,
                format!("row {y:02} the quick brown fox jumps"),
                Style::new(),
            );
        }
        let clean = text(&b);
        let mut g = b.clone();
        glitch(&mut g, area, Duration::from_millis(40), 7);
        let torn = text(&g);
        let moved = clean
            .lines()
            .zip(torn.lines())
            .filter(|(a, b)| a != b)
            .count();
        assert!((1..=9).contains(&moved), "{moved} rows:\n{torn}");
        // the same moment, the same glitch; over by its end
        let mut g2 = b.clone();
        glitch(&mut g2, area, Duration::from_millis(40), 7);
        assert_eq!(text(&g2), torn);
        let mut g3 = b.clone();
        glitch(&mut g3, area, GLITCH, 7);
        assert_eq!(text(&g3), clean);
    }

    #[test]
    fn rain_falls_only_in_empty_cells_and_moves() {
        let area = Rect::new(0, 0, 60, 20);
        let mut b = Buffer::empty(area);
        b.set_string(5, 5, "keep this text whole", Style::new());
        let Ambience::Particles(p) = crate::look::of("tokyo-night").ambience else {
            panic!("tokyo night rains");
        };
        assert_eq!(p.drift, crate::look::Drift::Down);
        let mut r1 = b.clone();
        particles(&mut r1, area, 1000, &p, &[]);
        let t1 = text(&r1);
        assert!(t1.contains("keep this text whole"), "{t1}");
        assert!(p.glyphs.iter().any(|g| t1.contains(g)), "{t1}");
        let mut r2 = b.clone();
        particles(&mut r2, area, 1400, &p, &[]);
        assert_ne!(text(&r2), t1, "it falls");
        // a cell with its own background (a pill, the selection) stays as it is
        let mut pill = Buffer::empty(area);
        for y in 0..20 {
            for x in 0..60 {
                pill[(x, y)].set_bg(Color::Rgb(1, 2, 3));
            }
        }
        let before = text(&pill);
        particles(&mut pill, area, 1000, &p, &[]);
        assert_eq!(text(&pill), before);
    }

    #[test]
    fn every_themes_ambience_stays_in_open_space_and_moves() {
        let area = Rect::new(0, 0, 60, 20);
        for lk in crate::look::ALL {
            let Ambience::Particles(p) = lk.ambience else {
                continue;
            };
            let mut b = Buffer::empty(area);
            b.set_string(5, 5, "keep this text whole", Style::new());
            let mut r1 = b.clone();
            particles(&mut r1, area, 1000, &p, &[]);
            let t1 = text(&r1);
            assert!(t1.contains("keep this text whole"), "{t1}");
            let drawn = t1.chars().filter(|c| !c.is_whitespace()).count();
            assert!(
                drawn > "keepthistextwhole".len(),
                "{p:?} drew nothing:\n{t1}"
            );
            let mut r2 = b.clone();
            particles(&mut r2, area, 2600, &p, &[]);
            assert_ne!(text(&r2), t1, "{p:?} stays still");
        }
    }

    #[test]
    fn no_theme_draws_over_an_agents_screen() {
        let area = Rect::new(0, 0, 60, 20);
        let screen = Rect::new(10, 3, 40, 12);
        let mut themes = 0;
        for lk in crate::look::ALL {
            let Ambience::Particles(p) = lk.ambience else {
                continue;
            };
            themes += 1;
            for ms in [0, 700, 1400, 2600, 5000, 9000] {
                let mut b = Buffer::empty(area);
                particles(&mut b, area, ms, &p, &[screen]);
                for y in screen.top()..screen.bottom() {
                    for x in screen.left()..screen.right() {
                        assert_eq!(b[(x, y)].symbol(), " ", "{p:?} at {ms} ms, {x},{y}");
                    }
                }
            }
        }
        assert!(themes >= 8, "{themes}");
    }

    #[test]
    fn the_ornament_ticks_and_is_one_column_a_glyph() {
        let lk = crate::look::of("cyberpunk");
        let t = |ms, m| -> String {
            ornament_of(&lk, ms, m)
                .iter()
                .map(|s| s.content.to_string())
                .collect()
        };
        let (a, b) = (t(0, Motion::On), t(900, Motion::On));
        assert!(!a.is_empty() && a != b, "{a} {b}");
        assert_eq!(ui::width(&a), a.chars().count());
        assert_eq!(t(900, Motion::Reduced), t(0, Motion::Reduced));
        assert!(t(0, Motion::Off).is_empty());
        assert!(ornament_of(&crate::look::of("dark"), 0, Motion::On).is_empty());
        // every theme's ornament moves, and keeps its width while it does
        for lk in crate::look::ALL {
            let w = |ms| -> (String, usize) {
                let s: String = ornament_of(lk, ms, Motion::On)
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect();
                let n = ui::width(&s);
                (s, n)
            };
            let (a, wa) = w(0);
            if a.is_empty() {
                continue;
            }
            let moved = (1..40).map(|i| w(i * 150)).any(|(b, wb)| {
                assert_eq!(wa, wb, "{lk:?}");
                b != a
            });
            assert!(moved, "{lk:?}'s ornament never moves");
        }
    }
}
