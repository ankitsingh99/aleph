//! Turning rendered logos into what the app embeds: palettes, thresholded
//! bitmaps, their run-length encoding, and the colour fitting that keeps a
//! mark visible on dark and light backgrounds (the same rules as
//! `src/logo.rs`, which applies them at run time to the terminal tiers).

use resvg::tiny_skia::Pixmap;
use std::collections::BTreeMap;

pub type Rgb = [u8; 3];

/// The characters of the run-length code (one per run).
pub const RUNS: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_";

/// A pixel is ink from half opacity up.
pub const INK: u8 = 128;

pub fn parse_hex(h: &str) -> Rgb {
    let v = u32::from_str_radix(h.trim_start_matches('#'), 16)
        .unwrap_or_else(|_| panic!("bad colour {h}"));
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The colours of a render, most used first: solid pixels only (edges blend
/// with transparency), near-duplicates merged, at most 4. A mark that renders
/// black (no fill of its own, like every Simple Icons file) gets `brand`.
pub fn palette(pm: &Pixmap, brand: Rgb) -> Vec<Rgb> {
    let mut counts: BTreeMap<Rgb, usize> = BTreeMap::new();
    for p in pm.pixels() {
        if p.alpha() >= 250 {
            let c = p.demultiply();
            *counts.entry([c.red(), c.green(), c.blue()]).or_default() += 1;
        }
    }
    let mut by: Vec<(Rgb, usize)> = counts.into_iter().collect();
    // most used first, ties by colour, so the order never depends on hashing
    by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut out: Vec<Rgb> = Vec::new();
    for (c, n) in by {
        // a colour that covers almost nothing is antialiasing between two others
        if n * 200 < pm.pixels().len() && !out.is_empty() {
            continue;
        }
        if out.iter().all(|o| dist(*o, c) > 48 * 48) {
            out.push(c);
        }
        if out.len() == 4 {
            break;
        }
    }
    match out.as_slice() {
        [] | [[0, 0, 0]] => vec![brand],
        _ => out,
    }
}

pub fn dist(a: Rgb, b: Rgb) -> u32 {
    (0..3)
        .map(|i| {
            let d = a[i] as i32 - b[i] as i32;
            (d * d) as u32
        })
        .sum()
}

/// Palette indices (0 = transparent, k = `pal[k-1]`) of `pm`, row-major; a
/// mark rendered black maps all its ink to the first colour. A pixel is ink
/// from half opacity — unless that loses a quarter of the mark's area, as
/// lines thinner than a pixel do (Kali's dragon at 16 px): then each pixel is
/// decided from `hi`, the same mark at 4×, and is ink when 3/16 of its block
/// is solid there, which keeps thin lines one pixel wide.
pub fn indices(pm: &Pixmap, hi: &Pixmap, pal: &[Rgb]) -> Vec<u8> {
    let colour = |p: &resvg::tiny_skia::PremultipliedColorU8| {
        let c = p.demultiply();
        let c = [c.red(), c.green(), c.blue()];
        1 + (0..pal.len()).min_by_key(|&i| dist(pal[i], c)).unwrap_or(0)
    };
    let area: f64 = pm.pixels().iter().map(|p| p.alpha() as f64 / 255.0).sum();
    let solid = pm.pixels().iter().filter(|p| p.alpha() >= INK).count();
    if solid as f64 >= 0.75 * area {
        return pm
            .pixels()
            .iter()
            .map(|p| if p.alpha() < INK { 0 } else { colour(p) as u8 })
            .collect();
    }
    let (n, hw) = (pm.width() as usize, hi.width() as usize);
    let k = hw / n;
    (0..n * n)
        .map(|i| {
            let (x, y) = (i % n, i / n);
            let mut votes = [0usize; 5];
            for dy in 0..k {
                for dx in 0..k {
                    let p = &hi.pixels()[(y * k + dy) * hw + x * k + dx];
                    if p.alpha() >= INK {
                        votes[colour(p)] += 1;
                    }
                }
            }
            let solid: usize = votes.iter().sum();
            if solid * 16 < k * k * 3 {
                return 0;
            }
            // the colour most of the block has (the first on a tie)
            (1..votes.len()).rev().max_by_key(|&c| votes[c]).unwrap() as u8
        })
        .collect()
}

/// An indexed PNG of `pm`, whose ink is `from`, drawn in `to`: `256 / n`
/// alpha steps per colour (colour k's at indices k·steps…; index 0 is
/// transparent). One byte a pixel, so a fraction of an RGBA PNG's size, and
/// recoloured by rewriting its PLTE chunk alone — which the app does at run
/// time to fit the theme.
pub fn indexed_png(pm: &Pixmap, from: &[Rgb], to: &[Rgb]) -> Vec<u8> {
    let steps = 256 / to.len();
    let px: Vec<u8> = pm
        .pixels()
        .iter()
        .map(|p| {
            let a = p.alpha() as usize;
            if a == 0 {
                return 0;
            }
            let c = p.demultiply();
            let c = [c.red(), c.green(), c.blue()];
            let k = (0..from.len())
                .min_by_key(|&i| dist(from[i], c))
                .unwrap_or(0);
            (k * steps + (a * (steps - 1) + 127) / 255) as u8
        })
        .collect();
    let mut plte = Vec::with_capacity(to.len() * steps * 3);
    let mut trns = Vec::with_capacity(to.len() * steps);
    for c in to {
        for s in 0..steps {
            plte.extend_from_slice(c);
            trns.push(((s * 255 + (steps - 1) / 2) / (steps - 1)) as u8);
        }
    }
    // palette images rarely gain from filters: keep whichever is smaller
    let enc = |adaptive: bool| {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, pm.width(), pm.height());
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::Eight);
        e.set_palette(plte.clone());
        e.set_trns(trns.clone());
        e.set_compression(png::Compression::Best);
        if adaptive {
            e.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        } else {
            e.set_filter(png::FilterType::NoFilter);
        }
        let mut w = e.write_header().expect("png header");
        w.write_image_data(&px).expect("png data");
        w.finish().expect("png end");
        out
    };
    let (a, b) = (enc(false), enc(true));
    if b.len() < a.len() {
        b
    } else {
        a
    }
}

