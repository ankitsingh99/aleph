//! Logos as images: iTerm2's inline images (OSC 1337; iTerm2, WezTerm).
//!
//! There are no placeholders: a picture is written at the cursor and covers
//! the cells it lands on until something's written over them. So [`lines`]
//! fills a logo's box with marker cells — a private-use character numbering
//! the picture, with kitty's row and column diacritics — and [`frame`] (the
//! host calls it inside `Terminal::draw`, before ratatui flushes) finds each
//! picture whose box is whole, blanks its cells and marks them `skip`, so
//! ratatui leaves them to the picture; [`take`], after the flush, writes the
//! pictures that are new or have moved. A picture that leaves frees its
//! cells: no longer skipped, they differ from the last frame, so ratatui
//! paints them and the picture's gone. A box drawn over (a popup) or cut off
//! is blank until it's whole again; a resize (ratatui clears the screen)
//! sends every picture again; [`clear`] blanks what's shown before a
//! hand-off.

use super::kitty::{base64, colours, recolour, DIACRITICS};
use super::{Logo, Paint};
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// Marker cells: U+F0000 + the picture's number (Supplementary Private Use
/// Area-A), then its row and column as kitty's diacritics.
const MARK: u32 = 0xF_0000;
/// Picture numbers that fit the area (U+F0000–U+FFFFD).
const MAX: u32 = 0xFFFE;

/// A picture, ready to write.
struct Pic {
    c: usize,
    r: usize,
    osc: String,
}

/// A picture on screen: its top-left cell and the cells' background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    x: u16,
    y: u16,
    n: u32,
    bg: Color,
}

#[derive(Default)]
struct State {
    /// (logo, size, colours) → picture number
    ids: HashMap<String, u32>,
    pics: Vec<Pic>,
    /// what the last frame showed
    shown: Vec<Shown>,
    /// the screen it was on
    area: Rect,
    /// escape sequences not written yet
    pending: String,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// A logo as a picture in `cols`×`rows` cells: marker cells in a box twice
/// as wide as tall, centred (at most 30 columns), the rest blank.
pub fn lines(logo: &Logo, cols: usize, rows: usize, paint: &Paint) -> Vec<Line<'static>> {
    with(|s| lines_in(s, logo, cols, rows, paint))
}

/// The frame's pictures found (see the module's notes); call it inside
/// `Terminal::draw`, after everything's drawn.
pub fn frame(buf: &mut Buffer) {
    with(|s| frame_in(s, buf))
}

/// The pictures to write after the frame's flushed (empty when none are
/// new).
pub fn take() -> String {
    with(|s| std::mem::take(&mut s.pending))
}

/// Escape sequences that blank every picture shown (and forget them, so
/// they're written again when they're drawn).
pub fn clear() -> String {
    with(clear_in)
}

/// The box a picture takes in `cols`×`rows`: (columns, rows, x, y), or None
/// when there's no room for one.
fn fit(cols: usize, rows: usize) -> Option<(usize, usize, usize, usize)> {
    let r = rows.min(cols / 2).min(DIACRITICS.len() / 2);
    let c = 2 * r;
    (r > 0).then(|| (c, r, (cols - c) / 2, (rows - r) / 2))
}

/// A logo's PNG in the theme's colours, the same as kitty gets.
fn png(logo: &Logo, paint: &Paint) -> Vec<u8> {
    recolour(logo.png, &colours(logo, paint))
}

fn lines_in(
    s: &mut State,
    logo: &Logo,
    cols: usize,
    rows: usize,
    paint: &Paint,
) -> Vec<Line<'static>> {
    let glyph = || {
        let g = super::glyph_with(logo.kind, logo.id, super::nerd(), paint);
        super::centred(vec![g], cols, rows)
    };
    let Some((c, r, x0, y0)) = fit(cols, rows) else {
        return glyph();
    };
    let key = format!(
        "{}/{}/{c}x{r}/{:?}",
        logo.kind.as_str(),
        logo.id,
        colours(logo, paint)
    );
    let n = match s.ids.get(&key) {
        Some(&n) => n,
        None if s.pics.len() as u32 >= MAX => return glyph(),
        None => {
            let n = s.pics.len() as u32;
            s.pics.push(Pic {
                c,
                r,
                osc: osc(&png(logo, paint), c, r),
            });
            s.ids.insert(key, n);
            n
        }
    };
    let mark = char::from_u32(MARK + n).unwrap_or(' ');
    (0..rows)
        .map(|y| {
            if y < y0 || y >= y0 + r {
                return Line::raw(" ".repeat(cols));
            }
            let mut cells = String::with_capacity(c * 8);
            for col in &DIACRITICS[..c] {
                cells.push(mark);
                cells.push(DIACRITICS[y - y0]);
                cells.push(*col);
            }
            Line::from(vec![
                Span::raw(" ".repeat(x0)),
                Span::raw(cells),
                Span::raw(" ".repeat(cols - c - x0)),
            ])
        })
        .collect()
}

