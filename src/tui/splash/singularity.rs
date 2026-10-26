//! `singularity`: an emerging universe. A void of quantum foam, a dim
//! violet haze boiling and breathing; a point appears and deepens — a
//! white-hot heart in a glow of gold and rose, diffraction spikes, beating
//! quicker, rings of the void drawn in to it with each beat — then draws
//! in its breath and erupts: a flash, and a fireball swelling past the
//! screen's edges, billowing and filamented, cooling from white through
//! gold and rose to violet behind its shock front, sparks thrown ahead. Its
//! afterglow drains into the cosmic web — threads condensing between knots
//! — and its brightest lobes become nebulae; galaxies form at the knots,
//! spirals winding up and turning (some edge-on), your machines the
//! biggest and brightest, named; the first stars light. It ends on that
//! deep field, the view pushing slowly in; with the logo, its galaxies and
//! stars stream into the ℵ.

use super::light::{lit, star, Light, Noise};
use super::scene::{ismooth, Frame};
use super::*;

type Rgb = (u8, u8, u8);

pub(super) const TITLE: &str = "a point becoming everything";
/// Its colours on the way into the ℵ: the young stars' violet, then gold.
pub(super) const COLS: [Rgb; 2] = [(176, 120, 255), (255, 214, 120)];

/// The fireball cooling: white-hot, pale gold, amber, rose, violet, indigo.
const COOL: [Rgb; 6] = [
    (255, 252, 240),
    (255, 222, 146),
    (255, 160, 92),
    (255, 96, 150),
    (160, 90, 255),
    (60, 44, 170),
];
/// The void's haze and its foam.
const VOID: Rgb = (96, 64, 200);
const FOAM: Rgb = (180, 160, 255);
/// The point's heart and its spikes.
const HOT: Rgb = (255, 248, 230);
const SPIKE: Rgb = (255, 232, 186);
/// The web: its threads (violet to rose), its knots.
const THREAD: [Rgb; 2] = [(130, 110, 255), (240, 110, 200)];
const KNOT: Rgb = (255, 200, 150);
/// Nebulae: teal at the rims, violet, magenta at the heart.
const NEB: [Rgb; 3] = [(40, 170, 220), (130, 60, 240), (240, 60, 150)];
/// A galaxy's heart, its arms, the young stars along them.
const CORE: Rgb = (255, 222, 160);
const ARMS: [Rgb; 3] = [(130, 165, 255), (175, 135, 255), (120, 205, 255)];
const YOUNG: Rgb = (255, 130, 200);

/// The acts, as shares of the scene: the point deepening; its breath in;
/// the bang; the fireball spreading; the web condensing; the galaxies
/// forming; your machines named; the view pushing in at the end, and how
/// far (a share of the way out).
const POINT: (f64, f64) = (0.03, 0.27);
const INHALE: (f64, f64) = (0.25, 0.32);
const BANG: f64 = 0.32;
const FIRE: f64 = 0.64;
const WEB: (f64, f64) = (0.46, 0.68);
const FORM: (f64, f64) = (0.58, 0.84);
const NAMES: (f64, f64) = (0.72, 0.8);
const DOLLY: (f64, f64) = (0.56, 1.0);
const PUSH: f64 = 0.08;

/// One moment of it.
struct Now {
    u: f64,
    fade: f64,
    /// the scene's rows (the wordmark and the console are below)
    rows: i32,
    /// the fireball: how far it has spread (0 → 1), its radius (rows)
    q: f64,
    r: f64,
    /// the screen's scale (1 at 150 × 40), for sizes
    s: f64,
    /// how far the view has pushed in
    push: f64,
    cx: f64,
    cy: f64,
}

impl Now {
    fn of(f: &Frame) -> Now {
        let u = f.u();
        let rows = (f.h as i32 - 3).max(1);
        // what it reaches at last: past the corners
        let reach = (f.w / 4.0 + 1.0).hypot(f.cy.max(rows as f64 - f.cy) + 1.0);
        let q = ((u - BANG) / (FIRE - BANG)).clamp(0.0, 1.0);
        Now {
            u,
            fade: f.fade(),
            rows,
            q,
            r: reach * (1.0 - (1.0 - q).powf(1.6)),
            s: (rows as f64 / 37.0).min(f.w / 150.0).clamp(0.4, 1.6),
            push: PUSH * f.act(DOLLY.0, DOLLY.1),
            cx: f.cx,
            cy: f.cy,
        }
    }

    /// Where something at `(x, y)` is as the view pushes in: the nearer
    /// (`depth`, 0 far → 1), the further out it goes.
    fn dolly(&self, x: f64, y: f64, depth: f64) -> (f64, f64) {
        let z = 1.0 + self.push * depth;
        (self.cx + (x - self.cx) * z, self.cy + (y - self.cy) * z)
    }
}

/// A knot of the web, and the galaxy that forms there.
struct Node {
    x: f64,
    y: f64,
    /// how much gathers there, 0–1
    w: f64,
    /// its galaxy: radius (rows), how round it's seen (1 face-on, ~0.2
    /// edge-on), its tilt, its arms and their pitch, how fast it turns, its
    /// arms' colour, when it forms
    size: f64,
    flat: f64,
    pa: f64,
    arms: u32,
    pitch: f64,
    spin: f64,
    arm: Rgb,
    born: f64,
    /// the machine it is, named
    name: Option<(String, Rgb)>,
}

