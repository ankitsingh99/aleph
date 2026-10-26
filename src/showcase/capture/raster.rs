//! A recording's frames as pixels: an RGB canvas, the computer's own
//! monospace font (rasterised by fontdue), and the poster's rule that box
//! drawing (light, heavy, double, rounded, dashed), block elements, shades,
//! quadrants and Braille are shapes in cell geometry, never glyphs — panels
//! close and bars are solid at any size.
//!
//! * fonts: `showcase.video_font` (a path), else the system's monospace —
//!   Menlo on macOS (its collection's regular, bold and italic faces),
//!   `fc-match monospace` on Linux; what it lacks comes from the symbol
//!   fonts already there (a Nerd Font, Apple Symbols, Arial Unicode, DejaVu,
//!   Noto), loaded only when a character needs one; what none has (colour
//!   emoji) is a small box;
//! * every glyph is clipped to its cell (two for a wide one), so a frame can
//!   be drawn again cell by cell.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type Rgb = (u8, u8, u8);

/// `a` toward `b` by `k` (0: a, 1: b).
pub fn mix(a: Rgb, b: Rgb, k: f32) -> Rgb {
    let m = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * k)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// `#rrggbb` → its colour.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((v(0)?, v(2)?, v(4)?))
}

// ------------------------------------------------------------------ canvas

/// A rectangle drawing stays inside: [x0, x1) × [y0, y1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Clip {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Clip {
        Clip { x0, y0, x1, y1 }
    }

    pub fn and(self, o: Clip) -> Clip {
        Clip::new(
            self.x0.max(o.x0),
            self.y0.max(o.y0),
            self.x1.min(o.x1),
            self.y1.min(o.y1),
        )
    }
}

/// Pixels, RGB, row by row (what ffmpeg's `rgb24` reads).
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// `p` over by `c` at `a` (0–256).
#[inline]
fn over(p: &mut [u8], c: Rgb, a: u32) {
    let k = 256 - a;
    p[0] = ((p[0] as u32 * k + c.0 as u32 * a) >> 8) as u8;
    p[1] = ((p[1] as u32 * k + c.1 as u32 * a) >> 8) as u8;
    p[2] = ((p[2] as u32 * k + c.2 as u32 * a) >> 8) as u8;
}

fn alpha(a: f32) -> u32 {
    (a.clamp(0.0, 1.0) * 256.0).round() as u32
}

