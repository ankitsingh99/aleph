//! `convergence`: the AI singularity. A network wakes across the screen:
//! nodes joined by fine edges, signals running along them as pulses; your
//! machines are its hubs, named, their sessions satellites round them in
//! their states' colours. It learns: activity spreads out from the hubs,
//! new nodes and edges keep appearing, attention arcs leap between distant
//! nodes, the pulses come faster and faster. Then it converges: everything
//! drifts toward the middle, spiralling in, the edges shortening, the
//! network ever denser and hotter, until it collapses into one brilliant
//! core and a ring of light goes out. It ends on the core, quietly
//! thinking: slow pulses, a few thoughts turning round it. With the logo,
//! the core unfolds into the ℵ, held as a constellation: nodes along its
//! strokes, signals running between them.

use super::scene::{ismooth, Frame, GATHER_S};
use super::*;

type Rgb = (u8, u8, u8);

pub(super) const TITLE: &str = "many minds becoming one";
/// Its colours on the way into the ℵ: synapse magenta, then gold.
pub(super) const COLS: [Rgb; 2] = [(255, 120, 230), (255, 214, 120)];

/// Synapses: the quiet edges' indigo, violet, the signals' cyan, attention's
/// magenta, white.
const INDIGO: Rgb = (78, 70, 200);
const VIOLET: Rgb = (150, 104, 255);
const CYAN: Rgb = (96, 228, 255);
const MAGENTA: Rgb = (255, 84, 214);
const WHITE: Rgb = (238, 244, 255);
/// The core: white-hot, gold, amber.
const CORE: [Rgb; 3] = [(255, 250, 228), (255, 206, 100), (236, 140, 50)];

/// The acts, as shares of the scene: it wakes; learns (growing, attention
/// leaping across it); converges, spiralling in; collapses into the core;
/// the ring goes out; the core thinks.
const WAKE: (f64, f64) = (0.0, 0.12);
const GROW: (f64, f64) = (0.14, 0.56);
const LEARN: (f64, f64) = (0.16, 0.6);
const CONVERGE: (f64, f64) = (0.5, 0.78);
const COLLAPSE: f64 = 0.78;
const RING: (f64, f64) = (0.785, 0.95);
const THINK: (f64, f64) = (0.83, 1.0);

/// How far it turns as it spirals in (radians, the outermost).
const SPIN: f64 = 1.3 * PI;

/// What a node is: a machine (named), one of its sessions (orbiting it),
/// or one of the network's own.
#[derive(Clone)]
enum Role {
    Hub(Option<String>),
    /// its hub, its orbit's radius (rows) and where on it
    Sat(usize, f64, f64),
    Plain,
}

#[derive(Clone)]
struct Node {
    role: Role,
    /// where it lives (cells), before it converges
    home: (f64, f64),
    /// when it appears, and when the activity reaches it (shares)
    born: f64,
    wake: f64,
    rgb: Rgb,
    /// how late it starts in (0–1)
    lag: f64,
}

/// The network at one moment.
struct Net {
    nodes: Vec<Node>,
    /// (from, to, when it's made)
    edges: Vec<(usize, usize, f64)>,
    /// the last row it lives on
    floor: f64,
    /// its reach from the centre (rows): the farthest home
    reach: f64,
}

/// The rows the network lives in (the wordmark and the console below).
fn floor(f: &Frame) -> f64 {
    (f.h - 4.0).max(2.0)
}

/// Distance in rows (cells are twice as tall as wide).
fn apart(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0) / 2.0).hypot(a.1 - b.1)
}