/// The web's knots: spread over the whole screen (a jittered grid), small
/// galaxies forming at most; your machines the big ones, at the best placed
/// knots (apart, with room for their names). Where they are as the view
/// pushes in.
fn nodes(f: &Frame, n: &Now) -> Vec<Node> {
    let rows = n.rows as f64;
    let nx = ((f.w / 30.0).round() as usize).clamp(3, 8);
    let ny = ((rows / 12.0).round() as usize).clamp(2, 5);
    let mut v: Vec<Node> = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            let h = hash(f.seed ^ 0x6a1a, (j * nx + i) as u64);
            let x = (i as f64 + 0.5 + 0.8 * (unit(h, 1) - 0.5)) / nx as f64 * f.w;
            let y = (j as f64 + 0.5 + 0.75 * (unit(h, 2) - 0.5)) / ny as f64 * (rows - 1.0);
            let w = 0.3 + 0.7 * unit(h, 3);
            let edge_on = unit(h, 4) < 0.3;
            v.push(Node {
                x,
                y,
                w,
                size: (0.9 + 1.3 * unit(h, 5) * w) * n.s,
                flat: if edge_on {
                    0.17 + 0.08 * unit(h, 6)
                } else {
                    0.42 + 0.4 * unit(h, 6)
                },
                pa: (unit(h, 7) - 0.5) * 1.1,
                arms: if unit(h, 8) < 0.75 { 2 } else { 3 },
                pitch: 0.38 + 0.2 * unit(h, 9),
                spin: (0.5 + 0.4 * unit(h, 10)) * if unit(h, 11) < 0.5 { 1.0 } else { -1.0 },
                arm: ARMS[(h % 3) as usize],
                born: FORM.0 + (FORM.1 - FORM.0 - 0.16) * unit(h, 12),
                name: None,
            });
        }
    }
    // your machines: as far apart as can be, with room for their names
    let fleet = f.info.fleet.len().min(6);
    let fit = (1.25 - 0.08 * fleet as f64).clamp(0.75, 1.15);
    let mut taken: Vec<usize> = Vec::new();
    for (k, m) in f.info.fleet.iter().take(6).enumerate() {
        let name: String = m.name.chars().take(14).collect();
        let size = (6.0 + 1.5 * unit(f.seed, 90 + k as u64)) * n.s * fit;
        let len = name.chars().count() as f64;
        // how far it is from fitting across (pushed out a little, later)
        let slack = |g: &Node| {
            let spare = 1.0 + PUSH * (g.x - f.cx).abs();
            let left = g.x - size * 2.0 - spare;
            let right = f.w - spare - (g.x + size * 2.0 + 2.0 + len);
            left.min(right)
        };
        let apart = |i: usize| {
            taken
                .iter()
                .map(|&t| ((v[i].x - v[t].x) / 2.0).hypot(v[i].y - v[t].y))
                .fold(1e3, f64::min)
                + unit(f.seed, 77 + i as u64)
        };
        let free: Vec<usize> = (0..v.len()).filter(|i| !taken.contains(i)).collect();
        let fits: Vec<usize> = free
            .iter()
            .copied()
            .filter(|&i| slack(&v[i]) >= 0.0)
            .collect();
        let best = if fits.is_empty() {
            free.iter()
                .copied()
                .max_by(|&a, &b| slack(&v[a]).total_cmp(&slack(&v[b])))
        } else {
            fits.into_iter()
                .max_by(|&a, &b| apart(a).total_cmp(&apart(b)))
        };
        let Some(i) = best else { break };
        taken.push(i);
        let g = &mut v[i];
        // moved over, if it must, to fit
        let (lo, hi) = (size * 2.0 + 2.0, f.w - 5.0 - size * 2.0 - len);
        if lo <= hi {
            g.x = g.x.clamp(lo, hi);
        }
        // and down, to show whole once the view has pushed in
        let half = size * 0.6;
        let (lo, hi) = (
            f.cy - (f.cy - half - 0.5) / (1.0 + PUSH),
            f.cy + (rows - 1.5 - half - f.cy) / (1.0 + PUSH),
        );
        g.y = if lo <= hi { g.y.clamp(lo, hi) } else { f.cy };
        g.name = Some((name, m.rgb));
        g.w = 1.0;
        g.size = size;
        g.flat = 0.5 + 0.22 * unit(f.seed, 95 + k as u64);
        g.arm = mix(g.arm, m.rgb, 0.5);
        g.born = 0.6 + 0.08 * unit(f.seed, 99 + k as u64);
    }
    for g in &mut v {
        (g.x, g.y) = n.dolly(g.x, g.y, 1.0);
    }
    v
}

/// A thread of the web: its knots, how far it bows off the straight, and
/// how it wavers.
type Thread = (usize, usize, f64, f64);

/// The web's threads: each knot to its nearest two or three (none too
/// long).
fn threads(f: &Frame, nodes: &[Node]) -> Vec<Thread> {
    let mut e: Vec<Thread> = Vec::new();
    let d = |a: &Node, b: &Node| ((a.x - b.x) / 2.0).hypot(a.y - b.y);
    for (i, a) in nodes.iter().enumerate() {
        let mut near: Vec<(f64, usize)> = nodes
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(j, b)| (d(a, b), j))
            .collect();
        near.sort_by(|x, y| x.0.total_cmp(&y.0));
        let keep = 2 + (hash(f.seed ^ 0x7e, i as u64) % 2) as usize;
        let first = near.first().map_or(0.0, |x| x.0);
        for &(dist, j) in near.iter().take(keep) {
            if dist > first * 2.2 {
                continue;
            }
            let (a, b) = (i.min(j), i.max(j));
            if !e.iter().any(|x| x.0 == a && x.1 == b) {
                let h = hash(f.seed ^ 0xb0, (a * 97 + b) as u64);
                e.push((a, b, (unit(h, 1) - 0.5) * 0.4, unit(h, 2) * TAU));
            }
        }
    }
    e
}

