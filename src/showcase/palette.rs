//! The cockpit palette (`showcase.palette` = cockpit, the default): while
//! showcase mode is on, the dashboard's greys turn deep-space — midnight
//! lines, faint text and selection, starlight text — and its free hues
//! nebula ones (teal, rose, starlight blue). Aleph's gold stays, and so does
//! what every state colour means: working blue, needs-you orange, done
//! green, down red, limit violet. A light terminal gets the same inks on
//! paper; NO_COLOR stays plain. The terminal's own background is never
//! painted (`base` is only ever a pill's text).
//!
//! Only the dashboard asks for it ([`repaint`], every frame), so printed
//! output keeps the theme as it is; `ui::c::init` calls [`tint`] each time
//! it builds a palette, so a theme or accent changed in Settings keeps it.

use crate::config::Config;
use crate::ui::c::Pal;
use ratatui::style::Color;
use std::sync::atomic::{AtomicBool, Ordering};

pub const SETTING: &str = "showcase.palette";

/// Has the dashboard asked for the cockpit's colours?
static WANT: AtomicBool = AtomicBool::new(false);

const fn rgb(x: u32) -> Color {
    Color::Rgb((x >> 16) as u8, (x >> 8 & 0xff) as u8, (x & 0xff) as u8)
}

/// Does `showcase.palette` ask for the cockpit (anything but `normal`)?
pub fn wants(cfg: &Config) -> bool {
    !matches!(
        cfg.get_or(SETTING).trim().to_ascii_lowercase().as_str(),
        "normal" | "off" | "theme"
    )
}

/// `ui::c::init`'s palette, as it's built: the cockpit's while wanted.
pub fn tint(p: &mut Pal) {
    if WANT.load(Ordering::Relaxed) && !p.mono {
        *p = cockpit(p);
    }
}

/// Every frame: showcase mode or the setting changed → the palette is built
/// again (with the theme and accent it was last given).
pub fn repaint(cfg: &Config) {
    let want = super::on() && wants(cfg);
    if WANT.swap(want, Ordering::Relaxed) != want {
        crate::ui::c::again();
    }
}

/// Tests that draw pictures: on or off, at once.
#[cfg(test)]
pub fn force(on: bool) {
    WANT.store(on, Ordering::Relaxed);
    crate::ui::c::again();
}

/// The cockpit's colours over `p` (pure): its accent, state colours and
/// light/dark kept, the rest deep space.
pub fn cockpit(p: &Pal) -> Pal {
    if p.mono {
        return p.clone();
    }
    let mut q = p.clone();
    if p.light {
        // midnight inks on paper
        q.teal = rgb(0x087a80);
        q.pink = rgb(0xc2366f);
        q.sky = rgb(0x1764a6);
        q.muted = rgb(0x4c5776);
        q.faint = rgb(0x7d88a6);
        q.line = rgb(0xc8d0e4);
        q.sel = rgb(0xe2e7f3);
        q.text = rgb(0x131a31);
        q.base = rgb(0xf7f8fd);
    } else {
        // deep space: midnight lines and selection, starlight text, nebula hues
        q.teal = rgb(0x2cc3b3);
        q.pink = rgb(0xf0709d);
        q.sky = rgb(0x8bd3ff);
        q.muted = rgb(0x9dabd2);
        q.faint = rgb(0x6271a3);
        q.line = rgb(0x2c3a63);
        q.sel = rgb(0x18234a);
        q.text = rgb(0xe8edfa);
        q.base = rgb(0x0a0e1c);
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blue_of(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("{c:?}"),
        }
    }

    #[test]
    fn the_cockpit_keeps_gold_and_what_states_mean() {
        for base in [Pal::dark(), Pal::light()] {
            let q = cockpit(&base);
            assert_eq!(q.accent, base.accent, "the gold stays");
            assert_eq!(q.light, base.light);
            // working blue, needs-you orange, done green, down red, limit violet
            for (a, b) in [
                (q.blue, base.blue),
                (q.peach, base.peach),
                (q.green, base.green),
                (q.red, base.red),
                (q.mauve, base.mauve),
                (q.yellow, base.yellow),
            ] {
                assert_eq!(a, b);
            }
            // the neutrals and free hues are the cockpit's own
            assert_ne!(q.line, base.line);
            assert_ne!(q.sel, base.sel);
            assert_ne!(q.faint, base.faint);
            assert_ne!(q.pink, base.pink);
            // midnight: lines and selection lean blue
            for c in [q.line, q.sel, q.faint, q.muted] {
                let (r, _, b) = blue_of(c);
                assert!(b > r, "{c:?}");
            }
        }
        // dark lines stay dark, paper's light
        let (r, g, b) = blue_of(cockpit(&Pal::dark()).line);
        assert!((r as u32 + g as u32 + b as u32) < 3 * 90);
        let (r, g, b) = blue_of(cockpit(&Pal::light()).line);
        assert!((r as u32 + g as u32 + b as u32) > 3 * 180);
        // NO_COLOR stays plain
        let m = cockpit(&Pal::mono());
        assert!(m.mono && m.line == Color::Reset && m.text == Color::Reset);
    }

    #[test]
    fn the_setting_says_which() {
        // the theme shows unless you ask for the cockpit
        assert!(!wants(&Config::in_memory(&[])));
        assert!(!wants(&Config::in_memory(&["showcase.palette=theme"])));
        assert!(wants(&Config::in_memory(&["showcase.palette=cockpit"])));
        assert!(!wants(&Config::in_memory(&["showcase.palette=normal"])));
        assert!(!wants(&Config::in_memory(&["showcase.palette=Normal "])));
    }

    #[test]
    fn tinting_waits_for_the_dashboard() {
        // (WANT is process-wide and only `repaint` sets it — never in tests)
        let mut p = Pal::dark();
        tint(&mut p);
        assert_eq!(p.line, Pal::dark().line);
    }
}
