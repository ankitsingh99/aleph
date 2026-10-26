//! Celebrations (`celebrate`, a part of `showcase.flair`): about a second
//! each, drawn into the frame and gone, never waiting on anything:
//!
//! * a session finished — a supernova: a ring of light expanding from its
//!   Fleet row (or its tile), white-hot near the centre and cooling outward
//!   through gold and orange to embers, debris glittering behind it, the
//!   row itself glowing done-green for a moment;
//! * a question answered — a pulsar's flash along its row: twice, as both
//!   its beams sweep past.
//!
//! The light goes into open sky (blank cells, never the one space between
//! two words); text it passes over only changes colour (so it's always
//! readable), and the terminal's background is never painted. Where each session is comes from the frame just drawn (its Fleet
//! row, its tile): one that isn't on screen isn't celebrated.

use super::flair::{self, Part};
use super::fx::{self, Rgb};
use crate::config::Config;
use crate::fleet::Transition;
use crate::ui::c;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::cell::RefCell;
use std::time::{Duration, Instant};

const NOVA_MS: f64 = 1000.0;
const PULSAR_MS: f64 = 800.0;

/// White-hot to embers: a supernova's light as it travels out.
const HOT: [Rgb; 5] = [
    (244, 248, 255),
    (255, 238, 196),
    (226, 180, 81),
    (236, 116, 52),
    (146, 48, 38),
];
/// A pulsar's beam.
const BEAM: Rgb = (205, 225, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    /// finished
    Nova,
    /// a question answered
    Pulsar,
}

impl Fx {
    fn ms(self) -> f64 {
        match self {
            Fx::Nova => NOVA_MS,
            Fx::Pulsar => PULSAR_MS,
        }
    }
}

struct Burst {
    key: String,
    fx: Fx,
    at: Instant,
}

thread_local! {
    static BURSTS: RefCell<Vec<Burst>> = const { RefCell::new(Vec::new()) };
}

/// What a session's change is worth (pure — tested): finished (done, or
/// working → idle) is a supernova; a question that stopped waiting a pulsar.
pub fn of(from: &str, to: &str) -> Option<Fx> {
    match (from, to) {
        ("", _) => None,
        (f, "done") if f != "done" => Some(Fx::Nova),
        ("working", "idle") => Some(Fx::Nova),
        ("blocked", t) if t != "blocked" => Some(Fx::Pulsar),
        _ => None,
    }
}

/// The fleet's transitions this tick (a machine's first poll isn't news).
pub fn transitions(cfg: &Config, tr: &[Transition], fresh: &[String]) {
    if tr.is_empty() || !flair::wants(cfg, Part::Celebrate) {
        return;
    }
    let now = Instant::now();
    for t in tr {
        if fresh.contains(&t.session.host) {
            continue;
        }
        if let Some(fx) = of(&t.from, &t.session.status) {
            celebrate(t.session.key(), fx, now);
        }
    }
}

/// `key` (a session's `host|pane`) gets `fx` from `at`.
pub fn celebrate(key: String, fx: Fx, at: Instant) {
    BURSTS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|b| b.key != key);
        v.push(Burst { key, fx, at });
        while v.len() > 16 {
            v.remove(0);
        }
    });
}

/// Where each session is on a frame: (`host|pane`, its row or tile).
pub type Places = Vec<(String, Rect)>;

thread_local! {
    /// what plays this frame, staged while the dashboard draws and laid in
    /// once the lens has masked it: (the content area, where each session is)
    static STAGED: RefCell<Option<(Rect, Places)>> = const { RefCell::new(None) };
}

/// While the dashboard draws its content area `body` (`quiet`: a popup is
/// up, and nothing plays over it): stage what plays this frame. `places`
/// (each session's row or tile) is asked for only while a burst plays.
pub fn stage(cfg: &Config, body: Rect, quiet: bool, places: impl FnOnce() -> Places) {
    let now = Instant::now();
    let bursts = BURSTS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|b| now.duration_since(b.at) < Duration::from_millis(b.fx.ms() as u64));
        !v.is_empty()
    });
    let staged = (flair::wants(cfg, Part::Celebrate) && !quiet && bursts).then(|| (body, places()));
    STAGED.with(|s| *s.borrow_mut() = staged);
}