/// A point `s` (0 → 1) along a thread, in cells: bowed off the straight
/// and wavering.
fn along(nodes: &[Node], t: &Thread, s: f64) -> (f64, f64) {
    let (a, b) = (&nodes[t.0], &nodes[t.1]);
    let (dx, dy) = ((b.x - a.x) / 2.0, b.y - a.y);
    let o = 4.0 * s * (1.0 - s) * (t.2 + 0.07 * (s * 2.5 * PI + t.3).sin());
    (
        a.x + (b.x - a.x) * s - dy * o * 2.0,
        a.y + (b.y - a.y) * s + dx * o,
    )
}

/// Where the web is, cell by cell (0 → 1): the afterglow drains toward it.
fn mask(f: &Frame, n: &Now, nodes: &[Node], edges: &[Thread]) -> Vec<f32> {
    let (cols, rows) = (f.w as i32, n.rows);
    let mut m = vec![0f32; (cols * rows).max(0) as usize];
    // a soft round blob `r` rows across, `k` at its middle
    let mut blob = |x: f64, y: f64, r: f64, k: f32| {
        let (rx, ry) = ((r * 2.0).ceil() as i32, r.ceil() as i32);
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        let inv = 1.0 / (r * r) as f32;
        for cy in (yi - ry).max(0)..=(yi + ry).min(rows - 1) {
            let dy = (cy as f64 - y) as f32;
            for cx in (xi - rx).max(0)..=(xi + rx).min(cols - 1) {
                let dx = (cx as f64 - x) as f32 / 2.0;
                let q = 1.0 - (dx * dx + dy * dy) * inv;
                if q > 0.0 {
                    let c = &mut m[(cy * cols + cx) as usize];
                    *c = c.max(k * q * q);
                }
            }
        }
    };
    let s = n.s.max(0.7);
    for t in edges {
        let (na, nb) = (&nodes[t.0], &nodes[t.1]);
        let len = ((nb.x - na.x) / 2.0).hypot(nb.y - na.y);
        let steps = len.ceil().max(1.0) as i32;
        for i in 0..=steps {
            let (x, y) = along(nodes, t, i as f64 / steps as f64);
            blob(x, y, 2.6 * s, 0.75);
        }
    }
    for g in nodes {
        blob(g.x, g.y, (2.8 + g.size) * s, 1.0);
    }
    m
}

pub(super) fn draw(cv: &mut Canvas, f: &Frame) {
    let n = Now::of(f);
    if n.fade <= 0.0 {
        return;
    }
    let noise = Noise::new(f.seed);
    let mut l = Light::new(f.w as i32, n.rows);
    void(&mut l, f, &n, &noise);
    point(&mut l, f, &n);
    let mut named: Vec<Node> = Vec::new();
    if n.u >= BANG {
        let nodes = nodes(f, &n);
        let edges = threads(f, &nodes);
        let m = mask(f, &n, &nodes, &edges);
        fire(&mut l, f, &n, &noise, &m);
        web(&mut l, f, &n, &nodes, &edges);
        dust(&mut l, f, &n);
        far(&mut l, f, &n);
        for g in &nodes {
            galaxy(&mut l, f, &n, g);
        }
        named = nodes.into_iter().filter(|g| g.name.is_some()).collect();
    }
    l.scale(n.fade as f32);
    // the fireball's gas boils; the rest holds still
    let boil = if (BANG..FIRE).contains(&n.u) {
        (f.t / 60.0) as u64
    } else {
        0
    };
    l.draw(cv, boil);
    stars(cv, &l, f, &n);
    heart(cv, f, &n);
    names(cv, f, &n, &named);
}

/// The void: a dim violet haze in clouds, breathing, swirling in toward
/// the point as it deepens; quantum foam flickering in it, falling inward.
fn void(l: &mut Light, f: &Frame, n: &Now, noise: &Noise) {
    let k = f.act(0.0, 0.07) * (1.0 - smooth(n.u, INHALE.0 + 0.02, BANG + 0.01));
    if k <= 0.01 {
        return;
    }
    let deep = f.act(POINT.0, POINT.1);
    let pull = f.act(POINT.0, INHALE.1);
    let t = f.t / 1000.0;
    let breath = (t * TAU * 0.7).sin();
    let (cx, cy) = (f.cx, f.cy);
    l.gas((0, f.w as i32 - 1), (0, n.rows - 1), |x, y| {
        let (dx, dy) = ((x - cx) / 2.0, y - cy);
        let d = dx.hypot(dy);
        // swirled round the point, more the nearer, and breathing
        let tw = pull * 2.4 / (1.0 + d * 0.25);
        let (sw, cw) = tw.sin_cos();
        let z = 1.0 + 0.05 * breath + pull * 0.5 / (1.0 + d * 0.1);
        let (rx, ry) = ((dx * cw - dy * sw) * z, (dx * sw + dy * cw) * z);
        let m = noise.fbm(rx / 7.0 + t * 0.2, ry / 4.0 - t * 0.15, 3);
        let v = smooth(m, 0.46, 0.82);
        let near = 1.0 / (1.0 + (d / (4.0 + 8.0 * deep)).powi(2));
        let kk = k * (0.85 + 0.15 * breath) * v * (0.16 + 0.6 * deep * near);
        lit(mix(VOID, COOL[3], near * deep * 0.7), kk)
    });
    // foam: specks flickering in and out, each a while in one place, then
    // somewhere else; falling in, a little round, as the point deepens
    let count = ((f.w * n.rows as f64) / 14.0).clamp(30.0, 800.0) as u64;
    let tick = f.t / 90.0;
    for i in 0..count {
        let life = tick / 5.0 + unit(f.seed, 6000 + i);
        let epoch = life.floor() as u64;
        let ph = life - life.floor();
        let h = hash(f.seed ^ 0xf0a, i << 20 ^ epoch);
        let (x, y) = (unit(h, 1) * f.w, unit(h, 2) * n.rows as f64);
        let (dx, dy) = ((x - cx) / 2.0, y - cy);
        let d = dx.hypot(dy).max(0.5);
        let fall = (deep * 0.6 + 0.4 * f.act(INHALE.0, INHALE.1)) * ph;
        let dd = d * (1.0 - fall * (8.0 / (d + 8.0)));
        let a = dy.atan2(dx) + fall * 3.0 / (d * 0.3 + 1.0);
        let on = (ph * PI).sin().powi(2) * (0.4 + 0.6 * unit(h, 3));
        let col = mix(FOAM, COOL[1], deep * (6.0 / (dd + 6.0)));
        let (px, py) = (cx + a.cos() * dd * 2.0, cy + a.sin() * dd);
        l.point(px, py, lit(col, k * on * (0.8 + 1.0 * deep)));
        if fall > 0.05 {
            // a short streak behind it as it falls
            for j in 1..3 {
                let back = dd + 0.3 * j as f64;
                l.point(
                    cx + a.cos() * back * 2.0,
                    cy + a.sin() * back,
                    lit(col, k * on * 0.5 / j as f64),
                );
            }
        }
    }
}