impl Net {
    fn of(f: &Frame) -> Net {
        let floor = floor(f);
        let (rx, ry) = (
            (f.w / 2.0 - 4.0).max(4.0),
            (f.cy - 0.5).min(floor - f.cy - 0.5).max(1.5),
        );
        let mut nodes: Vec<Node> = Vec::new();
        // the hubs: your machines (three at least), round the middle
        let fleet = &f.info.fleet;
        let m = fleet.len().clamp(3, 6);
        let turn = unit(f.seed, 0xc0) * TAU;
        for i in 0..m {
            let k = i as u64;
            let a = turn + (i as f64 + 0.3 * unit(f.seed, 0xc1 + k)) / m as f64 * TAU;
            let r = 0.5 + 0.18 * unit(f.seed, 0xc8 + k);
            let home = (f.cx + a.cos() * rx * r, f.cy + a.sin() * ry * r);
            let mach = fleet.get(i);
            nodes.push(Node {
                role: Role::Hub(mach.map(|m| m.name.chars().take(14).collect())),
                home,
                born: 0.01 + 0.04 * i as f64 / m as f64,
                wake: WAKE.0 + 0.02 + 0.03 * i as f64 / m as f64,
                rgb: mach.map_or(if i.is_multiple_of(2) { CYAN } else { MAGENTA }, |m| m.rgb),
                lag: 0.9 + 0.1 * unit(f.seed, 0xd0 + k),
            });
        }
        // their sessions, orbiting them (a machine without any: two of its own)
        let orbit = (ry * 0.16).clamp(1.2, 3.2);
        for h in 0..m {
            let cols: Vec<Rgb> = match fleet.get(h) {
                Some(mach) => mach.sessions.iter().take(8).copied().collect(),
                None => vec![VIOLET; 2],
            };
            let n = cols.len();
            for (j, rgb) in cols.into_iter().enumerate() {
                let k = (h * 16 + j) as u64;
                let a = (j as f64 + 0.4 * unit(f.seed, 0xe0 + k)) / n as f64 * TAU;
                let hub = nodes[h].home;
                nodes.push(Node {
                    role: Role::Sat(h, orbit * (0.85 + 0.3 * unit(f.seed, 0xe8 + k)), a),
                    home: (hub.0 + a.cos() * orbit * 2.0, hub.1 + a.sin() * orbit),
                    born: 0.04 + 0.06 * unit(f.seed, 0xf0 + k),
                    wake: 0.08 + 0.06 * unit(f.seed, 0xf8 + k),
                    rgb,
                    lag: 0.8 + 0.2 * unit(f.seed, 0x100 + k),
                });
            }
        }
        // the network's own nodes: half there from the first, the rest
        // appearing as it learns, ever faster
        let first = nodes.len();
        let count = ((f.w * floor) / 42.0).clamp(26.0, 190.0) as usize;
        let mut plain: Vec<Node> = (0..count)
            .map(|i| {
                let k = i as u64;
                let home = (
                    2.0 + unit(f.seed, 0x200 + k) * (f.w - 4.0),
                    0.5 + unit(f.seed, 0x300 + k) * (floor - 0.5),
                );
                let born = if i.is_multiple_of(2) {
                    0.12 * unit(f.seed, 0x400 + k)
                } else {
                    GROW.0 + (GROW.1 - GROW.0) * unit(f.seed, 0x400 + k).powf(0.6)
                };
                let pick = unit(f.seed, 0x500 + k);
                Node {
                    role: Role::Plain,
                    home,
                    born,
                    wake: born,
                    rgb: if pick < 0.6 {
                        CYAN
                    } else if pick < 0.85 {
                        VIOLET
                    } else {
                        MAGENTA
                    },
                    lag: unit(f.seed, 0x600 + k),
                }
            })
            .collect();
        plain.sort_by(|a, b| a.born.total_cmp(&b.born));
        nodes.extend(plain);
        // the activity spreads out from the hubs
        let hubs: Vec<(f64, f64)> = nodes[..m].iter().map(|n| n.home).collect();
        let far = rx.hypot(ry * 2.0) / 2.0 + ry;
        for n in nodes[first..].iter_mut() {
            let d = hubs
                .iter()
                .map(|&h| apart(h, n.home))
                .fold(f64::MAX, f64::min);
            n.wake = n.born.max(0.06 + 0.3 * (d / far).min(1.0));
        }
        // the edges: the hubs in a ring, each its sessions; each new node to
        // the nearest two that were there before it (a third, learned later)
        let mut edges = Vec::new();
        for i in 0..m {
            edges.push((i, (i + 1) % m, 0.02));
        }
        for (i, n) in nodes.iter().enumerate() {
            if let Role::Sat(h, _, _) = n.role {
                edges.push((h, i, n.born));
            }
        }
        for i in first..nodes.len() {
            let mut near: Vec<(f64, usize)> = (0..i)
                .filter(|&j| !matches!(nodes[j].role, Role::Sat(..)))
                .map(|j| (apart(nodes[i].home, nodes[j].home), j))
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            let born = nodes[i].born;
            for (r, &(_, j)) in near.iter().take(3).enumerate() {
                let k = (i * 4 + r) as u64;
                match r {
                    0 => edges.push((j, i, born)),
                    1 if unit(f.seed, 0x700 + k) < 0.75 => edges.push((j, i, born)),
                    2 if unit(f.seed, 0x700 + k) < 0.45 => {
                        let at = LEARN.0 + (LEARN.1 - LEARN.0) * unit(f.seed, 0x800 + k);
                        edges.push((j, i, born.max(at)));
                    }
                    _ => {}
                }
            }
        }
        let reach = nodes
            .iter()
            .map(|n| apart(n.home, (f.cx, f.cy)))
            .fold(1.0, f64::max);
        Net {
            nodes,
            edges,
            floor,
            reach,
        }
    }

    /// Node `i`'s place at `u` before it converges: a session orbiting its
    /// hub, everything drifting a little.
    fn place(&self, f: &Frame, i: usize, u: f64) -> (f64, f64) {
        let n = &self.nodes[i];
        let k = i as u64;
        match n.role {
            Role::Sat(h, r, a) => {
                let hub = self.place(f, h, u);
                let a = a + u * (1.5 + unit(f.seed, 0x900 + k));
                (hub.0 + a.cos() * r * 2.0, hub.1 + a.sin() * r)
            }
            _ => {
                let ph = unit(f.seed, 0xa00 + k) * TAU;
                (
                    n.home.0 + 0.7 * (TAU * u * 1.3 + ph).sin(),
                    n.home.1 + 0.3 * (TAU * u * 1.1 + ph * 1.7).sin(),
                )
            }
        }
    }

    /// How far node `i` has converged by `u`, 0–1 (the hubs last).
    fn pull(&self, i: usize, u: f64) -> f64 {
        let a = CONVERGE.0 + 0.07 * self.nodes[i].lag;
        smooth(u, a, CONVERGE.1)
    }

    /// Node `i` on screen at `u`: spiralling in toward the core as it
    /// converges (the inner ones turning faster).
    fn at(&self, f: &Frame, i: usize, u: f64) -> (f64, f64) {
        let (x, y) = self.place(f, i, u);
        let c = self.pull(i, u);
        if c <= 0.0 {
            return (x, y);
        }
        let (du, dv) = ((x - f.cx) / 2.0, y - f.cy);
        let r0 = du.hypot(dv);
        let r = r0 * (1.0 - c).powf(1.4);
        let a = dv.atan2(du) + SPIN * c * c * (1.0 + 0.8 * (1.0 - r0 / self.reach).max(0.0));
        (f.cx + a.cos() * r * 2.0, f.cy + a.sin() * r)
    }
}

/// The pulses' running count: slow at first, ever faster as it learns
/// (closed form, so a frame stays a pure function of its moment).
fn beats(u: f64) -> f64 {
    1.5 * u + 9.0 * ismooth(u, LEARN.0, CONVERGE.1)
}

