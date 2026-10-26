//! `pulsar`: a neutron star's lighthouse. It switches on in a flash: a
//! blazing blue-white core with six diffraction spikes, its magnetosphere's
//! dipole loops drawing out round it and turning with its magnetic axis, an
//! equatorial torus with wisps rippling out of it, faint jets along its
//! spin axis, all inside a wind nebula like the Crab's — a blue synchrotron
//! haze in a cage of rose and amber filaments. Its magnetic axis is tilted
//! from its spin axis, so its two conical beams sweep a cone across a deep
//! starfield, a trail of light behind each; once a turn one sweeps across
//! you and the sky pulses with it. Then a magnetar flare: a loop of its
//! field brightens, erupts and is thrown out, sparks race along the field
//! lines, a shockwave rings out and lights the filaments as it passes, and
//! its spin glitches. It spins up to a millisecond pulsar, its beams
//! blurring into a luminous disc, the torus blazing, then settles to a calm
//! sweep. Your machines are stars its beams light as they pass, named for
//! a while after. With the logo, the light along its beams gathers into
//! the ℵ, and the beam goes on sweeping over it.

use super::light::{lit, star, Light, Noise};
use super::scene::{ismooth, Frame, GATHER_S};
use super::*;

type Rgb = (u8, u8, u8);
type V = [f64; 3];

pub(super) const TITLE: &str = "a star keeping time";
/// Its colours on the way into the ℵ: the beams' cyan, then gold.
pub(super) const COLS: [Rgb; 2] = [(140, 235, 255), (255, 214, 120)];

/// The beams' light, faint to hot: deep blue, electric blue, cyan, white.
const BEAM: [Rgb; 4] = [
    (40, 70, 190),
    (50, 150, 255),
    (120, 230, 255),
    (240, 252, 255),
];
/// The magnetar's flare: violet, magenta, hot pink-white.
const FLARE: [Rgb; 3] = [(150, 70, 255), (255, 60, 200), (255, 205, 245)];
/// Its field's loops; the nebula's synchrotron haze (deep to pale) and its
/// filaments (rose, coral, amber); the torus; the star's heart.
const FIELD: Rgb = (120, 140, 255);
const HAZE: [Rgb; 2] = [(50, 90, 230), (150, 205, 255)];
const FIL: [Rgb; 3] = [(255, 60, 150), (255, 100, 110), (255, 172, 80)];
const TORUS: Rgb = (160, 220, 255);
const HOT: Rgb = (236, 248, 255);

/// The acts, as shares of the scene: the flare (the glitch with it); the
/// spin-up; settling to a calm sweep.
const FLARE_AT: f64 = 0.36;
const SPIN: (f64, f64) = (0.55, 0.76);
const SETTLE: (f64, f64) = (0.84, 0.95);

/// Its turns over the scene at the steady rate; the spin-up's top speed and
/// the calm one it settles to (× the steady); the glitch's jump (×) and how
/// soon it relaxes (a share of the scene).
const TURNS: f64 = 6.0;
const TOP: f64 = 40.0;
const CALM: f64 = 0.6;
const JUMP: f64 = 0.8;
const RELAX: f64 = 0.08;

/// Its geometry: the spin axis is this far from your line of sight (β),
/// leaning so on screen; the magnetic axis is this far from the spin axis
/// (α), so one beam brushes past you once a turn.
const BETA: f64 = 0.92;
const ALPHA: f64 = 0.97;
const LEAN: f64 = -PI / 2.0 + 0.42;
/// A beam's half-width (radians); how long its light lingers behind it
/// (ms); the beams' light kept by angle round the star, in this many bins.
const WIDTH: f64 = 0.1;
const LINGER: f64 = 55.0;
const BINS: usize = 360;

/// The spin rate at `u`, × the steady one: the glitch's jump (relaxing),
/// the spin-up (ever faster), the calm it settles to.
fn rate(u: f64) -> f64 {
    let g = if u > FLARE_AT {
        JUMP * (-(u - FLARE_AT) / RELAX).exp()
    } else {
        0.0
    };
    let up = ((u - SPIN.0) / (SPIN.1 - SPIN.0)).clamp(0.0, 1.0).powi(3);
    let down = smooth(u, SETTLE.0, SETTLE.1);
    1.0 + g + (TOP - 1.0) * (up - down) - (1.0 - CALM) * down
}

/// The turns it has made by `u` (`rate`'s integral, in closed form).
fn turns(u: f64) -> f64 {
    let g = if u > FLARE_AT {
        JUMP * RELAX * (1.0 - (-(u - FLARE_AT) / RELAX).exp())
    } else {
        0.0
    };
    let span = SPIN.1 - SPIN.0;
    let q = ((u - SPIN.0) / span).clamp(0.0, 1.0);
    let up = span * q.powi(4) / 4.0 + (u - SPIN.1).max(0.0);
    let down = ismooth(u, SETTLE.0, SETTLE.1);
    TURNS * (u + g + (TOP - 1.0) * (up - down) - (1.0 - CALM) * down)
}

/// The magnetic axis's phase round the spin axis at `u`.
fn angle(seed: u64, u: f64) -> f64 {
    TAU * turns(u) + unit(seed, 11) * TAU
}

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: V) -> V {
    let n = dot(a, a).sqrt().max(1e-9);
    [a[0] / n, a[1] / n, a[2] / n]
}

/// How squarely a beam along `d` points at you (`z` is toward you), 0–1.
fn on_you(d: V) -> f64 {
    let g = d[2].clamp(-1.0, 1.0).acos();
    (-(g / 0.2).powi(2)).exp()
}

/// The beams' light round the star at one moment, by angle on screen: each
/// beam, and the light it leaves as it sweeps (older fainter: a trail; spun
/// up, a disc); how foreshortened it was there.
struct Sweep {
    light: Vec<f32>,
    fore: Vec<f32>,
}

impl Sweep {
    fn at(&self, a: f64) -> (f64, f64) {
        let b = ((a / TAU).rem_euclid(1.0) * BINS as f64) as usize % BINS;
        (self.light[b] as f64, self.fore[b] as f64)
    }
}

/// One moment of the pulsar.
struct Now {
    u: f64,
    /// beam 0's phase (beam 1 points the other way)
    phi: f64,
    rate: f64,
    /// how distinct the beams still are (1; toward 0 as they blur)
    head: f64,
    /// a beam on you, 0–1
    pulse: f64,
    /// the flare's flash, 0–1
    flash: f64,
    /// the scene's light (its fade while the ℵ gathers)
    fade: f64,
    /// the sky's rows: two (one on a short screen) kept clear above the
    /// wordmark
    rows: i32,
    /// the screen's scale (1 at 150 × 40)
    s: f64,
    cx: f64,
    cy: f64,
    /// its spin axis, and two ways square to it (`e1` across the screen)
    spin: V,
    e1: V,
    e2: V,
    sweep: Sweep,
}

