//! Showcase mode's dress-up for the dashboard, each part switched by the
//! setting `showcase.flair` (a comma list; all on by default):
//!
//! * `header` — a gold shimmer sweeping across the `ℵ aleph` wordmark, a
//!   `✦ showcase` / `✦ demo` chip, and a tagline in the tab row's spare
//!   room (a new one every 8 s; the footer is the hints' and the toasts');
//! * `splash` — the dashboard's splash (`ui.splash`, `tui/splash.rs`) in
//!   showcase mode too, with the chip and a tagline (off: none in showcase);
//! * `keycast` — the keys just pressed as keycaps, bottom right, for 1.5 s:
//!   only keys the dashboard acted on in its normal mode, never text typed
//!   into a form, a filter, a message or a terminal;
//! * `spotlight` — a one-line caption naming the view just opened, and a
//!   faint starfield in empty states;
//! * `celebrate` — a supernova where a session finishes, a pulsar's flash
//!   where a question is answered
//!   (`celebrate.rs`);
//! * `hud` — the cockpit's corner brackets round the content, and live
//!   readouts at the right of the bottom status bar (`hud.rs`).
//!
//! Everything here is drawn into the frame before the lens runs, with the
//! foreground only (the terminal's own background stays), and costs a few
//! spans a frame.

use super::state::{self, Mode};
use crate::config::Config;
use crate::ui::{self, c};
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const TAGLINES: [&str; 6] = [
    "the point that contains all points",
    "Don't Panic",
    "mostly harmless",
    "Make it so",
    "never tell me the odds",
    "ℵ₀ sessions and counting",
];

const SETTING: &str = "showcase.flair";
/// how long the keys just pressed stay up (after the last one)
const KEYCAST_FOR: Duration = Duration::from_millis(1500);
/// how long a view's caption stays up
const SPOT_FOR: Duration = Duration::from_millis(1600);
/// a new tagline every 8 s
const TAGLINE_MS: u128 = 8000;
const STARS: [&str; 4] = ["✦", "·", "˚", "⋆"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Header,
    Splash,
    Keycast,
    Spotlight,
    // ---- delight
    Celebrate,
    Hud,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Part::Header => "header",
            Part::Splash => "splash",
            Part::Keycast => "keycast",
            Part::Spotlight => "spotlight",
            Part::Celebrate => "celebrate",
            Part::Hud => "hud",
        }
    }
}

/// Is this part of the dress-up switched on? (`all`, or a list; `off`: none)
pub fn wants(cfg: &Config, p: Part) -> bool {
    let v = cfg.get_or(SETTING);
    v.split(',')
        .map(str::trim)
        .any(|x| x.eq_ignore_ascii_case(p.name()) || x.eq_ignore_ascii_case("all"))
}

/// Milliseconds since the first flair was drawn (one clock for every part).
fn clock() -> u128 {
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis()
}

/// The tagline for this moment.
pub fn tagline(ms: u128) -> &'static str {
    TAGLINES[(ms / TAGLINE_MS) as usize % TAGLINES.len()]
}

// ------------------------------------------------------------------ header

/// ` ✦ showcase ` / ` ✦ demo `: gold on the selection grey, its star
/// twinkling now and then.
pub fn chip(mode: Mode, ms: u128) -> Span<'static> {
    let star = if (ms / 1300) % 4 == 3 { "✧" } else { "✦" };
    let what = if mode == Mode::Demo {
        "demo"
    } else {
        "showcase"
    };
    let st = if c::mono() {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new()
            .fg(c::ACCENT())
            .bg(c::SEL())
            .add_modifier(Modifier::BOLD)
    };
    Span::styled(format!(" {star} {what} "), st)
}

pub fn header(cfg: &Config, left: &mut Vec<Span<'static>>) {
    if !wants(cfg, Part::Header) || left.is_empty() {
        return;
    }
    let ms = clock();
    let word = left[0].content.to_string();
    let lit = ui::shimmer(&word, left[0].style, ms);
    let n = lit.len();
    left.splice(0..1, lit);
    left.insert(n, chip(state::mode(), ms));
    left.insert(n, Span::raw("  "));
}

