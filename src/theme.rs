//! Identity colours.
//!
//! * Machines get one of MACHINES (assigned once, saved as `host.<alias>.color`,
//!   so a machine keeps its colour everywhere: dashboard, wall tiles, launch
//!   card, herdr's sidebar and — optionally — herdr's own accent on that host).
//! * Sessions get a hue + glyph, picked to differ from the sessions already
//!   running, and reported to herdr as pane tokens (`hue`, `mark`) so herdr's
//!   sidebar can colour them too (`aleph theme herdr`).

use crate::config::Config;
use crate::ui::{self, c};
use ratatui::style::Color;

/// (name, dark-theme colour, light-theme colour)
pub const MACHINES: &[(&str, u32, u32)] = &[
    ("blue", 0x89b4fa, 0x1e66f5),
    ("mauve", 0xcba6f7, 0x8839ef),
    ("teal", 0x94e2d5, 0x179299),
    ("peach", 0xfab387, 0xfe640b),
    ("pink", 0xf5c2e7, 0xea76cb),
    ("green", 0xa6e3a1, 0x40a02b),
    // a lemon, so a yellow machine never reads as the gold accent
    ("yellow", 0xe3e57a, 0x7f7d00),
    ("sky", 0x89dceb, 0x04a5e5),
    ("lavender", 0xb4befe, 0x7287fd),
    ("flamingo", 0xf2cdcd, 0xdd7878),
];

/// Session hues: (name, glyph, dark colour, light colour). Glyph + colour
/// together keep sessions distinguishable even for colour-blind users.
pub const HUES: &[(&str, &str, u32, u32)] = &[
    ("ember", "●", 0xf38ba8, 0xd20f39),
    ("amber", "◆", 0xfab387, 0xfe640b),
    ("gold", "▲", 0xe3e57a, 0x7f7d00),
    ("moss", "■", 0xa6e3a1, 0x40a02b),
    ("tide", "★", 0x94e2d5, 0x179299),
    ("cobalt", "✚", 0x89b4fa, 0x1e66f5),
    ("plum", "♦", 0xcba6f7, 0x8839ef),
    ("rose", "▼", 0xf5c2e7, 0xea76cb),
    ("sky", "◆", 0x89dceb, 0x04a5e5),
    ("iris", "●", 0xb4befe, 0x7287fd),
    ("coral", "■", 0xeba0ac, 0xe64553),
    ("sand", "★", 0xf2cdcd, 0xdd7878),
];

fn rgb(x: u32) -> Color {
    Color::Rgb((x >> 16) as u8, (x >> 8 & 0xff) as u8, (x & 0xff) as u8)
}

fn pick(dark: u32, light: u32) -> Color {
    if c::mono() {
        return Color::Reset;
    }
    let p = c::pal();
    let x = if p.light { light } else { dark };
    // a theme on its own ground (cyberpunk's yellow, …) or a light one:
    // moved just enough to read on it as text does (4.5:1)
    if crate::look::get().ground || p.light {
        if let Color::Rgb(br, bg, bb) = p.base {
            let c = ((x >> 16) as u8, (x >> 8 & 0xff) as u8, (x & 0xff) as u8);
            let (r, g, b) = crate::logo::fit_to(c, (br, bg, bb), 4.5);
            return Color::Rgb(r, g, b);
        }
    }
    rgb(x)
}

/// A colour of your own (`host.<a>.color = #rrggbb`) on a theme's own ground:
/// moved just enough to read there.
fn on_ground(col: Color) -> Color {
    let p = c::pal();
    match (col, p.base, crate::look::get().ground || p.light) {
        (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb), true) => {
            let (r, g, b) = crate::logo::fit_to((r, g, b), (br, bg, bb), 4.5);
            Color::Rgb(r, g, b)
        }
        _ => col,
    }
}

