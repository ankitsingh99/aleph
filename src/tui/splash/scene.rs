//! The cosmos splashes' common ground: each scene is a module of its own
//! (`singularity.rs`, …) drawing one moment of itself from a [`Frame`], in
//! normalised time (`Frame::u`, 0 → 1 over `ui.splash_ms`), so any length
//! works. Each ends its own way; with `ui.splash_logo = particles` its own
//! particles (`sources`) then gather into the ℵ (`gather`), which is held
//! for `ui.splash_hold` and bursts. The console line and the wordmark are
//! shared.
//!
//! Like the cockpit: a frame is a pure function of its moment, the
//! background is never painted, every glyph is one column, light terminals
//! and NO_COLOR work.

use super::*;

/// A scene's own length (`ui.splash_ms = auto`).
pub(super) const SCENE_MS: u64 = 3000;
/// The smallest screen a scene draws on (smaller: classic).
pub(super) const MIN: (u16, u16) = (50, 14);
/// The particles' flight into the ℵ, after the scene.
pub(super) const GATHER_S: f64 = 750.0;

type Rgb = (u8, u8, u8);

/// One moment of a scene.
pub(super) struct Frame<'a> {
    /// ms since it began
    pub t: f64,
    /// the scene's length (ms; the ℵ, its hold and burst come after)
    pub len: f64,
    pub w: f64,
    pub h: f64,
    pub cx: f64,
    pub cy: f64,
    pub seed: u64,
    pub info: &'a Info,
    pub opts: Opts,
}

impl Frame<'_> {
    /// How far through the scene, 0 → 1.
    pub fn u(&self) -> f64 {
        (self.t / self.len).clamp(0.0, 1.0)
    }

    /// The scene's own light while the ℵ gathers (1 without the logo): it
    /// gives its particles up and goes dark.
    pub fn fade(&self) -> f64 {
        if !self.opts.logo {
            return 1.0;
        }
        1.0 - smooth(self.t, self.len, self.len + GATHER_S * 0.55)
    }

    /// Between `a` and `b` (shares of the scene), 0 → 1 smoothly.
    pub fn act(&self, a: f64, b: f64) -> f64 {
        smooth(self.u(), a, b)
    }

    /// A cell's distance from the centre, in rows (cells are twice as tall
    /// as wide).
    pub fn dist(&self, x: f64, y: f64) -> f64 {
        ((x - self.cx) / 2.0).hypot(y - self.cy)
    }
}

/// The running integral of `smooth(x, a, b)` up to `t`: how far something
/// gets whose speed ramps from 0 to 1 over `a`–`b` (a spin-up's turns, a
/// clock that slows) — closed form, so a frame stays a pure function.
pub(super) fn ismooth(t: f64, a: f64, b: f64) -> f64 {
    if t <= a {
        return 0.0;
    }
    if t >= b {
        return (b - a) / 2.0 + (t - b);
    }
    let q = (t - a) / (b - a);
    (b - a) * (q.powi(3) - q.powi(4) / 2.0)
}

/// How long a scene of `len` plays with `opts` (ms).
pub(super) fn total(len: f64, opts: Opts) -> u64 {
    let logo = if opts.logo {
        GATHER_S + opts.hold + BURST
    } else {
        0.0
    };
    (len + logo) as u64
}

impl<'a> Frame<'a> {
    /// The moment `ms` into a scene on a `w`×`h` screen.
    pub fn new(
        w: f64,
        h: f64,
        ms: u64,
        seed: u64,
        info: &'a Info,
        opts: Opts,
        len: Option<u64>,
    ) -> Frame<'a> {
        Frame {
            t: ms as f64,
            len: len.unwrap_or(SCENE_MS) as f64,
            w,
            h,
            // a row above the middle: the console line and wordmark sit below
            cx: (w / 2.0).floor(),
            cy: ((h - 3.0) / 2.0).floor(),
            seed,
            info,
            opts,
        }
    }
}

