//! The `:` command line, for whatever's quicker typed than found:
//! `:focus b31:temper` · `:tile` · `:split gpu2:weave` · `:new b31` ·
//! `:preset ml` · `:go inbox` · `:set ui_scale large` · `:note #lut check` ·
//! `:send ignite run the tests` · `:approve temper` · `:doctor` · `:q`.
//! Tab completes the word under the cursor (commands, sessions, machines,
//! presets, settings, tabs); ↑ recalls the last commands.

use super::forms::{Field, Res};
use super::{Dash, Handoff, Level, Mode, View};
use crate::herdr::{self, Herdr, Session};
use crate::ui::{self, c};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::sync::Mutex;

/// (command, arguments, what it does)
const COMMANDS: &[(&str, &str, &str)] = &[
    ("focus", "<session>", "that session fullscreen"),
    (
        "tile",
        "[sessions…]",
        "these (or every) session side by side",
    ),
    ("split", "<session>", "add a session beside the focused one"),
    ("new", "[machine]", "Quick start on a machine"),
    ("preset", "<name>", "start a preset"),
    ("go", "<fleet | monitor | inbox | settings>", "another tab"),
    (
        "set",
        "<setting> <value>",
        "change a setting (no value: back to default)",
    ),
    ("note", "<text #tags>", "a note on the selected session"),
    (
        "clone",
        "<machine> [--dir D] [--move]",
        "the selected session's agent on another machine (--move: then close this one)",
    ),
    ("send", "<session> <text…>", "prompt a session"),
    (
        "approve",
        "<session>",
        "answer its question with its yes option",
    ),
    ("doctor", "", "Settings › Doctor"),
    ("quit", "", "leave Aleph"),
];

static RECENT: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(crate) struct CmdLine {
    pub f: Field,
    /// a recalled command (↑ / ↓), counted back from the newest
    back: usize,
    pub err: Option<String>,
}

impl CmdLine {
    pub fn new() -> CmdLine {
        CmdLine {
            f: Field::new(""),
            back: 0,
            err: None,
        }
    }

    pub fn key(&mut self, d: &Dash, k: KeyEvent) -> Res {
        match k.code {
            KeyCode::Esc => return Res::Cancel,
            KeyCode::Enter => return Res::Submit,
            KeyCode::Tab => {
                let s = complete(d, &self.f.text);
                if let Some(first) = s.first() {
                    let words: Vec<&str> = self.f.text.split(' ').collect();
                    let mut w: Vec<String> = words[..words.len().saturating_sub(1)]
                        .iter()
                        .map(|x| x.to_string())
                        .collect();
                    w.push(first.clone());
                    self.f = Field::new(&format!("{} ", w.join(" ")));
                }
            }
            KeyCode::Up | KeyCode::Down => {
                let r = RECENT.lock().unwrap();
                if !r.is_empty() {
                    self.back = if k.code == KeyCode::Up {
                        (self.back + 1).min(r.len())
                    } else {
                        self.back.saturating_sub(1)
                    };
                    let t = if self.back == 0 {
                        String::new()
                    } else {
                        r[r.len() - self.back].clone()
                    };
                    self.f = Field::new(&t);
                }
            }
            _ => {
                self.f.key(&k);
                self.err = None;
            }
        }
        Res::Keep
    }

    pub fn draw(&self, d: &Dash, f: &mut Frame, area: Rect) {
        let sugg = complete(d, &self.f.text);
        let help = help_for(&self.f.text);
        // the input, a help or error line, the suggestions, and the panel's
        // border and padding
        let rows = 1 + 1 + sugg.len().min(6) as u16 + 4;
        let w = area.width.saturating_sub(4).min(100);
        let h = rows.min(area.height);
        let r = Rect::new(area.x + 2, area.y + area.height.saturating_sub(h + 1), w, h);
        crate::ui::clear(f, r);
        let mut lines: Vec<Line> = Vec::new();
        let mut l = vec![Span::styled(
            " : ",
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        )];
        l.extend(self.f.spans(
            true,
            "focus b31:temper · tile · go inbox · set … · tab completes",
        ));
        lines.push(Line::from(l));
        if let Some(e) = &self.err {
            lines.push(Line::from(Span::styled(
                format!("   {e}"),
                ui::fg(c::RED()),
            )));
        } else if let Some(h) = help {
            lines.push(Line::from(Span::styled(format!("   {h}"), ui::faint())));
        }
        for s in sugg.iter().take(6) {
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled(s.clone(), ui::muted()),
            ]));
        }
        f.render_widget(Paragraph::new(lines).block(ui::panel("Command", true)), r);
    }
}

/// `focus <session> — that session fullscreen` for the command being typed.
fn help_for(text: &str) -> Option<String> {
    let w = text.split_whitespace().next()?;
    COMMANDS
        .iter()
        .find(|(c, _, _)| *c == w || (w.len() >= 2 && c.starts_with(w)))
        .map(|(c, a, h)| format!("{c} {a} — {h}"))
}