/// Palette names → colour (`blue`, `ember`, …).
pub fn named(name: &str) -> Option<Color> {
    let n = name.to_lowercase();
    MACHINES
        .iter()
        .find(|m| m.0 == n)
        .map(|m| pick(m.1, m.2))
        .or_else(|| HUES.iter().find(|h| h.0 == n).map(|h| pick(h.2, h.3)))
}

/// Your own themes, from `theme.<name>.<field>` (`[themes.<name>]` in
/// aleph.toml), handed to the palette so `theme = <name>` finds them.
pub fn load_custom(cfg: &Config) {
    let mut by: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (k, v) in cfg.entries() {
        let Some((name, field)) = k.strip_prefix("theme.").and_then(|r| r.rsplit_once('.')) else {
            continue;
        };
        match by.iter_mut().find(|(n, _)| n == name) {
            Some((_, f)) => f.push((field.to_string(), v)),
            None => by.push((name.to_string(), vec![(field.to_string(), v)])),
        }
    }
    c::set_custom(
        by.into_iter()
            .map(|(n, f)| (n, c::Pal::custom(&f)))
            .collect(),
    );
}

/// A machine's colour (configured, or derived from its position).
pub fn machine(cfg: &Config, alias: &str) -> Color {
    if let Some(col) = cfg
        .host_get(alias, "color")
        .and_then(|s| ui::parse_color(&s))
    {
        return if c::mono() {
            Color::Reset
        } else {
            on_ground(col)
        };
    }
    let hosts = cfg.hosts();
    let i = hosts
        .iter()
        .position(|h| h == alias)
        .or_else(|| crate::sshcfg::is_local(alias).then_some(hosts.len()))
        .unwrap_or_else(|| (hash(alias) % MACHINES.len() as u64) as usize);
    let m = MACHINES[i % MACHINES.len()];
    pick(m.1, m.2)
}

/// Give every fleet machine without a colour the first unused palette name
/// and save it, so colours never shift when hosts are added or removed.
pub fn assign_machine_colors(cfg: &mut Config) {
    let hosts = cfg.fleet();
    let mut used: Vec<String> = hosts
        .iter()
        .filter_map(|h| cfg.host_get(h, "color"))
        .collect();
    for h in &hosts {
        if cfg.host_get(h, "color").is_some() {
            continue;
        }
        let Some(m) = MACHINES.iter().find(|m| !used.iter().any(|u| u == m.0)) else {
            break;
        };
        used.push(m.0.to_string());
        let _ = cfg.set(&format!("host.{h}.color"), m.0);
    }
}

/// Hex of a machine colour for the dark palette (used for herdr config, which
/// renders on dark themes by default).
pub fn machine_hex(cfg: &Config, alias: &str) -> String {
    let raw = cfg.host_get(alias, "color").unwrap_or_default();
    if raw.starts_with('#') {
        return raw;
    }
    let m = MACHINES
        .iter()
        .find(|m| m.0 == raw)
        .copied()
        .unwrap_or_else(|| {
            let i = cfg.hosts().iter().position(|h| h == alias).unwrap_or(0);
            MACHINES[i % MACHINES.len()]
        });
    format!("#{:06x}", m.1)
}

/// Colour of a session: its hue token, else its legacy glyph, else a hash.
pub fn session(hue: &str, mark: &str, key: &str) -> Color {
    if let Some(h) = HUES.iter().find(|h| h.0 == hue) {
        return pick(h.2, h.3);
    }
    let g = ui::glyph_color(mark);
    if g != c::MUTED() {
        return g;
    }
    let h = HUES[(hash(key) % HUES.len() as u64) as usize];
    pick(h.2, h.3)
}

/// Glyph for a session (token, else the hue's glyph).
pub fn glyph<'a>(hue: &str, mark: &'a str) -> &'a str {
    if !mark.is_empty() && mark != "·" {
        return mark;
    }
    HUES.iter().find(|h| h.0 == hue).map(|h| h.1).unwrap_or("●")
}

