//! A small inline picker for printed flows (`aleph setup`): no alternate
//! screen, drawn under the question on stderr, cleared afterwards and replaced
//! by a one-line answer.
//!
//! Keys: ↑↓ (j k) move · space toggles · a toggles all · enter confirms ·
//! esc / q skips · ctrl+c aborts. Without a terminal it falls back to a typed
//! answer (numbers or names, comma-separated).

use crate::out::{ask, say};
use crate::term;
use crate::ui::{self, c};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use ratatui::prelude::*;
use std::io::{self, IsTerminal, Write};

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub hint: String,
    pub on: bool,
    /// a logo before the label (an agent's glyph in its colour)
    pub icon: Option<Span<'static>>,
}

impl Item {
    pub fn new(label: impl Into<String>, hint: impl Into<String>, on: bool) -> Item {
        Item {
            label: label.into(),
            hint: hint.into(),
            on,
            icon: None,
        }
    }
    /// Builder: a logo before the label.
    pub fn icon(mut self, s: Span<'static>) -> Item {
        self.icon = Some(s);
        self
    }
}

const VIEW: usize = 10;

struct Raw;
impl Raw {
    fn on() -> io::Result<Raw> {
        terminal::enable_raw_mode()?;
        eprint!("\x1b[?25l");
        Ok(Raw)
    }
}
impl Drop for Raw {
    fn drop(&mut self) {
        eprint!("\x1b[?25h");
        let _ = io::stderr().flush();
        let _ = terminal::disable_raw_mode();
    }
}

fn tty() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// Pick any number of items. `None` = skipped (esc).
pub fn multi(title: &str, items: &[Item]) -> Option<Vec<usize>> {
    let mut v = items.to_vec();
    let r = if tty() {
        run(title, &mut v, true, 0)
    } else {
        typed(title, &mut v, true)
    };
    let chosen = r.map(|_| (0..v.len()).filter(|&i| v[i].on).collect::<Vec<_>>());
    summary(
        title,
        chosen.as_ref().map(|c| {
            c.iter()
                .map(|&i| v[i].label.clone())
                .collect::<Vec<_>>()
                .join(", ")
        }),
    );
    chosen
}

/// Pick one item (`default` starts selected). `None` = skipped.
pub fn single(title: &str, items: &[Item], default: usize) -> Option<usize> {
    let mut v = items.to_vec();
    let r = if tty() {
        run(title, &mut v, false, default)
    } else {
        typed(title, &mut v, false).map(|_| v.iter().position(|x| x.on).unwrap_or(default))
    };
    summary(title, r.map(|i| v[i].label.clone()));
    r
}

fn summary(title: &str, answer: Option<String>) {
    let mut s = vec![
        Span::styled("✓ ", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{title} "), ui::bold()),
    ];
    match answer {
        Some(a) if a.is_empty() => s.push(Span::styled("none", ui::muted())),
        Some(a) => s.push(Span::styled(a, ui::fg(c::ACCENT()))),
        None => {
            s[0] = Span::styled("○ ", ui::fg(c::YELLOW()));
            s.push(Span::styled("skipped", ui::muted()));
        }
    }
    say(Line::from(s));
}

/// Pure: the lines drawn for the current state (tested).
#[cfg(test)]
pub fn frame(
    title: &str,
    items: &[Item],
    multi: bool,
    cur: usize,
    top: usize,
    width: usize,
) -> Vec<Line<'static>> {
    frame_in(title, items, multi, cur, top, width, VIEW)
}

/// How many items fit: a taller terminal shows more (up to 16).
fn view() -> usize {
    terminal::size()
        .ok()
        .map(|s| (s.1 as usize).saturating_sub(10).clamp(VIEW, 16))
        .unwrap_or(VIEW)
}