pub(super) fn draw(cv: &mut Canvas, f: &Frame) {
    let fade = f.fade();
    if fade <= 0.0 {
        return;
    }
    let u = f.u();
    if u < COLLAPSE {
        let net = Net::of(f);
        let pos: Vec<(f64, f64)> = (0..net.nodes.len()).map(|i| net.at(f, i, u)).collect();
        edges(cv, f, &net, &pos, fade);
        trails(cv, f, &net, fade);
        attention(cv, f, &net, &pos, fade);
        nodes(cv, f, &net, &pos, fade);
        names(cv, f, &net, &pos, fade);
    }
    ring(cv, f, fade);
    think(cv, f, fade);
    orb(cv, f, fade);
}

/// A straight line of braille dots from `a` to `b` (as far as `upto`, 0–1).
fn line(d: &mut Dots, a: (f64, f64), b: (f64, f64), upto: f64, k: f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = ((dx.abs() * 2.0).max(dy.abs() * 4.0) * upto)
        .ceil()
        .max(1.0) as usize;
    for s in 0..=n {
        let q = upto * s as f64 / n as f64;
        d.add(a.0 + dx * q, a.1 + dy * q, k);
    }
}

/// A point `t` (0–1) along an arc from `a` to `b`, bowed to one side by
/// `bow` (a share of its length; the other side when negative).
fn arc_at(a: (f64, f64), b: (f64, f64), bow: f64, t: f64) -> (f64, f64) {
    // in rows, then back to cells
    let (du, dv) = ((b.0 - a.0) / 2.0, b.1 - a.1);
    let mid = (
        (a.0 + b.0) / 2.0 - dv * bow * 2.0,
        (a.1 + b.1) / 2.0 + du * bow,
    );
    let (p, r) = (1.0 - t, t);
    (
        p * p * a.0 + 2.0 * p * r * mid.0 + r * r * b.0,
        p * p * a.1 + 2.0 * p * r * mid.1 + r * r * b.1,
    )
}

/// How hot it's running, 0–1: as it converges.
fn heat(f: &Frame) -> f64 {
    f.act(CONVERGE.0, COLLAPSE)
}

/// The edges, growing in as they're made, with signals running along them
/// (an edge lit cyan where one is); violet, then magenta and gold as it all
/// converges.
fn edges(cv: &mut Canvas, f: &Frame, net: &Net, pos: &[(f64, f64)], fade: f64) {
    let u = f.u();
    let beat = beats(u);
    let heat = heat(f);
    let busy = (0.3 + 0.6 * f.act(LEARN.0, LEARN.1)) * (1.0 - heat);
    let mut d = Dots::default();
    for (e, &(a, b, made)) in net.edges.iter().enumerate() {
        let grow = smooth(u, made, made + 0.03);
        if grow <= 0.0 {
            continue;
        }
        let (pa, pb) = (pos[a], pos[b]);
        let woke = net.nodes[a].wake.max(net.nodes[b].wake);
        let awake = smooth(u, woke, woke + 0.05);
        line(
            &mut d,
            pa,
            pb,
            grow,
            (0.5 + 0.12 * awake + 0.25 * heat) * fade,
        );
        // its signal: a short bright run along it, one way or the other —
        // not every edge every beat, more of them as it learns
        if grow < 1.0 || awake <= 0.0 {
            continue;
        }
        let k = e as u64;
        let run = beat * (0.6 + 0.8 * unit(f.seed, 0xb00 + k)) + unit(f.seed, 0xb80 + k);
        if unit(f.seed ^ run.floor() as u64, 0xc00 + k) > busy {
            continue;
        }
        let s = run.fract() * 1.2;
        let (from, to) = if k.is_multiple_of(2) {
            (pa, pb)
        } else {
            (pb, pa)
        };
        for (j, lag) in [0.0, 0.06, 0.12].iter().enumerate() {
            let q = s - lag;
            if (0.0..=1.0).contains(&q) {
                let kk = [1.0, 0.85, 0.72][j] * awake * fade;
                d.add(
                    from.0 + (to.0 - from.0) * q,
                    from.1 + (to.1 - from.1) * q,
                    kk,
                );
            }
        }
    }
    let quiet = mix(INDIGO, VIOLET, 0.45);
    let hot = mix(MAGENTA, CORE[1], heat);
    d.draw(cv, |_, _, k| {
        if k > 0.68 {
            mix(mix(CYAN, WHITE, 0.25), hot, heat * 0.6)
        } else {
            mix(quiet, hot, heat * 0.85)
        }
    });
}

/// As it converges, each node leaves a fading trail along its way in: the
/// spiral made visible.
fn trails(cv: &mut Canvas, f: &Frame, net: &Net, fade: f64) {
    let u = f.u();
    if u < CONVERGE.0 {
        return;
    }
    let heat = heat(f);
    let mut d = Dots::default();
    for i in 0..net.nodes.len() {
        // every other one is plenty to show the spiral
        let hub = matches!(net.nodes[i].role, Role::Hub(..));
        if net.pull(i, u) <= 0.0 || net.nodes[i].born > u || !hub && i % 2 == 1 {
            continue;
        }
        let mut prev = net.at(f, i, u);
        for s in 1..=10 {
            let p = net.at(f, i, u - 0.0025 * s as f64);
            line(
                &mut d,
                prev,
                p,
                1.0,
                (0.95 - 0.07 * s as f64) * fade * heat.sqrt(),
            );
            prev = p;
        }
    }
    d.draw(cv, |x, y, k| {
        let near = 1.0 - (apart((x as f64, y as f64), (f.cx, f.cy)) / 12.0).min(1.0);
        mix(mix(VIOLET, MAGENTA, heat), CORE[1], near * 0.8 + k * 0.2)
    });
}

