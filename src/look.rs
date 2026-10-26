//! A theme's personality beyond its palette: how its panels are framed and
//! their titles set, its spinner and state glyphs, the living detail in the
//! dashboard's header, what fills empty space with `ui.ambience = rich`, a
//! glitch as views change, the splash it pairs with (`ui.splash = theme`),
//! and whether it paints its own ground under the whole screen.
//!
//! `ui::c::init` sets the look with the palette; `configure` applies the
//! settings that override a theme's own choices: `ui.ground` (theme or the
//! terminal's own background), `ui.panels` (its frames or another style),
//! `ui.ambience` (subtle, rich, off). `ui::finish` puts the ground and the
//! frames' last touches on every full-screen frame.

#[cfg(not(test))]
use std::sync::RwLock;

/// How panels are framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Rounded,
    Sharp,
    Heavy,
    Double,
    /// chamfered corners: ╱──╲
    Cut,
    /// only the corners, a HUD's targeting frame: ┌─  ─┐
    Brackets,
}

impl Frame {
    #[cfg(test)]
    pub const NAMES: [&'static str; 6] = ["rounded", "sharp", "heavy", "double", "cut", "brackets"];

    pub fn named(s: &str) -> Option<Frame> {
        Some(match s {
            "rounded" => Frame::Rounded,
            "sharp" => Frame::Sharp,
            "heavy" => Frame::Heavy,
            "double" => Frame::Double,
            "cut" => Frame::Cut,
            "brackets" => Frame::Brackets,
            _ => return None,
        })
    }
}

/// How a panel's title is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Titles {
    Plain,
    /// upper case
    Caps,
}

/// Which of the palette's colours a living detail takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Accent,
    Edge,
    Working,
    Sky,
    Mauve,
    Pink,
    Peach,
    Green,
    Text,
}

impl Ink {
    pub fn color(self) -> ratatui::style::Color {
        use crate::ui::c;
        match self {
            Ink::Accent => c::ACCENT(),
            Ink::Edge => c::EDGE(),
            Ink::Working => c::WORKING(),
            Ink::Sky => c::SKY(),
            Ink::Mauve => c::MAUVE(),
            Ink::Pink => c::PINK(),
            Ink::Peach => c::PEACH(),
            Ink::Green => c::GREEN(),
            Ink::Text => c::TEXT(),
        }
    }
}

/// The living detail in the dashboard's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ornament {
    None,
    /// a data readout ticking over: `▮ 4FA2·19C0` (cyberpunk)
    Hex,
    /// a radio telescope's trace scrolling, a peak every so many ms (pulsar)
    Trace(u128),
    /// frames in turn, each so many ms, in a colour: a star twinkling, a ring
    /// orbiting, code churning, a signal crossing a network, …
    Cycle(&'static [&'static str], u128, Ink),
}

/// How a theme's particles move through empty space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drift {
    /// falling streaks: rain, code, dust falling in
    Down,
    /// rising: embers, bubbles, mist
    Up,
    /// in place, twinkling: stars
    Still,
}

/// A theme's ambience: particles in the empty cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Particles {
    /// the head's glyph first, then the trail's (churned through)
    pub glyphs: &'static [&'static str],
    pub drift: Drift,
    /// rows a second (slowest, fastest)
    pub speed: (u64, u64),
    /// one column in this many carries one
    pub every: u64,
    /// its length in cells (shortest, longest)
    pub len: (u64, u64),
    /// its colours: most, and now and then
    pub inks: [Ink; 2],
}

/// What `ui.ambience = rich` puts in empty space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ambience {
    None,
    Particles(Particles),
}

/// How much of the ambience shows (`ui.ambience`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Off,
    /// the header's ornament
    Subtle,
    /// and the ambience in empty space
    Rich,
}

/// The glyphs a theme gives the session states (working is its spinner).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub needs: &'static str,
    pub idle: &'static str,
    pub done: &'static str,
}

