//! `neon`: a rain-soaked city at night, cyberpunk's other splash. A skyline
//! comes up out of a blue-violet haze: towers in layers, the far ones pale
//! with it, the near ones black against it, their windows lighting up
//! here and there in warm yellow, cyan and magenta, fog drifting across.
//! Rain sets in: thin slanted streaks, faster and brighter near, splashing
//! on a wet street that mirrors the city, dimmed and rippling. Neon signs
//! flicker on one after another — your machines' names, in yellow, cyan,
//! magenta and red, each its own stutter, then steady with a glow round it
//! — and the city's own beside them, stacked down the towers' edges.
//! Flying traffic crosses in lanes, lights with trails, some behind the
//! near towers and one lane in front. It ends steady in the rain, one sign
//! buzzing. With the logo the city dims and the ℵ lights over it as the
//! biggest sign of all, its tubes flickering on band by band, yellow with
//! a red glow round it, the rain falling in front of it.

use super::scene::{Frame, GATHER_S};
use super::*;

type Rgb = (u8, u8, u8);

pub(super) const TITLE: &str = "neon in the rain";
/// Its colours on the way into the ℵ: the signs' magenta, then yellow.
pub(super) const COLS: [Rgb; 2] = [MAGENTA, YELLOW];

/// Neon: electric yellow, cyan, magenta, off-red; the alarm's crimson for
/// the ℵ's glow.
const YELLOW: Rgb = (252, 238, 10);
const CYAN: Rgb = (94, 246, 255);
const MAGENTA: Rgb = (255, 46, 136);
const RED: Rgb = (255, 74, 87);
const CRIMSON: Rgb = (255, 0, 60);
const WHITE: Rgb = (255, 250, 236);
/// The haze behind the towers, high to low: deep blue, violet, the city's
/// own magenta glow at the horizon.
const HAZE: [Rgb; 3] = [(34, 30, 100), (86, 46, 150), (176, 64, 156)];
/// The towers: the far ones pale with the haze, the middle ones darker
/// (the near ones are black: nothing drawn), the near ones' rims; fog.
const FAR: Rgb = (104, 76, 160);
const MID: Rgb = (44, 34, 86);
const RIM: Rgb = (110, 86, 170);
const FOG: Rgb = (150, 140, 196);
/// Windows: mostly warm, some cyan, a few magenta.
const WARM: Rgb = (255, 196, 92);
/// The rain: far, near.
const DROP: [Rgb; 2] = [(100, 120, 186), (186, 214, 250)];
/// The signs' colours, in turn.
const NEON: [Rgb; 4] = [YELLOW, CYAN, MAGENTA, RED];
/// The ℵ's tubes: pale, electric, deep yellow; unlit.
const TUBE: [Rgb; 3] = [(255, 250, 160), (252, 238, 10), (226, 200, 0)];
const DEAD: Rgb = (96, 70, 30);

/// The acts, as shares of the scene: the haze and the towers coming up;
/// the windows lighting; the rain setting in; the street getting wet; the
/// signs flickering on; the traffic; the one sign buzzing at the end.
const DUSK: (f64, f64) = (0.0, 0.16);
const WAKE: (f64, f64) = (0.04, 0.55);
const RAIN: (f64, f64) = (0.06, 0.24);
const WET: (f64, f64) = (0.1, 0.32);
const SIGNS: (f64, f64) = (0.24, 0.62);
const TRAFFIC: f64 = 0.36;
const BUZZ: f64 = 0.78;
/// How long a sign stutters before it holds (a share of the scene).
const STUTTER: f64 = 0.07;

/// The towers' layers, far to near: how high (shares of the skyline, the
/// lowest and the highest), how wide and the gaps between (columns, at
/// 120 wide).
type Layer = (u8, f64, f64, (i32, i32), (i32, i32));
const LAYERS: [Layer; 3] = [
    (0, 0.36, 0.74, (3, 7), (1, 4)),
    (1, 0.24, 0.56, (4, 9), (1, 5)),
    (2, 0.12, 0.42, (5, 12), (3, 12)),
];

/// The city's own signs, for a small fleet: across, and down a tower's edge.
const WORDS: [&str; 4] = ["NOODLES", "OPEN 24H", "CLINIC", "DATA"];
const DOWN: [&str; 3] = ["HOTEL", "BAR", "RAMEN"];