/// Once the lens has masked the frame (whole names laid into their rows):
/// what was staged, into the open sky the masked frame has — so a glyph
/// never takes the room a longer name needs.
pub fn finish(buf: &mut Buffer) {
    if let Some((body, places)) = STAGED.with(|s| s.borrow_mut().take()) {
        play(buf, body, || places);
    }
}

/// Tests: into the content area `body`, now, what's playing (if
/// `celebrate` is on), the way `stage` and `finish` do it between them.
#[cfg(test)]
pub fn draw(cfg: &Config, buf: &mut Buffer, body: Rect, places: impl FnOnce() -> Places) {
    if flair::wants(cfg, Part::Celebrate) {
        play(buf, body, places);
    }
}

/// What's playing, drawn into `body` (`places` asked for only while a burst
/// plays).
fn play(buf: &mut Buffer, body: Rect, places: impl FnOnce() -> Places) {
    let now = Instant::now();
    let clip = body.intersection(buf.area);
    let live: Vec<(String, Fx, f64)> = BURSTS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|b| now.duration_since(b.at) < Duration::from_millis(b.fx.ms() as u64));
        v.iter()
            .map(|b| {
                let ms = now.duration_since(b.at).as_secs_f64() * 1000.0;
                (b.key.clone(), b.fx, ms)
            })
            .collect()
    });
    if live.is_empty() {
        return;
    }
    let places = places();
    for (key, fx, ms) in live {
        let Some((_, r)) = places.iter().find(|(k, _)| *k == key) else {
            continue;
        };
        let seed = fx::hash_str(&key);
        match fx {
            Fx::Nova => nova(buf, clip, *r, ms, seed),
            Fx::Pulsar => pulsar(buf, clip, *r, ms),
        }
    }
}

// ------------------------------------------------------------------ supernova

