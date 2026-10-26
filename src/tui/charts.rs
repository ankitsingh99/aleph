//! Charts for the dashboard's numbers (Usage, the timeline): pure functions
//! that lay out lines, so every size and colour can be tested.
//!
//! * [`share_bar`] + [`legend`]: a whole split into its parts, one row (the
//!   cost breakup under a total).
//! * [`columns`]: stacked columns over time, a value axis on the left, labels
//!   under them.
//! * [`histogram`]: counts in buckets.
//! * [`lines`]: one or more series as braille lines (2 × 4 dots a cell).
//! * [`calendar`]: a value per day, weeks across (GitHub's), [`week_hours`]:
//!   hour × weekday.
//!
//! Series take the theme's own hues ([`series`]), held readable on light
//! themes like every ink; with NO_COLOR they're told apart by glyph
//! (█ ▓ ▒ ░ ▚ ▞), never by colour alone.

use crate::ui::{self, c};
use ratatui::prelude::*;

/// The `i`th series' colour: the theme's hues in a fixed order.
pub fn series(i: usize) -> Color {
    let v = [
        c::BLUE(),
        c::PEACH(),
        c::GREEN(),
        c::MAUVE(),
        c::TEAL(),
        c::PINK(),
        c::YELLOW(),
        c::SKY(),
    ];
    v[i % v.len()]
}

/// The `i`th series' glyph: solid when colour tells them apart, a pattern
/// when there's none (NO_COLOR).
pub fn glyph(i: usize) -> &'static str {
    if !c::mono() {
        return "█";
    }
    ["█", "▓", "▒", "░", "▚", "▞"][i % 6]
}

/// One part of a whole: its name, value and colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub value: f64,
    pub color: Color,
}

/// Cells for each value of a whole `w` wide, by largest remainder (every
/// part over ½ a cell gets one; they add up to `w`).
pub fn apportion(values: &[f64], w: usize) -> Vec<usize> {
    let total: f64 = values.iter().filter(|v| **v > 0.0).sum();
    if total <= 0.0 || w == 0 {
        return vec![0; values.len()];
    }
    let exact: Vec<f64> = values
        .iter()
        .map(|v| v.max(0.0) / total * w as f64)
        .collect();
    let mut cells: Vec<usize> = exact.iter().map(|x| x.floor() as usize).collect();
    let mut left = w.saturating_sub(cells.iter().sum());
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| {
        (exact[*b] - exact[*b].floor()).total_cmp(&(exact[*a] - exact[*a].floor()))
    });
    for i in order {
        if left == 0 {
            break;
        }
        if exact[i] > 0.0 {
            cells[i] += 1;
            left -= 1;
        }
    }
    cells
}

/// The whole as one row `w` wide: each part its share, in its colour.
pub fn share_bar(parts: &[Part], w: usize) -> Line<'static> {
    let cells = apportion(&parts.iter().map(|p| p.value).collect::<Vec<_>>(), w);
    let mut v: Vec<Span<'static>> = parts
        .iter()
        .zip(&cells)
        .enumerate()
        .filter(|(_, (_, n))| **n > 0)
        .map(|(i, (p, n))| Span::styled(glyph(i).repeat(*n), Style::new().fg(p.color)))
        .collect();
    let used: usize = cells.iter().sum();
    if used < w {
        v.push(Span::styled(
            "·".repeat(w - used),
            Style::new().fg(c::LINE()),
        ));
    }
    Line::from(v)
}

/// The parts named, in columns: `■ name  value  share`, as many to a row as
/// fit `w` (`fmt` writes a value).
pub fn legend(parts: &[Part], w: usize, fmt: &dyn Fn(f64) -> String) -> Vec<Line<'static>> {
    let total: f64 = parts.iter().map(|p| p.value.max(0.0)).sum();
    let cells: Vec<(usize, Vec<Span<'static>>)> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pct = if total > 0.0 {
                100.0 * p.value / total
            } else {
                0.0
            };
            let val = fmt(p.value);
            let spans = vec![
                Span::styled(
                    format!("{} ", if c::mono() { glyph(i) } else { "■" }),
                    Style::new().fg(p.color),
                ),
                Span::styled(p.name.clone(), ui::bold()),
                Span::styled(format!("  {val}"), ui::fg(c::ACCENT())),
                Span::styled(format!("  {pct:.0}%"), ui::faint()),
            ];
            (
                ui::width(&p.name) + ui::width(&val) + 9 + format!("{pct:.0}").len(),
                spans,
            )
        })
        .collect();
    let colw = cells.iter().map(|(n, _)| *n).max().unwrap_or(1) + 2;
    let per = (w / colw.max(1)).max(1);
    cells
        .chunks(per)
        .map(|row| {
            let mut v: Vec<Span<'static>> = Vec::new();
            for (n, spans) in row {
                v.extend(spans.iter().cloned());
                v.push(Span::raw(" ".repeat(colw.saturating_sub(*n))));
            }
            Line::from(v)
        })
        .collect()
}