/// The point's heartbeat: its phase (beats so far) and its pulse, 0 → 1,
/// quickening from 1.2 a second to 5.
fn beat(f: &Frame) -> (f64, f64) {
    let ts = f.t / 1000.0;
    let (a, b) = (POINT.0 * f.len / 1000.0, INHALE.1 * f.len / 1000.0);
    let beats = 1.2 * ts + 3.8 * ismooth(ts, a, b);
    (beats, (0.5 + 0.5 * (TAU * beats).sin()).powi(3))
}

/// The point: a white-hot heart in a glow of gold and rose, diffraction
/// spikes, beating quicker as it deepens, a ring of the void drawn in to
/// it with each beat; drawing in its breath before it gives.
fn point(l: &mut Light, f: &Frame, n: &Now) {
    let on = f.act(POINT.0, POINT.0 + 0.07);
    let gone = smooth(n.u, BANG + 0.004, BANG + 0.05);
    let k = on * (1.0 - gone);
    if k <= 0.0 {
        return;
    }
    let deep = f.act(POINT.0, POINT.1);
    let inhale = f.act(INHALE.0, INHALE.1);
    let (beats, pulse) = beat(f);
    let s = n.s.max(0.55);
    let size = (0.4 + 1.8 * deep) * (1.0 - 0.8 * inhale) * (1.0 + 0.3 * pulse) * s;
    let heart = (1.6 + 3.0 * deep + 5.0 * inhale) * k;
    let (x, y) = (f.cx, f.cy);
    l.splat(x, y, 0.25 + 0.15 * size, lit(HOT, heart));
    l.halo(
        x,
        y,
        0.6 * size,
        3.0 + 10.0 * size,
        lit(COOL[1], (1.3 + 0.8 * pulse) * k * (0.35 + 0.65 * deep)),
    );
    l.halo(
        x,
        y,
        2.2 * size,
        6.0 + 14.0 * size,
        lit(COOL[3], 0.42 * k * deep * (1.0 - 0.7 * inhale)),
    );
    // its spikes: four long, four short between; drawn in with its breath
    let len = (1.2 + 11.0 * deep) * (1.0 - 0.75 * inhale) * (0.9 + 0.2 * pulse) * s;
    for i in 0..8 {
        let main = i % 2 == 0;
        let ll = if main { len } else { len * 0.4 };
        let kk = k * if main { 1.7 } else { 0.9 } * (0.5 + 0.5 * deep + inhale);
        l.ray(x, y, i as f64 * PI / 4.0, ll, lit(SPIKE, kk), |s| {
            (1.0 - s).powi(2)
        });
    }
    // the void drawn in with each beat: a ring closing on it
    let ring = deep * (1.0 - inhale) * k;
    if ring > 0.05 {
        let q = beats.fract();
        let r = (5.0 + 16.0 * deep) * s * (1.0 - q).powf(0.7);
        let steps = ((TAU * r * 8.0) as usize).clamp(16, 3000);
        let kk = ring * 0.55 * (q * PI).sin();
        let col = mix(VOID, COOL[3], q);
        for i in 0..steps {
            let a = TAU * i as f64 / steps as f64;
            l.point(x + a.cos() * r * 2.0, y + a.sin() * r, lit(col, kk));
        }
    }
}

/// The point's heart, crisp over its light; a burst as it gives.
fn heart(cv: &mut Canvas, f: &Frame, n: &Now) {
    let on = f.act(POINT.0, POINT.0 + 0.05) * (1.0 - smooth(n.u, BANG + 0.01, BANG + 0.06));
    if on <= 0.0 {
        return;
    }
    let deep = f.act(POINT.0, POINT.1);
    let g = if n.u >= BANG - 0.01 {
        "✹"
    } else if deep > 0.55 {
        "✸"
    } else {
        "✦"
    };
    cv.put_with(f.cx as i32, f.cy as i32, g, HOT, on * n.fade, true);
}

