//! The theme gallery: every theme (built in, and yours from `[themes.<name>]`)
//! beside the accents — once each: space turns them all to their light (or
//! dark) version where they have one. Moving the selection repaints the whole
//! dashboard behind it at once; enter keeps the theme and the accent, esc
//! puts back what you had. From the palette, and enter on Settings' theme or
//! accent.

use super::onboard::look::{accent_color, ACCENTS};
use super::splash::Kind;
use super::{Dash, Level, Mode, Pending};
use crate::ui::{self, c};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub(crate) struct Gallery {
    /// (name, title, light): each theme once, in the version shown
    themes: Vec<(String, String, bool)>,
    /// each one's other version (light for a dark one, and back)
    alt: Vec<Option<String>>,
    /// the light versions are shown
    light: bool,
    /// (value, title)
    accents: Vec<(String, String)>,
    ti: usize,
    ai: usize,
    /// 0: the themes, 1: the accents
    col: usize,
    /// what you had: (theme, accent)
    before: (String, String),
    /// `ui.splash` as it is
    splash: String,
}

/// A built-in theme's light version (Graphite's is Paper, Catppuccin's
/// Latte, the rest `<name>-light`).
fn light_of(name: &str) -> Option<String> {
    let l = match name {
        "dark" => "light".to_string(),
        "catppuccin" => "catppuccin-latte".to_string(),
        n => format!("{n}-light"),
    };
    c::THEMES.iter().any(|(n, _, _)| *n == l).then_some(l)
}

fn title_of(name: &str) -> String {
    c::THEMES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, t, _)| t.to_string())
        .unwrap_or_else(|| format!("{name} (yours)"))
}

pub(crate) fn open(d: &mut Dash) {
    // each theme once: a light version is its dark one's other side
    let lights: Vec<String> = c::THEMES
        .iter()
        .filter_map(|(n, _, _)| light_of(n))
        .collect();
    let mut themes: Vec<(String, String, bool)> = Vec::new();
    let mut alt: Vec<Option<String>> = Vec::new();
    for (n, t, l) in c::THEMES {
        if lights.iter().any(|x| x == n) {
            continue;
        }
        themes.push((n.to_string(), t.to_string(), *l));
        alt.push(light_of(n));
    }
    for n in c::custom_names() {
        let light = c::pal_of(&n).light;
        themes.push((n.clone(), format!("{n} (yours)"), light));
        alt.push(None);
    }
    let mut accents: Vec<(String, String)> = ACCENTS
        .iter()
        .map(|(v, t)| (v.to_string(), t.to_string()))
        .collect();
    let (theme, accent) = (d.cfg.get_or("theme"), d.cfg.get_or("accent"));
    if !accent.is_empty() && !accents.iter().any(|(v, _)| *v == accent) {
        accents.push((accent.clone(), format!("{accent} (yours)")));
    }
    // the theme in use: its own entry, or its dark one's turned light
    let mut light = false;
    let ti = match themes.iter().position(|(n, _, _)| *n == theme) {
        Some(i) => i,
        None => match alt
            .iter()
            .position(|a| a.as_deref() == Some(theme.as_str()))
        {
            Some(i) => {
                light = true;
                i
            }
            None => 0,
        },
    };
    let ai = accents
        .iter()
        .position(|(v, _)| *v == accent || (accent.is_empty() && v == "gold"))
        .unwrap_or(0);
    let mut g = Gallery {
        themes,
        alt,
        light: false,
        accents,
        ti,
        ai,
        col: 0,
        before: (theme, accent),
        splash: d.cfg.get_or("ui.splash"),
    };
    if light {
        flip(&mut g);
    }
    d.mode = Mode::Gallery(g);
}

/// Every theme with another version shows that one (light ⇄ dark).
fn flip(g: &mut Gallery) {
    g.light = !g.light;
    for (t, a) in g.themes.iter_mut().zip(g.alt.iter_mut()) {
        if let Some(other) = a.take() {
            if t.2 != g.light {
                let was = std::mem::replace(&mut t.0, other);
                t.1 = title_of(&t.0);
                t.2 = g.light;
                *a = Some(was);
            } else {
                *a = Some(other);
            }
        }
    }
}

/// Paint with (theme, accent) now (never in tests: the palette is the
/// whole process's).
fn paint(theme: &str, accent: &str) {
    if cfg!(test) {
        return;
    }
    ui::c::init(theme, None);
    if let Some(a) = accent_color(accent) {
        ui::c::init(theme, Some(a));
    }
}

fn chosen(g: &Gallery) -> (String, String) {
    (
        g.themes.get(g.ti).map(|t| t.0.clone()).unwrap_or_default(),
        g.accents.get(g.ai).map(|a| a.0.clone()).unwrap_or_default(),
    )
}

