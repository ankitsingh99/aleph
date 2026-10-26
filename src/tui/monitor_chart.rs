//! Braille charts for the Monitor (btop style): every history is stretched
//! to the full width, so a few samples never leave most of a chart empty.
//! Each cell holds 2×4 dots. A series is a filled area (coloured by height
//! on the heat scale, or in its own colour) or a thin line. Areas can stack
//! on a base series. There's an optional y scale in a left gutter and a time
//! axis along the bottom row. Under NO_COLOR, overlaid series stay apart by
//! texture: dashed lines and sparser fills.
//!
//! Missing values (NaN, ±inf, or a series shorter than the others, aligned
//! at the newest sample) are simply not drawn.

use crate::ui::{self, c};
use ratatui::prelude::*;

/// How a series is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// filled from the base (or 0) up to the value
    Area,
    /// a one-dot line through the values
    Line,
}

/// A series' colour.
#[derive(Clone, Copy, Debug)]
pub enum Paint {
    Solid(Color),
    /// by height: the heat scale, cool at the bottom, red at the top
    Heat,
}

/// One series. Values are in the chart's units (0..`Plot::max`).
pub struct Ser<'a> {
    pub data: &'a [f64],
    pub kind: Kind,
    pub paint: Paint,
    /// an area filled from these values instead of from 0 (stacking)
    pub base: Option<&'a [f64]>,
}

impl<'a> Ser<'a> {
    pub fn area(data: &'a [f64], paint: Paint) -> Ser<'a> {
        Ser {
            data,
            kind: Kind::Area,
            paint,
            base: None,
        }
    }
    pub fn line(data: &'a [f64], col: Color) -> Ser<'a> {
        Ser {
            data,
            kind: Kind::Line,
            paint: Paint::Solid(col),
            base: None,
        }
    }
    pub fn on(mut self, base: &'a [f64]) -> Ser<'a> {
        self.base = Some(base);
        self
    }
}

/// A chart: series (later ones drawn on top), the value at the top edge, and
/// what goes around it.
pub struct Plot<'a> {
    pub series: Vec<Ser<'a>>,
    pub max: f64,
    /// y-scale labels at fractions of the height (0 = bottom, 1 = top)
    pub y: Vec<(f64, String)>,
    /// seconds per sample: draws a time axis along the bottom row
    pub period: Option<f64>,
    /// grow downwards from the top edge (the lower half of a mirrored chart)
    pub flip: bool,
    /// the y-label gutter is at least this wide (so stacked charts line up)
    pub gutter: usize,
}

impl<'a> Plot<'a> {
    pub fn new(series: Vec<Ser<'a>>, max: f64) -> Plot<'a> {
        Plot {
            series,
            max,
            y: vec![],
            period: None,
            flip: false,
            gutter: 0,
        }
    }
    /// `100%` · `50%` · `0` labels for a fraction chart (of its `max`).
    pub fn pct_axis(self) -> Plot<'a> {
        self.axis(|v| {
            if v <= 0.0 {
                "0".into()
            } else {
                format!("{:.0}%", v * 100.0)
            }
        })
    }
    /// top · middle · bottom labels from a formatter (rates, loads, …); an
    /// empty label is left out.
    pub fn axis(mut self, fmt: impl Fn(f64) -> String) -> Plot<'a> {
        let m = self.max;
        self.y = vec![(1.0, fmt(m)), (0.5, fmt(m / 2.0)), (0.0, fmt(0.0))];
        self.y.retain(|(_, t)| !t.is_empty());
        self
    }
    pub fn time(mut self, period: f64) -> Plot<'a> {
        self.period = Some(period);
        self
    }
    pub fn flipped(mut self) -> Plot<'a> {
        self.flip = true;
        self
    }
    pub fn gutter(mut self, w: usize) -> Plot<'a> {
        self.gutter = w;
        self
    }
    /// The gutter its labels need.
    pub fn label_w(&self) -> usize {
        self.y.iter().map(|(_, t)| ui::width(t)).max().unwrap_or(0)
    }
}