impl Now {
    fn of(f: &Frame) -> Now {
        // unclamped: while the ℵ gathers, it goes on turning
        let u = f.t / f.len;
        let r = rate(u.min(1.5));
        let rows = (f.h as i32 - 3 - if f.h >= 20.0 { 2 } else { 1 }).max(1);
        let spin = [BETA.sin() * LEAN.cos(), BETA.sin() * LEAN.sin(), BETA.cos()];
        let e1 = norm(cross([0.0, 0.0, 1.0], spin));
        let e2 = cross(spin, e1);
        let mut n = Now {
            u,
            phi: angle(f.seed, u),
            rate: r,
            head: 1.0 / (1.0 + 0.15 * (r - 1.0).max(0.0)),
            pulse: 0.0,
            flash: window(
                u,
                FLARE_AT - 0.004,
                FLARE_AT + 0.003,
                FLARE_AT + 0.008,
                FLARE_AT + 0.03,
            ),
            fade: f.fade(),
            rows,
            s: (rows as f64 / 35.0).min(f.w / 150.0).clamp(0.45, 1.6),
            cx: f.cx,
            cy: f.cy.min((rows as f64 / 2.0).floor()),
            spin,
            e1,
            e2,
            sweep: Sweep {
                light: Vec::new(),
                fore: Vec::new(),
            },
        };
        let m = n.axis(n.phi);
        n.pulse = on_you(m).max(on_you([-m[0], -m[1], -m[2]])) * n.head;
        n.sweep = n.sweep(f);
        n
    }

    /// The magnetic axis at phase `phi` (beam 0's way).
    fn axis(&self, phi: f64) -> V {
        let (ca, sa) = (ALPHA.cos(), ALPHA.sin());
        let (cp, sp) = (phi.cos(), phi.sin());
        let mut m = [0.0; 3];
        for (i, v) in m.iter_mut().enumerate() {
            *v = ca * self.spin[i] + sa * (cp * self.e1[i] + sp * self.e2[i]);
        }
        m
    }

    /// A point `p` (rows from the star) on screen.
    fn screen(&self, p: V) -> (f64, f64) {
        (self.cx + p[0] * 2.0, self.cy + p[1])
    }

    /// The beams' light by angle: sampled back over the last of their
    /// sweep, 0.05 radian of phase apart — finer where a beam swings past
    /// you, its direction on screen whipping round.
    fn sweep(&self, f: &Frame) -> Sweep {
        let mut light = vec![0f32; BINS];
        let mut fore = vec![1f32; BINS];
        let step = 0.05;
        let per_u = TAU * TURNS * self.rate.max(0.05);
        let du = step / per_u;
        let span = (3.0 * LINGER / f.len).min(TAU * 1.05 / per_u);
        let k = ((span / du).ceil() as usize).clamp(1, 140);
        let dim = 0.6 + 0.4 * self.head;
        let grow = f.act(0.04, 0.14);
        let mut phases: Vec<f64> = Vec::with_capacity(k * 2);
        let on_screen = |p: f64| {
            let m = self.axis(p);
            m[1].atan2(m[0])
        };
        for j in 0..k {
            let p = self.phi - j as f64 * step;
            let turn = ((on_screen(p - step) - on_screen(p) + PI).rem_euclid(TAU) - PI).abs();
            let most = 1 + (11.0 * self.head) as usize;
            let sub = ((turn / 0.07).ceil() as usize).clamp(1, most);
            for i in 0..sub {
                phases.push(p - step * i as f64 / sub as f64);
            }
        }
        for p in phases {
            let ago = (self.phi - p) / per_u * f.len;
            let w = (-ago / LINGER).exp() * dim * grow;
            let m = self.axis(p);
            for b in [1.0, -1.0] {
                let d = [m[0] * b, m[1] * b, m[2] * b];
                let l = d[0].hypot(d[1]).max(0.08);
                let a = d[1].atan2(d[0]);
                let wd = (WIDTH / l).min(0.28);
                let bright = w * (1.0 + 1.5 * on_you(d));
                let c = (a / TAU).rem_euclid(1.0) * BINS as f64;
                let half = (2.0 * wd / TAU * BINS as f64).ceil() as i64;
                let c0 = c.floor() as i64;
                for i in c0 - half..=c0 + half {
                    let q = (i as f64 + 0.5 - c) * TAU / BINS as f64 / (2.0 * wd);
                    if q.abs() >= 1.0 {
                        continue;
                    }
                    let v = (bright * (1.0 - q * q).powi(4)) as f32;
                    let at = i.rem_euclid(BINS as i64) as usize;
                    if v > light[at] {
                        light[at] = v;
                        fore[at] = l as f32;
                    }
                }
            }
        }
        Sweep { light, fore }
    }

    /// The beams' light at `(dx, dy)` (rows from the star): where they are
    /// and have lately been, brightest near the star (but for its field),
    /// fainter along a beam the more it points at you (you look down it).
    fn beam_at(&self, dx: f64, dy: f64) -> f64 {
        let d = dx.hypot(dy);
        if d < 0.3 {
            return 0.0;
        }
        let (lt, fo) = self.sweep.at(dy.atan2(dx));
        if lt < 0.005 {
            return 0.0;
        }
        // (spun up, its light blurred round: foreshortened less)
        let along = d / fo.max(0.1).max(1.0 - self.head);
        lt * (0.15 + 0.85 * (-along / (14.0 * self.s)).exp())
            * smooth(d, 3.0 * self.s, 7.5 * self.s)
    }

    /// The spin-up's energy (0 → 1 → 0): the torus and nebula blaze.
    fn energy(&self) -> f64 {
        smooth(self.u, SPIN.0, SPIN.1) * (1.0 - smooth(self.u, SETTLE.0, SETTLE.1))
    }
}

/// The flare's shockwave: how far it has gone (0 → 1) and its radius
/// (rows).
fn shock(f: &Frame, n: &Now) -> (f64, f64) {
    let q = ((n.u - FLARE_AT) / 0.16).clamp(0.0, 1.0);
    let reach = (f.w / 4.0).hypot(n.rows as f64 / 2.0) * 1.05;
    (q, reach * (1.0 - (1.0 - q).powi(2)))
}

/// How lit the filaments are after the shockwave passed (0 → 1 → 0).
fn after_shock(n: &Now) -> f64 {
    if n.u > FLARE_AT {
        smooth(n.u, FLARE_AT, FLARE_AT + 0.04)
            * (1.0 - smooth(n.u, FLARE_AT + 0.12, FLARE_AT + 0.3))
    } else {
        0.0
    }
}

