//! `event-horizon`: a black hole, close. First only its pull: the stars
//! behind it bent aside, arcs, a far galaxy drawn into an Einstein ring.
//! Then the hole itself: its shadow, edged by the thin photon ring; its
//! disc near edge-on, the far side lensed up over the shadow and under it,
//! brighter and bluer on the side coming toward you, dimmer and redder going
//! away, its turbulence turning; faint jets from the poles. A star falls in:
//! stretched into a stream (spaghettified) that wraps the disc, it flares
//! where it hits. Then you cross the horizon: everything slows and reddens
//! as the shadow fills the screen, the photon ring the last light. Your
//! machines are stars in the lensed sky, named. With the logo, Hawking
//! sparks rise from the ring into the ℵ.

use super::scene::{ismooth, Frame};
use super::*;

type Rgb = (u8, u8, u8);

pub(super) const TITLE: &str = "the point of no return";
/// Its colours on the way into the ℵ: the ring's amber, then gold.
pub(super) const COLS: [Rgb; 2] = [(255, 176, 96), (255, 214, 120)];

/// The disc's light, hottest inside: white, pale gold, amber, orange, ember.
const HEAT: [Rgb; 5] = [
    (255, 250, 236),
    (255, 224, 150),
    (255, 178, 80),
    (232, 112, 44),
    (150, 46, 30),
];
/// Doppler: blue-shifted coming toward you, red-shifted going away.
const BLUE: Rgb = (170, 200, 255);
const RED: Rgb = (190, 48, 34);
/// The photon ring, and what it reddens to as you fall.
const RING: Rgb = (255, 242, 214);
const LAST: Rgb = (255, 120, 64);
/// What everything else reddens to.
const EMBER: Rgb = (140, 30, 24);
const JET: Rgb = (130, 150, 255);
const GALAXY: [Rgb; 2] = [(170, 195, 255), (215, 180, 255)];

/// The acts, as shares of the scene.
const LENS: (f64, f64) = (0.0, 0.2);
const SHADOW: (f64, f64) = (0.12, 0.26);
const IGNITE: (f64, f64) = (0.18, 0.38);
const JETS: (f64, f64) = (0.3, 0.46);
/// The star: from far off (start) to its last (end); its stream hits the
/// disc (`HIT`).
const FALL: (f64, f64) = (0.36, 0.66);
const HIT: f64 = 0.59;
const CROSS: (f64, f64) = (0.78, 1.0);

/// The disc's tilt: a circle in its plane is this much as tall as wide.
const FLAT: f64 = 0.14;
/// Its inner and outer edge, × the shadow's radius.
const RIN: f64 = 1.45;
const ROUT: f64 = 4.0;
/// The star's way in: where it starts round the hole, how far it swings
/// (under the shadow, to the disc's bright side), and how tilted its orbit
/// is until it settles into the disc.
const PHI0: f64 = -0.35 * PI;
const SWING: f64 = 1.35 * PI;
const ORBIT: f64 = 0.45;

/// A braille cell's dots: [column][row].
const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// The shadow's radius before you fall in (rows).
fn rs0(f: &Frame) -> f64 {
    (f.h * 0.13).min(f.w * 0.065).clamp(2.0, 8.0)
}

/// The shadow's radius as you cross: the photon ring just inside the screen.
fn rs_last(f: &Frame) -> f64 {
    ((f.cy - 0.6).min(f.w / 4.0 - 1.0) / 1.03).max(rs0(f))
}

/// One moment of the hole.
struct Hole {
    u: f64,
    /// the shadow's radius (rows), growing to fill the screen as you fall
    rs: f64,
    /// how far the shadow is there, 0–1
    shadow: f64,
    /// the Einstein radius (rows)
    lens: f64,
    /// the disc lit, its far side lensed round the shadow, 0–1
    disc: f64,
    halo: f64,
    /// the stream hitting the disc: its flare, 0–1
    flare: f64,
    /// how red everything has gone, falling in, 0–1
    red: f64,
    /// the hole's own clock (shares of the scene), slowing as you fall
    tau: f64,
    fade: f64,
}

