//! The autopilot tour (`showcase.tour`): a timed walk through the dashboard
//! for booths, kiosks and recordings. Each step (`showcase.tour_views`, in
//! order) stays `showcase.tour_step`: it shows a view or sub-view, walks its
//! list (Fleet's sessions, the Inbox's rows, Monitor's machines), opens Focus
//! on a working session or a machine's sessions side by side, and captions
//! what the view is for — then the next, and round again.
//!
//! * Started by `showcase.tour_key` (ctrl+t), the palette's "Showcase: start
//!   the tour", or — `showcase.tour = idle` — by itself after
//!   `showcase.tour_idle` without a key or a click.
//! * Any key or click stops it and does nothing else: the dashboard is yours
//!   again where the tour left it.
//! * Navigation only, through [`Pilot`]: it never approves, sends, closes,
//!   attaches or types into a session, so it's safe on the real (masked)
//!   fleet.
//! * `▶ tour 2/5` in the header while it runs (beside the mode's chip and a
//!   recording's), never over the content; the caption is the spotlight's,
//!   held for the step.

use super::Pilot;
use crate::config::Config;
use crate::ui::c;
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Every step there is, in the order Settings lists them.
pub const VIEWS: &[&str] = &[
    "fleet",
    "focus",
    "tile",
    "monitor",
    "overview",
    "cpu",
    "memory",
    "gpu",
    "processes",
    "network",
    "services",
    "usage",
    "inbox",
    "settings",
    "doctor",
];
const SHORTEST: Duration = Duration::from_secs(3);
/// Walking a list: this many moves in a step (the first item, then these).
const MOVES: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Off,
    Key,
    Idle,
}

/// The tour as the settings have it.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub start: Start,
    pub keys: Vec<String>,
    pub idle: Duration,
    pub steps: Vec<&'static str>,
    pub step: Duration,
}

fn dur(cfg: &Config, key: &str, or: u64) -> Duration {
    crate::log::parse_duration(cfg.get_or(key).trim()).unwrap_or(Duration::from_secs(or))
}

pub fn settings(cfg: &Config) -> Settings {
    let start = match cfg.get_or("showcase.tour").trim() {
        "off" | "false" | "no" => Start::Off,
        "idle" => Start::Idle,
        _ => Start::Key,
    };
    let keys = cfg
        .get_or("showcase.tour_key")
        .split(',')
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty() && !matches!(k.as_str(), "off" | "none"))
        .collect();
    Settings {
        start,
        keys,
        idle: dur(cfg, "showcase.tour_idle", 120).max(Duration::from_secs(5)),
        steps: steps(&cfg.get_or("showcase.tour_views")),
        step: dur(cfg, "showcase.tour_step", 8).max(SHORTEST),
    }
}

/// `showcase.tour_views` as steps: the names it knows, in its order (`all`:
/// every one; none known: Fleet alone).
pub fn steps(v: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for w in v.split(',').map(|w| w.trim().to_lowercase()) {
        let all = w == "all";
        for x in VIEWS.iter().filter(|x| all || **x == w) {
            if !all || !out.contains(x) {
                out.push(x);
            }
        }
    }
    if out.is_empty() {
        out.push("fleet");
    }
    out
}

/// What each step is for: (its caption's title, the words).
pub fn caption(step: &str) -> (&'static str, &'static str) {
    match step {
        "fleet" => (
            "Fleet",
            "every agent on every machine, live — the preview follows the selection",
        ),
        "focus" => ("Focus", "one agent, full screen: reply, approve, queue"),
        "tile" => (
            "Tile",
            "a machine's agents side by side, every one of them live",
        ),
        "monitor" => ("Monitor", "CPU, GPU, memory and spend across the fleet"),
        "overview" => (
            "Monitor › Overview",
            "one machine in depth: cores, GPUs, disks, its agents",
        ),
        "cpu" => (
            "Monitor › CPU",
            "every core on every machine, and its history",
        ),
        "memory" => ("Monitor › Memory", "memory and swap across the fleet"),
        "gpu" => ("Monitor › GPU", "every GPU: load, memory, power and heat"),
        "processes" => (
            "Monitor › Processes",
            "what runs where, the agents' own first",
        ),
        "network" => (
            "Monitor › Network",
            "each machine's link: traffic and latency",
        ),
        "services" => (
            "Monitor › Services",
            "local LLM servers and AI gateways, found on every machine",
        ),
        "usage" => (
            "Monitor › Usage",
            "tokens, time and spend, from the agents' own logs",
        ),
        "inbox" => ("Inbox", "what needs you, what finished, what changed"),
        "settings" => ("Settings", "every setting, explained, and switchable here"),
        "doctor" => (
            "Settings › Doctor",
            "checks every machine, and fixes what it can",
        ),
        _ => ("Aleph", "every agent on every machine"),
    }
}