impl Canvas {
    pub fn new(w: usize, h: usize, c: Rgb) -> Canvas {
        let mut px = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            px.extend_from_slice(&[c.0, c.1, c.2]);
        }
        Canvas { w, h, px }
    }

    pub fn all(&self) -> Clip {
        Clip::new(0, 0, self.w as i32, self.h as i32)
    }

    #[cfg(test)]
    pub fn at(&self, x: usize, y: usize) -> Rgb {
        let i = (y * self.w + x) * 3;
        (self.px[i], self.px[i + 1], self.px[i + 2])
    }

    fn clipped(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<(usize, usize, usize, usize)> {
        let c = Clip::new(x0, y0, x1, y1).and(self.all());
        (c.x0 < c.x1 && c.y0 < c.y1).then_some((
            c.x0 as usize,
            c.y0 as usize,
            c.x1 as usize,
            c.y1 as usize,
        ))
    }

    /// Fill [x0, x1) × [y0, y1).
    pub fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
        let Some((x0, y0, x1, y1)) = self.clipped(x0, y0, x1, y1) else {
            return;
        };
        let rgb = [c.0, c.1, c.2];
        for y in y0..y1 {
            let row = &mut self.px[(y * self.w + x0) * 3..(y * self.w + x1) * 3];
            for p in row.as_chunks_mut::<3>().0 {
                *p = rgb;
            }
        }
    }

    /// [x0, x1) × [y0, y1) under `c` at opacity `a`.
    pub fn blend(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb, a: f32) {
        let a = alpha(a);
        if a >= 256 {
            return self.rect(x0, y0, x1, y1, c);
        }
        let Some((x0, y0, x1, y1)) = self.clipped(x0, y0, x1, y1) else {
            return;
        };
        for y in y0..y1 {
            let row = &mut self.px[(y * self.w + x0) * 3..(y * self.w + x1) * 3];
            for p in row.as_chunks_mut::<3>().0 {
                over(p, c, a);
            }
        }
    }

    /// One pixel under `c` at `a` (0–1).
    #[inline]
    fn dot(&mut self, x: i32, y: i32, c: Rgb, a: f32, clip: Clip) {
        if x < clip.x0 || y < clip.y0 || x >= clip.x1 || y >= clip.y1 || a <= 0.0 {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 3;
        over(&mut self.px[i..i + 3], c, alpha(a));
    }

    /// A coverage mask (a glyph's) in `c`, its top left at (x, y).
    pub fn mask(&mut self, x: i32, y: i32, w: usize, cov: &[u8], c: Rgb, clip: Clip) {
        if w == 0 {
            return;
        }
        let clip = clip.and(self.all());
        for (r, row) in cov.chunks_exact(w).enumerate() {
            let py = y + r as i32;
            if py < clip.y0 || py >= clip.y1 {
                continue;
            }
            for (k, &a) in row.iter().enumerate() {
                let px = x + k as i32;
                if a == 0 || px < clip.x0 || px >= clip.x1 {
                    continue;
                }
                let i = (py as usize * self.w + px as usize) * 3;
                over(&mut self.px[i..i + 3], c, a as u32 + (a as u32 >> 7));
            }
        }
    }

    /// A filled disc, antialiased.
    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, clip: Clip) {
        let (x0, x1) = ((cx - r - 1.0).floor() as i32, (cx + r + 1.0).ceil() as i32);
        let (y0, y1) = ((cy - r - 1.0).floor() as i32, (cy + r + 1.0).ceil() as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                self.dot(x, y, c, r + 0.5 - d, clip);
            }
        }
    }

    /// A quarter of a ring (radius `r` to its line's middle, `t` thick),
    /// antialiased: the quarter on the `(sx, sy)` side of its centre (−1
    /// left or up, +1 right or down).
    pub fn ring(
        &mut self,
        (cx, cy): (f32, f32),
        r: f32,
        t: f32,
        (sx, sy): (i8, i8),
        c: Rgb,
        clip: Clip,
    ) {
        let reach = r + t;
        let (x0, x1) = if sx < 0 {
            ((cx - reach).floor() as i32, cx.ceil() as i32 + 1)
        } else {
            (cx.floor() as i32 - 1, (cx + reach).ceil() as i32)
        };
        let (y0, y1) = if sy < 0 {
            ((cy - reach).floor() as i32, cy.ceil() as i32 + 1)
        } else {
            (cy.floor() as i32 - 1, (cy + reach).ceil() as i32)
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * (sx as f32) < -0.5 || dy * (sy as f32) < -0.5 {
                    continue;
                }
                let d = (dx * dx + dy * dy).sqrt();
                self.dot(x, y, c, t / 2.0 + 0.5 - (d - r).abs(), clip);
            }
        }
    }

    /// A rounded rectangle, antialiased, at opacity `a`.
    #[allow(clippy::too_many_arguments)]
    pub fn rrect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, c: Rgb, a: f32) {
        let clip = self.all();
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hw, hh) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        let r = r.min(hw).min(hh);
        for y in y0.floor() as i32..y1.ceil() as i32 {
            for x in x0.floor() as i32..x1.ceil() as i32 {
                // the signed distance to the rounded box
                let qx = (x as f32 + 0.5 - cx).abs() - (hw - r);
                let qy = (y as f32 + 0.5 - cy).abs() - (hh - r);
                let out = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
                let d = out + qx.max(qy).min(0.0) - r;
                self.dot(x, y, c, (0.5 - d).clamp(0.0, 1.0) * a, clip);
            }
        }
    }

    /// Polygons filled (even-odd), antialiased: five samples a row, exact
    /// across it.
    pub fn fill(&mut self, polys: &[Vec<(f32, f32)>], c: Rgb, a: f32, clip: Clip) {
        let clip = clip.and(self.all());
        let pts = polys.iter().flatten();
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for &(x, y) in pts {
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let x0 = (lo.0.floor() as i32).max(clip.x0);
        let x1 = (hi.0.ceil() as i32 + 1).min(clip.x1);
        let y0 = (lo.1.floor() as i32).max(clip.y0);
        let y1 = (hi.1.ceil() as i32).min(clip.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        const S: usize = 5;
        let n = (x1 - x0) as usize;
        let mut row = vec![0f32; n + 1];
        let mut xs: Vec<f32> = Vec::new();
        for py in y0..y1 {
            row.iter_mut().for_each(|v| *v = 0.0);
            for k in 0..S {
                let y = py as f32 + (k as f32 + 0.5) / S as f32;
                xs.clear();
                for p in polys {
                    for i in 0..p.len() {
                        let (a0, b0) = (p[i], p[(i + 1) % p.len()]);
                        if (a0.1 <= y) != (b0.1 <= y) {
                            xs.push(a0.0 + (y - a0.1) * (b0.0 - a0.0) / (b0.1 - a0.1));
                        }
                    }
                }
                xs.sort_by(f32::total_cmp);
                for pair in xs.as_chunks::<2>().0 {
                    let (l, r) = (pair[0] - x0 as f32, pair[1] - x0 as f32);
                    span(&mut row, l.max(0.0), r.min(n as f32), 1.0 / S as f32);
                }
            }
            for (i, &v) in row[..n].iter().enumerate() {
                if v > 0.0 {
                    self.dot(x0 + i as i32, py, c, v.min(1.0) * a, clip);
                }
            }
        }
    }
}

/// Coverage `w` added to `row` over [l, r) (fractions at the ends).
fn span(row: &mut [f32], l: f32, r: f32, w: f32) {
    if r <= l {
        return;
    }
    let (i0, i1) = (l.floor() as usize, r.floor() as usize);
    if i0 == i1 {
        row[i0] += (r - l) * w;
        return;
    }
    row[i0] += (i0 as f32 + 1.0 - l) * w;
    for v in &mut row[i0 + 1..i1] {
        *v += w;
    }
    if i1 < row.len() {
        row[i1] += (r - i1 as f32) * w;
    }
}

/// An SVG path's outlines (absolute M, L, H, V, C and Z only — the mark's),
/// curves flattened, scaled by `k` and moved to (x, y).
pub fn path(d: &str, k: f32, (x, y): (f32, f32)) -> Vec<Vec<(f32, f32)>> {
    let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let toks: Vec<String> = {
        let mut v = Vec::new();
        let mut num = String::new();
        for ch in d.chars() {
            if ch.is_ascii_alphabetic() {
                if !num.is_empty() {
                    v.push(std::mem::take(&mut num));
                }
                v.push(ch.to_string());
            } else if ch == ',' || ch.is_whitespace() || (ch == '-' && !num.is_empty()) {
                if !num.is_empty() {
                    v.push(std::mem::take(&mut num));
                }
                if ch == '-' {
                    num.push(ch);
                }
            } else {
                num.push(ch);
            }
        }
        if !num.is_empty() {
            v.push(num);
        }
        v
    };
    let mut i = 0;
    let mut cmd = 'M';
    let mut at = (0f32, 0f32);
    let num = |i: &mut usize| -> f32 {
        let v = toks.get(*i).and_then(|t| t.parse().ok()).unwrap_or(0.0);
        *i += 1;
        v
    };
    let put = |p: (f32, f32)| (x + p.0 * k, y + p.1 * k);
    while i < toks.len() {
        if let Some(c) = toks[i].chars().next().filter(|c| c.is_ascii_alphabetic()) {
            cmd = c;
            i += 1;
            if c == 'Z' || c == 'z' {
                if cur.len() > 2 {
                    polys.push(std::mem::take(&mut cur));
                }
                cur.clear();
                continue;
            }
        }
        match cmd {
            'M' => {
                if cur.len() > 2 {
                    polys.push(std::mem::take(&mut cur));
                }
                cur.clear();
                at = (num(&mut i), num(&mut i));
                cur.push(put(at));
                cmd = 'L';
            }
            'L' => {
                at = (num(&mut i), num(&mut i));
                cur.push(put(at));
            }
            'H' => {
                at.0 = num(&mut i);
                cur.push(put(at));
            }
            'V' => {
                at.1 = num(&mut i);
                cur.push(put(at));
            }
            'C' => {
                let (c1, c2) = ((num(&mut i), num(&mut i)), (num(&mut i), num(&mut i)));
                let to = (num(&mut i), num(&mut i));
                for s in 1..=12 {
                    let t = s as f32 / 12.0;
                    let u = 1.0 - t;
                    let b = |a: f32, b: f32, c: f32, d: f32| {
                        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
                    };
                    cur.push(put((b(at.0, c1.0, c2.0, to.0), b(at.1, c1.1, c2.1, to.1))));
                }
                at = to;
            }
            _ => i += 1,
        }
    }
    if cur.len() > 2 {
        polys.push(cur);
    }
    polys
}

// ------------------------------------------------------------------ fonts

/// A face: a font file and its index in a collection.
pub type FaceAt = (PathBuf, u32);

/// Where the faces come from: regular, bold, italic, bold italic (only the
/// regular is a must), then the fonts asked when those lack a character.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontSpec {
    pub faces: [Option<FaceAt>; 4],
    pub fallbacks: Vec<FaceAt>,
}

fn be16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// Where face `index` starts in a font file (a collection's offset table).
fn face_at(d: &[u8], index: u32) -> Option<usize> {
    if d.get(0..4) == Some(b"ttcf") {
        (index < be32(d, 8)?).then_some(())?;
        return Some(be32(d, 12 + index as usize * 4)? as usize);
    }
    (index == 0).then_some(0)
}

/// Where a face's table starts.
fn table(d: &[u8], face: usize, tag: &[u8; 4]) -> Option<usize> {
    let n = be16(d, face + 4)? as usize;
    (0..n.min(256)).find_map(|t| {
        let rec = face + 12 + t * 16;
        (d.get(rec..rec + 4)? == tag).then_some(())?;
        Some(be32(d, rec + 8)? as usize)
    })
}

/// A font file's faces: (index, bold, italic), from each face's `head`
/// table (`macStyle`). One for a plain font, every one for a collection.
pub fn faces(data: &[u8]) -> Vec<(u32, bool, bool)> {
    let style = |i: u32| -> Option<(bool, bool)> {
        let at = table(data, face_at(data, i)?, b"head")?;
        let ms = be16(data, at + 44)?;
        Some((ms & 1 != 0, ms & 2 != 0))
    };
    let n = if data.get(0..4) == Some(b"ttcf") {
        be32(data, 8).unwrap_or(0).min(64)
    } else {
        1
    };
    let v: Vec<(u32, bool, bool)> = (0..n)
        .filter_map(|i| style(i).map(|(b, it)| (i, b, it)))
        .collect();
    if v.is_empty() {
        vec![(0, false, false)]
    } else {
        v
    }
}

/// A face's Unicode character map: its `cmap` subtable of format 12 (all
/// of Unicode) or 4 (the BMP). Read by hand: fontdue merges every subtable,
/// and a Mac Roman one (Menlo has it) turns `°` into `∞` and `·` into `∑`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cmap {
    format: u16,
    sub: Vec<u8>,
}

