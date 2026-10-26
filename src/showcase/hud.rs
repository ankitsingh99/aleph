//! The cockpit HUD (`hud`, a part of `showcase.flair`): a canopy's frame
//! round the dashboard's content, and live readouts in it.
//!
//! * Brackets — faint heavy corners where the content meets the rule under
//!   the tabs and the footer, their arms reaching along those edges and down
//!   (or up) the side columns. An arm stops at the first thing drawn: along
//!   the rule only rule is replaced (never a tab's underline), elsewhere
//!   only blank cells are used.
//! * Readouts — set into the rule's free end, beside the top-right bracket:
//!   machines up of all, sessions working and waiting for you, today's
//!   spend (`usage::now::cached`, as the daemon keeps it) and a pulsar's
//!   heartbeat (PSR B1919+21 turns every 1.337 s). Shorter forms when room
//!   is short, none when even the heartbeat doesn't fit.
//!
//! Foreground only, a few dozen cells a frame.

use super::flair::{self, Part};
use super::fx;
use super::Gauges;
use crate::config::Config;
use crate::ui::{self, c};
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use std::sync::OnceLock;
use std::time::Instant;

/// PSR B1919+21's period, the first pulsar found (1967).
const PULSE_MS: u128 = 1337;

fn clock() -> u128 {
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis()
}

/// How far a bracket's arm reaches along an edge `w` cells wide.
fn arm(w: u16) -> u16 {
    (w / 14).clamp(2, 8)
}

/// Is this cell the rule's own (a plain line, or nothing)?
fn rule_cell(buf: &Buffer, x: u16, y: u16) -> bool {
    buf.cell((x, y))
        .is_some_and(|c| !c.skip && matches!(c.symbol(), "─" | " ") && c.bg == Color::Reset)
}

/// The canopy's corners round `body` (the content: the rule under the tabs
/// is the row above it, the footer the row below).
pub fn brackets(cfg: &Config, buf: &mut Buffer, body: Rect) {
    if !flair::wants(cfg, Part::Hud) || body.width < 24 || body.height < 6 || body.y == 0 {
        return;
    }
    let area = buf.area;
    let (top, bottom) = (body.y - 1, body.bottom());
    if bottom >= area.bottom() {
        return;
    }
    let (left, right) = (body.left(), body.right() - 1);
    let (la, lv) = (arm(body.width), (body.height / 7).clamp(1, 3));
    let col = c::FAINT();
    for (x0, y0, sx, sy, corner, end_h, end_v, rule) in [
        (left, top, 1i32, 1i32, "┏", "╸", "╹", true),
        (right, top, -1, 1, "┓", "╺", "╹", true),
        (left, bottom, 1, -1, "┗", "╸", "╻", false),
        (right, bottom, -1, -1, "┛", "╺", "╻", false),
    ] {
        let free = if rule {
            rule_cell(buf, x0, y0)
        } else {
            fx::blank(buf, x0, y0)
        };
        if !free {
            continue;
        }
        let h = reach(buf, (x0, y0), (sx, 0), la, rule);
        let v = reach(buf, (x0, y0), (0, sy), lv, false);
        let mut lay = |x: i32, y: i32, g: &str| {
            if let Some(cell) = buf.cell_mut((x as u16, y as u16)) {
                cell.set_symbol(g).set_fg(col);
            }
        };
        lay(x0 as i32, y0 as i32, corner);
        for i in 1..=h as i32 {
            let g = if i == h as i32 { end_h } else { "━" };
            lay(x0 as i32 + sx * i, y0 as i32, g);
        }
        for i in 1..=v as i32 {
            let g = if i == v as i32 { end_v } else { "┃" };
            lay(x0 as i32, y0 as i32 + sy * i, g);
        }
    }
}

/// How far an arm from `at` may go along `step`, up to `n`: along the rule
/// as far as it's rule; elsewhere over blank cells, stopping a cell short of
/// whatever's drawn (so it never touches it; the screen's edge is fine).
fn reach(buf: &Buffer, at: (u16, u16), step: (i32, i32), n: u16, rule: bool) -> u16 {
    let a = buf.area;
    let free = |i: i32| {
        let (x, y) = (at.0 as i32 + step.0 * i, at.1 as i32 + step.1 * i);
        let inside = x >= a.left() as i32
            && x < a.right() as i32
            && y >= a.top() as i32
            && y < a.bottom() as i32;
        inside.then(|| {
            if rule {
                rule_cell(buf, x as u16, y as u16)
            } else {
                fx::blank(buf, x as u16, y as u16)
            }
        })
    };
    let mut len = 0;
    for i in 1..=n as i32 {
        if free(i) != Some(true) {
            break;
        }
        len = i as u16;
    }
    if !rule && free(len as i32 + 1) == Some(false) {
        len = len.saturating_sub(1);
    }
    len
}