impl Hole {
    fn of(f: &Frame) -> Hole {
        let u = f.u();
        let r0 = rs0(f);
        let rs = r0 * (rs_last(f) / r0).powf(f.act(CROSS.0, CROSS.1));
        let gone = 1.0 - smooth(u, CROSS.0 + 0.01, CROSS.0 + 0.13);
        Hole {
            u,
            rs,
            shadow: f.act(SHADOW.0, SHADOW.1),
            lens: rs * 2.4 * f.act(LENS.0, LENS.1),
            disc: f.act(IGNITE.0, IGNITE.1) * gone,
            halo: f.act(IGNITE.0 + 0.05, IGNITE.1 + 0.04) * gone,
            flare: window(u, HIT, HIT + 0.012, HIT + 0.05, HIT + 0.18),
            red: f.act(CROSS.0 + 0.02, CROSS.1),
            tau: u - 0.94 * ismooth(u, CROSS.0, CROSS.1),
            fade: f.fade(),
        }
    }
}

pub(super) fn draw(cv: &mut Canvas, f: &Frame) {
    let h = Hole::of(f);
    if h.fade <= 0.0 {
        return;
    }
    sky(cv, f, &h);
    galaxies(cv, f, &h);
    named(cv, f, &h);
    jets(cv, f, &h);
    hole(cv, f, &h);
    stream(cv, f, &h);
    impact(cv, f, &h);
}

/// Where a point at `(u, v)` (rows from the hole) is seen: its two images,
/// one pushed out and one pulled in across (the lens equation), each with
/// how much it's brightened.
fn images(u: f64, v: f64, re: f64) -> [(f64, f64, f64); 2] {
    let b = u.hypot(v).max(1e-6);
    if re < 1e-3 {
        return [(u, v, 1.0), (0.0, 0.0, 0.0)];
    }
    let root = (b * b + 4.0 * re * re).sqrt();
    let (tp, tm) = ((b + root) / 2.0, (b - root) / 2.0);
    let mag = |t: f64| (1.0 / (1.0 - (re / t).powi(4))).abs().min(6.0);
    [
        (u * tp / b, v * tp / b, mag(tp)),
        (u * tm / b, v * tm / b, mag(tm)),
    ]
}

/// How much of an image at `(u, v)` shows past the shadow.
fn shows(h: &Hole, u: f64, v: f64) -> f64 {
    if u.hypot(v) < h.rs * 1.05 {
        1.0 - h.shadow
    } else {
        1.0
    }
}

/// The sky behind: stars drifting slowly past, bent round the hole (two
/// images each, piling up round the Einstein ring, the middle emptied).
fn sky(cv: &mut Canvas, f: &Frame, h: &Hole) {
    let on = f.act(0.0, 0.06) * h.fade * (1.0 - h.red).powf(1.5);
    if on <= 0.02 {
        return;
    }
    let n = ((f.w * f.h) / 9.0).clamp(80.0, 1400.0) as u64;
    let drift = h.tau * f.w * 0.06;
    let mut faint = Dots::default();
    for i in 0..n {
        let x = (unit(f.seed, 2000 + i) * f.w - drift).rem_euclid(f.w);
        let y = unit(f.seed, 3000 + i) * (f.h - 3.0);
        let (u, v) = ((x - f.cx) / 2.0, y - f.cy);
        let bright = unit(f.seed, 4000 + i).powi(3);
        let tw = 0.8 + 0.2 * (h.tau * 40.0 * (1.0 + unit(f.seed, 5000 + i)) + i as f64).sin();
        let col = mix(class(hash(f.seed, i)), EMBER, h.red);
        for (iu, iv, mag) in images(u, v, h.lens) {
            if mag < 0.12 {
                continue;
            }
            let k = (0.22 + 0.78 * bright) * mag.sqrt().min(2.0) * shows(h, iu, iv) * on * tw;
            let (sx, sy) = (f.cx + iu * 2.0, f.cy + iv);
            if sy > f.h - 4.0 {
                continue;
            }
            if bright > 0.2 || k > 0.62 {
                let g = match k {
                    k if k > 0.85 => "✦",
                    k if k > 0.6 => "•",
                    k if k > 0.38 => "∙",
                    _ => "·",
                };
                cv.put(sx.round() as i32, sy.round() as i32, g, col, k.min(1.0));
            } else {
                faint.add(sx, sy, k.min(1.0));
            }
        }
    }
    let tint = mix((200, 210, 240), EMBER, h.red);
    for ((x, y), (bits, k)) in faint.0 {
        if cv.blank(x, y) {
            let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
            let mut b = [0u8; 4];
            cv.put(x, y, ch.encode_utf8(&mut b), tint, k * 0.8);
        }
    }
}