/// The eruption: a flash; a fireball swelling past the screen's edges,
/// billowing (its lobes lit from inside) and filamented, cooling from white
/// through gold and rose to violet, hottest at its shock front, a corona
/// round it, sparks thrown ahead. Its afterglow drains into the web (`m`)
/// and fades, but for its brightest lobes: they become nebulae, in their
/// own colours, dust across them.
fn fire(l: &mut Light, f: &Frame, n: &Now, noise: &Noise, m: &[f32]) {
    let (q, r) = (n.q, n.r.max(0.3));
    let t = f.t / 1000.0;
    let (cx, cy) = (f.cx, f.cy);
    let cols = f.w as i32;
    // the flash: a white wash for a moment
    let flash = 1.0 - smooth(n.u, BANG, BANG + 0.05);
    if flash > 0.01 {
        l.gas((0, cols - 1), (0, n.rows - 1), |x, y| {
            let d = ((x - cx) / 2.0).hypot(y - cy);
            lit(COOL[1], flash * (0.05 + 2.5 / (1.0 + (d / 7.0).powi(2))))
        });
    }
    let heat = (1.0 - q).powf(1.2);
    let gain = 3.4 / (1.0 + 6.0 * q);
    let drain = f.act(WEB.0, WEB.1);
    let late = 1.0 - 0.9 * f.act(0.62, 0.9);
    let neb = f.act(0.55, 0.85);
    // the nebulae drift out as the view pushes in
    let z = 1.0 + n.push * 0.5;
    let halo = 5.0 * (1.0 - q).powi(2);
    let (x0, x1) = (
        (cx - (r + halo) * 2.0 - 3.0) as i32,
        (cx + (r + halo) * 2.0 + 3.0) as i32,
    );
    let (y0, y1) = ((cy - r - halo - 2.0) as i32, (cy + r + halo + 2.0) as i32);
    l.gas((x0, x1), (y0, y1), |x, y| {
        let (dx, dy) = ((x - cx) / 2.0 / z, (y - cy) / z);
        let d = dx.hypot(dy).max(0.01);
        let rr = d / r;
        if d > r + halo + 0.5 {
            return [0.0; 3];
        }
        // the gas's own place, carried out with it as it spreads
        let sc = 2.6 / r.max(3.0);
        let (gu, gv) = (dx * sc + 3.0 + t * 0.35, dy * sc - 1.0 - t * 0.25);
        let (au, av) = (dx / d, dy / d);
        let n1 = noise.fbm(gu, gv, 3);
        // read a little inward: lobes facing out are lit, facing in shadowed
        let n2 = noise.fbm(gu - au * 0.12, gv - av * 0.12, 3);
        let relief = ((n1 - n2) * 9.0).clamp(-1.0, 1.0);
        let lobes = smooth(n1, 0.36, 0.7);
        let fil = noise.ridge(gu * 1.6 + 6.0, gv * 1.6, 2).powi(4);
        // fingers: streaks from the middle out
        let fing = noise.at(au * 2.2 + 40.0, av * 2.2 + 40.0);
        // filled at first, hollowing to a shell as it spreads
        let body = (1.0 - q) * (1.0 - 0.35 * rr * rr) + q * (0.2 + 0.8 * rr * rr);
        let dens = body * (0.04 + lobes * (1.0 + 0.8 * relief) + 1.6 * fil) * (0.6 + 0.8 * fing);
        let inside = 1.0 - smooth(rr, 0.95, 1.03);
        let front = (-((rr - 1.0) * r / 0.8).powi(2)).exp() * (1.0 - q).powf(0.7);
        let corona = if d > r && halo > 0.01 {
            (-(d - r) / (halo * 0.4)).exp() * (1.0 - q).powi(2)
        } else {
            0.0
        };
        let temp =
            (heat * (0.62 + 0.38 * rr) + 0.3 * (lobes - 0.5) * heat + 0.4 * front).clamp(0.0, 1.0);
        let col = ramp(&COOL, 1.0 - temp);
        // late, it gathers where the web is and fades elsewhere
        let (ci, ri) = (x.round() as i32, y.round() as i32);
        let w = if ci >= 0 && ri >= 0 && ci < cols && ri < n.rows {
            m[(ri * cols + ci) as usize] as f64
        } else {
            0.0
        };
        let keep = (1.0 - drain * (1.0 - 2.4 * w)).clamp(0.0, 1.8);
        let k = gain * dens * inside * keep * late + 1.8 * front + 0.9 * corona;
        let mut out = lit(col, k);
        // its brightest lobes become nebulae: wisps of glowing gas, their
        // colour shifting across them, lanes of dust dark through them
        if neb > 0.0 {
            let cloud = smooth(n1, 0.47, 0.74);
            if cloud > 0.0 {
                let wisp = noise.ridge(dx / 2.6 + 13.0, dy / 1.6 - 4.0, 2);
                let lanes = noise.ridge(dx / 7.0 + 70.0, dy / 4.0, 2).powi(4);
                let hue = noise.at(gu * 0.6 - 50.0, gv * 0.6 + 20.0);
                let tone = (cloud * 0.6 + hue * 0.9 - 0.3 + 0.3 * wisp).clamp(0.0, 1.0);
                let nc = mix(ramp(&NEB, tone), (255, 226, 244), wisp.powi(6) * 0.5);
                let glow = cloud
                    * (0.15 + 1.2 * wisp.powi(3) + 0.3 * relief.max(0.0))
                    * (1.0 - 0.9 * lanes);
                let nl = lit(nc, neb * 1.25 * glow);
                out = [out[0] + nl[0], out[1] + nl[1], out[2] + nl[2]];
            }
        }
        out
    });
    // the shock front, a thin bright line, rippled
    let fk = (1.0 - q).powf(0.8);
    if fk > 0.02 && r > 0.5 {
        let steps = ((TAU * r * 8.0) as usize).clamp(24, 6000);
        let col = mix(HOT, COOL[3], q);
        for i in 0..steps {
            let a = TAU * i as f64 / steps as f64;
            let rip = 1.0 + 0.03 * (noise.at(a.cos() * 3.0 + 70.0, a.sin() * 3.0) - 0.5);
            l.point(
                cx + a.cos() * r * rip * 2.0,
                cy + a.sin() * r * rip,
                lit(col, 1.6 * fk),
            );
        }
    }
    // sparks thrown ahead of it: short streaks out from the middle
    let sk = (1.0 - q).powf(1.1) * smooth(q, 0.0, 0.03);
    if sk > 0.02 {
        for i in 0..240u64 {
            let a = unit(f.seed, 8000 + i) * TAU;
            let v = 1.02 + 0.3 * unit(f.seed, 8300 + i);
            let (ca, sa) = (a.cos(), a.sin());
            let col = mix(HOT, COOL[2], unit(f.seed, 8600 + i) * 0.6 + q * 0.4);
            for j in 0..4 {
                let d = r * v - j as f64 * 0.35;
                l.point(
                    cx + ca * d * 2.0,
                    cy + sa * d,
                    lit(col, sk * (1.4 - 0.3 * j as f64)),
                );
            }
        }
    }
}

