//! Printed (non-TUI) output in the uv / Homebrew / Claude Code style:
//! `error:` / `warning:` / `hint:` prefixes, `✓` confirmations, `? q ›`
//! prompts, a star spinner on stderr and Claude Code's rounded welcome box.

use crate::term;
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn say(l: Line) {
    eprintln!("{}", ui::to_ansi(&l, term::color_err()));
}
pub fn out(l: Line) {
    println!("{}", ui::to_ansi(&l, term::color_out()));
}
pub fn prefixed(word: &str, col: Color, text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{word}:"), ui::fg(col).add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {text}")),
    ])
}
pub fn err(text: &str) {
    say(prefixed("error", c::RED(), text));
}
pub fn warn(text: &str) {
    say(prefixed("warning", c::YELLOW(), text));
}
pub fn hint(text: &str) {
    say(prefixed("hint", c::BLUE(), text));
}
pub fn ok(text: &str) {
    say(Line::from(vec![
        Span::styled("✓ ", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD)),
        Span::raw(text.to_string()),
    ]));
}
pub fn head(text: &str) {
    say(Line::from(vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled(text.to_string(), ui::bold()),
    ]));
}

pub fn fail(m: &str) -> ExitCode {
    err(m);
    ExitCode::from(2)
}

/// `? question (hint) › ` and the answer.
pub fn ask(q: &str, hint_text: &str) -> String {
    ask_line(q, hint_text).unwrap_or_default()
}

/// [`ask`], with None when stdin ended (Ctrl-D) instead of an answer.
fn ask_line(q: &str, hint_text: &str) -> Option<String> {
    let l = Line::from(vec![
        Span::styled("? ", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD)),
        Span::styled(q.to_string(), ui::bold()),
        Span::styled(
            if hint_text.is_empty() {
                String::new()
            } else {
                format!(" ({hint_text})")
            },
            ui::muted(),
        ),
        Span::styled(" › ", ui::faint()),
    ]);
    eprint!("{}", ui::to_ansi(&l, term::color_err()));
    let _ = io::stderr().flush();
    let mut s = String::new();
    match io::stdin().lock().read_line(&mut s) {
        Ok(0) | Err(_) => {
            eprintln!();
            None
        }
        Ok(_) => Some(s.trim().to_string()),
    }
}

/// A yes/no question; Enter means yes, Ctrl-D (end of input) means no.
pub fn yes(q: &str) -> bool {
    io::stdin().is_terminal()
        && ask_line(q, "Y/n").is_some_and(|a| {
            let a = a.to_lowercase();
            a.is_empty() || a.starts_with('y')
        })
}

/// Claude Code–style spinner on stderr while something slow runs.
pub struct Spinner {
    state: Arc<Mutex<Option<String>>>,
    on: bool,
}

impl Spinner {
    pub fn start(label: String) -> Spinner {
        let on = io::stderr().is_terminal();
        let state = Arc::new(Mutex::new(Some(label)));
        if on {
            let st = state.clone();
            std::thread::spawn(move || {
                let t0 = Instant::now();
                loop {
                    {
                        let g = st.lock().unwrap();
                        let Some(l) = g.as_ref() else { break };
                        let line = Line::from(vec![
                            Span::styled(format!("{} ", ui::spinner(t0)), ui::fg(c::ACCENT())),
                            Span::styled(format!("{l}…"), ui::muted()),
                            Span::styled(format!(" {}s", t0.elapsed().as_secs()), ui::faint()),
                        ]);
                        eprint!("\r\x1b[2K{}", ui::to_ansi(&line, true));
                        let _ = io::stderr().flush();
                    }
                    std::thread::sleep(Duration::from_millis(90));
                }
            });
        }
        Spinner { state, on }
    }
    pub fn set(&self, label: String) {
        *self.state.lock().unwrap() = Some(label);
    }
    /// Clear the spinner line and run `f` while holding the output lock.
    pub fn print(&self, f: impl FnOnce()) {
        let _g = self.state.lock().unwrap();
        if self.on {
            eprint!("\r\x1b[2K");
            let _ = io::stderr().flush();
        }
        f();
        let _ = io::stdout().flush();
    }
    pub fn stop(self) {
        let mut g = self.state.lock().unwrap();
        *g = None;
        if self.on {
            eprint!("\r\x1b[2K");
            let _ = io::stderr().flush();
        }
    }
}

/// Rounded box in the brand accent (Claude Code's welcome box).
pub fn boxed(w: usize, body: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let b = ui::fg(c::ACCENT());
    let inner = w - 4;
    let mut out = vec![Line::from(Span::styled(
        format!("╭{}╮", "─".repeat(w - 2)),
        b,
    ))];
    for l in body {
        let pad = inner.saturating_sub(ui::line_width(&l));
        let mut s = vec![Span::styled("│ ", b)];
        s.extend(l.spans);
        s.push(Span::raw(" ".repeat(pad)));
        s.push(Span::styled(" │", b));
        out.push(Line::from(s));
    }
    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(w - 2)),
        b,
    )));
    out
}

/// One column of a [`table`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Col {
    pub head: &'static str,
    /// shrinks, its cells cut with …, when the table is too wide
    pub flex: bool,
    /// numbers: right-aligned
    pub right: bool,
    /// left out when the table is too wide even with every flexible column
    /// shrunk (the last such first)
    pub optional: bool,
}