/// Two far galaxies: one drifting right behind the hole, its light bent
/// into arcs and then all the way round, an Einstein ring; one off to the
/// side, an arc and its faint counter-image.
fn galaxies(cv: &mut Canvas, f: &Frame, h: &Hole) {
    let on = f.act(0.02, 0.12) * h.fade * (1.0 - h.red).powi(4) * (1.0 - 0.5 * h.disc);
    if on <= 0.02 {
        return;
    }
    let full = rs0(f) * 2.4;
    // the sky's drift (rows): the first is behind the hole a fifth of the way in
    let drift = h.tau * f.w * 0.03;
    let places = [
        (0.22 * f.w * 0.03 - drift, 0.35, 1.0, 320u64),
        (-full * 0.95 - drift, -full * 0.42, 0.8, 160),
    ];
    for (g, (gu, gv, size, n)) in places.into_iter().enumerate() {
        let mut d = Dots::default();
        for i in 0..n {
            let lane = 6000 + g as u64 * 1000 + i;
            let a = unit(f.seed, lane) * TAU;
            let rr = size * unit(f.seed, lane + 500).sqrt();
            let (pu, pv) = (gu + a.cos() * rr * 1.3, gv + a.sin() * rr * 0.7);
            let core = 1.0 - rr / size;
            for (iu, iv, mag) in images(pu, pv, h.lens) {
                if mag < 0.08 {
                    continue;
                }
                let k = (0.3 + 0.45 * core) * mag.sqrt().min(2.2) * on * shows(h, iu, iv);
                d.add(f.cx + iu * 2.0, f.cy + iv, k.min(1.0));
            }
        }
        let col = mix(GALAXY[g], EMBER, h.red);
        d.draw(cv, |_, _, k| mix(col, (240, 244, 255), k * 0.5));
    }
}

/// Your machines: stars in the lensed sky, each named (until the star
/// falls in).
fn named(cv: &mut Canvas, f: &Frame, h: &Hole) {
    let on = f.act(0.06, 0.16) * h.fade * (1.0 - h.red).powf(1.5);
    let fleet = &f.info.fleet;
    if on <= 0.02 || fleet.is_empty() {
        return;
    }
    let names = 1.0 - f.act(0.44, 0.54);
    let m = fleet.len().min(6);
    let full = rs0(f) * 2.4;
    for (i, mach) in fleet.iter().take(m).enumerate() {
        let a = (i as f64 + 0.5) / m as f64 * TAU + unit(f.seed, 90) * TAU;
        let b = full * (1.2 + 0.45 * unit(f.seed, 91 + i as u64));
        let [(iu, iv, _), _] = images(a.cos() * b, a.sin() * b * 0.7, h.lens);
        let name: String = mach.name.chars().take(14).collect();
        let len = name.chars().count() as f64;
        let x = (f.cx + iu * 2.0).clamp(1.0, f.w - 2.0);
        let y = (f.cy + iv).clamp(0.0, f.h - 5.0);
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        cv.put_with(xi, yi, "✦", mach.rgb, on, true);
        if names > 0.0 {
            // its name to the right, or the left where the screen ends
            let nx = if x + 2.0 + len < f.w {
                xi + 2
            } else {
                xi - 1 - len as i32
            };
            cv.text(nx, yi, &name, mach.rgb, on * names * 0.8);
        }
    }
}

/// Faint jets from the poles, knots running out along them.
fn jets(cv: &mut Canvas, f: &Frame, h: &Hole) {
    let on = f.act(JETS.0, JETS.1) * h.fade * (1.0 - smooth(h.u, CROSS.0, CROSS.0 + 0.1));
    if on <= 0.02 {
        return;
    }
    let mut d = Dots::default();
    for sign in [-1.0, 1.0] {
        let base = h.rs * if sign < 0.0 { 2.05 } else { 1.5 };
        let edge = if sign < 0.0 { f.cy } else { f.h - 4.5 - f.cy };
        let len = edge - base;
        let mut dd = 0.0;
        while dd < len {
            let y = f.cy + sign * (base + dd);
            let knot = (TAU * (dd / 3.5 - h.tau * 9.0)).cos().max(0.0).powi(4);
            let fall = (-dd / (len * 0.7 + 1.0)).exp();
            let k = on * (0.28 + 0.4 * knot) * (0.4 + 0.6 * fall);
            let wid = 0.1 + dd * 0.022;
            d.add(f.cx, y, k);
            d.add(f.cx - wid * 2.0, y, k * 0.55);
            d.add(f.cx + wid * 2.0, y, k * 0.55);
            dd += 0.25;
        }
    }
    d.draw(cv, |_, _, k| mix(JET, (225, 232, 255), k));
}