/// The cosmic web: threads condensing between the knots (from the knots
/// out), clumped, wavering, gas strewn along them; the knots warm with
/// gathered light.
fn web(l: &mut Light, f: &Frame, n: &Now, nodes: &[Node], edges: &[Thread]) {
    let c = f.act(WEB.0, WEB.1);
    if c <= 0.0 {
        return;
    }
    let k = c * (1.0 - 0.5 * f.act(0.8, 1.0));
    for (e, t) in edges.iter().enumerate() {
        let (na, nb) = (&nodes[t.0], &nodes[t.1]);
        let len = ((nb.x - na.x) / 2.0).hypot(nb.y - na.y);
        let steps = (len * 8.0).ceil().max(2.0) as i32;
        let col = mix(THREAD[0], THREAD[1], unit(f.seed, 9100 + e as u64));
        let (ux, uy) = (
            (nb.x - na.x) / 2.0 / len.max(0.1),
            (nb.y - na.y) / len.max(0.1),
        );
        for i in 0..=steps {
            let s = i as f64 / steps as f64;
            // from the knots in toward the middle
            let mid = 0.5 - (s - 0.5).abs();
            let reveal = smooth(c * 1.4 - mid * 0.9, 0.0, 0.35);
            if reveal <= 0.0 {
                continue;
            }
            // clumped: beads along it
            let bead = 0.5 + 0.5 * (s * len * 1.3 + t.3 * 3.0).sin() * (s * len * 0.47 + t.3).cos();
            let kk = k * reveal * (0.4 + 1.4 * bead * bead);
            let (x, y) = along(nodes, t, s);
            l.point(x, y, lit(col, kk));
            // gas strewn either side of it
            if i % 2 == 0 {
                let h = hash(f.seed ^ 0x3e, (e as u64) << 16 ^ i as u64);
                let o = (unit(h, 1) - 0.5) * 1.8 * n.s.max(0.6);
                l.point(
                    x - uy * o * 2.0,
                    y + ux * o,
                    lit(col, kk * (0.55 - o.abs() * 0.25)),
                );
            }
        }
    }
    for g in nodes.iter().filter(|g| g.name.is_none()) {
        l.halo(
            g.x,
            g.y,
            0.35 * n.s.max(0.6),
            (2.0 + 0.8 * g.size) * n.s.max(0.6),
            lit(KNOT, k * 0.5 * g.w),
        );
    }
}

/// The faint stars: a fine dust of points lighting up behind everything.
fn dust(l: &mut Light, f: &Frame, n: &Now) {
    let rows = n.rows as f64;
    let count = ((f.w * rows) / 6.0).clamp(60.0, 2000.0) as u64;
    for i in 0..count {
        let h = hash(f.seed ^ 0xd057, i);
        let born = 0.54 + 0.3 * unit(h, 0);
        let on = smooth(n.u, born, born + 0.05);
        if on <= 0.0 {
            continue;
        }
        let tw = 0.7 + 0.3 * (f.t / (130.0 + 170.0 * unit(h, 4)) + i as f64).sin();
        let (x, y) = n.dolly(unit(h, 1) * f.w, unit(h, 2) * rows, 0.25);
        l.point(x, y, lit(class(h), on * tw * (0.25 + 0.5 * unit(h, 3))));
    }
}

/// The brighter stars, as glyphs over the light, in their classes'
/// colours; twinkling; never over a galaxy's bright heart.
fn stars(cv: &mut Canvas, l: &Light, f: &Frame, n: &Now) {
    if n.u < BANG {
        return;
    }
    let rows = n.rows as f64;
    let count = ((f.w * rows) / 70.0).clamp(12.0, 220.0) as u64;
    for i in 0..count {
        let h = hash(f.seed ^ 0x57a5, i);
        let born = 0.56 + 0.3 * unit(h, 0);
        let on = smooth(n.u, born, born + 0.04) * n.fade;
        if on <= 0.0 {
            continue;
        }
        let (x, y) = n.dolly(unit(h, 1) * f.w, unit(h, 2) * rows, 0.6);
        if y > rows - 0.5 || l.level(x.round() as i32, y.round() as i32) > 0.45 {
            continue;
        }
        let b = unit(h, 3).powi(3);
        let tw = 0.8 + 0.2 * (f.t / (110.0 + 150.0 * unit(h, 4)) + i as f64).sin();
        star(cv, x, y, class(h), on * tw * (0.3 + 0.75 * b));
    }
}