const UP: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Stacked columns: `cols` = (label, a value per series), oldest first;
/// `colors` per series; `w` × `h` cells with the value axis (`fmt`) on the
/// left and labels under. More columns than fit are summed in pairs (and so
/// on); fewer are widened.
pub fn columns(
    cols: &[(String, Vec<f64>)],
    colors: &[Color],
    w: usize,
    h: usize,
    fmt: &dyn Fn(f64) -> String,
) -> Vec<Line<'static>> {
    if h < 2 || w < 10 || cols.is_empty() {
        return Vec::new();
    }
    let rows = h - 1;
    let max = cols
        .iter()
        .map(|(_, v)| v.iter().filter(|x| **x > 0.0).sum::<f64>())
        .fold(0.0, f64::max);
    let axis = [fmt(max), fmt(max / 2.0), fmt(0.0)];
    let aw = axis.iter().map(|s| ui::width(s)).max().unwrap_or(1) + 1;
    let room = w.saturating_sub(aw + 1).max(1);
    // fold neighbours together until they fit, then give each its width
    let mut group = 1;
    while cols.len().div_ceil(group) > room {
        group += 1;
    }
    let shown: Vec<(String, Vec<f64>)> = cols
        .chunks(group)
        .map(|ch| {
            let n = ch[0].1.len();
            let mut v = vec![0.0; n];
            for (_, x) in ch {
                for (a, b) in v.iter_mut().zip(x) {
                    *a += b.max(0.0);
                }
            }
            (ch[0].0.clone(), v)
        })
        .collect();
    let cw = (room / shown.len()).clamp(1, 6);
    let bar = if cw >= 3 { cw - 1 } else { cw };
    let mut out = Vec::new();
    for r in 0..rows {
        // row 0 is the top; each row 8 eighths
        let floor = ((rows - 1 - r) * 8) as f64;
        let label = match r {
            0 => &axis[0],
            _ if r == rows / 2 && rows > 2 => &axis[1],
            _ if r + 1 == rows => &axis[2],
            _ => "",
        };
        let mut v = vec![Span::styled(
            format!("{label:>w$} ", w = aw - 1),
            ui::faint(),
        )];
        for (_, vals) in &shown {
            let total: f64 = vals.iter().sum();
            let eighths = if max > 0.0 {
                total / max * (rows * 8) as f64
            } else {
                0.0
            };
            let here = (eighths - floor).clamp(0.0, 8.0).round() as usize;
            // the colour: the series whose band covers most of this cell
            let mid = (floor + 4.0) / (rows * 8) as f64 * max;
            let mut acc = 0.0;
            let mut who = vals.len().saturating_sub(1);
            for (i, x) in vals.iter().enumerate() {
                acc += x;
                if acc >= mid {
                    who = i;
                    break;
                }
            }
            let col = colors.get(who).copied().unwrap_or(c::ACCENT());
            let g = if c::mono() && here == 8 {
                glyph(who)
            } else {
                UP[here]
            };
            v.push(Span::styled(g.repeat(bar), Style::new().fg(col)));
            if cw > bar {
                v.push(Span::raw(" ".repeat(cw - bar)));
            }
        }
        out.push(Line::from(v));
    }
    // labels under, where they fit
    let mut under = " ".repeat(aw);
    let mut next = 0usize;
    for (i, (l, _)) in shown.iter().enumerate() {
        let at = i * cw;
        if at >= next && at + ui::width(l) <= room {
            under.push_str(&" ".repeat(at - (ui::width(&under) - aw)));
            under.push_str(l);
            next = at + ui::width(l) + 1;
        }
    }
    out.push(Line::from(Span::styled(under, ui::faint())));
    out
}

/// A histogram: (from, to, count) buckets as columns, their ranges under
/// (`fmt` writes a bound).
pub fn histogram(
    buckets: &[(f64, f64, u64)],
    w: usize,
    h: usize,
    color: Color,
    fmt: &dyn Fn(f64) -> String,
) -> Vec<Line<'static>> {
    let cols: Vec<(String, Vec<f64>)> = buckets
        .iter()
        .map(|(a, _, n)| (fmt(*a), vec![*n as f64]))
        .collect();
    columns(&cols, &[color], w, h, &|v| format!("{v:.0}"))
}

/// Braille dots for one cell (2 across, 4 down): bit for (x, y).
fn dot(x: usize, y: usize) -> u32 {
    match (x, y) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (0, 3) => 0x40,
        _ => 0x80,
    }
}