/// One theme's personality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub frame: Frame,
    pub titles: Titles,
    /// the spinner: its frames and how long each shows (ms)
    pub spin: (&'static [&'static str], u128),
    pub glyphs: Glyphs,
    pub ornament: Ornament,
    pub ambience: Ambience,
    /// a short glitch as the view changes
    pub glitch: bool,
    /// the splash it plays with `ui.splash = theme`
    pub splash: &'static str,
    /// paints its own ground (the palette's `base`) under everything
    pub ground: bool,
    pub level: Level,
    /// panel titles as HUD tags: on a block of the frame's colour
    pub tags: bool,
    /// the header and footer rows on a band of the frame's colour
    pub band: bool,
    /// panels as cards filled with the ground's colour, the space between
    /// them (margins, the tab row, the strips) in the frame's colour
    pub cards: bool,
}

pub const SPIN_ALEPH: &[&str] = &["⋅", "∙", "∘", "ℵ", "ℵ", "∘", "∙", "⋅"];
/// a data glitch hopping between the quadrants
pub const SPIN_GLITCH: &[&str] = &["▖", "▘", "▝", "▗", "▚", "▞"];
/// halfwidth katakana: one column each
const KANA: &[&str] = &[
    "ｱ", "ｶ", "ｻ", "ﾀ", "ﾅ", "ﾊ", "ﾏ", "ﾔ", "ﾗ", "ﾜ", "ｦ", "ﾝ", "0", "1", "Z",
];

const DEFAULT_GLYPHS: Glyphs = Glyphs {
    needs: "▲",
    idle: "●",
    done: "●",
};

/// Aleph's own: Graphite and Paper, rounded panels, the ℵ spinner, the cockpit.
const BASE: Look = Look {
    frame: Frame::Rounded,
    titles: Titles::Plain,
    spin: (SPIN_ALEPH, 130),
    glyphs: DEFAULT_GLYPHS,
    ornament: Ornament::None,
    ambience: Ambience::None,
    glitch: false,
    splash: "cockpit",
    ground: true,
    level: Level::Subtle,
    tags: false,
    band: false,
    cards: false,
};

/// A neural interface's HUD: brackets for frames, titles in capitals, a
/// glitching spinner, the objective's diamond for what needs you, a data
/// readout in the header, a glitch between views. No rain: the HUD stays
/// clear (`ui.ambience = rich` adds nothing here).
const CYBERPUNK: Look = Look {
    frame: Frame::Brackets,
    titles: Titles::Caps,
    spin: (SPIN_GLITCH, 90),
    glyphs: Glyphs {
        needs: "◆",
        idle: "□",
        done: "■",
    },
    ornament: Ornament::Hex,
    ambience: Ambience::None,
    glitch: true,
    splash: "cyberdeck",
    tags: true,
    ..BASE
};

/// Cyberpunk in daylight: the dark one's brackets, red tags and yellow
/// selection on a concrete ground, no cards.
const CYBERPUNK_LIGHT: Look = CYBERPUNK;

/// A point becoming everything: soft rounded panels, a point swelling as it
/// works, a star for what needs you, a star twinkling in the header, a field
/// of stars.
const SINGULARITY: Look = Look {
    spin: (&["·", "∙", "•", "●", "•", "∙"], 140),
    glyphs: Glyphs {
        needs: "✦",
        idle: "○",
        done: "●",
    },
    ornament: Ornament::Cycle(&["✦", "✧", "·", "✧"], 380, Ink::Accent),
    ambience: Ambience::Particles(Particles {
        glyphs: &["·", "∙", "✧"],
        drift: Drift::Still,
        speed: (0, 0),
        every: 7,
        len: (1, 1),
        inks: [Ink::Mauve, Ink::Accent],
    }),
    splash: "singularity",
    ..BASE
};

/// An instrument: sharp panels, capitals, a beam sweeping round as it
/// works, a radio trace in the header beating at the pulsar's period, its
/// beams' glints in empty space.
const PULSAR: Look = Look {
    frame: Frame::Sharp,
    titles: Titles::Caps,
    spin: (&["◜", "◝", "◞", "◟"], 90),
    glyphs: Glyphs {
        needs: "◉",
        idle: "◌",
        done: "●",
    },
    ornament: Ornament::Trace(1337),
    ambience: Ambience::Particles(Particles {
        glyphs: &["·", "∙"],
        drift: Drift::Still,
        speed: (0, 0),
        every: 9,
        len: (1, 1),
        inks: [Ink::Sky, Ink::Pink],
    }),
    splash: "pulsar",
    ..BASE
};