fn session_words(d: &Dash) -> Vec<String> {
    d.fleet
        .sessions()
        .iter()
        .map(|s| format!("{}:{}", s.host, s.name))
        .collect()
}

/// Completions for the last word of `text`.
pub(crate) fn complete(d: &Dash, text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split(' ').collect();
    let last = words.last().copied().unwrap_or("");
    let pool: Vec<String> = if words.len() <= 1 {
        COMMANDS.iter().map(|c| c.0.to_string()).collect()
    } else {
        match words[0] {
            "focus" | "f" | "split" | "approve" | "send" | "tile" => session_words(d),
            "new" => d.fleet.hosts.iter().map(|h| h.alias.clone()).collect(),
            "preset" => d.cfg.presets().into_iter().map(|p| p.name).collect(),
            "go" => View::ALL.iter().map(|v| v.name().to_lowercase()).collect(),
            "set" if words.len() == 2 => crate::config::KNOWN
                .iter()
                .map(|k| k.key.to_string())
                .collect(),
            "set" => crate::config::known(words[1])
                .map(|k| match k.kind {
                    crate::config::Kind::Choice(o) | crate::config::Kind::List(o) => {
                        o.iter().map(|x| x.to_string()).collect()
                    }
                    crate::config::Kind::Bool => vec!["true".into(), "false".into()],
                    _ => vec![],
                })
                .unwrap_or_default(),
            _ => vec![],
        }
    };
    let mut v: Vec<(i32, String)> = pool
        .into_iter()
        .filter_map(|p| crate::fuzzy::score(last, &p).map(|(s, _)| (s, p)))
        .collect();
    if !last.is_empty() {
        v.sort_by_key(|x| -x.0);
    }
    v.into_iter().map(|x| x.1).take(12).collect()
}

fn find(d: &Dash, q: &str) -> Option<Session> {
    let all: Vec<Session> = d.fleet.sessions().into_iter().cloned().collect();
    herdr::pick(&all, q).cloned()
}