/// One frame of scene `kind`.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    kind: Kind,
    buf: &mut Buffer,
    area: Rect,
    ms: u64,
    seed: u64,
    info: &Info,
    spin: &str,
    opts: Opts,
    len: Option<u64>,
) {
    let pal = c::pal();
    let mut cv = Canvas {
        buf,
        area,
        light: pal.light,
        mono: pal.mono,
        scan: None,
    };
    let (w, h) = (area.width as f64, area.height as f64);
    let f = Frame::new(w, h, ms, seed, info, opts, len);
    scene(&mut cv, kind, &f, spin);
}

/// One frame of scene `kind` on a canvas: the scene, the ℵ gathered from
/// it, the console line.
pub(super) fn scene(cv: &mut Canvas, kind: Kind, f: &Frame, spin: &str) {
    let opts = f.opts;
    let (title, cols, src): (&str, [Rgb; 2], Vec<(f64, f64)>) = match kind {
        Kind::Singularity => {
            super::singularity::draw(cv, f);
            (
                super::singularity::TITLE,
                super::singularity::COLS,
                super::singularity::sources(f),
            )
        }
        Kind::Pulsar => {
            super::pulsar::draw(cv, f);
            (
                super::pulsar::TITLE,
                super::pulsar::COLS,
                super::pulsar::sources(f),
            )
        }
        Kind::Horizon => {
            super::horizon::draw(cv, f);
            (
                super::horizon::TITLE,
                super::horizon::COLS,
                super::horizon::sources(f),
            )
        }
        Kind::Matrix => {
            super::matrix::draw(cv, f);
            (
                super::matrix::TITLE,
                super::matrix::COLS,
                super::matrix::sources(f),
            )
        }
        Kind::Convergence => {
            super::convergence::draw(cv, f);
            (
                super::convergence::TITLE,
                super::convergence::COLS,
                super::convergence::sources(f),
            )
        }
        Kind::Cyberdeck => {
            super::cyberdeck::draw(cv, f);
            (
                super::cyberdeck::TITLE,
                super::cyberdeck::COLS,
                super::cyberdeck::sources(f),
            )
        }
        Kind::Neon => {
            super::neon::draw(cv, f);
            (
                super::neon::TITLE,
                super::neon::COLS,
                super::neon::sources(f),
            )
        }
        // not scenes: nothing of their own
        _ => ("", [(226, 180, 81), (243, 205, 115)], Vec::new()),
    };
    if opts.logo {
        gather(cv, f, &src, cols);
        // the pulsar's beam goes on sweeping over the held ℵ, a lighthouse's
        if kind == Kind::Pulsar {
            super::pulsar::paint(cv, f);
        }
        // the rain runs on through the held ℵ
        if kind == Kind::Matrix {
            super::matrix::paint(cv, f);
        }
        // the held ℵ, a constellation: signals running along its strokes
        if kind == Kind::Convergence {
            super::convergence::paint(cv, f);
        }
        // the ℵ coming into focus out of its red and cyan ghosts, in yellow
        if kind == Kind::Cyberdeck {
            super::cyberdeck::paint(cv, f);
        }
        // the ℵ lit over the city as its biggest sign, tube by tube
        if kind == Kind::Neon {
            super::neon::paint(cv, f);
        }
    }
    console(cv, f, title, spin);
}

