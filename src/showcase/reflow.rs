//! Laying whole-name stand-ins into a row that was drawn for the real
//! words: each swap takes its place and the rest of its run of text moves
//! along, into the spaces that end the run (a column's padding, the gap
//! before the next column, a line's end). A run stops at two spaces, a
//! border or block glyph, or a cell that can't move (a wide character, an
//! image). When the spaces run out, the run is cut with `…` — never the
//! cells after it, so columns, borders and mouse targets stay put.

use super::scan::Edit;

/// A row's cell: its character and what it carries (a style, a span), or
/// `None` for one that must stay where it is.
pub type Slot<T> = Option<(char, T)>;

/// Glyphs that end a run: lines and boxes, blocks, braille (charts), and
/// the private-use area (Nerd Font icons, powerline caps).
fn boundary(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0x2800..=0x28FF | 0xE000..=0xF8FF) || c == '\u{0}'
}

fn space<T>(s: &Slot<T>) -> bool {
    matches!(s, Some((' ', _)))
}

/// `row` with `edits` (left to right, not overlapping) laid in. With `grow`
/// a run with nothing after it may make the row longer (a printed line);
/// else the row keeps its width (a terminal row).
pub fn reflow<T: Clone>(row: &[Slot<T>], edits: &[Edit], grow: bool) -> Vec<Slot<T>> {
    let mut row: Vec<Slot<T>> = row.to_vec();
    // right to left: an edit only moves what's after it
    for (a, b, r) in edits.iter().rev() {
        let (a, b) = (*a, *b);
        if b > row.len() || a >= b || row[a..b].iter().any(Option::is_none) {
            continue;
        }
        let n = row.len();
        // the run after the word, up to two spaces, a border, a fixed cell
        let mut e = b;
        while e < n {
            match &row[e] {
                Some((c, _)) if boundary(*c) => break,
                Some((' ', _)) if e + 1 >= n || space(&row[e + 1]) => break,
                Some(_) => e += 1,
                None => break,
            }
        }
        // spaces at its end belong to the padding
        while e > b && space(&row[e - 1]) {
            e -= 1;
        }
        // the spaces that end it, and whether anything comes after them
        let mut p = e;
        while p < n && space(&row[p]) {
            p += 1;
        }
        let follows = p < n;
        let usable = if follows {
            (p - e).saturating_sub(1)
        } else {
            p - e
        };
        let tag = |k: usize| row[(a + k).min(b - 1)].as_ref().map(|x| x.1.clone());
        let mut seg: Vec<(char, T)> = r
            .iter()
            .enumerate()
            .filter_map(|(k, c)| tag(k).map(|t| (*c, t)))
            .collect();
        seg.extend(row[b..e].iter().flatten().cloned());
        let room = (e - a) + usable;
        if seg.len() > room && !(grow && !follows) {
            seg.truncate(room);
            if let Some(last) = seg.last_mut() {
                last.0 = '…';
            }
        }
        // write it, then spaces (in the cells' own style) up to where the
        // run's spaces ended
        let end = p.max(a + seg.len());
        let blank: Vec<Slot<T>> = (a..end)
            .map(|k| {
                row.get(k)
                    .and_then(|s| s.as_ref())
                    .or_else(|| row[a..].iter().rev().flatten().next())
                    .map(|(_, t)| (' ', t.clone()))
            })
            .collect();
        let mut out: Vec<Slot<T>> = blank;
        for (k, x) in seg.into_iter().enumerate() {
            out[k] = Some(x);
        }
        row.splice(a..p, out);
        if !grow {
            row.truncate(n);
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(s: &str) -> Vec<Slot<u8>> {
        s.chars().map(|c| Some((c, 0))).collect()
    }
    fn text(r: &[Slot<u8>]) -> String {
        r.iter().map(|s| s.as_ref().map_or('#', |x| x.0)).collect()
    }
    fn edit(s: &str, real: &str, fake: &str) -> Edit {
        let at = s.find(real).unwrap();
        let a = s[..at].chars().count();
        (a, a + real.chars().count(), fake.chars().collect())
    }

    #[test]
    fn a_longer_name_takes_its_columns_padding() {
        let s = "▌b31       ⠿ working";
        let col = |t: &str| t.chars().position(|c| c == '⠿');
        let r = reflow(&row(s), &[edit(s, "b31", "normandy")], false);
        assert_eq!(text(&r), "▌normandy  ⠿ working");
        // a shorter one leaves spaces, the next column where it was
        let r = reflow(&row(s), &[edit(s, "b31", "io")], false);
        assert!(text(&r).starts_with("▌io "));
        assert_eq!(col(&text(&r)), col(s));
    }

    #[test]
    fn the_rest_of_a_sentence_moves_along() {
        let s = "approved hunt on b31 (1)            ";
        let e = [edit(s, "hunt", "black_mesa"), edit(s, "b31", "normandy")];
        let r = reflow(&row(s), &e, false);
        assert_eq!(text(&r), "approved black_mesa on normandy (1) ");
        assert_eq!(r.len(), s.chars().count());
    }

    #[test]
    fn without_room_the_run_is_cut_and_the_border_stays() {
        let s = "│ b31 up │";
        let r = reflow(&row(s), &[edit(s, "b31", "millennium-falcon")], false);
        assert_eq!(text(&r), "│ mille… │");
        let s = "x b31  y";
        let r = reflow(&row(s), &[edit(s, "b31", "normandy")], false);
        assert_eq!(text(&r), "x nor… y");
    }

    #[test]
    fn a_printed_line_may_grow_at_its_end() {
        let s = "✓ approved hunt on b31";
        let r = reflow(&row(s), &[edit(s, "b31", "normandy")], true);
        assert_eq!(text(&r), "✓ approved hunt on normandy");
    }

    #[test]
    fn cells_that_cant_move_are_left_alone() {
        let mut r = row("b31 ab");
        r[1] = None;
        let out = reflow(&r, &[(0, 3, "normandy".chars().collect())], false);
        assert_eq!(text(&out), "b#1 ab");
    }
}