/// How bright the disc is at radius `re` (rows, in its plane), 0–1 across
/// it: hottest inside, a sharp inner edge, a soft outer one.
fn profile(h: &Hole, re: f64) -> f64 {
    let x = ((re - h.rs * RIN) / (h.rs * (ROUT - RIN))).clamp(0.0, 1.0);
    (1.0 - x).powf(0.9) * smooth(x, -0.02, 0.03)
}

/// The disc's light at radius `re` and angle `phi` (0 to your right, π/2
/// nearest you), but for its profile: its turbulence turning (quicker
/// inside), brighter and bluer where it comes toward you, dimmer and redder
/// going away, hotter inside. A gain and a colour.
fn shade(h: &Hole, re: f64, phi: f64) -> (f64, Rgb) {
    let rin = h.rs * RIN;
    let re = re.max(rin);
    let x = ((re - rin) / (h.rs * (ROUT - RIN))).clamp(0.0, 1.0);
    let lr = (re / rin).ln();
    // it turns so its left side comes toward you
    let psi = phi + 16.0 * (rin / re).powf(1.5) * h.tau;
    let n = 0.5
        + 0.22 * (4.0 * psi + 7.0 * lr + 1.0).sin()
        + 0.16 * (9.0 * psi - 13.0 * lr + 2.3).sin()
        + 0.12 * (31.0 * x + 2.0 * (2.0 * psi).sin()).sin();
    let tex = 0.45 + 0.55 * n;
    let beta = -phi.cos() * 0.5 * (rin / re).sqrt();
    let boost = (1.0 + beta).powf(2.4).clamp(0.3, 2.0);
    let heat = ramp(&HEAT, 0.16 + 0.8 * x);
    let col = if beta > 0.0 {
        mix(heat, BLUE, beta * 1.6)
    } else {
        mix(heat, RED, -beta * 1.3)
    };
    (tex * boost * 1.35 * (1.0 + 0.5 * h.flare), col)
}

/// The band the disc's far side is seen in, lensed over the shadow (`v` <
/// 0) or under it: its inner and outer radius (rows), and how bright.
fn band(h: &Hole, v: f64) -> (f64, f64, f64) {
    let span = h.rs * (ROUT - RIN);
    if v < 0.0 {
        (h.rs * 1.1, h.rs * 1.1 + 0.33 * span, 0.95)
    } else {
        (h.rs * 1.05, h.rs * 1.05 + 0.1 * span, 0.5)
    }
}

/// A cell's shading, worked out once at its middle (its dots share it): the
/// disc's there, and its lensed image's.
struct Here {
    disc: (f64, Rgb),
    halo: (f64, Rgb),
}

impl Here {
    fn at(h: &Hole, u: f64, v: f64) -> Here {
        if h.disc <= 0.04 {
            return Here {
                disc: (0.0, RING),
                halo: (0.0, RING),
            };
        }
        let (rin, rout) = (h.rs * RIN, h.rs * ROUT);
        let (r1, r2, _) = band(h, v);
        let x = ((u.hypot(v) - r1) / (r2 - r1)).clamp(0.0, 1.0);
        Here {
            disc: shade(h, u.hypot(v / FLAT), (v / FLAT).atan2(u)),
            halo: shade(h, rin + x * (rout - rin), v.atan2(u)),
        }
    }
}

