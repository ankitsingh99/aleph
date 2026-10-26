//! The session journal in the dashboard (`crate::journal`): a day's file,
//! styled and scrollable — the palette's "Journal: today" / "yesterday", the
//! Inbox's `J`. Today is put together again as it opens (and saved: the
//! same as `aleph journal --write`); another day shows its file, or is put
//! together when there's none.
//!
//! Keys: ↑↓ / PgUp / PgDn scroll · ←→ the day before / after · t today · r put
//! it together again now (and save it) · esc closes.
//!
//! What's shown goes through the lens (names by the frame's own pass, $ by
//! [`crate::journal::shown`]); the file stays real.

use super::{Dash, Mode};
use crate::journal;
use crate::ui::{self, c};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What the view knows of its day.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Text {
    /// nothing yet (being put together)
    #[default]
    Waiting,
    /// the file as saved
    Saved(String),
    /// put together just now, not saved (another day without a file)
    Fresh(String),
    Failed(String),
}

pub(crate) struct JournalView {
    day: i64,
    scroll: u16,
    text: Arc<Mutex<Text>>,
    busy: Arc<AtomicBool>,
}

/// Open the journal at `day` (a local day number).
pub(crate) fn open(d: &mut Dash, day: i64) {
    let v = JournalView {
        day,
        scroll: 0,
        text: Arc::new(Mutex::new(
            journal::read(&d.cfg, day).map_or(Text::Waiting, Text::Saved),
        )),
        busy: Arc::new(AtomicBool::new(false)),
    };
    let today = ui::local_day(crate::cache::now());
    let none = matches!(v.now(), Text::Waiting);
    if day == today || none {
        // today: as it stands now (and saved); a day without a file: put together
        v.refresh(d, day == today);
    }
    d.mode = Mode::Journal(v);
}

impl JournalView {
    /// A view with its text known (render tests).
    #[cfg(test)]
    pub(crate) fn with(day: i64, text: Text) -> JournalView {
        JournalView {
            day,
            scroll: 0,
            text: Arc::new(Mutex::new(text)),
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    fn now(&self) -> Text {
        self.text.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// Put the day together again, off the UI thread (`save`: write the file
    /// too). Never in tests.
    fn refresh(&self, d: &Dash, save: bool) {
        if cfg!(test) || self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (cfg, day, text, busy) = (
            d.cfg.clone(),
            self.day,
            self.text.clone(),
            self.busy.clone(),
        );
        let live: Vec<crate::herdr::Session> = d.fleet.sessions().into_iter().cloned().collect();
        std::thread::spawn(move || {
            let built = journal::build(&cfg, day, &live);
            let md = journal::markdown(&built);
            let t = if save {
                match journal::write(&cfg, &built) {
                    Ok(_) => Text::Saved(md),
                    Err(e) => Text::Failed(format!("couldn't save it: {e}")),
                }
            } else {
                Text::Fresh(md)
            };
            if let Ok(mut g) = text.lock() {
                *g = t;
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn working(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
}

pub(crate) fn draw(d: &Dash, f: &mut Frame, area: Rect, v: &JournalView) {
    crate::ui::clear(f, area);
    let today = ui::local_day(crate::cache::now());
    let when = match today - v.day {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        _ => crate::usage::day_name(v.day),
    };
    let state = match (v.now(), v.working()) {
        (_, true) => format!("{} putting it together…", ui::spinner(Instant::now())),
        (Text::Saved(_), _) => format!(
            "saved · {}",
            crate::doctor::tilde(&journal::path(&d.cfg, v.day))
        ),
        (Text::Fresh(_), _) => "not saved — r saves it".to_string(),
        (Text::Failed(e), _) => e,
        (Text::Waiting, _) => String::new(),
    };
    let head = Rect { height: 2, ..area };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    " journal ",
                    ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(journal::day_title(v.day), ui::bold()),
                Span::styled(format!("  {when}  "), ui::muted()),
                Span::styled(state, ui::faint()),
            ]),
            ui::hints(&[
                ("↑↓", "scroll"),
                ("←→", "day"),
                ("t", "today"),
                ("r", "again, saved"),
                ("esc", "close"),
            ]),
        ]),
        head,
    );
    let body = Rect {
        y: area.y + 2,
        height: area.height.saturating_sub(2),
        ..area
    };
    let block = ui::panel(
        Line::from(Span::styled(
            format!(" {}.md ", crate::usage::day_name(v.day)),
            ui::muted(),
        )),
        true,
    );
    let inner = block.inner(body);
    f.render_widget(block, body);
    let lines: Vec<Line> = match v.now() {
        Text::Saved(md) | Text::Fresh(md) => md
            .lines()
            .filter(|l| !l.starts_with("<!--"))
            .map(|l| journal::styled(&journal::shown(l)))
            .collect(),
        Text::Failed(e) => vec![Line::from(Span::styled(e, ui::fg(c::RED())))],
        Text::Waiting => vec![Line::from(Span::styled(
            format!("{} putting the day together…", ui::spinner(Instant::now())),
            ui::muted(),
        ))],
    };
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((v.scroll, 0)),
        inner,
    );
}

pub(crate) fn key(d: &mut Dash, mut v: JournalView, k: KeyEvent) {
    let today = ui::local_day(crate::cache::now());
    let page = 20;
    let lines = match v.now() {
        Text::Saved(m) | Text::Fresh(m) => m.lines().count() as u16,
        _ => 1,
    };
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => return,
        KeyCode::Down | KeyCode::Char('j') => v.scroll = (v.scroll + 1).min(lines),
        KeyCode::Up | KeyCode::Char('k') => v.scroll = v.scroll.saturating_sub(1),
        KeyCode::PageDown | KeyCode::Char(' ') => v.scroll = (v.scroll + page).min(lines),
        KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(page),
        KeyCode::Home | KeyCode::Char('g') => v.scroll = 0,
        KeyCode::End | KeyCode::Char('G') => v.scroll = lines.saturating_sub(page),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('[') => {
            open(d, v.day - 1);
            return;
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(']') if v.day < today => {
            open(d, v.day + 1);
            return;
        }
        KeyCode::Char('t') if v.day != today => {
            open(d, today);
            return;
        }
        KeyCode::Char('r') => {
            v.refresh(d, true);
            d.note("putting the day together again");
        }
        _ => {}
    }
    d.mode = Mode::Journal(v);
}
