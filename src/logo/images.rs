//! Logos as images: which protocol this terminal speaks, and the host's side
//! of it.
//!
//! * kitty's graphics protocol with Unicode placeholders (kitty, Ghostty;
//!   [`super::kitty`]): a picture shows wherever its placeholder cells are, so
//!   ratatui moves and overwrites it like text;
//! * iTerm2's inline images, OSC 1337 (iTerm2, WezTerm; [`super::iterm`]): a
//!   picture is written at a place after the frame and stays until its cells
//!   are written over.
//!
//! A host that shows them says so ([`set_output`]: `auto` picks images only
//! then) and, every frame, hands the frame's buffer to [`frame`] inside
//! `Terminal::draw`, writes [`take`] after it, and writes [`clear`] before
//! giving the terminal to anyone else. Images are off inside tmux, screen,
//! zellij and herdr (they'd need passthrough), under NO_COLOR, and in
//! terminals not known to show them; `ALEPH_IMAGES` = `0`, `1`, `kitty` or
//! `iterm2` overrides (mosh can't carry them: `aleph connect --mosh` sets 0
//! there).

use super::{iterm, kitty, Logo, Paint};
use ratatui::buffer::Buffer;
use ratatui::text::Line;
use std::sync::atomic::{AtomicBool, Ordering};

/// How pictures get to the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    /// kitty's graphics protocol, Unicode placeholders
    Kitty,
    /// iTerm2's inline images (OSC 1337)
    Iterm,
}

impl Proto {
    pub fn label(self) -> &'static str {
        match self {
            Proto::Kitty => "kitty graphics (Unicode placeholders)",
            Proto::Iterm => "iTerm2 inline images (OSC 1337)",
        }
    }
}

/// The protocol and the terminal's name, or why there are no images here.
pub type Support = Result<(Proto, &'static str), String>;

/// Does the host write [`take`] after each frame?
static OUTPUT: AtomicBool = AtomicBool::new(false);

/// The host hands each frame to [`frame`], writes [`take`] after it and
/// [`clear`] before handing the terminal over: `auto` may pick images.
pub fn set_output(on: bool) {
    OUTPUT.store(on, Ordering::Relaxed);
}

/// Can logos be images here? Ok(the protocol, the terminal) or Err(why not).
pub fn usable() -> Support {
    let s = support()?;
    if OUTPUT.load(Ordering::Relaxed) {
        Ok(s)
    } else {
        Err(format!(
            "{} shows images, but this screen doesn't draw them yet",
            s.1
        ))
    }
}

/// Does this terminal show images, and how? (Asked once: the answer is in
/// the environment we started with, no terminal query.)
pub fn support() -> Support {
    static V: std::sync::OnceLock<Support> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        support_in(&|k: &str| {
            if k == "ALEPH_IMAGES" {
                crate::brand::env("IMAGES")
            } else {
                std::env::var(k).ok()
            }
        })
    })
    .clone()
}

/// The protocol pictures are drawn with here (kitty's when there's none:
/// nothing draws them then).
pub fn proto() -> Proto {
    support().map_or(Proto::Kitty, |s| s.0)
}

/// Inside a multiplexer, which would have to pass pictures (and queries)
/// through? Its name.
pub fn multiplexer(env: &dyn Fn(&str) -> Option<String>) -> Option<&'static str> {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    let term = get("TERM").unwrap_or_default();
    if get("TMUX").is_some() || term.starts_with("tmux") {
        Some("tmux")
    } else if get("STY").is_some() || term.starts_with("screen") {
        Some("screen")
    } else if get("ZELLIJ").is_some() {
        Some("zellij")
    } else if get("HERDR_ENV").is_some() {
        Some("herdr")
    } else {
        None
    }
}