/// Does this step walk a list while it's up?
fn walks(step: &str) -> bool {
    matches!(step, "fleet" | "inbox" | "overview" | "gpu")
}

// ------------------------------------------------------------------ state

struct Run {
    step: usize,
    /// when the step began
    at: Instant,
    moves: u32,
    /// the last session Focus showed, the last machine Tile did
    focused: Option<String>,
    tiled: Option<String>,
}

struct Tour {
    run: Option<Run>,
    /// the last key or click
    input: Instant,
    /// the key or the palette asked for it
    asked: bool,
}

thread_local! {
    static T: RefCell<Tour> = RefCell::new(Tour {
        run: None,
        input: Instant::now(),
        asked: false,
    });
}

/// Is the tour on now?
#[cfg(test)]
pub fn running() -> bool {
    T.with(|t| t.borrow().run.is_some())
}

/// A key or a click: a running tour stops, and true (the input does nothing
/// else); either way it's the time of the last input (`idle`).
pub fn input() -> bool {
    let stopped = T.with(|t| {
        let mut t = t.borrow_mut();
        t.input = Instant::now();
        t.asked = false;
        t.run.take().is_some()
    });
    if stopped {
        // the spotlight's own caption, held back while the tour ran, goes too
        super::flair::hush();
    }
    stopped
}

/// Is `k` the tour's key?
pub fn is_key(cfg: &Config, k: &str) -> bool {
    let s = settings(cfg);
    s.start != Start::Off && s.keys.iter().any(|x| x == k)
}

/// A key in the dashboard's normal mode: true when it was the tour's key
/// (the tour starts with the next tick).
pub fn key(cfg: &Config, k: &str) -> bool {
    if !super::on() || !is_key(cfg, k) {
        return false;
    }
    start();
    true
}

/// Start with the next tick (the palette, the key).
pub fn start() {
    T.with(|t| t.borrow_mut().asked = true);
}

/// Every turn of the dashboard's loop: start (asked, or idle long enough),
/// or carry on — the next move, the next step.
pub fn tick(p: &mut impl Pilot) {
    let s = settings(p.cfg());
    let off = !super::on() || super::paused() || s.start == Start::Off;
    // (the run is taken out while the dashboard moves, and put back)
    let Some((run, asked, input)) = T.with(|t| {
        let mut t = t.borrow_mut();
        if off {
            t.run = None;
            t.asked = false;
            return None;
        }
        Some((t.run.take(), t.asked, t.input))
    }) else {
        return;
    };
    let run = match run {
        None => {
            let idle = s.start == Start::Idle && input.elapsed() >= s.idle;
            if !(asked || idle) || !p.free() {
                return;
            }
            let mut r = Run {
                step: 0,
                at: Instant::now(),
                moves: 0,
                focused: None,
                tiled: None,
            };
            enter(&mut r, &s, p, 0);
            Some(r)
        }
        // something else took the screen (a popup from the background)
        Some(_) if !p.free() => None,
        Some(mut r) => {
            let el = r.at.elapsed();
            let step = s.steps.get(r.step).copied().unwrap_or("fleet");
            let due = s.step.mul_f64((r.moves + 1) as f64 / (MOVES + 2) as f64);
            if el >= s.step {
                let next = (r.step + 1) % s.steps.len();
                enter(&mut r, &s, p, next);
            } else if walks(step) && r.moves < MOVES && el >= due {
                r.moves += 1;
                p.walk(false);
            }
            Some(r)
        }
    };
    T.with(|t| {
        let mut t = t.borrow_mut();
        t.asked = false;
        t.run = run;
    });
}

/// Begin step `i` (or the next that can be shown: Focus needs a session).
fn enter(r: &mut Run, s: &Settings, p: &mut impl Pilot, i: usize) {
    let n = s.steps.len();
    for k in 0..n {
        let at = (i + k) % n;
        let step = s.steps[at];
        let shown = match step {
            "focus" => p.focus(r.focused.as_deref()).map(|x| r.focused = Some(x)),
            "tile" => p.tile(r.tiled.as_deref()).map(|x| r.tiled = Some(x)),
            other => p.show(other).then_some(()),
        };
        if shown.is_some() {
            if walks(step) {
                p.walk(true);
            }
            r.step = at;
            r.at = Instant::now();
            r.moves = 0;
            return;
        }
    }
    // nothing to show: Fleet, as it is
    p.show("fleet");
    r.step = 0;
    r.at = Instant::now();
    r.moves = 0;
}

