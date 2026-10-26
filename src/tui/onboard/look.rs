//! Step 2, Look & feel: theme, accent, room (scale), mouse, the spinner,
//! icons and logos, what enter opens. Theme, accent, scale, spinner and
//! icons switch live — the whole of setup follows as you change them — and
//! the preview beside the rows draws a small dashboard in the palette you're
//! choosing (on its own paper when it's the other family from your
//! terminal's) with a logo gallery at the chosen style and size.

use super::rows::Row;
use super::{Aside, Ev, Hand, Wiz};
use crate::logo::{self, Kind, Paint};
use crate::ui::{self, c};
use ratatui::prelude::*;

#[derive(Default)]
pub(crate) struct State {
    /// preview as NO_COLOR would show it
    pub mono: bool,
}

fn opts(v: &[&str]) -> Vec<(String, String)> {
    v.iter().map(|x| (x.to_string(), x.to_string())).collect()
}

/// The accents to pick from: Aleph's gold, Claude's orange, then the
/// machine palette (any #hex works too).
pub(crate) const ACCENTS: &[(&str, &str)] = &[
    ("gold", "gold"),
    ("claude", "claude orange"),
    ("blue", "blue"),
    ("mauve", "mauve"),
    ("teal", "teal"),
    ("green", "green"),
    ("peach", "peach"),
    ("pink", "pink"),
    ("sky", "sky"),
    ("lavender", "lavender"),
];

/// An `accent` value as a colour for `init` (None: the theme's own gold).
pub(crate) fn accent_color(v: &str) -> Option<Color> {
    match v.trim() {
        "" | "gold" | "aleph" => None,
        o => ui::parse_color(o),
    }
}

pub(crate) fn enter(_w: &mut Wiz) {}

/// A setting changed: the ones setup can show live, live (never in tests:
/// the palette and the spinner are the whole process's).
pub(crate) fn changed(w: &mut Wiz, key: &str) {
    if w.quiet {
        return;
    }
    match key {
        "theme" | "accent" => {
            let theme = w.value("theme");
            // the theme first: named accents have a light and a dark shade
            ui::c::init(&theme, None);
            if let Some(a) = accent_color(&w.value("accent")) {
                ui::c::init(&theme, Some(a));
            }
        }
        "ui.spinner" => ui::set_spinner(&w.value("ui.spinner")),
        "ui_scale" => ui::set_scale(&w.value("ui_scale")),
        "icons" => crate::icons::init(&w.value("icons")),
        _ => {}
    }
}

/// Leaving a step: what's live follows the plan (or the settings again).
pub(crate) fn leave(w: &mut Wiz) {
    for k in ["theme", "ui.spinner", "ui_scale", "icons"] {
        changed(w, k);
    }
}

