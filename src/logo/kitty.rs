//! Logos as images: kitty's graphics protocol with Unicode placeholders
//! (kitty, Ghostty).
//!
//! A logo's PNG goes to the terminal once per size (`a=T,U=1`: a virtual
//! placement `c`×`r` cells big), and the picture shows wherever cells hold
//! U+10EEEE with combining marks for their row and column and the image's id
//! as their foreground colour. To ratatui those are ordinary cells — diffed,
//! moved and overwritten like text — so nothing has to track where images
//! are, and a cell that's redrawn as something else simply stops showing it.
//!
//! [`take`] is what the host writes after a frame (the pictures that frame
//! asked for: cells whose picture hasn't arrived show nothing until it does)
//! and [`clear`] what it writes before handing the terminal over; which
//! terminals get this, and the host's side, are in [`super::images`].

use super::{Logo, Paint};
use ratatui::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// The character kitty draws image cells for.
const PLACEHOLDER: char = '\u{10EEEE}';

/// Row and column numbers 0–29 as combining marks (the start of kitty's
/// rowcolumn-diacritics.txt; iTerm2's marker cells use them too).
pub(super) const DIACRITICS: [char; 30] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}',
    '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}', '\u{350}', '\u{351}', '\u{352}', '\u{357}',
    '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}', '\u{36E}', '\u{36F}',
];

/// Our image ids start here (24 bits: they're a cell's foreground colour).
const FIRST_ID: u32 = 0xA1_0000;

#[derive(Default)]
struct State {
    /// (logo, size, colours) → image id
    ids: HashMap<String, u32>,
    /// escape sequences not written yet
    pending: String,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// The pictures asked for since the last call, as escape sequences to write
/// to the terminal (empty when there are none).
pub fn take() -> String {
    with(|s| std::mem::take(&mut s.pending))
}

/// Escape sequences that delete every picture sent (and forget them, so
/// they're sent again when asked for).
pub fn clear() -> String {
    with(|s| {
        s.pending.clear();
        let mut ids: Vec<u32> = s.ids.drain().map(|(_, id)| id).collect();
        ids.sort_unstable();
        ids.iter()
            .map(|id| format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\"))
            .collect()
    })
}

/// A logo as an image in `cols`×`rows` cells: placeholder cells in a box
/// twice as wide as tall, centred (at most 30 columns), the rest blank.
/// Asks for the picture ([`take`]) the first time this size is drawn.
pub fn lines(logo: &Logo, cols: usize, rows: usize, paint: &Paint) -> Vec<Line<'static>> {
    with(|s| lines_in(s, logo, cols, rows, paint))
}

fn lines_in(
    s: &mut State,
    logo: &Logo,
    cols: usize,
    rows: usize,
    paint: &Paint,
) -> Vec<Line<'static>> {
    let r = rows.min(cols / 2).min(DIACRITICS.len() / 2);
    if r == 0 {
        // no room for a picture: its glyph
        let g = super::glyph_with(logo.kind, logo.id, super::nerd(), paint);
        return super::centred(vec![g], cols, rows);
    }
    let c = 2 * r;
    let (x0, y0) = ((cols - c) / 2, (rows - r) / 2);
    let colours = colours(logo, paint);
    let key = format!("{}/{}/{c}x{r}/{colours:?}", logo.kind.as_str(), logo.id);
    let id = match s.ids.get(&key) {
        Some(&id) => id,
        None => {
            let id = FIRST_ID + s.ids.len() as u32;
            s.ids.insert(key, id);
            s.pending
                .push_str(&transmit(id, c, r, &recolour(logo.png, &colours)));
            id
        }
    };
    let fg = Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8);
    (0..rows)
        .map(|y| {
            if y < y0 || y >= y0 + r {
                return Line::raw(" ".repeat(cols));
            }
            let mut cells = String::with_capacity(c * 8);
            for col in &DIACRITICS[..c] {
                cells.push(PLACEHOLDER);
                cells.push(DIACRITICS[y - y0]);
                cells.push(*col);
            }
            Line::from(vec![
                Span::raw(" ".repeat(x0)),
                Span::styled(cells, Style::new().fg(fg)),
                Span::raw(" ".repeat(cols - c - x0)),
            ])
        })
        .collect()
}

