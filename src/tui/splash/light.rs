//! Light for the cosmos scenes: an additive raster into which a scene adds
//! its glows, gas, filaments and stars as light, then draws it all at once.
//! Two layers:
//!
//! * fine light, at braille-dot resolution (two dots across a cell, four
//!   down: square) — stars, threads, a galaxy's arms, a heart's glow —
//!   drawn as braille: each dot up as the light there is dense (against a
//!   threshold of interleaved gradient noise: an even tone, in no lattice),
//!   each cell coloured by its lit dots' light;
//! * gas, a cell apiece — haze, nebulae, a fireball — drawn as a fine
//!   stipple (`░`) where it's thin, so it reads as an even haze rather than
//!   scattered dots, and in dots with the fine light where it's dense.
//!
//! Light is tone-mapped: its brightness sets how many dots are up, its hue
//! is kept but for the brightest light, which whitens (a hot core), and
//! faint light dims. Coordinates are the scenes' own: cells (`x` across,
//! `y` down, a cell's middle on whole numbers); sizes are in rows (a row is
//! two columns across), so a circle is round.

use super::*;

type Rgb = (u8, u8, u8);

/// Light: red, green, blue. 1 is a full colour; more is brighter still
/// (and whiter, as a channel saturates).
pub(super) type L = [f32; 3];

/// A colour as light, `k` of it.
pub(super) fn lit(rgb: Rgb, k: f64) -> L {
    let k = k as f32 / 255.0;
    [rgb.0 as f32 * k, rgb.1 as f32 * k, rgb.2 as f32 * k]
}

/// Light to brightness, 0 → 1: about 1 − e^−v, so light past 1 saturates.
#[inline]
fn tm(v: f32) -> f32 {
    1.0 - 1.0 / (1.0 + v + 0.5 * v * v)
}

/// Gas is drawn as a stipple between these brightnesses (tone-mapped): in
/// sparse dots fainter (no wall of stipple where it's barely there), in
/// dense dots brighter.
const THIN: f32 = 0.12;
const DENSE: f32 = 0.4;
/// Light whitens between these (its brightest channel).
const WHITE: (f32, f32) = (1.6, 5.0);

pub(super) struct Light {
    cols: i32,
    rows: i32,
    /// dots across and down
    w: i32,
    h: i32,
    px: Vec<L>,
    gas: Vec<L>,
}

impl Light {
    /// Dark, `cols` × `rows` cells.
    pub fn new(cols: i32, rows: i32) -> Light {
        let (cols, rows) = (cols.max(0), rows.max(0));
        let (w, h) = (cols * 2, rows * 4);
        Light {
            cols,
            rows,
            w,
            h,
            px: vec![[0.0; 3]; (w * h) as usize],
            gas: vec![[0.0; 3]; (cols * rows) as usize],
        }
    }

    /// A point (cells) in dots: their middles on whole numbers.
    #[inline]
    fn dot(x: f64, y: f64) -> (f64, f64) {
        (2.0 * x + 0.5, 4.0 * y + 1.5)
    }

    #[inline]
    fn add(&mut self, ix: i32, iy: i32, l: L, k: f32) {
        if ix < 0 || iy < 0 || ix >= self.w || iy >= self.h {
            return;
        }
        let p = &mut self.px[(iy * self.w + ix) as usize];
        p[0] += l[0] * k;
        p[1] += l[1] * k;
        p[2] += l[2] * k;
    }

    /// A point of light: the one dot it's on.
    pub fn point(&mut self, x: f64, y: f64, l: L) {
        let (dx, dy) = Light::dot(x, y);
        self.add(dx.round() as i32, dy.round() as i32, l, 1.0);
    }

    /// A soft blob: a Gaussian `r` rows across (its sigma), `l` at its
    /// middle.
    pub fn splat(&mut self, x: f64, y: f64, r: f64, l: L) {
        let (dx, dy) = Light::dot(x, y);
        let s = (r * 4.0) as f32;
        if s < 0.4 {
            self.add(dx.round() as i32, dy.round() as i32, l, 1.0);
            return;
        }
        let rad = (s * 2.4).ceil() as i32;
        let inv = 1.0 / (2.0 * s * s);
        let (ix, iy) = (dx.round() as i32, dy.round() as i32);
        for y in (iy - rad).max(0)..=(iy + rad).min(self.h - 1) {
            let ddy = y as f32 - dy as f32;
            for x in (ix - rad).max(0)..=(ix + rad).min(self.w - 1) {
                let ddx = x as f32 - dx as f32;
                let k = (-(ddx * ddx + ddy * ddy) * inv).exp();
                self.add(x, y, l, k);
            }
        }
    }