/// Series as braille lines over the same axes: `w` × `h` cells (the value
/// axis, `fmt`, on the left). Where two cross, the later one's colour.
pub fn lines(
    series_: &[(Vec<f64>, Color)],
    w: usize,
    h: usize,
    fmt: &dyn Fn(f64) -> String,
) -> Vec<Line<'static>> {
    lines_to(series_, w, h, fmt, None)
}

/// [`lines`] with the top of the value axis fixed (`top`: 1.0 for shares),
/// else the largest value.
pub fn lines_to(
    series_: &[(Vec<f64>, Color)],
    w: usize,
    h: usize,
    fmt: &dyn Fn(f64) -> String,
    top: Option<f64>,
) -> Vec<Line<'static>> {
    if h < 2 || w < 10 || series_.iter().all(|(s, _)| s.is_empty()) {
        return Vec::new();
    }
    let max = top.unwrap_or_else(|| {
        series_
            .iter()
            .flat_map(|(s, _)| s.iter())
            .copied()
            .filter(|v| v.is_finite())
            .fold(0.0, f64::max)
    });
    let max = max.max(f64::MIN_POSITIVE);
    let axis = [fmt(max), fmt(0.0)];
    let aw = axis.iter().map(|s| ui::width(s)).max().unwrap_or(1) + 1;
    let cw = w.saturating_sub(aw).max(1);
    let (dw, dh) = (cw * 2, h * 4);
    let mut cells: Vec<Vec<(u32, Option<Color>)>> = vec![vec![(0, None); cw]; h];
    for (s, col) in series_ {
        if s.is_empty() {
            continue;
        }
        let n = s.len();
        let mut prev: Option<(usize, usize)> = None;
        for x in 0..dw {
            let i = if n == 1 {
                0
            } else {
                x * (n - 1) / (dw - 1).max(1)
            };
            let v = s[i.min(n - 1)];
            let v = if v.is_finite() { v.max(0.0) } else { 0.0 };
            let y = dh - 1 - ((v / max).min(1.0) * (dh - 1) as f64).round() as usize;
            // join to the last dot so a steep rise stays a line
            let (lo, hi) = match prev {
                Some((_, py)) => (py.min(y), py.max(y)),
                None => (y, y),
            };
            for yy in lo..=hi {
                let c = &mut cells[yy / 4][x / 2];
                c.0 |= dot(x % 2, yy % 4);
                c.1 = Some(*col);
            }
            prev = Some((x, y));
        }
    }
    cells
        .into_iter()
        .enumerate()
        .map(|(r, row)| {
            let label = match r {
                0 => &axis[0],
                _ if r + 1 == h => &axis[1],
                _ => "",
            };
            let mut v = vec![Span::styled(
                format!("{label:>w$} ", w = aw - 1),
                ui::faint(),
            )];
            for (bits, col) in row {
                let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                v.push(match col {
                    Some(col) if bits > 0 => Span::styled(ch.to_string(), Style::new().fg(col)),
                    _ => Span::raw(" "),
                });
            }
            Line::from(v)
        })
        .collect()
}

/// A level glyph for a share of the most (0 nothing, then four steps).
fn level(frac: f64) -> (&'static str, Color) {
    match frac {
        f if f <= 0.0 => ("·", c::LINE()),
        f if f < 0.25 => ("■", c::TEAL()),
        f if f < 0.5 => ("■", c::GREEN()),
        f if f < 0.75 => ("■", c::YELLOW()),
        _ => ("■", c::PEACH()),
    }
}

/// A value per day as weeks across (Monday at the top), the newest week on
/// the right: `days` = (local day number, value), up to `weeks` back from `today`.
pub fn calendar(days: &[(i64, f64)], today: i64, weeks: usize) -> Vec<Line<'static>> {
    let max = days.iter().map(|d| d.1).fold(0.0, f64::max);
    let by: std::collections::HashMap<i64, f64> = days.iter().copied().collect();
    // the Monday of this week (1970-01-01 was a Thursday)
    let monday = today - (today + 3).rem_euclid(7);
    let first = monday - 7 * (weeks.max(1) as i64 - 1);
    const W: [&str; 7] = ["Mo", "  ", "We", "  ", "Fr", "  ", "Su"];
    (0..7)
        .map(|wd| {
            let mut v = vec![Span::styled(format!("{} ", W[wd]), ui::faint())];
            for wk in 0..weeks {
                let d = first + 7 * wk as i64 + wd as i64;
                if d > today {
                    v.push(Span::raw("  "));
                    continue;
                }
                let x = by.get(&d).copied().unwrap_or(0.0);
                let (g, col) = level(if max > 0.0 { x / max } else { 0.0 });
                v.push(Span::styled(format!("{g} "), Style::new().fg(col)));
            }
            Line::from(v)
        })
        .collect()
}