/// The top of a percentage chart for a peak (fraction): 10 %, 20 %, 50 % or
/// 100 %, so a quiet machine's history isn't a flat line at the bottom.
pub fn pct_top(peak: f64) -> f64 {
    let p = if peak.is_finite() { peak.max(0.0) } else { 1.0 };
    [0.1, 0.2, 0.5, 1.0]
        .into_iter()
        .find(|m| p <= m * 0.95)
        .unwrap_or(1.0)
}

/// A missing value: NaN or ±inf.
fn val(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// `data` (aligned at the newest of `n` samples) as `w` columns: the bucket
/// maximum when there are more samples than columns, linear interpolation
/// when fewer. None = nothing to draw there.
pub fn resample(data: &[f64], n: usize, w: usize) -> Vec<Option<f64>> {
    let m = data.len().min(n);
    if m == 0 || w == 0 || n == 0 {
        return vec![None; w];
    }
    let pad = n - m;
    let data = &data[data.len() - m..];
    let at = |i: usize| -> Option<f64> {
        if i < pad {
            None
        } else {
            val(data[i - pad])
        }
    };
    if n == 1 {
        return vec![at(0); w];
    }
    if n >= w {
        (0..w)
            .map(|x| {
                let lo = x * n / w;
                let hi = ((x + 1) * n / w).clamp(lo + 1, n);
                (lo..hi).filter_map(at).reduce(f64::max)
            })
            .collect()
    } else {
        (0..w)
            .map(|x| {
                let t = if w > 1 {
                    x as f64 * (n - 1) as f64 / (w - 1) as f64
                } else {
                    (n - 1) as f64
                };
                let i = (t.floor() as usize).min(n - 1);
                let fr = t - i as f64;
                match (at(i), at((i + 1).min(n - 1))) {
                    (Some(a), Some(b)) => Some(a + (b - a) * fr),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) if fr > 0.5 => Some(b),
                    _ => None,
                }
            })
            .collect()
    }
}

/// The dot bit for column `dx` (0..2) and row `dy` (0..4, from the top) of a cell.
const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Under NO_COLOR, series `i` keeps only some dots so overlays stay apart.
fn texture(i: usize, kind: Kind, x: usize, y: usize) -> bool {
    if !c::mono() || i == 0 {
        return true;
    }
    match kind {
        Kind::Line => x % (2 * i + 2) < 2,
        Kind::Area => match i {
            1 => (x + y).is_multiple_of(2),
            _ => x.is_multiple_of(2) && y.is_multiple_of(2),
        },
    }
}

/// Smooth heat for chart fills and gradient bars: teal → green → yellow →
/// red as `frac` rises (palette colours, interpolated when they're RGB).
pub fn grad(frac: f64) -> Color {
    let f = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let stops = [
        (0.0, c::TEAL()),
        (0.4, c::GREEN()),
        (0.7, c::YELLOW()),
        (1.0, c::RED()),
    ];
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if f <= b {
            let t = ((f - a) / (b - a)).clamp(0.0, 1.0);
            return mix(ca, cb, t);
        }
    }
    c::RED()
}

fn mix(a: Color, b: Color, t: f64) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ if t < 0.5 => a,
        _ => b,
    }
}

/// `90s`, `5m`, `2h`.
pub fn span(secs: f64) -> String {
    let s = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    if s < 90.0 {
        format!("{s:.0}s")
    } else if s < 90.0 * 60.0 {
        format!("{:.0}m", s / 60.0)
    } else {
        format!("{:.0}h", s / 3600.0)
    }
}

/// A round-ish number ≥ `v` for an auto-scaled axis (1, 1.2, 1.5, 2, 2.5,
/// 3, 4, 5, 6, 8 × 10ⁿ): close enough above the peak that the chart uses
/// its height.
pub fn nice_max(v: f64) -> f64 {
    let v = if v.is_finite() && v > 0.0 { v } else { 1.0 };
    let e = 10f64.powf(v.log10().floor());
    for m in [1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0] {
        if m * e >= v * 0.999_999 {
            return m * e;
        }
    }
    10.0 * e
}

/// The samples the chart spans (the longest series).
fn samples(p: &Plot) -> usize {
    p.series.iter().map(|s| s.data.len()).max().unwrap_or(0)
}