/// (step, of, its name, how far along 0–1) while running.
fn now(cfg: &Config) -> Option<(usize, usize, &'static str, f32)> {
    let s = settings(cfg);
    T.with(|t| {
        let t = t.borrow();
        let r = t.run.as_ref()?;
        let step = *s.steps.get(r.step)?;
        let k = r.at.elapsed().as_secs_f32() / s.step.as_secs_f32();
        Some((r.step + 1, s.steps.len(), step, k.min(1.0)))
    })
}

// ------------------------------------------------------------------ on screen

/// The header, after the mode's chip (and a recording's): `▶ tour 2/5`.
pub fn chip(cfg: &Config, left: &mut Vec<Span<'static>>) {
    let Some((i, n, _, _)) = now(cfg) else {
        return;
    };
    let (play, words) = if c::mono() {
        let s = Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD);
        (s, s)
    } else {
        let bg = Style::new().bg(c::SEL());
        (
            bg.fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            bg.fg(c::TEXT()).add_modifier(Modifier::BOLD),
        )
    };
    let at = left.len().saturating_sub(1).max(1).min(left.len());
    left.insert(at, Span::styled(format!("tour {i}/{n} "), words));
    left.insert(at, Span::styled(" ▶ ", play));
    left.insert(at, Span::raw("  "));
}

/// The step's caption, where the spotlight's goes (true: drawn, so the
/// spotlight's own isn't); faint for its last tenth. None under a popup.
pub fn overlay(cfg: &Config, buf: &mut Buffer, area: Rect, quiet: bool) -> bool {
    let Some((_, _, step, k)) = now(cfg) else {
        return false;
    };
    if quiet || area.height < 6 {
        return true;
    }
    let (title, what) = caption(step);
    super::flair::caption_line(buf, area, title, what, k > 0.9);
    true
}

/// The palette's entry: (id, label, what it does).
pub fn items(cfg: &Config) -> Vec<(&'static str, String, String)> {
    let s = settings(cfg);
    if !super::on() || s.start == Start::Off {
        return Vec::new();
    }
    let key = s
        .keys
        .first()
        .map(|k| format!(" · {k}"))
        .unwrap_or_default();
    vec![(
        "tour",
        "Showcase: start the tour".to_string(),
        format!(
            "a {} walk through {} — any key stops it{key}",
            crate::ui::dur(s.step.as_secs() * s.steps.len() as u64),
            s.steps.join(", ")
        ),
    )]
}