/// Attention: arcs leaping between distant nodes as it learns, a bright
/// signal riding each from one end to the other.
fn attention(cv: &mut Canvas, f: &Frame, net: &Net, pos: &[(f64, f64)], fade: f64) {
    let u = f.u();
    if !(LEARN.0..CONVERGE.1).contains(&u) {
        return;
    }
    let n = net.nodes.len();
    let count = (n / 6).clamp(8, 30) as u64;
    let wide = (f.w / 4.0).max(8.0);
    let mut d = Dots::default();
    for k in 0..count {
        let h = hash(f.seed ^ 0xa77e, k);
        // more of them, later
        let start = LEARN.0 + (LEARN.1 - LEARN.0 - 0.04) * unit(h, 1).powf(0.55);
        let life = 0.08 + 0.07 * unit(h, 2);
        let q = (u - start) / life;
        if !(0.0..1.0).contains(&q) {
            continue;
        }
        // two nodes there by then, far apart
        let i = (hash(h, 3) % n as u64) as usize;
        let Some(j) = (4..10)
            .map(|l| (hash(h, l) % n as u64) as usize)
            .find(|&j| j != i && apart(net.nodes[i].home, net.nodes[j].home) > wide)
        else {
            continue;
        };
        if net.nodes[i].born > start || net.nodes[j].born > start {
            continue;
        }
        let (a, b) = (pos[i], pos[j]);
        let bow = if hash(h, 11).is_multiple_of(2) {
            0.3
        } else {
            -0.3
        };
        let reach = smooth(q, 0.0, 0.35);
        let ride = smooth(q, 0.2, 0.85);
        let gone = 1.0 - smooth(q, 0.65, 1.0);
        let steps = (apart(a, b) * 5.2).ceil().max(8.0) as usize;
        for s in 0..=steps {
            let t = reach * s as f64 / steps as f64;
            let (x, y) = arc_at(a, b, bow, t);
            let glow = (-((t - ride) / 0.06).powi(2)).exp();
            d.add(x, y, ((0.6 + 0.4 * glow) * gone * fade).min(1.0));
        }
    }
    // bowed out, an arc stays above the wordmark
    let floor = net.floor as i32;
    d.0.retain(|&(_, y), _| y <= floor);
    d.draw(cv, |_, _, k| {
        mix(MAGENTA, (255, 220, 248), (k - 0.62).max(0.0) * 2.4)
    });
}

/// The nodes: flashing as they're born or the activity reaches them, the
/// hubs bright in their machines' colours, the sessions in their states';
/// gold as they fall in.
fn nodes(cv: &mut Canvas, f: &Frame, net: &Net, pos: &[(f64, f64)], fade: f64) {
    let u = f.u();
    for (i, n) in net.nodes.iter().enumerate() {
        let on = smooth(u, n.born, n.born + 0.02);
        if on <= 0.0 {
            continue;
        }
        let (x, y) = pos[i];
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        if y > net.floor + 0.5 {
            continue;
        }
        let awake = smooth(u, n.wake, n.wake + 0.04);
        // a flash as it's born, and as the activity reaches it
        let flash = window(u, n.born, n.born + 0.006, n.born + 0.012, n.born + 0.04).max(window(
            u,
            n.wake,
            n.wake + 0.006,
            n.wake + 0.015,
            n.wake + 0.05,
        ));
        let heat = net.pull(i, u);
        let rgb = mix(mix(n.rgb, WHITE, flash * 0.8), CORE[1], heat * 0.7);
        match &n.role {
            Role::Hub(_) => {
                let k = on * (0.8 + 0.2 * awake) * fade;
                let mut d = Dots::default();
                d.ring(x, y, 0.85, 0.6, k * 0.85);
                d.ring(x, y, 1.55, 0.6, k * 0.4);
                d.0.remove(&(xi, yi));
                d.draw(cv, |_, _, kk| {
                    mix(mix(rgb, VIOLET, 0.25), WHITE, (kk - 0.6).max(0.0))
                });
                cv.put_with(xi, yi, "✸", mix(rgb, WHITE, 0.2), k, true);
            }
            Role::Sat(..) => {
                let k = on * (0.65 + 0.35 * awake + 0.3 * flash) * fade;
                cv.put(xi, yi, "•", rgb, k.min(1.0));
            }
            Role::Plain => {
                let k = on * (0.55 + 0.3 * awake + 0.4 * flash + 0.2 * heat) * fade;
                let g = if flash > 0.4 || heat > 0.5 {
                    "•"
                } else {
                    "∙"
                };
                cv.put(xi, yi, g, rgb, k.min(1.0));
            }
        }
    }
}

/// The hubs' names, over everything, until they converge.
fn names(cv: &mut Canvas, f: &Frame, net: &Net, pos: &[(f64, f64)], fade: f64) {
    let u = f.u();
    let k = (1.0 - f.act(CONVERGE.0, CONVERGE.0 + 0.08)) * fade;
    if k <= 0.0 {
        return;
    }
    for (i, n) in net.nodes.iter().enumerate() {
        let Role::Hub(Some(name)) = &n.role else {
            continue;
        };
        let on = smooth(u, n.born + 0.02, n.born + 0.06) * k;
        let (xi, yi) = (pos[i].0.round() as i32, pos[i].1.round() as i32);
        let w = name.chars().count() as i32;
        // to its right, or its left where the screen ends first
        let x = if xi + 4 + w < f.w as i32 {
            xi + 4
        } else {
            xi - 3 - w
        };
        cv.clear(x - 1, yi);
        cv.clear(x + w, yi);
        cv.text(x, yi, name, mix(n.rgb, WHITE, 0.15), on);
    }
}

/// The room round the middle (rows): as far as the screen lets things
/// reach from it, up and down.
fn room(f: &Frame) -> f64 {
    (f.cy - 0.5).min(floor(f) - f.cy - 0.5).max(2.0)
}