/// Hour × weekday (Monday first): each hour's share of the busiest.
pub fn week_hours(heat: &[[f64; 24]; 7], w: usize) -> Vec<Line<'static>> {
    let max = heat.iter().flatten().copied().fold(0.0, f64::max);
    let cell = if w >= 3 + 24 * 2 { 2 } else { 1 };
    const W: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
    let mut out: Vec<Line<'static>> = (0..7)
        .map(|d| {
            let mut v = vec![Span::styled(format!("{} ", W[d]), ui::faint())];
            for x in heat[d] {
                let (g, col) = level(if max > 0.0 { x / max } else { 0.0 });
                v.push(Span::styled(
                    if cell == 2 {
                        format!("{g} ")
                    } else {
                        g.to_string()
                    },
                    Style::new().fg(col),
                ));
            }
            Line::from(v)
        })
        .collect();
    let mut axis = String::from("   ");
    for h in (0..24).step_by(6) {
        let s = format!("{h:02}");
        axis.push_str(&format!("{s:<w$}", w = 6 * cell));
    }
    out.push(Line::from(Span::styled(axis, ui::faint())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn shares_add_up_to_the_width() {
        for w in [0usize, 1, 7, 40, 121] {
            let c = apportion(&[51.0, 24.0, 18.0, 7.0], w);
            assert_eq!(c.iter().sum::<usize>(), w, "{w}");
        }
        assert_eq!(apportion(&[0.0, 0.0], 10), vec![0, 0]);
        assert_eq!(apportion(&[1.0, 0.0, 1.0], 3).iter().sum::<usize>(), 3);
        let parts = vec![
            Part {
                name: "work".into(),
                value: 210.0,
                color: Color::Blue,
            },
            Part {
                name: "home".into(),
                value: 98.0,
                color: Color::Red,
            },
        ];
        let l = share_bar(&parts, 30);
        assert_eq!(ui::width(&text(&l)), 30);
        let lg = legend(&parts, 80, &|v| format!("${v:.0}"));
        assert_eq!(lg.len(), 1);
        assert!(text(&lg[0]).contains("work  $210  68%"), "{}", text(&lg[0]));
        assert_eq!(
            legend(&parts, 20, &|v| format!("{v}")).len(),
            2,
            "narrow: one a row"
        );
    }

    #[test]
    fn charts_fit_any_size() {
        let cols: Vec<(String, Vec<f64>)> = (0..40)
            .map(|i| (format!("d{i}"), vec![i as f64, (40 - i) as f64]))
            .collect();
        for (w, h) in [(0, 0), (9, 1), (10, 2), (30, 6), (80, 12), (300, 40)] {
            let c = columns(&cols, &[Color::Blue, Color::Red], w, h, &|v| {
                format!("{v:.0}")
            });
            for l in &c {
                assert!(ui::width(&text(l)) <= w.max(1), "{w}×{h}: {}", text(l));
            }
            if w >= 10 && h >= 2 {
                assert_eq!(c.len(), h);
            }
            let ln = lines(&[(vec![1.0, 5.0, 2.0, 9.0], Color::Green)], w, h, &|v| {
                format!("{v:.0}")
            });
            for l in &ln {
                assert!(ui::width(&text(l)) <= w.max(1));
            }
            let hs = histogram(
                &[(0.0, 0.1, 3), (0.1, 0.3, 9), (0.3, 1.0, 1)],
                w,
                h,
                Color::Blue,
                &|v| format!("{v}"),
            );
            assert!(hs.iter().all(|l| ui::width(&text(l)) <= w.max(1)));
        }
        // a rising line reaches the top row at its end
        let ln = lines(&[(vec![0.0, 10.0], Color::Green)], 20, 4, &|v| {
            format!("{v:.0}")
        });
        assert!(text(&ln[0])
            .trim_end()
            .ends_with(|c: char| (c as u32) >= 0x2800));
        assert!(columns(&[], &[], 40, 8, &|v| format!("{v}")).is_empty());
        assert!(
            lines(
                &[(vec![f64::NAN, f64::INFINITY], Color::Red)],
                40,
                5,
                &|v| format!("{v}")
            )
            .len()
                == 5
        );
    }

    #[test]
    fn calendars_and_week_hours() {
        let today = 20_752; // a Monday
        let cal = calendar(&[(today, 5.0), (today - 1, 1.0)], today, 12);
        assert_eq!(cal.len(), 7);
        assert!(
            text(&cal[0]).trim_end().ends_with('■'),
            "today, Monday, last column"
        );
        let mut heat = [[0.0; 24]; 7];
        heat[0][9] = 10.0;
        let wh = week_hours(&heat, 60);
        assert_eq!(wh.len(), 8);
        assert!(text(&wh[0]).contains('■'));
    }
}