/// What the help lists for it: (keys, what, the palette's id, search words).
pub fn help(cfg: &Config) -> Option<(String, String, &'static str, &'static [&'static str])> {
    let s = settings(cfg);
    if s.start == Start::Off || s.keys.is_empty() {
        return None;
    }
    Some((
        s.keys.join(" · "),
        "showcase: start the autopilot tour (any key stops it)".into(),
        "tour",
        &[
            "autopilot",
            "tour",
            "demo",
            "kiosk",
            "booth",
            "walkthrough",
            "present",
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dashboard that only says what was asked of it.
    struct Fake {
        cfg: Config,
        free: bool,
        did: Vec<String>,
        sessions: Vec<&'static str>,
    }

    impl Pilot for Fake {
        fn cfg(&self) -> &Config {
            &self.cfg
        }
        fn free(&self) -> bool {
            self.free
        }
        fn show(&mut self, name: &str) -> bool {
            self.did.push(format!("show {name}"));
            name != "services"
        }
        fn walk(&mut self, first: bool) {
            self.did.push(if first { "first" } else { "next" }.into());
        }
        fn focus(&mut self, after: Option<&str>) -> Option<String> {
            let i = after
                .and_then(|a| self.sessions.iter().position(|s| *s == a))
                .map_or(0, |i| i + 1);
            let s = self
                .sessions
                .get(i % self.sessions.len().max(1))?
                .to_string();
            self.did.push(format!("focus {s}"));
            Some(s)
        }
        fn tile(&mut self, _after: Option<&str>) -> Option<String> {
            None
        }
    }

    fn fake(lines: &[&str]) -> Fake {
        let mut l = vec!["showcase.tour_step=3s"];
        l.extend_from_slice(lines);
        Fake {
            cfg: Config::in_memory(&l),
            free: true,
            did: Vec::new(),
            sessions: vec!["a|1", "b|2"],
        }
    }

    fn rewind(by: Duration) {
        T.with(|t| {
            let mut t = t.borrow_mut();
            t.input -= by;
            if let Some(r) = t.run.as_mut() {
                r.at -= by;
            }
        });
    }

    #[test]
    fn the_settings_read_as_a_plan() {
        let s = settings(&Config::in_memory(&[]));
        assert_eq!(s.start, Start::Key);
        assert_eq!(s.keys, vec!["ctrl+t".to_string()]);
        assert_eq!(s.steps, vec!["fleet", "focus", "monitor", "usage", "inbox"]);
        assert_eq!(
            (s.step, s.idle),
            (Duration::from_secs(8), Duration::from_secs(120))
        );
        assert_eq!(steps("all"), VIEWS.to_vec());
        assert_eq!(steps("inbox, nowhere, fleet"), vec!["inbox", "fleet"]);
        assert_eq!(steps(""), vec!["fleet"]);
        let s = settings(&Config::in_memory(&[
            "showcase.tour=idle",
            "showcase.tour_key=f8, ctrl+t",
            "showcase.tour_step=1s",
        ]));
        assert_eq!(s.start, Start::Idle);
        assert_eq!(s.keys, vec!["f8".to_string(), "ctrl+t".to_string()]);
        assert_eq!(s.step, SHORTEST);
        let off = Config::in_memory(&["showcase.tour=off"]);
        assert!(!is_key(&off, "ctrl+t"));
        assert!(is_key(&Config::in_memory(&[]), "ctrl+t"));
        // every step has a caption of its own
        for v in VIEWS {
            assert_ne!(caption(v).0, "Aleph", "{v}");
        }
    }

    #[test]
    fn it_walks_the_steps_and_loops() {
        crate::showcase::force("masked");
        let mut p = fake(&["showcase.tour_views=fleet,focus,services,inbox"]);
        // nothing until asked
        tick(&mut p);
        assert!(!running() && p.did.is_empty());
        assert!(key(&p.cfg.clone(), "ctrl+t"));
        tick(&mut p);
        assert!(running());
        assert_eq!(p.did, vec!["show fleet", "first"]);
        // the list walks while the step is up
        rewind(Duration::from_millis(1100));
        tick(&mut p);
        assert_eq!(p.did.last().map(String::as_str), Some("next"));
        // the next step: Focus on a session
        rewind(Duration::from_secs(3));
        tick(&mut p);
        assert_eq!(p.did.last().map(String::as_str), Some("focus a|1"));
        assert_eq!(now(&p.cfg).map(|x| (x.0, x.1, x.2)), Some((2, 4, "focus")));
        // a step that can't be shown is passed over (services)
        rewind(Duration::from_secs(3));
        tick(&mut p);
        assert!(p.did.iter().any(|d| d == "show services"));
        assert_eq!(now(&p.cfg).map(|x| x.2), Some("inbox"));
        // and round again; Focus shows the next session this time
        rewind(Duration::from_secs(3));
        tick(&mut p);
        assert_eq!(now(&p.cfg).map(|x| x.2), Some("fleet"));
        rewind(Duration::from_secs(3));
        tick(&mut p);
        assert_eq!(p.did.last().map(String::as_str), Some("focus b|2"));
        // any key stops it, and that's all it does
        assert!(input());
        assert!(!running() && !input());
        tick(&mut p);
        assert!(!running());
        crate::showcase::force("off");
    }

    #[test]
    fn idle_starts_it_and_off_or_busy_never_do() {
        crate::showcase::force("masked");
        let mut p = fake(&["showcase.tour=idle", "showcase.tour_idle=30s"]);
        input();
        tick(&mut p);
        assert!(!running());
        // a popup open: not yet
        rewind(Duration::from_secs(31));
        p.free = false;
        tick(&mut p);
        assert!(!running());
        p.free = true;
        tick(&mut p);
        assert!(running());
        // something took the screen: it stops
        p.free = false;
        tick(&mut p);
        assert!(!running());
        // off: the key does nothing; showcase off: nothing at all
        let mut q = fake(&["showcase.tour=off"]);
        assert!(!key(&q.cfg.clone(), "ctrl+t"));
        start();
        tick(&mut q);
        assert!(!running());
        crate::showcase::force("off");
        let mut r = fake(&[]);
        assert!(!key(&r.cfg.clone(), "ctrl+t"));
        start();
        tick(&mut r);
        assert!(!running() && r.did.is_empty());
        assert!(items(&r.cfg).is_empty());
    }
}