/// `■■` in a colour (nothing under NO_COLOR).
fn chip(col: Option<Color>) -> Vec<Span<'static>> {
    match col {
        Some(c) if !ui::c::mono() => vec![Span::raw("  "), Span::styled("■■", Style::new().fg(c))],
        _ => vec![],
    }
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let ch = |key: &str, label: &str, o: Vec<(String, String)>| {
        let cur = w.value(key);
        let mut o = o;
        if !o.iter().any(|(v, _)| *v == cur) {
            o.push((cur.clone(), cur.clone()));
        }
        Row::choice(key, label, o, cur, w.pending(key))
    };
    let theme = w.value("theme");
    let light_term = ui::c::terminal_is_light();
    let theme_note = match theme.as_str() {
        "light" if !light_term => "Paper is for light terminals — hard to read on a dark one",
        "dark" if light_term => "Graphite is for dark terminals — this one looks light",
        "auto" if light_term => "→ Paper (the terminal looks light)",
        "auto" => "→ Graphite",
        _ => "",
    };
    let accent = w.value("accent");
    let accent_col = accent_color(&accent).or_else(|| Some(c::pal().accent_default()));
    vec![
        Row::head("Colours"),
        ch(
            "theme",
            "theme",
            ui::c::THEMES
                .iter()
                .map(|(n, _, _)| (n.to_string(), n.to_string()))
                .collect(),
        )
            .note(theme_note)
            .why(
                "auto follows the terminal (it's asked for its background once a run; else COLORFGBG says light or dark); dark is Graphite, light is Paper. Setup switches as you choose; the terminal's own background is never painted.",
            ),
        ch(
            "accent",
            "accent",
            ACCENTS
                .iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect(),
        )
        .spans(chip(accent_col))
        .why("The brand colour: selection bars, focused panels, the ℵ. Aleph's gold (#E2B451), Claude's orange, a colour from the machine palette, or any #hex (custom… below). Setup switches as you choose."),
        Row::button("accent-hex", "custom accent (#hex)…")
            .why("Any colour as #rrggbb or #rgb, e.g. #89b4fa."),
        Row::local(
            "mono",
            "preview",
            vec![
                ("colour".into(), "in colour".into()),
                ("mono".into(), "as NO_COLOR".into()),
            ],
            usize::from(w.look.mono),
        )
        .why("NO_COLOR=1 in the environment turns every colour off; meaning is then carried by glyphs, bold and reverse video. This only changes the preview."),
        Row::head("Room"),
        ch("ui_scale", "scale", opts(&["compact", "comfortable", "large"])).why(
            "A terminal app can't change the font, so bigger is room: padding inside panels, margins, a blank line between list rows (large). Setup shows it live. The font is your terminal's (cmd +).",
        ),
        Row::toggle("mouse", "mouse", w.flag("mouse"), w.pending("mouse"))
            .why("Click, double-click and the wheel in the dashboard. Off leaves the terminal's own selection alone."),
        ch("ui.spinner", "spinner", opts(&["aleph", "stars", "dots", "line"]))
            .why("The busy spinner: aleph (a pulsing ℵ), stars (Claude Code's), braille dots, a plain line. Setup's own spinner follows it live."),
        Row::head("Logos"),
        ch("icons", "icons", opts(&["auto", "nerd", "emoji", "text"])).why(
            "Logos for machines, GPUs and agents in one-row places: auto uses Nerd Font glyphs when your terminal's font has them, else emoji; text is plain names.",
        ),
        ch("ui.logos", "logos", opts(&["auto", "images", "thumbnails", "ascii", "glyphs", "text"])).why(
            "How logos are drawn where there's room: auto = pictures in kitty and Ghostty, block thumbnails elsewhere, glyphs or names in tight spots. aleph icons shows them all.",
        ),
        ch("ui.logo_size", "logo size", opts(&["small", "medium", "large"]))
            .why("small 6×3 cells, medium 8×4, large 16×8 — where there's room for it."),
        Row::head("Behaviour"),
        ch("enter_opens", "enter opens", opts(&["focus", "attach", "direct"])).why(
            "What enter does on a session in Fleet: focus (Aleph's own full-screen view, with the queue and approvals), attach (herdr itself), direct (only that agent's terminal).",
        ),
    ]
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Local(id, i) if id == "mono" => w.look.mono = i == 1,
        Ev::Button(id) if id == "accent-hex" => {
            let cur = w.value("accent");
            let v = if cur.starts_with('#') {
                cur
            } else {
                String::new()
            };
            w.edit(
                "Custom accent",
                "a colour as #rrggbb or #rgb, e.g. #89b4fa",
                &v,
                "accent-hex",
            );
        }
        Ev::Input(id, v) if id == "accent-hex" => {
            if v.is_empty() {
                w.set("accent", "");
            } else if v.starts_with('#') && ui::parse_color(&v).is_some() {
                w.set("accent", &v);
            } else {
                w.toast(true, format!("{v} isn't a colour — #rrggbb or #rgb"));
                return None;
            }
            changed(w, "accent");
        }
        _ => {}
    }
    None
}

// ------------------------------------------------------------------ the preview

/// The palette the preview draws with (theme, accent, NO_COLOR as chosen).
pub(crate) fn palette(theme: &str, accent: &str, mono: bool) -> ui::c::Pal {
    if mono || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return ui::c::Pal::mono();
    }
    let mut p = ui::c::pal_of(theme);
    p.accent = match accent {
        "gold" | "aleph" | "" => p.accent_default(),
        "claude" => Color::Rgb(0xd9, 0x77, 0x57),
        other => ui::parse_color(other).unwrap_or(p.accent),
    };
    p
}