// ------------------------------------------------------------------ keycast

thread_local! {
    /// (key as shown, what it did — `showcase.keycast = names` —, times in a row, when last)
    static KEYS: RefCell<VecDeque<(String, String, u32, Instant)>> = const { RefCell::new(VecDeque::new()) };
    /// (view, caption, since its first frame)
    static SPOT: RefCell<Option<(&'static str, &'static str, Option<Instant>)>> = const { RefCell::new(None) };
    /// capture: the keycast this frame, staged by `footer` for `lay_keys`
    /// (after the lens): (x, its row, the other row it may take, its
    /// keycaps, their width)
    static STAGED: RefCell<Option<Staged>> = const { RefCell::new(None) };
}

/// A key as a keycap shows it: `↵`, `↑`, `⌃K`, `⇧⇥`.
pub fn pretty(k: &str) -> String {
    let (mut pre, mut rest) = (String::new(), k);
    loop {
        if let Some(r) = rest.strip_prefix("ctrl+") {
            pre.push('⌃');
            rest = r;
        } else if let Some(r) = rest.strip_prefix("alt+") {
            pre.push('⌥');
            rest = r;
        } else {
            break;
        }
    }
    let base = match rest {
        "enter" => "↵".to_string(),
        "esc" => "esc".to_string(),
        "tab" => "⇥".to_string(),
        "shift+tab" => "⇧⇥".to_string(),
        "backspace" => "⌫".to_string(),
        "delete" => "⌦".to_string(),
        "space" => "␣".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "home" => "⇱".to_string(),
        "end" => "⇲".to_string(),
        "pageup" => "⇞".to_string(),
        "pagedown" => "⇟".to_string(),
        x if !pre.is_empty() && x.chars().count() == 1 => x.to_uppercase(),
        x => x.to_string(),
    };
    pre + &base
}

/// A key the dashboard acted on, and (asked only for names) what it does
/// there: (its key map action's name, the words).
pub fn key(cfg: &Config, k: &str, does: impl FnOnce() -> Option<(&'static str, String)>) {
    if k.is_empty() || !wants(cfg, Part::Keycast) {
        return;
    }
    // capture: the keycap says what the key did (`showcase.keycast`)
    let what = super::capture::names::of(cfg, does);
    record(&pretty(k), &what, Instant::now());
}

/// Nothing just pressed, no caption: the next frame is the screen alone
/// (a poster's).
pub fn hush() {
    KEYS.with(|q| q.borrow_mut().clear());
    SPOT.with(|s| *s.borrow_mut() = None);
}

/// The same key again counts up (`j ×3`); a pause starts a new run.
fn record(k: &str, what: &str, now: Instant) {
    KEYS.with(|q| {
        let mut q = q.borrow_mut();
        if q.back()
            .is_some_and(|(_, _, _, at)| now.duration_since(*at) > KEYCAST_FOR)
        {
            q.clear();
        }
        match q.back_mut() {
            Some((last, w, n, at)) if last == k && w == what => {
                *n += 1;
                *at = now;
            }
            _ => q.push_back((k.to_string(), what.to_string(), 1, now)),
        }
        // fewer when they're named (they're wider)
        let most = if what.is_empty() { 5 } else { 3 };
        while q.len() > most {
            q.pop_front();
        }
    });
}

/// The keys still up at `now`, as keycaps (gold while fresh, grey as they go).
fn keycast(now: Instant) -> Option<Vec<Span<'static>>> {
    KEYS.with(|q| {
        let q = q.borrow();
        let (_, _, _, last) = q.back()?;
        let age = now.duration_since(*last);
        if age > KEYCAST_FOR {
            return None;
        }
        let fresh = age < KEYCAST_FOR.mul_f32(0.7);
        let cap = if c::mono() {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else if fresh {
            Style::new()
                .fg(c::BASE())
                .bg(c::ACCENT())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(c::TEXT()).bg(c::LINE())
        };
        let named = if fresh {
            ui::fg(c::TEXT())
        } else {
            ui::muted()
        };
        let mut v = Vec::new();
        for (i, (k, what, n, _)) in q.iter().enumerate() {
            if i > 0 {
                v.push(Span::raw(if what.is_empty() { " " } else { "  " }));
            }
            v.push(Span::styled(format!(" {k} "), cap));
            if *n > 1 {
                v.push(Span::styled(format!("×{n}"), ui::muted()));
            }
            // what it did (`showcase.keycast = names`)
            if !what.is_empty() {
                v.push(Span::styled(format!(" {what}"), named));
            }
        }
        Some(v)
    })
}

fn width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| ui::width(&s.content)).sum::<usize>() as u16
}