impl Cmap {
    pub fn read(d: &[u8], index: u32) -> Option<Cmap> {
        let cmap = table(d, face_at(d, index)?, b"cmap")?;
        let n = be16(d, cmap + 2)? as usize;
        let mut best: Option<(u8, usize, u16)> = None;
        for i in 0..n.min(64) {
            let rec = cmap + 4 + i * 8;
            let (pid, eid) = (be16(d, rec)?, be16(d, rec + 2)?);
            let at = cmap + be32(d, rec + 4)? as usize;
            let format = be16(d, at)?;
            let unicode = pid == 0 || (pid == 3 && matches!(eid, 1 | 10));
            let rank = match format {
                12 if unicode => 2,
                4 if unicode => 1,
                _ => 0,
            };
            if rank > 0 && best.is_none_or(|b| rank > b.0) {
                best = Some((rank, at, format));
            }
        }
        let (_, at, format) = best?;
        let len = if format == 12 {
            be32(d, at + 4)? as usize
        } else {
            be16(d, at + 2)? as usize
        };
        Some(Cmap {
            format,
            sub: d.get(at..at + len)?.to_vec(),
        })
    }

    /// The glyph for `c` (0: none).
    pub fn get(&self, c: char) -> u16 {
        self.find(c as u32).unwrap_or(0)
    }

    fn find(&self, u: u32) -> Option<u16> {
        let b = &self.sub;
        if self.format == 12 {
            let n = be32(b, 12)? as usize;
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let g = 16 + mid * 12;
                let (s, e) = (be32(b, g)?, be32(b, g + 4)?);
                if u < s {
                    hi = mid;
                } else if u > e {
                    lo = mid + 1;
                } else {
                    return Some((be32(b, g + 8)? + (u - s)) as u16);
                }
            }
            return Some(0);
        }
        if u > 0xffff {
            return Some(0);
        }
        let segs = be16(b, 6)? as usize; // ×2
        let (ends, starts) = (14, 16 + segs);
        let (deltas, ranges) = (starts + segs, starts + 2 * segs);
        for s in 0..segs / 2 {
            if u > be16(b, ends + 2 * s)? as u32 {
                continue;
            }
            let start = be16(b, starts + 2 * s)? as u32;
            if u < start {
                return Some(0);
            }
            let delta = be16(b, deltas + 2 * s)?;
            let ro = be16(b, ranges + 2 * s)? as usize;
            if ro == 0 {
                return Some((u as u16).wrapping_add(delta));
            }
            let g = be16(b, ranges + 2 * s + ro + 2 * (u - start) as usize)?;
            return Some(if g == 0 { 0 } else { g.wrapping_add(delta) });
        }
        Some(0)
    }
}

/// The faces of one file put in their places (a collection's by style; a
/// plain font's `…Regular…` siblings: `…Bold…`, `…Italic…`, `…BoldItalic…`).
fn family(path: &Path) -> [Option<FaceAt>; 4] {
    let mut out: [Option<FaceAt>; 4] = Default::default();
    let Ok(data) = std::fs::read(path) else {
        return out;
    };
    let fs = faces(&data);
    if data.get(0..4) == Some(b"ttcf") {
        for (i, b, it) in fs {
            let slot = b as usize | (it as usize) << 1;
            if out[slot].is_none() {
                out[slot] = Some((path.to_path_buf(), i));
            }
        }
        if out[0].is_none() {
            out[0] = Some((path.to_path_buf(), 0));
        }
        return out;
    }
    out[0] = Some((path.to_path_buf(), 0));
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.contains("Regular") {
        for (slot, word) in [(1, "Bold"), (2, "Italic"), (3, "BoldItalic")] {
            let p = path.with_file_name(name.replace("Regular", word));
            if p.is_file() {
                out[slot] = Some((p, 0));
            }
        }
    }
    out
}

/// `fc-match`'s file and index for a pattern.
fn fc_match(pattern: &str) -> Option<FaceAt> {
    crate::proc::which("fc-match")?;
    let o = crate::proc::cap(
        "fc-match",
        &["-f", "%{file}|%{index}", pattern],
        std::time::Duration::from_secs(5),
    );
    let t = o.text();
    let (f, i) = t.trim().split_once('|')?;
    let p = PathBuf::from(f);
    p.is_file().then(|| (p, i.trim().parse().unwrap_or(0)))
}

/// The fonts a frame is drawn with: `setting` (a path; empty: the system's
/// monospace) and the symbol fonts already on this computer.
pub fn find(setting: &str) -> Result<FontSpec, String> {
    let mut spec = FontSpec::default();
    let s = setting.trim();
    if !s.is_empty() {
        let p = crate::config::expand(s);
        if !p.is_file() {
            return Err(format!("showcase.video_font: no font at {}", p.display()));
        }
        spec.faces = family(&p);
    } else if cfg!(target_os = "macos") {
        for p in [
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/SFNSMono.ttf",
            "/System/Library/Fonts/Monaco.ttf",
        ] {
            let p = Path::new(p);
            if p.is_file() {
                spec.faces = family(p);
                break;
            }
        }
    } else {
        let reg = fc_match("monospace");
        let same = |f: &Option<FaceAt>| f.is_some() && *f == reg;
        let bold = fc_match("monospace:bold");
        let italic = fc_match("monospace:italic");
        let both = fc_match("monospace:bold:italic");
        spec.faces = [
            reg.clone(),
            bold.clone().filter(|_| !same(&bold)),
            italic.clone().filter(|_| !same(&italic)),
            both.clone().filter(|_| !same(&both)),
        ];
    }
    if spec.faces[0].is_none() {
        return Err(if cfg!(target_os = "macos") {
            "no monospace font found (Menlo.ttc) — set showcase.video_font to a .ttf".into()
        } else {
            "no monospace font found (fc-match monospace) — set showcase.video_font to a .ttf"
                .into()
        });
    }
    spec.fallbacks = fallbacks();
    spec.fallbacks
        .retain(|f| !spec.faces.iter().flatten().any(|g| g == f));
    Ok(spec)
}

/// Fonts for what the monospace lacks, in the order they're asked: a Nerd
/// Font's icons, then symbols, then everything else.
fn fallbacks() -> Vec<FaceAt> {
    let mut v: Vec<FaceAt> = Vec::new();
    let home = crate::config::home();
    let nerd_dirs = if cfg!(target_os = "macos") {
        vec![home.join("Library/Fonts"), PathBuf::from("/Library/Fonts")]
    } else {
        vec![home.join(".local/share/fonts"), home.join(".fonts")]
    };
    // a Nerd Font's monospaced icons (the dashboard's, when it uses them)
    let mut nerd: Vec<PathBuf> = nerd_dirs
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| {
            let n = p
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            n.contains("NerdFont") && n.contains("Mono") && n.contains("Regular")
        })
        .collect();
    nerd.sort_by_key(|p| !p.to_string_lossy().contains("Symbols"));
    v.extend(nerd.into_iter().take(1).map(|p| (p, 0)));
    if cfg!(target_os = "macos") {
        for p in [
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ] {
            let p = PathBuf::from(p);
            if p.is_file() {
                v.push((p, 0));
            }
        }
    } else {
        for pat in [
            "Symbols Nerd Font Mono",
            "DejaVu Sans Mono",
            "DejaVu Sans",
            "Noto Sans Symbols 2",
            "Noto Sans Symbols",
            "Noto Sans CJK SC",
        ] {
            if let Some(f) = fc_match(pat) {
                if !v.contains(&f) {
                    v.push(f);
                }
            }
        }
    }
    v
}

