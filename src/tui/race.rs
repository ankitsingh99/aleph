//! The race view (`crate::race`): each racer's diff from where they all
//! started, side by side, refreshed every 10 s while it's open. ←→ picks a
//! column, ↑↓ / PgUp / PgDn scroll it, enter opens that session in Focus, r
//! refreshes now, esc closes.

use super::{Dash, Level, Mode};
use crate::race::Race;
use crate::ui::{self, c};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Diffs = Arc<Mutex<Vec<Option<Result<(String, String, String), String>>>>>;

pub(crate) struct RaceView {
    race: Race,
    col: usize,
    scroll: Vec<usize>,
    diffs: Diffs,
    /// when the diffs were last asked for (a fetch at a time)
    asked: Mutex<Option<Instant>>,
    busy: Arc<std::sync::atomic::AtomicBool>,
}

const EVERY: Duration = Duration::from_secs(10);
/// The most diff lines shown per racer.
const LINES: usize = 2000;

pub(crate) fn open(d: &mut Dash, race: Race) {
    let n = race.racers.len();
    let v = RaceView {
        race,
        col: 0,
        scroll: vec![0; n],
        diffs: Arc::new(Mutex::new(vec![None; n])),
        asked: Mutex::new(None),
        busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    v.fetch();
    d.mode = Mode::Race(v);
}

impl RaceView {
    /// A view with its diffs already read (render tests).
    #[cfg(test)]
    pub(crate) fn with(race: Race, diffs: Vec<(String, String, String)>) -> RaceView {
        let n = race.racers.len();
        RaceView {
            race,
            col: 0,
            scroll: vec![0; n],
            diffs: Arc::new(Mutex::new(diffs.into_iter().map(|d| Some(Ok(d))).collect())),
            asked: Mutex::new(Some(Instant::now())),
            busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Ask for every racer's diff, off the UI thread (never in tests).
    fn fetch(&self) {
        use std::sync::atomic::Ordering;
        if let Ok(mut a) = self.asked.lock() {
            *a = Some(Instant::now());
        }
        if cfg!(test) || self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (race, diffs, busy) = (self.race.clone(), self.diffs.clone(), self.busy.clone());
        std::thread::spawn(move || {
            for (i, r) in race.racers.iter().enumerate() {
                let d = crate::race::diff_of(&race.host, &race.base, r, LINES);
                if let Ok(mut g) = diffs.lock() {
                    if let Some(slot) = g.get_mut(i) {
                        *slot = Some(d);
                    }
                }
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn stale(&self) -> bool {
        self.asked
            .lock()
            .ok()
            .and_then(|a| *a)
            .is_none_or(|t| t.elapsed() > EVERY)
    }
}

fn lines_of(d: &Option<Result<(String, String, String), String>>) -> Vec<Line<'static>> {
    match d {
        None => vec![Line::from(Span::styled(
            format!("{} reading its diff…", ui::spinner(Instant::now())),
            ui::muted(),
        ))],
        Some(Err(e)) => vec![Line::from(Span::styled(
            format!("couldn't read it: {e}"),
            ui::fg(c::RED()),
        ))],
        Some(Ok((stat, untracked, diff))) => {
            let mut v: Vec<Line> = Vec::new();
            if stat.trim().is_empty() && untracked.trim().is_empty() {
                v.push(Line::from(Span::styled("no changes yet", ui::faint())));
            }
            for l in stat.lines() {
                v.push(Line::from(Span::styled(l.to_string(), ui::muted())));
            }
            for l in untracked.lines() {
                v.push(Line::from(Span::styled(
                    format!("new {}", l.trim_start_matches("?? ")),
                    ui::fg(c::GREEN()),
                )));
            }
            if !diff.is_empty() {
                v.push(Line::raw(""));
            }
            for l in diff.lines() {
                let st = if l.starts_with("+++") || l.starts_with("---") || l.starts_with("diff ") {
                    ui::bold()
                } else if l.starts_with('+') {
                    ui::fg(c::GREEN())
                } else if l.starts_with('-') {
                    ui::fg(c::RED())
                } else if l.starts_with("@@") {
                    ui::fg(c::TEAL())
                } else {
                    Style::new()
                };
                v.push(Line::from(Span::styled(l.replace('\t', "    "), st)));
            }
            v
        }
    }
}

pub(crate) fn draw(d: &Dash, f: &mut Frame, area: Rect, v: &RaceView) {
    if v.stale() {
        v.fetch();
    }
    crate::ui::clear(f, area);
    let head = Rect { height: 2, ..area };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" race ", ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD)),
                Span::styled(v.race.id.clone(), ui::bold()),
                Span::styled(
                    format!("  on {} · {}  ", v.race.host, v.race.dir),
                    ui::muted(),
                ),
                Span::raw(format!("“{}”", v.race.short())),
            ]),
            ui::hints(&[
                ("←→", "racer"),
                ("↑↓", "scroll"),
                ("enter", "focus"),
                ("r", "refresh"),
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
    let n = v.race.racers.len().max(1) as u16;
    let w = body.width / n;
    let diffs = v.diffs.lock().map(|g| g.clone()).unwrap_or_default();
    for (i, r) in v.race.racers.iter().enumerate() {
        let x = body.x + w * i as u16;
        let col = Rect {
            x,
            width: if i as u16 == n - 1 {
                body.right() - x
            } else {
                w
            },
            ..body
        };
        let live = d
            .fleet
            .sessions()
            .into_iter()
            .find(|s| s.host == v.race.host && s.pane_id == r.pane)
            .map(|s| s.status.clone())
            .unwrap_or_else(|| "gone".into());
        let title = Line::from(vec![
            Span::styled(format!(" {} ", crate::agents::label(&r.agent)), ui::bold()),
            Span::styled(format!("{} · {live} ", r.name), ui::muted()),
        ]);
        let block = ui::panel(title, i == v.col);
        let inner = block.inner(col);
        f.render_widget(block, col);
        let ls = lines_of(diffs.get(i).unwrap_or(&None));
        let top = v
            .scroll
            .get(i)
            .copied()
            .unwrap_or(0)
            .min(ls.len().saturating_sub(1));
        let shown: Vec<Line> = ls
            .into_iter()
            .skip(top)
            .take(inner.height as usize)
            .collect();
        f.render_widget(Paragraph::new(shown), inner);
    }
}

pub(crate) fn key(d: &mut Dash, mut v: RaceView, k: KeyEvent) {
    let n = v.race.racers.len().max(1);
    let page = 20;
    let s = &mut v.scroll[v.col.min(n - 1)];
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => return,
        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => v.col = (v.col + n - 1) % n,
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => v.col = (v.col + 1) % n,
        KeyCode::Down | KeyCode::Char('j') => *s += 1,
        KeyCode::Up | KeyCode::Char('k') => *s = s.saturating_sub(1),
        KeyCode::PageDown | KeyCode::Char(' ') => *s += page,
        KeyCode::PageUp => *s = s.saturating_sub(page),
        KeyCode::Home | KeyCode::Char('g') => *s = 0,
        KeyCode::Char('r') => {
            if let Ok(mut a) = v.asked.lock() {
                *a = None;
            }
            d.note("reading the diffs again");
        }
        KeyCode::Enter => {
            if let Some(r) = v.race.racers.get(v.col) {
                let key = d
                    .fleet
                    .sessions()
                    .into_iter()
                    .find(|s| s.host == v.race.host && s.pane_id == r.pane)
                    .map(|s| s.key());
                match key {
                    Some(key) => {
                        let from = d.view;
                        super::wall::focus_on(d, &key, from);
                        return;
                    }
                    None => d.toast(Level::Info, format!("{} isn't running any more", r.name)),
                }
            }
        }
        _ => {}
    }
    d.mode = Mode::Race(v);
}