/// [`frame`], `view` items at a time.
fn frame_in(
    title: &str,
    items: &[Item],
    multi: bool,
    cur: usize,
    top: usize,
    width: usize,
    view: usize,
) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(vec![
        Span::styled("? ", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD)),
        Span::styled(title.to_string(), ui::bold()),
    ])];
    let wl = items
        .iter()
        .map(|i| ui::width(&i.label))
        .max()
        .unwrap_or(4)
        .min(28);
    if top > 0 {
        out.push(Line::from(Span::styled(
            format!("    ↑ {} more", top),
            ui::faint(),
        )));
    }
    let end = (top + view).min(items.len());
    for (i, it) in items.iter().enumerate().take(end).skip(top) {
        let here = i == cur;
        let mut s = vec![if here {
            Span::styled("┃ ", ui::fg(c::ACCENT()))
        } else {
            Span::raw("  ")
        }];
        if multi {
            s.push(if it.on {
                Span::styled("◉ ", ui::fg(c::ACCENT()))
            } else {
                Span::styled("○ ", ui::faint())
            });
        }
        let icons = items.iter().any(|i| i.icon.is_some());
        if icons {
            s.push(it.icon.clone().unwrap_or_else(|| Span::raw(" ")));
            s.push(Span::raw(" "));
        }
        s.push(Span::styled(
            ui::pad(&it.label, wl),
            if here { ui::bold() } else { Style::new() },
        ));
        let used = 2 + if multi { 2 } else { 0 } + if icons { 2 } else { 0 } + wl + 2;
        if !it.hint.is_empty() && width > used + 4 {
            s.push(Span::styled(
                format!("  {}", ui::trunc(&it.hint, width - used - 1)),
                ui::faint(),
            ));
        }
        out.push(Line::from(s));
    }
    if end < items.len() {
        out.push(Line::from(Span::styled(
            format!("    ↓ {} more", items.len() - end),
            ui::faint(),
        )));
    }
    let keys: &[(&str, &str)] = if multi {
        &[
            ("↑↓", "move"),
            ("space", "toggle"),
            ("a", "all"),
            ("enter", "confirm"),
            ("esc", "skip"),
        ]
    } else {
        &[("↑↓", "move"), ("enter", "choose"), ("esc", "skip")]
    };
    out.push(ui::hints_fit(keys, width as u16));
    out
}

fn draw(lines: &[Line], prev: usize, color: bool) -> usize {
    let mut s = String::new();
    if prev > 0 {
        s.push('\r');
        if prev > 1 {
            s.push_str(&format!("\x1b[{}A", prev - 1));
        }
        s.push_str("\x1b[J");
    }
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            s.push_str("\r\n");
        }
        s.push_str(&ui::to_ansi(l, color));
    }
    eprint!("{s}");
    let _ = io::stderr().flush();
    lines.len()
}

fn clear(prev: usize) {
    if prev > 0 {
        let mut s = String::from("\r");
        if prev > 1 {
            s.push_str(&format!("\x1b[{}A", prev - 1));
        }
        s.push_str("\x1b[J");
        eprint!("{s}");
        let _ = io::stderr().flush();
    }
}

fn run(title: &str, items: &mut [Item], multi: bool, start: usize) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let raw = match Raw::on() {
        Ok(r) => r,
        Err(_) => return typed(title, &mut items.to_vec(), multi),
    };
    let color = term::color_err();
    let mut cur = start.min(items.len() - 1);
    let mut top = cur.saturating_sub(view() - 1);
    let mut drawn = 0;
    let result = loop {
        let width = terminal::size()
            .ok()
            .map(|s| s.0 as usize)
            .filter(|w| *w > 0)
            .unwrap_or(80)
            .max(20)
            - 1;
        let vw = view();
        if cur < top {
            top = cur;
        } else if cur >= top + vw {
            top = cur + 1 - vw;
        }
        drawn = draw(
            &frame_in(title, items, multi, cur, top, width, vw),
            drawn,
            color,
        );
        let Ok(ev) = event::read() else {
            break None;
        };
        let Event::Key(k) = ev else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                clear(drawn);
                drop(raw);
                eprintln!();
                std::process::exit(130);
            }
            KeyCode::Up | KeyCode::Char('k') => cur = cur.checked_sub(1).unwrap_or(items.len() - 1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => cur = (cur + 1) % items.len(),
            KeyCode::Home => cur = 0,
            KeyCode::End => cur = items.len() - 1,
            KeyCode::Char(' ') if multi => items[cur].on = !items[cur].on,
            KeyCode::Char('a') if multi => {
                let all = items.iter().all(|i| i.on);
                items.iter_mut().for_each(|i| i.on = !all);
            }
            KeyCode::Enter => {
                // single, or nothing marked yet: enter takes the highlighted item
                if !multi || !items.iter().any(|i| i.on) {
                    items
                        .iter_mut()
                        .enumerate()
                        .for_each(|(i, x)| x.on = i == cur);
                }
                break Some(cur);
            }
            KeyCode::Esc | KeyCode::Char('q') => break None,
            _ => {}
        }
    };
    clear(drawn);
    drop(raw);
    result
}

