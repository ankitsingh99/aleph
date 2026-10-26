//! The dashboard's frame, masked just before it's flushed: each row's text
//! is scanned (`scan::edits`) and the swaps written back. With whole names
//! (`showcase.names = full`) a swap is laid into its row (`reflow`): the
//! rest of its run moves into the run's spaces, cut with `…` when they run
//! out. With same-length ones it's cell for cell. Either way borders,
//! columns, colours and mouse targets stay exactly where they were; image
//! cells and wide characters are left alone.

use super::{dict, reflow, scan};
use ratatui::buffer::Buffer;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

/// An image placeholder (kitty's U+10EEEE, iTerm's markers): never text.
fn image(sym: &str) -> bool {
    sym.as_bytes().first().is_some_and(|b| *b >= 0xF3)
}

pub fn buffer(buf: &mut Buffer) {
    let d = dict::get();
    let full = d.names.tui_full();
    let area = buf.area;
    for y in area.top()..area.bottom() {
        // the row as characters, each with the cell it came from
        let mut chars: Vec<char> = Vec::with_capacity(area.width as usize);
        let mut at: Vec<Option<u16>> = Vec::with_capacity(area.width as usize);
        let mut any = false;
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            if cell.skip {
                chars.push('\u{0}');
                at.push(None);
                continue;
            }
            let sym = cell.symbol();
            let mut cs = sym.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) if !image(sym) && c.width() == Some(1) => {
                    any |= c.is_ascii_alphanumeric() || c == '/' || c == '~';
                    chars.push(c);
                    at.push(Some(x));
                }
                // a wide or combined grapheme, an image: a boundary
                _ => {
                    chars.push('\u{0}');
                    at.push(None);
                }
            }
        }
        if !any {
            continue;
        }
        let edits = scan::edits(&d, &chars, full);
        if edits.is_empty() {
            continue;
        }
        if full {
            let row: Vec<reflow::Slot<Style>> = chars
                .iter()
                .zip(&at)
                .map(|(c, x)| x.map(|x| (*c, buf[(x, y)].style())))
                .collect();
            let out = reflow::reflow(&row, &edits, false);
            for (k, slot) in out.into_iter().enumerate() {
                let (Some(x), Some((c, st))) = (at[k], slot) else {
                    continue;
                };
                if chars[k] == c && buf[(x, y)].style() == st {
                    continue;
                }
                let cell = &mut buf[(x, y)];
                let mut s = [0u8; 4];
                cell.reset();
                cell.set_symbol(c.encode_utf8(&mut s)).set_style(st);
            }
            continue;
        }
        for (a, b, r) in edits {
            // only where every character is one plain cell
            if b - a != r.len() || at[a..b].iter().any(Option::is_none) {
                continue;
            }
            for (k, c) in r.iter().enumerate() {
                if let Some(x) = at[a + k] {
                    let mut s = [0u8; 4];
                    buf[(x, y)].set_symbol(c.encode_utf8(&mut s));
                }
            }
        }
    }
}