pub(super) fn draw(cv: &mut Canvas, f: &Frame) {
    let n = Now::of(f);
    if n.fade <= 0.0 {
        return;
    }
    let noise = Noise::new(f.seed);
    let mut l = Light::new(f.w as i32, n.rows);
    dust(&mut l, f, &n);
    haze(&mut l, f, &n, &noise);
    veins(&mut l, f, &n, &noise);
    glow(&mut l, f, &n);
    disc(&mut l, f, &n);
    jets(&mut l, f, &n);
    torus(&mut l, f, &n);
    field(&mut l, f, &n);
    flare(&mut l, f, &n);
    beams(&mut l, f, &n);
    heart(&mut l, f, &n);
    l.scale(n.fade as f32);
    // its light shimmers while it's spun up and as the flare passes
    let boil = if n.head < 0.6 || (FLARE_AT..FLARE_AT + 0.12).contains(&n.u) {
        (f.t / 50.0) as u64
    } else {
        0
    };
    l.draw(cv, boil);
    stars(cv, &l, f, &n);
    core(cv, f, &n);
    machines(cv, f, &n);
}

/// The nebula's half-widths (rows), across and down: growing a little.
fn nebula(f: &Frame, n: &Now) -> (f64, f64) {
    let grow = 0.94 + 0.06 * n.u.min(1.2);
    (f.w / 4.0 * 0.64 * grow, n.rows as f64 / 2.0 * 0.88 * grow)
}

/// The faint stars: a fine dust of points, brightening as a beam passes.
fn dust(l: &mut Light, f: &Frame, n: &Now) {
    let on = f.act(0.0, 0.08);
    if on <= 0.0 {
        return;
    }
    let rows = n.rows as f64;
    let count = ((f.w * rows) / 9.0).clamp(40.0, 1400.0) as u64;
    for i in 0..count {
        let h = hash(f.seed ^ 0xd057, i);
        let (x, y) = (unit(h, 1) * f.w, unit(h, 2) * rows);
        let (dx, dy) = ((x - n.cx) / 2.0, y - n.cy);
        let beam = (n.beam_at(dx, dy) * n.head).min(1.0);
        let tw = 0.7 + 0.3 * (f.t / (130.0 + 170.0 * unit(h, 4)) + i as f64).sin();
        let k = on * (tw * (0.22 + 0.4 * unit(h, 3)) + 1.2 * beam);
        l.point(x, y, lit(mix(class(h), BEAM[3], beam), k));
    }
}

/// The nebula's synchrotron haze: a sparse blue glow, mottled, clear round
/// the star; brighter as a beam meets you, spun up, after the shockwave.
fn haze(l: &mut Light, f: &Frame, n: &Now, noise: &Noise) {
    let neb = f.act(0.03, 0.22);
    if neb <= 0.0 {
        return;
    }
    let t = f.t / 1000.0;
    let (na, nb) = nebula(f, n);
    let s = n.s;
    let boost = 1.0 + 1.0 * n.pulse + 0.4 * n.energy() + 0.3 * after_shock(n);
    let (cx, cy) = (n.cx, n.cy);
    let (x0, x1) = ((cx - na * 2.0) as i32 - 1, (cx + na * 2.0) as i32 + 1);
    let (y0, y1) = ((cy - nb) as i32 - 1, (cy + nb) as i32 + 1);
    l.wash((x0, x1), (y0, y1), |x, y| {
        let (dx, dy) = ((x - cx) / 2.0, y - cy);
        let e = (dx / na).hypot(dy / nb);
        if e >= 1.0 {
            return [0.0; 3];
        }
        let d = dx.hypot(dy);
        let m = noise.fbm(dx / 7.0 + 5.0 + t * 0.05, dy / 4.0, 3);
        let inner = (1.0 - e * e).powf(1.4);
        let clear = smooth(d, 5.0 * s, 11.0 * s);
        let k = neb * inner * clear * (0.04 + 0.22 * smooth(m, 0.35, 0.8)) * boost;
        lit(mix(HAZE[0], HAZE[1], inner), k)
    });
}

/// The nebula's cage of filaments: veins of rose, coral and amber, thickest
/// round its rim, lit up as the shockwave passes and as it spins up.
fn veins(l: &mut Light, f: &Frame, n: &Now, noise: &Noise) {
    let neb = f.act(0.03, 0.22);
    if neb <= 0.0 {
        return;
    }
    let (na, nb) = nebula(f, n);
    let energy = n.energy();
    let (qs, rs) = shock(f, n);
    let shocking = n.u > FLARE_AT && qs < 1.0;
    let after = after_shock(n);
    let (cx, cy) = (n.cx, n.cy);
    let (x0, x1) = (
        (cx - na * 2.0 * 1.12).floor() as i32,
        (cx + na * 2.0 * 1.12).ceil() as i32,
    );
    let (y0, y1) = (
        (cy - nb * 1.12).floor() as i32,
        (cy + nb * 1.12).ceil() as i32,
    );
    l.field((x0, x1), (y0, y1), |x, y| {
        let (dx, dy) = ((x - cx) / 2.0, y - cy);
        let e = (dx / na).hypot(dy / nb);
        if e > 1.1 {
            return [0.0; 3];
        }
        let shell = 0.25 * smooth(1.1 - e, 0.0, 0.3) + (-((e - 0.82) / 0.22).powi(2)).exp();
        if shell < 0.05 {
            return [0.0; 3];
        }
        let r = noise.ridge(dx / 4.0 + 30.0, dy / 2.4 - 10.0, 2);
        let v = ((r - 0.62) / 0.38).max(0.0);
        if v <= 0.0 {
            return [0.0; 3];
        }
        let d = dx.hypot(dy);
        let hit = if shocking {
            (-((d - rs) / 2.5).powi(2)).exp() * 2.0
        } else {
            0.0
        };
        let col = ramp(&FIL, noise.at(dx / 12.0 + 3.0, dy / 7.0 - 9.0));
        let k = neb * shell * v * v * 3.0 * (1.0 + hit + 0.8 * after + 0.6 * energy);
        lit(mix(col, FLARE[2], hit * 0.3), k)
    });
}

/// Flashes of light over the sky: as it switches on, as a beam sweeps
/// across you, as the flare goes off; the shockwave's glow behind its
/// front.
fn glow(l: &mut Light, f: &Frame, n: &Now) {
    let ignite = window(n.u, 0.0, 0.012, 0.02, 0.07);
    let p = n.pulse * n.pulse;
    let (qs, rs) = shock(f, n);
    let shocking = n.u > FLARE_AT && qs < 1.0;
    if ignite <= 0.01 && p <= 0.01 && n.flash <= 0.01 && !shocking {
        return;
    }
    let s = n.s;
    let (cx, cy) = (n.cx, n.cy);
    l.wash((0, f.w as i32 - 1), (0, n.rows - 1), |x, y| {
        let (dx, dy) = ((x - cx) / 2.0, y - cy);
        let d = dx.hypot(dy);
        let mut out = [0f32; 3];
        let mut add = |c: [f32; 3]| {
            out[0] += c[0];
            out[1] += c[1];
            out[2] += c[2];
        };
        if p > 0.01 {
            add(lit(BEAM[3], p * 0.55 / (1.0 + (d / (11.0 * s)).powi(2))));
        }
        if ignite > 0.01 {
            add(lit(HOT, ignite * 2.6 / (1.0 + (d / 3.0).powi(2)).powf(1.5)));
        }
        if n.flash > 0.01 {
            add(lit(
                FLARE[2],
                n.flash * (0.02 + 1.4 / (1.0 + (d / 4.5).powi(2))),
            ));
        }
        if shocking && d < rs {
            let behind = (-(rs - d) / 1.1).exp();
            add(lit(mix(FLARE[1], FLARE[0], qs), (1.0 - qs) * 0.4 * behind));
        }
        out
    });
}