/// Mass: heavy panels, a ring turning as it works, a ring orbiting in the
/// header, dust falling inward.
const EVENT_HORIZON: Look = Look {
    frame: Frame::Heavy,
    spin: (&["◐", "◓", "◑", "◒"], 160),
    glyphs: Glyphs {
        needs: "▲",
        idle: "◌",
        done: "◉",
    },
    ornament: Ornament::Cycle(&["○", "◎", "●", "◎"], 420, Ink::Accent),
    ambience: Ambience::Particles(Particles {
        glyphs: &["∙", "·"],
        drift: Drift::Down,
        speed: (2, 5),
        every: 6,
        len: (1, 2),
        inks: [Ink::Peach, Ink::Accent],
    }),
    splash: "event-horizon",
    ..BASE
};

/// A green screen: sharp panels, capitals, katakana churning as it works and
/// in the header, digital rain in empty space.
const MATRIX: Look = Look {
    frame: Frame::Sharp,
    titles: Titles::Caps,
    spin: (KANA, 90),
    glyphs: Glyphs {
        needs: "▲",
        idle: "·",
        done: "■",
    },
    ornament: Ornament::Cycle(KANA, 110, Ink::Accent),
    ambience: Ambience::Particles(Particles {
        glyphs: KANA,
        drift: Drift::Down,
        speed: (6, 16),
        every: 3,
        len: (3, 8),
        inks: [Ink::Accent, Ink::Green],
    }),
    splash: "matrix",
    ..BASE
};

/// A network: double-ruled panels, a signal circling as it works, a signal
/// crossing three nodes in the header, synapses firing in empty space.
const CONVERGENCE: Look = Look {
    frame: Frame::Double,
    spin: (&["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"], 80),
    glyphs: Glyphs {
        needs: "◈",
        idle: "○",
        done: "●",
    },
    ornament: Ornament::Cycle(&["●─○─○", "○─●─○", "○─○─●", "○─●─○"], 260, Ink::Edge),
    ambience: Ambience::Particles(Particles {
        glyphs: &["•", "·"],
        drift: Drift::Still,
        speed: (0, 0),
        every: 8,
        len: (1, 1),
        inks: [Ink::Sky, Ink::Pink],
    }),
    splash: "convergence",
    ..BASE
};

/// Frost: soft panels, a flake forming as it works and in the header, snow.
const NORD: Look = Look {
    spin: (&["◌", "○", "◎", "●", "◎", "○"], 130),
    ornament: Ornament::Cycle(&["·", "∗", "✳", "∗"], 420, Ink::Sky),
    ambience: Ambience::Particles(Particles {
        glyphs: &["∗", "·"],
        drift: Drift::Down,
        speed: (1, 3),
        every: 5,
        len: (1, 1),
        inks: [Ink::Text, Ink::Sky],
    }),
    ..BASE
};

/// Gothic: double-ruled panels, a clock hand turning, a moon waxing in the
/// header, mist rising.
const DRACULA: Look = Look {
    frame: Frame::Double,
    spin: (&["◴", "◷", "◶", "◵"], 120),
    glyphs: Glyphs {
        needs: "▲",
        idle: "○",
        done: "●",
    },
    ornament: Ornament::Cycle(&["○", "◐", "●", "◑"], 600, Ink::Mauve),
    ambience: Ambience::Particles(Particles {
        glyphs: &["·", "˙"],
        drift: Drift::Up,
        speed: (1, 3),
        every: 6,
        len: (1, 2),
        inks: [Ink::Mauve, Ink::Pink],
    }),
    ..BASE
};

/// Retro warmth: heavy panels, a bar meter as it works, a level meter in the
/// header, embers rising.
const GRUVBOX: Look = Look {
    frame: Frame::Heavy,
    spin: (&["▁", "▃", "▅", "▇", "▅", "▃"], 100),
    glyphs: Glyphs {
        needs: "▲",
        idle: "■",
        done: "■",
    },
    ornament: Ornament::Cycle(&["▁▃▅", "▃▅▇", "▅▇▅", "▇▅▃", "▅▃▁", "▃▁▃"], 160, Ink::Peach),
    ambience: Ambience::Particles(Particles {
        glyphs: &["·", "∙"],
        drift: Drift::Up,
        speed: (2, 6),
        every: 7,
        len: (1, 2),
        inks: [Ink::Peach, Ink::Accent],
    }),
    ..BASE
};

