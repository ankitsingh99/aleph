//! The status bar's memory (`ctrl+n` in every view; `key.alerts`): the
//! alerts you missed — Inbox events (needs you, finished, unreachable, at
//! its limit) that came after you last looked at the Inbox — and the last 20
//! toasts, each with its time and level, marked when it showed while you
//! were in Focus or Tile (watching one agent, not the status bar).
//!
//! In the popup: ↑↓ move · enter goes to it (a live session's row in Fleet,
//! else the Inbox) · `r` marks the missed ones read · `i` the Inbox · esc.

use super::{Dash, Level, Mode, View};
use crate::events::Event;
use crate::ui::{self, c};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::collections::VecDeque;

/// How many toasts are kept.
pub(crate) const KEEP: usize = 20;
/// How many missed alerts the popup lists (the newest).
const MISSED: usize = 20;

/// A toast as it was shown.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Toasted {
    pub t: u64,
    pub level: Level,
    pub text: String,
    /// shown while you were in Focus or Tile
    pub away: bool,
}

#[derive(Default)]
pub(crate) struct State {
    /// the last `KEEP` toasts, oldest first
    pub log: VecDeque<Toasted>,
}

impl State {
    pub(crate) fn push(&mut self, level: Level, text: &str, away: bool) {
        // the same words again (a retry, a key held down): once, the newest time
        if let Some(last) = self.log.back_mut() {
            if last.text == text && last.level == level {
                last.t = crate::cache::now();
                last.away |= away;
                return;
            }
        }
        self.log.push_back(Toasted {
            t: crate::cache::now(),
            level,
            text: text.to_string(),
            away,
        });
        while self.log.len() > KEEP {
            self.log.pop_front();
        }
    }
}

/// The popup: the missed alerts as they were when it opened, and the pick.
pub(crate) struct Popup {
    missed: Vec<Event>,
    sel: usize,
}

/// What a row of the popup is.
enum Item<'a> {
    Missed(&'a Event),
    Toast(&'a Toasted),
}

fn items<'a>(d: &'a Dash, p: &'a Popup) -> Vec<Item<'a>> {
    p.missed
        .iter()
        .map(Item::Missed)
        .chain(d.alerts.log.iter().rev().map(Item::Toast))
        .collect()
}

/// Open it (ctrl+n).
pub(crate) fn open(d: &mut Dash) {
    let missed = super::inbox::missed(d, MISSED);
    d.mode = Mode::Alerts(Popup { missed, sel: 0 });
}

/// (glyph, colour, word) for a toast's level — the status bar's own.
pub(crate) fn level_look(l: Level) -> (&'static str, Color, &'static str) {
    match l {
        Level::Ok => ("✓", c::GREEN(), "ok"),
        Level::Info => ("●", c::BLUE(), "info"),
        Level::Warn => ("▲", c::YELLOW(), "warning"),
        Level::Err => ("✗", c::RED(), "error"),
    }
}

pub(crate) fn key(d: &mut Dash, mut p: Popup, k: KeyEvent) {
    let n = items(d, &p).len();
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => return,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
            p.sel = (p.sel + 1).min(n.saturating_sub(1))
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => p.sel = p.sel.saturating_sub(1),
        KeyCode::Home => p.sel = 0,
        KeyCode::End => p.sel = n.saturating_sub(1),
        KeyCode::Char('r') if !p.missed.is_empty() => {
            super::inbox::mark_read(d);
            let k = p.missed.len();
            p.missed.clear();
            p.sel = 0;
            d.note(format!(
                "{k} alert{} marked read",
                if k == 1 { "" } else { "s" }
            ));
        }
        KeyCode::Char('i') => {
            d.switch(View::Inbox);
            return;
        }
        KeyCode::Enter => {
            let target = match items(d, &p).get(p.sel) {
                Some(Item::Missed(e)) => Some((e.host.clone(), e.name.clone())),
                _ => None,
            };
            if let Some((host, name)) = target {
                go_to(d, &host, &name);
            }
            return;
        }
        _ => {}
    }
    d.mode = Mode::Alerts(p);
}

/// A missed alert's session: its row in Fleet while it runs, else the Inbox.
fn go_to(d: &mut Dash, host: &str, name: &str) {
    let key = d
        .fleet
        .host(host)
        .and_then(|h| {
            h.sessions
                .iter()
                .find(|s| s.name == name || s.display() == name)
        })
        .map(|s| s.key());
    match key {
        Some(k) => {
            d.switch(View::Fleet);
            d.fv.sel = Some(super::fleet::RowId::Sess(k));
        }
        None => d.switch(View::Inbox),
    }
}