/// A supernova over `r` (a Fleet row, or a tile), `ms` in (pure).
pub fn nova(buf: &mut Buffer, clip: Rect, r: Rect, ms: f64, seed: u64) {
    let k = (ms / NOVA_MS).clamp(0.0, 1.0);
    if k >= 1.0 || clip.width == 0 {
        return;
    }
    let row = r.height <= 2;
    // a row's centre is by the session's name; a tile's is its middle
    let (cx, cy) = if row {
        (
            r.x as f64 + (r.width as f64 / 6.0).clamp(4.0, 12.0),
            r.y as f64,
        )
    } else {
        (
            r.x as f64 + r.width as f64 / 2.0,
            r.y as f64 + r.height as f64 / 2.0,
        )
    };
    // distances in rows (a cell is about half as wide as it's tall); along a
    // row the ring stretches, the way a row is long
    let stretch = if row { 1.8 } else { 1.0 };
    let reach = if row {
        (r.width as f64 / (2.0 * stretch) * 0.42).clamp(4.0, 9.0)
    } else {
        (r.width as f64 / 4.0)
            .min(r.height as f64 / 2.0)
            .clamp(2.5, 12.0)
    };
    let radius = reach * (1.0 - (1.0 - k).powi(3));
    let fade = (1.0 - k).powf(1.3);
    let thick = 0.38 + 0.45 * k;
    let dist = |x: f64, y: f64| ((x - cx) / (2.0 * stretch)).hypot(y - cy);
    let ring = |x: f64, y: f64| (-((dist(x, y) - radius) / thick).powi(2)).exp() * fade;
    let rx = (reach + 1.5) * 2.0 * stretch;
    let x0 = (cx - rx).max(clip.left() as f64) as u16;
    let x1 = (cx + rx).min(clip.right() as f64 - 1.0).max(0.0) as u16;
    let y0 = (cy - reach - 1.5).max(clip.top() as f64) as u16;
    let y1 = (cy + reach + 1.5).min(clip.bottom() as f64 - 1.0).max(0.0) as u16;
    let done = fx::pal_rgb(c::DONE(), (70, 167, 88));
    let sky = fx::Sky::of(buf, clip);
    for y in y0..=y1 {
        for x in x0..=x1 {
            if !clip.contains((x, y).into()) {
                continue;
            }
            let (fx_, fy) = (x as f64, y as f64);
            let d = dist(fx_, fy);
            // cooling outward: white-hot near the centre, embers at the edge
            let rgb = fx::ramp(&HOT, d / reach * 0.9 + 0.1 * k);
            let here = ring(fx_, fy);
            if sky.open(x, y) {
                let h = fx::hash(seed, (x as u64) << 16 | y as u64);
                if here > 0.45 && h.is_multiple_of(9) {
                    let g = ["✦", "✧", "⋆", "∗"][(h >> 8) as usize % 4];
                    sky.put(buf, x, y, g, fx::glow(rgb, 0.4 + 0.6 * here));
                } else if let Some((ch, top)) = fx::braille_at(fx_, fy, ring) {
                    let mut s = [0u8; 4];
                    sky.put(
                        buf,
                        x,
                        y,
                        ch.encode_utf8(&mut s),
                        fx::glow(rgb, 0.3 + 0.7 * top),
                    );
                } else if d < radius - thick * 1.5 && h % 23 == 3 {
                    // debris the shell left behind
                    let g = if (h >> 12).is_multiple_of(3) {
                        "∙"
                    } else {
                        "·"
                    };
                    sky.put(buf, x, y, g, fx::glow(rgb, 0.55 * fade));
                }
            } else {
                // text it passes over lights up and keeps its letters (a
                // space between words stays a space)
                fx::tint(buf, x, y, rgb, 0.85 * here);
            }
        }
    }
    // the row itself: done-green for a moment, white where it began
    if row {
        let g = (1.0 - k / 0.4).max(0.0);
        for x in r.left()..r.right() {
            if clip.contains((x, r.y).into()) {
                fx::tint(buf, x, r.y, done, 0.7 * g);
            }
        }
    }
    if k < 0.2 {
        let (x, y) = (cx.round() as u16, cy.round() as u16);
        if clip.contains((x, y).into()) && !fx::put(buf, x, y, "✸", fx::glow(HOT[0], 1.0)) {
            fx::tint(buf, x, y, HOT[0], 0.9 * (1.0 - k / 0.2));
        }
    }
}

// ------------------------------------------------------------------ pulsar

/// Two flashes, as both beams sweep past (the second fainter): 0..1 at `ms`.
fn flashes(ms: f64) -> f64 {
    let f = |t0: f64, a: f64| a * (-((ms - t0) / 75.0).powi(2)).exp();
    (f(90.0, 1.0) + f(420.0, 0.75)).min(1.0)
}