/// The light at `(u, v)` (rows from the centre), its cell's shading `here`:
/// brightness, colour, and whether it's the sharp photon ring. The ring
/// hugging the shadow; the disc (its near side across the shadow, its far
/// side hidden behind it); the far side lensed over the shadow, and a
/// thinner image under it.
fn light(h: &Hole, here: &Here, u: f64, v: f64) -> (f64, Rgb, bool) {
    let r = u.hypot(v);
    let rs = h.rs;
    let mut best = (0.0, RING, false);
    if h.shadow > 0.0 {
        let d = (r - rs * 1.03) / 0.17;
        let left = 1.0 - 0.35 * (u / r.max(0.1)) * (1.0 - h.red);
        let k = ((-d * d).exp() + 0.25 * (-(d / 4.0).powi(2)).exp())
            * h.shadow
            * left
            * (1.0 + 0.4 * h.flare);
        best = (k.min(1.0), mix(RING, LAST, h.red * 0.8), true);
    }
    if h.disc > 0.04 {
        let (rin, rout) = (rs * RIN, rs * ROUT);
        let re = u.hypot(v / FLAT);
        if (rin..=rout).contains(&re) && (v >= 0.0 || r > rs * 1.02) {
            let k = profile(h, re) * here.disc.0 * h.disc;
            if k > best.0 {
                best = (k, here.disc.1, false);
            }
        }
        // the far side, seen over the top of the shadow and under it
        let (r1, r2, w) = band(h, v);
        if h.halo > 0.04 && r > r1 && r < r2 {
            let re = rin + (r - r1) / (r2 - r1) * (rout - rin);
            let k = profile(h, re) * here.halo.0 * w * (v / r).abs().powf(0.6) * h.halo;
            if k > best.0 {
                best = (k, here.halo.1, false);
            }
        }
    }
    let (k, c, sharp) = best;
    let c = if sharp { c } else { mix(c, EMBER, h.red * 0.8) };
    (k, c, sharp)
}

/// A braille cell lit from `f` (each dot's brightness, colour, and whether
/// it's a sharp line): each dot against a threshold of its own, hashed from
/// where it is and `tick` (light thinning out as a grain, never a grid);
/// sharp lines at a fixed one. The glyph, the brightest lit dot and its
/// colour.
fn cell8(
    x: i32,
    y: i32,
    tick: u64,
    f: impl Fn(f64, f64) -> (f64, Rgb, bool),
) -> Option<(char, f64, Rgb)> {
    let h = hash(tick, (x as u32 as u64) << 20 | y as u32 as u64);
    let (mut bits, mut top, mut col) = (0u32, 0.0f64, RING);
    for (i, column) in BITS.iter().enumerate() {
        for (j, bit) in column.iter().enumerate() {
            let (k, c, sharp) = f(
                x as f64 - 0.25 + i as f64 * 0.5,
                y as f64 - 0.375 + j as f64 * 0.25,
            );
            let th = if sharp {
                0.35
            } else {
                0.03 + 0.94 * unit(h, (i * 4 + j) as u64)
            };
            if k > th {
                bits |= bit;
                if k > top {
                    top = k;
                    col = c;
                }
            }
        }
    }
    (bits != 0).then(|| (char::from_u32(0x2800 + bits).unwrap_or(' '), top, col))
}

/// The hole: every cell near its light, drawn from `light`. Only the rows
/// and spans where there is any (the disc's rows whole, round the shadow a
/// ring), so it stays quick however big the shadow grows.
fn hole(cv: &mut Canvas, f: &Frame, h: &Hole) {
    if h.shadow <= 0.0 && h.disc <= 0.0 {
        return;
    }
    let rs = h.rs;
    let (rin, rout) = (rs * RIN, rs * ROUT);
    let ring = rs * 1.03 + 0.9;
    let halo = rs * 1.1 + 0.33 * (rout - rin);
    let r_hi = ring + (halo - ring).max(0.0) * (h.halo > 0.04) as u8 as f64;
    let r_lo = rs * 0.95;
    let dv = if h.disc > 0.04 {
        rout * FLAT + 0.8
    } else {
        -1.0
    };
    let k0 = h.fade;
    // its grain boils, slower as time slows
    let tick = (h.tau * 40.0) as u64 ^ f.seed;
    let top = ((f.cy - r_hi).floor() as i32 - 1).max(0);
    let bottom = ((f.cy + r_hi).ceil() as i32 + 1).min(f.h as i32 - 4);
    for y in top..=bottom {
        let v = y as f64 - f.cy;
        let mut spans = Vec::with_capacity(2);
        if v.abs() <= dv {
            spans.push((-rout - 0.6, rout + 0.6));
        } else if r_hi + 0.5 > v.abs() {
            let a = ((r_lo * r_lo - v * v).max(0.0).sqrt() - 0.6).max(0.0);
            let b = (r_hi * r_hi - v * v).max(0.0).sqrt() + 0.6;
            spans.push((-b, -a));
            spans.push((a, b));
        }
        for (u0, u1) in spans {
            let xa = ((f.cx + u0 * 2.0).floor() as i32).max(0);
            let xb = ((f.cx + u1 * 2.0).ceil() as i32).min(f.w as i32 - 1);
            for x in xa..=xb {
                let here = Here::at(h, (x as f64 - f.cx) / 2.0, y as f64 - f.cy);
                let cell = cell8(x, y, tick, |fx, fy| {
                    light(h, &here, (fx - f.cx) / 2.0, fy - f.cy)
                });
                if let Some((ch, top, col)) = cell {
                    let mut b = [0u8; 4];
                    cv.put(
                        x,
                        y,
                        ch.encode_utf8(&mut b),
                        col,
                        (0.3 + 0.7 * top.min(1.0)) * k0,
                    );
                }
            }
        }
    }
}