pub(crate) fn draw(d: &Dash, f: &mut Frame, area: Rect, p: &Popup) {
    let now = crate::cache::now();
    let w = 88u16.min(area.width.saturating_sub(4)).max(20);
    let iw = w.saturating_sub(4) as usize;
    let all = items(d, p);
    let mut lines: Vec<Line> = Vec::new();
    // which line each item is on (for the bar and the scroll)
    let mut at: Vec<usize> = Vec::new();
    let head = |t: &str, more: String| {
        Line::from(vec![
            Span::styled(
                t.to_string(),
                ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {more}"), ui::faint()),
        ])
    };
    lines.push(head(
        "Missed",
        if p.missed.is_empty() {
            "nothing — you've seen every alert".to_string()
        } else {
            format!("{} since you last opened the Inbox", p.missed.len())
        },
    ));
    let when = |t: u64| ui::clock(t);
    let row =
        |time: String, g: &str, col: Color, word: &str, ww: usize, text: String, tail: String| {
            let mut v = vec![
                Span::styled(format!("  {time}  "), ui::faint()),
                Span::styled(format!("{g} "), ui::fg(col).add_modifier(Modifier::BOLD)),
                Span::styled(ui::pad(word, ww), ui::fg(col)),
                Span::raw("  "),
            ];
            let used = 2 + 5 + 2 + 2 + ww + 2;
            let tw = ui::width(&tail);
            let room = iw.saturating_sub(used + if tw > 0 { tw + 2 } else { 0 });
            let text = ui::trunc(&text, room);
            let fill = room.saturating_sub(ui::width(&text));
            v.push(Span::raw(text));
            if tw > 0 {
                v.push(Span::raw(" ".repeat(fill + 2)));
                v.push(Span::styled(tail, ui::faint()));
            }
            Line::from(v)
        };
    for it in &all {
        if let Item::Missed(e) = it {
            let (g, col, word) = e.kind.look();
            let what = if e.name.is_empty() {
                e.host.clone()
            } else {
                format!("{} · {}", e.name, e.host)
            };
            at.push(lines.len());
            lines.push(row(
                when(e.t),
                g,
                col,
                word,
                12,
                what,
                ui::ago(now.saturating_sub(e.t)),
            ));
        }
    }
    lines.push(Line::raw(""));
    lines.push(head(
        "Toasts",
        if d.alerts.log.is_empty() {
            "none yet".to_string()
        } else {
            format!("the last {}", d.alerts.log.len())
        },
    ));
    for it in &all {
        if let Item::Toast(t) = it {
            let (g, col, word) = level_look(t.level);
            at.push(lines.len());
            lines.push(row(
                when(t.t),
                g,
                col,
                word,
                8,
                t.text.clone(),
                if t.away {
                    "in Focus".to_string()
                } else {
                    String::new()
                },
            ));
        }
    }
    // the box: as tall as it needs, at most the screen
    let foot = 2;
    let pad = ui::padding();
    let h = (lines.len() as u16 + foot + 2 + pad.top + pad.bottom)
        .min(area.height.saturating_sub(2))
        .max(6);
    let r = ui::centered(area, w, h);
    crate::ui::clear(f, r);
    let block = ui::panel(
        Line::from(vec![
            Span::raw(" Alerts "),
            Span::styled("· ctrl+n ", ui::faint()),
        ]),
        true,
    );
    let inner = block.inner(r);
    f.render_widget(block, r);
    let body_h = inner.height.saturating_sub(foot) as usize;
    let sel_line = at.get(p.sel).copied().unwrap_or(0);
    let skip = (sel_line + 1).saturating_sub(body_h);
    let shown: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .skip(skip)
        .take(body_h)
        .map(|(i, l)| {
            if i == sel_line && !all.is_empty() {
                l.style(ui::selected())
            } else {
                l
            }
        })
        .collect();
    let body = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(1),
        height: body_h as u16,
        ..inner
    };
    f.render_widget(Paragraph::new(shown), body);
    if !all.is_empty() && sel_line >= skip && sel_line < skip + body_h {
        let y = body.y + (sel_line - skip) as u16;
        if let Some(cell) = f.buffer_mut().cell_mut((inner.x, y)) {
            cell.set_symbol("┃").set_fg(c::ACCENT());
        }
    }
    let mut keys = vec![("↑↓", "move"), ("enter", "go to it")];
    if !p.missed.is_empty() {
        keys.push(("r", "mark read"));
    }
    keys.extend([("i", "Inbox"), ("esc", "close")]);
    let hint = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    f.render_widget(Paragraph::new(ui::hints_fit(&keys, hint.width)), hint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_twenty_and_folds_repeats() {
        let mut s = State::default();
        for i in 0..25 {
            s.push(Level::Ok, &format!("toast {i}"), false);
        }
        assert_eq!(s.log.len(), KEEP);
        assert_eq!(s.log.front().unwrap().text, "toast 5");
        s.push(Level::Warn, "again", false);
        s.push(Level::Warn, "again", true);
        assert_eq!(s.log.len(), KEEP);
        assert!(s.log.back().unwrap().away);
        assert_eq!(s.log.iter().filter(|t| t.text == "again").count(), 1);
    }
}