pub fn footer(cfg: &Config, buf: &mut Buffer, r: Rect, used: u16) {
    STAGED.with(|s| *s.borrow_mut() = None);
    if r.width == 0 || r.height == 0 || !wants(cfg, Part::Keycast) {
        return;
    }
    let Some(keys) = keycast(Instant::now()) else {
        return;
    };
    let w = width(&keys);
    let x = r.right().saturating_sub(w + 1);
    // beside the hints when there's room, else just above them
    let y = if r.width >= used + w + 3 || r.y == buf.area.y {
        r.y
    } else {
        r.y - 1
    };
    // capture: staged, and laid in after the lens (`lay_keys`), so whole
    // names take the footer's spaces first and never flicker to `…`
    let other = if y == r.y {
        r.y.checked_sub(1).filter(|o| *o >= buf.area.y)
    } else {
        Some(r.y)
    };
    if x >= r.x {
        STAGED.with(|s| *s.borrow_mut() = Some((x, y, other, keys, w)));
    }
}

type Staged = (u16, u16, Option<u16>, Vec<Span<'static>>, u16);

/// After the lens: the keycast `footer` staged. Its row when every cell it
/// needs is still free there (blank, or a line: a panel's edge, the HUD's
/// bracket), else the other row when that one is, else its row all the same
/// — it never took a name's room (the lens has laid the names already), at
/// most it covers what's there, as it always did.
pub fn lay_keys(buf: &mut Buffer) {
    let Some((x, y, other, keys, w)) = STAGED.with(|s| s.borrow_mut().take()) else {
        return;
    };
    let free = |y: u16| {
        (x..x + w).all(|x| {
            buf.cell((x, y)).is_some_and(|c| {
                let line = c
                    .symbol()
                    .chars()
                    .next()
                    .is_some_and(|ch| ('\u{2500}'..='\u{257f}').contains(&ch));
                !c.skip && ((c.symbol() == " " && c.bg == Color::Reset) || line)
            })
        })
    };
    let at = if free(y) {
        y
    } else {
        other.filter(|o| free(*o)).unwrap_or(y)
    };
    buf.set_line(x, at, &Line::from(keys), w);
}

/// The tagline, centred in `r` (the tab row's gap) when it fits: a new one
/// every 8 s, coming in faint.
pub fn spare(cfg: &Config, buf: &mut Buffer, r: Rect) {
    if !wants(cfg, Part::Header) {
        return;
    }
    let ms = clock();
    let fresh = ms % TAGLINE_MS < 500;
    let spans = vec![
        Span::styled("✦ ", ui::fg(if fresh { c::FAINT() } else { c::ACCENT() })),
        Span::styled(
            tagline(ms),
            if fresh { ui::faint() } else { ui::muted() }.add_modifier(Modifier::ITALIC),
        ),
    ];
    let w = width(&spans);
    if r.width >= w + 6 {
        buf.set_line(r.x + (r.width - w) / 2, r.y, &Line::from(spans), w);
    }
}

// ------------------------------------------------------------------ spotlight

/// Each view's headline feature.
pub fn caption(view: &str) -> Option<(&'static str, &'static str)> {
    Some(match view {
        "Fleet" => ("Fleet", "every agent on every machine, live"),
        "Focus" => ("Focus", "one agent, full screen: reply, approve, queue"),
        "Tile" => ("Tile", "agents side by side, every one of them live"),
        "Monitor" => ("Monitor", "CPU, GPU, memory and spend across the fleet"),
        "Inbox" => ("Inbox", "what needs you, what finished, what changed"),
        "Settings" => ("Settings", "every setting, and a doctor that fixes things"),
        _ => return None,
    })
}

pub fn switched(view: &str) {
    let c = caption(view);
    SPOT.with(|s| *s.borrow_mut() = c.map(|(v, t)| (v, t, None)));
}

/// The caption still up at `now` (its clock starts on its first frame, so
/// one opened behind the splash still gets its turn), and how far along.
fn spotlight(now: Instant) -> Option<(&'static str, &'static str, f32)> {
    SPOT.with(|s| {
        let mut s = s.borrow_mut();
        let (v, t, at) = s.as_mut()?;
        let since = *at.get_or_insert(now);
        let age = now.duration_since(since);
        if age > SPOT_FOR {
            *s = None;
            return None;
        }
        Some((*v, *t, age.as_secs_f32() / SPOT_FOR.as_secs_f32()))
    })
}

pub fn overlay(cfg: &Config, buf: &mut Buffer, area: Rect, quiet: bool) {
    if quiet || area.height < 6 || !wants(cfg, Part::Spotlight) {
        return;
    }
    let Some((view, what, k)) = spotlight(Instant::now()) else {
        return;
    };
    // fading for its last quarter
    caption_line(buf, area, view, what, k > 0.75);
}

/// A caption just above the footer, centred — where a video puts its
/// caption: `✦ View  what it's for` (the spotlight's, and the autopilot
/// tour's), faint as it goes.
pub fn caption_line(buf: &mut Buffer, area: Rect, view: &str, what: &str, faint: bool) {
    if area.height < 3 {
        return;
    }
    let (vs, ts) = if faint {
        (ui::faint(), ui::faint())
    } else {
        (
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            Style::new().fg(c::TEXT()),
        )
    };
    let spans = vec![
        Span::raw(" "),
        Span::styled("✦ ", ui::fg(c::ACCENT())),
        Span::styled(view.to_string(), vs),
        Span::styled("  ", ts),
        Span::styled(what.to_string(), ts.add_modifier(Modifier::ITALIC)),
        Span::raw(" "),
    ];
    let w = width(&spans).min(area.width);
    let x = area.x + (area.width - w) / 2;
    let y = area.bottom() - 2;
    buf.set_line(x, y, &Line::from(spans), w);
}

/// A star's place in the field: a well-mixed hash of the cell.
fn hash(x: u16, y: u16) -> u64 {
    let mut z = ((x as u64) << 32 | y as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn starfield(cfg: &Config, buf: &mut Buffer, r: Rect) {
    if !wants(cfg, Part::Spotlight) {
        return;
    }
    stars(buf, r, clock());
}

/// Glyphs on the blank rows of `r` (a row with anything on it is left
/// alone), faint, one now and then twinkling gold.
pub fn stars(buf: &mut Buffer, r: Rect, ms: u128) {
    let r = r.intersection(buf.area);
    let tick = (ms / 600) as u64;
    for y in r.top()..r.bottom() {
        if (r.left()..r.right()).any(|x| buf[(x, y)].symbol() != " ") {
            continue;
        }
        for x in r.left()..r.right() {
            let h = hash(x, y);
            if !h.is_multiple_of(41) {
                continue;
            }
            let glyph = STARS[((h >> 8) % STARS.len() as u64) as usize];
            let twinkle = (tick + (h >> 20)).is_multiple_of(11);
            let col = if twinkle { c::ACCENT() } else { c::FAINT() };
            buf[(x, y)].set_symbol(glyph).set_fg(col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn cfg(flair: Option<&str>) -> Config {
        match flair {
            Some(v) => Config::in_memory(&[&format!("{SETTING}={v}")]),
            None => Config::in_memory(&[]),
        }
    }

    #[test]
    fn every_part_is_on_by_default_and_each_can_go() {
        let all = cfg(None);
        for p in [
            Part::Header,
            Part::Splash,
            Part::Keycast,
            Part::Spotlight,
            Part::Celebrate,
            Part::Hud,
        ] {
            assert!(wants(&all, p), "{p:?}");
        }
        let some = cfg(Some("header, keycast"));
        assert!(wants(&some, Part::Header) && wants(&some, Part::Keycast));
        assert!(!wants(&some, Part::Splash) && !wants(&some, Part::Spotlight));
        let none = cfg(Some("off"));
        assert!(!wants(&none, Part::Header) && !wants(&none, Part::Splash));
        assert!(wants(&cfg(Some("all")), Part::Spotlight));
        assert!(crate::showcase::own_key(SETTING) && !crate::showcase::own_key("theme"));
    }

    #[test]
    fn the_shimmer_moves_and_keeps_the_text() {
        let base = Style::new().fg(Color::Rgb(0xe2, 0xb4, 0x51));
        let at = |ms| ui::shimmer(" ℵ aleph", base, ms);
        let text: String = at(0).iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, " ℵ aleph");
        // resting: every character its own gold
        let rest = at(ui::SHIMMER_SWEEP_MS as u128 + 400);
        assert!(rest.iter().all(|s| s.style.fg == base.fg));
        // mid-sweep: some brighter, and not the same ones a moment later
        let a: Vec<_> = at(500).iter().map(|s| s.style.fg).collect();
        let b: Vec<_> = at(900).iter().map(|s| s.style.fg).collect();
        assert!(a.iter().any(|c| *c != base.fg));
        assert_ne!(a, b);
        // without truecolour it's left alone
        let plain = ui::shimmer("ℵ aleph", Style::new().fg(Color::Yellow), 500);
        assert_eq!(plain.len(), 1);
    }

    #[test]
    fn the_header_gets_a_chip_and_keeps_what_follows() {
        state::force(Some(state::State {
            mode: Mode::Demo,
            salt: vec![1; 32],
            epoch: 1,
            ..Default::default()
        }));
        let mut left = vec![
            Span::styled(" ℵ aleph", Style::new().fg(Color::Rgb(0xe2, 0xb4, 0x51))),
            Span::raw("   "),
            Span::raw("● "),
        ];
        header(&cfg(None), &mut left);
        let text: String = left.iter().map(|s| s.content.to_string()).collect();
        assert!(text.starts_with(" ℵ aleph  "), "{text}");
        assert!(text.contains(" demo ") && text.ends_with("   ● "), "{text}");
        let mut off = vec![Span::raw(" ℵ aleph"), Span::raw("   ")];
        header(&cfg(Some("splash")), &mut off);
        assert_eq!(off.len(), 2);
        state::force(None);
    }

    #[test]
    fn taglines_rotate() {
        assert_eq!(tagline(0), TAGLINES[0]);
        assert_eq!(tagline(TAGLINE_MS), TAGLINES[1]);
        assert_eq!(tagline(TAGLINE_MS * 6), TAGLINES[0]);
    }

    #[test]
    fn keycast_counts_repeats_and_lets_go() {
        let t = Instant::now();
        record("j", "", t);
        record("j", "", t + Duration::from_millis(100));
        record("↵", "", t + Duration::from_millis(200));
        let text = |v: Vec<Span>| v.iter().map(|s| s.content.to_string()).collect::<String>();
        let up = keycast(t + Duration::from_millis(300)).unwrap();
        assert_eq!(text(up), " j ×2  ↵ ");
        assert!(keycast(t + Duration::from_millis(1800)).is_none());
        // after a pause, a new run
        record("q", "", t + Duration::from_millis(2000));
        assert_eq!(
            text(keycast(t + Duration::from_millis(2100)).unwrap()),
            " q "
        );
        // with names: what each did, beside it (three at most)
        let t = t + Duration::from_secs(5);
        record("j", "next session", t);
        record("j", "next session", t + Duration::from_millis(50));
        record("a", "attach", t + Duration::from_millis(100));
        assert_eq!(
            text(keycast(t + Duration::from_millis(150)).unwrap()),
            " j ×2 next session   a  attach"
        );
        for (i, k) in ["1", "2", "3"].iter().enumerate() {
            record(k, "view", t + Duration::from_millis(200 + i as u64));
        }
        assert!(text(keycast(t + Duration::from_millis(250)).unwrap()).starts_with(" 1 "));
        hush();
        assert!(keycast(t + Duration::from_millis(250)).is_none());
        assert_eq!(pretty("ctrl+k"), "⌃K");
        assert_eq!(pretty("shift+tab"), "⇧⇥");
        assert_eq!(pretty("enter"), "↵");
        assert_eq!(pretty("G"), "G");
    }

    #[test]
    fn the_keycast_lands_after_the_lens_and_never_on_a_name() {
        let row = |b: &Buffer, y: u16| {
            (0..b.area.width)
                .map(|x| b[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        let foot = Rect::new(0, 2, 60, 1);
        record("j", "next session", Instant::now());
        let mut b = Buffer::empty(Rect::new(0, 0, 60, 3));
        b.set_string(0, 2, " j next", Style::new());
        footer(&cfg(None), &mut b, foot, 8);
        // staged during the draw: nothing in the frame the lens sees
        assert_eq!(row(&b, 2).trim(), "j next");
        lay_keys(&mut b);
        assert!(row(&b, 2).contains(" j  next session"), "{}", row(&b, 2));
        // a name the lens moved into that room keeps it: the keycast goes
        // just above
        let mut b = Buffer::empty(Rect::new(0, 0, 60, 3));
        footer(&cfg(None), &mut b, foot, 8);
        b.set_string(36, 2, "Zaphod Beeblebrox", Style::new());
        lay_keys(&mut b);
        assert!(row(&b, 2).contains("Zaphod Beeblebrox"), "{}", row(&b, 2));
        assert!(row(&b, 1).contains(" j  next session"), "{}", row(&b, 1));
        // a line (a panel's edge, the HUD's bracket) it may cover
        let mut b = Buffer::empty(Rect::new(0, 0, 60, 3));
        b.set_string(0, 2, "━".repeat(60), Style::new());
        footer(&cfg(None), &mut b, foot, 0);
        lay_keys(&mut b);
        assert!(row(&b, 2).contains("next session"));
        // laid once: the next frame stages its own
        let mut b = Buffer::empty(Rect::new(0, 0, 60, 3));
        lay_keys(&mut b);
        assert_eq!(row(&b, 2).trim(), "");
    }

    #[test]
    fn the_caption_waits_for_its_first_frame() {
        switched("Inbox");
        let t = Instant::now() + Duration::from_secs(5);
        let (v, what, k) = spotlight(t).unwrap();
        assert_eq!(v, "Inbox");
        assert!(what.contains("needs you") && k < 0.01);
        assert!(spotlight(t + Duration::from_millis(1000)).is_some());
        assert!(spotlight(t + Duration::from_millis(1700)).is_none());
        switched("Nowhere");
        assert!(spotlight(t).is_none());
        for v in ["Fleet", "Focus", "Tile", "Monitor", "Inbox", "Settings"] {
            assert!(caption(v).is_some(), "{v}");
        }
    }

    #[test]
    fn stars_fill_blank_rows_only() {
        let mut t = Terminal::new(TestBackend::new(80, 12)).unwrap();
        t.draw(|f| {
            f.render_widget(
                ratatui::widgets::Paragraph::new("All quiet"),
                Rect::new(30, 5, 20, 1),
            );
            let all = f.area();
            stars(f.buffer_mut(), all, 0);
        })
        .unwrap();
        let b = t.backend().buffer().clone();
        let row = |y: u16| {
            (0..80)
                .map(|x| b[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert_eq!(row(5).trim(), "All quiet");
        let n: usize = (0..12)
            .map(|y| row(y).chars().filter(|c| !c.is_whitespace()).count())
            .sum();
        assert!(n > 9 + 5 && n < 9 + 80, "{n}");
        // only glyphs: no cell's background is touched
        assert!((0..12).all(|y| (0..80).all(|x| b[(x, y)].bg == Color::Reset)));
    }
}