/// Draw `p` into `r` (clipped to the frame).
pub fn plot(f: &mut Frame, r: Rect, p: &Plot) {
    let r = r.intersection(f.area());
    if r.width == 0 || r.height == 0 {
        return;
    }
    let n = samples(p);
    // the gutter: right-aligned labels and the axis line
    let lw = p.label_w().max(if p.y.is_empty() { 0 } else { p.gutter });
    let gw = if lw > 0 && r.width as usize >= lw + 12 && r.height >= 3 {
        lw + 1
    } else {
        0
    };
    // the lower half of a mirror keeps its axis (the middle line) longer
    let xa = p.period.is_some() && r.height >= if p.flip { 2 } else { 5 };
    let pr = Rect {
        x: r.x + gw as u16,
        y: r.y + u16::from(xa && p.flip),
        width: r.width - gw as u16,
        height: r.height - u16::from(xa),
    };
    let faint = Style::new().fg(c::LINE());
    let buf = f.buffer_mut();
    if gw > 0 {
        let rows = pr.height as usize;
        let row_of = |frac: f64| -> usize {
            let k = ((1.0 - frac.clamp(0.0, 1.0)) * (rows - 1) as f64).round() as usize;
            if p.flip {
                rows - 1 - k
            } else {
                k
            }
        };
        for j in 0..rows {
            buf.set_string(r.x + lw as u16, pr.y + j as u16, "│", faint);
        }
        for (frac, t) in p.y.iter().filter(|(_, t)| !t.is_empty()) {
            let j = row_of(*frac);
            let x = r.x + (lw - ui::width(t)) as u16;
            buf.set_string(x, pr.y + j as u16, t, ui::faint());
            buf.set_string(r.x + lw as u16, pr.y + j as u16, "┤", faint);
        }
    }
    if xa {
        let y = if p.flip { r.y } else { r.bottom() - 1 };
        time_axis(buf, Rect { y, height: 1, ..r }, gw, n, p);
    }
    if n < 2 {
        let msg = "collecting history…";
        if pr.width as usize > msg.len() && pr.height > 0 {
            let x = pr.x + (pr.width - msg.len() as u16) / 2;
            buf.set_string(x, pr.y + (pr.height - 1) / 2, msg, ui::faint());
        }
        return;
    }
    dots(buf, pr, p, n);
}

fn time_axis(buf: &mut Buffer, r: Rect, gw: usize, n: usize, p: &Plot) {
    let faint = Style::new().fg(c::LINE());
    let w = r.width as usize;
    if gw > 0 {
        let corner = if p.flip { "┌" } else { "└" };
        buf.set_string(r.x + gw as u16 - 1, r.y, corner, faint);
    }
    let aw = w - gw;
    buf.set_string(r.x + gw as u16, r.y, "─".repeat(aw), faint);
    let secs = n.saturating_sub(1) as f64 * p.period.unwrap_or(1.0);
    if n < 2 || aw < 12 {
        return;
    }
    let mut put = |x: usize, t: String| {
        let tw = ui::width(&t) + 2;
        if x + tw <= w {
            buf.set_string(r.x + x as u16, r.y, format!(" {t} "), ui::faint());
        }
    };
    put(gw, format!("−{}", span(secs)));
    if aw >= 40 {
        let mid = format!("−{}", span(secs / 2.0));
        put(gw + aw / 2 - (ui::width(&mid) + 2) / 2, mid);
    }
    put(w - 5, "now".into());
}