/// The star's way in, `s` along it (0 → 1, then on round the disc's inner
/// edge): on an orbit tilted to the disc, from the upper right, down and
/// round beneath the shadow, settling into the disc's plane as it hits its
/// bright side. Where that is (rows from the centre), and whether it's
/// behind the hole there.
fn path(f: &Frame, h: &Hole, s: f64) -> (f64, f64, bool) {
    let rend = h.rs * RIN * 1.06;
    let rfar = ((f.w / 4.0 - 2.0) * 0.95).max(rend);
    let (re, phi, flat) = if s <= 1.0 {
        let s = s.max(0.0);
        (
            rend + (rfar - rend) * (1.0 - s).powf(1.5),
            PHI0 + SWING * s.powf(1.2),
            FLAT + (ORBIT - FLAT) * (1.0 - smooth(s, 0.72, 1.0)),
        )
    } else {
        (rend, PHI0 + SWING + (s - 1.0) * 1.4 * PI, FLAT)
    };
    (re * phi.cos(), re * phi.sin() * flat, phi.sin() < 0.0)
}

/// The falling star: whole as it comes (a bright point), then stretched
/// along its way into a stream (spaghettified), its head going ahead round
/// the disc while its tail follows; it fades into the disc.
fn stream(cv: &mut Canvas, f: &Frame, h: &Hole) {
    let u = h.u;
    if !(FALL.0..FALL.1 + 0.12).contains(&u) {
        return;
    }
    let on = smooth(u, FALL.0, FALL.0 + 0.03) * (1.0 - smooth(u, FALL.1, FALL.1 + 0.1)) * h.fade;
    let q = ((u - FALL.0) / (FALL.1 - FALL.0)).clamp(0.0, 1.0);
    let at = 1.15 * q.powf(1.5);
    let spread = 0.5 * smooth(u, 0.5, 0.64);
    let n = 220u64;
    let mut d = Dots::default();
    for i in 0..n {
        let o = i as f64 / (n - 1) as f64;
        let s = at + spread * (o - 0.35);
        let (pu, pv, behind) = path(f, h, s);
        if behind && pu.hypot(pv) < h.rs * 1.03 {
            continue;
        }
        // brightest at its heart, fading into the disc once round it
        let heart = 1.0 - ((o - 0.35) / 0.65).abs();
        let gone = 1.0 - ((s - 1.0) / 0.5).clamp(0.0, 1.0);
        let k = on * (0.6 + 0.4 * heart) * gone;
        d.add(f.cx + pu * 2.0, f.cy + pv, k);
        // a stream, not a thread: a little glow either side of it
        if heart > 0.3 {
            d.add(f.cx + pu * 2.0, f.cy + pv - 0.25, k * 0.7);
            d.add(f.cx + pu * 2.0, f.cy + pv + 0.25, k * 0.7);
        }
    }
    d.draw(cv, |_, _, k| mix(HEAT[2], HEAT[0], k));
    // whole, a star with a glow and spikes; they go as it's torn
    let whole = (1.0 - smooth(spread, 0.0, 0.14)) * on;
    let (pu, pv, behind) = path(f, h, at);
    if whole > 0.02 && !(behind && pu.hypot(pv) < h.rs * 1.03) {
        let (sx, sy) = (f.cx + pu * 2.0, f.cy + pv);
        let mut g = Dots::default();
        g.ring(sx, sy, 0.75, 0.6, whole * 0.6);
        g.0.remove(&(sx.round() as i32, sy.round() as i32));
        g.draw(cv, |_, _, _| HEAT[1]);
        let (x, y) = (sx.round() as i32, sy.round() as i32);
        for (d, k) in [(2, 0.5), (3, 0.25)] {
            cv.put(x - d, y, "─", (255, 230, 180), whole * k);
            cv.put(x + d, y, "─", (255, 230, 180), whole * k);
        }
        cv.put_with(x, y, "✸", (255, 250, 232), whole, true);
    }
}