/// A marker cell's (picture, row, column).
fn decode(sym: &str) -> Option<(u32, usize, usize)> {
    let mut it = sym.chars();
    let n = (it.next()? as u32).checked_sub(MARK).filter(|n| *n < MAX)?;
    let mut place = || {
        let d = it.next()?;
        DIACRITICS.iter().position(|x| *x == d)
    };
    let (row, col) = (place()?, place()?);
    Some((n, row, col))
}

fn frame_in(s: &mut State, buf: &mut Buffer) {
    let area = buf.area;
    if area != s.area {
        // a new size: ratatui cleared the screen, pictures and all
        s.shown.clear();
        s.area = area;
    }
    let mut marks: Vec<(u16, u16, u32, usize, usize)> = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let sym = buf[(x, y)].symbol();
            // (cheap: every marker's UTF-8 starts with F3)
            if sym.as_bytes().first() == Some(&0xF3) {
                if let Some((n, row, col)) = decode(sym) {
                    marks.push((x, y, n, row, col));
                }
            }
        }
    }
    let mut now: Vec<Shown> = Vec::new();
    for &(x, y, n, row, col) in &marks {
        let Some(pic) = s.pics.get(n as usize) else {
            continue;
        };
        if (row, col) != (0, 0)
            || x as usize + pic.c > area.right() as usize
            || y as usize + pic.r > area.bottom() as usize
        {
            continue;
        }
        let whole = (0..pic.r).all(|dy| {
            (0..pic.c)
                .all(|dx| decode(buf[(x + dx as u16, y + dy as u16)].symbol()) == Some((n, dy, dx)))
        });
        if !whole {
            continue;
        }
        now.push(Shown {
            x,
            y,
            n,
            bg: buf[(x, y)].bg,
        });
        for dy in 0..pic.r as u16 {
            for dx in 0..pic.c as u16 {
                buf[(x + dx, y + dy)].set_symbol(" ").set_skip(true);
            }
        }
    }
    // the rest (drawn over, cut off) is blank
    for &(x, y, ..) in &marks {
        let cell = &mut buf[(x, y)];
        if !cell.skip {
            cell.set_symbol(" ");
        }
    }
    for p in &now {
        if !s.shown.contains(p) {
            if let Some(pic) = s.pics.get(p.n as usize) {
                s.pending
                    .push_str(&at(p.x, p.y, pic.c, pic.r, p.bg, &pic.osc));
            }
        }
    }
    s.shown = now;
}

fn clear_in(s: &mut State) -> String {
    s.pending.clear();
    let mut out = String::new();
    for p in std::mem::take(&mut s.shown) {
        if let Some(pic) = s.pics.get(p.n as usize) {
            out.push_str(&blank(p.x, p.y, pic.c, pic.r));
        }
    }
    if out.is_empty() {
        out
    } else {
        format!("\x1b7\x1b[0m{out}\x1b8")
    }
}

/// Spaces over a box `c`×`r` at cell (`x`, `y`) (from 0), in the pen's
/// colours.
fn blank(x: u16, y: u16, c: usize, r: usize) -> String {
    (0..r as u16)
        .map(|dy| format!("\x1b[{};{}H{}", y + dy + 1, x + 1, " ".repeat(c)))
        .collect()
}

/// A PNG as an inline picture `c`×`r` cells big: `ESC ] 1337 ; File=… :
/// <base64> BEL`. iTerm2 keeps the aspect inside the box and leaves the
/// cursor on the picture's last row; `doNotMoveCursor` (WezTerm's) keeps it
/// put; iTerm2 skips arguments it doesn't know.
pub fn osc(png: &[u8], c: usize, r: usize) -> String {
    format!(
        "\x1b]1337;File=inline=1;size={};width={c};height={r};preserveAspectRatio=1;doNotMoveCursor=1:{}\x07",
        png.len(),
        base64(png)
    )
}