/// Spun up, the beams' light blurred into a luminous disc in its
/// equatorial plane, hot inside, ripples of light running out across it.
fn disc(l: &mut Light, f: &Frame, n: &Now) {
    let blur = 1.0 - n.head;
    if blur < 0.05 {
        return;
    }
    let t = f.t / 1000.0;
    let s = n.s;
    let (cx, cy) = (n.cx, n.cy);
    let rmax = (f.w / 4.0).min(n.rows as f64 / 2.0) * 0.95;
    // in its equatorial plane (the torus's), read back from the screen
    let (e1, e2) = (n.e1, n.e2);
    let det = e1[0] * e2[1] - e1[1] * e2[0];
    let rd = torus_r(n) * 1.25;
    l.wash((0, f.w as i32 - 1), (0, n.rows - 1), |x, y| {
        let (dx, dy) = ((x - cx) / 2.0, y - cy);
        let d = dx.hypot(dy);
        if d > rmax {
            return [0.0; 3];
        }
        // the beams' light blurred into a soft fan all round
        let bk = n.beam_at(dx, dy) * (1.0 - smooth(d, rmax * 0.5, rmax));
        let mut out = lit(ramp(&BEAM, 0.3 + 0.5 * bk), 0.16 * bk * blur);
        // the disc's own glow, hot inside
        if det.abs() > 0.05 {
            let a1 = (dx * e2[1] - dy * e2[0]) / det;
            let a2 = (e1[0] * dy - e1[1] * dx) / det;
            let rho = a1.hypot(a2);
            if rho < rd {
                let q = rho / rd;
                let g = blur * (0.03 + 1.4 * (-rho / (2.0 * s)).exp()) * (1.0 - q * q);
                let c = lit(ramp(&BEAM, 0.95 - 0.7 * q), g);
                out = [out[0] + c[0], out[1] + c[1], out[2] + c[2]];
            }
        }
        out
    });
    // ripples of light running out across it, ring after ring
    for k in 0..5 {
        let q = (t * 1.4 + k as f64 / 5.0).fract();
        let r = rd * (0.12 + 0.88 * q);
        let kk = blur * 2.6 * (1.0 - q).powf(0.6) * smooth(q, 0.0, 0.08);
        if kk < 0.03 {
            continue;
        }
        let steps = ((TAU * r * 8.0) as usize).clamp(24, 3000);
        for i in 0..steps {
            let a = TAU * i as f64 / steps as f64;
            let p = [
                r * (a.cos() * e1[0] + a.sin() * e2[0]),
                r * (a.cos() * e1[1] + a.sin() * e2[1]),
                r * (a.cos() * e1[2] + a.sin() * e2[2]),
            ];
            let near = 0.6 + 0.4 * (p[2] / r);
            let (x, y) = n.screen(p);
            l.point(x, y, lit(ramp(&BEAM, 0.9 - 0.6 * q), kk * near));
        }
    }
}

/// Jets along the spin axis, both ways: faint, knots running out along
/// them.
fn jets(l: &mut Light, f: &Frame, n: &Now) {
    let on = f.act(0.08, 0.22) * (0.7 + 0.8 * n.energy());
    if on <= 0.0 {
        return;
    }
    let t = f.t / 1000.0;
    let len = (n.rows as f64 * 0.42).max(3.0);
    for sign in [1.0, -1.0] {
        let d = [n.spin[0] * sign, n.spin[1] * sign, n.spin[2] * sign];
        let steps = (len * 8.0) as i32;
        for i in 0..steps {
            let s = 1.5 * n.s + i as f64 / 8.0;
            let knot = (TAU * (s / 3.0 - t * 1.6)).cos().max(0.0).powi(6);
            let k = on * (0.35 + 1.1 * knot) * (1.0 - s / (len + 1.5 * n.s)).max(0.0);
            let (x, y) = n.screen([d[0] * s, d[1] * s, d[2] * s]);
            l.point(x, y, lit(mix(BEAM[1], TORUS, knot), k));
        }
    }
}

/// The torus's radius (rows).
fn torus_r(n: &Now) -> f64 {
    9.5 * n.s
}

/// The torus: a bright ring round its equator (the near side brighter),
/// clumped, wisps rippling out of it — quicker and brighter spun up.
fn torus(l: &mut Light, f: &Frame, n: &Now) {
    let on = f.act(0.05, 0.2) * (1.0 + 1.2 * n.energy());
    if on <= 0.0 {
        return;
    }
    let t = f.t / 1000.0;
    let rt = torus_r(n);
    let speed = 0.7 + 1.6 * n.energy();
    for w in 0..5 {
        let (r, k) = match w {
            0 => (rt, 1.6),
            1 => (rt * 1.05, 0.9),
            _ => {
                let q = (t * speed * 0.6 + w as f64 / 3.0).fract();
                (rt * (1.1 + 0.8 * q), 0.9 * (1.0 - q) * smooth(q, 0.0, 0.1))
            }
        };
        let steps = ((TAU * r * 8.0) as usize).clamp(24, 3000);
        for i in 0..steps {
            let a = TAU * i as f64 / steps as f64;
            let p = [
                r * (a.cos() * n.e1[0] + a.sin() * n.e2[0]),
                r * (a.cos() * n.e1[1] + a.sin() * n.e2[1]),
                r * (a.cos() * n.e1[2] + a.sin() * n.e2[2]),
            ];
            let near = 0.55 + 0.45 * (p[2] / r);
            let clump = 0.6 + 0.4 * (a * 7.0 + w as f64 * 2.1 + t * 1.5).sin();
            let (x, y) = n.screen(p);
            l.point(x, y, lit(TORUS, on * k * near * clump));
        }
    }
}

/// A point `th` (radians from the magnetic pole) along a dipole field line
/// `ll` rows out at its widest, toward `w` (square to `m`).
fn line(m: V, w: V, ll: f64, th: f64) -> V {
    let r = ll * th.sin().powi(2);
    let (ct, st) = (th.cos(), th.sin());
    let mut p = [0.0; 3];
    for (i, v) in p.iter_mut().enumerate() {
        *v = r * (ct * m[i] + st * w[i]);
    }
    p
}