impl Col {
    pub const fn new(head: &'static str) -> Col {
        Col {
            head,
            flex: false,
            right: false,
            optional: false,
        }
    }
    pub const fn flex(self) -> Col {
        Col { flex: true, ..self }
    }
    pub const fn right(self) -> Col {
        Col {
            right: true,
            ..self
        }
    }
    pub const fn optional(self) -> Col {
        Col {
            optional: true,
            ..self
        }
    }
}

/// A bordered table: rounded corners, faint rules, bold heads, one styled
/// span a cell. As wide as its cells and at most `max` columns: flexible
/// columns shrink (never below their head or 8), then optional ones go
/// (pure — tested).
pub fn table(cols: &[Col], rows: &[Vec<Span<'static>>], max: usize) -> Vec<Line<'static>> {
    let natural = |i: usize| {
        rows.iter()
            .filter_map(|r| r.get(i))
            .map(|s| ui::width(&s.content))
            .max()
            .unwrap_or(0)
            .max(ui::width(cols[i].head))
    };
    let mut keep: Vec<usize> = (0..cols.len()).collect();
    let mut w: Vec<usize> = (0..cols.len()).map(natural).collect();
    let total = |keep: &[usize], w: &[usize]| {
        keep.iter().map(|&i| w[i]).sum::<usize>() + 3 * keep.len().saturating_sub(1) + 4
    };
    while total(&keep, &w) > max {
        // the widest flexible column that can still give
        let give = keep
            .iter()
            .copied()
            .filter(|&i| cols[i].flex && w[i] > ui::width(cols[i].head).max(8))
            .max_by_key(|&i| w[i]);
        if let Some(i) = give {
            w[i] -= 1;
            continue;
        }
        match keep.iter().rposition(|&i| cols[i].optional) {
            Some(k) => {
                keep.remove(k);
            }
            None => break,
        }
    }
    let rule = ui::faint();
    let line = |l: &str, m: &str, r: &str| {
        let mid: Vec<String> = keep.iter().map(|&i| "─".repeat(w[i] + 2)).collect();
        Line::from(Span::styled(format!("{l}{}{r}", mid.join(m)), rule))
    };
    let cell = |i: usize, s: &Span<'static>| {
        let t = ui::trunc(&s.content, w[i]);
        let gap = " ".repeat(w[i].saturating_sub(ui::width(&t)));
        let t = if cols[i].right {
            format!("{gap}{t}")
        } else {
            format!("{t}{gap}")
        };
        Span::styled(t, s.style)
    };
    let row = |cells: Vec<Span<'static>>| {
        let mut v = vec![Span::styled("│ ", rule)];
        for (k, c) in cells.into_iter().enumerate() {
            if k > 0 {
                v.push(Span::styled(" │ ", rule));
            }
            v.push(c);
        }
        v.push(Span::styled(" │", rule));
        Line::from(v)
    };
    let mut out = vec![line("╭", "┬", "╮")];
    out.push(row(keep
        .iter()
        .map(|&i| cell(i, &Span::styled(cols[i].head, ui::bold())))
        .collect()));
    out.push(line("├", "┼", "┤"));
    for r in rows {
        let blank = Span::raw("");
        out.push(row(keep
            .iter()
            .map(|&i| cell(i, r.get(i).unwrap_or(&blank)))
            .collect()));
    }
    out.push(line("╰", "┴", "╯"));
    out
}

/// Print JSON (pretty when a human is looking).
/// A plain line on stdout (TSV, a path, a URL): masked in showcase mode.
pub fn raw(s: &str) {
    println!("{}", crate::showcase::text(s));
}

pub fn json(v: &serde_json::Value) {
    // showcase mode: masked by field (a no-op in other builds)
    let masked;
    let v = if crate::showcase::on() {
        masked = crate::showcase::masked(v.clone());
        &masked
    } else {
        v
    };
    if io::stdout().is_terminal() {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    } else {
        println!("{v}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_bordered_aligned_and_gives_way_when_narrow() {
        let cols = [
            Col::new("name"),
            Col::new("n").right(),
            Col::new("about").flex(),
            Col::new("extra").optional(),
        ];
        let rows = vec![
            vec![
                Span::raw("alpha"),
                Span::raw("7"),
                Span::raw("a long description that has to give way"),
                Span::raw("x"),
            ],
            vec![Span::raw("b"), Span::raw("1234")],
        ];
        let text = |w: usize| -> Vec<String> {
            table(&cols, &rows, w)
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        let t = text(200);
        assert_eq!(t.len(), 6, "{t:#?}");
        assert!(t[0].starts_with("╭─") && t[0].contains('┬') && t[0].ends_with('╮'));
        assert_eq!(
            t[1],
            "│ name  │    n │ about                                   │ extra │"
        );
        assert!(t[2].starts_with('├') && t[5].starts_with('╰'));
        // numbers to the right; a short row's missing cells blank
        assert_eq!(
            t[4],
            "│ b     │ 1234 │                                         │       │"
        );
        // narrower: the flexible one is cut with …, then the optional one goes
        let t = text(50);
        assert!(t.iter().all(|l| ui::width(l) <= 50), "{t:#?}");
        assert!(t[3].contains('…') && t[1].contains("extra"), "{t:#?}");
        let t = text(30);
        assert!(
            !t[1].contains("extra") && t.iter().all(|l| ui::width(l) <= 30),
            "{t:#?}"
        );
    }
}