/// How large the core is drawn: as the screen allows (1 on a 40-row one).
fn scale(f: &Frame) -> f64 {
    (room(f) / 17.5).clamp(0.7, 1.6)
}

/// The core's size (rows) and light now, and its flash: kindling as
/// everything falls in, thrown wide and blinding as it collapses, settling
/// small and steady.
fn core(f: &Frame) -> (f64, f64, f64) {
    let u = f.u();
    let grow = smooth(u, CONVERGE.1 - 0.16, COLLAPSE);
    let flash = window(
        u,
        COLLAPSE - 0.012,
        COLLAPSE,
        COLLAPSE + 0.012,
        COLLAPSE + 0.09,
    );
    let calm = smooth(u, COLLAPSE + 0.02, THINK.0);
    let r = (0.5 + 2.4 * grow) * (1.0 - 0.45 * calm) + 3.2 * flash;
    let light = (grow.sqrt() * (1.0 - 0.15 * calm) + flash).min(1.0);
    (r * scale(f), light, flash)
}

/// The core: a glowing orb of braille, white at its heart, gold, amber at
/// its edge, a soft glow round it; spikes thrown out as it collapses.
fn orb(cv: &mut Canvas, f: &Frame, fade: f64) {
    let (r, light, flash) = core(f);
    if light <= 0.02 {
        return;
    }
    // a small screen's: no larger than the room it has
    let r = r.min(room(f) * 0.8);
    let breath = if f.u() > THINK.0 {
        0.9 + 0.1 * (TAU * (f.u() - THINK.0) / 0.075).cos()
    } else {
        1.0
    };
    let (cx, cy) = (f.cx, f.cy);
    let floor = floor(f) as i32;
    let reach = r * 2.2 + 1.0;
    let (x0, x1) = ((cx - reach * 2.0) as i32 - 1, (cx + reach * 2.0) as i32 + 1);
    let (y0, y1) = (
        (cy - reach) as i32 - 1,
        ((cy + reach) as i32 + 1).min(floor),
    );
    for y in y0..=y1 {
        for x in x0..=x1 {
            let cell = braille_at(x, y, true, |fx, fy| {
                let d = apart((fx, fy), (cx, cy)) / r.max(0.3);
                let body = ((1.0 - d * d).max(0.0).powf(0.5) * 1.5).min(1.0);
                let glow = 0.42 * (-(d - 1.0).max(0.0) * 2.2).exp() * (d > 0.9) as u8 as f64;
                (body + glow) * light * breath
            });
            if let Some((ch, top)) = cell {
                let mut b = [0u8; 4];
                cv.put_with(
                    x,
                    y,
                    ch.encode_utf8(&mut b),
                    ramp(&CORE, 0.3 + 0.7 * (1.0 - top.min(1.0))),
                    (0.35 + 0.65 * top).min(1.0) * fade,
                    top > 0.7,
                );
            }
        }
    }
    if flash > 0.05 {
        for s in 0..12 {
            let a = s as f64 * TAU / 12.0 + PI / 12.0;
            let len = r + 2.0 + 9.0 * flash * if s % 3 == 0 { 1.0 } else { 0.55 };
            let mut j = r.max(1.0);
            while j <= len {
                let (x, y) = (cx + a.cos() * j * 2.0, cy + a.sin() * j * 0.62);
                if y.round() as i32 > floor {
                    break;
                }
                cv.put(
                    x.round() as i32,
                    y.round() as i32,
                    stroke(a.cos() * 2.0, a.sin() * 0.62),
                    mix(CORE[0], CORE[1], j / len),
                    flash * fade * (1.0 - j / (len + 1.0)),
                );
                j += 1.0;
            }
        }
    }
    cv.put_with(cx as i32, cy as i32, "✸", CORE[0], light * fade, true);
}

/// The ring of light going out from the collapse: a white-gold front, a
/// magenta band and a cyan one behind it, fading as it spreads.
fn ring(cv: &mut Canvas, f: &Frame, fade: f64) {
    let q = f.act(RING.0, RING.1);
    if q <= 0.0 || q >= 1.0 {
        return;
    }
    let reach = (f.w / 4.0).hypot(f.h / 2.0) * 1.15;
    let r = reach * (1.0 - (1.0 - q).powi(2));
    let k = (1.0 - q).powf(0.7) * fade;
    let floor = floor(f) as i32;
    let bands = [
        (1.0, 1.0, mix(CORE[0], CORE[1], q)),
        (0.975, 0.85, mix(CORE[1], MAGENTA, 0.3 + 0.7 * q)),
        (0.94, 0.7, MAGENTA),
        (0.9, 0.5, mix(MAGENTA, CYAN, 0.6)),
        (0.84, 0.3, CYAN),
    ];
    for (s, kk, col) in bands {
        let mut d = Dots::default();
        d.ring(f.cx, f.cy, r * s, 1.0, k * kk);
        for ((x, y), (bits, kd)) in d.0 {
            if y <= floor && cv.blank(x, y) {
                let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                let mut b = [0u8; 4];
                cv.put(x, y, ch.encode_utf8(&mut b), col, kd);
            }
        }
    }
}

/// One of the core's thoughts at `u`: where it turns round the core (cells).
fn thought(f: &Frame, i: u64, u: f64) -> (f64, f64) {
    let r = room(f) * (0.38 + 0.4 * unit(f.seed, 0xd00 + i));
    let a = (i as f64 + 0.5 * unit(f.seed, 0xd10 + i)) / THOUGHTS as f64 * TAU
        + u * (1.0 + 1.2 * unit(f.seed, 0xd20 + i));
    (f.cx + a.cos() * r * 2.0, f.cy + a.sin() * r * 0.85)
}