/// A glyph ready to lay down: its coverage, where its top left sits from
/// the cell's left and the baseline, and whether it's made bold by hand.
#[derive(Clone, Debug)]
pub struct Glyph {
    pub dx: i32,
    pub top: i32,
    pub w: usize,
    pub cov: Vec<u8>,
    pub fake_bold: bool,
}

/// A face loaded: the font, and its own Unicode map.
struct Face {
    font: fontdue::Font,
    cmap: Option<Cmap>,
}

impl Face {
    /// `ch`'s glyph (0: none).
    fn gid(&self, ch: char) -> u16 {
        match &self.cmap {
            Some(c) => c.get(ch),
            None => self.font.lookup_glyph_index(ch),
        }
    }
}

struct Lazy {
    at: FaceAt,
    face: Option<Option<Face>>,
}

/// The faces, loaded; glyphs rasterised once each.
pub struct Fonts {
    faces: [Option<Face>; 4],
    more: Vec<Lazy>,
    /// an advance, per px (the regular face's `M`)
    pub adv: f32,
    /// ascent and descent per px (descent below the baseline: negative)
    pub ascent: f32,
    pub descent: f32,
    /// what was loaded, for the CLI
    pub what: String,
    cache: HashMap<(char, u8, u32, i32, i32), Option<Glyph>>,
}

fn load(at: &FaceAt) -> Option<Face> {
    let data = std::fs::read(&at.0).ok()?;
    let cmap = Cmap::read(&data, at.1);
    let font = fontdue::Font::from_bytes(
        data,
        fontdue::FontSettings {
            collection_index: at.1,
            scale: 40.0,
            load_substitutions: false,
        },
    )
    .ok()?;
    Some(Face { font, cmap })
}

/// Text a little heavier than a plain blend draws it (closer to how a
/// terminal shows it on a dark background).
fn weight(cov: Vec<u8>) -> Vec<u8> {
    static LUT: std::sync::OnceLock<[u8; 256]> = std::sync::OnceLock::new();
    let lut = LUT.get_or_init(|| {
        let mut t = [0u8; 256];
        for (i, v) in t.iter_mut().enumerate() {
            *v = ((i as f32 / 255.0).powf(0.8) * 255.0).round() as u8;
        }
        t
    });
    cov.into_iter().map(|c| lut[c as usize]).collect()
}

impl Fonts {
    pub fn load(spec: &FontSpec) -> Result<Fonts, String> {
        let mut faces: [Option<Face>; 4] = Default::default();
        for (i, f) in spec.faces.iter().enumerate() {
            if let Some(at) = f {
                faces[i] = load(at);
            }
        }
        let Some(reg) = faces[0].as_ref() else {
            let at = spec.faces[0]
                .as_ref()
                .map(|f| f.0.display().to_string())
                .unwrap_or_default();
            return Err(format!("couldn't read the font {at}"));
        };
        let adv = reg.font.metrics_indexed(reg.gid('M'), 100.0).advance_width / 100.0;
        let lm = reg.font.horizontal_line_metrics(100.0);
        let (ascent, descent) = lm
            .map(|l| (l.ascent / 100.0, l.descent / 100.0))
            .unwrap_or((0.8, -0.2));
        let name = spec.faces[0]
            .as_ref()
            .and_then(|f| f.0.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let styles: Vec<&str> = ["regular", "bold", "italic", "bold italic"]
            .iter()
            .zip(&faces)
            .filter(|(_, f)| f.is_some())
            .map(|(s, _)| *s)
            .collect();
        Ok(Fonts {
            what: format!("{name} ({})", styles.join(", ")),
            faces,
            more: spec
                .fallbacks
                .iter()
                .map(|at| Lazy {
                    at: at.clone(),
                    face: None,
                })
                .collect(),
            adv: if adv > 0.1 { adv } else { 0.6 },
            ascent,
            descent,
            cache: HashMap::new(),
        })
    }

    /// `ch` in `style` (1 bold, 2 italic) at `px`, fitted into a span `span`
    /// px wide and `height` tall. None: no font here has it.
    pub fn glyph(
        &mut self,
        ch: char,
        style: u8,
        px: f32,
        span: i32,
        height: i32,
    ) -> Option<&Glyph> {
        let key = (ch, style, (px * 16.0) as u32, span, height);
        if !self.cache.contains_key(&key) {
            let g = self.make(ch, style, px, span, height);
            self.cache.insert(key, g);
        }
        self.cache.get(&key).and_then(Option::as_ref)
    }

    fn make(&mut self, ch: char, style: u8, px: f32, span: i32, height: i32) -> Option<Glyph> {
        let style = style as usize & 3;
        // the face: the style's, the regular, then the fallbacks
        let mut order: Vec<usize> = vec![style];
        if style == 3 {
            order.extend([1, 2]);
        }
        order.push(0);
        let mut from: Option<(&Face, u16, bool, bool)> = None;
        for i in order {
            if let Some(f) = self.faces[i].as_ref() {
                let g = f.gid(ch);
                if g != 0 {
                    from = Some((f, g, true, style & 1 != 0 && i & 1 == 0));
                    break;
                }
            }
        }
        if from.is_none() {
            // loaded only when a character needs one
            for l in self.more.iter_mut() {
                let f = l.face.get_or_insert_with(|| load(&l.at));
                if f.as_ref().is_some_and(|f| f.gid(ch) != 0) {
                    break;
                }
            }
            from = self
                .more
                .iter()
                .filter_map(|l| l.face.as_ref().and_then(Option::as_ref))
                .find(|f| f.gid(ch) != 0)
                .map(|f| (f, f.gid(ch), false, style & 1 != 0));
        }
        let (face, gid, mono, fake_bold) = from?;
        let (mut m, mut cov) = face.font.rasterize_indexed(gid, px);
        // too big for its cell(s): smaller (a symbol font's, a CJK glyph in
        // one cell)
        let fit = ((span as f32 * 0.94) / m.width.max(1) as f32)
            .min((height as f32 * 0.92) / m.height.max(1) as f32);
        let wide = m.width as i32 > span || (!mono && m.advance_width > span as f32 * 1.05);
        if fit < 1.0 && (wide || m.height as i32 > height) {
            (m, cov) = face.font.rasterize_indexed(gid, px * fit);
        }
        let dx = if mono && m.width as i32 <= span {
            ((span as f32 - m.advance_width) / 2.0).round() as i32 + m.xmin
        } else {
            (span - m.width as i32) / 2
        };
        Some(Glyph {
            dx,
            top: -(m.ymin + m.height as i32),
            w: m.width,
            cov: weight(cov),
            fake_bold,
        })
    }
}

// ------------------------------------------------------------------ cells

/// A grid cell's geometry in px, and the strokes drawn in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cells {
    pub cw: i32,
    pub ch: i32,
    /// the font size
    pub px: f32,
    /// the baseline, from the cell's top
    pub base: i32,
    /// a light line's thickness, and a heavy one's
    pub light: i32,
    pub heavy: i32,
}

impl Cells {
    pub fn new(cw: i32, ch: i32, px: f32, fonts: &Fonts) -> Cells {
        let (asc, desc) = (fonts.ascent * px, -fonts.descent * px);
        let base = ((ch as f32 - (asc + desc)) / 2.0 + asc).round() as i32;
        let light = (px * 0.087).round().max(1.0) as i32;
        let heavy = ((px * 0.174).round() as i32).max(light + 1);
        Cells {
            cw,
            ch,
            px,
            base: base.clamp(1, ch),
            light,
            heavy,
        }
    }
}

/// One cell (or a wide character's two) as the screen has it, colours
/// resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub text: String,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// cells it covers (2: a wide character)
    pub cells: u8,
}