/// The way square to the magnetic axis `m` that lies flattest to you: the
/// field's loops drawn toward it (and away) show whole.
fn broadside(n: &Now, m: V) -> V {
    let z = [0.0, 0.0, 1.0];
    let face = [z[0] - m[2] * m[0], z[1] - m[2] * m[1], z[2] - m[2] * m[2]];
    if dot(face, face) < 1e-4 {
        return norm(cross(m, n.spin));
    }
    norm(cross(norm(face), m))
}

/// The field-line shells' widest reach (rows), inside out.
fn shells(n: &Now) -> [f64; 3] {
    [3.0 * n.s, 5.0 * n.s, 7.4 * n.s]
}

/// The magnetosphere: the dipole's loops either side of the magnetic axis,
/// turning with it (behind the star hidden), drawn out as it switches on;
/// smeared as it spins up; lit magenta as the flare comes.
fn field(l: &mut Light, f: &Frame, n: &Now) {
    let on = f.act(0.02, 0.13);
    if on <= 0.0 {
        return;
    }
    let hot = window(
        n.u,
        FLARE_AT - 0.06,
        FLARE_AT,
        FLARE_AT + 0.04,
        FLARE_AT + 0.14,
    );
    // the loop that erupts is gone from its place until the field mends
    let gone = window(
        n.u,
        FLARE_AT - 0.002,
        FLARE_AT,
        FLARE_AT + 0.1,
        FLARE_AT + 0.2,
    );
    let col = mix(FIELD, FLARE[1], hot * 0.8);
    // spun up, a few turns' worth of it at once, fainter
    let smear = if n.head < 0.9 { 3 } else { 1 };
    for j in 0..smear {
        let m = n.axis(n.phi - j as f64 * 0.7);
        let w = broadside(n, m);
        let kk = on * 2.4 * (0.3 + 0.7 * n.head) / smear as f64 * (1.0 + 0.6 * n.pulse + 0.8 * hot);
        for (si, ll) in shells(n).iter().enumerate() {
            let ll = ll * (0.3 + 0.7 * on);
            let steps = ((ll * 16.0) as usize).clamp(16, 240);
            for side in [1.0, -1.0] {
                if j == 0 && si == 1 && side > 0.0 && gone > 0.0 {
                    continue;
                }
                let ws = [w[0] * side, w[1] * side, w[2] * side];
                for i in 0..=steps {
                    let th = 0.12 + (PI - 0.24) * i as f64 / steps as f64;
                    let p = line(m, ws, ll, th);
                    let r = dot(p, p).sqrt();
                    // behind the star: hidden
                    if p[2] < 0.0 && r < 1.3 * n.s {
                        continue;
                    }
                    let k = kk * (1.0 - 0.18 * si as f64);
                    let (x, y) = n.screen(p);
                    l.point(
                        x,
                        y,
                        lit(mix(col, HOT, (1.6 * n.s / r.max(0.1)).min(0.35)), k),
                    );
                }
            }
        }
    }
}

/// The magnetar flare: a loop of the field brightening, erupting (thrown
/// out where it was when it gave, swelling, twisting), sparks racing along
/// the field lines, the shockwave's bright front.
fn flare(l: &mut Light, f: &Frame, n: &Now) {
    let u = n.u;
    if !(FLARE_AT - 0.06..FLARE_AT + 0.26).contains(&u) {
        return;
    }
    let pre = smooth(u, FLARE_AT - 0.06, FLARE_AT);
    let e = smooth(u, FLARE_AT, FLARE_AT + 0.14);
    let k = pre * (1.0 - smooth(u, FLARE_AT + 0.06, FLARE_AT + 0.18));
    let m = n.axis(if u < FLARE_AT {
        n.phi
    } else {
        angle(f.seed, FLARE_AT)
    });
    let w = broadside(n, m);
    let tw = norm(cross(m, w));
    if k > 0.02 {
        let ll = shells(n)[1] * (1.0 + 4.5 * e);
        let steps = ((ll * 14.0) as usize).clamp(24, 600);
        for strand in 0..3 {
            // twisting as it's thrown out
            let tilt = (strand as f64 - 1.0) * 0.1 + e * 0.5;
            let ws = norm([
                w[0] + tw[0] * tilt,
                w[1] + tw[1] * tilt,
                w[2] + tw[2] * tilt,
            ]);
            for i in 0..=steps {
                let th = 0.12 + (PI - 0.24) * i as f64 / steps as f64;
                let (x, y) = n.screen(line(m, ws, ll, th));
                let col = ramp(&FLARE, 0.95 - 0.7 * e);
                l.point(x, y, lit(col, k * (2.6 - 0.6 * strand as f64)));
            }
        }
    }
    // sparks racing along the field lines, pole to pole
    let sp = window(
        u,
        FLARE_AT,
        FLARE_AT + 0.01,
        FLARE_AT + 0.1,
        FLARE_AT + 0.18,
    );
    if sp > 0.02 {
        for i in 0..24u64 {
            let ll = shells(n)[(i % 3) as usize];
            let side = if (i / 3) % 2 == 0 { 1.0 } else { -1.0 };
            let ws = [w[0] * side, w[1] * side, w[2] * side];
            let off = unit(f.seed, 600 + i);
            for (j, lag) in [0.0, 0.03, 0.06, 0.09].iter().enumerate() {
                let q = ((u - FLARE_AT) * 8.0 + off - lag).rem_euclid(1.0);
                let th = 0.12 + (PI - 0.24) * q;
                let (x, y) = n.screen(line(m, ws, ll, th));
                let col = if j == 0 { FLARE[2] } else { FLARE[1] };
                l.point(x, y, lit(col, sp * [3.0, 1.6, 0.8, 0.4][j]));
            }
        }
    }
    // the shockwave's front: a ring racing out
    let (qs, rs) = shock(f, n);
    if u > FLARE_AT && qs < 1.0 && rs > 0.5 {
        let steps = ((TAU * rs * 8.0) as usize).clamp(24, 6000);
        let col = mix(FLARE[2], FLARE[1], 0.55 + 0.45 * qs);
        let k = 2.3 * (1.0 - qs).powf(0.6);
        for i in 0..steps {
            let a = TAU * i as f64 / steps as f64;
            for (dr, kk) in [(0.0, 1.0), (-0.25, 0.7), (-0.5, 0.35)] {
                let r = rs + dr;
                l.point(
                    n.cx + a.cos() * r * 2.0,
                    n.cy + a.sin() * r,
                    lit(col, k * kk),
                );
            }
        }
    }
}