/// No terminal: list the items and read a typed answer.
fn typed(title: &str, items: &mut [Item], multi: bool) -> Option<usize> {
    for (i, it) in items.iter().enumerate() {
        say(Line::from(vec![
            Span::styled(format!("  {:>2}. ", i + 1), ui::faint()),
            Span::raw(if multi && it.on { "◉ " } else { "  " }),
            it.icon
                .clone()
                .map(|s| Span::styled(format!("{} ", s.content), s.style))
                .unwrap_or_else(|| Span::raw("")),
            Span::raw(it.label.clone()),
            Span::styled(
                if it.hint.is_empty() {
                    String::new()
                } else {
                    format!("  {}", it.hint)
                },
                ui::faint(),
            ),
        ]));
    }
    let a = ask(
        title,
        if multi {
            "numbers or names, comma-separated · enter keeps the marked ones · - skips"
        } else {
            "number or name · enter keeps the default · - skips"
        },
    );
    apply_typed(&a, items, multi)
}

/// Pure: apply a typed answer (`1,3`, `b31, gpu2`, ``, `-`). Tested.
pub fn apply_typed(a: &str, items: &mut [Item], multi: bool) -> Option<usize> {
    let a = a.trim();
    if a == "-" {
        return None;
    }
    if a.is_empty() {
        return Some(items.iter().position(|i| i.on).unwrap_or(0));
    }
    let mut picked = Vec::new();
    for w in a.split([',', ' ']).map(str::trim).filter(|w| !w.is_empty()) {
        let i = w
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .filter(|&i| i < items.len())
            .or_else(|| items.iter().position(|it| it.label == w));
        if let Some(i) = i {
            picked.push(i);
        }
    }
    if picked.is_empty() {
        return None;
    }
    if !multi {
        picked.truncate(1);
    }
    for (i, it) in items.iter_mut().enumerate() {
        it.on = picked.contains(&i);
    }
    picked.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![
            Item::new("b31", "selene@10.0.0.49", true),
            Item::new("gpu2", "tester@gpu2.lan", false),
            Item::new("dex42", "", false),
        ]
    }

    #[test]
    fn typed_answers() {
        let mut v = items();
        assert_eq!(apply_typed("", &mut v, true), Some(0));
        assert!(v[0].on && !v[1].on);
        assert_eq!(apply_typed("2, dex42", &mut v, true), Some(1));
        assert_eq!(
            v.iter().map(|i| i.on).collect::<Vec<_>>(),
            vec![false, true, true]
        );
        assert_eq!(apply_typed("-", &mut v, true), None);
        assert_eq!(apply_typed("nope", &mut v, true), None);
        let mut v = items();
        assert_eq!(apply_typed("gpu2 dex42", &mut v, false), Some(1));
        assert_eq!(v.iter().filter(|i| i.on).count(), 1);
    }

    #[test]
    fn frame_layout() {
        let v = items();
        let f = frame("Machines", &v, true, 1, 0, 60);
        let text: Vec<String> = f
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert!(text[0].contains("Machines"));
        assert!(text[1].starts_with("  ◉ b31"));
        assert!(text[2].starts_with("┃ ○ gpu2"));
        assert!(f.iter().all(|l| ui::line_width(l) <= 60));
        // long lists scroll
        let many: Vec<Item> = (0..25)
            .map(|i| Item::new(format!("h{i}"), "", false))
            .collect();
        let f = frame("Agents", &many, false, 20, 11, 60);
        let text: Vec<String> = f
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert!(text[1].contains("↑ 11 more"));
        assert!(text.iter().any(|t| t.contains("↓ 4 more")));
        assert!(text.iter().any(|t| t.starts_with("┃ h20")));
    }
}