    /// A glow round a point: bright at its heart, with the long soft fall
    /// of light scattered (1 / (1 + (d/r)²)^1.5), out to `reach` rows.
    pub fn halo(&mut self, x: f64, y: f64, r: f64, reach: f64, l: L) {
        let (dx, dy) = Light::dot(x, y);
        let (s, rad) = ((r * 4.0).max(0.3) as f32, (reach * 4.0).ceil() as i32);
        let inv = 1.0 / (s * s);
        let cut = (rad * rad) as f32;
        let (ix, iy) = (dx.round() as i32, dy.round() as i32);
        for y in (iy - rad).max(0)..=(iy + rad).min(self.h - 1) {
            let ddy = y as f32 - dy as f32;
            for x in (ix - rad).max(0)..=(ix + rad).min(self.w - 1) {
                let ddx = x as f32 - dx as f32;
                let d2 = ddx * ddx + ddy * ddy;
                if d2 > cut {
                    continue;
                }
                let q = 1.0 / (1.0 + d2 * inv);
                // fading to nothing at its reach
                let edge = 1.0 - d2 / cut;
                self.add(x, y, l, q * q.sqrt() * edge);
            }
        }
    }

    /// A thin line of light from `(x, y)` out along `a` (radians, on a
    /// round screen: 0 right, π/2 down) for `len` rows, fading as `fall`
    /// goes (its light at a share `s` of the way).
    pub fn ray(&mut self, x: f64, y: f64, a: f64, len: f64, l: L, fall: impl Fn(f64) -> f64) {
        let (ca, sa) = (a.cos(), a.sin());
        // a step of half a dot
        let n = (len * 8.0).ceil().max(1.0) as i32;
        for i in 1..=n {
            let s = i as f64 / n as f64;
            let d = s * len;
            let k = fall(s) as f32;
            if k > 0.0 {
                let (dx, dy) = Light::dot(x + ca * d * 2.0, y + sa * d);
                self.add(dx.round() as i32, dy.round() as i32, l, k);
            }
        }
    }

    /// A smooth field of fine light over cells `x0..=x1`, `y0..=y1`: `f` (a
    /// point in cells) read on a grid of every other column and every row
    /// (square: a row is two columns), each dot between the four grid
    /// points round it. For broad glows, cheaply.
    pub fn wash(
        &mut self,
        (x0, x1): (i32, i32),
        (y0, y1): (i32, i32),
        mut f: impl FnMut(f64, f64) -> L,
    ) {
        let (x0, x1) = (x0.max(0), x1.min(self.cols - 1));
        let (y0, y1) = (y0.max(0), y1.min(self.rows - 1));
        if x0 > x1 || y0 > y1 {
            return;
        }
        // a step beyond the box all round, for the dots at its edges
        let (gx0, gy0) = (x0 - 2, y0 - 1);
        let gw = ((x1 - x0) / 2 + 4) as usize;
        let gh = (y1 - y0 + 3) as usize;
        let mut g: Vec<L> = Vec::with_capacity(gw * gh);
        for gy in 0..gh {
            for gx in 0..gw {
                g.push(f((gx0 + 2 * gx as i32) as f64, (gy0 + gy as i32) as f64));
            }
        }
        for iy in y0 * 4..(y1 + 1) * 4 {
            let yd = iy as f32 / 4.0 - 0.375 - gy0 as f32;
            let gy = yd.floor() as usize;
            let ty = yd - gy as f32;
            let row = (iy * self.w) as usize;
            for ix in x0 * 2..(x1 + 1) * 2 {
                let xd = (ix as f32 / 2.0 - 0.25 - gx0 as f32) / 2.0;
                let gx = xd.floor() as usize;
                let tx = xd - gx as f32;
                let (a, b) = (g[gy * gw + gx], g[gy * gw + gx + 1]);
                let (c, d) = (g[(gy + 1) * gw + gx], g[(gy + 1) * gw + gx + 1]);
                let p = &mut self.px[row + ix as usize];
                for ch in 0..3 {
                    let top = a[ch] + (b[ch] - a[ch]) * tx;
                    let bot = c[ch] + (d[ch] - c[ch]) * tx;
                    p[ch] += top + (bot - top) * ty;
                }
            }
        }
    }