/// A small dashboard in palette `p`, `w` columns wide.
pub(crate) fn mock(p: &ui::c::Pal, scale: &str, spin: &str, w: usize) -> Vec<Line<'static>> {
    let fg = |col: Color| {
        if p.mono {
            Style::new()
        } else {
            Style::new().fg(col)
        }
    };
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let pad = match scale {
        "compact" => 0,
        "large" => 2,
        _ => 1,
    };
    let inner = w.saturating_sub(2 + pad * 2);
    let border = |l: &str, r: &str, mid: Vec<Span<'static>>| {
        let used: usize = mid.iter().map(|s| ui::width(&s.content)).sum();
        let mut v = vec![Span::styled(l.to_string(), fg(p.line))];
        v.extend(mid);
        v.push(Span::raw(" ".repeat(w.saturating_sub(used + 2))));
        v.push(Span::styled(r.to_string(), fg(p.line)));
        Line::from(v)
    };
    let row = |sel: bool, glyph: &str, col: Color, name: &str, host: &str, task: &str| {
        let mut st = Style::new();
        if sel {
            st = if p.mono {
                st.add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                st.fg(p.text).bg(p.sel).add_modifier(Modifier::BOLD)
            };
        }
        let mut v: Vec<Span<'static>> = vec![Span::raw(" ".repeat(pad))];
        v.push(Span::styled(
            if sel { "┃" } else { " " }.to_string(),
            fg(p.accent).patch(st),
        ));
        let body = format!(
            " {glyph} {} {} {}",
            ui::pad(name, 7),
            ui::pad(host, 5),
            task
        );
        let body = ui::pad(
            &ui::trunc(&body, inner.saturating_sub(1)),
            inner.saturating_sub(1),
        );
        let (g, rest) = body.split_at(body.char_indices().nth(3).map_or(body.len(), |x| x.0));
        v.push(Span::styled(
            g.to_string(),
            fg(col).patch(st).add_modifier(Modifier::BOLD),
        ));
        v.push(Span::styled(rest.to_string(), fg(p.text).patch(st)));
        v.push(Span::raw(" ".repeat(pad)));
        border("│", "│", v)
    };
    let title = format!("╭ Fleet {}", "─".repeat(w.saturating_sub(9)));
    let mut out = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} aleph ", crate::brand::MARK),
                fg(p.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("● ", fg(p.green)),
            Span::styled("2/2 machines · ", fg(p.muted)),
            Span::styled(format!("{spin} 1 working "), fg(p.blue)),
            Span::styled("▲ 1 needs you", fg(p.peach).patch(bold)),
        ]),
        Line::from(vec![
            Span::styled(ui::trunc(&title, w.saturating_sub(1)), fg(p.accent)),
            Span::styled("╮", fg(p.accent)),
        ]),
    ];
    if pad > 0 {
        out.push(border("│", "│", vec![]));
    }
    out.push(row(
        true,
        "▲",
        p.peach,
        "temper",
        "b31",
        "Run the cargo benches",
    ));
    if pad > 1 {
        out.push(border("│", "│", vec![]));
    }
    out.push(row(
        false,
        "✻",
        p.blue,
        "ignite",
        "b31",
        "Profile the engine",
    ));
    if pad > 1 {
        out.push(border("│", "│", vec![]));
    }
    out.push(row(
        false,
        "✓",
        p.green,
        "weave",
        "gpu2",
        "Distil the teacher",
    ));
    out.push(row(
        false,
        "◷",
        p.mauve,
        "drift",
        "gpu2",
        "at limit · 2:40am",
    ));
    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(w.saturating_sub(2))),
        fg(p.accent),
    )));
    let key = |k: &str| {
        Span::styled(
            format!(" {k} "),
            if p.mono {
                Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::new()
                    .fg(p.text)
                    .bg(p.line)
                    .add_modifier(Modifier::BOLD)
            },
        )
    };
    out.push(Line::from(vec![
        Span::raw(" "),
        key("enter"),
        Span::styled(" open  ", fg(p.muted)),
        key("i"),
        Span::styled(" new  ", fg(p.muted)),
        key("?"),
        Span::styled(" help", fg(p.muted)),
    ]));
    out.into_iter()
        .map(|l| super::rows::fit(l.spans, w))
        .collect()
}