/// How far a beam along screen direction `(ux, uy)` (rows, a unit) runs
/// before it leaves the sky.
fn reach(f: &Frame, n: &Now, ux: f64, uy: f64) -> f64 {
    let (w, rows) = (f.w, n.rows as f64);
    let sx = if ux > 1e-6 {
        (w - 1.0 - n.cx) / (2.0 * ux)
    } else if ux < -1e-6 {
        n.cx / (-2.0 * ux)
    } else {
        f64::MAX
    };
    let sy = if uy > 1e-6 {
        (rows - 1.0 - n.cy) / uy
    } else if uy < -1e-6 {
        n.cy / -uy
    } else {
        f64::MAX
    };
    sx.min(sy).max(0.0)
}

/// The two beams: from the poles out to the sky's edge, a white-hot core in
/// a cone of cyan light widening as it goes, brightest near the star,
/// brighter still as one sweeps toward you; a trail of fainter light
/// behind each. Faint as they blur into the disc.
fn beams(l: &mut Light, f: &Frame, n: &Now) {
    let grow = f.act(0.04, 0.16);
    let k0 = grow * n.head;
    if k0 <= 0.02 {
        return;
    }
    let sc = n.s;
    // the trail: where it was a few moments ago
    let back = TAU * TURNS * n.rate / f.len * (LINGER / 3.0);
    for j in (0..4).rev() {
        let m = n.axis(n.phi - j as f64 * back);
        let kj = k0
            * if j == 0 {
                1.0
            } else {
                0.4 * (-(j as f64) / 2.0).exp()
            };
        for b in [1.0, -1.0] {
            let d = [m[0] * b, m[1] * b, m[2] * b];
            let fl = d[0].hypot(d[1]);
            if fl < 0.05 {
                continue;
            }
            let (ux, uy) = (d[0] / fl, d[1] / fl);
            let len = reach(f, n, ux, uy) * grow;
            let toward = 1.0 + 1.5 * on_you(d);
            let mut s = 1.2 * sc;
            while s < len {
                // its own length (it may point toward you)
                let along = s / fl;
                let i = kj * toward * (0.7 + 1.8 * (-along / (10.0 * sc)).exp());
                let (x, y) = (n.cx + ux * s * 2.0, n.cy + uy * s);
                let col = mix(BEAM[2], BEAM[3], (-along / (7.0 * sc)).exp());
                l.point(x, y, lit(col, i * 1.5));
                if j == 0 {
                    // the cone round it
                    let wd = (0.15 + along * WIDTH).min(0.2 + 0.075 * s);
                    let mut o = -2.0 * wd;
                    while o <= 2.0 * wd {
                        let g = (-(o / wd).powi(2) * 1.4).exp();
                        if o.abs() > 0.12 && g > 0.04 {
                            let c = ramp(&BEAM, 0.45 + 0.4 * g);
                            l.point(x - uy * o * 2.0, y + ux * o, lit(c, i * 0.4 * g));
                        }
                        o += 0.22;
                    }
                }
                s += 0.18;
            }
        }
    }
}

/// The star's light: a blazing core, its corona, six diffraction spikes
/// (and two short ones across); flaring as a beam sweeps across you,
/// magenta-white as it flares, a flash as it switches on.
fn heart(l: &mut Light, f: &Frame, n: &Now) {
    let on = f.act(0.0, 0.03);
    if on <= 0.0 {
        return;
    }
    let ignite = window(n.u, 0.0, 0.012, 0.02, 0.07);
    let hot = n.pulse.max(ignite).max(n.flash);
    let s = n.s;
    let (x, y) = (n.cx, n.cy);
    let col = mix(HOT, FLARE[2], n.flash);
    let corona = mix(BEAM[2], FLARE[1], n.flash);
    l.splat(
        x,
        y,
        (0.75 + 0.35 * hot) * s,
        lit(col, on * (4.5 + 3.0 * hot)),
    );
    l.halo(
        x,
        y,
        (1.1 + 0.9 * hot) * s,
        (4.5 + 8.0 * hot) * s,
        lit(corona, on * (1.0 + 1.4 * hot)),
    );
    let len = (6.0 + 9.0 * hot) * s;
    for i in 0..6 {
        let a = PI / 6.0 + i as f64 * PI / 3.0;
        l.ray(x, y, a, len, lit(col, on * (2.0 + 1.6 * hot)), |q| {
            (1.0 - q).powi(2)
        });
    }
    for a in [0.0, PI] {
        l.ray(x, y, a, len * 0.4, lit(col, on * (1.2 + hot)), |q| {
            (1.0 - q).powi(2)
        });
    }
    // switching on: a ring of light thrown out from it
    let q = n.u / 0.09;
    if q < 1.0 {
        let r = (f.w / 4.0).hypot(n.rows as f64 / 2.0) * (1.0 - (1.0 - q).powi(2));
        let k = 2.4 * (1.0 - q).powf(0.8);
        let steps = ((TAU * r * 8.0) as usize).clamp(16, 6000);
        for i in 0..steps {
            let a = TAU * i as f64 / steps as f64;
            for (dr, kk) in [(0.0, 1.0), (-0.35, 0.45)] {
                let rr = (r + dr).max(0.0);
                l.point(
                    x + a.cos() * rr * 2.0,
                    y + a.sin() * rr,
                    lit(mix(HOT, BEAM[1], q), k * kk),
                );
            }
        }
    }
}

/// The star's heart, crisp over its light.
fn core(cv: &mut Canvas, f: &Frame, n: &Now) {
    let on = f.act(0.0, 0.03) * n.fade;
    if on <= 0.0 {
        return;
    }
    let hot = n.pulse.max(n.flash);
    let g = if hot > 0.4 { "✹" } else { "✸" };
    let col = mix(HOT, FLARE[2], n.flash);
    cv.put_with(n.cx as i32, n.cy as i32, g, col, on, true);
}

/// The brighter stars, as glyphs over the light, lit as a beam passes;
/// never over the nebula's bright heart.
fn stars(cv: &mut Canvas, l: &Light, f: &Frame, n: &Now) {
    let on = f.act(0.0, 0.08) * n.fade;
    if on <= 0.0 {
        return;
    }
    let rows = n.rows as f64;
    let count = ((f.w * rows) / 60.0).clamp(12.0, 260.0) as u64;
    for i in 0..count {
        let h = hash(f.seed ^ 0x57a5, i);
        let (x, y) = (unit(h, 1) * f.w, unit(h, 2) * rows);
        if y > rows - 0.6 {
            continue;
        }
        let (dx, dy) = ((x - n.cx) / 2.0, y - n.cy);
        if dx.hypot(dy) < 9.0 * n.s || l.level(x.round() as i32, y.round() as i32) > 0.5 {
            continue;
        }
        let beam = n.beam_at(dx, dy).min(1.0);
        let b = unit(h, 3).powi(3);
        let tw = 0.8 + 0.2 * (f.t / (110.0 + 150.0 * unit(h, 4)) + i as f64).sin();
        let k = on * (tw * (0.25 + 0.6 * b) + 0.7 * beam);
        star(cv, x, y, mix(class(h), BEAM[3], beam), k);
    }
}