/// A cell's own key, for hashing.
fn key(x: i32, y: i32) -> u64 {
    (x as u32 as u64) << 20 | y as u32 as u64
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Roof {
    Flat,
    /// its edges a couple of rows lower
    Step,
    /// an antenna with a beacon blinking on top
    Mast,
}

#[derive(Clone, Copy, Debug)]
struct Tower {
    x0: i32,
    x1: i32,
    top: i32,
    /// 0 far, 1 middle, 2 near
    layer: u8,
    roof: Roof,
    h: u64,
}

impl Tower {
    /// Its roof's row over column `x`.
    fn top_at(&self, x: i32) -> i32 {
        let inset = ((self.x1 - self.x0) / 4).max(1);
        if self.roof == Roof::Step && (x < self.x0 + inset || x > self.x1 - inset) {
            self.top + 2
        } else {
            self.top
        }
    }

    /// The mast's column, and how tall it is.
    fn mast(&self) -> Option<(i32, i32)> {
        (self.roof == Roof::Mast).then(|| ((self.x0 + self.x1) / 2, 2 + (self.h % 3) as i32))
    }
}

/// A sign: its text, where (its box's top left), its colour, when it comes
/// on; framed (a box round it) or run down a tower's edge; whether it's
/// the one buzzing at the end, and the letter of it that's dead.
#[derive(Clone, Debug)]
struct Sign {
    text: Vec<char>,
    x: i32,
    y: i32,
    col: Rgb,
    on: f64,
    framed: bool,
    down: bool,
    buzz: bool,
    dead: Option<usize>,
    machine: bool,
}

impl Sign {
    fn w(&self) -> i32 {
        if self.down {
            1
        } else {
            self.text.len() as i32 + if self.framed { 4 } else { 0 }
        }
    }

    fn h(&self) -> i32 {
        if self.down {
            self.text.len() as i32
        } else if self.framed {
            3
        } else {
            1
        }
    }

    /// How lit it is at `t` (0 dark, 1 steady): off; a stutter, the tube
    /// catching; steady with a little hum; the one buzzing at the end.
    fn light(&self, f: &Frame, i: usize) -> f64 {
        let u = f.u();
        if u < self.on {
            return 0.0;
        }
        let tick = (f.t / 45.0) as u64;
        let r = unit(f.seed ^ 0x5165 ^ (i as u64) << 40, tick);
        if u < self.on + STUTTER {
            // more often on as it catches
            let q = (u - self.on) / STUTTER;
            return if r < 0.25 + 0.7 * q {
                0.6 + 0.4 * q
            } else {
                0.08
            };
        }
        if self.buzz && u >= BUZZ {
            // dropping out now and then, in bursts
            let b = unit(f.seed ^ 0xb022, (f.t / 70.0) as u64);
            if b < 0.3 {
                return if r < 0.5 { 0.12 } else { 0.55 };
            }
        }
        0.93 + 0.07 * r
    }
}

/// The city at one moment's size: its rows (the street's below the
/// skyline's), its towers by layer, its signs.
struct City {
    rows: i32,
    /// the street's first row
    ys: i32,
    towers: Vec<Tower>,
    /// each layer's tower over each column (an index, or none)
    by: [Vec<Option<usize>>; 3],
    signs: Vec<Sign>,
}

impl City {
    fn of(f: &Frame) -> City {
        let w = f.w as i32;
        let rows = (f.h as i32 - 3).max(4);
        let ys = ((rows as f64) * 0.64).round() as i32;
        let ysf = ys as f64;
        let mut towers: Vec<Tower> = Vec::new();
        let mut signs = plan(f, w, ys);
        // each sign across on a near tower of its own
        for s in signs.iter().filter(|s| !s.down) {
            let h = hash(f.seed ^ 0x70e1, s.x as u64);
            let pad = 2 + (h % 3) as i32;
            towers.push(Tower {
                x0: s.x - pad,
                x1: s.x + s.w() - 1 + pad + (h >> 8) as i32 % 2,
                top: (s.y - 1 - (unit(h, 1) * ysf * 0.18) as i32).max(1),
                layer: 2,
                roof: if unit(h, 2) < 0.35 {
                    Roof::Mast
                } else {
                    Roof::Flat
                },
                h,
            });
        }
        // the rest of the near layer in the gaps; the middle and far ones
        // packed behind, taller as they go back
        let scale = (w as f64 / 120.0).clamp(0.6, 1.6);
        for (layer, lo, hi, (wa, wb), (ga, gb)) in LAYERS {
            let mut x = -2 - (hash(f.seed, layer as u64) % 4) as i32;
            let mut k = 0u64;
            while x < w + 2 {
                let h = hash(f.seed ^ 0x0c17 ^ (layer as u64) << 32, k);
                k += 1;
                let tw = (wa as f64 + (wb - wa) as f64 * unit(h, 1)) * scale;
                let tw = (tw.round() as i32).max(2);
                let gap = ga + (unit(h, 2) * (gb - ga + 1) as f64) as i32;
                let (x0, x1) = (x, x + tw - 1);
                x = x1 + 1 + gap;
                // the near layer leaves the signs' towers be
                if layer == 2
                    && towers
                        .iter()
                        .any(|t| t.layer == 2 && t.x0 <= x1 + 1 && x0 <= t.x1 + 1)
                {
                    continue;
                }
                let top = (ysf - ysf * (lo + (hi - lo) * unit(h, 3))).round() as i32;
                let r = unit(h, 4);
                towers.push(Tower {
                    x0,
                    x1,
                    top: top.clamp(1, ys - 2),
                    layer,
                    roof: if r < 0.25 {
                        Roof::Step
                    } else if r < 0.42 {
                        Roof::Mast
                    } else {
                        Roof::Flat
                    },
                    h,
                });
            }
        }
        // far off, one tower above the rest, a mast on it
        let h = hash(f.seed, 0x3e6a);
        let mw = ((7.0 + 5.0 * unit(h, 1)) * scale).round() as i32;
        let mx = (w as f64 * (0.15 + 0.7 * unit(h, 2))) as i32;
        towers.push(Tower {
            x0: mx - mw / 2,
            x1: mx - mw / 2 + mw - 1,
            top: ((ysf * 0.1).round() as i32).max(3),
            layer: 0,
            roof: Roof::Mast,
            h,
        });
        // the city's signs down the edges of near towers tall enough, that
        // carry no sign across
        let across: Vec<(i32, i32)> = signs
            .iter()
            .filter(|s| !s.down)
            .map(|s| (s.x, s.x + s.w() - 1))
            .collect();
        let mut free = towers.iter().filter(|t| {
            t.layer == 2
                && t.x1 - t.x0 >= 4
                && t.x0 >= 1
                && t.x1 < w - 1
                && !across.iter().any(|&(a, b)| t.x0 <= b && a <= t.x1)
        });
        for s in signs.iter_mut().filter(|s| s.down) {
            let n = s.text.len() as i32;
            let Some(t) = free.find(|t| ys - t.top - 2 > n) else {
                break;
            };
            s.x = if (t.h >> 3).is_multiple_of(2) {
                t.x0 + 1
            } else {
                t.x1 - 1
            };
            s.y = t.top + 1 + (unit(t.h, 9) * (ys - t.top - n - 2) as f64) as i32;
        }
        signs.retain(|s| s.x != i32::MIN);
        // the one that buzzes at the end: the city's own (one down a
        // tower's edge first), a letter of it dead; else the last of yours,
        // whole
        let pick = signs
            .iter()
            .position(|s| s.down)
            .or_else(|| signs.iter().position(|s| !s.machine))
            .or(signs.len().checked_sub(1));
        if let Some(i) = pick {
            let s = &mut signs[i];
            s.buzz = true;
            let n = s.text.len() as u64;
            let j = 1 + (hash(f.seed, 0xdead) % (n.max(2) - 1)) as usize;
            if !s.machine && s.text.get(j).is_some_and(|c| *c != ' ') {
                s.dead = Some(j);
            }
        }
        let mut by: [Vec<Option<usize>>; 3] = Default::default();
        for (l, col) in by.iter_mut().enumerate() {
            *col = vec![None; w.max(0) as usize];
            for (i, t) in towers.iter().enumerate() {
                if t.layer as usize != l {
                    continue;
                }
                for x in t.x0.max(0)..=t.x1.min(w - 1) {
                    let slot = &mut col[x as usize];
                    if slot.is_none_or(|j| towers[j].top > t.top) {
                        *slot = Some(i);
                    }
                }
            }
        }
        City {
            rows,
            ys,
            towers,
            by,
            signs,
        }
    }
}

/// Where the signs go: your machines' names across (the city's own words
/// to make three), each in a slot of the skyline's width, at its own
/// height, in its own colour, coming on in its own turn; and two of the
/// city's down towers' edges (placed with the towers). On a small screen
/// they're bare words, no frame.
fn plan(f: &Frame, w: i32, ys: i32) -> Vec<Sign> {
    let framed = ys >= 12;
    let mut words: Vec<(String, bool)> = f
        .info
        .fleet
        .iter()
        .take(6)
        .map(|m| (m.name.chars().take(14).collect(), true))
        .collect();
    for g in WORDS {
        if words.len() >= 3 {
            break;
        }
        words.push((g.to_string(), false));
    }
    // as many as fit the width, side by side with room between
    let room = |ws: &[(String, bool)]| {
        ws.iter()
            .map(|(s, _)| s.chars().count() as i32 + if framed { 4 } else { 0 } + 6)
            .sum::<i32>()
    };
    while words.len() > 1 && room(&words) > w - 2 {
        words.pop();
    }
    let n = words.len();
    let slot = w as f64 / n as f64;
    // the order they come on in: shuffled
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        order.swap(
            i,
            (hash(f.seed ^ 0x0de4, i as u64) % (i as u64 + 1)) as usize,
        );
    }
    let mut signs: Vec<Sign> = words
        .into_iter()
        .enumerate()
        .map(|(i, (text, machine))| {
            let h = hash(f.seed ^ 0x5197, i as u64);
            let text: Vec<char> = text.chars().collect();
            let sw = text.len() as i32 + if framed { 4 } else { 0 };
            let free = (slot - sw as f64 - 6.0).max(0.0);
            let cx = slot * (i as f64 + 0.5) + (unit(h, 1) - 0.5) * free * 0.7;
            let top = if framed { 0.2 } else { 0.25 };
            let y = (ys as f64 * (top + 0.36 * unit(h, 2))).round() as i32;
            let turn = order.iter().position(|&o| o == i).unwrap_or(i);
            Sign {
                text,
                x: (cx - sw as f64 / 2.0).round() as i32,
                y: y.max(1),
                col: NEON[(i + (f.seed % 4) as usize) % 4],
                on: SIGNS.0 + (SIGNS.1 - SIGNS.0 - STUTTER) * turn as f64 / n.max(1) as f64,
                framed,
                down: false,
                buzz: false,
                dead: None,
                machine,
            }
        })
        .collect();
    // the city's own, down a tower's edge (placed with the towers)
    if ys >= 10 {
        for (j, word) in DOWN.iter().take(2).enumerate() {
            let h = hash(f.seed ^ 0xd0e1, j as u64);
            signs.push(Sign {
                text: word.chars().collect(),
                x: i32::MIN,
                y: 0,
                col: [MAGENTA, CYAN][(j + (h % 2) as usize) % 2],
                on: SIGNS.0 + 0.04 + (SIGNS.1 - SIGNS.0 - STUTTER - 0.04) * unit(h, 1),
                framed: false,
                down: true,
                buzz: false,
                dead: None,
                machine: false,
            });
        }
    }
    signs
}

