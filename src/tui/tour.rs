//! The coach-mark tour after setup: one card per tab (Fleet, Monitor,
//! Inbox, Settings), each under its tab, the tab itself lit. `enter` / → the
//! next card, ← the one before, `esc` ends it. Either way `tour = done` is
//! saved, so it shows once.

use super::{Dash, View};
use crate::ui::{self, c};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// (tab, title, lines)
const CARDS: &[(View, &str, &[&str])] = &[
    (
        View::Fleet,
        "Fleet — every session, every machine",
        &[
            "Sessions that need you come first. j/k move, enter opens one (Focus: its screen, its queue, approve).",
            "{new} starts a new session (enter only ever opens). {approve} approves, {send} sends a message, ? lists every key.",
        ],
    ),
    (
        View::Monitor,
        "Monitor — the machines themselves",
        &[
            "CPU, GPU, memory and the processes behind them on each machine, plus Usage (tokens, time, cost) and Services (local LLM servers).",
        ],
    ),
    (
        View::Inbox,
        "Inbox — what happened while you were away",
        &[
            "Questions, finished turns, limits, machines going down and back. enter goes to the session; r replies; / searches notes and events.",
        ],
    ),
    (
        View::Settings,
        "Settings — everything setup chose, and more",
        &[
            "Every option with its default and where it comes from, the doctor's checks with f to fix. aleph setup <step> runs one step of setup again.",
        ],
    ),
];

pub(crate) fn len() -> usize {
    CARDS.len()
}

/// Show card `i` (its tab underneath).
pub(crate) fn show(d: &mut Dash, i: usize) {
    if let Some((v, ..)) = CARDS.get(i) {
        d.switch(*v);
    }
}

fn done(d: &mut Dash) {
    let _ = d.cfg.set("tour", "done");
    d.switch(View::Fleet);
}

/// A key while card `i` shows: the next card, or None when the tour ends.
pub(crate) fn key(d: &mut Dash, i: usize, k: KeyEvent) -> Option<usize> {
    let n = match k.code {
        KeyCode::Enter
        | KeyCode::Right
        | KeyCode::Char(' ')
        | KeyCode::Char('l')
        | KeyCode::Tab => i + 1,
        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => i.saturating_sub(1),
        KeyCode::Esc | KeyCode::Char('q') => {
            done(d);
            return None;
        }
        _ => i,
    };
    if n >= CARDS.len() {
        done(d);
        return None;
    }
    show(d, n);
    Some(n)
}

/// The card, just under the tab bar.
pub(crate) fn draw(d: &Dash, f: &mut Frame, area: Rect, i: usize) {
    let Some((v, title, text)) = CARDS.get(i) else {
        return;
    };
    let w = 64u16.min(area.width.saturating_sub(4));
    if w < 24 || area.height < 8 {
        return;
    }
    let pad = ui::padding();
    let tw = (w - 2 - pad.left - pad.right).max(10) as usize;
    let mut lines: Vec<Line> = Vec::new();
    // the keys as this keymap has them (key.<action> can move them)
    let k = |a: super::Act| d.keys.key_for(a);
    for t in text.iter() {
        let t = t
            .replace("{new}", &k(super::Act::New))
            .replace("{approve}", &k(super::Act::Approve))
            .replace("{send}", &k(super::Act::Send));
        lines.extend(ui::wrap(&t, tw).into_iter().map(Line::from));
    }
    lines.push(Line::raw(""));
    let mut foot = vec![Span::styled(
        format!("{}/{}  ", i + 1, CARDS.len()),
        ui::faint(),
    )];
    foot.extend(
        ui::hints(&[
            ("enter", if i + 1 == CARDS.len() { "done" } else { "next" }),
            ("←", "back"),
            ("esc", "end the tour"),
        ])
        .spans,
    );
    lines.push(Line::from(foot));
    let h = (lines.len() as u16 + 2 + pad.top + pad.bottom).min(area.height.saturating_sub(4));
    // under its tab: tabs are laid out left to right, ~12 columns each
    let place = View::ALL.iter().position(|x| x == v).unwrap_or(0) as u16;
    let x = (area.x + 1 + place * 12).min(area.x + area.width.saturating_sub(w + 1));
    let r = Rect::new(x, area.y + 3, w, h);
    let arrow = Rect::new(
        (area.x + 3 + place * 12).min(area.x + area.width.saturating_sub(2)),
        area.y + 2,
        1,
        1,
    );
    f.render_widget(
        Paragraph::new(Span::styled("▲", ui::fg(c::ACCENT()))),
        arrow,
    );
    crate::ui::clear(f, r);
    f.render_widget(Paragraph::new(lines).block(ui::panel(*title, true)), r);
}