/// Paint `lines` on the preview palette's own background (a swatch), only
/// when it's the other family from the terminal's (a light preview on a dark
/// terminal); otherwise the terminal's background stays.
fn swatch(lines: Vec<Line<'static>>, p: &ui::c::Pal, w: usize) -> Vec<Line<'static>> {
    if p.mono || p.light == ui::c::terminal_is_light() {
        return lines;
    }
    lines
        .into_iter()
        .map(|l| {
            let used = ui::line_width(&l);
            // the swatch's own ink where a span has none (the terminal's
            // would be the other family's)
            let ink = l.style.fg.unwrap_or(p.text);
            let mut spans: Vec<Span<'static>> = l
                .spans
                .into_iter()
                .map(|s| {
                    let mut st = s.style;
                    if st.bg.is_none() {
                        st = st.bg(p.base);
                    }
                    if st.fg.is_none() {
                        st = st.fg(ink);
                    }
                    Span::styled(s.content, st)
                })
                .collect();
            spans.push(Span::styled(
                " ".repeat(w.saturating_sub(used)),
                Style::new().fg(p.text).bg(p.base),
            ));
            Line::from(spans)
        })
        .collect()
}

/// A few logos at the chosen style and size, side by side.
fn gallery(setting: &str, size: &str, w: usize, mono: bool) -> Vec<Line<'static>> {
    let (cols, rows) = logo::size_of(size);
    let d = logo::decide_for(setting, rows);
    let mut paint = Paint::here();
    if mono {
        paint.mono = true;
    }
    let items: [(Kind, &str); 5] = [
        (Kind::Agent, "claude"),
        (Kind::Agent, "codex"),
        (Kind::Os, "Ubuntu"),
        (Kind::Gpu, "nvidia"),
        (Kind::Server, "ollama"),
    ];
    let fit = ((w + 2) / (cols + 2)).clamp(1, items.len());
    let pics: Vec<Vec<Line<'static>>> = items
        .iter()
        .take(fit)
        .filter_map(|(k, id)| logo::lookup(*k, id))
        .map(|l| logo::render(l, d.tier, cols, rows, &paint))
        .collect();
    let mut out: Vec<Line<'static>> = Vec::new();
    for r in 0..rows {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for p in &pics {
            if let Some(l) = p.get(r) {
                spans.extend(l.spans.iter().cloned());
                let used = ui::line_width(l);
                spans.push(Span::raw(" ".repeat(cols.saturating_sub(used) + 2)));
            }
        }
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(
        ui::trunc(&format!("{} · {}", d.tier.as_str(), d.why), w),
        ui::faint(),
    )));
    out
}

pub(crate) fn aside(w: &Wiz, width: u16) -> Option<Aside> {
    let iw = (width as usize).saturating_sub(6).clamp(24, 64);
    let p = palette(&w.value("theme"), &w.value("accent"), w.look.mono);
    let scale = w.value("ui_scale");
    let mut lines = swatch(mock(&p, &scale, ui::spinner(w.t0), iw), &p, iw);
    lines.push(Line::raw(""));
    lines.extend(gallery(
        &w.value("ui.logos"),
        &w.value("ui.logo_size"),
        iw,
        w.look.mono,
    ));
    let what = match w.value("theme").as_str() {
        "light" => "Paper",
        "dark" => "Graphite",
        _ if p.light => "auto → Paper",
        _ => "auto → Graphite",
    };
    Some(Aside {
        title: format!(
            "Preview · {what}{}",
            if p.mono { " · NO_COLOR" } else { "" }
        ),
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_palette_follows_the_choices() {
        let p = palette("light", "claude", false);
        if std::env::var_os("NO_COLOR").is_none() {
            assert!(p.light);
            assert_eq!(p.accent, Color::Rgb(0xd9, 0x77, 0x57));
            assert_eq!(
                palette("dark", "gold", false).accent,
                Color::Rgb(0xe2, 0xb4, 0x51)
            );
            assert_eq!(
                palette("dark", "#89b4fa", false).accent,
                Color::Rgb(0x89, 0xb4, 0xfa)
            );
        }
        assert!(palette("dark", "gold", true).mono);
    }

    #[test]
    fn the_mock_fits_its_width_at_every_scale() {
        let p = palette("dark", "gold", false);
        for scale in ["compact", "comfortable", "large"] {
            for w in [24, 40, 64] {
                for l in mock(&p, scale, "ℵ", w) {
                    assert!(
                        ui::line_width(&l) <= w + 1,
                        "{scale} {w}: {}",
                        ui::line_width(&l)
                    );
                }
            }
        }
        assert!(mock(&p, "large", "ℵ", 40).len() > mock(&p, "compact", "ℵ", 40).len());
    }
}