/// One particle's way into the ℵ: swirled round the mark (contracting as it
/// turns, from where the scene left it), then in to its own cell by `land`,
/// from the scene's colour to gold.
#[allow(clippy::too_many_arguments)]
fn swirl(
    seed: u64,
    k: u64,
    from: (f64, f64),
    centre: (f64, f64),
    to: (f64, f64),
    start: f64,
    land: f64,
    t: f64,
    cols: [Rgb; 2],
) -> ((f64, f64), Rgb) {
    let mid = start + (land - start) * 0.62;
    let (fu, fv) = ((from.0 - centre.0) / 2.0, from.1 - centre.1);
    let (rf, af) = (fu.hypot(fv).max(1.0), fv.atan2(fu));
    let (tu, tv) = ((to.0 - centre.0) / 2.0, to.1 - centre.1);
    let (rt, at) = (tu.hypot(tv), tv.atan2(tu));
    let mut turn = (at - af).rem_euclid(TAU);
    if turn < PI * 0.6 {
        turn += TAU;
    }
    // a little of its own wobble, so the stream isn't one rigid spiral
    let wob = (unit(seed, 4100 + k) - 0.5) * 0.6;
    let vortex = |q: f64| {
        let r = rf + (rt + 1.5 - rf) * q;
        let a = af + (turn + wob) * q;
        (centre.0 + a.cos() * r * 2.0, centre.1 + a.sin() * r)
    };
    if t < mid {
        let q = smooth(t, start, mid);
        return (vortex(q), mix(cols[0], cols[1], q));
    }
    let q = smooth(t, mid, land);
    let v = vortex(1.0);
    (
        (v.0 + (to.0 - v.0) * q, v.1 + (to.1 - v.1) * q),
        mix(cols[1], (243, 205, 115), q),
    )
}

/// The ℵ gathered from a scene's own particles (`from`: where it left
/// them; none: from the centre), held with a glint, then bursting into
/// gold sparks as the dashboard comes.
pub(super) fn gather(cv: &mut Canvas, f: &Frame, from: &[(f64, f64)], cols: [Rgb; 2]) {
    let start = f.len;
    if f.t < start {
        return;
    }
    let done = start + GATHER_S;
    let hold_end = done + f.opts.hold;
    let burst = smooth(f.t, hold_end, hold_end + BURST);
    let flare = window(f.t, done, done + 50.0, done + 70.0, done + 140.0);
    let glint = ((f.t - done) / f.opts.hold.max(300.0)).clamp(0.0, 1.0) * 1.6 - 0.3;
    let (x0, y0, lc, lr, cells) = logo_box(f.w, f.h, f.cx, f.cy);
    let centre = (x0 as f64 + lc as f64 / 2.0, y0 as f64 + lr as f64 / 2.0);
    let gold = accent();
    for (k, (tx, ty, bits)) in cells.iter().enumerate() {
        let k = k as u64;
        let (txf, tyf) = (*tx as f64, *ty as f64);
        let fpos = ((tx - x0) as f64 / lc as f64 + (ty - y0) as f64 / lr as f64) / 2.0;
        if burst > 0.0 {
            let (du, dv) = (txf - centre.0, (tyf - centre.1) * 2.0);
            let n = du.hypot(dv).max(0.5);
            let (x, y) = (txf + du / n * burst * 28.0, tyf + dv / n * burst * 7.0);
            let g = if k.is_multiple_of(7) { "✦" } else { "∙" };
            cv.put(
                x.round() as i32,
                y.round() as i32,
                g,
                mix((255, 246, 214), gold, burst),
                1.0 - burst * 0.85,
            );
            continue;
        }
        let order = 0.75 * fpos + 0.25 * unit(f.seed, 2900 + k);
        let land = start + GATHER_S * (0.35 + 0.65 * order);
        if f.t >= land {
            let shine = (1.0 - ((fpos - glint).abs() / 0.12)).max(0.0);
            logo_cell(cv, *tx, *ty, *bits, fpos, shine, flare);
            continue;
        }
        let src = if from.is_empty() {
            (f.cx, f.cy)
        } else {
            from[(hash(f.seed, k) % from.len() as u64) as usize]
        };
        let at = |t: f64| swirl(f.seed, k, src, centre, (txf, tyf), start, land, t, cols);
        for (lag, g, kk) in [(36.0, "·", 0.3), (18.0, "∙", 0.55)] {
            if f.t - lag > start {
                let ((x, y), col) = at(f.t - lag);
                cv.put(x.round() as i32, y.round() as i32, g, col, kk);
            }
        }
        let ((x, y), col) = at(f.t);
        cv.put(x.round() as i32, y.round() as i32, "•", col, 0.95);
    }
}