/// What a cell of the skyline is (the street reflects each its own way).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Is {
    Haze,
    Far,
    Mid,
    /// a near tower: black, but for its rims, windows and the glow on it
    Near,
    Rim,
    Window,
    Mast,
    Beacon,
    Sign,
    Glow,
    Car,
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    g: char,
    col: Rgb,
    k: f64,
    bold: bool,
    is: Is,
}

fn cell(g: char, col: Rgb, k: f64, is: Is) -> Cell {
    Cell {
        g,
        col,
        k,
        bold: false,
        is,
    }
}

/// The lights of one moment: the signs (and the near lane's traffic) in
/// front of everything, their glow on the towers, the far lanes' traffic
/// behind the near towers.
#[derive(Default)]
struct Lights {
    front: HashMap<(i32, i32), Cell>,
    glow: HashMap<(i32, i32), Cell>,
    back: HashMap<(i32, i32), Cell>,
}

impl Lights {
    fn of(f: &Frame, city: &City) -> Lights {
        let mut l = Lights::default();
        for (i, s) in city.signs.iter().enumerate() {
            l.sign(f, s, i);
        }
        l.traffic(f, city);
        l
    }

    /// A sign: its tubes (unlit, dim, before it comes on), its frame, its
    /// glow on the tower round it.
    fn sign(&mut self, f: &Frame, s: &Sign, i: usize) {
        let light = s.light(f, i);
        let lit = light > 0.3;
        let tube = |k: f64| if lit { k * light } else { 0.14 };
        let mut put = |x: i32, y: i32, g: char, k: f64, bold: bool| {
            let c = Cell {
                g,
                col: if lit {
                    s.col
                } else {
                    mix(s.col, (60, 50, 70), 0.5)
                },
                k,
                bold: bold && lit,
                is: Is::Sign,
            };
            self.front.insert((x, y), c);
        };
        if s.down {
            for (j, ch) in s.text.iter().enumerate() {
                let dead = s.dead == Some(j);
                put(
                    s.x,
                    s.y + j as i32,
                    *ch,
                    if dead { 0.12 } else { tube(1.0) },
                    !dead,
                );
            }
        } else {
            let (x, y) = (s.x, s.y);
            let tx = x + if s.framed { 2 } else { 0 };
            let ty = y + s.framed as i32;
            if s.framed {
                let w = s.w();
                for j in 1..w - 1 {
                    put(x + j, y, '─', tube(0.7), false);
                    put(x + j, y + 2, '─', tube(0.7), false);
                }
                for (dx, dy, g) in [(0, 0, '╭'), (w - 1, 0, '╮'), (0, 2, '╰'), (w - 1, 2, '╯')]
                {
                    put(x + dx, y + dy, g, tube(0.75), false);
                }
                put(x, y + 1, '│', tube(0.7), false);
                put(x + w - 1, y + 1, '│', tube(0.7), false);
                // its padding: dark, nothing behind showing through
                put(x + 1, y + 1, ' ', 0.0, false);
                put(x + w - 2, y + 1, ' ', 0.0, false);
            }
            // (its spaces dark too, and its dead letter)
            for (j, ch) in s.text.iter().enumerate() {
                let dead = s.dead == Some(j);
                let k = if dead { 0.12 } else { tube(1.0) };
                put(tx + j as i32, ty, *ch, k, !dead);
            }
        }
        // the glow, on the tower round it
        if !lit {
            return;
        }
        let (gx, gy) = (3, 1);
        for y in s.y - gy..s.y + s.h() + gy {
            for x in s.x - gx..s.x + s.w() + gx {
                let dx = if x < s.x {
                    (s.x - x) as f64 / gx as f64
                } else if x >= s.x + s.w() {
                    (x - s.x - s.w() + 1) as f64 / gx as f64
                } else {
                    0.0
                };
                let dy = if y < s.y {
                    (s.y - y) as f64 / gy as f64
                } else if y >= s.y + s.h() {
                    (y - s.y - s.h() + 1) as f64 / gy as f64
                } else {
                    0.0
                };
                let d = dx.max(dy);
                if d <= 0.0 || d > 1.0 || self.front.contains_key(&(x, y)) {
                    continue;
                }
                let k = 0.42 * light * (1.0 - d * 0.6);
                self.glow.insert((x, y), cell('░', s.col, k, Is::Glow));
            }
        }
    }