const THOUGHTS: u64 = 7;

/// The core, quietly thinking: slow rings going out from it, a few
/// thoughts turning round it on threads, signals running out to them and
/// back.
fn think(cv: &mut Canvas, f: &Frame, fade: f64) {
    let u = f.u();
    let on = f.act(THINK.0, THINK.0 + 0.06) * fade;
    if on <= 0.0 {
        return;
    }
    // slow pulses going out
    let mut d = Dots::default();
    for p in 0..3 {
        let q = ((u - THINK.0) / 0.075 - p as f64).rem_euclid(3.0) / 3.0;
        d.ring(
            f.cx,
            f.cy,
            (1.8 + q * 8.0) * scale(f),
            0.62,
            on * 0.85 * (1.0 - q).powf(1.3),
        );
    }
    let floor = floor(f) as i32;
    d.0.retain(|&(_, y), _| y <= floor);
    let rings = d;
    // its thoughts, on threads, a signal running along each
    let mut threads = Dots::default();
    let mut dots = Vec::new();
    for i in 0..THOUGHTS {
        let born = THINK.0 + 0.1 * unit(f.seed, 0xd30 + i);
        let k = smooth(u, born, born + 0.04) * on;
        if k <= 0.0 {
            continue;
        }
        let p = thought(f, i, u);
        let c = (f.cx, f.cy);
        let bow = if i.is_multiple_of(2) { 0.22 } else { -0.22 };
        let steps = (apart(c, p) * 5.0).ceil().max(6.0) as usize;
        for s in 0..=steps {
            let (x, y) = arc_at(c, p, bow, s as f64 / steps as f64);
            threads.add(x, y, 0.58 * k);
        }
        let s = (u * (3.0 + 2.0 * unit(f.seed, 0xd40 + i)) + unit(f.seed, 0xd50 + i)).fract();
        let s = if i.is_multiple_of(2) { s } else { 1.0 - s };
        for lag in [0.0, 0.04, 0.08] {
            let (x, y) = arc_at(c, p, bow, (s - lag).clamp(0.0, 1.0));
            threads.add(x, y, 0.95 * k);
        }
        dots.push((p, i, k));
    }
    for ((x, y), (bits, k)) in rings.0 {
        if !threads.0.contains_key(&(x, y)) {
            let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
            let mut b = [0u8; 4];
            cv.put(x, y, ch.encode_utf8(&mut b), mix(VIOLET, CORE[1], k), k);
        }
    }
    threads.draw(cv, |_, _, k| {
        if k > 0.7 {
            mix(CYAN, WHITE, 0.3)
        } else {
            mix(INDIGO, VIOLET, 0.75)
        }
    });
    for ((x, y), i, k) in dots {
        let col = if i.is_multiple_of(3) { MAGENTA } else { CYAN };
        let mut halo = Dots::default();
        halo.ring(x, y, 0.55, 0.6, 0.5 * k);
        halo.0.remove(&(x.round() as i32, y.round() as i32));
        halo.draw(cv, |_, _, _| col);
        cv.put_with(x.round() as i32, y.round() as i32, "•", col, k, true);
    }
}

/// Where its particles are as the ℵ gathers: the core (most of them), and
/// its thoughts.
pub(super) fn sources(f: &Frame) -> Vec<(f64, f64)> {
    let mut v = vec![(f.cx, f.cy); 12];
    v.extend((0..THOUGHTS).map(|i| thought(f, i, 1.0)));
    v
}

/// The held ℵ's nodes (cells) and the edges between them.
type Constellation = (Vec<(i32, i32)>, Vec<(usize, usize)>);

/// The held ℵ's constellation: nodes spread along its strokes (solid cells
/// at least `gap` rows apart, in an order of their own), each joined to its
/// nearest two.
fn constellation(f: &Frame) -> Constellation {
    let (_, _, _, lr, cells) = logo_box(f.w, f.h, f.cx, f.cy);
    let gap = (lr as f64 * 0.2).max(1.5);
    let mut solid: Vec<(u64, (i32, i32))> = cells
        .iter()
        .filter(|c| c.2 == 0xFF)
        .map(|c| (hash(f.seed ^ 0xc057, key(c.0, c.1)), (c.0, c.1)))
        .collect();
    solid.sort_unstable();
    let pt = |p: (i32, i32)| (p.0 as f64, p.1 as f64);
    let mut nodes: Vec<(i32, i32)> = Vec::new();
    for (_, p) in solid {
        if nodes.len() < 28 && nodes.iter().all(|&q| apart(pt(p), pt(q)) >= gap) {
            nodes.push(p);
        }
    }
    let mut edges = Vec::new();
    for (i, &p) in nodes.iter().enumerate() {
        let mut near: Vec<(f64, usize)> = (0..nodes.len())
            .filter(|&j| j != i)
            .map(|j| (apart(pt(p), pt(nodes[j])), j))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for &(_, j) in near.iter().take(2) {
            if !edges.contains(&(j, i)) {
                edges.push((i, j));
            }
        }
    }
    (nodes, edges)
}

fn key(x: i32, y: i32) -> u64 {
    (x as u32 as u64) << 20 | y as u32 as u64
}