/// Your machines: stars a beam lights as it passes, named for a while
/// after (spun up, lit all the while).
fn machines(cv: &mut Canvas, f: &Frame, n: &Now) {
    let on = f.act(0.06, 0.16) * n.fade;
    let fleet = &f.info.fleet;
    if on <= 0.0 || fleet.is_empty() {
        return;
    }
    let rows = n.rows as f64;
    let m = fleet.len().min(6);
    for (i, mach) in fleet.iter().take(m).enumerate() {
        let a = (i as f64 + 0.5) / m as f64 * TAU + unit(f.seed, 70) * TAU;
        let (rx, ry) = (f.w / 4.0 * 0.86, rows / 2.0 * 0.8);
        let x = (n.cx + a.cos() * rx * 2.0).clamp(1.0, f.w - 2.0);
        let y = (n.cy + a.sin() * ry).clamp(0.0, rows - 1.0);
        let name: String = mach.name.chars().take(14).collect();
        let len = name.chars().count() as i32;
        let (dx, dy) = ((x - n.cx) / 2.0, y - n.cy);
        let since = since_lit(f, n, dy.atan2(dx));
        let glow = (-since / 450.0).exp();
        let beam = n.beam_at(dx, dy).min(1.0);
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        let k = (0.4 + 0.6 * glow.max(beam)) * on;
        cv.put_with(
            xi,
            yi,
            if glow > 0.5 { "✦" } else { "✧" },
            mix(mach.rgb, BEAM[3], beam * 0.5),
            k,
            glow > 0.5,
        );
        if glow > 0.06 {
            let nx = if xi + 2 + len < f.w as i32 {
                xi + 2
            } else {
                xi - 1 - len
            };
            cv.text(nx, yi, &name, mach.rgb, on * glow.min(1.0) * 0.95);
        }
    }
}

/// How long ago (ms) a beam last swept over the screen angle `a` (round
/// the star): stepping back over its last turn.
fn since_lit(f: &Frame, n: &Now, a: f64) -> f64 {
    let step = 0.05;
    let per_ms = TAU * TURNS * n.rate.max(0.05) / f.len;
    for j in 0..140 {
        let m = n.axis(n.phi - j as f64 * step);
        for b in [1.0, -1.0] {
            let (px, py) = (m[0] * b, m[1] * b);
            let fl = px.hypot(py).max(0.08);
            let off = (py.atan2(px) - a + PI).rem_euclid(TAU) - PI;
            if off.abs() < (WIDTH / fl).min(1.2) {
                return j as f64 * step / per_ms;
            }
        }
    }
    f64::MAX
}

/// Where its particles are as the ℵ gathers: along its two beams, and the
/// star.
pub(super) fn sources(f: &Frame) -> Vec<(f64, f64)> {
    let n = Now::of(&Frame {
        t: f.len,
        info: f.info,
        ..*f
    });
    let mut v = vec![(n.cx, n.cy); 4];
    let m = n.axis(n.phi);
    for b in [1.0, -1.0] {
        let (px, py) = (m[0] * b, m[1] * b);
        let fl = px.hypot(py).max(0.05);
        let (ux, uy) = (px / fl, py / fl);
        let len = reach(f, &n, ux, uy);
        for i in 0..30 {
            let s = 1.0 + (len - 1.0).max(0.0) * i as f64 / 29.0;
            v.push((n.cx + ux * s * 2.0, n.cy + uy * s));
        }
    }
    v
}