    /// The traffic: lanes across the sky, each its own way and speed, its
    /// vehicles coming in one after another once the city's awake: a white
    /// headlight, a body in its underglow, a red trail behind. The far
    /// lanes pass behind the near towers; the nearest, in front.
    fn traffic(&mut self, f: &Frame, city: &City) {
        let w = f.w as i32;
        let ys = city.ys as f64;
        let lanes = [
            ((ys * 0.16).round() as i32, 1, 20.0, false),
            ((ys * 0.32).round() as i32, -1, 30.0, false),
            ((ys * 0.5).round() as i32, 1, 48.0, true),
        ];
        let span = (w + 50) as f64;
        for (li, (y, dir, speed, front)) in lanes.into_iter().enumerate() {
            let speed = speed * (w as f64 / 120.0).clamp(0.7, 1.6);
            for j in 0..3u64 {
                let h = hash(f.seed ^ 0x7a4f ^ (li as u64) << 8, j);
                let start =
                    f.len * (TRAFFIC + 0.13 * j as f64 + 0.06 * li as f64 + 0.05 * unit(h, 1));
                if f.t < start {
                    continue;
                }
                let run = (speed * (f.t - start) / 1000.0).rem_euclid(span);
                let x = if dir > 0 {
                    -6.0 + run
                } else {
                    w as f64 + 5.0 - run
                };
                let hx = x.round() as i32;
                let glow = if unit(h, 2) < 0.5 { CYAN } else { MAGENTA };
                let trail = if front { 9 } else { 5 };
                let scale = if front { 1.0 } else { 0.7 };
                let map = if front {
                    &mut self.front
                } else {
                    &mut self.back
                };
                map.insert(
                    (hx, y),
                    Cell {
                        g: '•',
                        col: WHITE,
                        k: scale,
                        bold: front,
                        is: Is::Car,
                    },
                );
                if front {
                    map.insert((hx - dir, y), cell('▬', glow, 0.9, Is::Car));
                }
                for k in 1..=trail {
                    let tx = hx - dir * (k + front as i32);
                    let q = 1.0 - k as f64 / (trail + 1) as f64;
                    map.entry((tx, y))
                        .or_insert(cell('─', RED, scale * (0.2 + 0.7 * q), Is::Car));
                }
            }
        }
    }
}

/// Fog: two bands drifting across the towers (0–1).
fn fog(f: &Frame, city: &City, x: i32, y: i32) -> f64 {
    let ys = city.ys as f64;
    let mut v: f64 = 0.0;
    for (b, at, speed) in [(0u64, 0.42, 0.6), (1, 0.74, -0.9)] {
        let row = ys * at + (f.t / 1000.0 * 0.4 + b as f64).sin() * 0.6;
        let band = (1.0 - ((y as f64 - row) / 1.6).abs()).max(0.0);
        let drift = 0.5 + 0.5 * (x as f64 * 0.07 + f.t / 1000.0 * speed + b as f64 * 3.0).sin();
        v = v.max(band * drift);
    }
    v * f.act(DUSK.0 + 0.06, DUSK.1 + 0.1)
}

/// A tower of `layer` at `(x, y)`, and what of it: its body, its mast,
/// the beacon on top.
fn tower(city: &City, layer: usize, x: i32, y: i32) -> Option<(&Tower, Is)> {
    let i = (*city.by[layer].get(x as usize)?)?;
    let t = &city.towers[i];
    if y >= t.top_at(x) {
        return Some((t, Is::Near));
    }
    let (mx, mh) = t.mast()?;
    (x == mx && y >= t.top - mh).then_some((
        t,
        if y == t.top - mh {
            Is::Beacon
        } else {
            Is::Mast
        },
    ))
}

/// A tower's window at `(x, y)`, lit by now (its own moment in the wake),
/// if there's one: every other column, every other row, a share of them.
fn window(f: &Frame, t: &Tower, x: i32, y: i32, share: f64) -> Option<Rgb> {
    if x <= t.x0 || x >= t.x1 || (x - t.x0) % 2 != 1 || (y - t.top) % 2 != 1 {
        return None;
    }
    let h = hash(t.h ^ 0x3171, key(x, y));
    if unit(h, 1) > share {
        return None;
    }
    let wake = WAKE.0 + (WAKE.1 - WAKE.0) * unit(h, 2);
    if f.u() < wake {
        return None;
    }
    // a screen inside, flickering
    if unit(h, 4) < 0.06 {
        let fl = unit(h, (f.t / 160.0) as u64);
        return Some(mix(CYAN, (60, 90, 255), fl));
    }
    Some(match unit(h, 3) {
        r if r < 0.7 => WARM,
        r if r < 0.9 => CYAN,
        _ => MAGENTA,
    })
}

/// The skyline at `(x, y)`: the front lights, a near tower (its rims,
/// windows and the glow on it, black otherwise), the far lanes, the middle
/// and far towers, the haze.
fn sky(f: &Frame, city: &City, lights: &Lights, x: i32, y: i32) -> Option<Cell> {
    if x < 0 || x >= f.w as i32 || y < 0 || y >= city.ys {
        return None;
    }
    if let Some(c) = lights.front.get(&(x, y)) {
        return Some(*c);
    }
    let fg = fog(f, city, x, y);
    let up = |a: f64| f.act(a, a + 0.08);
    if let Some((t, part)) = tower(city, 2, x, y) {
        let k = up(DUSK.0 + 0.08);
        match part {
            Is::Mast => return Some(cell('│', RIM, 0.5 * k, Is::Mast)),
            Is::Beacon => return Some(beacon(f, t, k)),
            _ => {}
        }
        if let Some(c) = lights.glow.get(&(x, y)) {
            return Some(*c);
        }
        if let Some(col) = window(f, t, x, y, 0.3) {
            return Some(cell('▪', col, 0.75 * (1.0 - 0.4 * fg), Is::Window));
        }
        if y == t.top_at(x) {
            return Some(cell('▔', RIM, 0.55 * k, Is::Rim));
        }
        if x == t.x0 || x == t.x1 {
            let g = if x == t.x0 { '▏' } else { '▕' };
            return Some(cell(g, RIM, 0.3 * k, Is::Rim));
        }
        if fg > 0.15 {
            return Some(cell('░', FOG, fg * 0.5 * k, Is::Near));
        }
        return Some(cell(' ', RIM, 0.0, Is::Near));
    }
    if let Some(c) = lights.back.get(&(x, y)) {
        return Some(*c);
    }
    // the middle and far towers' fills: floors (a seventh-eighths block,
    // the line between them showing), darker the nearer
    for (layer, fill, col, kk, share, is) in [
        (1usize, '▇', MID, 0.9, 0.2, Is::Mid),
        (0, '▇', FAR, 0.5, 0.14, Is::Far),
    ] {
        let Some((t, part)) = tower(city, layer, x, y) else {
            continue;
        };
        let k = up(DUSK.0 + 0.04 * (1 - layer) as f64) * kk;
        return Some(match part {
            Is::Mast => cell('│', mix(col, RIM, 0.5), 0.5 * k, Is::Mast),
            Is::Beacon => beacon(f, t, k),
            _ => match window(f, t, x, y, share) {
                Some(w) => cell(
                    if layer == 1 { '▪' } else { '·' },
                    w,
                    (0.45 + 0.1 * layer as f64) * (1.0 - 0.5 * fg),
                    Is::Window,
                ),
                _ => cell(fill, mix(col, FOG, fg * 0.7), k * (1.0 + 0.5 * fg), is),
            },
        });
    }
    haze(f, city, x, y)
}