fn dots(buf: &mut Buffer, pr: Rect, p: &Plot, n: usize) {
    let (cw, ch) = (pr.width as usize, pr.height as usize);
    let (wd, hd) = (cw * 2, ch * 4);
    let max = if p.max.is_finite() && p.max > 0.0 {
        p.max
    } else {
        1.0
    };
    let cells = cw * ch;
    let mut mask = vec![0u8; cells];
    let k = p.series.len().max(1);
    // dots per series per cell: the cell takes the colour of the series with
    // the most dots in it (later series win ties)
    let mut count = vec![0u8; cells * k];
    for (si, s) in p.series.iter().enumerate() {
        let v = resample(s.data, n, wd);
        let b = s.base.map(|b| resample(b, n, wd));
        let level = |x: f64| (x / max).clamp(0.0, 1.0) * hd as f64;
        let mut set = |x: usize, yb: usize| {
            if yb >= hd || !texture(si, s.kind, x, yb) {
                return;
            }
            let yt = if p.flip { yb } else { hd - 1 - yb };
            let cell = (yt / 4) * cw + x / 2;
            mask[cell] |= BITS[yt % 4][x % 2];
            count[cell * k + si] = count[cell * k + si].saturating_add(1);
        };
        match s.kind {
            Kind::Area => {
                for (x, y) in v.iter().enumerate() {
                    let Some(y) = y else { continue };
                    let lo = b
                        .as_ref()
                        .and_then(|b| b[x])
                        .map_or(0, |b| level(b).round() as usize);
                    let mut top = level(*y).round() as usize;
                    if *y > 0.0 && top <= lo && b.is_none() {
                        top = lo + 1; // a sliver, so small values still show
                    }
                    for yb in lo..top.min(hd) {
                        set(x, yb);
                    }
                }
            }
            Kind::Line => {
                let yi = |y: f64| (y / max).clamp(0.0, 1.0) * (hd - 1) as f64;
                for x in 0..wd {
                    let Some(y) = v[x] else { continue };
                    let y = yi(y);
                    // reach halfway to each neighbour: a continuous thin line
                    let (mut lo, mut hi) = (y, y);
                    for nb in [x.checked_sub(1), Some(x + 1)].into_iter().flatten() {
                        if let Some(Some(o)) = v.get(nb) {
                            let m = (y + yi(*o)) / 2.0;
                            lo = lo.min(m);
                            hi = hi.max(m);
                        }
                    }
                    for yb in lo.round() as usize..=hi.round() as usize {
                        set(x, yb);
                    }
                }
            }
        }
    }
    for row in 0..ch {
        // the row's value on the heat scale (its middle, in chart units:
        // a 10 % chart stays cool all the way up)
        let frac = if p.flip {
            (row as f64 + 0.5) / ch as f64
        } else {
            1.0 - (row as f64 + 0.5) / ch as f64
        } * max.min(1.0);
        for col in 0..cw {
            let cell = row * cw + col;
            let m = mask[cell];
            if m == 0 {
                continue;
            }
            // a line through an area keeps its colour; else the area with most dots
            let cnt = &count[cell * k..cell * k + k];
            let line = (0..k)
                .rev()
                .find(|&si| cnt[si] > 0 && p.series[si].kind == Kind::Line);
            let best = line.unwrap_or_else(|| {
                let mut b = 0;
                for si in 0..k {
                    if cnt[si] > 0 && cnt[si] >= cnt[b] {
                        b = si;
                    }
                }
                b
            });
            let col_c = match p.series.get(best).map(|s| s.paint) {
                Some(Paint::Solid(cc)) => cc,
                Some(Paint::Heat) | None => grad(frac),
            };
            if let Some(cl) = buf.cell_mut((pr.x + col as u16, pr.y + row as u16)) {
                cl.set_char(char::from_u32(0x2800 + m as u32).unwrap_or(' '))
                    .set_fg(col_c);
            }
        }
    }
}

/// A one-row braille sparkline (4 levels × 2 samples per cell), stretched to
/// `w` cells; `max` is full height. Coloured by `paint` (Heat: by value).
pub fn spark(data: &[f64], w: usize, max: f64, paint: Paint) -> Vec<Span<'static>> {
    if w == 0 {
        return vec![];
    }
    let n = data.len();
    if n < 2 {
        return vec![Span::raw(" ".repeat(w))];
    }
    let max = if max.is_finite() && max > 0.0 {
        max
    } else {
        1.0
    };
    let v = resample(data, n, w * 2);
    let mut out: Vec<Span<'static>> = Vec::new();
    let (mut cur, mut cur_c) = (String::new(), None::<Color>);
    for cell in 0..w {
        let mut m = 0u8;
        let mut peak = 0.0f64;
        for dx in 0..2 {
            let Some(y) = v[cell * 2 + dx] else { continue };
            let q = (y / max).clamp(0.0, 1.0);
            peak = peak.max(q);
            let mut top = (q * 4.0).round() as usize;
            if y > 0.0 && top == 0 {
                top = 1;
            }
            for yb in 0..top.min(4) {
                m |= BITS[3 - yb][dx];
            }
        }
        let col = match paint {
            Paint::Solid(cc) => cc,
            Paint::Heat => grad(peak),
        };
        let ch = if m == 0 {
            '⠀'
        } else {
            char::from_u32(0x2800 + m as u32).unwrap_or(' ')
        };
        if cur_c != Some(col) && !cur.is_empty() {
            out.push(Span::styled(
                std::mem::take(&mut cur),
                Style::new().fg(cur_c.unwrap_or(col)),
            ));
        }
        cur_c = Some(col);
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(Span::styled(
            cur,
            Style::new().fg(cur_c.unwrap_or(c::TEXT())),
        ));
    }
    out
}