/// Where the stream hits the disc: a flash, a ring of light spreading in
/// the disc's plane, sparks thrown out and cooling.
fn impact(cv: &mut Canvas, f: &Frame, h: &Hole) {
    let u = h.u;
    if !(HIT..HIT + 0.2).contains(&u) {
        return;
    }
    let q = smooth(u, HIT, HIT + 0.18);
    let k = (1.0 - q) * smooth(u, HIT, HIT + 0.01) * h.fade;
    if k <= 0.02 {
        return;
    }
    let (pu, pv, _) = path(f, h, 1.0);
    let (x, y) = (f.cx + pu * 2.0, f.cy + pv);
    let mut d = Dots::default();
    let r = 0.6 + q.powf(0.6) * h.rs * 1.4;
    d.ring(x, y, r, 0.38, k);
    d.ring(x, y, r * 0.9, 0.38, k * 0.5);
    d.draw(cv, |_, _, kk| mix(HEAT[2], HEAT[0], kk));
    for i in 0..40u64 {
        let a = unit(f.seed, 800 + i) * TAU;
        let dist = q.powf(0.5) * (1.5 + 5.0 * unit(f.seed, 900 + i));
        let (sx, sy) = (x + a.cos() * dist * 2.0, y + a.sin() * dist * 0.6);
        cv.put(
            sx.round() as i32,
            sy.round() as i32,
            if i % 5 == 0 { "•" } else { "∙" },
            ramp(&HEAT, q * 0.9),
            k * (0.6 + 0.4 * unit(f.seed, 1000 + i)),
        );
    }
    let flash = 1.0 - smooth(u, HIT, HIT + 0.07);
    if flash > 0.02 {
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        cv.put_with(xi, yi, "✸", HEAT[0], flash * h.fade, true);
        let len = (1.0 + 4.0 * flash) as i32;
        for s in 1..=len {
            let kk = flash * h.fade * (1.0 - s as f64 / (len + 1) as f64);
            cv.put(xi - s, yi, "─", HEAT[1], kk);
            cv.put(xi + s, yi, "─", HEAT[1], kk);
        }
        cv.put(xi, yi - 1, "│", HEAT[1], flash * h.fade * 0.6);
        cv.put(xi, yi + 1, "│", HEAT[1], flash * h.fade * 0.6);
    }
}