/// Draw a cell with its top left at (x, y).
pub fn paint(cv: &mut Canvas, fonts: &mut Fonts, m: &Cells, (x, y): (i32, i32), l: &Look) {
    let w = m.cw * l.cells.max(1) as i32;
    cv.rect(x, y, x + w, y + m.ch, l.bg);
    let clip = Clip::new(x, y, x + w, y + m.ch);
    let ch = l.text.chars().next().unwrap_or(' ');
    if l.underline {
        let uy = y + (m.base + m.light.max(2)).min(m.ch - m.light);
        cv.rect(x, uy, x + w, uy + m.light, l.fg);
    }
    if matches!(ch, ' ' | '\u{a0}' | '\0') || ch.is_control() {
        return;
    }
    if shape(cv, m, (x, y), ch, l.fg, clip) {
        return;
    }
    let style = l.bold as u8 | (l.italic as u8) << 1;
    match fonts.glyph(ch, style, m.px, w, m.ch) {
        Some(g) => {
            let top = (y + m.base + g.top)
                .clamp(y, (y + m.ch - (g.cov.len() / g.w.max(1)) as i32).max(y));
            cv.mask(x + g.dx, top, g.w, &g.cov, l.fg, clip);
            if g.fake_bold {
                cv.mask(x + g.dx + 1, top, g.w, &g.cov, l.fg, clip);
            }
        }
        None => {
            // nothing here has it (a colour emoji): a small box
            let (bw, bh) = ((w as f32 * 0.56).round(), (m.ch as f32 * 0.5).round());
            let (bx, by) = (
                x as f32 + (w as f32 - bw) / 2.0,
                y as f32 + (m.ch as f32 - bh) / 2.0,
            );
            let faint = mix(l.bg, l.fg, 0.45);
            cv.rrect(bx, by, bx + bw, by + bh, 2.0, faint, 1.0);
            let t = m.light as f32;
            cv.rrect(bx + t, by + t, bx + bw - t, by + bh - t, 1.0, l.bg, 1.0);
        }
    }
}

/// Box drawing U+2500–U+257F: each character's arms (up, right, down,
/// left): 1 light, 2 heavy, 3 double; 9: drawn otherwise (dashes, arcs,
/// diagonals).
const ARMS: [[u8; 4]; 128] = [
    [0, 1, 0, 1], // ─
    [0, 2, 0, 2], // ━
    [1, 0, 1, 0], // │
    [2, 0, 2, 0], // ┃
    [9, 9, 9, 9], // ┄
    [9, 9, 9, 9], // ┅
    [9, 9, 9, 9], // ┆
    [9, 9, 9, 9], // ┇
    [9, 9, 9, 9], // ┈
    [9, 9, 9, 9], // ┉
    [9, 9, 9, 9], // ┊
    [9, 9, 9, 9], // ┋
    [0, 1, 1, 0], // ┌
    [0, 2, 1, 0], // ┍
    [0, 1, 2, 0], // ┎
    [0, 2, 2, 0], // ┏
    [0, 0, 1, 1], // ┐
    [0, 0, 1, 2], // ┑
    [0, 0, 2, 1], // ┒
    [0, 0, 2, 2], // ┓
    [1, 1, 0, 0], // └
    [1, 2, 0, 0], // ┕
    [2, 1, 0, 0], // ┖
    [2, 2, 0, 0], // ┗
    [1, 0, 0, 1], // ┘
    [1, 0, 0, 2], // ┙
    [2, 0, 0, 1], // ┚
    [2, 0, 0, 2], // ┛
    [1, 1, 1, 0], // ├
    [1, 2, 1, 0], // ┝
    [2, 1, 1, 0], // ┞
    [1, 1, 2, 0], // ┟
    [2, 1, 2, 0], // ┠
    [2, 2, 1, 0], // ┡
    [1, 2, 2, 0], // ┢
    [2, 2, 2, 0], // ┣
    [1, 0, 1, 1], // ┤
    [1, 0, 1, 2], // ┥
    [2, 0, 1, 1], // ┦
    [1, 0, 2, 1], // ┧
    [2, 0, 2, 1], // ┨
    [2, 0, 1, 2], // ┩
    [1, 0, 2, 2], // ┪
    [2, 0, 2, 2], // ┫
    [0, 1, 1, 1], // ┬
    [0, 1, 1, 2], // ┭
    [0, 2, 1, 1], // ┮
    [0, 2, 1, 2], // ┯
    [0, 1, 2, 1], // ┰
    [0, 1, 2, 2], // ┱
    [0, 2, 2, 1], // ┲
    [0, 2, 2, 2], // ┳
    [1, 1, 0, 1], // ┴
    [1, 1, 0, 2], // ┵
    [1, 2, 0, 1], // ┶
    [1, 2, 0, 2], // ┷
    [2, 1, 0, 1], // ┸
    [2, 1, 0, 2], // ┹
    [2, 2, 0, 1], // ┺
    [2, 2, 0, 2], // ┻
    [1, 1, 1, 1], // ┼
    [1, 1, 1, 2], // ┽
    [1, 2, 1, 1], // ┾
    [1, 2, 1, 2], // ┿
    [2, 1, 1, 1], // ╀
    [1, 1, 2, 1], // ╁
    [2, 1, 2, 1], // ╂
    [2, 1, 1, 2], // ╃
    [2, 2, 1, 1], // ╄
    [1, 1, 2, 2], // ╅
    [1, 2, 2, 1], // ╆
    [2, 2, 1, 2], // ╇
    [1, 2, 2, 2], // ╈
    [2, 1, 2, 2], // ╉
    [2, 2, 2, 1], // ╊
    [2, 2, 2, 2], // ╋
    [9, 9, 9, 9], // ╌
    [9, 9, 9, 9], // ╍
    [9, 9, 9, 9], // ╎
    [9, 9, 9, 9], // ╏
    [0, 3, 0, 3], // ═
    [3, 0, 3, 0], // ║
    [0, 3, 1, 0], // ╒
    [0, 1, 3, 0], // ╓
    [0, 3, 3, 0], // ╔
    [0, 0, 1, 3], // ╕
    [0, 0, 3, 1], // ╖
    [0, 0, 3, 3], // ╗
    [1, 3, 0, 0], // ╘
    [3, 1, 0, 0], // ╙
    [3, 3, 0, 0], // ╚
    [1, 0, 0, 3], // ╛
    [3, 0, 0, 1], // ╜
    [3, 0, 0, 3], // ╝
    [1, 3, 1, 0], // ╞
    [3, 1, 3, 0], // ╟
    [3, 3, 3, 0], // ╠
    [1, 0, 1, 3], // ╡
    [3, 0, 3, 1], // ╢
    [3, 0, 3, 3], // ╣
    [0, 3, 1, 3], // ╤
    [0, 1, 3, 1], // ╥
    [0, 3, 3, 3], // ╦
    [1, 3, 0, 3], // ╧
    [3, 1, 0, 1], // ╨
    [3, 3, 0, 3], // ╩
    [1, 3, 1, 3], // ╪
    [3, 1, 3, 1], // ╫
    [3, 3, 3, 3], // ╬
    [9, 9, 9, 9], // ╭
    [9, 9, 9, 9], // ╮
    [9, 9, 9, 9], // ╯
    [9, 9, 9, 9], // ╰
    [9, 9, 9, 9], // ╱
    [9, 9, 9, 9], // ╲
    [9, 9, 9, 9], // ╳
    [0, 0, 0, 1], // ╴
    [1, 0, 0, 0], // ╵
    [0, 1, 0, 0], // ╶
    [0, 0, 1, 0], // ╷
    [0, 0, 0, 2], // ╸
    [2, 0, 0, 0], // ╹
    [0, 2, 0, 0], // ╺
    [0, 0, 2, 0], // ╻
    [0, 2, 0, 1], // ╼
    [1, 0, 2, 0], // ╽
    [0, 1, 0, 2], // ╾
    [2, 0, 1, 0], // ╿
];

