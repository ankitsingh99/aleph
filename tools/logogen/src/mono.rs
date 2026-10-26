//! Monogram tiles for agents and servers without a usable mark, and the few
//! logos drawn from plain shapes. The letters come from a tiny stroke font
//! defined here (polylines, round caps), never from a system font, so the
//! SVGs are the same on every machine.
//!
//! Agents get a rounded square, servers a circle — so a server never looks
//! like an agent — with the letters cut out (transparent), which keeps every
//! logo a single colour: on a terminal the cut-outs show its own background.

/// A glyph: polylines in a box `w` wide and 10 tall (y grows downwards).
/// `fine` ones (⌘) are drawn larger with thinner strokes, or they'd clog.
struct Glyph {
    w: f32,
    strokes: Vec<Vec<(f32, f32)>>,
    fine: bool,
}

/// Points along an elliptical arc; angles in degrees, 0° = right, 90° = down.
fn arc(cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    let n = (((a1 - a0).abs() / 12.0).ceil() as usize).max(2);
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

fn cat(parts: &[Vec<(f32, f32)>]) -> Vec<(f32, f32)> {
    parts.iter().flatten().copied().collect()
}

fn glyph(c: char) -> Glyph {
    let g = |w: f32, strokes: Vec<Vec<(f32, f32)>>| Glyph {
        w,
        strokes,
        fine: false,
    };
    let o = || arc(3.5, 5.0, 3.5, 5.0, 0.0, 360.0);
    let p_bowl = || {
        cat(&[
            vec![(0.0, 10.0), (0.0, 0.0), (3.0, 0.0)],
            arc(3.0, 2.8, 2.8, 2.8, -90.0, 90.0),
            vec![(0.0, 5.6)],
        ])
    };
    match c {
        'A' => g(
            6.4,
            vec![
                vec![(0.0, 10.0), (3.2, 0.0), (6.4, 10.0)],
                vec![(1.15, 6.4), (5.25, 6.4)],
            ],
        ),
        'C' => g(6.0, vec![arc(3.4, 5.0, 3.4, 5.0, -40.0, -320.0)]),
        'D' => g(
            6.2,
            vec![cat(&[
                vec![(0.0, 0.0), (0.0, 10.0)],
                arc(2.4, 5.0, 3.8, 5.0, 90.0, -90.0),
                vec![(0.0, 0.0)],
            ])],
        ),
        'F' => g(
            5.6,
            vec![
                vec![(5.6, 0.0), (0.0, 0.0), (0.0, 10.0)],
                vec![(0.0, 4.9), (4.4, 4.9)],
            ],
        ),
        'G' => g(
            6.6,
            vec![cat(&[
                arc(3.4, 5.0, 3.4, 5.0, -40.0, -340.0),
                vec![(6.6, 5.4), (3.8, 5.4)],
            ])],
        ),
        'H' => g(
            6.0,
            vec![
                vec![(0.0, 0.0), (0.0, 10.0)],
                vec![(6.0, 0.0), (6.0, 10.0)],
                vec![(0.0, 5.0), (6.0, 5.0)],
            ],
        ),
        'J' => g(
            5.0,
            vec![cat(&[
                vec![(5.0, 0.0)],
                arc(2.6, 7.0, 2.4, 3.0, 0.0, 180.0),
            ])],
        ),
        'K' => g(
            6.0,
            vec![
                vec![(0.0, 0.0), (0.0, 10.0)],
                vec![(5.8, 0.0), (0.3, 5.6)],
                vec![(2.3, 3.8), (6.0, 10.0)],
            ],
        ),
        'L' => g(5.4, vec![vec![(0.0, 0.0), (0.0, 10.0), (5.4, 10.0)]]),
        'M' => g(
            7.4,
            vec![vec![
                (0.0, 10.0),
                (0.0, 0.0),
                (3.7, 6.4),
                (7.4, 0.0),
                (7.4, 10.0),
            ]],
        ),
        'O' => g(7.0, vec![o()]),
        'P' => g(5.8, vec![p_bowl()]),
        'Q' => g(7.0, vec![o(), vec![(4.3, 7.2), (7.2, 10.6)]]),
        'R' => g(6.0, vec![p_bowl(), vec![(2.8, 5.6), (6.0, 10.0)]]),
        'S' => g(
            6.0,
            vec![cat(&[
                arc(3.0, 2.65, 2.85, 2.65, -20.0, -270.0),
                arc(3.0, 7.6, 3.0, 2.4, -90.0, 160.0),
            ])],
        ),
        'T' => g(
            6.2,
            vec![vec![(0.0, 0.0), (6.2, 0.0)], vec![(3.1, 0.0), (3.1, 10.0)]],
        ),
        'U' => g(
            6.0,
            vec![cat(&[
                vec![(0.0, 0.0)],
                arc(3.0, 7.0, 3.0, 3.0, 180.0, 0.0),
                vec![(6.0, 0.0)],
            ])],
        ),
        'W' => g(
            8.0,
            vec![vec![
                (0.0, 0.0),
                (2.0, 10.0),
                (4.0, 3.2),
                (6.0, 10.0),
                (8.0, 0.0),
            ]],
        ),
        'X' => g(
            6.2,
            vec![vec![(0.0, 0.0), (6.2, 10.0)], vec![(6.2, 0.0), (0.0, 10.0)]],
        ),
        'π' => g(
            7.4,
            vec![
                vec![(0.0, 1.2), (7.4, 1.2)],
                vec![(2.3, 1.2), (2.0, 10.0)],
                vec![(5.3, 1.2), (5.5, 10.0)],
            ],
        ),
        '∀' => g(
            6.4,
            vec![
                vec![(0.0, 0.0), (3.2, 10.0), (6.4, 0.0)],
                vec![(1.15, 3.6), (5.25, 3.6)],
            ],
        ),
        // ⌘: a square whose sides run on into four loops
        '⌘' => {
            let (a, b, r) = (3.2, 6.8, 1.6);
            let mut s = vec![
                vec![(r, a), (10.0 - r, a)],
                vec![(r, b), (10.0 - r, b)],
                vec![(a, r), (a, 10.0 - r)],
                vec![(b, r), (b, 10.0 - r)],
            ];
            for (x, y) in [(r, r), (10.0 - r, r), (r, 10.0 - r), (10.0 - r, 10.0 - r)] {
                s.push(arc(x, y, r, r, 0.0, 360.0));
            }
            Glyph {
                fine: true,
                ..g(10.0, s)
            }
        }
        _ => panic!("the monogram font has no {c:?}: add it to tools/logogen/src/mono.rs"),
    }
}

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The SVG of a monogram tile: `text` cut out of a rounded square (`round`
/// = false, agents) or a circle (servers), in `color`.
pub fn monogram(what: &str, text: &str, color: &str, round: bool) -> String {
    let glyphs: Vec<Glyph> = text.chars().map(glyph).collect();
    let n = glyphs.len().clamp(1, 3);
    let gap = 2.0;
    let w: f32 = glyphs.iter().map(|g| g.w).sum::<f32>() + gap * (glyphs.len() as f32 - 1.0);
    // cap height and stroke shrink as letters are added; wide runs shrink to fit
    let (cap, stroke) = match glyphs.as_slice() {
        [g] if g.fine => (13.5, 1.7),
        _ => [(11.0, 2.7), (8.8, 2.3), (6.6, 1.9)][n - 1],
    };
    let max_w = if round { 15.5 } else { 16.5 };
    let s = (cap / 10.0f32).min(max_w / w);
    let (x0, y0) = (12.0 - w * s / 2.0, 12.0 - 10.0 * s / 2.0);
    let mut d = String::new();
    let mut off = 0.0;
    for g in &glyphs {
        for line in &g.strokes {
            for (i, (x, y)) in line.iter().enumerate() {
                d.push(if i == 0 { 'M' } else { 'L' });
                d.push_str(&num(x0 + (off + x) * s));
                d.push(' ');
                d.push_str(&num(y0 + y * s));
            }
        }
        off += g.w + gap;
    }
    let tile = if round {
        format!(r#"<circle cx="12" cy="12" r="11" fill="{color}" mask="url(#m)"/>"#)
    } else {
        format!(
            r#"<rect x="1" y="1" width="22" height="22" rx="5.5" fill="{color}" mask="url(#m)"/>"#
        )
    };
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">"#,
            "<title>{what} (monogram drawn for Aleph)</title>",
            r##"<mask id="m"><rect width="24" height="24" fill="#fff"/>"##,
            r##"<path fill="none" stroke="#000" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round" d="{d}"/>"##,
            "</mask>{tile}</svg>\n"
        ),
        what = xml(what),
        stroke = num(stroke),
        d = d,
        tile = tile
    )
}

/// The SVG of a logo drawn from plain shapes (`body` inside a 24×24 viewBox).
pub fn drawn(what: &str, body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><title>{} (drawn for Aleph)</title>{body}</svg>
"#,
        xml(what)
    )
}
