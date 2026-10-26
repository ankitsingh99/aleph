//! Light motion (`ui.motion`, on by default, in every build): a session row
//! that needs you breathes softly, and one that just finished gets a tick —
//! `✓ finished`, a band of light crossing it — for a moment.
//!
//! Only the text's own colour moves: never a background, never a glyph over
//! the text. `reduced` keeps the tick and stills the breathing; `off` stills
//! both; NO_COLOR has none. When showcase's celebrations play, they take the
//! finish instead (never both at once).

use super::Dash;
use crate::fleet::Transition;
use crate::herdr::Session;
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a finished session's tick lasts.
pub(crate) const TICK: Duration = Duration::from_millis(1600);
/// One breath of a row that needs you.
const BREATH_MS: u64 = 2400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    On,
    Reduced,
    Off,
}

impl Motion {
    /// From `ui.motion` (pure — tested; NO_COLOR is `of`'s).
    pub(crate) fn parse(v: &str) -> Motion {
        match v.trim().to_lowercase().as_str() {
            "off" | "false" | "none" | "no" => Motion::Off,
            "reduced" | "reduce" | "less" => Motion::Reduced,
            _ => Motion::On,
        }
    }
}

/// What moves here now.
pub(crate) fn of(d: &Dash) -> Motion {
    if c::mono() {
        return Motion::Off;
    }
    Motion::parse(&d.cfg.get_or("ui.motion"))
}

#[derive(Default)]
pub(crate) struct State {
    /// sessions that just finished: `host|pane` → when
    finished: HashMap<String, Instant>,
}

/// Finished, as the tick means it (pure — tested): working → done or idle,
/// or anything → done.
pub(crate) fn finished(from: &str, to: &str) -> bool {
    match (from, to) {
        ("", _) => false,
        ("working", "done" | "idle") => true,
        (f, "done") => f != "done",
        _ => false,
    }
}

/// The fleet's changes this tick (a machine's first poll isn't news).
pub(crate) fn on_transitions(d: &mut Dash, tr: &[Transition], fresh: &[String]) {
    let now = Instant::now();
    d.motion
        .finished
        .retain(|_, t| now.duration_since(*t) < TICK);
    if tr.is_empty() || of(d) == Motion::Off || crate::showcase::celebrating(&d.cfg) {
        return;
    }
    for t in tr {
        if fresh.contains(&t.session.host) {
            continue;
        }
        if finished(&t.from, &t.session.status) {
            d.motion.finished.insert(t.session.key(), now);
        }
    }
}

/// How far into its tick a session is (0..1), while it lasts.
pub(crate) fn ticking(d: &Dash, s: &Session) -> Option<f64> {
    let at = d.motion.finished.get(&s.key())?;
    let k = at.elapsed().as_secs_f64() / TICK.as_secs_f64();
    (k < 1.0 && of(d) != Motion::Off).then_some(k)
}

/// The status column of a session row, `w` wide: breathing while it needs
/// you, the tick while it has just finished, else as given.
pub(crate) fn status(
    d: &Dash,
    s: &Session,
    text: &str,
    base: Style,
    w: usize,
) -> Vec<Span<'static>> {
    let text = ui::pad(text, w);
    if let Some(k) = ticking(d, s) {
        return tick(&ui::pad("✓ finished", w), k);
    }
    if s.blocked() && s.limit.is_none() && of(d) == Motion::On {
        return vec![Span::styled(text, ui::pulse(c::NEEDS(), d.t0, BREATH_MS))];
    }
    vec![Span::styled(text, base)]
}

/// `text` in done-green, a band of light crossing it once over the tick
/// (`k`: 0 → 1), bold at first.
pub(crate) fn tick(text: &str, k: f64) -> Vec<Span<'static>> {
    let mut base = ui::fg(c::DONE());
    if k < 0.5 {
        base = base.add_modifier(Modifier::BOLD);
    }
    let n = text.chars().count() as f64;
    let pos = -2.0 + k * (n + 4.0);
    text.chars()
        .enumerate()
        .map(|(i, ch)| {
            let near = (1.0 - (i as f64 - pos).abs() / 2.5).clamp(0.0, 1.0);
            Span::styled(ch.to_string(), ui::shimmer_one(base, near))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_what_counts_as_finished() {
        assert_eq!(Motion::parse(""), Motion::On);
        assert_eq!(Motion::parse("on"), Motion::On);
        assert_eq!(Motion::parse("Reduced"), Motion::Reduced);
        assert_eq!(Motion::parse("off"), Motion::Off);
        assert!(finished("working", "done"));
        assert!(finished("working", "idle"));
        assert!(finished("blocked", "done"));
        assert!(!finished("", "done"));
        assert!(!finished("idle", "working"));
        assert!(!finished("blocked", "idle"));
        assert!(!finished("done", "done"));
    }

    #[test]
    fn the_tick_only_changes_colours() {
        let v = tick("✓ finished", 0.3);
        let text: String = v.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "✓ finished");
        assert!(v.iter().all(|s| s.style.bg.is_none()));
    }
}