/// Far galaxies: small smudges between the near ones, for depth.
fn far(l: &mut Light, f: &Frame, n: &Now) {
    let on = f.act(0.66, 0.86);
    if on <= 0.0 {
        return;
    }
    let count = ((f.w * n.rows as f64) / 220.0).clamp(4.0, 50.0) as u64;
    for i in 0..count {
        let h = hash(f.seed ^ 0xfa5, i);
        let (x, y) = n.dolly(unit(h, 1) * f.w, unit(h, 2) * n.rows as f64, 0.4);
        let (r, flat, pa) = (
            (0.4 + 0.5 * unit(h, 3)) * n.s.max(0.6),
            0.25 + 0.5 * unit(h, 4),
            (unit(h, 5) - 0.5) * 1.4,
        );
        let col = mix(ARMS[(h % 3) as usize], CORE, 0.3);
        let steps = (r * 10.0).ceil() as i32;
        for j in -steps..=steps {
            let s = j as f64 / steps as f64 * r;
            let k = on * 0.55 * (1.0 - (s / r).abs()).powf(0.7);
            l.point(x + pa.cos() * s * 2.0, y + pa.sin() * s * flat, lit(col, k));
        }
        l.point(x, y, lit(CORE, on * 0.9));
    }
}

/// A galaxy forming at a knot: a glow, then a spiral winding up — a warm
/// bulge in a soft disc, arms turning, warm near the heart and blue-white
/// out along them (an edge-on one a streak with a dust lane), young stars
/// along them in pink.
fn galaxy(l: &mut Light, f: &Frame, n: &Now, g: &Node) {
    let form = smooth(n.u, g.born, g.born + 0.16);
    if form <= 0.0 {
        return;
    }
    let bright = if g.name.is_some() {
        1.2
    } else {
        0.7 + 0.4 * g.w
    };
    let k = form * bright;
    let size = g.size * (0.6 + 0.4 * form) * (1.0 + n.push);
    let (ca, sa) = (g.pa.cos(), g.pa.sin());
    let spin = g.spin * f.t / 1000.0;
    let wind = (0.25 + 0.75 * form) / g.pitch.tan();
    let edge_on = g.flat < 0.3;
    // a point of its own plane (rows from its heart, `pu` along its long
    // axis, `pv` across before it's tilted away) on screen
    let at = |pu: f64, pv: f64| {
        let pv = pv * g.flat;
        (g.x + (pu * ca - pv * sa) * 2.0, g.y + pu * sa + pv * ca)
    };
    // its body: a soft disc, the warm bulge (a dark lane across it, edge-on)
    let (x0, x1) = (
        (g.x - size * 2.0 - 1.0).floor() as i32,
        (g.x + size * 2.0 + 1.0).ceil() as i32,
    );
    let (y0, y1) = (
        (g.y - size - 1.0).floor() as i32,
        (g.y + size + 1.0).ceil() as i32,
    );
    l.field((x0, x1), (y0, y1), |x, y| {
        let (dx, dy) = ((x - g.x) / 2.0, y - g.y);
        let pu = dx * ca + dy * sa;
        let pv = -dx * sa + dy * ca;
        let r = pu.hypot(pv / g.flat) / size;
        if r > 1.1 {
            return [0.0; 3];
        }
        let bulge = (-(r / 0.08).powi(2)).exp() * 2.0 + (-(r / 0.2).powi(2)).exp() * 0.35;
        let disc = (-r / 0.4).exp() * (1.0 - smooth(r, 0.6, 1.05));
        let lane = if edge_on {
            1.0 - 0.8 * (-(pv / (0.05 * size)).powi(2)).exp() * smooth(r, 0.06, 0.2)
        } else {
            1.0
        };
        let (la, lb) = (
            lit(g.arm, disc * if edge_on { 0.75 } else { 0.3 } * k * lane),
            lit(CORE, bulge * k * lane.max(0.35)),
        );
        [la[0] + lb[0], la[1] + lb[1], la[2] + lb[2]]
    });
    if edge_on {
        return;
    }
    // its arms: logarithmic spirals out from the bulge, winding up as it
    // forms, turning; warm near the heart, its own colour out along them,
    // a fainter strand beside each
    let arms = g.arms as f64;
    let steps = (size * 28.0).ceil() as i32;
    for m in 0..g.arms {
        let base = TAU * m as f64 / arms - spin;
        for s in 0..=steps {
            let q = s as f64 / steps as f64;
            // grown out from the heart as it forms
            let reach = smooth(form * 1.25 - q, 0.0, 0.2);
            if reach <= 0.0 {
                break;
            }
            let r = 0.1 + 0.95 * q;
            let th = base + wind * (r / 0.1).ln();
            let b = k * reach * (0.85 - 0.55 * q);
            let col = mix(CORE, g.arm, smooth(q, 0.0, 0.25));
            let (x, y) = at(th.cos() * r * size, th.sin() * r * size);
            l.point(x, y, lit(col, b));
            let th2 = th + 0.12 / r.max(0.2);
            let (x, y) = at(th2.cos() * r * size, th2.sin() * r * size);
            l.point(x, y, lit(col, b * 0.5));
        }
    }
    // young stars along the arms, pink and white
    if form > 0.3 {
        let knots = (4.0 + 6.0 * size) as u64;
        for j in 0..knots {
            let h = hash(f.seed ^ 0x9a1, ((g.x * 131.0 + g.y) as u64) << 8 ^ j);
            let r = 0.25 + 0.65 * unit(h, 1);
            let base = TAU * (h % g.arms as u64) as f64 / arms - spin;
            let th = base + wind * (r / 0.1).ln() + 0.25 * (unit(h, 2) - 0.5);
            let (x, y) = at(th.cos() * r * size, th.sin() * r * size);
            let tw = 0.6 + 0.4 * (f.t / 150.0 + j as f64).sin();
            let col = if j % 3 == 0 { YOUNG } else { (225, 235, 255) };
            l.point(x, y, lit(col, k * 1.2 * tw * (form - 0.3)));
        }
    }
}