    /// Gas over cells `x0..=x1`, `y0..=y1`: `f` (a point in cells) read at
    /// every other column of every row, the columns between halfway.
    pub fn gas(
        &mut self,
        (x0, x1): (i32, i32),
        (y0, y1): (i32, i32),
        mut f: impl FnMut(f64, f64) -> L,
    ) {
        let (x0, x1) = (x0.max(0), x1.min(self.cols - 1));
        let (y0, y1) = (y0.max(0), y1.min(self.rows - 1));
        if x0 > x1 || y0 > y1 {
            return;
        }
        for y in y0..=y1 {
            let row = (y * self.cols) as usize;
            let mut here = f(x0 as f64, y as f64);
            let mut x = x0;
            while x <= x1 {
                let next = f((x + 2) as f64, y as f64);
                let g = &mut self.gas[row + x as usize];
                for ch in 0..3 {
                    g[ch] += here[ch];
                }
                if x < x1 {
                    let g = &mut self.gas[row + x as usize + 1];
                    for ch in 0..3 {
                        g[ch] += (here[ch] + next[ch]) * 0.5;
                    }
                }
                here = next;
                x += 2;
            }
        }
    }

    /// A field of light read at every dot of cells `x0..=x1`, `y0..=y1`:
    /// for fine light (a galaxy's arms, a star's heart).
    pub fn field(
        &mut self,
        (x0, x1): (i32, i32),
        (y0, y1): (i32, i32),
        mut f: impl FnMut(f64, f64) -> L,
    ) {
        let (x0, x1) = (x0.max(0), x1.min(self.cols - 1));
        let (y0, y1) = (y0.max(0), y1.min(self.rows - 1));
        for iy in y0 * 4..(y1 + 1).max(y0) * 4 {
            let y = iy as f64 / 4.0 - 0.375;
            for ix in x0 * 2..(x1 + 1).max(x0) * 2 {
                let l = f(ix as f64 / 2.0 - 0.25, y);
                if l[0] + l[1] + l[2] > 0.0 {
                    self.add(ix, iy, l, 1.0);
                }
            }
        }
    }

    /// Every light × `k`.
    pub fn scale(&mut self, k: f32) {
        for p in self.px.iter_mut().chain(self.gas.iter_mut()) {
            p[0] *= k;
            p[1] *= k;
            p[2] *= k;
        }
    }

    /// The light drawn. A cell's dots up as its fine light is dense (with
    /// its gas, where that's dense too), each against a threshold of
    /// interleaved gradient noise (`frame` moves it, for gas that boils),
    /// coloured by its lit dots' light and the gas round them; else its
    /// thin gas as a stipple. No colour (NO_COLOR): only the brighter
    /// light, so shapes still read.
    pub fn draw(&self, cv: &mut Canvas, frame: u64) {
        const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
        let shift = (frame % 64) as f32 * 5.588_238;
        let ign = |x: i32, y: i32| {
            let v = 0.067_110_56 * (x as f32 + shift) + 0.005_837_15 * (y as f32 + shift);
            (52.982_918 * v.fract()).fract()
        };
        // a colour and its brightness from light (tone-mapped)
        // a colour and its brightness from light: its hue kept, whitening
        // only as it gets really bright (a hot core, not a bright line)
        let shade = |l: L| {
            let m = l[0].max(l[1]).max(l[2]).max(1e-4);
            let white = 0.9 * ((m - WHITE.0) / (WHITE.1 - WHITE.0)).clamp(0.0, 1.0);
            let ch = |v: f32| ((v / m + (1.0 - v / m) * white) * 255.0) as u8;
            ((ch(l[0]), ch(l[1]), ch(l[2])), tm(m))
        };
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                let g = self.gas[(cy * self.cols + cx) as usize];
                let gv = g[0].max(g[1]).max(g[2]);
                let mut any = gv > 0.01;
                for j in 0..4 {
                    if any {
                        break;
                    }
                    let at = ((cy * 4 + j) * self.w + cx * 2) as usize;
                    let (a, b) = (self.px[at], self.px[at + 1]);
                    any = a[0] + a[1] + a[2] + b[0] + b[1] + b[2] > 0.01;
                }
                if !any {
                    continue;
                }
                let gl = tm(gv);
                // faint gas and dense gas are drawn in dots; between, a stipple
                let dots = !(THIN..DENSE).contains(&gl);
                let (mut bits, mut sum, mut n) = (0u32, [0f32; 3], 0f32);
                for (i, col) in BITS.iter().enumerate() {
                    for (j, bit) in col.iter().enumerate() {
                        let (ix, iy) = (cx * 2 + i as i32, cy * 4 + j as i32);
                        let mut p = self.px[(iy * self.w + ix) as usize];
                        if dots {
                            for ch in 0..3 {
                                p[ch] += g[ch];
                            }
                        }
                        let v = p[0].max(p[1]).max(p[2]);
                        if v < 0.004 {
                            continue;
                        }
                        let b = tm(v);
                        let cover = if cv.mono {
                            ((b - 0.3) / 0.45).clamp(0.0, 1.0)
                        } else {
                            b
                        };
                        if cover > ign(ix, iy) {
                            bits |= bit;
                            // a lit dot in thin gas takes a little of its colour
                            let k = if dots { 0.0 } else { 0.6 };
                            for ch in 0..3 {
                                sum[ch] += p[ch] + g[ch] * k;
                            }
                            n += 1.0;
                        }
                    }
                }
                let mut b = [0u8; 4];
                if bits != 0 {
                    let (rgb, k) = shade([sum[0] / n, sum[1] / n, sum[2] / n]);
                    let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                    cv.put_with(
                        cx,
                        cy,
                        ch.encode_utf8(&mut b),
                        rgb,
                        (k as f64).sqrt(),
                        false,
                    );
                } else if !dots && !(cv.mono && gl < 0.3) {
                    // thin gas: an even stipple, as bright as it is
                    let (rgb, _) = shade(g);
                    let k = 0.3 + 0.32 * ((gl - THIN) / (DENSE - THIN));
                    cv.put_with(cx, cy, "░", rgb, k as f64, false);
                }
            }
        }
    }

    /// How bright the light is in cell `(x, y)` (its brightest dot, or its
    /// gas), 0 → 1.
    pub fn level(&self, x: i32, y: i32) -> f64 {
        if x < 0 || y < 0 || x >= self.cols || y >= self.rows {
            return 0.0;
        }
        let g = self.gas[(y * self.cols + x) as usize];
        let mut top = g[0].max(g[1]).max(g[2]);
        for j in 0..4 {
            for i in 0..2 {
                let p = self.px[((y * 4 + j) * self.w + x * 2 + i) as usize];
                top = top.max(p[0].max(p[1]).max(p[2]));
            }
        }
        tm(top) as f64
    }
}