/// Run what was typed. Err keeps the line open with the message.
pub(crate) fn run(d: &mut Dash, cl: CmdLine) -> Option<Handoff> {
    let text = cl.f.text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    {
        let mut r = RECENT.lock().unwrap();
        r.retain(|x| *x != text);
        r.push(text.clone());
        if r.len() > 30 {
            r.remove(0);
        }
    }
    let mut words = text.split_whitespace();
    let cmd = words.next().unwrap_or("");
    let rest: Vec<&str> = words.collect();
    let arg = rest.join(" ");
    let fail = |d: &mut Dash, cl: CmdLine, e: String| {
        let mut cl = cl;
        cl.err = Some(e);
        d.mode = Mode::Command(cl);
        None
    };
    // a command can be shortened while it's unambiguous (`fo`, `ti`)
    let full = COMMANDS
        .iter()
        .map(|c| c.0)
        .find(|c| *c == cmd)
        .or_else(|| {
            let v: Vec<&str> = COMMANDS
                .iter()
                .map(|c| c.0)
                .filter(|c| c.starts_with(cmd))
                .collect();
            (v.len() == 1).then(|| v[0])
        })
        .or(match cmd {
            "q" => Some("quit"),
            "f" => Some("focus"),
            _ => None,
        });
    let Some(full) = full else {
        return fail(d, cl, format!("no command '{cmd}' — tab lists them"));
    };
    let from = if d.view == View::Wall {
        d.wall.back.unwrap_or(View::Fleet)
    } else {
        d.view
    };
    match full {
        "focus" => match find(d, &arg) {
            Some(s) => super::wall::focus_on(d, &s.key(), from),
            None => return fail(d, cl, format!("no session '{arg}'")),
        },
        "tile" => {
            let keys: Vec<String> = if rest.is_empty() {
                d.fleet.sessions().iter().map(|s| s.key()).collect()
            } else {
                let mut v = Vec::new();
                for q in &rest {
                    match find(d, q) {
                        Some(s) => v.push(s.key()),
                        None => return fail(d, cl, format!("no session '{q}'")),
                    }
                }
                v
            };
            if keys.is_empty() {
                return fail(d, cl, "nothing to tile".into());
            }
            super::wall::tile(d, keys, from);
        }
        "split" => {
            let Some(s) = find(d, &arg) else {
                return fail(d, cl, format!("no session '{arg}'"));
            };
            if d.view == View::Wall {
                super::wall::split_with(d, s.key());
            } else {
                let mut keys: Vec<String> = super::fleet::selected_session(d)
                    .map(|x| vec![x.key()])
                    .unwrap_or_default();
                keys.push(s.key());
                super::wall::tile(d, keys, from);
            }
        }
        "new" => {
            let h = if arg.is_empty() {
                d.fv.selected_host(d)
                    .or_else(|| d.fleet.hosts.first().map(|h| h.alias.clone()))
            } else {
                Some(arg.clone())
            };
            match h.filter(|h| d.fleet.host(h).is_some()) {
                Some(h) => super::quick::open(d, &h),
                None => return fail(d, cl, format!("no machine '{arg}'")),
            }
        }
        "preset" => {
            if d.cfg.preset(&arg).is_none() {
                return fail(d, cl, format!("no preset '{arg}'"));
            }
            let mut p = super::palette::Palette::presets(d);
            p.choose_preset(&arg);
            return super::palette::run(d, p);
        }
        "go" => {
            let a = arg.to_lowercase();
            match View::ALL
                .iter()
                .find(|v| v.name().to_lowercase().starts_with(&a) && !a.is_empty())
            {
                Some(v) => d.switch(*v),
                None => return fail(d, cl, "go fleet · monitor · inbox · settings".into()),
            }
        }
        "set" => {
            let Some(key) = rest.first() else {
                return fail(d, cl, "set <setting> <value>".into());
            };
            let val = rest[1..].join(" ");
            if let Err(e) = crate::config::check(key, &val).map(|_| ()) {
                if !val.is_empty() {
                    return fail(d, cl, e);
                }
            }
            super::settings::save(d, key, &val);
        }
        "clone" => {
            let Some(s) = super::fleet::selected_session(d).or_else(|| d.wall.focused_session(d))
            else {
                return fail(d, cl, "select a session first".into());
            };
            let Some(to) = rest
                .iter()
                .find(|w| !w.starts_with('-'))
                .map(|w| w.to_string())
            else {
                return fail(d, cl, "clone <machine> [--dir D] [--move]".into());
            };
            let dir = rest
                .iter()
                .position(|w| *w == "--dir")
                .and_then(|i| rest.get(i + 1))
                .map(|w| w.to_string());
            let to = if dir.as_deref() == Some(to.as_str()) {
                String::new()
            } else {
                to
            };
            if to.is_empty() || !d.fleet.hosts.iter().any(|h| h.alias == to) {
                return fail(d, cl, format!("'{to}' isn't a machine in the fleet"));
            }
            let mv = rest.contains(&"--move");
            let cfg = d.cfg.clone();
            d.toast(
                Level::Info,
                format!("starting {} on {to}…", crate::agents::label(&s.agent)),
            );
            d.spawn(Some(to.clone()), move || {
                let name = crate::clone::clone(&cfg, &s, &to, dir.as_deref())?;
                if mv {
                    crate::clone::close_original(&cfg, &s).map_err(|e| {
                        format!("{name} runs on {to}, but {} stays: {e}", s.display())
                    })?;
                    return Ok(format!("moved {} to {to} as {name}", s.display()));
                }
                Ok(format!("{name} runs on {to} (a clone of {})", s.display()))
            });
        }
        "note" => {
            let Some(s) = super::fleet::selected_session(d).or_else(|| d.wall.focused_session(d))
            else {
                return fail(d, cl, "select a session first".into());
            };
            match crate::notes::set(&s.host, &s.name, &arg) {
                Ok(()) => d.note(format!("noted on {}", s.display())),
                Err(e) => d.error(format!("couldn't save the note: {e}")),
            }
        }
        "send" => {
            let Some((q, msg)) = arg.split_once(' ') else {
                return fail(d, cl, "send <session> <text…>".into());
            };
            let Some(s) = find(d, q) else {
                return fail(d, cl, format!("no session '{q}'"));
            };
            let (t, msg) = (d.target_of(&s.host), msg.to_string());
            d.spawn(Some(s.host.clone()), move || {
                Herdr::new(&t)
                    .prompt(&s.pane_id, &msg)
                    .map(|_| format!("sent to {}", s.display()))
            });
        }
        "approve" => {
            let Some(s) = find(d, &arg).filter(|s| s.blocked()) else {
                return fail(d, cl, format!("'{arg}' isn't asking anything"));
            };
            let t = d.target_of(&s.host);
            d.spawn(Some(s.host.clone()), move || {
                Herdr::new(&t)
                    .approve(&s.pane_id)
                    .map(|k| format!("approved {} ({})", s.display(), k.join(" ")))
            });
        }
        "doctor" => {
            d.switch(View::Settings);
            super::settings::go_section(d, "Doctor");
        }
        "quit" => return Some(Handoff::Quit),
        _ => d.toast(Level::Info, format!(":{full} isn't wired yet")),
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_names_the_command() {
        assert!(help_for("fo").unwrap().starts_with("focus <session>"));
        assert!(help_for("set ui_scale")
            .unwrap()
            .starts_with("set <setting>"));
        assert!(help_for("zz").is_none());
    }
}