/// The heartbeat at `ms`: a flash as the beam sweeps past, a fainter one
/// for the other beam, a dot between.
fn heartbeat(ms: u128) -> Span<'static> {
    let phase = (ms % PULSE_MS) as f64 / PULSE_MS as f64;
    let beam = (205, 225, 255);
    if c::mono() {
        return Span::styled(if phase < 0.09 { "✸" } else { "·" }, ui::bold());
    }
    match phase {
        p if p < 0.09 => Span::styled(
            "✸",
            Style::new()
                .fg(fx::glow(beam, 1.0))
                .add_modifier(Modifier::BOLD),
        ),
        p if (0.5..0.56).contains(&p) => Span::styled("✦", ui::fg(fx::glow(beam, 0.7))),
        _ => Span::styled("·", ui::fg(fx::glow(beam, 0.45))),
    }
}

/// The readouts at `level` of shortness (0: with their words; 1: numbers;
/// 2: machines and what waits; 3: the heartbeat alone).
pub fn line(g: &Gauges, cost: Option<(f64, bool)>, ms: u128, level: u8) -> Vec<Span<'static>> {
    let num = |s: String| Span::styled(s, Style::new().fg(c::TEXT()).add_modifier(Modifier::BOLD));
    let word = |s: &'static str| Span::styled(s, ui::faint());
    let gap = || Span::raw("  ");
    let mut v = Vec::new();
    if level <= 2 {
        let mcol = if g.up == g.machines {
            c::DONE()
        } else if g.up == 0 {
            c::DOWN()
        } else {
            c::YELLOW()
        };
        v.extend([
            Span::styled("⟡ ", ui::fg(mcol)),
            num(format!("{}/{}", g.up, g.machines)),
        ]);
        if level == 0 {
            v.push(word(" up"));
        }
        v.push(gap());
    }
    if level <= 1 {
        v.extend([
            Span::styled("✻ ", ui::fg(c::WORKING())),
            num(g.working.to_string()),
        ]);
        if level == 0 {
            v.push(word(" working"));
        }
        v.push(gap());
    }
    if level <= 2 {
        let ncol = if g.needs > 0 { c::NEEDS() } else { c::FAINT() };
        v.extend([
            Span::styled(format!("{} ", crate::look::needs()), ui::fg(ncol)),
            num(g.needs.to_string()),
        ]);
        if level == 0 {
            v.push(word(if g.needs == 1 {
                " needs you"
            } else {
                " need you"
            }));
        }
        v.push(gap());
    }
    if let (true, Some((cost, est))) = (level <= 1, cost) {
        // the header's own figure (scaled or hidden with `showcase.money`)
        let s = crate::usage::dash::money_short(cost, est);
        v.push(Span::styled(
            s,
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ));
        if level == 0 {
            v.push(word(" today"));
        }
        v.push(gap());
    }
    v.push(heartbeat(ms));
    v
}

fn width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| ui::width(&s.content)).sum::<usize>() as u16
}