/// A box-drawing character's arms (None: not one, or drawn otherwise).
pub fn arms(ch: char) -> Option<[u8; 4]> {
    let i = (ch as u32).checked_sub(0x2500)? as usize;
    ARMS.get(i).copied().filter(|a| a[0] != 9)
}

/// A dashed line: (horizontal, dashes, heavy).
fn dashes(ch: char) -> Option<(bool, i32, bool)> {
    Some(match ch {
        '┄' => (true, 3, false),
        '┅' => (true, 3, true),
        '┆' => (false, 3, false),
        '┇' => (false, 3, true),
        '┈' => (true, 4, false),
        '┉' => (true, 4, true),
        '┊' => (false, 4, false),
        '┋' => (false, 4, true),
        '╌' => (true, 2, false),
        '╍' => (true, 2, true),
        '╎' => (false, 2, false),
        '╏' => (false, 2, true),
        _ => return None,
    })
}

/// A block element: rectangles as fractions of the cell (x0, y0, x1, y1),
/// and how opaque (the shades).
pub fn block(ch: char) -> Option<(Vec<[f32; 4]>, f32)> {
    let full = |v: Vec<[f32; 4]>| Some((v, 1.0));
    const UL: [f32; 4] = [0.0, 0.0, 0.5, 0.5];
    const UR: [f32; 4] = [0.5, 0.0, 1.0, 0.5];
    const LL: [f32; 4] = [0.0, 0.5, 0.5, 1.0];
    const LR: [f32; 4] = [0.5, 0.5, 1.0, 1.0];
    match ch {
        '█' => full(vec![[0.0, 0.0, 1.0, 1.0]]),
        '▀' => full(vec![[0.0, 0.0, 1.0, 0.5]]),
        '▔' => full(vec![[0.0, 0.0, 1.0, 0.125]]),
        '▕' => full(vec![[0.875, 0.0, 1.0, 1.0]]),
        '▐' => full(vec![[0.5, 0.0, 1.0, 1.0]]),
        '\u{2581}'..='\u{2588}' => {
            let k = (ch as u32 - 0x2580) as f32;
            full(vec![[0.0, 1.0 - k / 8.0, 1.0, 1.0]])
        }
        '\u{2589}'..='\u{258f}' => {
            let k = (0x2590 - ch as u32) as f32;
            full(vec![[0.0, 0.0, k / 8.0, 1.0]])
        }
        '░' => Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.25)),
        '▒' => Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.5)),
        '▓' => Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.75)),
        '▖' => full(vec![LL]),
        '▗' => full(vec![LR]),
        '▘' => full(vec![UL]),
        '▝' => full(vec![UR]),
        '▙' => full(vec![UL, LL, LR]),
        '▚' => full(vec![UL, LR]),
        '▛' => full(vec![UL, UR, LL]),
        '▜' => full(vec![UL, UR, LR]),
        '▞' => full(vec![UR, LL]),
        '▟' => full(vec![UR, LL, LR]),
        _ => None,
    }
}

/// A Braille pattern's dots: (column 0–1, row 0–3).
pub fn braille(ch: char) -> Option<Vec<(u8, u8)>> {
    let v = ch as u32;
    if !(0x2800..=0x28ff).contains(&v) {
        return None;
    }
    let bits = (v - 0x2800) as u8;
    const AT: [(u8, u8); 8] = [
        (0, 0),
        (0, 1),
        (0, 2),
        (1, 0),
        (1, 1),
        (1, 2),
        (0, 3),
        (1, 3),
    ];
    Some(
        (0..8)
            .filter(|i| bits & (1 << i) != 0)
            .map(|i| AT[i as usize])
            .collect(),
    )
}

/// The character as a shape in its cell, when it's one: true if drawn.
fn shape(cv: &mut Canvas, m: &Cells, (x, y): (i32, i32), ch: char, fg: Rgb, clip: Clip) -> bool {
    if let Some(a) = arms(ch) {
        lines(cv, m, (x, y), a, fg);
        return true;
    }
    let tl = m.light;
    // a light stroke's top (a horizontal's) and left (a vertical's)
    let hy = |t: i32| y + (m.ch - t) / 2;
    let vx = |t: i32| x + (m.cw - t) / 2;
    if let Some((horiz, n, heavy)) = dashes(ch) {
        let t = if heavy { m.heavy } else { tl };
        let len = if horiz { m.cw } else { m.ch } as f32;
        let slot = len / n as f32;
        for i in 0..n {
            let a = (i as f32 * slot + slot * 0.2).round() as i32;
            let b = ((i + 1) as f32 * slot - slot * 0.2).round() as i32;
            if horiz {
                cv.rect(x + a, hy(t), x + b.max(a + 1), hy(t) + t, fg);
            } else {
                cv.rect(vx(t), y + a, vx(t) + t, y + b.max(a + 1), fg);
            }
        }
        return true;
    }
    if matches!(ch, '╭' | '╮' | '╯' | '╰') {
        let (lcx, lcy) = (
            vx(tl) as f32 + tl as f32 / 2.0,
            hy(tl) as f32 + tl as f32 / 2.0,
        );
        let right = matches!(ch, '╭' | '╰');
        let down = matches!(ch, '╭' | '╮');
        let r = if right {
            (x + m.cw) as f32 - lcx
        } else {
            lcx - x as f32
        };
        let (sx, sy) = (
            if right { 1.0 } else { -1.0 },
            if down { 1.0 } else { -1.0 },
        );
        let (acx, acy) = (lcx + sx * r, lcy + sy * r);
        cv.ring((acx, acy), r, tl as f32, (-sx as i8, -sy as i8), fg, clip);
        if down {
            cv.rect(vx(tl), acy.floor() as i32, vx(tl) + tl, y + m.ch, fg);
        } else {
            cv.rect(vx(tl), y, vx(tl) + tl, acy.ceil() as i32, fg);
        }
        return true;
    }
    if matches!(ch, '╱' | '╲' | '╳') {
        let hw = tl as f32 * 0.5 * (1.0 + (m.cw as f32 / m.ch as f32).powi(2)).sqrt();
        let (l, r, t, b) = (x as f32, (x + m.cw) as f32, y as f32, (y + m.ch) as f32);
        let mut polys = Vec::new();
        if ch != '╲' {
            polys.push(vec![(r - hw, t), (r + hw, t), (l + hw, b), (l - hw, b)]);
        }
        if ch != '╱' {
            polys.push(vec![(l - hw, t), (l + hw, t), (r + hw, b), (r - hw, b)]);
        }
        for p in polys {
            cv.fill(&[p], fg, 1.0, clip);
        }
        return true;
    }
    if let Some((rects, op)) = block(ch) {
        for r in rects {
            let fx = |f: f32| x + (f * m.cw as f32).round() as i32;
            let fy = |f: f32| y + (f * m.ch as f32).round() as i32;
            cv.blend(fx(r[0]), fy(r[1]), fx(r[2]), fy(r[3]), fg, op);
        }
        return true;
    }
    if let Some(dots) = braille(ch) {
        let r = (m.cw as f32 * 0.14).max(0.9);
        for (c, row) in dots {
            let cx = x as f32 + m.cw as f32 * (0.25 + 0.5 * c as f32);
            let cy = y as f32 + m.ch as f32 * (0.125 + 0.25 * row as f32);
            cv.disc(cx, cy, r, fg, clip);
        }
        return true;
    }
    false
}