/// Neon nights: cut corners, a sign flickering, city rain in blue and magenta.
const TOKYO_NIGHT: Look = Look {
    frame: Frame::Cut,
    spin: (&["◇", "◈", "◆", "◈"], 120),
    glyphs: Glyphs {
        needs: "◆",
        idle: "◇",
        done: "●",
    },
    ornament: Ornament::Cycle(&["◆", "◆", "◇", "◆", "◆", "◆"], 140, Ink::Pink),
    ambience: Ambience::Particles(Particles {
        glyphs: &["╷", "│"],
        drift: Drift::Down,
        speed: (8, 18),
        every: 4,
        len: (2, 4),
        inks: [Ink::Working, Ink::Pink],
    }),
    ..BASE
};

/// Pastel calm: soft panels, an arc going round, a heart of a dot breathing,
/// bubbles rising.
const CATPPUCCIN: Look = Look {
    spin: (&["◜", "◠", "◝", "◞", "◡", "◟"], 110),
    glyphs: Glyphs {
        needs: "▲",
        idle: "○",
        done: "●",
    },
    ornament: Ornament::Cycle(&["·", "•", "●", "•"], 320, Ink::Pink),
    ambience: Ambience::Particles(Particles {
        glyphs: &["◦", "°"],
        drift: Drift::Up,
        speed: (1, 3),
        every: 7,
        len: (1, 1),
        inks: [Ink::Mauve, Ink::Pink],
    }),
    ..BASE
};

/// Precision: sharp panels, a square's quarter turning, nothing that
/// doesn't need to move.
const SOLARIZED: Look = Look {
    frame: Frame::Sharp,
    spin: (&["◰", "◳", "◲", "◱"], 140),
    glyphs: Glyphs {
        needs: "▲",
        idle: "□",
        done: "■",
    },
    ..BASE
};

/// Every theme's own look, for the tests.
#[cfg(test)]
pub const ALL: &[Look] = &[
    BASE,
    CYBERPUNK,
    SINGULARITY,
    PULSAR,
    EVENT_HORIZON,
    MATRIX,
    CONVERGENCE,
    NORD,
    DRACULA,
    GRUVBOX,
    TOKYO_NIGHT,
    CATPPUCCIN,
    SOLARIZED,
];

/// The glyph for a session that needs you (▲, cyberpunk's ◆, …).
pub fn needs() -> &'static str {
    get().glyphs.needs
}

/// A theme's own look (before the settings).
pub fn of(theme: &str) -> Look {
    if theme == "cyberpunk-light" {
        return CYBERPUNK_LIGHT;
    }
    match theme.trim_end_matches("-light") {
        // the terminal's own: Aleph adapts to it, paints nothing under it
        "auto" | "dark" | "light" | "graphite" | "paper" | "" => Look {
            ground: false,
            ..BASE
        },
        "cyberpunk" => CYBERPUNK,
        "singularity" => SINGULARITY,
        "pulsar" => PULSAR,
        "event-horizon" => EVENT_HORIZON,
        "matrix" | "matrix-mono" => MATRIX,
        "convergence" => CONVERGENCE,
        "nord" => NORD,
        "dracula" => DRACULA,
        "gruvbox" => GRUVBOX,
        "tokyo-night" => TOKYO_NIGHT,
        "catppuccin" | "catppuccin-latte" => CATPPUCCIN,
        "solarized" => SOLARIZED,
        _ => BASE,
    }
}

/// What the settings change: (`ui.ground` on, `ui.panels`, `ui.ambience`).
#[derive(Clone, Copy, Debug)]
struct Over {
    ground: bool,
    frame: Option<Frame>,
    level: Level,
}

const OVER0: Over = Over {
    ground: true,
    frame: None,
    level: Level::Subtle,
};

// one look for the process; each test thread its own (tests set themes side by side)
#[cfg(not(test))]
static THEME: RwLock<Option<Look>> = RwLock::new(None);
#[cfg(not(test))]
static OVER: RwLock<Over> = RwLock::new(OVER0);
#[cfg(test)]
thread_local! {
    static THEME_T: std::cell::Cell<Option<Look>> = const { std::cell::Cell::new(None) };
    static OVER_T: std::cell::Cell<Over> = const { std::cell::Cell::new(OVER0) };
}

fn theme_now() -> Option<Look> {
    #[cfg(not(test))]
    return THEME.read().ok().and_then(|g| *g);
    #[cfg(test)]
    THEME_T.with(|t| t.get())
}