/// The ℵ held as a constellation: its nodes lit, signals running between
/// them along its strokes, each cell they pass lit cyan (or magenta) for a
/// moment, the gold showing everywhere else.
pub(super) fn paint(cv: &mut Canvas, f: &Frame) {
    let done = f.len + GATHER_S;
    let end = done + f.opts.hold;
    if f.t < done || f.t >= end {
        return;
    }
    let on = smooth(f.t, done, done + 150.0) * (1.0 - smooth(f.t, end - 120.0, end));
    let (x0, y0, lc, lr, _) = logo_box(f.w, f.h, f.cx, f.cy);
    let (nodes, edges) = constellation(f);
    // the hold's own clock, in shares of the scene
    let tau = (f.t - done) / f.len;
    let mut lit: HashMap<(i32, i32), (f64, Rgb)> = HashMap::new();
    for &p in &nodes {
        lit.insert(p, (0.32, mix(MAGENTA, WHITE, 0.5)));
    }
    for (e, &(a, b)) in edges.iter().enumerate() {
        let k = e as u64;
        let (pa, pb) = (nodes[a], nodes[b]);
        let s = (tau * (5.0 + 4.0 * unit(f.seed, 0xe00 + k)) + unit(f.seed, 0xe80 + k)).fract();
        let col = if k.is_multiple_of(3) { MAGENTA } else { CYAN };
        for (j, lag) in [0.0, 0.12, 0.24].iter().enumerate() {
            let q = s * 1.3 - lag;
            if !(0.0..=1.0).contains(&q) {
                continue;
            }
            let x = (pa.0 as f64 + (pb.0 - pa.0) as f64 * q).round() as i32;
            let y = (pa.1 as f64 + (pb.1 - pa.1) as f64 * q).round() as i32;
            let kk = [0.85, 0.55, 0.3][j];
            let c = if j == 0 { mix(col, WHITE, 0.45) } else { col };
            let e = lit.entry((x, y)).or_insert((0.0, c));
            if kk > e.0 {
                *e = (kk, c);
            }
        }
    }
    for ((x, y), (k, col)) in lit {
        if cv.blank(x, y) {
            continue;
        }
        let sym = cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
            .symbol()
            .to_string();
        let fpos = ((x - x0) as f64 / lc as f64 + (y - y0) as f64 / lr as f64) / 2.0;
        cv.put_with(x, y, &sym, mix(ramp(&GOLD, fpos), col, k * on), 1.0, true);
    }
}

#[cfg(test)]
mod tests {
    use super::super::scene;
    use super::*;

    fn info() -> Info {
        let m = |name: &str, rgb: Rgb, sessions: Vec<Rgb>| Mach {
            name: name.into(),
            rgb,
            up: true,
            sessions,
        };
        Info {
            machines: 2,
            sessions: 5,
            needs: 1,
            fleet: vec![
                m("normandy", (120, 200, 255), vec![(255, 150, 60); 3]),
                m("serenity", (120, 255, 160), vec![(90, 160, 255); 2]),
            ],
        }
    }

    /// A moment of the scene (a share of it) on a dark 120×32 screen.
    fn shot(u: f64, logo: bool, info: &Info) -> (Buffer, Frame<'_>) {
        let f = scene::at(120, 32, (scene::SCENE_MS as f64 * u) as u64, logo, info);
        (scene::dark(Kind::Convergence, &f), f)
    }

    fn rgb(b: &Buffer, x: u16, y: u16) -> Rgb {
        match b[(x, y)].fg {
            Color::Rgb(r, g, bl) => (r, g, bl),
            _ => (0, 0, 0),
        }
    }