/// Run-length code: one `RUNS` character per run, `index · span + (len − 1)`
/// with `span = 64 / (colours + 1)`.
pub fn rle(px: &[u8], colours: usize) -> String {
    let span = 64 / (colours + 1);
    let mut s = String::new();
    let mut i = 0;
    while i < px.len() {
        let v = px[i] as usize;
        let mut n = 1;
        while i + n < px.len() && px[i + n] as usize == v && n < span {
            n += 1;
        }
        s.push(RUNS[v * span + n - 1] as char);
        i += n;
    }
    s
}

/// The inverse of [`rle`] (for the round-trip check).
pub fn unrle(s: &str, colours: usize, len: usize) -> Vec<u8> {
    let span = 64 / (colours + 1);
    let mut v = Vec::with_capacity(len);
    for b in s.bytes() {
        let k = RUNS.iter().position(|r| *r == b).expect("run code");
        v.extend(std::iter::repeat_n((k / span) as u8, k % span + 1));
    }
    v
}

// ------------------------------------------------------------------ colour fitting

/// The app's own backgrounds and text colours (`ui::c::Pal::dark` / `light`).
pub const DARK_BG: Rgb = [0x11, 0x11, 0x11];
pub const DARK_TEXT: Rgb = [0xee, 0xee, 0xee];
pub const LIGHT_BG: Rgb = [0xfc, 0xfc, 0xfc];
pub const LIGHT_TEXT: Rgb = [0x20, 0x20, 0x20];

/// Black, white and greys: marks that are "ink", not a colour.
pub fn greyish(c: Rgb) -> bool {
    let (mx, mn) = (*c.iter().max().unwrap(), *c.iter().min().unwrap());
    mx - mn <= 24
}

fn lin(v: u8) -> f64 {
    let v = v as f64 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub fn luminance(c: Rgb) -> f64 {
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (luminance(a) + 0.05, luminance(b) + 0.05);
    x.max(y) / x.min(y)
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let [r, g, b] = c.map(|v| v as f64 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 {
        d / (2.0 - mx - mn)
    } else {
        d / (mx + mn)
    };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |t: f64| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

/// The colour a mark is drawn in on `bg`: ink (black/white/grey marks) takes
/// the theme's text colour; a colour too close to the background is made
/// lighter or darker (same hue) until it stands out (3:1, WCAG's bar for
/// graphics); anything else keeps its brand colour.
pub fn fit(c: Rgb, bg: Rgb, text: Rgb) -> Rgb {
    if greyish(c) {
        return text;
    }
    if contrast(c, bg) >= 3.0 {
        return c;
    }
    let (h, s, l) = to_hsl(c);
    let up = luminance(bg) < 0.5;
    let (mut lo, mut hi) = if up { (l, 1.0) } else { (0.0, l) };
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        let ok = contrast(from_hsl(h, s, mid), bg) >= 3.0;
        // keep the lightness that just passes, as close to the brand as can be
        match (up, ok) {
            (true, true) | (false, false) => hi = mid,
            _ => lo = mid,
        }
    }
    from_hsl(h, s, if up { hi } else { lo })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_round_trip() {
        let px = vec![
            0, 0, 1, 1, 1, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        ];
        for colours in [2usize, 3, 4] {
            let s = rle(&px, colours);
            assert_eq!(unrle(&s, colours, px.len()), px);
        }
        let long = vec![1u8; 1000];
        assert_eq!(unrle(&rle(&long, 1), 1, 1000), long);
    }

    #[test]
    fn fitting_keeps_marks_visible() {
        assert_eq!(fit([0, 0, 0], DARK_BG, DARK_TEXT), DARK_TEXT);
        // Ubuntu's orange is fine on both
        assert_eq!(
            fit([0xe9, 0x54, 0x20], DARK_BG, DARK_TEXT),
            [0xe9, 0x54, 0x20]
        );
        // a navy on dark gets lighter, a yellow on white darker
        let navy = fit([0x26, 0x25, 0x77], DARK_BG, DARK_TEXT);
        assert!(contrast(navy, DARK_BG) >= 3.0 && navy[2] > 0x77);
        let yellow = fit([0xff, 0xd2, 0x1e], LIGHT_BG, LIGHT_TEXT);
        assert!(contrast(yellow, LIGHT_BG) >= 3.0 && yellow[0] < 0xff);
    }
}