/// A picture `c`×`r` at cell (`x`, `y`) (from 0) on the cells' background
/// `bg` (WezTerm gives its cells the pen's): its box blanked first, so a
/// terminal that drops the picture shows nothing stale there; the cursor
/// put back where it was.
fn at(x: u16, y: u16, c: usize, r: usize, bg: Color, osc: &str) -> String {
    let bg = match bg {
        Color::Rgb(r, g, b) => format!("\x1b[48;2;{r};{g};{b}m"),
        Color::Indexed(i) => format!("\x1b[48;5;{i}m"),
        _ => String::new(),
    };
    format!(
        "\x1b7\x1b[0m{bg}{}\x1b[{};{}H{osc}\x1b8",
        blank(x, y, c, r),
        y + 1,
        x + 1
    )
}

/// A row of logos for printed output (`aleph icons`), each in a box
/// `cols`×`rows` starting at the column given: `rows` new lines, then each
/// picture written in its place; the cursor ends `rows` lines down, at the
/// start of the line (where the row's captions go).
pub fn printed(items: &[(&Logo, usize)], cols: usize, rows: usize, paint: &Paint) -> String {
    let mut out = "\n".repeat(rows);
    let Some((c, r, x0, y0)) = fit(cols, rows) else {
        return out;
    };
    out.push_str(&format!("\x1b[{rows}A"));
    for (logo, col) in items {
        out.push_str("\x1b7");
        if y0 > 0 {
            out.push_str(&format!("\x1b[{y0}B"));
        }
        out.push_str(&format!("\x1b[{}G", col + x0 + 1));
        out.push_str(&osc(&png(logo, paint), c, r));
        out.push_str("\x1b8");
    }
    out.push_str(&format!("\x1b[{rows}B\r"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::widgets::{Clear, Paragraph, Widget};

    fn paint() -> Paint {
        Paint {
            mono: false,
            truecolor: true,
            base: (0x11, 0x11, 0x11),
            text: (0xee, 0xee, 0xee),
        }
    }

    fn logo(id: &str) -> &'static Logo {
        crate::logo::Kind::ALL
            .iter()
            .find_map(|&k| crate::logo::find(k, id))
            .unwrap()
    }

    #[test]
    fn the_exact_bytes() {
        assert_eq!(
            osc(b"hello", 8, 4),
            "\x1b]1337;File=inline=1;size=5;width=8;height=4;preserveAspectRatio=1;doNotMoveCursor=1:aGVsbG8=\x07"
        );
        // at a cell (1-based on the wire): the cursor saved, the box blanked,
        // the picture, the cursor put back
        assert_eq!(
            at(2, 3, 2, 2, Color::Reset, "P"),
            "\x1b7\x1b[0m\x1b[4;3H  \x1b[5;3H  \x1b[4;3HP\x1b8"
        );
        assert_eq!(
            at(0, 0, 1, 1, Color::Rgb(1, 2, 3), "P"),
            "\x1b7\x1b[0m\x1b[48;2;1;2;3m\x1b[1;1H \x1b[1;1HP\x1b8"
        );
        assert_eq!(
            at(0, 0, 1, 1, Color::Indexed(236), "P"),
            "\x1b7\x1b[0m\x1b[48;5;236m\x1b[1;1H \x1b[1;1HP\x1b8"
        );
        // a real logo: its committed PNG, recoloured as kitty's is
        let ubuntu = logo("ubuntu");
        let mut s = State::default();
        lines_in(&mut s, ubuntu, 6, 3, &paint());
        let want = recolour(ubuntu.png, &colours(ubuntu, &paint()));
        assert_eq!(s.pics[0].osc, osc(&want, 6, 3));
        assert!(s.pics[0].osc.starts_with(&format!(
            "\x1b]1337;File=inline=1;size={};width=6;height=3;",
            want.len()
        )));
        assert_eq!(&want[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn markers_fill_the_box_and_say_where_they_are() {
        let mut s = State::default();
        let l = lines_in(&mut s, logo("claude"), 10, 4, &paint());
        assert_eq!(l.len(), 4);
        assert!(l.iter().all(|x| x.width() == 10), "{l:?}");
        // 8×4 cells, a blank column each side; each cell one column to ratatui
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        Paragraph::new(l).render(buf.area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(decode(buf[(1, 0)].symbol()), Some((0, 0, 0)));
        assert_eq!(decode(buf[(8, 3)].symbol()), Some((0, 3, 7)));
        assert_eq!(buf[(9, 3)].symbol(), " ");
        // one picture per logo, size and colours
        lines_in(&mut s, logo("claude"), 10, 4, &paint());
        assert_eq!(s.pics.len(), 1);
        lines_in(&mut s, logo("claude"), 6, 3, &paint());
        lines_in(&mut s, logo("ubuntu"), 6, 3, &paint());
        assert_eq!(s.pics.len(), 3);
        // no room: the glyph, no picture
        let g = lines_in(&mut s, logo("ubuntu"), 1, 1, &paint());
        assert!(decode(&g[0].to_string()).is_none());
        assert_eq!(s.pics.len(), 3);
        assert_eq!(decode("x"), None);
        assert_eq!(decode("\u{F0000}"), None);
    }

    /// A 20×6 screen with `l` drawn at (x, y) and, maybe, a popup over it.
    fn screen(s: &mut State, l: &[Line<'static>], x: u16, y: u16, popup: Option<Rect>) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 6));
        let w = l.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
        let area = Rect::new(x, y, w, l.len() as u16).intersection(buf.area);
        Paragraph::new(l.to_vec()).render(area, &mut buf);
        if let Some(p) = popup {
            Clear.render(p, &mut buf);
            Paragraph::new("popup").render(p, &mut buf);
        }
        frame_in(s, &mut buf);
        buf
    }

    #[test]
    fn whole_pictures_are_written_once_and_kept_from_ratatui() {
        let mut s = State::default();
        let l = lines_in(&mut s, logo("ubuntu"), 6, 3, &paint());
        let b1 = screen(&mut s, &l, 2, 1, None);
        // its cells: blank, skipped
        for y in 1..4 {
            for x in 2..8 {
                assert!(b1[(x, y)].skip && b1[(x, y)].symbol() == " ", "{x},{y}");
            }
        }
        assert!(!b1[(1, 1)].skip && !b1[(8, 1)].skip && !b1[(2, 0)].skip);
        let p = std::mem::take(&mut s.pending);
        assert_eq!(p, at(2, 1, 6, 3, Color::Reset, &s.pics[0].osc));
        // the same frame again: nothing to write
        let b2 = screen(&mut s, &l, 2, 1, None);
        assert!(s.pending.is_empty());
        assert!(b1.diff(&b2).is_empty());
        // moved down a row: written there; the row it left is painted again
        let b3 = screen(&mut s, &l, 2, 2, None);
        assert_eq!(s.pending, at(2, 2, 6, 3, Color::Reset, &s.pics[0].osc));
        s.pending.clear();
        let painted: Vec<(u16, u16)> = b2.diff(&b3).iter().map(|(x, y, _)| (*x, *y)).collect();
        assert_eq!(painted, (2..8).map(|x| (x, 1)).collect::<Vec<_>>());
        // a popup over part of it: no picture, its cells blank and painted
        let b4 = screen(&mut s, &l, 2, 2, Some(Rect::new(6, 3, 8, 2)));
        assert!(s.pending.is_empty() && s.shown.is_empty());
        assert!((2..8).all(|x| !b4[(x, 2)].skip && b4[(x, 2)].symbol() == " "));
        assert!(b3.diff(&b4).iter().any(|(x, y, _)| (*x, *y) == (2, 2)));
        // the popup gone: written again
        screen(&mut s, &l, 2, 2, None);
        assert_eq!(s.pending, at(2, 2, 6, 3, Color::Reset, &s.pics[0].osc));
        s.pending.clear();
        // cut off by the screen's edge: blank
        let b5 = screen(&mut s, &l, 16, 4, None);
        assert!(s.pending.is_empty() && s.shown.is_empty());
        assert!((0..20)
            .all(|x| (0..6).all(|y| !b5[(x, y)].skip && decode(b5[(x, y)].symbol()).is_none())));
    }

    #[test]
    fn a_new_background_or_size_sends_it_again() {
        let mut s = State::default();
        let l = lines_in(&mut s, logo("debian"), 6, 3, &paint());
        screen(&mut s, &l, 0, 0, None);
        s.pending.clear();
        // selected: the row's background changes under the picture
        let sel: Vec<Line<'static>> = l
            .iter()
            .map(|x| {
                x.clone()
                    .style(Style::new().bg(Color::Rgb(0x26, 0x26, 0x26)))
            })
            .collect();
        screen(&mut s, &sel, 0, 0, None);
        assert_eq!(
            s.pending,
            at(0, 0, 6, 3, Color::Rgb(0x26, 0x26, 0x26), &s.pics[0].osc)
        );
        s.pending.clear();
        // a resize: ratatui cleared the screen
        let mut big = Buffer::empty(Rect::new(0, 0, 30, 8));
        Paragraph::new(sel.clone()).render(Rect::new(0, 0, 6, 3), &mut big);
        frame_in(&mut s, &mut big);
        assert!(!s.pending.is_empty());
    }

    #[test]
    fn clearing_blanks_what_is_shown() {
        let mut s = State::default();
        let l = lines_in(&mut s, logo("fedora"), 6, 3, &paint());
        screen(&mut s, &l, 3, 2, None);
        assert_eq!(
            clear_in(&mut s),
            "\x1b7\x1b[0m\x1b[3;4H      \x1b[4;4H      \x1b[5;4H      \x1b8"
        );
        assert!(s.pending.is_empty() && s.shown.is_empty());
        assert_eq!(clear_in(&mut s), "");
        // forgotten: the next frame writes it again
        screen(&mut s, &l, 3, 2, None);
        assert!(!s.pending.is_empty());
    }

    #[test]
    fn ratatui_leaves_a_picture_alone_and_paints_over_it_once_it_is_gone() {
        use ratatui::backend::TestBackend;
        let mut s = State::default();
        let l = lines_in(&mut s, logo("nvidia"), 6, 3, &paint());
        let mut t = ratatui::Terminal::new(TestBackend::new(10, 4)).unwrap();
        let fill = |c: &str| vec![Line::raw(c.repeat(10)); 4];
        t.draw(|f| f.render_widget(Paragraph::new(fill("x")), f.area()))
            .unwrap();
        t.draw(|f| {
            f.render_widget(Paragraph::new(l.clone()), Rect::new(1, 0, 6, 3));
            frame_in(&mut s, f.buffer_mut());
        })
        .unwrap();
        // under the picture the screen still has what was there (the picture covers it)
        let b = t.backend().buffer().clone();
        assert_eq!(b[(1, 0)].symbol(), "x");
        assert_eq!(b[(6, 2)].symbol(), "x");
        assert_eq!(b[(0, 0)].symbol(), " ", "beside it, painted");
        assert_eq!(b[(1, 3)].symbol(), " ", "below it, painted");
        // gone: painted again
        t.draw(|f| {
            f.render_widget(Paragraph::new(fill("y")), f.area());
            frame_in(&mut s, f.buffer_mut());
        })
        .unwrap();
        let b = t.backend().buffer();
        assert!((0..10).all(|x| (0..4).all(|y| b[(x, y)].symbol() == "y")));
        assert!(s.shown.is_empty());
    }

    #[test]
    fn printed_rows_make_room_then_fill_it() {
        let ubuntu = logo("ubuntu");
        let p = paint();
        let want = osc(&png(ubuntu, &p), 8, 4);
        assert_eq!(
            printed(&[(ubuntu, 2), (ubuntu, 12)], 8, 4, &p),
            format!("\n\n\n\n\x1b[4A\x1b7\x1b[3G{want}\x1b8\x1b7\x1b[13G{want}\x1b8\x1b[4B\r")
        );
        // a box wider than the picture: centred in it; taller: lower
        let want = osc(&png(ubuntu, &p), 6, 3);
        assert_eq!(
            printed(&[(ubuntu, 4)], 10, 3, &p),
            format!("\n\n\n\x1b[3A\x1b7\x1b[7G{want}\x1b8\x1b[3B\r")
        );
        assert_eq!(
            printed(&[(ubuntu, 0)], 6, 5, &p),
            format!("\n\n\n\n\n\x1b[5A\x1b7\x1b[1B\x1b[1G{want}\x1b8\x1b[5B\r")
        );
        assert_eq!(printed(&[(ubuntu, 0)], 1, 1, &p), "\n");
    }
}