/// A mast's beacon, blinking red.
fn beacon(f: &Frame, t: &Tower, k: f64) -> Cell {
    let on = (((f.t + (t.h % 900) as f64) / 450.0) as u64).is_multiple_of(2);
    let mut c = cell('•', CRIMSON, if on { k } else { 0.25 * k }, Is::Beacon);
    c.bold = on;
    c
}

/// The haze behind it all: thin up high, thick and glowing magenta at the
/// horizon, drifting.
fn haze(f: &Frame, city: &City, x: i32, y: i32) -> Option<Cell> {
    let ys = city.ys as f64;
    let hz = ((y as f64 - ys * 0.02) / (ys * 0.98)).clamp(0.0, 1.0);
    let glow = hz.powf(1.3);
    let drift = 0.8 + 0.2 * (x as f64 * 0.11 + y as f64 * 0.7 + f.t / 1000.0 * 0.5).sin();
    let up = f.act(DUSK.0, DUSK.0 + 0.1);
    let d = glow * drift * up;
    let fg = fog(f, city, x, y);
    let col = mix(ramp(&HAZE, hz), FOG, fg * 0.5);
    let k = (0.25 + 0.55 * glow + 0.25 * fg) * up;
    // an even glow, thickest low down; thin up high, a cell here and there
    let r = unit(f.seed ^ 0x4a2e, key(x, y));
    let g = match d + fg * 0.3 {
        d if d > 0.72 => '▒',
        d if d > 0.42 => '░',
        d if r < d * 1.6 => '░',
        _ if r < 0.025 * up => '·',
        _ => return None,
    };
    Some(cell(g, col, k, Is::Haze))
}

pub(super) fn draw(cv: &mut Canvas, f: &Frame) {
    // with the logo the city dims under its sign, but goes on
    let fade = if f.opts.logo {
        0.28 + 0.72 * f.fade()
    } else {
        1.0
    };
    let city = City::of(f);
    let lights = Lights::of(f, &city);
    skyline(cv, f, &city, &lights, fade);
    street(cv, f, &city, &lights, fade);
    rain(cv, f, &city, &lights, fade, None);
}

fn put(cv: &mut Canvas, x: i32, y: i32, c: Cell, k: f64) {
    if c.g == ' ' {
        return;
    }
    let mut b = [0u8; 4];
    let g = if cv.mono && matches!(c.is, Is::Mid | Is::Far | Is::Haze) {
        // no colour to tell them by: just their edges' worth
        return;
    } else {
        c.g
    };
    cv.put_with(x, y, g.encode_utf8(&mut b), c.col, c.k * k, c.bold);
}

/// The skyline, cell by cell.
fn skyline(cv: &mut Canvas, f: &Frame, city: &City, lights: &Lights, fade: f64) {
    for y in 0..city.ys {
        for x in 0..f.w as i32 {
            if let Some(c) = sky(f, city, lights, x, y) {
                put(cv, x, y, c, fade);
            }
        }
    }
}

/// The wet street: the skyline mirrored in it, dimmer the nearer,
/// rippling; each lit sign's light smeared down it, broken by the ripples.
fn street(cv: &mut Canvas, f: &Frame, city: &City, lights: &Lights, fade: f64) {
    let wet = f.act(WET.0, WET.1);
    if wet <= 0.0 {
        return;
    }
    let (ys, rows) = (city.ys, city.rows);
    let deep = (rows - ys).max(1) as f64;
    let t = f.t / 1000.0;
    // each row's ripple: a sway, more the nearer
    let sway = |y: i32| {
        let depth = (y - ys) as f64 / deep;
        (y as f64 * 1.9 + t * 3.5).sin() * (0.3 + 1.4 * depth)
    };
    for y in ys..rows {
        let d = y - ys;
        let depth = d as f64 / deep;
        let ym = ys - 1 - d;
        let row = sway(y);
        let k = wet * fade * (0.5 - 0.32 * depth);
        for x in 0..f.w as i32 {
            let wob = (row + 0.5 * (x as f64 * 0.31 + y as f64 + t * 5.0).sin()).round() as i32;
            let Some(src) = sky(f, city, lights, x + wob, ym) else {
                continue;
            };
            let (g, kk) = match src.is {
                Is::Sign => ('▒', 0.9),
                Is::Glow => ('░', 0.8),
                Is::Window | Is::Beacon => ('·', 0.8),
                Is::Car => ('─', 0.8),
                Is::Mid | Is::Far => ('▀', 0.8),
                Is::Rim | Is::Mast => ('─', 0.5),
                Is::Haze => (src.g, 0.6),
                Is::Near => continue,
            };
            put(
                cv,
                x,
                y,
                Cell {
                    g,
                    k: src.k * kk,
                    bold: false,
                    ..src
                },
                k,
            );
        }
    }
    for (i, s) in city.signs.iter().enumerate() {
        let light = s.light(f, i);
        if light < 0.3 {
            continue;
        }
        let (xa, xb) = match (s.down, s.framed) {
            (true, _) => (s.x, s.x),
            (false, true) => (s.x + 2, s.x + s.w() - 3),
            (false, false) => (s.x, s.x + s.w() - 1),
        };
        for y in ys..rows {
            let depth = (y - ys) as f64 / deep;
            let wob = sway(y).round() as i32;
            for x in xa..=xb {
                let r = unit(f.seed ^ 0x57ea, key(x, y) ^ ((f.t / 110.0) as u64) << 40);
                if r < 0.3 {
                    continue;
                }
                let k = light * wet * fade * (0.8 - 0.6 * depth) * (0.6 + 0.4 * r);
                let g = match k {
                    k if k > 0.5 => "┃",
                    k if k > 0.3 => "│",
                    _ => "╎",
                };
                cv.put_with(x + wob, y, g, s.col, k, false);
            }
        }
    }
}