/// A legend swatch for series `i` (a dash pattern under NO_COLOR).
pub fn swatch(i: usize, col: Color) -> Span<'static> {
    let g = if c::mono() {
        ["━━", "╍╍", "┅┅", "┈┈"][i.min(3)]
    } else {
        "━━"
    };
    Span::styled(g, Style::new().fg(col))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn resample_stretches_and_buckets() {
        let d = [0.0, 1.0];
        let v = resample(&d, 2, 5);
        assert_eq!(v.len(), 5);
        assert_eq!(v[0], Some(0.0));
        assert_eq!(v[4], Some(1.0));
        assert!((v[2].unwrap() - 0.5).abs() < 1e-9);
        // more samples than columns: bucket maxima
        let d: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let v = resample(&d, 100, 10);
        assert_eq!(v[0], Some(9.0));
        assert_eq!(v[9], Some(99.0));
        // a shorter series is aligned at the newest sample
        let v = resample(&[5.0, 5.0], 10, 10);
        assert_eq!(v[0], None);
        assert_eq!(v[9], Some(5.0));
        // missing values stay missing
        let v = resample(&[f64::NAN, f64::INFINITY], 2, 4);
        assert!(v.iter().all(|x| x.is_none()));
        assert!(resample(&[], 0, 3).iter().all(|x| x.is_none()));
    }

    #[test]
    fn nice_numbers() {
        assert_eq!(nice_max(0.0), 1.0);
        assert_eq!(nice_max(0.7), 0.8);
        assert_eq!(nice_max(1.3), 1.5);
        assert_eq!(nice_max(2.2), 2.5);
        assert_eq!(nice_max(3.0), 3.0);
        assert_eq!(nice_max(70.0), 80.0);
        assert_eq!(nice_max(f64::NAN), 1.0);
        assert_eq!(span(30.0), "30s");
        assert_eq!(span(299.0), "5m");
        assert_eq!(span(7200.0), "2h");
    }

    #[test]
    fn a_short_history_fills_the_width() {
        let mut t = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let d = [0.5, 0.5, 0.5];
        t.draw(|f| {
            plot(
                f,
                f.area(),
                &Plot::new(vec![Ser::area(&d, Paint::Heat)], 1.0),
            )
        })
        .unwrap();
        let b = t.backend().buffer();
        // the bottom row is filled from the first column to the last
        for x in 0..40 {
            assert_ne!(b[(x, 5)].symbol(), " ", "column {x}");
        }
        // and the top row is empty (50%)
        assert!((0..40).all(|x| b[(x, 0)].symbol() == " "));
    }

    #[test]
    fn plots_survive_anything() {
        let weird = [f64::NAN, -5.0, 1e300, f64::INFINITY, 0.3];
        for (w, h) in [(0, 0), (1, 1), (3, 2), (13, 5), (80, 20)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| {
                let p = Plot::new(
                    vec![
                        Ser::area(&weird, Paint::Heat),
                        Ser::line(&weird, Color::Red).on(&weird),
                    ],
                    f64::NAN,
                )
                .pct_axis()
                .time(1.0);
                plot(f, f.area(), &p);
                plot(f, Rect::new(500, 500, 9, 9), &p);
                let _ = spark(&weird, w as usize, 0.0, Paint::Heat);
            })
            .unwrap();
        }
        assert_eq!(spark(&[0.0, 1.0], 3, 1.0, Paint::Heat).len(), 3);
    }
}