/// A pulsar's flash along `r` (a Fleet row; a tile's border), `ms` in (pure).
pub fn pulsar(buf: &mut Buffer, clip: Rect, r: Rect, ms: f64) {
    if ms >= PULSAR_MS || clip.width == 0 {
        return;
    }
    let p = flashes(ms) * (1.0 - fx::smooth(ms, 600.0, PULSAR_MS));
    // the cells to light: a row, or a tile's frame
    let cells: Vec<(u16, u16)> = if r.height <= 2 {
        (r.left()..r.right()).map(|x| (x, r.y)).collect()
    } else {
        let mut v: Vec<(u16, u16)> = Vec::new();
        for x in r.left()..r.right() {
            v.push((x, r.top()));
            v.push((x, r.bottom() - 1));
        }
        for y in r.top() + 1..r.bottom().saturating_sub(1) {
            v.push((r.left(), y));
            v.push((r.right() - 1, y));
        }
        v
    };
    // each beam sweeps along the top as it passes
    let sweep = |t0: f64, a: f64, x: u16| {
        let front = r.x as f64 + (ms - t0 + 40.0) / 240.0 * r.width as f64;
        let back = front - 10.0;
        let xf = x as f64;
        if xf > front || xf < back {
            return 0.0;
        }
        a * ((xf - back) / 10.0).powf(1.5)
    };
    let sky = fx::Sky::of(buf, clip);
    for (x, y) in cells {
        if !clip.contains((x, y).into()) {
            continue;
        }
        let beam = if y == r.y {
            sweep(60.0, 1.0, x).max(sweep(390.0, 0.75, x))
        } else {
            0.0
        };
        let b = (0.6 * p).max(beam);
        if b <= 0.03 {
            continue;
        }
        if fx::blank(buf, x, y) {
            if beam > 0.08 {
                let g = if beam > 0.6 { "━" } else { "─" };
                sky.put(buf, x, y, g, fx::glow(BEAM, 0.3 + 0.7 * beam));
            }
        } else {
            fx::tint(buf, x, y, BEAM, b);
        }
    }
    // the pulsar itself, just left of the row (where the selection bar goes)
    if r.height <= 2 && r.x > clip.left() {
        let (x, y) = (r.x - 1, r.y);
        let g = if p > 0.5 { "✸" } else { "✦" };
        if !fx::put(buf, x, y, g, fx::glow(BEAM, 0.35 + 0.65 * p)) {
            fx::tint(buf, x, y, BEAM, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    /// A screen with one Fleet-like row of text in the middle.
    fn screen() -> (Buffer, Rect, Rect) {
        let body = Rect::new(0, 0, 100, 20);
        let mut b = Buffer::empty(body);
        let row = Rect::new(4, 10, 90, 1);
        b.set_string(6, 10, "● ignite   working   ~/work/aleph", Style::new());
        b.set_string(6, 8, "● temper   blocked", Style::new());
        (b, body, row)
    }

    fn text(b: &Buffer, y: u16) -> String {
        (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()
    }

    fn lit(b: &Buffer) -> usize {
        let blank = Buffer::empty(b.area);
        let (s, _, _) = screen();
        (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| b[(x, y)] != s[(x, y)] && b[(x, y)] != blank[(x, y)])
            .count()
    }

    #[test]
    fn what_each_change_is_worth() {
        assert_eq!(of("working", "done"), Some(Fx::Nova));
        assert_eq!(of("blocked", "done"), Some(Fx::Nova));
        assert_eq!(of("working", "idle"), Some(Fx::Nova));
        assert_eq!(of("blocked", "working"), Some(Fx::Pulsar));
        assert_eq!(of("blocked", "idle"), Some(Fx::Pulsar));
        assert_eq!(of("", "done"), None, "a new session isn't news");
        assert_eq!(of("idle", "working"), None);
        assert_eq!(of("blocked", "blocked"), None);
        assert_eq!(of("done", "done"), None);
    }

    #[test]
    fn a_supernova_lights_blanks_and_keeps_the_text() {
        let (mut b, body, row) = screen();
        nova(&mut b, body, row, 350.0, 7);
        // the row and its neighbour still read the same
        assert!(text(&b, 10).contains("● ignite   working   ~/work/aleph"));
        assert!(text(&b, 8).contains("● temper   blocked"));
        // light in the blanks above and below
        assert!(lit(&b) > 20, "{}", lit(&b));
        let rows_lit = (0..20)
            .filter(|&y| y != 10 && text(&b, y).trim() != "")
            .count();
        assert!(rows_lit >= 4, "{rows_lit}");
        // the row glows (its text changed colour, not its letters)
        assert_ne!(b[(8, 10)].fg, Color::Reset);
        // never a background, never a wide glyph
        for y in 0..20 {
            for x in 0..100 {
                assert_eq!(b[(x, y)].bg, Color::Reset);
                assert!(crate::ui::width(b[(x, y)].symbol()) <= 1);
            }
        }
        // done by its second
        let (mut b, body, row) = screen();
        nova(&mut b, body, row, NOVA_MS + 1.0, 7);
        assert_eq!(lit(&b), 0);
    }

    #[test]
    fn the_ring_grows_and_stays_inside() {
        let (mut early, body, row) = screen();
        nova(&mut early, body, row, 120.0, 7);
        let (mut late, _, _) = screen();
        nova(&mut late, body, row, 600.0, 7);
        let reach = |b: &Buffer| {
            (0..20)
                .filter(|&y| (0..100).any(|x| b[(x, y)].symbol() != " " && y != 10 && y != 8))
                .map(|y| (y as i32 - 10).abs())
                .max()
                .unwrap_or(0)
        };
        assert!(
            reach(&late) > reach(&early),
            "{} {}",
            reach(&late),
            reach(&early)
        );
        // a small clip: nothing outside it
        let (mut b, _, row) = screen();
        let clip = Rect::new(0, 9, 100, 3);
        nova(&mut b, clip, row, 500.0, 7);
        for y in (0..9).chain(12..20) {
            assert_eq!(text(&b, y), text(&screen().0, y), "row {y}");
        }
    }

    #[test]
    fn a_pulsar_flashes_twice_along_its_row() {
        assert!(flashes(90.0) > 0.9 && flashes(420.0) > 0.6);
        assert!(flashes(250.0) < 0.3 && flashes(700.0) < 0.05);
        let (mut b, body, row) = screen();
        pulsar(&mut b, body, row, 90.0);
        assert!(text(&b, 10).contains("ignite"));
        assert!(matches!(b[(8, 10)].fg, Color::Rgb(..)), "the row lights");
        // the star where the selection bar goes
        assert!(matches!(b[(3, 10)].symbol(), "✸" | "✦"));
        // its neighbours are left alone
        assert_eq!(text(&b, 8), text(&screen().0, 8));
        let (mut b, body, row) = screen();
        pulsar(&mut b, body, row, PULSAR_MS);
        assert_eq!(lit(&b), 0);
    }

    #[test]
    fn bursts_land_where_their_session_is() {
        let cfg = Config::in_memory(&[]);
        let (mut b, body, row) = screen();
        celebrate(
            "b31|w1:p1".into(),
            Fx::Nova,
            Instant::now() - Duration::from_millis(300),
        );
        let mut asked = false;
        draw(&cfg, &mut b, body, || {
            asked = true;
            vec![("b31|w1:p1".to_string(), row)]
        });
        assert!(asked && lit(&b) > 20);
        // off: nothing, and nobody asked where
        let off = Config::in_memory(&["showcase.flair=header"]);
        let (mut b, body, _) = screen();
        celebrate("b31|w1:p1".into(), Fx::Nova, Instant::now());
        draw(&off, &mut b, body, || panic!("asked"));
        assert_eq!(lit(&b), 0);
        // not on screen: not celebrated
        let (mut b, body, _) = screen();
        draw(&cfg, &mut b, body, Vec::new);
        assert_eq!(lit(&b), 0);
        BURSTS.with(|v| v.borrow_mut().clear());
    }

    #[test]
    fn a_tile_bursts_from_its_middle_and_flashes_its_frame() {
        let body = Rect::new(0, 0, 100, 30);
        let tile = Rect::new(50, 5, 40, 20);
        let mut b = Buffer::empty(body);
        nova(&mut b, body, tile, 400.0, 3);
        let drawn = |b: &Buffer, xs: std::ops::Range<u16>| {
            (0..30)
                .flat_map(|y| xs.clone().map(move |x| (x, y)))
                .filter(|&(x, y)| b[(x, y)].symbol() != " ")
                .count()
        };
        assert!(drawn(&b, 50..90) > 20, "{}", drawn(&b, 50..90));
        assert_eq!(drawn(&b, 0..30), 0, "far from the tile: dark");
        // its frame, twice over
        let mut b = Buffer::empty(body);
        ratatui::widgets::Widget::render(ratatui::widgets::Block::bordered(), tile, &mut b);
        pulsar(&mut b, body, tile, 90.0);
        for (x, y) in [(50, 5), (89, 24), (50, 15), (70, 24)] {
            assert!(matches!(b[(x, y)].fg, Color::Rgb(..)), "({x},{y}) lit");
            assert_ne!(b[(x, y)].symbol(), " ");
        }
        assert_eq!(b[(70, 15)].symbol(), " ", "its inside stays");
    }
}