/// The rain: thin slanted streaks, the near ones longer, brighter and
/// quicker, each landing somewhere on the street with a splash; never over
/// a sign's letters. Over the ℵ's cells (`lit`) a drop catches its light
/// instead of hiding it.
fn rain(
    cv: &mut Canvas,
    f: &Frame,
    city: &City,
    lights: &Lights,
    fade: f64,
    lit: Option<&std::collections::HashSet<(i32, i32)>>,
) {
    let on = f.act(RAIN.0, RAIN.1);
    if on <= 0.0 {
        return;
    }
    let (w, rows, ys) = (f.w as i32, city.rows, city.ys);
    let t = f.t / 1000.0;
    let n = (w * rows / 16).max(20) as u64;
    for i in 0..n {
        let h = hash(f.seed ^ 0x7a1d, i);
        if unit(h, 0) > on {
            continue;
        }
        let near = unit(h, 1) < 0.3;
        let (v, len) = if near {
            (34.0 + 16.0 * unit(h, 2), 3 + (h >> 20) as i32 % 3)
        } else {
            (16.0 + 10.0 * unit(h, 2), 1 + (h >> 20) as i32 % 2)
        };
        // where it lands: the near ones nearer (lower on the street)
        let reach = (rows - ys) as f64;
        let land = ys
            + if near {
                (reach * (0.4 + 0.6 * unit(h, 3))) as i32
            } else {
                (reach * 0.55 * unit(h, 3)) as i32
            }
            .min(rows - ys - 1);
        let period = land as f64 + len as f64 + rows as f64 * (0.2 + 0.6 * unit(h, 4));
        let s = unit(h, 5) * period + v * t;
        let cyc = (s / period).floor();
        let y = (s - cyc * period).floor() as i32 - len;
        // each fall from its own column, drifting left as it falls
        let x0 = (hash(h, cyc as u64) % (w + rows) as u64) as i32;
        let col = DROP[near as usize];
        let k = (if near { 0.75 } else { 0.42 }) * fade;
        if y <= land {
            for j in 0..len {
                let (x, yy) = (x0 - y + j, y - j);
                if yy < 0
                    || yy > land
                    || lights.front.get(&(x, yy)).is_some_and(|c| c.is == Is::Sign)
                {
                    continue;
                }
                let kk = k * (1.0 - 0.5 * j as f64 / len as f64);
                if lit.is_some_and(|m| m.contains(&(x, yy))) {
                    catch(cv, x, yy, kk);
                    continue;
                }
                cv.put_with(x, yy, "╱", col, kk, near && j == 0);
            }
        } else {
            // the splash, for a moment after it lands
            let since = (s - cyc * period - len as f64 - land as f64) / v;
            let x = x0 - land;
            if since < 0.06 {
                cv.put_with(x, land, "∙", mix(col, WHITE, 0.4), k, near);
            } else if since < 0.14 && near {
                cv.put(x - 1, land, "·", col, k * 0.7);
                cv.put(x + 1, land, "·", col, k * 0.7);
            }
        }
    }
}

/// A drop crossing a lit cell: the cell a little whiter where it passes.
fn catch(cv: &mut Canvas, x: i32, y: i32, k: f64) {
    if x < 0 || y < 0 || x >= cv.area.width as i32 || y >= cv.area.height as i32 {
        return;
    }
    let pos = (cv.area.x + x as u16, cv.area.y + y as u16);
    if let Some(c) = cv.buf.cell_mut(pos) {
        if let Color::Rgb(r, g, b) = c.fg {
            let (r, g, b) = mix((r, g, b), WHITE, 0.45 * k);
            c.fg = Color::Rgb(r, g, b);
        }
    }
}

/// Where its particles are as the ℵ gathers: the signs and the lit windows.
pub(super) fn sources(f: &Frame) -> Vec<(f64, f64)> {
    let city = City::of(f);
    let mut v: Vec<(f64, f64)> = Vec::new();
    for s in &city.signs {
        for j in 0..s.text.len() as i32 {
            let (x, y) = if s.down {
                (s.x, s.y + j)
            } else {
                (
                    s.x + j + if s.framed { 2 } else { 0 },
                    s.y + s.framed as i32,
                )
            };
            v.push((x as f64, y as f64));
        }
    }
    for t in city.towers.iter().filter(|t| t.layer >= 1) {
        for y in (t.top + 1..city.ys).step_by(2) {
            for x in (t.x0 + 1..t.x1).step_by(2) {
                if window(f, t, x, y, 0.3).is_some() {
                    v.push((x as f64, y as f64));
                }
            }
        }
    }
    v
}

/// Is the ℵ there at `(x, y)` (a block or its braille edge)?
fn landed(cv: &Canvas, x: i32, y: i32) -> bool {
    !cv.blank(x, y)
        && cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
            .symbol()
            .chars()
            .next()
            .is_some_and(|c| c == '█' || ('\u{2801}'..='\u{28FF}').contains(&c))
}

/// The ℵ sign's tubes: bands across its diagonal, lit in turn.
const TUBES: usize = 5;

/// Which tube a cell of the ℵ is on (`fpos`: along its diagonal, 0–1).
fn tube_of(fpos: f64) -> usize {
    ((fpos * TUBES as f64) as usize).min(TUBES - 1)
}

/// When tube `i` comes on (ms into the run), after its cells have landed.
fn tube_on(f: &Frame, i: usize) -> f64 {
    f.len + GATHER_S * 0.62 + i as f64 * 95.0
}

/// How lit tube `i` is at `t`: dark, stuttering on, steady.
fn tube_light(f: &Frame, i: usize) -> f64 {
    let on = tube_on(f, i);
    if f.t < on {
        return 0.0;
    }
    if f.t < on + 110.0 {
        let r = unit(f.seed ^ 0x70be ^ (i as u64) << 32, (f.t / 30.0) as u64);
        return if r < 0.35 + 0.5 * (f.t - on) / 110.0 {
            0.85
        } else {
            0.1
        };
    }
    1.0
}