/// Where its particles are as the ℵ gathers: round the photon ring, the
/// last light (Hawking sparks rising from it).
pub(super) fn sources(f: &Frame) -> Vec<(f64, f64)> {
    let r = rs_last(f) * 1.03;
    (0..72)
        .map(|i| {
            let a = TAU * i as f64 / 72.0;
            (f.cx + a.cos() * r * 2.0, f.cy + a.sin() * r)
        })
        .collect()
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

    fn lit(b: &Buffer, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < b.area.width as i32
            && y < b.area.height as i32
            && b[(x as u16, y as u16)].symbol() != " "
    }

    /// A moment of the scene (ms) on a dark 120×32 screen, and its frame.
    fn shot(ms: u64, logo: bool, info: &Info) -> (Buffer, Frame<'_>) {
        let f = scene::at(120, 32, ms, logo, info);
        (scene::dark(Kind::Horizon, &f), f)
    }

    #[test]
    fn the_shadow_stays_dark_inside_its_ring_with_the_disc_across_it() {
        let info = info();
        let (b, f) = shot(1350, false, &info);
        let h = Hole::of(&f);
        let (cx, cy) = (f.cx as i32, f.cy as i32);
        // the shadow's upper half: nothing (the stars behind it hidden, the
        // disc's far side behind it)
        for y in 0..32 {
            for x in 0..120 {
                let (u, v) = ((x as f64 - f.cx) / 2.0, y as f64 - f.cy);
                if v < -0.5 && u.hypot(v) < h.rs * 0.85 {
                    assert!(!lit(&b, x, y), "({x},{y}) in the shadow is lit");
                }
            }
        }
        // its edge, the photon ring, at the top; the disc's near side across
        // the shadow, below the middle
        let top = (f.cy - h.rs * 1.03).round() as i32;
        assert!(
            (top - 1..=top + 1).any(|y| lit(&b, cx, y)),
            "no ring at the top"
        );
        let near = (h.rs * RIN * FLAT).ceil() as i32;
        assert!(
            (near..=near + 2).any(|dy| lit(&b, cx, cy + dy)),
            "no disc across the shadow"
        );
    }

    #[test]
    fn the_side_coming_toward_you_is_brighter_and_bluer() {
        let info = info();
        let (b, f) = shot(1350, false, &info);
        let h = Hole::of(&f);
        // the disc's middle rows, out past the shadow: left against right
        let side = |sign: f64| {
            let (mut light, mut blue, mut n) = (0.0f64, 0.0f64, 0.0f64);
            for y in (f.cy as i32 - 1)..=(f.cy as i32 + 1) {
                let mut u = h.rs * 1.6;
                while u < h.rs * 3.4 {
                    let x = (f.cx + sign * u * 2.0).round() as u16;
                    if let Color::Rgb(r, g, bl) = b[(x, y as u16)].fg {
                        light += r as f64 + g as f64 + bl as f64;
                        // its hue, whatever its brightness (drawn up from the dark)
                        blue += (bl as f64 - 22.0) / (r as f64 - 16.0).max(1.0);
                        n += 1.0;
                    }
                    u += 0.5;
                }
            }
            (light, blue / n.max(1.0))
        };
        let (left, right) = (side(-1.0), side(1.0));
        assert!(left.0 > right.0 * 1.3, "brighter: {left:?} {right:?}");
        assert!(left.1 > right.1 * 2.0, "bluer: {left:?} {right:?}");
    }

    #[test]
    fn a_star_falls_in_torn_into_a_stream_and_flares_where_it_hits() {
        let info = info();
        // whole as it comes: a star, out past the disc
        let (b, f) = shot(1350, false, &info);
        let h = Hole::of(&f);
        let star = (0..32)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .find(|&(x, y)| b[(x as u16, y as u16)].symbol() == "✸");
        let (x, y) = star.expect("the star");
        let (u, v) = ((x as f64 - f.cx) / 2.0, y as f64 - f.cy);
        assert!(u.hypot(v) > h.rs * 2.0, "it starts far out");
        // torn: no star any more, but its stream lit along its way
        let ms = (f.len * 0.575) as u64;
        let (b, f) = shot(ms, false, &info);
        let h = Hole::of(&f);
        assert!(!(0..32).any(|y| (0..120).any(|x| b[(x, y)].symbol() == "✸")));
        let q = ((0.575 - FALL.0) / (FALL.1 - FALL.0)).clamp(0.0, 1.0);
        let at = 1.15 * q.powf(1.5);
        let along = (0..9)
            .filter(|i| {
                let (pu, pv, _) = path(&f, &h, at - 0.06 + 0.015 * *i as f64);
                lit(
                    &b,
                    (f.cx + pu * 2.0).round() as i32,
                    (f.cy + pv).round() as i32,
                )
            })
            .count();
        assert!(along >= 8, "the stream: {along} of 9");
        // where it hits the disc, a flash
        let (b, f) = shot((f.len * (HIT + 0.01)) as u64, false, &info);
        let h = Hole::of(&f);
        let (pu, pv, _) = path(&f, &h, 1.0);
        let (x, y) = ((f.cx + pu * 2.0).round() as u16, (f.cy + pv).round() as u16);
        assert_eq!(b[(x, y)].symbol(), "✸");
    }

    #[test]
    fn crossing_the_horizon_it_ends_dark_but_for_the_photon_ring() {
        let info = info();
        let (b, f) = shot(scene::SCENE_MS - 1, false, &info);
        let ring = rs_last(&f) * 1.03;
        let (mut on, mut near) = (0, 0);
        for y in 0..29 {
            for x in 0..120 {
                if lit(&b, x, y) {
                    on += 1;
                    let (u, v) = ((x as f64 - f.cx) / 2.0, y as f64 - f.cy);
                    near += ((u.hypot(v) - ring).abs() < 1.2) as usize;
                }
            }
        }
        assert!(near > 60, "the ring: {near} cells");
        assert!(near * 10 >= on * 9, "{on} lit, {near} on the ring");
        // with the logo, its sparks rise from that ring
        assert!(sources(&f)
            .iter()
            .all(|&(x, y)| { ((((x - f.cx) / 2.0).hypot(y - f.cy)) - ring).abs() < 0.01 }));
    }
}