fn set_theme(l: Look) {
    #[cfg(not(test))]
    if let Ok(mut g) = THEME.write() {
        *g = Some(l);
    }
    #[cfg(test)]
    THEME_T.with(|t| t.set(Some(l)));
}

fn over_now() -> Over {
    #[cfg(not(test))]
    return OVER.read().map(|g| *g).unwrap_or(OVER0);
    #[cfg(test)]
    OVER_T.with(|t| t.get())
}

fn set_over(o: Over) {
    #[cfg(not(test))]
    if let Ok(mut g) = OVER.write() {
        *g = o;
    }
    #[cfg(test)]
    OVER_T.with(|t| t.set(o));
}

/// The theme in use (from `ui::c::init`).
pub fn set(theme: &str) {
    set_theme(of(theme));
}

/// The settings that override a theme's look.
pub fn configure(cfg: &crate::config::Config) {
    let o = Over {
        ground: cfg.get_or("ui.ground") != "terminal",
        frame: Frame::named(&cfg.get_or("ui.panels")),
        level: match cfg.get_or("ui.ambience").as_str() {
            "off" => Level::Off,
            "rich" => Level::Rich,
            _ => Level::Subtle,
        },
    };
    set_over(o);
}

/// The look in use: the theme's, with the settings over it. Motion off
/// (`ui.motion`) or no colour: nothing moves.
pub fn get() -> Look {
    let mut l = theme_now().unwrap_or(Look {
        ground: false,
        ..BASE
    });
    {
        let o = over_now();
        l.ground &= o.ground;
        if let Some(f) = o.frame {
            l.frame = f;
        }
        l.level = o.level;
    }
    if crate::ui::c::mono() {
        l.ground = false;
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminals_own_themes_paint_nothing_and_cyberpunk_its_hud() {
        assert!(!of("auto").ground && !of("dark").ground && !of("light").ground);
        assert!(of("nord").ground);
        let c = of("cyberpunk");
        assert_eq!(
            (c.frame, c.titles, c.splash),
            (Frame::Brackets, Titles::Caps, "cyberdeck")
        );
        // no rain, by night or day: the same HUD
        assert_eq!(c.ambience, Ambience::None);
        assert_eq!(of("cyberpunk-light"), c);
        assert!(c.tags && !c.band);
        for n in Frame::NAMES {
            assert!(Frame::named(n).is_some(), "{n}");
        }
    }

    #[test]
    fn every_theme_looks_like_itself_and_every_glyph_is_one_column() {
        // themes that look alike: none of the cosmos or the classics share a
        // frame, spinner and ornament all three
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert!(
                    (a.frame, a.spin, a.ornament, a.band) != (b.frame, b.spin, b.ornament, b.band),
                    "{a:?} and {b:?} look the same"
                );
            }
        }
        let one = |g: &str| assert_eq!(crate::ui::width(g), 1, "{g:?}");
        for l in ALL {
            assert!(!l.spin.0.is_empty());
            l.spin.0.iter().for_each(|g| one(g));
            for g in [l.glyphs.needs, l.glyphs.idle, l.glyphs.done] {
                one(g);
            }
            if let Ornament::Cycle(frames, _, _) = l.ornament {
                let w = crate::ui::width(frames[0]);
                assert!(
                    frames.iter().all(|f| crate::ui::width(f) == w),
                    "{frames:?}"
                );
                assert!(frames.iter().all(|f| f.chars().count() == w), "{frames:?}");
            }
            if let Ambience::Particles(p) = l.ambience {
                p.glyphs.iter().for_each(|g| one(g));
                assert!(p.every >= 2 && p.len.0 >= 1 && p.len.0 <= p.len.1);
            }
        }
        // each named theme has its own
        for (t, want) in [
            ("singularity", "singularity"),
            ("pulsar-light", "pulsar"),
            ("matrix-mono", "matrix"),
            ("convergence", "convergence"),
            ("nord", "cockpit"),
        ] {
            assert_eq!(of(t).splash, want, "{t}");
        }
        assert_eq!(of("tokyo-night").frame, Frame::Cut);
        assert_eq!(of("gruvbox").frame, Frame::Heavy);
        assert_eq!(of("dracula").frame, Frame::Double);
    }
}