/// With the logo, after the scene: the ℵ as the biggest sign over the
/// city: landed, its tubes dark until they flicker on band by band, in
/// yellow with a red glow round them; the rain falling in front of it.
pub(super) fn paint(cv: &mut Canvas, f: &Frame) {
    let done = f.len + GATHER_S;
    let end = done + f.opts.hold;
    if f.t < f.len || f.t >= end {
        return;
    }
    let (x0, y0, lc, lr, cells) = logo_box(f.w, f.h, f.cx, f.cy);
    let own: std::collections::HashSet<(i32, i32)> = cells.iter().map(|c| (c.0, c.1)).collect();
    let mark: Vec<(i32, i32, String, usize, f64)> = cells
        .iter()
        .filter(|c| landed(cv, c.0, c.1))
        .map(|&(x, y, _)| {
            let sym = cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
                .symbol()
                .to_string();
            let fpos = ((x - x0) as f64 / lc as f64 + (y - y0) as f64 / lr as f64) / 2.0;
            (x, y, sym, tube_of(fpos), fpos)
        })
        .collect();
    // the glow first, round the lit tubes, outside the mark
    for (x, y, _, i, _) in &mark {
        let light = tube_light(f, *i);
        if light < 0.3 {
            continue;
        }
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-2, 0), (2, 0)] {
            let (gx, gy) = (x + dx, y + dy);
            if own.contains(&(gx, gy)) {
                continue;
            }
            // beside the tubes, a cell either way; above and below, fainter
            let (g, k) = match (dx.abs(), dy) {
                (1, 0) => ("▒", 0.55),
                (2, _) => ("░", 0.3),
                _ => ("░", 0.42),
            };
            cv.put_with(gx, gy, g, CRIMSON, light * k, false);
        }
    }
    let hum = 0.94 + 0.06 * unit(f.seed ^ 0x4e0, (f.t / 50.0) as u64);
    for (x, y, sym, i, fpos) in &mark {
        let light = tube_light(f, *i);
        let (col, k) = if light <= 0.0 {
            (DEAD, 0.7)
        } else {
            (mix(DEAD, ramp(&TUBE, *fpos), light), light.max(0.5) * hum)
        };
        cv.put_with(*x, *y, sym, col, k, light > 0.5);
    }
    // the rain, in front of it
    let city = City::of(f);
    let lights = Lights::of(f, &city);
    let lit: std::collections::HashSet<(i32, i32)> = mark.iter().map(|m| (m.0, m.1)).collect();
    rain(cv, f, &city, &lights, 0.8, Some(&lit));
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

    fn shot(w: u16, h: u16, u: f64, logo: bool, info: &Info) -> (Buffer, Frame<'_>) {
        let ms = (scene::SCENE_MS as f64 * u) as u64;
        let f = scene::at(w, h, ms, logo, info);
        (scene::dark(Kind::Neon, &f), f)
    }

    fn row(b: &Buffer, y: i32) -> String {
        (0..b.area.width)
            .map(|x| b[(x, y as u16)].symbol())
            .collect()
    }

    fn rgb(b: &Buffer, x: i32, y: i32) -> Option<Rgb> {
        match b[(x as u16, y as u16)].fg {
            Color::Rgb(r, g, bl) => Some((r, g, bl)),
            _ => None,
        }
    }

    /// Is `c` some brightness of `of` (on the dark canvas)?
    fn tint_of(c: Rgb, of: Rgb) -> bool {
        let v = |p: Rgb| [p.0 as f64 - 16.0, p.1 as f64 - 16.0, p.2 as f64 - 22.0];
        let (a, b) = (v(c), v(of));
        let dot: f64 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let n = |p: [f64; 3]| p.iter().map(|x| x * x).sum::<f64>().sqrt();
        dot / (n(a) * n(b)).max(1.0) > 0.98
    }

    #[test]
    fn your_machines_are_signs_horizontal_runs_of_their_names() {
        let info = info(&["normandy", "serenity", "tycho"]);
        // all lit, before one of them starts buzzing
        let (b, f) = shot(120, 32, 0.74, false, &info);
        let city = City::of(&f);
        for name in ["normandy", "serenity", "tycho"] {
            let y = (0..city.ys)
                .find(|&y| row(&b, y).contains(name))
                .unwrap_or_else(|| panic!("{name} isn't up"));
            let s = city
                .signs
                .iter()
                .find(|s| s.text.iter().collect::<String>() == name)
                .unwrap();
            assert!(!s.down && s.machine && s.framed);
            // in its own neon, bold, with its frame round it
            let x = s.x + 2;
            assert_eq!(y, s.y + 1);
            let c = rgb(&b, x, y).unwrap();
            assert!(NEON.contains(&s.col) && tint_of(c, s.col), "{name}: {c:?}");
            assert!(c.0.max(c.1).max(c.2) > 200, "{name} isn't bright: {c:?}");
            assert!(b[(x as u16, y as u16)].modifier.contains(Modifier::BOLD));
            assert!(row(&b, s.y).contains('╭') && row(&b, s.y + 2).contains('╯'));
        }
        // before the signs, dark tubes only: no name lit
        let (b, _) = shot(120, 32, 0.1, false, &info);
        for s in city.signs.iter().filter(|s| s.machine) {
            for j in 0..s.text.len() as i32 {
                let (x, y) = (s.x + 2 + j, s.y + 1);
                let c = rgb(&b, x, y).unwrap();
                assert!(c.0.max(c.1).max(c.2) < 120, "{:?} lit: {c:?}", s.text);
                assert!(!b[(x as u16, y as u16)].modifier.contains(Modifier::BOLD));
            }
        }
    }

    #[test]
    fn each_sign_flickers_on_in_its_own_turn() {
        let info = info(&["normandy", "serenity", "tycho"]);
        let f = scene::at(120, 32, 0, false, &info);
        let city = City::of(&f);
        let mut on: Vec<f64> = city.signs.iter().map(|s| s.on).collect();
        on.sort_by(f64::total_cmp);
        on.dedup();
        assert_eq!(on.len(), city.signs.len(), "each its own moment");
        // a sign's stutter: on and off while it catches, then it holds
        let s = &city.signs[0];
        let lights: Vec<f64> = (0..40)
            .map(|i| {
                let u = s.on + STUTTER * i as f64 / 40.0;
                let f = scene::at(120, 32, (scene::SCENE_MS as f64 * u) as u64, false, &info);
                s.light(&f, 0)
            })
            .collect();
        assert!(lights.iter().any(|&l| l < 0.2) && lights.iter().any(|&l| l > 0.6));
        let f = scene::at(
            120,
            32,
            (scene::SCENE_MS as f64 * (s.on + STUTTER + 0.02)) as u64,
            false,
            &info,
        );
        assert!(s.light(&f, 0) > 0.9);
    }

    #[test]
    fn it_ends_steady_but_for_one_sign_buzzing() {
        let info = info(&["normandy", "serenity", "tycho"]);
        let f = scene::at(120, 32, 0, false, &info);
        let city = City::of(&f);
        let buzz: Vec<usize> = (0..city.signs.len())
            .filter(|&i| city.signs[i].buzz)
            .collect();
        assert_eq!(buzz.len(), 1);
        let low = |i: usize| {
            (0..60).any(|k| {
                let u = BUZZ + (1.0 - BUZZ) * k as f64 / 60.0;
                let f = scene::at(120, 32, (scene::SCENE_MS as f64 * u) as u64, false, &info);
                city.signs[i].light(&f, i) < 0.6
            })
        };
        for i in 0..city.signs.len() {
            assert_eq!(low(i), i == buzz[0], "sign {i}");
        }
    }

    #[test]
    fn the_rain_falls_between_frames() {
        let info = info(&["normandy"]);
        let drops = |ms: u64| {
            let f = scene::at(120, 32, ms, false, &info);
            let b = scene::dark(Kind::Neon, &f);
            (0..32)
                .flat_map(|y| (0..120).map(move |x| (x, y)))
                .filter(|&(x, y)| b[(x as u16, y as u16)].symbol() == "╱")
                .collect::<Vec<_>>()
        };
        let (a, b) = (drops(1500), drops(1540));
        assert!(a.len() > 40 && b.len() > 40, "{} {}", a.len(), b.len());
        assert_ne!(a, b);
        // and none before it sets in
        assert!(drops(60).is_empty());
    }

    #[test]
    fn the_street_mirrors_the_skyline() {
        let info = info(&["normandy", "serenity"]);
        let (b, f) = shot(120, 32, 0.9, false, &info);
        let city = City::of(&f);
        // under each lit sign, its light smeared down the street in its colour
        for s in city.signs.iter().filter(|s| !s.down) {
            let streaks = (city.ys..city.rows)
                .flat_map(|y| (s.x - 1..s.x + s.w() + 1).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    ["┃", "│", "╎"].contains(&b[(x as u16, y as u16)].symbol())
                        && rgb(&b, x, y).is_some_and(|c| tint_of(c, s.col))
                })
                .count();
            assert!(streaks > 6, "{:?}: {streaks}", s.text);
        }
        // the towers' floors at the skyline's foot, mirrored just below it
        let fills: Vec<i32> = (0..120)
            .filter(|&x| b[(x as u16, city.ys as u16 - 1)].symbol() == "▇")
            .collect();
        let mirrored = fills
            .iter()
            .filter(|&&x| {
                (x - 2..=x + 2).any(|xx| b[(xx.max(0) as u16, city.ys as u16)].symbol() == "▀")
            })
            .count();
        assert!(
            fills.len() > 10 && mirrored * 10 >= fills.len() * 7,
            "{mirrored} of {}",
            fills.len()
        );
        // and the street is dimmer than the sky
        let light = |y0: i32, y1: i32| {
            let mut v = Vec::new();
            for y in y0..y1 {
                for x in 0..120 {
                    if let Some(c) = rgb(&b, x, y) {
                        v.push(c.0 as u32 + c.1 as u32 + c.2 as u32);
                    }
                }
            }
            v.iter().sum::<u32>() as f64 / v.len().max(1) as f64
        };
        assert!(light(city.ys, city.rows) < light(0, city.ys));
    }

    #[test]
    fn traffic_crosses_in_lanes_after_the_city_wakes() {
        let info = info(&["normandy"]);
        let cars = |u: f64| {
            let f = scene::at(120, 32, (scene::SCENE_MS as f64 * u) as u64, false, &info);
            let city = City::of(&f);
            let l = Lights::of(&f, &city);
            l.front
                .iter()
                .chain(l.back.iter())
                .filter(|(_, c)| c.is == Is::Car && c.g == '•')
                .map(|(p, _)| *p)
                .collect::<Vec<_>>()
        };
        assert!(cars(0.2).is_empty());
        let (a, b) = (cars(0.8), cars(0.82));
        assert!(!a.is_empty() && a != b, "{a:?} {b:?}");
    }

    #[test]
    fn with_the_logo_the_signs_tubes_light_in_order() {
        let info = info(&["normandy"]);
        let f = scene::at(120, 32, 0, true, &info);
        for i in 1..TUBES {
            assert!(tube_on(&f, i) > tube_on(&f, i - 1));
        }
        let len = scene::SCENE_MS as f64;
        let at = |ms: f64| {
            let f = scene::at(120, 32, ms as u64, true, &info);
            scene::dark(Kind::Neon, &f)
        };
        // part way: the first band lit yellow, the last still dark
        let b = at(tube_on(&f, 1) + 130.0);
        let (x0, y0, lc, lr, cells) = logo_box(120.0, 32.0, f.cx, f.cy);
        let band = |i: usize| {
            cells
                .iter()
                .filter(|c| {
                    let fpos =
                        ((c.0 - x0) as f64 / lc as f64 + (c.1 - y0) as f64 / lr as f64) / 2.0;
                    tube_of(fpos) == i && b[(c.0 as u16, c.1 as u16)].symbol() == "█"
                })
                .map(|c| rgb(&b, c.0, c.1).unwrap())
                .collect::<Vec<_>>()
        };
        let yellow = |c: &Rgb| c.0 > 200 && c.1 > 180 && c.2 < 170;
        let first = band(0);
        assert!(!first.is_empty() && first.iter().all(yellow), "{first:?}");
        assert!(band(TUBES - 1).iter().all(|c| !yellow(c)));
        // held: all of it lit, a red glow round it
        let b = at(len + GATHER_S + 300.0);
        let lit = cells
            .iter()
            .filter(|c| b[(c.0 as u16, c.1 as u16)].symbol() == "█")
            .filter(|c| rgb(&b, c.0, c.1).is_some_and(|c| yellow(&c)))
            .count();
        assert!(lit > 40, "{lit}");
        let red = (0..32)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&(x, y)| ["▒", "░"].contains(&b[(x as u16, y as u16)].symbol()))
            .filter(|&(x, y)| rgb(&b, x, y).is_some_and(|c| tint_of(c, CRIMSON)))
            .count();
        assert!(red > 40, "{red}");
    }

    #[test]
    fn any_fleet_none_one_or_many_draws_on_any_screen() {
        let long: Vec<String> = (0..12)
            .map(|i| format!("a-machine-with-a-name-longer-than-a-small-screen-{i}"))
            .collect();
        let long: Vec<&str> = long.iter().map(String::as_str).collect();
        for info in [info(&[]), info(&["normandy"]), info(&long)] {
            for (w, h) in [(50, 14), (80, 24), (200, 60)] {
                for u in [0.05, 0.2, 0.44, 0.65, 0.8, 1.0] {
                    for logo in [false, true] {
                        let (b, _) = shot(w, h, u, logo, &info);
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