/// A star as a glyph, as bright as `k` (0 → 1): a dot, a bigger one, a
/// four-pointed sparkle (all one column wide, none East Asian ambiguous).
pub(super) fn star(cv: &mut Canvas, x: f64, y: f64, rgb: Rgb, k: f64) {
    let g = match k {
        k if k > 0.82 => "✦",
        k if k > 0.6 => "✧",
        k if k > 0.36 => "∙",
        _ => "⋅",
    };
    cv.put_with(
        x.round() as i32,
        y.round() as i32,
        g,
        rgb,
        k.min(1.0),
        k > 0.82,
    );
}

/// Value noise: smooth random hills a unit apart, from a seed.
pub(super) struct Noise {
    v: [f32; 256],
    p: [u8; 256],
}

impl Noise {
    pub fn new(seed: u64) -> Noise {
        let mut v = [0f32; 256];
        let mut p = [0u8; 256];
        for i in 0..256 {
            v[i] = unit(seed ^ 0x4e01, i as u64) as f32;
            p[i] = i as u8;
        }
        for i in (1..256).rev() {
            p.swap(i, (hash(seed ^ 0x4e02, i as u64) % (i as u64 + 1)) as usize);
        }
        Noise { v, p }
    }

    #[inline]
    fn lat(&self, i: i32, j: i32) -> f32 {
        let a = self.p[(i & 255) as usize] as i32;
        self.v[self.p[((a + j) & 255) as usize] as usize]
    }

    /// 0 → 1, smooth.
    #[inline]
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let (fx, fy) = (x.floor(), y.floor());
        let (i, j) = (fx as i32, fy as i32);
        let (tx, ty) = ((x - fx) as f32, (y - fy) as f32);
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let (a, b) = (self.lat(i, j), self.lat(i + 1, j));
        let (c, d) = (self.lat(i, j + 1), self.lat(i + 1, j + 1));
        (a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy) as f64
    }

    /// `oct` octaves of it, each twice as fine and half as strong (turned a
    /// little, so no lattice shows): 0 → 1.
    pub fn fbm(&self, x: f64, y: f64, oct: u32) -> f64 {
        let (mut x, mut y) = (x, y);
        let (mut sum, mut amp, mut tot) = (0.0, 1.0, 0.0);
        for _ in 0..oct {
            sum += self.at(x, y) * amp;
            tot += amp;
            amp *= 0.5;
            (x, y) = (x * 1.6 - y * 1.2 + 19.7, x * 1.2 + y * 1.6 - 7.3);
        }
        sum / tot
    }

    /// Ridges: 1 along the lines where it crosses its middle, falling away
    /// either side: filaments.
    pub fn ridge(&self, x: f64, y: f64, oct: u32) -> f64 {
        let (mut x, mut y) = (x, y);
        let (mut sum, mut amp, mut tot) = (0.0, 1.0, 0.0);
        for _ in 0..oct {
            let r = 1.0 - (2.0 * self.at(x, y) - 1.0).abs();
            sum += r * r * amp;
            tot += amp;
            amp *= 0.5;
            (x, y) = (x * 1.7 - y * 1.1 + 3.1, x * 1.1 + y * 1.7 + 11.9);
        }
        sum / tot
    }
}
