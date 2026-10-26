//! ANSI SGR text → ratatui lines, for coloured previews (`herdr pane read
//! --ansi`). Handles 16/256/truecolor, bold/dim/italic/underline/reverse;
//! every other escape (cursor moves, OSC titles, …) is dropped.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

fn idx(n: u16) -> Color {
    Color::Indexed(n.min(255) as u8)
}

fn apply(st: &mut Style, params: &[u16]) {
    let mut i = 0;
    if params.is_empty() {
        *st = Style::new();
        return;
    }
    while i < params.len() {
        let p = params[i];
        match p {
            0 => *st = Style::new(),
            1 => *st = st.add_modifier(Modifier::BOLD),
            2 => *st = st.add_modifier(Modifier::DIM),
            3 => *st = st.add_modifier(Modifier::ITALIC),
            4 => *st = st.add_modifier(Modifier::UNDERLINED),
            7 => *st = st.add_modifier(Modifier::REVERSED),
            9 => *st = st.add_modifier(Modifier::CROSSED_OUT),
            22 => *st = st.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => *st = st.remove_modifier(Modifier::ITALIC),
            24 => *st = st.remove_modifier(Modifier::UNDERLINED),
            27 => *st = st.remove_modifier(Modifier::REVERSED),
            29 => *st = st.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => *st = st.fg(idx(p - 30)),
            39 => *st = st.fg(Color::Reset),
            40..=47 => *st = st.bg(idx(p - 40)),
            49 => *st = st.bg(Color::Reset),
            90..=97 => *st = st.fg(idx(p - 90 + 8)),
            100..=107 => *st = st.bg(idx(p - 100 + 8)),
            38 | 48 => {
                let (col, used) = match params.get(i + 1) {
                    Some(5) => (params.get(i + 2).map(|n| idx(*n)), 2),
                    Some(2) => (
                        match (params.get(i + 2), params.get(i + 3), params.get(i + 4)) {
                            (Some(r), Some(g), Some(b)) => {
                                Some(Color::Rgb(*r as u8, *g as u8, *b as u8))
                            }
                            _ => None,
                        },
                        4,
                    ),
                    _ => (None, 0),
                };
                if let Some(c) = col {
                    *st = if p == 38 { st.fg(c) } else { st.bg(c) };
                }
                i += used;
            }
            _ => {}
        }
        i += 1;
    }
}

/// Parse text with ANSI escapes into styled lines (tabs expanded).
pub fn lines(s: &str) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur = String::new();
    let mut st = Style::new();
    let mut it = s.chars().peekable();
    let flush = |cur: &mut String, spans: &mut Vec<Span<'static>>, st: Style| {
        if !cur.is_empty() {
            // on a theme's own ground, its colours made to read there
            let st = match st.fg {
                Some(c) => st.fg(crate::ui::ink(c)),
                None => st,
            };
            spans.push(Span::styled(std::mem::take(cur), st));
        }
    };
    while let Some(ch) = it.next() {
        match ch {
            '\x1b' => match it.peek() {
                Some('[') => {
                    it.next();
                    let mut buf = String::new();
                    let mut fin = ' ';
                    for c in it.by_ref() {
                        if ('@'..='~').contains(&c) {
                            fin = c;
                            break;
                        }
                        buf.push(c);
                    }
                    if fin == 'm' {
                        flush(&mut cur, &mut spans, st);
                        let params: Vec<u16> = buf
                            .split([';', ':'])
                            .map(|x| x.parse().unwrap_or(0))
                            .collect();
                        apply(&mut st, if buf.is_empty() { &[] } else { &params });
                    }
                }
                Some(']') => {
                    // OSC … BEL or ST
                    it.next();
                    while let Some(c) = it.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && it.peek() == Some(&'\\') {
                            it.next();
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            },
            '\n' => {
                flush(&mut cur, &mut spans, st);
                out.push(Line::from(std::mem::take(&mut spans)));
            }
            '\r' => {}
            '\t' => cur.push_str("    "),
            c if (c as u32) < 0x20 => {}
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut spans, st);
    if !spans.is_empty() {
        out.push(Line::from(spans));
    }
    out
}

/// Plain text of a line.
pub fn plain(l: &Line) -> String {
    l.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_and_resets() {
        let v = lines("\x1b[1;38;2;217;119;87m✻ hi\x1b[0m there\n\x1b[31mred\x1b]0;title\x07!\n");
        assert_eq!(v.len(), 2);
        assert_eq!(plain(&v[0]), "✻ hi there");
        assert_eq!(v[0].spans[0].style.fg, Some(Color::Rgb(217, 119, 87)));
        assert!(v[0].spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(v[0].spans[1].style, Style::new());
        assert_eq!(plain(&v[1]), "red!");
        assert_eq!(v[1].spans[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(
            lines("\x1b[38;5;208mx")[0].spans[0].style.fg,
            Some(Color::Indexed(208))
        );
    }
}
