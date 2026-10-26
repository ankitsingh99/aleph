//! Saved workspaces in the dashboard (`crate::workspace`): `W` opens the
//! picker (enter or 1–9 goes there, `s` saves where you are now, `d` twice
//! deletes one); the palette lists them too.

use super::monitor_draw::Sub;
use super::{Dash, Level, Mode, View};
use crate::ui::{self, c};
use crate::workspace::{self, Workspace};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub(crate) struct Picker {
    list: Vec<Workspace>,
    sel: usize,
    /// `d` once: the row it'd delete (again to do it)
    armed: Option<usize>,
}

pub(crate) fn open(d: &mut Dash) {
    let list = workspace::all(&d.cfg);
    // the one you're in, if any, selected
    let here = here(d);
    let sel = list
        .iter()
        .position(|w| w.view == here.view && w.sub == here.sub && w.layout == here.layout)
        .unwrap_or(0);
    d.mode = Mode::Spaces(Picker {
        list,
        sel,
        armed: None,
    });
}

fn view_name(v: View) -> &'static str {
    match v {
        View::Monitor => "monitor",
        View::Inbox => "inbox",
        View::Settings => "settings",
        View::Fleet | View::Wall => "fleet",
    }
}

fn view_of(s: &str) -> View {
    View::named(s).unwrap_or(View::Fleet)
}

fn sub_of(s: &str) -> Option<Sub> {
    let s = s.to_lowercase();
    Sub::ALL.into_iter().find(|x| {
        x.name().to_lowercase() == s
            || matches!((x, s.as_str()), (Sub::Procs, "procs") | (Sub::Net, "net"))
    })
}

/// Where you are now, as a workspace (no name yet).
pub(crate) fn here(d: &Dash) -> Workspace {
    let view = view_name(d.view);
    let sub = if d.view == View::Monitor {
        d.mon.sub.name().to_lowercase()
    } else {
        String::new()
    };
    let machine = match d.view {
        View::Monitor => d.fleet.hosts.get(d.mon.sel).map(|h| h.alias.clone()),
        View::Fleet => d.fv.selected_host(d),
        _ => None,
    }
    .unwrap_or_default();
    Workspace {
        view: view.to_string(),
        sub,
        layout: d.cfg.get(&format!("layout.{view}")).unwrap_or_default(),
        filter: if d.view == View::Fleet {
            d.fv.filter.clone()
        } else {
            String::new()
        },
        machine,
        ..Default::default()
    }
}

/// Go there: the tab, Monitor's sub-view, the layout, the filter, the machine.
pub(crate) fn apply(d: &mut Dash, w: &Workspace) {
    let view = view_of(&w.view);
    d.switch(view);
    if view == View::Monitor {
        if let Some(s) = sub_of(&w.sub) {
            d.mon.sub = s;
            d.mon.filtering = false;
        }
    }
    let key = format!("layout.{}", view_name(view));
    if !w.layout.is_empty() && d.cfg.get(&key).as_deref() != Some(w.layout.as_str()) {
        let _ = d.cfg.set(&key, &w.layout);
    }
    if view == View::Fleet {
        d.fv.filter = w.filter.clone();
        d.fv.filtering = false;
        d.fv.invalidate_preview();
    }
    if !w.machine.is_empty() {
        if let Some(i) = d.fleet.hosts.iter().position(|h| h.alias == w.machine) {
            d.mon.sel = i;
            if view == View::Fleet {
                d.fv.sel = Some(super::fleet::RowId::Host(w.machine.clone()));
            }
        }
    }
    let t = if w.title.is_empty() {
        &w.name
    } else {
        &w.title
    };
    d.toast(Level::Ok, format!("workspace {t}"));
}

/// The input's answer: save where you are now under that title.
pub(crate) fn save_named(d: &mut Dash, title: &str) {
    let name = workspace::slug(title);
    if name.is_empty() {
        d.error("a workspace needs a name with a letter or digit in it".to_string());
        return;
    }
    let w = Workspace {
        name: name.clone(),
        title: title.trim().to_string(),
        ..here(d)
    };
    match workspace::save(&mut d.cfg, &w) {
        Ok(()) => d.toast(
            Level::Ok,
            format!("saved workspace {} · W goes back there", w.title),
        ),
        Err(e) => d.error(e),
    }
}