pub(crate) fn key(d: &mut Dash, mut g: Gallery, k: KeyEvent) {
    let (nt, na) = (g.themes.len().max(1), g.accents.len().max(1));
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            paint(&g.before.0, &g.before.1);
            return;
        }
        KeyCode::Enter => {
            let (theme, accent) = chosen(&g);
            if theme != g.before.0 {
                super::settings::save(d, "theme", &theme);
            }
            if accent != g.before.1 {
                super::settings::save(d, "accent", &accent);
            }
            let tt = g.themes.get(g.ti).map(|t| t.1.clone()).unwrap_or_default();
            d.toast(Level::Ok, format!("theme {tt} · accent {accent}"));
            // showcase's cockpit colours would hide the theme you just kept
            #[cfg(feature = "showcase")]
            if crate::showcase::on()
                && !matches!(
                    d.cfg.get_or("showcase.palette").trim(),
                    "theme" | "normal" | "off"
                )
            {
                super::settings::save(d, "showcase.palette", "theme");
                d.toast(
                    Level::Info,
                    "showcase shows your theme now (showcase.palette = theme; cockpit puts deep space back)",
                );
            }
            // its own splash too? (only asked when a splash of your own is set)
            let own = Kind::of_theme(&theme);
            let now = d.cfg.get_or("ui.splash");
            if theme != g.before.0 && now != "theme" && Kind::named(&now) != Some(own) {
                d.mode = Mode::Confirm {
                    title: "Its splash too?".into(),
                    text: format!(
                        "{tt} has a splash of its own: {}. Play it as the dashboard starts? Yes sets ui.splash = theme, so the splash follows the theme from now on; no keeps {now}.",
                        own.name()
                    ),
                    on_yes: Pending::Set(
                        "ui.splash".into(),
                        "theme".into(),
                        format!("splash: the theme's own ({})", own.name()),
                    ),
                };
            }
            return;
        }
        // light ⇄ dark, for every theme that has both
        KeyCode::Char(' ') | KeyCode::Char('t') => flip(&mut g),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => g.col = 0,
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => g.col = 1,
        KeyCode::Down | KeyCode::Char('j') if g.col == 0 => g.ti = (g.ti + 1) % nt,
        KeyCode::Up | KeyCode::Char('k') if g.col == 0 => g.ti = (g.ti + nt - 1) % nt,
        KeyCode::Down | KeyCode::Char('j') => g.ai = (g.ai + 1) % na,
        KeyCode::Up | KeyCode::Char('k') => g.ai = (g.ai + na - 1) % na,
        _ => {}
    }
    let (theme, accent) = chosen(&g);
    paint(&theme, &accent);
    d.mode = Mode::Gallery(g);
}

/// A theme's colours as a row of blocks: accent, then the states.
fn swatch(theme: &str) -> Vec<Span<'static>> {
    let p = c::pal_of(theme);
    if c::mono() {
        return vec![Span::raw("      ")];
    }
    [p.accent, p.blue, p.peach, p.green, p.red, p.mauve, p.text]
        .into_iter()
        .map(|col| Span::styled("▆", Style::new().fg(col)))
        .collect()
}

pub(crate) fn draw(f: &mut Frame, area: Rect, g: &Gallery) {
    let w = area.width.saturating_sub(4).clamp(50, 84);
    let lw = (w as usize).saturating_sub(4) * 3 / 5;
    let rows = g.themes.len().max(g.accents.len());
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled(
            ui::pad("Themes", lw),
            if g.col == 0 { ui::bold() } else { ui::muted() },
        ),
        Span::styled("Accents", if g.col == 1 { ui::bold() } else { ui::muted() }),
    ])];
    for i in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        if let Some((n, t, light)) = g.themes.get(i) {
            let on = i == g.ti;
            spans.push(Span::styled(
                if on && g.col == 0 {
                    "┃ "
                } else if on {
                    "· "
                } else {
                    "  "
                },
                ui::fg(c::ACCENT()),
            ));
            spans.extend(swatch(n));
            // ☀ a light one; ◐ it has the other version too (space)
            let both = g.alt.get(i).is_some_and(Option::is_some);
            let t = format!(
                " {t}{}{}",
                if *light { " ☀" } else { "" },
                if both { " ◐" } else { "" }
            );
            spans.push(Span::styled(
                ui::pad(&t, lw.saturating_sub(9)),
                if on { ui::bold() } else { Style::new() },
            ));
        } else {
            spans.push(Span::raw(" ".repeat(lw)));
        }
        if let Some((v, t)) = g.accents.get(i) {
            let on = i == g.ai;
            let col = accent_color(v).unwrap_or_else(|| c::pal().accent_default());
            spans.push(Span::styled(
                if on && g.col == 1 {
                    "┃ "
                } else if on {
                    "· "
                } else {
                    "  "
                },
                ui::fg(c::ACCENT()),
            ));
            spans.push(Span::styled("● ", ui::fg(col)));
            spans.push(Span::styled(
                t.clone(),
                if on { ui::bold() } else { Style::new() },
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    // what the states look like in it (the screen behind shows the rest)
    lines.push(Line::from(vec![
        Span::styled(
            "ℵ accent  ",
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ),
        Span::styled("● working  ", ui::fg(c::WORKING())),
        Span::styled(
            format!("{} needs you  ", crate::look::needs()),
            ui::fg(c::NEEDS()),
        ),
        Span::styled("✓ done  ", ui::fg(c::DONE())),
        Span::styled("✗ down  ", ui::fg(c::DOWN())),
        Span::styled("◷ at limit", ui::fg(c::LIMIT())),
    ]));
    // the splash that goes with it
    let theme = g.themes.get(g.ti).map(|t| t.0.as_str()).unwrap_or("");
    let own = Kind::of_theme(theme);
    lines.push(Line::from(vec![
        Span::styled("its splash  ", ui::muted()),
        Span::styled(own.name(), ui::fg(c::ACCENT())),
        Span::styled(
            if g.splash == "theme" {
                "  · plays as the dashboard starts (ui.splash = theme)".to_string()
            } else {
                format!("  · yours is {}: enter asks", g.splash)
            },
            ui::faint(),
        ),
    ]));
    lines.push(Line::raw(""));
    lines.push(ui::hints(&[
        ("↑↓", "pick"),
        (
            "space",
            if g.light {
                "dark versions"
            } else {
                "light versions"
            },
        ),
        ("←→", "themes · accents"),
        ("enter", "keep"),
        ("esc", "put back"),
    ]));
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let r = ui::centered(area, w, h);
    crate::ui::clear(f, r);
    let block = ui::panel("Theme gallery", true).padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(r);
    f.render_widget(block, r);
    f.render_widget(Paragraph::new(lines), inner);
}
