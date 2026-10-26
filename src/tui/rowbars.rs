//! The tiny bars in each Fleet session row (`ui.row_bars`), right after its
//! status: tokens per minute over the last half hour as a sparkline (three
//! minutes a cell), and how full its context window is as a meter. Both come
//! from the session's own logs (`usage::dash::Live`, read every minute).
//!
//! A session without numbers gets blank room, so the columns never shift;
//! a list too narrow for them leaves the bars out for every row.

use super::Dash;
use crate::config::Config;
use crate::herdr::Session;
use crate::ui::{self, c};
use ratatui::prelude::*;

/// Sparkline cells, and the minutes each covers.
pub(crate) const SPARK: usize = 10;
const PER: u64 = 3;
/// The context meter's cells.
pub(crate) const METER: usize = 5;
/// The list has to be at least this wide for the bars.
const MIN_LIST: usize = 96;

/// What `ui.row_bars` asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Parts {
    pub tokens: bool,
    pub context: bool,
}

impl Parts {
    /// From `ui.row_bars` (pure — tested): `tokens,context` (the default,
    /// or `all` / `on`), one of them, `-tokens`, or `off`.
    pub(crate) fn parse(v: &str) -> Parts {
        let mut p = Parts::default();
        let mut any = false;
        for w in v.split(',').map(|w| w.trim().to_lowercase()) {
            match w.as_str() {
                "" => {}
                "all" | "on" => {
                    p = Parts {
                        tokens: true,
                        context: true,
                    };
                    any = true;
                }
                "off" | "none" => any = true,
                "tokens" => {
                    p.tokens = true;
                    any = true;
                }
                "context" => {
                    p.context = true;
                    any = true;
                }
                "-tokens" | "-context" => {
                    if !any {
                        p = Parts {
                            tokens: true,
                            context: true,
                        };
                    }
                    any = true;
                    if w == "-tokens" {
                        p.tokens = false;
                    } else {
                        p.context = false;
                    }
                }
                _ => {}
            }
        }
        if !any {
            return Parts {
                tokens: true,
                context: true,
            };
        }
        p
    }

    /// The room the bars take in a row (two spaces either side included).
    pub(crate) fn width(self) -> usize {
        let w = match (self.tokens, self.context) {
            (true, true) => SPARK + 1 + METER,
            (true, false) => SPARK,
            (false, true) => METER,
            (false, false) => return 0,
        };
        w + 4
    }
}

pub(crate) fn parts(cfg: &Config) -> Parts {
    Parts::parse(&cfg.get_or("ui.row_bars"))
}

/// The room the bars take in every row of a `list_w`-wide list: none when
/// they're off or it's too narrow.
pub(crate) fn room(cfg: &Config, list_w: usize) -> usize {
    if list_w < MIN_LIST {
        return 0;
    }
    parts(cfg).width()
}

/// Tokens per minute in each of `n` cells of `per` minutes, the newest last,
/// ending at `now` (pure — tested).
pub(crate) fn rate(recent: &[(u64, u64)], now: u64, n: usize, per: u64) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    let cell = per.max(1) * 60;
    let from = now.saturating_sub(n as u64 * cell);
    let mut v = vec![0f64; n];
    for &(t, tok) in recent {
        if t < from || t > now {
            continue;
        }
        let i = (((t - from) / cell) as usize).min(n - 1);
        v[i] += tok as f64;
    }
    v.iter().map(|x| x / per.max(1) as f64).collect()
}

/// A rate's height, 0..1, on one scale for every row (logarithmic: about
/// 300 tokens a minute is the floor, 2M the top).
pub(crate) fn height(per_min: f64) -> f64 {
    if per_min <= 0.0 {
        return 0.0;
    }
    ((per_min.log10() - 2.5) / (6.3 - 2.5)).clamp(0.05, 1.0)
}

/// A session's bars, `room` wide (what `room` gave), blank where there's
/// nothing to show.
pub(crate) fn slot(d: &Dash, s: &Session, room: usize) -> Vec<Span<'static>> {
    if room == 0 {
        return Vec::new();
    }
    let p = parts(&d.cfg);
    let live = super::usage::live_for(d, s);
    let mut v = vec![Span::raw("  ")];
    if p.tokens {
        match live.filter(|l| !l.recent.is_empty()) {
            Some(l) => v.extend(spark(&rate(&l.recent, crate::cache::now(), SPARK, PER))),
            None => v.push(Span::raw(" ".repeat(SPARK))),
        }
        if p.context {
            v.push(Span::raw(" "));
        }
    }
    if p.context {
        match live.and_then(|l| l.ctx_pct()) {
            Some(pct) => v.extend(meter(pct)),
            None => v.push(Span::raw(" ".repeat(METER))),
        }
    }
    v.push(Span::raw("  "));
    v
}

/// The sparkline: a quiet cell is a faint baseline, a busy one rises.
fn spark(rates: &[f64]) -> Vec<Span<'static>> {
    const B: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    rates
        .iter()
        .map(|r| {
            let h = height(*r);
            if h <= 0.0 {
                Span::styled("▁", ui::fg(c::LINE()))
            } else {
                let i = ((h * 7.0).round() as usize).min(7);
                Span::styled(B[i], ui::fg(c::WORKING()))
            }
        })
        .collect()
}

/// The context meter, coloured as it fills (the usage chip's colours).
fn meter(pct: u64) -> Vec<Span<'static>> {
    let col = if pct >= 85 {
        c::RED()
    } else if pct >= 65 {
        c::YELLOW()
    } else {
        c::MUTED()
    };
    ui::bar(pct as f64 / 100.0, METER, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_from_the_setting() {
        let all = Parts {
            tokens: true,
            context: true,
        };
        assert_eq!(Parts::parse("tokens,context"), all);
        assert_eq!(Parts::parse(""), all);
        assert_eq!(Parts::parse("all"), all);
        assert_eq!(Parts::parse("off"), Parts::default());
        assert_eq!(
            Parts::parse("context"),
            Parts {
                tokens: false,
                context: true
            }
        );
        assert_eq!(
            Parts::parse("-context"),
            Parts {
                tokens: true,
                context: false
            }
        );
        assert_eq!(all.width(), 2 + SPARK + 1 + METER + 2);
        assert_eq!(Parts::default().width(), 0);
    }

    #[test]
    fn rates_per_minute_by_cell() {
        // ten 3-minute cells ending at 10000: from 8200, the newest 9820..
        let r = rate(
            &[(10_000, 600), (9_999, 300), (9_700, 90), (100, 5)],
            10_000,
            10,
            3,
        );
        assert_eq!(r.len(), 10);
        // the newest cell: 900 tokens over 3 minutes; the one before: 90 / 3
        assert_eq!(r[9], 300.0);
        assert_eq!(r[8], 30.0);
        assert_eq!(r[0], 0.0);
        assert_eq!(height(0.0), 0.0);
        assert!(height(1_000.0) < height(100_000.0));
        assert_eq!(height(1e9), 1.0);
        // a trickle still shows
        assert!(height(10.0) > 0.0);
    }
}