/// The decision table: the override, NO_COLOR, multiplexers, then the
/// terminal by what it says about itself (iTerm2's `LC_TERMINAL` comes
/// through ssh, which `TERM_PROGRAM` doesn't).
fn support_in(env: &dyn Fn(&str) -> Option<String>) -> Support {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    let term = get("TERM").unwrap_or_default();
    let program = get("TERM_PROGRAM").unwrap_or_default();
    let known = if get("KITTY_WINDOW_ID").is_some() || term == "xterm-kitty" {
        Some((Proto::Kitty, "kitty"))
    } else if program.eq_ignore_ascii_case("ghostty") || term == "xterm-ghostty" {
        Some((Proto::Kitty, "Ghostty"))
    } else if program == "iTerm.app" {
        Some((Proto::Iterm, "iTerm2"))
    } else if program == "WezTerm" {
        Some((Proto::Iterm, "WezTerm"))
    } else if program.is_empty() && get("LC_TERMINAL").as_deref() == Some("iTerm2") {
        Some((Proto::Iterm, "iTerm2"))
    } else {
        None
    };
    match get("ALEPH_IMAGES").map(|v| v.to_lowercase()).as_deref() {
        Some("0" | "off" | "no" | "false") => return Err("ALEPH_IMAGES=0".into()),
        Some("kitty") => return Ok((Proto::Kitty, "this terminal (ALEPH_IMAGES=kitty)")),
        Some("iterm" | "iterm2") => {
            return Ok((Proto::Iterm, "this terminal (ALEPH_IMAGES=iterm2)"))
        }
        Some(_) => {
            return Ok((
                known.map_or(Proto::Kitty, |k| k.0),
                "this terminal (ALEPH_IMAGES)",
            ))
        }
        None => {}
    }
    if get("NO_COLOR").is_some() {
        return Err("NO_COLOR is set".into());
    }
    if let Some(m) = multiplexer(env) {
        return Err(format!("inside {m} images need passthrough"));
    }
    if let Some(k) = known {
        return Ok(k);
    }
    Err(match program.as_str() {
        "WarpTerminal" => "Warp doesn't show inline images".into(),
        "Apple_Terminal" => "Terminal.app doesn't show images".into(),
        "" => "this terminal isn't known to show images".into(),
        p => format!("{p} isn't known to show images"),
    })
}

/// A logo as an image in `cols`×`rows` cells, in this terminal's protocol.
pub fn lines(logo: &Logo, cols: usize, rows: usize, paint: &Paint) -> Vec<Line<'static>> {
    match proto() {
        Proto::Kitty => kitty::lines(logo, cols, rows, paint),
        Proto::Iterm => iterm::lines(logo, cols, rows, paint),
    }
}

/// Inside `Terminal::draw`, after the views drew: the frame's pictures that
/// need it (iTerm2's) found and their cells kept from ratatui.
pub fn frame(buf: &mut Buffer) {
    iterm::frame(buf);
}

/// What to write to the terminal after a frame: the pictures it asked for
/// (empty when there are none).
pub fn take() -> String {
    kitty::take() + &iterm::take()
}

/// What to write before handing the terminal over or leaving: every picture
/// shown taken away (and forgotten, so it's sent again when it's drawn).
pub fn clear() -> String {
    kitty::clear() + &iterm::clear()
}