/// The line under the scene: its name and the fleet as it comes (spinner,
/// machines, sessions), and the wordmark shimmering in (showcase: its chip).
pub(super) fn console(cv: &mut Canvas, f: &Frame, title: &str, spin: &str) {
    let (w, h) = (f.w as i32, f.h as i32);
    let info = f.info;
    let mut status = title.to_string();
    if f.u() > 0.45 {
        status += &match info.machines {
            0 => " · this machine".to_string(),
            1 => " · 1 machine".to_string(),
            n => format!(" · {n} machines"),
        };
        if info.sessions > 0 {
            status += &format!(
                " · {} session{}",
                info.sessions,
                if info.sessions == 1 { "" } else { "s" }
            );
        }
        if info.needs > 0 {
            status += &format!(
                " · {} need{} you",
                info.needs,
                if info.needs == 1 { "s" } else { "" }
            );
        }
    }
    let line = format!("{spin} {status}");
    let n = line.chars().count() as i32;
    cv.text(
        (w - n) / 2,
        h - 2,
        &line,
        rgb_of(c::MUTED(), (150, 150, 160)),
        0.9,
    );
    // the wordmark, shimmering in for the last third
    let a = f.act(0.66, 0.8);
    if a <= 0.0 {
        return;
    }
    let word = format!("{} aleph", crate::brand::MARK);
    let base = Style::new().fg(c::ACCENT()).add_modifier(Modifier::BOLD);
    let mut spans = ui::shimmer(&word, base, f.t as u128 * 2 + 300);
    if let Some((chip, _)) = crate::showcase::splash_extras(f.t as u128) {
        spans.push(Span::raw("  "));
        spans.push(chip);
    }
    let wn: i32 = spans.iter().map(|s| ui::width(&s.content) as i32).sum();
    let mut x = (w - wn) / 2;
    let y = h - 3;
    for s in spans {
        let st = match s.style.fg {
            Some(Color::Rgb(r, g, b)) if a < 1.0 && !cv.mono => {
                let (r2, g2, b2) = mix((16, 16, 22), (r, g, b), a);
                s.style.fg(Color::Rgb(r2, g2, b2))
            }
            _ => s.style,
        };
        for ch in s.content.chars() {
            if x >= 0 && x < w && y >= 0 {
                let pos = (cv.area.x + x as u16, cv.area.y + y as u16);
                if let Some(cell) = cv.buf.cell_mut(pos) {
                    // whatever background it had stays (a popup\'s surface, the ground)
                    let bg = cell.bg;
                    cell.reset();
                    cell.bg = bg;
                    cell.set_char(ch).set_style(st);
                }
            }
            x += 1;
        }
    }
}

/// Tests: one frame of `kind` on a dark canvas, whatever theme other tests
/// set meanwhile (so colours can be compared).
#[cfg(test)]
pub(super) fn dark(kind: Kind, f: &Frame) -> Buffer {
    let area = Rect::new(0, 0, f.w as u16, f.h as u16);
    let mut buf = Buffer::empty(area);
    let mut cv = Canvas {
        buf: &mut buf,
        area,
        light: false,
        mono: false,
        scan: None,
    };
    scene(&mut cv, kind, f, "ℵ");
    buf
}

/// Tests: the moment `ms` into a scene of the default length on a `w`×`h`
/// screen, with the logo or without.
#[cfg(test)]
pub(super) fn at(w: u16, h: u16, ms: u64, logo: bool, info: &Info) -> Frame<'_> {
    let opts = Opts {
        end: End::Fall,
        fx: Fx::parse("all"),
        hold: HOLD,
        logo,
    };
    Frame::new(w as f64, h as f64, ms, 42, info, opts, None)
}