pub(crate) fn draw(f: &mut Frame, area: Rect, p: &Picker) {
    let w = area.width.saturating_sub(4).clamp(40, 76);
    let tw = p
        .list
        .iter()
        .map(|x| ui::width(&x.title))
        .max()
        .unwrap_or(10)
        .clamp(8, 24);
    let mut lines: Vec<Line> = Vec::new();
    for (i, x) in p.list.iter().enumerate() {
        let on = i == p.sel;
        let mut spans = vec![
            Span::styled(if on { "┃ " } else { "  " }, ui::fg(c::ACCENT())),
            Span::styled(
                if i < 9 {
                    format!("{} ", i + 1)
                } else {
                    "  ".into()
                },
                ui::faint(),
            ),
            Span::styled(
                format!("{}  ", ui::pad(&x.title, tw)),
                if on { ui::bold() } else { Style::new() },
            ),
            Span::styled(
                ui::trunc(&x.what(), (w as usize).saturating_sub(tw + 18)),
                ui::muted(),
            ),
        ];
        if x.starter {
            spans.push(Span::styled("  starter", ui::faint()));
        }
        if p.armed == Some(i) {
            spans.push(Span::styled("  d again deletes", ui::fg(c::RED())));
        }
        lines.push(Line::from(spans));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("  none yet", ui::faint())));
    }
    lines.push(Line::raw(""));
    lines.push(ui::hints(&[
        ("enter", "go"),
        ("1–9", "go"),
        ("s", "save here"),
        ("d", "delete"),
        ("esc", "close"),
    ]));
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let r = ui::centered(area, w, h);
    crate::ui::clear(f, r);
    let block = ui::panel("Workspaces", true).padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(r);
    f.render_widget(block, r);
    f.render_widget(Paragraph::new(lines), inner);
}

pub(crate) fn key(d: &mut Dash, mut p: Picker, k: KeyEvent) {
    let n = p.list.len();
    let armed = p.armed.take();
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('W') => return,
        KeyCode::Down | KeyCode::Char('j') if n > 0 => p.sel = (p.sel + 1) % n,
        KeyCode::Up | KeyCode::Char('k') if n > 0 => p.sel = (p.sel + n - 1) % n,
        KeyCode::Enter => {
            if let Some(w) = p.list.get(p.sel).cloned() {
                apply(d, &w);
            }
            return;
        }
        KeyCode::Char(c @ '1'..='9') => {
            if let Some(w) = p.list.get((c as u8 - b'1') as usize).cloned() {
                apply(d, &w);
                return;
            }
        }
        KeyCode::Char('s') => {
            let title = p
                .list
                .get(p.sel)
                .filter(|w| !w.starter)
                .map(|w| w.title.clone())
                .unwrap_or_default();
            d.mode = Mode::Input(super::forms::Input::new(
                "Save this as a workspace".into(),
                "its name: W and the palette go back here (the tab, its layout, Fleet's filter, the machine)",
                title,
                super::forms::For::SaveWorkspace,
            ));
            return;
        }
        KeyCode::Char('d') => {
            if let Some(w) = p.list.get(p.sel).cloned() {
                if w.starter {
                    d.note("a starter: there's nothing saved to delete");
                } else if armed == Some(p.sel) {
                    match workspace::remove(&mut d.cfg, &w.name) {
                        Ok(()) => {
                            d.toast(Level::Ok, format!("workspace {} deleted", w.title));
                            p.list = workspace::all(&d.cfg);
                            p.sel = p.sel.min(p.list.len().saturating_sub(1));
                        }
                        Err(e) => d.error(e),
                    }
                } else {
                    p.armed = Some(p.sel);
                }
            }
        }
        _ => {}
    }
    d.mode = Mode::Spaces(p);
}