/// The terminal described in a short line: `iTerm2 · iTerm2 inline images
/// (OSC 1337)`, or why it shows none (`aleph icons`).
pub fn describe() -> String {
    match support() {
        Ok((p, t)) => format!("{t} · {}", p.label()),
        Err(e) => format!("none · {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let v: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| v.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn which_terminals_show_images_and_how() {
        let yes = |pairs: &[(&str, &str)]| support_in(&env(pairs));
        let no = |pairs: &[(&str, &str)]| support_in(&env(pairs)).unwrap_err();
        use Proto::{Iterm, Kitty};
        assert_eq!(yes(&[("KITTY_WINDOW_ID", "1")]), Ok((Kitty, "kitty")));
        assert_eq!(yes(&[("TERM", "xterm-kitty")]), Ok((Kitty, "kitty")));
        assert_eq!(yes(&[("TERM_PROGRAM", "ghostty")]), Ok((Kitty, "Ghostty")));
        assert_eq!(yes(&[("TERM", "xterm-ghostty")]), Ok((Kitty, "Ghostty")));
        // iTerm2 and WezTerm: OSC 1337
        assert_eq!(yes(&[("TERM_PROGRAM", "iTerm.app")]), Ok((Iterm, "iTerm2")));
        assert_eq!(yes(&[("TERM_PROGRAM", "WezTerm")]), Ok((Iterm, "WezTerm")));
        // over ssh only LC_* comes along
        assert_eq!(yes(&[("LC_TERMINAL", "iTerm2")]), Ok((Iterm, "iTerm2")));
        // … but a terminal that says what it is wins over one it was started from
        assert!(
            no(&[("TERM_PROGRAM", "WarpTerminal"), ("LC_TERMINAL", "iTerm2")]).contains("Warp")
        );
        assert_eq!(
            yes(&[("KITTY_WINDOW_ID", "3"), ("LC_TERMINAL", "iTerm2")]),
            Ok((Kitty, "kitty"))
        );
        // none of them
        assert!(no(&[("TERM_PROGRAM", "WarpTerminal")]).contains("Warp"));
        assert!(no(&[("TERM_PROGRAM", "Apple_Terminal")]).contains("Terminal.app"));
        assert!(no(&[("TERM_PROGRAM", "vscode")]).contains("vscode"));
        assert!(no(&[]).contains("isn't known"));
        // never through a multiplexer, never under NO_COLOR
        for m in [
            ("TMUX", "/tmp/tmux-1/default,1,0"),
            ("STY", "1.pts-0"),
            ("ZELLIJ", "0"),
            ("HERDR_ENV", "1"),
        ] {
            assert!(
                no(&[("TERM_PROGRAM", "iTerm.app"), m]).contains("passthrough"),
                "{m:?}"
            );
        }
        assert!(no(&[("KITTY_WINDOW_ID", "1"), ("TERM", "screen-256color")]).contains("screen"));
        assert!(no(&[("TERM_PROGRAM", "WezTerm"), ("NO_COLOR", "1")]).contains("NO_COLOR"));
        // the override, every way
        assert_eq!(
            yes(&[("ALEPH_IMAGES", "1")]).map(|s| s.0),
            Ok(Kitty),
            "unknown terminal: kitty's"
        );
        assert_eq!(
            yes(&[
                ("ALEPH_IMAGES", "1"),
                ("TERM_PROGRAM", "iTerm.app"),
                ("TMUX", "x")
            ])
            .map(|s| s.0),
            Ok(Iterm)
        );
        assert_eq!(yes(&[("ALEPH_IMAGES", "iterm2")]).map(|s| s.0), Ok(Iterm));
        assert_eq!(
            yes(&[("ALEPH_IMAGES", "kitty"), ("TERM_PROGRAM", "iTerm.app")]).map(|s| s.0),
            Ok(Kitty)
        );
        assert!(no(&[("ALEPH_IMAGES", "0"), ("KITTY_WINDOW_ID", "1")]).contains("ALEPH_IMAGES"));
        assert!(
            no(&[("ALEPH_IMAGES", "off"), ("TERM_PROGRAM", "iTerm.app")]).contains("ALEPH_IMAGES")
        );
        // empty values count as unset
        assert_eq!(
            yes(&[
                ("ALEPH_IMAGES", ""),
                ("TMUX", ""),
                ("TERM_PROGRAM", "iTerm.app")
            ]),
            Ok((Iterm, "iTerm2"))
        );
    }

    #[test]
    fn the_tier_per_terminal() {
        use crate::logo::Tier::{Glyphs, Images, Text, Thumbnails};
        let tier = |setting: &str, pairs: &[(&str, &str)]| {
            let s = support_in(&env(pairs));
            crate::logo::choose(setting, 4, s.map(|s| s.1), true).tier
        };
        let iterm = ("TERM_PROGRAM", "iTerm.app");
        assert_eq!(tier("auto", &[iterm]), Images);
        assert_eq!(tier("auto", &[("TERM_PROGRAM", "WezTerm")]), Images);
        assert_eq!(tier("auto", &[("LC_TERMINAL", "iTerm2")]), Images);
        assert_eq!(tier("auto", &[("KITTY_WINDOW_ID", "1")]), Images);
        // off in a multiplexer and under NO_COLOR: glyphs (thumbnails when asked for images)
        assert_eq!(tier("auto", &[iterm, ("TMUX", "x")]), Glyphs);
        assert_eq!(tier("auto", &[iterm, ("NO_COLOR", "1")]), Glyphs);
        assert_eq!(tier("images", &[iterm, ("ZELLIJ", "0")]), Thumbnails);
        // Warp stays on thumbnails
        assert_eq!(tier("auto", &[("TERM_PROGRAM", "WarpTerminal")]), Glyphs);
        assert_eq!(
            tier("images", &[("TERM_PROGRAM", "WarpTerminal")]),
            Thumbnails
        );
        // ui.logos forces the others
        assert_eq!(tier("thumbnails", &[iterm]), Thumbnails);
        assert_eq!(tier("text", &[("TERM_PROGRAM", "WezTerm")]), Text);
        let why = crate::logo::choose("auto", 4, support_in(&env(&[iterm])).map(|s| s.1), true).why;
        assert_eq!(why, "iTerm2 shows images");
    }

    #[test]
    fn multiplexers_by_name() {
        assert_eq!(multiplexer(&env(&[("TMUX", "x")])), Some("tmux"));
        assert_eq!(
            multiplexer(&env(&[("TERM", "tmux-256color")])),
            Some("tmux")
        );
        assert_eq!(multiplexer(&env(&[("TERM", "screen")])), Some("screen"));
        assert_eq!(multiplexer(&env(&[("ZELLIJ", "0")])), Some("zellij"));
        assert_eq!(multiplexer(&env(&[("HERDR_ENV", "1")])), Some("herdr"));
        assert_eq!(multiplexer(&env(&[("TERM", "xterm-256color")])), None);
    }
}