/// The theme's colours for a logo's palette (black marks in its text colour
/// …), as its cells would be drawn: what [`recolour`] paints the PNG in.
pub(super) fn colours(logo: &Logo, paint: &Paint) -> Vec<(u8, u8, u8)> {
    logo.palette
        .iter()
        .map(|&col| match col {
            Color::Rgb(r, g, b) => super::fit((r, g, b), paint.base, paint.text),
            _ => paint.text,
        })
        .collect()
}

/// Escape sequences that send a PNG as image `id` and make it a virtual
/// placement `c`×`r` cells big (`q=2`: no replies on our stdin), in chunks
/// of 4096 base64 bytes.
fn transmit(id: u32, c: usize, r: usize, png: &[u8]) -> String {
    let data = base64(png);
    let chunks: Vec<&str> = data
        .as_bytes()
        .chunks(4096)
        .map(|b| std::str::from_utf8(b).unwrap_or(""))
        .collect();
    let mut out = String::with_capacity(data.len() + chunks.len() * 40);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            out.push_str(&format!(
                "\x1b_Ga=T,U=1,f=100,i={id},c={c},r={r},q=2,m={more};"
            ));
        } else {
            out.push_str(&format!("\x1b_Gm={more},q=2;"));
        }
        out.push_str(chunk);
        out.push_str("\x1b\\");
    }
    out
}

/// A logo's PNG drawn in `colours` (one per palette colour): the PNG is
/// indexed, `256 / colours` alpha steps per colour (tools/logogen), so this
/// rewrites its PLTE chunk (and the chunk's CRC) and nothing else.
pub fn recolour(png: &[u8], colours: &[(u8, u8, u8)]) -> Vec<u8> {
    let mut out = png.to_vec();
    let mut at = 8;
    while at + 12 <= out.len() {
        let len = u32::from_be_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]) as usize;
        let end = at + 8 + len;
        if end + 4 > out.len() {
            break;
        }
        if &out[at + 4..at + 8] == b"PLTE" && !colours.is_empty() {
            let entries = len / 3;
            let per = (entries / colours.len()).max(1);
            for e in 0..entries {
                let (r, g, b) = colours[(e / per).min(colours.len() - 1)];
                out[at + 8 + e * 3..at + 11 + e * 3].copy_from_slice(&[r, g, b]);
            }
            let crc = crc32(&out[at + 4..end]);
            out[end..end + 4].copy_from_slice(&crc.to_be_bytes());
            break;
        }
        at = end + 4;
    }
    out
}