/// The least-used hue among `in_use` (ties broken by `seed`).
pub fn pick_hue(in_use: &[String], seed: u64) -> (&'static str, &'static str) {
    let count = |n: &str| in_use.iter().filter(|u| u.as_str() == n).count();
    let min = HUES.iter().map(|h| count(h.0)).min().unwrap_or(0);
    let free: Vec<&(&str, &str, u32, u32)> = HUES.iter().filter(|h| count(h.0) == min).collect();
    let h = free[(seed % free.len() as u64) as usize];
    (h.0, h.1)
}

pub fn hash(s: &str) -> u64 {
    // FNV-1a: stable across runs and platforms
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_themes_and_themes_on_their_own_ground_keep_every_ink_readable() {
        let rgb = |c: ratatui::style::Color| match c {
            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("a theme's colours are rgb"),
        };
        for (n, _, _) in crate::ui::c::THEMES.iter().filter(|t| t.0 != "auto") {
            let p = crate::ui::c::pal_of(n);
            if !(p.light || crate::look::of(n).ground) {
                continue;
            }
            // a light theme's inks are held on its cards (a shade under its
            // paper: the darkest surface text sits on), and higher
            let bg = rgb(if p.light { p.panel } else { p.base });
            let on = |c| crate::logo::contrast(bg, rgb(c));
            let (tx, mu, fa, ink, li) = if p.light {
                (11.9, 5.9, 4.4, 4.4, 2.9)
            } else {
                (11.0, 6.0, 4.0, 4.5, 1.5)
            };
            assert!(on(p.text) >= tx, "{n}: text {:.1}", on(p.text));
            assert!(on(p.muted) >= mu, "{n}: muted {:.1}", on(p.muted));
            assert!(on(p.faint) >= fa, "{n}: faint {:.1}", on(p.faint));
            assert!(on(p.line) >= li, "{n}: lines {:.2}", on(p.line));
            for (what, col) in [
                ("accent", p.accent),
                ("edge", p.edge),
                ("working", p.working),
                ("needs", p.needs),
                ("done", p.done),
                ("idle", p.idle),
                ("limit", p.limit),
                ("down", p.down),
                ("blue", p.blue),
                ("green", p.green),
                ("red", p.red),
                ("peach", p.peach),
                ("mauve", p.mauve),
                ("teal", p.teal),
                ("sky", p.sky),
            ] {
                assert!(on(col) >= ink, "{n}: {what} {:.1}", on(col));
            }
            // a light ground is paper: near white in the theme's hue (L*
            // 94–98.5), never mid-tone, never a bare white glare (Paper
            // itself is the terminal's own)
            if p.light && crate::look::of(n).ground {
                // its relative luminance, from its contrast with black
                let l = crate::logo::contrast(rgb(p.base), (0, 0, 0)) * 0.05 - 0.05;
                assert!((0.85..0.97).contains(&l), "{n}: its ground ({l:.2})");
            }
        }
    }

    #[test]
    fn every_theme_is_readable_and_yours_build_from_toml() {
        let rgb = |c: ratatui::style::Color| match c {
            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("a theme's colours are rgb"),
        };
        for (n, _, light) in crate::ui::c::THEMES.iter().filter(|t| t.0 != "auto") {
            assert!(crate::ui::c::Pal::named(n).is_some(), "{n}");
            // as it's used: made readable on its ground where it paints one
            let p = crate::ui::c::pal_of(n);
            assert_eq!(p.light, *light, "{n}");
            let (bg, text) = (rgb(p.base), rgb(p.text));
            assert!(
                crate::logo::contrast(bg, text) >= 7.0,
                "{n}: text on its base"
            );
            for (what, col) in [
                ("accent", p.accent),
                ("working", p.blue),
                ("done", p.green),
                ("down", p.red),
                ("needs", p.peach),
                ("at limit", p.mauve),
            ] {
                assert!(
                    crate::logo::contrast(bg, rgb(col)) >= 2.5,
                    "{n}: {what} on its base"
                );
            }
            if !*light {
                continue;
            }
            // a light theme: paper near white, cards a shade under it,
            // popups lighter, a selection that shows; every ink held to its
            // floor on the cards (the darkest surface text sits on)
            let white = (255, 255, 255);
            let (panel, pop, sel) = (rgb(p.panel), rgb(p.pop), rgb(p.sel));
            assert!(
                crate::logo::contrast(bg, white) < 1.2,
                "{n}: paper, not mid-tone"
            );
            assert!(
                crate::logo::contrast(pop, white) < crate::logo::contrast(bg, white),
                "{n}: popups lighter than the paper"
            );
            assert!(crate::logo::contrast(panel, bg) > 1.04, "{n}: cards show");
            assert!(
                crate::logo::contrast(sel, panel) > 1.08,
                "{n}: the selection shows"
            );
            let on = |c: ratatui::style::Color| crate::logo::contrast(panel, rgb(c));
            assert!(on(p.text) >= 11.9, "{n}: text {}", on(p.text));
            assert!(
                crate::logo::contrast(sel, rgb(p.text)) >= 6.9,
                "{n}: text on the selection"
            );
            assert!(
                on(p.muted) >= 5.9 && on(p.faint) >= 4.4,
                "{n}: muted, faint"
            );
            assert!(on(p.line) >= 2.9, "{n}: frame lines {}", on(p.line));
            for (what, col) in [
                ("frame", p.edge),
                ("working", p.working),
                ("needs", p.needs),
                ("done", p.done),
                ("idle", p.idle),
                ("limit", p.limit),
                ("down", p.down),
                ("yellow", p.yellow),
                ("sky", p.sky),
            ] {
                assert!(on(col) >= 4.4, "{n}: {what} on its cards {}", on(col));
            }
        }
        let cfg = Config::in_memory(&["theme.mine.base=nord", "theme.mine.accent=#ff0000"]);
        load_custom(&cfg);
        assert_eq!(crate::ui::c::custom_names(), ["mine"]);
        let p = crate::ui::c::pal_of("mine");
        // your red, made just light enough to read on Nord's ground
        let (r, g, b) = rgb(p.accent);
        assert!(r > 200 && g < r && b < r, "{:?}", p.accent);
        assert!(crate::logo::contrast(rgb(p.base), (r, g, b)) >= 4.5);
        assert_eq!(p.blue, crate::ui::c::Pal::named("nord").unwrap().blue);
        crate::ui::c::set_custom(Vec::new());
        // a custom theme's keys are [themes.<name>] in aleph.toml
        assert_eq!(
            crate::tomlcfg::to_path("theme.mine.accent"),
            ["themes", "mine", "accent"]
        );
        assert_eq!(crate::tomlcfg::to_path("theme"), ["ui", "theme"]);
        // one of yours, once its table is read
        load_custom(&cfg);
        assert!(crate::config::check("theme", "mine").is_ok());
        assert!(crate::config::check("theme", "nord").is_ok());
        assert!(crate::config::check("theme", "purple").is_err());
        crate::ui::c::set_custom(Vec::new());
    }

    #[test]
    fn hues_spread_and_named_colours() {
        let mut used: Vec<String> = Vec::new();
        for i in 0..HUES.len() {
            let (n, _) = pick_hue(&used, i as u64 * 7919);
            assert!(
                !used.contains(&n.to_string()),
                "{n} reused before all hues were taken"
            );
            used.push(n.into());
        }
        let (n, _) = pick_hue(&used, 3);
        assert!(HUES.iter().any(|h| h.0 == n));
        assert!(named("blue").is_some() && named("ember").is_some() && named("nope").is_none());
        assert_eq!(hash("b31"), hash("b31"));
    }
}
