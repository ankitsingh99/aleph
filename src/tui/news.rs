//! The What's new card (`crate::whatsnew`): once after an update, what
//! changed since you last opened Aleph. Notes are in plain words, and each
//! deploy's commits sit under enter. Closing it marks it seen.

use super::{Dash, Mode};
use crate::ui::{self, c};
use crate::whatsnew::Entry;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::cell::Cell;

pub(crate) struct Card {
    entries: Vec<Entry>,
    /// the commits too
    more: bool,
    top: usize,
    /// how far it can scroll, as last drawn
    max_top: Cell<usize>,
}

impl Card {
    fn new(entries: Vec<Entry>) -> Card {
        Card {
            entries,
            more: false,
            top: 0,
            max_top: Cell::new(0),
        }
    }
}

/// After the splash: the card, when there's something you haven't seen.
pub(crate) fn open_if_new(d: &mut Dash) {
    if !matches!(d.mode, Mode::Normal) {
        return;
    }
    let e = crate::whatsnew::to_show(&d.cfg);
    if !e.is_empty() {
        d.mode = Mode::News(Card::new(e));
    }
}

/// The palette's "What's new": the latest few, seen or not.
pub(crate) fn open(d: &mut Dash) {
    let e: Vec<Entry> = crate::whatsnew::all().iter().take(3).cloned().collect();
    if e.is_empty() {
        d.note("nothing new written down in this build");
        return;
    }
    d.mode = Mode::News(Card::new(e));
}

fn lines(card: &Card, w: usize) -> Vec<Line<'static>> {
    let mut v = Vec::new();
    for (i, e) in card.entries.iter().enumerate() {
        if i > 0 {
            v.push(Line::raw(""));
        }
        v.push(Line::from(vec![
            Span::styled("● ", ui::fg(c::ACCENT())),
            Span::styled(e.title.clone(), ui::bold()),
            Span::styled(format!("  {}", e.date), ui::faint()),
        ]));
        for n in &e.notes {
            for (j, l) in ui::wrap(n, w.saturating_sub(4).max(10))
                .into_iter()
                .enumerate()
            {
                v.push(Line::from(vec![
                    Span::styled(if j == 0 { "  • " } else { "    " }, ui::fg(c::ACCENT())),
                    Span::raw(l),
                ]));
            }
        }
        if card.more && !e.commits.is_empty() {
            v.push(Line::from(Span::styled("    commits", ui::muted())));
            for cm in &e.commits {
                v.push(Line::from(Span::styled(
                    format!("    · {}", ui::trunc(cm, w.saturating_sub(6))),
                    ui::faint(),
                )));
            }
        }
    }
    v
}

pub(crate) fn draw(f: &mut Frame, area: Rect, card: &Card) {
    let w = area.width.saturating_sub(4).clamp(30, 92);
    let body = lines(card, w.saturating_sub(4) as usize);
    // the body, a blank and the hints inside the border
    let h = (body.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(6);
    let r = ui::centered(area, w, h);
    crate::ui::clear(f, r);
    let title = Line::from(vec![
        Span::styled(" ℵ ", ui::fg(c::ACCENT())),
        Span::styled("What's new ", ui::bold()),
    ]);
    let block = ui::panel(title, true).padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let rows = inner.height.saturating_sub(2) as usize;
    let max_top = body.len().saturating_sub(rows);
    card.max_top.set(max_top);
    let top = card.top.min(max_top);
    let shown: Vec<Line> = body.into_iter().skip(top).take(rows).collect();
    f.render_widget(
        Paragraph::new(shown),
        Rect {
            height: rows as u16,
            ..inner
        },
    );
    let mut hints = vec![("enter", if card.more { "fewer" } else { "commits" })];
    if max_top > 0 {
        hints.push(("↑↓", "scroll"));
    }
    hints.push(("esc", "close"));
    f.render_widget(
        Paragraph::new(ui::hints(&hints)),
        Rect {
            y: inner.bottom().saturating_sub(1),
            height: 1,
            ..inner
        },
    );
}

pub(crate) fn key(d: &mut Dash, mut card: Card, k: KeyEvent) {
    let max = card.max_top.get();
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            crate::whatsnew::mark_seen();
            return;
        }
        KeyCode::Enter | KeyCode::Char('m') => card.more = !card.more,
        KeyCode::Down | KeyCode::Char('j') => card.top = (card.top + 1).min(max),
        KeyCode::Up | KeyCode::Char('k') => card.top = card.top.min(max).saturating_sub(1),
        KeyCode::PageDown | KeyCode::Char(' ') => card.top = (card.top + 10).min(max),
        KeyCode::PageUp => card.top = card.top.min(max).saturating_sub(10),
        KeyCode::Home | KeyCode::Char('g') => card.top = 0,
        KeyCode::End | KeyCode::Char('G') => card.top = max,
        _ => {}
    }
    d.mode = Mode::News(card);
}