/// The bottom status bar `r`, right of its first `used` cells (the key
/// hints): the readouts, right aligned, the longest form that fits.
pub fn readouts(cfg: &Config, buf: &mut Buffer, r: Rect, used: u16, g: impl FnOnce() -> Gauges) {
    let r = r.intersection(buf.area);
    if !flair::wants(cfg, Part::Hud) || r.width < 12 || r.height == 0 {
        return;
    }
    let g = g();
    let now = crate::usage::now::cached();
    let at = |p: &str| now.as_ref().and_then(|u| u.pointer(p).cloned());
    let cost = at("/total/today/cost")
        .and_then(|v| v.as_f64())
        .filter(|c| *c > 0.0)
        .map(|c| {
            (
                c,
                at("/total/today/estimated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )
        });
    place(buf, r, used, &g, cost, clock());
}

/// How much of the status bar the readouts want kept for them (their short
/// form), so the key hints leave it free: 0 when they're off.
pub fn room(cfg: &Config, g: &Gauges) -> u16 {
    if !flair::wants(cfg, Part::Hud) {
        return 0;
    }
    width(&line(g, Some((0.85, true)), 0, 1)) + 3
}

/// Lay the readouts into the status bar `r` past its first `used` cells
/// (pure, given the time — tested): the longest form that fits, right
/// aligned, two cells clear of the hints. None when even the heartbeat
/// doesn't fit.
pub fn place(
    buf: &mut Buffer,
    r: Rect,
    used: u16,
    g: &Gauges,
    cost: Option<(f64, bool)>,
    ms: u128,
) -> Option<u16> {
    let room = r.width.saturating_sub(used + 3);
    let spans = (0..=3)
        .map(|lv| line(g, cost, ms, lv))
        .find(|s| width(s) <= room)?;
    let w = width(&spans);
    let x = r.right().saturating_sub(w + 1);
    buf.set_line(x, r.y, &Line::from(spans), w);
    Some(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn row(b: &Buffer, y: u16) -> String {
        (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()
    }

    /// A dashboard-like frame: header, tabs over their rule, content,
    /// footer hints.
    fn frame(w: u16, h: u16) -> (Buffer, Rect) {
        let mut b = Buffer::empty(Rect::new(0, 0, w, h));
        b.set_string(0, 0, " ℵ aleph   ● 2/2 machines", Style::new());
        b.set_string(0, 1, " 1 Fleet  2 Monitor", Style::new());
        b.set_string(0, 2, "─".repeat(w as usize), Style::new());
        b.set_string(1, 2, "━━━━━━━━━", Style::new());
        b.set_string(1, 3, "╭ Machines ─────────╮", Style::new());
        b.set_string(1, h - 1, "enter focus  a attach", Style::new());
        (b, Rect::new(0, 3, w, h - 4))
    }

    #[test]
    fn brackets_frame_the_content_and_never_cover_it() {
        let cfg = Config::in_memory(&[]);
        let (mut b, body) = frame(100, 30);
        let before = b.clone();
        brackets(&cfg, &mut b, body);
        let top = row(&b, 2);
        // left: only the rule's own cell (the tab's underline stays)
        assert!(top.starts_with("┏━━━━━━━━━──"), "{top}");
        // right: a corner and its arm along the rule
        assert!(top.ends_with("─╺━━━━━━┓"), "{top}");
        // down the side columns, until something's drawn
        assert_eq!(b[(99, 3)].symbol(), "┃");
        assert_eq!(b[(0, 3)].symbol(), "┃");
        // the footer's blank ends
        let foot = row(&b, 29);
        assert!(foot.starts_with("┗enter"), "{foot}");
        assert!(foot.trim_end().ends_with("━━┛"), "{foot}");
        // everything that was drawn is still there
        for y in 0..30 {
            for x in 0..100 {
                let (was, now) = (&before[(x, y)], &b[(x, y)]);
                if was.symbol() != " " && was.symbol() != "─" {
                    assert_eq!(was.symbol(), now.symbol(), "({x},{y})");
                }
                assert_eq!(now.bg, Color::Reset);
            }
        }
        // off: nothing
        let (mut b, body) = frame(100, 30);
        brackets(&Config::in_memory(&["showcase.flair=header"]), &mut b, body);
        assert_eq!(b, frame(100, 30).0);
    }

    #[test]
    fn readouts_fit_the_room_or_go() {
        let g = Gauges {
            up: 4,
            machines: 5,
            working: 3,
            needs: 1,
        };
        let text = |s: &[Span]| s.iter().map(|s| s.content.to_string()).collect::<String>();
        assert_eq!(
            text(&line(&g, Some((12.4, true)), 0, 0)),
            "⟡ 4/5 up  ✻ 3 working  ▲ 1 needs you  $12.40* today  ✸"
        );
        assert_eq!(
            text(&line(&g, Some((12.4, false)), 0, 1)),
            "⟡ 4/5  ✻ 3  ▲ 1  $12.40  ✸"
        );
        assert_eq!(text(&line(&g, None, 700, 2)), "⟡ 4/5  ▲ 1  ✦");
        assert_eq!(text(&line(&g, None, 300, 3)), "·");
        // wide: the long form, right-aligned in the status bar after the hints
        let (mut b, _) = frame(140, 20);
        let bar = Rect::new(0, 19, 140, 1);
        assert!(place(&mut b, bar, 22, &g, Some((12.4, true)), 0).is_some());
        let foot = row(&b, 19);
        assert!(
            foot.trim_start().starts_with("enter focus  a attach"),
            "{foot}"
        );
        assert!(foot.trim_end().ends_with("$12.40* today  ✸"), "{foot}");
        // narrow: shorter, then none
        let (mut b, _) = frame(60, 20);
        assert!(place(
            &mut b,
            Rect::new(0, 19, 60, 1),
            30,
            &g,
            Some((3.0, false)),
            0
        )
        .is_some());
        assert!(!row(&b, 19).contains("working"), "{}", row(&b, 19));
        let (mut b, _) = frame(60, 20);
        assert!(place(&mut b, Rect::new(0, 19, 60, 1), 58, &g, None, 0).is_none());
        assert_eq!(b, frame(60, 20).0);
    }

    #[test]
    fn the_heartbeat_flashes_once_a_turn() {
        assert_eq!(heartbeat(0).content, "✸");
        assert_eq!(heartbeat(PULSE_MS / 2 + 20).content, "✦");
        assert_eq!(heartbeat(400).content, "·");
        assert_eq!(heartbeat(PULSE_MS + 10).content, "✸");
        for s in [
            "⟡", "✻", "▲", "✸", "✦", "┤", "├", "┏", "┓", "┗", "┛", "╸", "╺", "╹", "╻",
        ] {
            assert_eq!(ui::width(s), 1, "{s}");
        }
    }
}