/// PNG's CRC-32 (IEEE).
fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// Standard base64, padded.
pub(super) fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let n = (ch[0] as u32) << 16
            | (*ch.get(1).unwrap_or(&0) as u32) << 8
            | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= ch.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint() -> Paint {
        Paint {
            mono: false,
            truecolor: true,
            base: (0x11, 0x11, 0x11),
            text: (0xee, 0xee, 0xee),
        }
    }

    #[test]
    fn checksums_and_base64() {
        // every PNG ends with this chunk
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    /// Every chunk of a PNG and whether its CRC holds.
    fn chunks(png: &[u8]) -> Vec<(String, bool)> {
        let mut v = Vec::new();
        let mut at = 8;
        while at + 12 <= png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let end = at + 8 + len;
            let crc = u32::from_be_bytes(png[end..end + 4].try_into().unwrap());
            v.push((
                String::from_utf8_lossy(&png[at + 4..at + 8]).to_string(),
                crc32(&png[at + 4..end]) == crc,
            ));
            at = end + 4;
        }
        v
    }

    #[test]
    fn recolouring_rewrites_the_palette_alone() {
        let apple = crate::logo::find(crate::logo::Kind::Os, "apple").unwrap();
        let png = recolour(apple.png, &[(0xee, 0xee, 0xee)]);
        assert_eq!(png.len(), apple.png.len());
        let names: Vec<String> = chunks(&png).into_iter().map(|c| c.0).collect();
        assert!(
            names.starts_with(&["IHDR".to_string(), "PLTE".to_string()]),
            "{names:?}"
        );
        assert!(chunks(&png).iter().all(|c| c.1), "a CRC doesn't hold");
        // the palette is all the new colour; the pixels are untouched
        let plte = 8 + 25 + 8;
        assert!(png[plte..plte + 256 * 3]
            .chunks(3)
            .all(|e| e == [0xee, 0xee, 0xee]));
        let idat = |p: &[u8]| p.windows(4).position(|w| w == b"IDAT").unwrap();
        assert_eq!(png[idat(&png)..], apple.png[idat(apple.png)..]);
    }

    #[test]
    fn placeholders_are_cells_with_the_id_as_their_colour() {
        let mut s = State::default();
        let logo = crate::logo::find(crate::logo::Kind::Agent, "claude").unwrap();
        let l = lines_in(&mut s, logo, 10, 4, &paint());
        assert_eq!(l.len(), 4);
        for line in &l {
            assert_eq!(line.width(), 10, "{line:?}");
        }
        // 8×4 cells, centred: one blank column each side
        let cells = &l[0].spans[1];
        assert_eq!(cells.style.fg, Some(Color::Rgb(0xa1, 0, 0)));
        let first: Vec<char> = cells.content.chars().take(3).collect();
        assert_eq!(first, vec![PLACEHOLDER, DIACRITICS[0], DIACRITICS[0]]);
        let row2: Vec<char> = l[2].spans[1].content.chars().skip(3 * 7).collect();
        assert_eq!(row2, vec![PLACEHOLDER, DIACRITICS[2], DIACRITICS[7]]);
        // the picture is asked for once
        assert!(s
            .pending
            .starts_with("\x1b_Ga=T,U=1,f=100,i=10551296,c=8,r=4,q=2,m="));
        assert!(s.pending.ends_with("\x1b\\"));
        let sent = s.pending.len();
        lines_in(&mut s, logo, 10, 4, &paint());
        assert_eq!(s.pending.len(), sent);
        // another size is another image
        lines_in(&mut s, logo, 6, 3, &paint());
        assert!(s.pending.len() > sent && s.ids.len() == 2);
    }

    #[test]
    fn a_placeholder_cell_is_one_column_to_ratatui() {
        use ratatui::backend::TestBackend;
        let mut s = State::default();
        let logo = crate::logo::find(crate::logo::Kind::Os, "ubuntu").unwrap();
        let l = lines_in(&mut s, logo, 4, 2, &paint());
        let mut t = ratatui::Terminal::new(TestBackend::new(4, 2)).unwrap();
        t.draw(|f| f.render_widget(ratatui::widgets::Paragraph::new(l.clone()), f.area()))
            .unwrap();
        let buf = t.backend().buffer();
        for x in 0..4u16 {
            let sym = buf[(x, 0)].symbol();
            assert!(sym.starts_with(PLACEHOLDER), "{x}: {sym:?}");
            assert_eq!(sym.chars().nth(2), Some(DIACRITICS[x as usize]));
        }
        // too small for a picture: its glyph
        let g = lines_in(&mut s, logo, 1, 1, &paint());
        assert_eq!(g.len(), 1);
        assert!(!g[0].to_string().contains(PLACEHOLDER));
    }

    #[test]
    fn clearing_deletes_what_was_sent() {
        // (the global state: other tests use their own)
        let logo = crate::logo::find(crate::logo::Kind::Gpu, "nvidia").unwrap();
        lines(logo, 8, 4, &paint());
        assert!(take().contains("a=T,U=1"));
        assert!(take().is_empty());
        let c = clear();
        assert!(c.contains("a=d,d=I,i="), "{c}");
        // forgotten: drawn again, sent again
        lines(logo, 8, 4, &paint());
        assert!(take().contains("a=T,U=1"));
        clear();
    }
}