/// Your machines' galaxies named, beside them.
fn names(cv: &mut Canvas, f: &Frame, n: &Now, named: &[Node]) {
    let on = f.act(NAMES.0, NAMES.1) * n.fade;
    if on <= 0.0 {
        return;
    }
    for g in named {
        let Some((name, rgb)) = &g.name else {
            continue;
        };
        let size = g.size * (1.0 + n.push);
        let y = g.y.round() as i32;
        let lx = (g.x + size * 2.0 + 2.0).round() as i32;
        cv.text(lx, y, name, mix(*rgb, (255, 255, 255), 0.3), on);
    }
}

/// Where its particles are as the ℵ gathers: the galaxies and the brighter
/// stars.
pub(super) fn sources(f: &Frame) -> Vec<(f64, f64)> {
    let end = Frame {
        t: f.len,
        info: f.info,
        ..*f
    };
    let n = Now::of(&end);
    let mut v: Vec<(f64, f64)> = nodes(&end, &n).iter().map(|g| (g.x, g.y)).collect();
    let rows = n.rows as f64;
    let count = ((f.w * rows) / 70.0).clamp(12.0, 220.0) as u64;
    for i in 0..count {
        let h = hash(f.seed ^ 0x57a5, i);
        v.push(n.dolly(unit(h, 1) * f.w, unit(h, 2) * rows, 0.6));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::super::scene;
    use super::*;

    fn info() -> Info {
        let m = |name: &str, rgb| Mach {
            name: name.into(),
            rgb,
            up: true,
            sessions: vec![],
        };
        Info {
            machines: 3,
            sessions: 4,
            needs: 0,
            fleet: vec![
                m("normandy", (120, 200, 255)),
                m("serenity", (255, 150, 120)),
                m("tycho", (150, 255, 160)),
            ],
        }
    }

    fn lit_cells(b: &Buffer, rows: u16) -> usize {
        (0..rows)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .filter(|p| b[*p].symbol() != " ")
            .count()
    }

    /// Its worst frame's time at 200 × 60 (`cargo test --release
    /// frame_times -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn frame_times() {
        let info = info();
        let mut times: Vec<(f64, u64)> = Vec::new();
        for round in 0..2 {
            for ms in (0..4600).step_by(16) {
                let f = scene::at(200, 60, ms, true, &info);
                let t0 = std::time::Instant::now();
                let b = scene::dark(Kind::Singularity, &f);
                let dt = t0.elapsed().as_secs_f64() * 1000.0;
                std::hint::black_box(b);
                if round == 1 {
                    times.push((dt, ms));
                }
            }
        }
        let mean = times.iter().map(|t| t.0).sum::<f64>() / times.len() as f64;
        times.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("worst {:?}; mean {mean:.2} ms", &times[..6]);
    }

    #[test]
    fn the_point_burns_white_hot_at_the_heart_of_its_glow() {
        let info = info();
        let f = scene::at(120, 32, 700, false, &info);
        let b = scene::dark(Kind::Singularity, &f);
        let (cx, cy) = (f.cx as u16, f.cy as u16);
        let Color::Rgb(r, g, bl) = b[(cx, cy)].fg else {
            panic!("no heart");
        };
        assert!(r > 230 && g > 210 && bl > 170, "{r} {g} {bl}");
        // its spikes reach out level with it, either side
        let along = (6..14).filter(|d| b[(cx + d, cy)].symbol() != " ").count();
        assert!(along >= 6, "spike: {along}");
    }

    #[test]
    fn the_eruption_fills_the_screen_and_cools_as_it_spreads() {
        let info = info();
        let rows = 29;
        let at = |u: f64| {
            let f = scene::at(120, 32, (scene::SCENE_MS as f64 * u) as u64, false, &info);
            scene::dark(Kind::Singularity, &f)
        };
        // early: a ball, white-gold; then it fills nearly every cell
        let early = at(BANG + 0.04);
        let later = at(FIRE - 0.06);
        assert!(
            lit_cells(&later, rows) > 120 * 29 / 2,
            "{}",
            lit_cells(&later, rows)
        );
        let warm = |b: &Buffer| {
            let (mut r, mut bl) = (0f64, 0f64);
            for y in 0..rows {
                for x in 0..120 {
                    if let Color::Rgb(rr, _, bb) = b[(x, y)].fg {
                        r += rr as f64;
                        bl += bb as f64;
                    }
                }
            }
            r / bl.max(1.0)
        };
        assert!(
            warm(&early) > warm(&later) * 1.15,
            "{} {}",
            warm(&early),
            warm(&later)
        );
    }

    #[test]
    fn it_ends_on_a_deep_field_with_your_machines_named() {
        let info = info();
        let f = scene::at(120, 32, scene::SCENE_MS - 1, false, &info);
        let b = scene::dark(Kind::Singularity, &f);
        let all: String = (0..29)
            .map(|y| (0..120).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect();
        for m in ["normandy", "serenity", "tycho"] {
            assert!(all.contains(m), "{m}");
        }
        // rich, not sparse: a good share of the sky has light in it
        let n = lit_cells(&b, 29);
        assert!(n > 120 * 29 / 5, "{n}");
    }
}