/// The ℵ held, its beams sweeping on over it like a lighthouse's: the
/// mark's cells lit cyan as one passes (white along it), fading behind it,
/// and the beams themselves faint beyond the mark.
pub(super) fn paint(cv: &mut Canvas, f: &Frame) {
    let done = f.len + GATHER_S;
    let end = done + f.opts.hold;
    if f.t < done || f.t >= end {
        return;
    }
    let on = smooth(f.t, done, done + 120.0) * (1.0 - smooth(f.t, end - 100.0, end));
    let n = Now::of(f);
    let (x0, y0, lc, lr, cells) = logo_box(f.w, f.h, f.cx, f.cy);
    for (x, y, _) in cells {
        if cv.blank(x, y) {
            continue;
        }
        let (dx, dy) = ((x as f64 - n.cx) / 2.0, y as f64 - n.cy);
        let (lt, _) = n.sweep.at(dy.atan2(dx));
        let lit = (lt.min(1.0) * on).max(0.0);
        if lit < 0.08 {
            continue;
        }
        let sym = cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
            .symbol()
            .to_string();
        let fpos = ((x - x0) as f64 / lc as f64 + (y - y0) as f64 / lr as f64) / 2.0;
        let light = mix(BEAM[2], BEAM[3], lit * lit * 0.5);
        cv.put_with(
            x,
            y,
            &sym,
            mix(ramp(&GOLD, fpos), light, lit * 0.9),
            1.0,
            true,
        );
    }
    // the beams beyond the mark, behind it
    let m = n.axis(n.phi);
    for b in [1.0, -1.0] {
        let (px, py) = (m[0] * b, m[1] * b);
        let fl = px.hypot(py);
        if fl < 0.05 {
            continue;
        }
        let (ux, uy) = (px / fl, py / fl);
        let len = reach(f, &n, ux, uy);
        let mut d = Dots::default();
        let mut s = 1.0;
        while s < len {
            d.add(
                n.cx + ux * s * 2.0,
                n.cy + uy * s,
                on * (0.4 + 0.4 * (-s / 14.0).exp()),
            );
            s += 0.15;
        }
        for ((x, y), (bits, k)) in d.0 {
            if cv.blank(x, y) {
                let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                let mut buf = [0u8; 4];
                cv.put(x, y, ch.encode_utf8(&mut buf), ramp(&BEAM, 0.3 + k), k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::scene;
    use super::*;

    fn info() -> Info {
        Info {
            machines: 1,
            sessions: 2,
            needs: 0,
            fleet: vec![Mach {
                name: "normandy".into(),
                rgb: (120, 200, 255),
                up: true,
                sessions: vec![],
            }],
        }
    }

    fn rgb(b: &Buffer, x: u16, y: u16) -> (u8, u8, u8) {
        match b[(x, y)].fg {
            Color::Rgb(r, g, bl) => (r, g, bl),
            _ => (0, 0, 0),
        }
    }

    /// How bright a cell is drawn (its colour's brightest channel, as a
    /// share), 0 if blank.
    fn bright(b: &Buffer, (x, y): (f64, f64)) -> f64 {
        let (x, y) = (x.round() as i32, y.round() as i32);
        if x < 0 || y < 0 || x >= b.area.width as i32 || y >= b.area.height as i32 {
            return 0.0;
        }
        if b[(x as u16, y as u16)].symbol() == " " {
            return 0.0;
        }
        let (r, g, bl) = rgb(b, x as u16, y as u16);
        r.max(g).max(bl) as f64 / 255.0
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
                let b = scene::dark(Kind::Pulsar, &f);
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
    fn its_spin_is_the_integral_of_its_rate() {
        // the beams and their trails both read `turns`: it must be `rate`'s sum
        let mut sum = 0.0;
        let dt = 1e-5;
        for i in 0..120_000 {
            let u = (i as f64 + 0.5) * dt;
            sum += rate(u) * dt;
            if i % 10_000 == 9_999 {
                let u = (i + 1) as f64 * dt;
                assert!((TURNS * sum - turns(u)).abs() < 1e-3, "at {u}");
            }
        }
        // the glitch's jump, the spin-up's top, the calm it ends on
        assert!(rate(FLARE_AT + 0.001) > 1.7 && rate(FLARE_AT - 0.001) < 1.01);
        assert!((rate(0.8) - TOP).abs() < 0.5);
        assert!((rate(1.0) - CALM).abs() < 0.01);
    }

    #[test]
    fn once_a_turn_a_beam_sweeps_across_you() {
        // the magnetic axis brushes your line of sight: some phase of a
        // turn points a beam (nearly) at you, the rest well away
        let info = info();
        let f = scene::at(120, 32, 500, false, &info);
        let n = Now::of(&f);
        let best = (0..360)
            .map(|i| on_you(n.axis(i as f64 * TAU / 360.0)))
            .fold(0.0, f64::max);
        assert!(best > 0.85, "{best}");
        let lit = (0..360)
            .filter(|i| on_you(n.axis(*i as f64 * TAU / 360.0)) > 0.5)
            .count();
        assert!(lit < 60, "{lit} of 360 degrees on you");
    }

    #[test]
    fn a_beam_is_brighter_along_it_than_across_it() {
        let info = info();
        for ms in [450, 610, 790] {
            let f = scene::at(120, 32, ms, false, &info);
            let b = scene::dark(Kind::Pulsar, &f);
            let n = Now::of(&f);
            let m = n.axis(n.phi);
            let (mut along, mut across) = (0.0, 0.0);
            for sign in [1.0, -1.0] {
                let (px, py) = (m[0] * sign, m[1] * sign);
                let fl = px.hypot(py).max(0.05);
                let (ux, uy) = (px / fl, py / fl);
                let len = reach(&f, &n, ux, uy);
                for q in [0.5, 0.62, 0.74, 0.86] {
                    let s = len * q;
                    along += bright(&b, (n.cx + ux * s * 2.0, n.cy + uy * s));
                    // square to it, as far out
                    let (vx, vy) = (-uy, ux);
                    let lv = reach(&f, &n, vx, vy).min(s);
                    across += bright(&b, (n.cx + vx * lv * 2.0, n.cy + vy * lv));
                }
            }
            assert!(
                along > across * 1.6 && along > 3.0,
                "{ms}: {along} along, {across} across"
            );
        }
    }

    #[test]
    fn spun_up_its_beams_blur_into_a_disc_of_light_and_it_settles_calm() {
        let info = info();
        // round the star, past the field's loops: lit by the beams all
        // round when spun up, only near them while it turns steadily
        let ring = |ms: u64| {
            let f = scene::at(120, 32, ms, false, &info);
            let n = Now::of(&f);
            (0..32)
                .filter(|i| {
                    let a = TAU * *i as f64 / 32.0;
                    n.beam_at(a.cos() * 9.0, a.sin() * 9.0) > 0.25
                })
                .count()
        };
        assert!(ring(2400) >= 24, "spun up: {}", ring(2400));
        assert!(ring(750) <= 12, "steady: {}", ring(750));
        assert!(ring(2990) <= 12, "settled: {}", ring(2990));
    }

    #[test]
    fn the_flare_rings_out_in_magenta() {
        let info = info();
        let ms = (scene::SCENE_MS as f64 * (FLARE_AT + 0.05)) as u64;
        let f = scene::at(120, 32, ms, false, &info);
        let b = scene::dark(Kind::Pulsar, &f);
        let n = Now::of(&f);
        let (_, rs) = shock(&f, &n);
        // round its front: magenta
        let hits = (0..48)
            .filter(|i| {
                let a = TAU * *i as f64 / 48.0;
                let (x, y) = (n.cx + a.cos() * rs * 2.0, n.cy + a.sin() * rs);
                let (x, y) = (x.round() as i32, y.round() as i32);
                if x < 0 || y < 0 || x >= 120 || y >= n.rows {
                    return false;
                }
                let (r, g, bl) = rgb(&b, x as u16, y as u16);
                r as i32 > g as i32 + 50 && bl as i32 > g as i32 + 30
            })
            .count();
        assert!(hits >= 12, "{hits}");
    }

    #[test]
    fn nothing_is_drawn_on_the_rows_above_the_console() {
        // the rows between the sky and the wordmark stay clear, the whole
        // way through (no trace, no readout)
        let info = info();
        for (w, h) in [(120u16, 32u16), (80, 20), (60, 16), (50, 14)] {
            let clear: Vec<u16> = if h >= 20 {
                vec![h - 5, h - 4]
            } else {
                vec![h - 4]
            };
            for ms in (0..scene::SCENE_MS).step_by(40) {
                let f = scene::at(w, h, ms, false, &info);
                let b = scene::dark(Kind::Pulsar, &f);
                for &y in &clear {
                    let row: String = (0..w).map(|x| b[(x, y)].symbol()).collect();
                    assert!(row.trim().is_empty(), "{w}×{h} at {ms}: row {y}: {row:?}");
                }
            }
        }
    }

    #[test]
    fn your_machine_is_named_as_a_beam_lights_it() {
        let info = info();
        // spun up, the beams light it all the while
        let f = scene::at(120, 32, 2400, false, &info);
        let b = scene::dark(Kind::Pulsar, &f);
        let all: String = (0..32)
            .map(|y| (0..120).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect();
        assert!(all.contains("normandy"));
    }

    #[test]
    fn held_the_mark_is_lit_by_the_beam_like_a_lighthouse() {
        let info = info();
        let ms = (scene::SCENE_MS as f64 + scene::GATHER_S + 300.0) as u64;
        let f = scene::at(120, 32, ms, true, &info);
        let b = scene::dark(Kind::Pulsar, &f);
        let (mut cyan, mut gold) = (0, 0);
        for y in 0..32 {
            for x in 0..120 {
                if b[(x, y)].symbol() == "█" {
                    let (r, _, bl) = rgb(&b, x, y);
                    if bl > r {
                        cyan += 1;
                    } else {
                        gold += 1;
                    }
                }
            }
        }
        assert!(cyan >= 3 && gold > cyan * 2, "{cyan} lit, {gold} gold");
    }
}