    /// Every lit cell above the console.
    fn lit(b: &Buffer, f: &Frame) -> Vec<(u16, u16)> {
        (0..=floor(f) as u16)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&p| b[p].symbol() != " ")
            .collect()
    }

    fn row(b: &Buffer, y: u16) -> String {
        (0..120).map(|x| b[(x, y)].symbol()).collect()
    }

    #[test]
    fn your_machines_are_the_hubs_named_their_sessions_round_them() {
        let info = info();
        let (b, f) = shot(0.25, false, &info);
        let net = Net::of(&f);
        for (h, name) in ["normandy", "serenity"].iter().enumerate() {
            let (x, y) = net.at(&f, h, f.u());
            let (x, y) = (x.round() as u16, y.round() as u16);
            assert_eq!(b[(x, y)].symbol(), "✸", "{name}'s hub");
            assert!(row(&b, y).contains(name), "{name}: {}", row(&b, y));
        }
        // the sessions: in their states' colours, near their hub
        let near = |hub: usize, col: Rgb| {
            net.nodes.iter().enumerate().any(|(i, n)| {
                let Role::Sat(h, _, _) = n.role else {
                    return false;
                };
                let (x, y) = net.at(&f, i, f.u());
                h == hub
                    && n.rgb == col
                    && apart((x, y), net.at(&f, hub, f.u())) < 4.0
                    && b[(x.round() as u16, y.round() as u16)].symbol() == "•"
            })
        };
        assert!(near(0, (255, 150, 60)) && near(1, (90, 160, 255)));
        // a third hub of its own (three at least), unnamed
        assert!(matches!(net.nodes[2].role, Role::Hub(None)));
    }

    #[test]
    fn it_grows_and_its_signals_come_ever_faster() {
        let info = info();
        let born = |u: f64| {
            let (_, f) = shot(u, false, &info);
            let net = Net::of(&f);
            net.nodes.iter().filter(|n| n.born <= u).count()
        };
        assert!(born(0.5) as f64 > born(0.12) as f64 * 1.6);
        let (early, _) = shot(0.12, false, &info);
        let (late, f) = shot(0.45, false, &info);
        assert!(lit(&late, &f).len() > lit(&early, &f).len() * 3 / 2);
        // the pulse rate climbs, in closed form
        let rate = |u: f64| (beats(u + 0.01) - beats(u)) / 0.01;
        assert!(rate(0.6) > rate(0.1) * 4.0, "{} {}", rate(0.1), rate(0.6));
        assert!((0..100).all(|i| beats(i as f64 / 100.0) <= beats((i + 1) as f64 / 100.0)));
    }

    #[test]
    fn attention_leaps_across_it_in_magenta() {
        let info = info();
        let magenta = (15..28).any(|i| {
            let (b, f) = shot(i as f64 * 0.02, false, &info);
            lit(&b, &f)
                .iter()
                .filter(|&&(x, y)| {
                    let (r, g, bl) = rgb(&b, x, y);
                    r > 180 && bl > 150 && (g as i32) < r as i32 - 60
                })
                .count()
                > 15
        });
        assert!(magenta);
    }

    #[test]
    fn its_nodes_converge_on_the_middle() {
        let info = info();
        let spread = |u: f64| {
            let (b, f) = shot(u, false, &info);
            let cells = lit(&b, &f);
            let d: f64 = cells
                .iter()
                .map(|&(x, y)| apart((x as f64, y as f64), (f.cx, f.cy)))
                .sum();
            d / cells.len().max(1) as f64
        };
        let (early, mid, late) = (spread(0.4), spread(0.66), spread(0.74));
        assert!(
            mid < early * 0.8 && late < early * 0.4,
            "{early} {mid} {late}"
        );
        // every node nearer late than early, the spiral turning them
        let (_, f) = shot(0.4, false, &info);
        let net = Net::of(&f);
        let c = (f.cx, f.cy);
        for i in 0..net.nodes.len() {
            let (a, z) = (net.at(&f, i, 0.4), net.at(&f, i, 0.72));
            assert!(apart(z, c) < apart(a, c) * 0.6 + 0.5, "node {i}");
        }
    }

    #[test]
    fn it_collapses_into_one_brilliant_core_and_a_ring_goes_out() {
        let info = info();
        let (b, f) = shot(COLLAPSE, false, &info);
        let light = |p: (u16, u16)| {
            let (r, g, bl) = rgb(&b, p.0, p.1);
            r as u32 + g as u32 + bl as u32
        };
        let (cx, cy) = (f.cx as u16, f.cy as u16);
        assert_eq!(b[(cx, cy)].symbol(), "✸");
        let top = lit(&b, &f).into_iter().map(light).max().unwrap();
        assert_eq!(light((cx, cy)), top, "the core is the brightest");
        // nothing of the network left out past the core and its spikes
        let (r, _, _) = core(&f);
        let far = lit(&b, &f)
            .into_iter()
            .filter(|&(x, y)| apart((x as f64, y as f64), (f.cx, f.cy)) > r + 12.0)
            .count();
        assert_eq!(far, 0);
        // a moment later, the ring: lit cells far out, round the middle
        let (b, f) = shot((RING.0 + RING.1) / 2.0, false, &info);
        let q = f.act(RING.0, RING.1);
        let reach = (f.w / 4.0).hypot(f.h / 2.0) * 1.15 * (1.0 - (1.0 - q).powi(2));
        let on = lit(&b, &f)
            .into_iter()
            .filter(|&(x, y)| (apart((x as f64, y as f64), (f.cx, f.cy)) - reach).abs() < 1.5)
            .count();
        assert!(on > 40, "{on} on the ring");
    }

    #[test]
    fn without_the_logo_it_ends_on_the_core_quietly_thinking() {
        let info = info();
        let (b, f) = shot(0.9999, false, &info);
        assert_eq!(b[(f.cx as u16, f.cy as u16)].symbol(), "✸");
        let room = (f.cy - 0.5).min(floor(&f) - f.cy - 0.5) + 1.5;
        let cells = lit(&b, &f);
        assert!(cells.len() > 30, "{}", cells.len());
        for (x, y) in cells {
            let d = apart((x as f64, y as f64), (f.cx, f.cy));
            assert!(d < room.max(10.5), "({x},{y}) is {d:.1} out");
        }
        // its thoughts, each a node of its own
        let dots = (0..THOUGHTS)
            .filter(|&i| {
                let (x, y) = thought(&f, i, f.u());
                b[(x.round() as u16, y.round() as u16)].symbol() == "•"
            })
            .count();
        assert!(dots >= 5, "{dots} thoughts");
    }

    #[test]
    fn held_the_mark_is_a_constellation_with_signals_running_through_it() {
        let info = info();
        let (nodes, edges) = constellation(&scene::at(120, 32, 0, true, &info));
        assert!(nodes.len() >= 8 && edges.len() >= nodes.len(), "{nodes:?}");
        let ms = (scene::SCENE_MS as f64 + scene::GATHER_S + 300.0) as u64;
        let f = scene::at(120, 32, ms, true, &info);
        let b = scene::dark(Kind::Convergence, &f);
        let (mut lit, mut gold) = (0, 0);
        for y in 0..32 {
            for x in 0..120 {
                if b[(x, y)].symbol() == "█" {
                    let (_, g, bl) = rgb(&b, x, y);
                    if bl as i32 > g as i32 - 20 && bl > 120 {
                        lit += 1;
                    } else {
                        gold += 1;
                    }
                }
            }
        }
        assert!(lit >= 5 && gold > lit * 4, "{lit} lit, {gold} gold");
    }

    #[test]
    fn any_fleet_none_one_or_many_draws_on_any_screen() {
        let many = Info {
            machines: 12,
            sessions: 96,
            needs: 0,
            fleet: (0..12)
                .map(|i| Mach {
                    name: format!("a-machine-with-a-long-name-{i}"),
                    rgb: (200, 120, 255),
                    up: true,
                    sessions: vec![(255, 150, 60); 8],
                })
                .collect(),
        };
        let one = Info {
            fleet: many.fleet[..1].to_vec(),
            ..Info::default()
        };
        for info in [Info::default(), one, many] {
            for (w, h) in [(50, 14), (200, 60)] {
                for u in [0.05, 0.3, 0.6, 0.77, 0.9, 1.0] {
                    let ms = (scene::SCENE_MS as f64 * u) as u64;
                    let f = scene::at(w, h, ms, false, &info);
                    let b = scene::dark(Kind::Convergence, &f);
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