/// Box drawing's straight arms, joined: singles through the middle, doubles
/// as two tracks meeting at their corners.
fn lines(cv: &mut Canvas, m: &Cells, (x, y): (i32, i32), a: [u8; 4], fg: Rgb) {
    let (tl, th) = (m.light, m.heavy);
    let t = |v: u8| if v == 2 { th } else { tl };
    let hy = |t: i32| y + (m.ch - t) / 2;
    let vx = |t: i32| x + (m.cw - t) / 2;
    let (cx, cy) = (x + m.cw / 2, y + m.ch / 2);
    // the gap between a double's tracks
    let gx = (tl + 1).max((m.cw as f32 * 0.18).round() as i32);
    let gy = (tl + 1)
        .max((m.ch as f32 * 0.09).round() as i32)
        .max(gx.min(m.ch / 5));
    let single = |v: u8| v == 1 || v == 2;
    let [up, right, down, left] = a;
    // across the horizontal arms: where a vertical's strokes stand
    let vs = [up, down]
        .iter()
        .filter(|v| single(**v))
        .map(|v| t(*v))
        .max()
        .unwrap_or(0);
    let hs = [left, right]
        .iter()
        .filter(|v| single(**v))
        .map(|v| t(*v))
        .max()
        .unwrap_or(0);
    let (vl, vr) = (vx(tl) - gx, vx(tl) + gx); // a vertical double's tracks (left edges)
    let (ht, hb) = (hy(tl) - gy, hy(tl) + gy); // a horizontal double's (top edges)
                                               // Each arm runs from the cell's edge to where the crossing arms stand:
                                               // (s, e) = where it starts going right (down), ends going left (up).
                                               // A single crosses a double that's on one side only, stops at the near
                                               // track of one on both; a double's track turns at the inner corner on
                                               // its own side, the outer one on the other.
                                               // right and left arms
    for (v, rightward) in [(right, true), (left, false)] {
        if v == 0 {
            continue;
        }
        if single(v) {
            let tt = t(v);
            let (s, e) = if up == 3 && down == 3 {
                (vr, vl + tl)
            } else if up == 3 || down == 3 {
                (vl, vr + tl)
            } else if vs > 0 {
                (vx(vs), vx(vs) + vs)
            } else {
                (cx, cx)
            };
            if rightward {
                cv.rect(s, hy(tt), x + m.cw, hy(tt) + tt, fg);
            } else {
                cv.rect(x, hy(tt), e, hy(tt) + tt, fg);
            }
        } else {
            for (track, near, far) in [(ht, up, down), (hb, down, up)] {
                let (s, e) = if near == 3 {
                    (vr, vl + tl)
                } else if single(near) {
                    (vx(t(near)), vx(t(near)) + t(near))
                } else if far == 3 {
                    (vl, vr + tl)
                } else if single(far) {
                    (vx(t(far)), vx(t(far)) + t(far))
                } else {
                    (cx, cx)
                };
                if rightward {
                    cv.rect(s, track, x + m.cw, track + tl, fg);
                } else {
                    cv.rect(x, track, e, track + tl, fg);
                }
            }
        }
    }
    // down and up arms
    for (v, downward) in [(down, true), (up, false)] {
        if v == 0 {
            continue;
        }
        if single(v) {
            let tt = t(v);
            let (s, e) = if left == 3 && right == 3 {
                (hb, ht + tl)
            } else if left == 3 || right == 3 {
                (ht, hb + tl)
            } else if hs > 0 {
                (hy(hs), hy(hs) + hs)
            } else {
                (cy, cy)
            };
            if downward {
                cv.rect(vx(tt), s, vx(tt) + tt, y + m.ch, fg);
            } else {
                cv.rect(vx(tt), y, vx(tt) + tt, e, fg);
            }
        } else {
            for (track, near, far) in [(vl, left, right), (vr, right, left)] {
                let (s, e) = if near == 3 {
                    (hb, ht + tl)
                } else if single(near) {
                    (hy(t(near)), hy(t(near)) + t(near))
                } else if far == 3 {
                    (ht, hb + tl)
                } else if single(far) {
                    (hy(t(far)), hy(t(far)) + t(far))
                } else {
                    (cy, cy)
                };
                if downward {
                    cv.rect(track, s, track + tl, y + m.ch, fg);
                } else {
                    cv.rect(track, y, track + tl, e, fg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(cw: i32, ch: i32) -> Cells {
        Cells {
            cw,
            ch,
            px: 12.0,
            base: ch * 3 / 4,
            light: 1,
            heavy: 2,
        }
    }

    /// The cell's pixels as '#' (lit) and '.' (dark).
    fn art(cv: &Canvas) -> Vec<String> {
        (0..cv.h)
            .map(|y| {
                (0..cv.w)
                    .map(|x| if cv.at(x, y).0 > 127 { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    fn draw(ch: char, cw: i32, chh: i32) -> Vec<String> {
        let mut cv = Canvas::new(cw as usize, chh as usize, (0, 0, 0));
        let clip = cv.all();
        assert!(
            shape(&mut cv, &cells(cw, chh), (0, 0), ch, (255, 255, 255), clip),
            "{ch}"
        );
        art(&cv)
    }

    #[test]
    fn lines_meet_their_neighbours_at_the_cell_edges() {
        // ─ crosses the whole cell on the middle row; │ the whole height
        let h = draw('─', 7, 15);
        assert_eq!(h[7], "#######");
        assert!(h.iter().enumerate().all(|(i, r)| i == 7 || r == "......."));
        let v = draw('│', 7, 15);
        assert!(v.iter().all(|r| r == "...#..."), "{v:?}");
        // a corner: the arms reach the edges and close in the middle
        let c = draw('┌', 7, 15);
        assert_eq!(c[7], "...####");
        assert_eq!(c[14], "...#...");
        assert_eq!(c[0], ".......");
        // heavy is thicker; ┃ two columns
        assert!(draw('┃', 7, 15).iter().all(|r| r == "..##..."));
        // a double: two tracks; its corner's tracks meet theirs
        let d = draw('═', 7, 15);
        assert_eq!(d.iter().filter(|r| *r == "#######").count(), 2, "{d:?}");
        let k = draw('╔', 7, 15);
        assert!(k[14].matches('#').count() == 2, "{k:?}");
        assert!(k.iter().any(|r| r.ends_with("######")), "{k:?}");
        // half lines stop in the middle
        assert_eq!(draw('╴', 8, 15)[7], "####....");
        assert_eq!(draw('╶', 8, 15)[7], "....####");
        // every arm of the table is drawn by the same rule
        for c in 0x2500u32..0x2580 {
            let ch = char::from_u32(c).unwrap();
            let mut cv = Canvas::new(9, 19, (0, 0, 0));
            let clip = cv.all();
            assert!(
                shape(&mut cv, &cells(9, 19), (0, 0), ch, (255, 255, 255), clip),
                "{ch}"
            );
            assert!(cv.px.iter().any(|p| *p > 0), "{ch} drew nothing");
        }
    }

    #[test]
    fn a_rounded_corner_joins_the_straight_lines() {
        let a = draw('╭', 8, 15);
        // reaches the right edge on the line's row and the bottom on its column
        assert!(a[7].ends_with('#'), "{a:?}");
        assert_eq!(&a[14][3..5], "#.", "{a:?}");
        let b = draw('╯', 8, 15);
        assert!(
            b[7].starts_with('#') && b[0].chars().nth(3) == Some('#'),
            "{b:?}"
        );
    }

    #[test]
    fn blocks_shades_and_braille() {
        let full = draw('█', 6, 12);
        assert!(full.iter().all(|r| r == "######"));
        let low = draw('▄', 6, 12);
        assert!(low[..6].iter().all(|r| r == "......") && low[6..].iter().all(|r| r == "######"));
        let left = draw('▌', 6, 12);
        assert!(left.iter().all(|r| r == "###..."));
        // a shade is the colour, partly
        let mut cv = Canvas::new(4, 4, (0, 0, 0));
        let clip = cv.all();
        shape(&mut cv, &cells(4, 4), (0, 0), '▒', (200, 200, 200), clip);
        assert_eq!(cv.at(1, 1), (100, 100, 100));
        // Braille: ⣿ eight dots, ⠁ one (top left)
        let mut cv = Canvas::new(10, 20, (0, 0, 0));
        let clip = cv.all();
        shape(&mut cv, &cells(10, 20), (0, 0), '⠁', (255, 255, 255), clip);
        assert!(cv.at(2, 2).0 > 100 && cv.at(7, 2).0 == 0 && cv.at(2, 17).0 == 0);
        assert_eq!(braille('⣿').unwrap().len(), 8);
        assert_eq!(braille('a'), None);
        assert_eq!(arms('a'), None);
        assert_eq!(arms('╭'), None);
        assert_eq!(arms('╬'), Some([3, 3, 3, 3]));
    }

    #[test]
    fn the_canvas_blends_clips_and_fills() {
        let mut cv = Canvas::new(10, 10, (10, 20, 30));
        cv.rect(-5, -5, 2, 2, (255, 0, 0));
        assert_eq!(cv.at(0, 0), (255, 0, 0));
        assert_eq!(cv.at(2, 2), (10, 20, 30));
        cv.blend(5, 5, 20, 20, (110, 120, 130), 0.5);
        assert_eq!(cv.at(9, 9), (60, 70, 80));
        // a square path: its inside full, its outside untouched
        let mut cv = Canvas::new(10, 10, (0, 0, 0));
        let all = cv.all();
        cv.fill(
            &[vec![(2.0, 2.0), (8.0, 2.0), (8.0, 8.0), (2.0, 8.0)]],
            (255, 255, 255),
            1.0,
            all,
        );
        assert_eq!(cv.at(5, 5), (255, 255, 255));
        assert_eq!(cv.at(1, 5), (0, 0, 0));
        assert_eq!(cv.at(8, 5), (0, 0, 0));
        // half a pixel covered: half the colour
        let mut cv = Canvas::new(4, 1, (0, 0, 0));
        let all = cv.all();
        cv.fill(
            &[vec![(0.0, 0.0), (1.5, 0.0), (1.5, 1.0), (0.0, 1.0)]],
            (200, 200, 200),
            1.0,
            all,
        );
        assert_eq!(cv.at(1, 0), (100, 100, 100));
        assert_eq!(mix((0, 0, 0), (200, 100, 50), 0.5), (100, 50, 25));
        assert_eq!(parse_hex("#e2b451"), Some((0xe2, 0xb4, 0x51)));
        assert_eq!(parse_hex("e2b451"), None);
    }

    #[test]
    fn the_mark_is_three_outlines() {
        let p = path(super::super::poster::MARK, 0.25, (10.0, 10.0));
        assert_eq!(p.len(), 3);
        let all = p.iter().flatten();
        let (mx, my) = all.fold((0f32, 0f32), |a, &(x, y)| (a.0.max(x), a.1.max(y)));
        assert!(
            mx <= 10.0 + 25.0 && my <= 10.0 + 25.0 && mx > 30.0,
            "{mx} {my}"
        );
    }

    #[test]
    fn a_collection_names_its_faces_by_style() {
        // a made-up collection: each face one table, `head`, with its style
        let mk = |styles: &[u16]| -> Vec<u8> {
            let n = styles.len();
            let mut d = Vec::new();
            d.extend_from_slice(b"ttcf");
            d.extend_from_slice(&[0, 1, 0, 0]);
            d.extend_from_slice(&(n as u32).to_be_bytes());
            let header = 12 + 4 * n;
            let face_len = 12 + 16 + 54;
            for i in 0..n {
                d.extend_from_slice(&((header + i * face_len) as u32).to_be_bytes());
            }
            for (i, ms) in styles.iter().enumerate() {
                let off = header + i * face_len;
                let mut f = vec![0u8; face_len];
                f[4..6].copy_from_slice(&1u16.to_be_bytes());
                f[12..16].copy_from_slice(b"head");
                let head_at = (off + 28) as u32;
                f[20..24].copy_from_slice(&head_at.to_be_bytes());
                f[28 + 44..28 + 46].copy_from_slice(&ms.to_be_bytes());
                d.extend_from_slice(&f);
            }
            d
        };
        let v = faces(&mk(&[0, 1, 2, 3]));
        assert_eq!(
            v,
            vec![
                (0, false, false),
                (1, true, false),
                (2, false, true),
                (3, true, true)
            ]
        );
        assert_eq!(faces(b"junk"), vec![(0, false, false)]);
    }

    #[test]
    fn the_unicode_map_is_read_by_hand() {
        // format 4: A–C by a delta (→ 10, 11, 12), a–b through the glyph
        // array (→ 7, 0), the end segment
        let w = |v: &[u16]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
        let segx2 = 6u16;
        let mut sub = w(&[4, 0, 0, segx2, 0, 0, 0]);
        sub.extend(w(&[0x43, 0x62, 0xffff, 0]));
        sub.extend(w(&[0x41, 0x61, 0xffff]));
        sub.extend(w(&[10u16.wrapping_sub(0x41), 0, 1]));
        // seg 1's range offset: from its own place to the glyph array
        sub.extend(w(&[0, 4, 0]));
        sub.extend(w(&[7, 0]));
        let len = sub.len() as u16;
        sub[2..4].copy_from_slice(&len.to_be_bytes());
        let c = Cmap { format: 4, sub };
        assert_eq!(
            (c.get('A'), c.get('C'), c.get('a'), c.get('b')),
            (10, 12, 7, 0)
        );
        assert_eq!((c.get('D'), c.get('😀')), (0, 0));
        // format 12: one group, U+1F600–U+1F602 → 500…
        let mut sub = w(&[12, 0]);
        sub.extend([0, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0, 1]);
        sub.extend(0x1f600u32.to_be_bytes());
        sub.extend(0x1f602u32.to_be_bytes());
        sub.extend(500u32.to_be_bytes());
        let c = Cmap { format: 12, sub };
        assert_eq!((c.get('😁'), c.get('😃'), c.get('A')), (501, 0, 0));
    }

    /// The real fonts here (macOS: Menlo's four faces).
    #[test]
    fn the_system_monospace_is_found() {
        let Ok(spec) = find("") else {
            return;
        };
        let mut f = Fonts::load(&spec).unwrap();
        assert!(f.adv > 0.4 && f.adv < 0.8, "{}", f.adv);
        let g = f.glyph('M', 0, 14.0, 9, 18).cloned().unwrap();
        assert!(g.w > 3 && g.cov.iter().any(|c| *c > 200));
        assert!(f.glyph('M', 1, 14.0, 9, 18).is_some());
        // a colour emoji: no outline anywhere (a box is drawn instead)
        let _ = f.glyph('🚀', 0, 14.0, 18, 18);
        if cfg!(target_os = "macos") {
            assert!(spec.faces[1].is_some(), "{spec:?}");
            assert_eq!(spec.faces[1].as_ref().unwrap().1, 1);
            // Menlo's Mac Roman map isn't taken for Unicode: ° and · are
            // their own glyphs, not ∞ and ∑
            let reg = f.faces[0].as_ref().unwrap();
            assert!(reg.cmap.is_some());
            assert_ne!(reg.gid('°'), reg.gid('∞'));
            assert_ne!(reg.gid('·'), reg.gid('∑'));
            assert_ne!(reg.gid('°'), 0);
        }
        assert!(find("/no/such/font.ttf").is_err());
    }
}
