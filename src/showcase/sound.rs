//! Sound cues (`showcase.sounds` = on; off by default). A question, a
//! finished turn, a usage limit and a machine going down each have a sound:
//! `showcase.sound.needs` / `.done` / `.limit` / `.down`, a macOS system
//! sound's name (Glass, Hero, Basso, Sosumi, …), a sound file's path, or
//! `off`. `aleph showcase sound [event…]` plays them, to choose.
//!
//! The process that runs the hub plays them — the daemon, or the dashboard
//! while no daemon runs (`Router::tick` asks, and only the hub ticks a
//! router) — so each event sounds once however many windows are open; the
//! demo world's events count too. Several at once make one sound, the most
//! urgent (a machine down, a limit, a question, a finished turn), and never
//! two within a second. The settings are read again every few seconds, so
//! switching them reaches a running daemon too.
//!
//! macOS plays `/System/Library/Sounds/<name>.aiff` with `afplay`; Linux
//! the freedesktop theme's nearest with `canberra-gtk-play`, else `paplay`,
//! when either is there; anywhere else, nothing. Fire and forget, with a
//! timeout. Never in tests or in the fake world (`ALEPH_FAKE_*` set), nor
//! with `ALEPH_NO_SOUND`: there it only says what it would have played (the
//! debug log, and `aleph showcase sound`).

use crate::config::Config;
use crate::events::{Event, Kind};
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

pub const SETTING: &str = "showcase.sounds";
/// never two sounds closer than this
const GAP: Duration = Duration::from_millis(1100);
/// how far back an event still counts (another process may log it late)
const LOOK_BACK: u64 = 30;
/// how long a sound may play
const PLAY_FOR: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cue {
    /// a turn finished (the least urgent)
    Done,
    /// a session needs you
    Needs,
    /// stopped at a usage limit
    Limit,
    /// a machine stopped answering
    Down,
}

impl Cue {
    pub const ALL: [Cue; 4] = [Cue::Needs, Cue::Done, Cue::Limit, Cue::Down];

    pub fn name(self) -> &'static str {
        match self {
            Cue::Needs => "needs",
            Cue::Done => "done",
            Cue::Limit => "limit",
            Cue::Down => "down",
        }
    }

    pub fn parse(s: &str) -> Option<Cue> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "needs" | "needs-you" | "blocked" | "question" => Cue::Needs,
            "done" | "finished" => Cue::Done,
            "limit" => Cue::Limit,
            "down" | "error" | "unreachable" => Cue::Down,
            _ => return None,
        })
    }

    /// Its setting (`showcase.sound.needs`, …).
    pub fn key(self) -> &'static str {
        match self {
            Cue::Needs => "showcase.sound.needs",
            Cue::Done => "showcase.sound.done",
            Cue::Limit => "showcase.sound.limit",
            Cue::Down => "showcase.sound.down",
        }
    }

    /// What it's for, in words.
    pub fn what(self) -> &'static str {
        match self {
            Cue::Needs => "a session needs you",
            Cue::Done => "a turn finished",
            Cue::Limit => "a usage limit",
            Cue::Down => "a machine went down",
        }
    }

    /// An event's cue, if it has one (pure).
    pub fn of(e: &Event) -> Option<Cue> {
        Some(match e.kind {
            Kind::Blocked => Cue::Needs,
            Kind::Done => Cue::Done,
            Kind::Limit => Cue::Limit,
            // a machine's own error (no session): unreachable
            Kind::Error if e.name.is_empty() => Cue::Down,
            _ => return None,
        })
    }
}

/// Are sound cues on (showcase mode on, and `showcase.sounds` says so)?
pub fn wanted(cfg: &Config) -> bool {
    super::on()
        && matches!(
            cfg.get_or(SETTING).trim().to_ascii_lowercase().as_str(),
            "on" | "true" | "yes" | "1"
        )
}

/// Why no sound may play here (tests, the fake world, `ALEPH_NO_SOUND`).
pub fn hushed() -> Option<&'static str> {
    if cfg!(test) {
        return Some("tests");
    }
    if std::env::var_os("ALEPH_NO_SOUND").is_some() {
        return Some("ALEPH_NO_SOUND is set");
    }
    if std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("ALEPH_FAKE_")) {
        return Some("the fake world");
    }
    None
}

/// The freedesktop sound theme's nearest to a macOS sound's name (a name
/// it doesn't know is taken as the theme's own: `bell`, `complete`, …).
fn freedesktop(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "glass" | "ping" | "tink" | "pop" => "message-new-instant",
        "hero" | "purr" | "blow" => "complete",
        "basso" | "funk" | "bottle" => "dialog-warning",
        "sosumi" | "submarine" | "frog" | "morse" => "dialog-error",
        other => return other.to_string(),
    }
    .to_string()
}

/// Is a sound setting a "no sound"?
fn silent(s: &str) -> bool {
    s.trim().is_empty()
        || matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "off" | "none" | "no" | "false" | "-"
        )
}

/// (program, args) that play `sound` on `os` (std's name), or None (pure —
/// tested): `has` finds a program, `exists` a file.
pub fn command(
    os: &str,
    sound: &str,
    has: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> Option<(String, Vec<String>)> {
    let s = sound.trim();
    if silent(s) {
        return None;
    }
    let file = s.contains('/');
    match os {
        "macos" => {
            let path = if file {
                s.to_string()
            } else {
                format!("/System/Library/Sounds/{s}.aiff")
            };
            (has("afplay") && exists(&path)).then(|| ("afplay".to_string(), vec![path]))
        }
        "linux" if file => {
            (has("paplay") && exists(s)).then(|| ("paplay".to_string(), vec![s.to_string()]))
        }
        "linux" => {
            let id = freedesktop(s);
            if has("canberra-gtk-play") {
                return Some((
                    "canberra-gtk-play".to_string(),
                    vec!["-i".into(), id, "-d".into(), crate::brand::BIN.into()],
                ));
            }
            let f = format!("/usr/share/sounds/freedesktop/stereo/{id}.oga");
            (has("paplay") && exists(&f)).then(|| ("paplay".to_string(), vec![f]))
        }
        _ => None,
    }
}

/// What came of a cue.
#[derive(Debug, PartialEq, Eq)]
pub enum Played {
    /// it's playing: the command
    Now(String),
    /// it would have, but not here (why, the command)
    Hushed(&'static str, String),
    /// the setting says no sound
    Off,
    /// nothing here plays it
    Nothing,
}

/// Play `cue`'s sound (`wait`: until it's done, for the command line).
pub fn play(cfg: &Config, cue: Cue, wait: bool) -> Played {
    let sound = cfg.get_or(cue.key());
    if silent(&sound) {
        return Played::Off;
    }
    let has = |p: &str| crate::proc::which(p).is_some();
    let exists = |f: &str| std::path::Path::new(f).exists();
    let Some((prog, args)) = command(std::env::consts::OS, &sound, &has, &exists) else {
        return Played::Nothing;
    };
    let line = format!("{prog} {}", args.join(" "));
    if let Some(why) = hushed() {
        crate::log::debug(
            "showcase",
            &format!("sound for {} not played ({why}): {line}", cue.name()),
        );
        return Played::Hushed(why, line);
    }
    crate::log::debug("showcase", &format!("sound for {}: {line}", cue.name()));
    let run = move || {
        let _ = crate::proc::run(
            std::process::Command::new(&prog).args(&args),
            PLAY_FOR,
            None,
        );
    };
    if wait {
        run();
    } else {
        std::thread::spawn(run);
    }
    Played::Now(line)
}

// ------------------------------------------------------------------ the ear

/// What the hub has heard: the event log's new events, each once.
struct Ear {
    /// nothing before this is news (it starts at "now": no replay)
    start: u64,
    /// the log as last read (its length and time), to read it only anew
    stamp: Option<(u64, Option<SystemTime>)>,
    seen: HashSet<String>,
    order: VecDeque<String>,
    /// (host, session, cue, when): herdr and an agent's own hook say the
    /// same news once
    told: VecDeque<(String, String, Cue, u64)>,
    last: Option<Instant>,
}

impl Ear {
    fn new(start: u64) -> Ear {
        Ear {
            start,
            stamp: None,
            seen: HashSet::new(),
            order: VecDeque::new(),
            told: VecDeque::new(),
            last: None,
        }
    }

    /// The most urgent new cue in `evs` at `now` (pure but for what it
    /// remembers — tested).
    fn hear(&mut self, evs: &[Event], now: u64) -> Option<Cue> {
        let mut best = None;
        for e in evs {
            if e.t < self.start || e.t + LOOK_BACK < now || e.source == "test" {
                continue;
            }
            let Some(cue) = Cue::of(e) else {
                continue;
            };
            let id = e.id();
            if !self.seen.insert(id.clone()) {
                continue;
            }
            self.order.push_back(id);
            while self.order.len() > 1000 {
                if let Some(x) = self.order.pop_front() {
                    self.seen.remove(&x);
                }
            }
            let again = self.told.iter().any(|(h, n, c, t)| {
                *h == e.host && *n == e.name && *c == cue && t.abs_diff(e.t) <= LOOK_BACK
            });
            if again {
                continue;
            }
            self.told
                .push_back((e.host.clone(), e.name.clone(), cue, e.t));
            while self.told.len() > 200 {
                self.told.pop_front();
            }
            best = best.max(Some(cue));
        }
        best
    }
}

static EAR: Mutex<Option<Ear>> = Mutex::new(None);

/// The settings as the files have them now (a daemon keeps the config it
/// started with): read again every few seconds.
fn settings() -> Config {
    static C: Mutex<Option<(Instant, Config)>> = Mutex::new(None);
    let mut g = C.lock().unwrap_or_else(|e| e.into_inner());
    match &*g {
        Some((at, c)) if at.elapsed() < Duration::from_secs(4) => c.clone(),
        _ => {
            let c = Config::load();
            *g = Some((Instant::now(), c.clone()));
            c
        }
    }
}

/// Each hub tick: a sound for what the event log says just happened.
pub fn tick() {
    // (safety: the pause is quiet too, with nothing kept to play after)
    if cfg!(test) || !super::on() || super::paused() {
        if let Ok(mut g) = EAR.lock() {
            *g = None;
        }
        return;
    }
    let cfg = settings();
    let mut g = EAR.lock().unwrap_or_else(|e| e.into_inner());
    if !wanted(&cfg) {
        *g = None;
        return;
    }
    let now = crate::cache::now();
    let ear = g.get_or_insert_with(|| Ear::new(now));
    let stamp = std::fs::metadata(crate::events::file())
        .ok()
        .map(|m| (m.len(), m.modified().ok()));
    if stamp == ear.stamp {
        return;
    }
    ear.stamp = stamp;
    let Some(cue) = ear.hear(&crate::events::load(80), now) else {
        return;
    };
    if ear.last.is_some_and(|t| t.elapsed() < GAP) {
        return;
    }
    ear.last = Some(Instant::now());
    drop(g);
    play(&cfg, cue, false);
}

// ------------------------------------------------------------------ the command

/// `aleph showcase sound [event…]`: play each (all four by default), or
/// say why not.
pub fn cmd(rest: &[String]) -> std::process::ExitCode {
    use crate::out::{err, hint, ok, warn};
    use std::process::ExitCode;
    let cfg = Config::load();
    let mut cues: Vec<Cue> = Vec::new();
    for a in rest.iter().filter(|a| !a.starts_with('-')) {
        match Cue::parse(a) {
            Some(c) => cues.push(c),
            None => {
                err(&format!(
                    "no sound for “{a}”: needs, done, limit or down (aleph showcase sound [event…])"
                ));
                return ExitCode::from(2);
            }
        }
    }
    if cues.is_empty() {
        cues = Cue::ALL.to_vec();
    }
    if !wanted(&cfg) {
        let why = if super::on() {
            "showcase.sounds is off"
        } else {
            "showcase mode is off"
        };
        warn(&format!(
            "{why}: events won't sound (this plays them anyway)"
        ));
    }
    for (i, c) in cues.iter().enumerate() {
        let sound = cfg.get_or(c.key());
        let label = format!("{:<6} {:<10} {}", c.name(), sound, c.what());
        match play(&cfg, *c, true) {
            Played::Now(_) => ok(&label),
            Played::Hushed(why, line) => ok(&format!("{label} — not played ({why}): {line}")),
            Played::Off => ok(&format!("{label} — off")),
            Played::Nothing => warn(&format!(
                "{label} — nothing here plays it (macOS: a system sound or a file; Linux: canberra-gtk-play or paplay)"
            )),
        }
        if i + 1 < cues.len() && hushed().is_none() {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    hint("aleph settings set showcase.sound.<event> <name | path | off> · showcase.sounds on");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: Kind, host: &str, name: &str, t: u64) -> Event {
        Event {
            t,
            host: host.into(),
            name: name.into(),
            kind,
            pane: format!("w1:{name}"),
            seq: t,
            ..Default::default()
        }
    }

    #[test]
    fn each_event_has_its_cue() {
        assert_eq!(Cue::of(&ev(Kind::Blocked, "b31", "x", 1)), Some(Cue::Needs));
        assert_eq!(Cue::of(&ev(Kind::Done, "b31", "x", 1)), Some(Cue::Done));
        assert_eq!(Cue::of(&ev(Kind::Limit, "b31", "x", 1)), Some(Cue::Limit));
        assert_eq!(Cue::of(&ev(Kind::Error, "b31", "", 1)), Some(Cue::Down));
        assert_eq!(Cue::of(&ev(Kind::New, "b31", "x", 1)), None);
        assert_eq!(Cue::of(&ev(Kind::HostUp, "b31", "", 1)), None);
        for c in Cue::ALL {
            assert_eq!(Cue::parse(c.name()), Some(c));
            assert!(crate::config::known(c.key()).is_some(), "{}", c.key());
        }
        assert!(Cue::Down > Cue::Limit && Cue::Limit > Cue::Needs && Cue::Needs > Cue::Done);
        // the defaults
        let cfg = Config::in_memory(&[]);
        let names: Vec<String> = Cue::ALL.iter().map(|c| cfg.get_or(c.key())).collect();
        assert_eq!(names, ["Glass", "Hero", "Basso", "Sosumi"]);
        assert_eq!(cfg.get_or(SETTING), "off");
    }

    #[test]
    fn each_event_sounds_once_and_the_most_urgent_wins() {
        let mut ear = Ear::new(1000);
        let now = 1010;
        // newest first, as the log reads
        let evs = vec![
            ev(Kind::Done, "b31", "ignite", 1009),
            ev(Kind::Blocked, "gpu2", "weave", 1008),
            ev(Kind::New, "b31", "temper", 1007),
            // older than this hub: never news
            ev(Kind::Error, "dex42", "", 990),
        ];
        assert_eq!(ear.hear(&evs, now), Some(Cue::Needs));
        assert_eq!(ear.hear(&evs, now), None, "heard already");
        // the same question told again by the agent's own hook
        let mut hook = ev(Kind::Blocked, "gpu2", "weave", 1011);
        hook.source = "agent".into();
        hook.pane = String::new();
        assert_eq!(ear.hear(&[hook], 1012), None);
        // a machine going down beats a finished turn
        let evs = vec![
            ev(Kind::Done, "b31", "temper", 1020),
            ev(Kind::Error, "dex42", "", 1019),
        ];
        assert_eq!(ear.hear(&evs, 1021), Some(Cue::Down));
        // too late to be news; `aleph notify test`'s own
        assert_eq!(ear.hear(&[ev(Kind::Done, "b31", "z", 1030)], 1100), None);
        let mut t = ev(Kind::Done, "b31", "y", 1100);
        t.source = "test".into();
        assert_eq!(ear.hear(&[t], 1101), None);
    }

    #[test]
    fn commands_per_platform() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert_eq!(
            command("macos", "Glass", &all, &all),
            Some((
                "afplay".into(),
                vec!["/System/Library/Sounds/Glass.aiff".into()]
            ))
        );
        assert_eq!(
            command("macos", "/u/x.wav", &all, &all).map(|c| c.1),
            Some(vec!["/u/x.wav".into()])
        );
        // a name macOS doesn't have: nothing
        assert_eq!(command("macos", "Nope", &all, &none), None);
        assert_eq!(command("macos", "off", &all, &all), None);
        assert_eq!(command("macos", " ", &all, &all), None);
        // Linux: the freedesktop theme's nearest
        let canberra = |p: &str| p == "canberra-gtk-play";
        let c = command("linux", "Hero", &canberra, &none).unwrap();
        assert_eq!(c.0, "canberra-gtk-play");
        assert_eq!(c.1[..2], ["-i".to_string(), "complete".to_string()]);
        let pa = |p: &str| p == "paplay";
        assert_eq!(
            command("linux", "Sosumi", &pa, &all),
            Some((
                "paplay".into(),
                vec!["/usr/share/sounds/freedesktop/stereo/dialog-error.oga".into()]
            ))
        );
        assert_eq!(command("linux", "Sosumi", &pa, &none), None);
        assert_eq!(command("linux", "Basso", &none, &all), None);
        assert_eq!(command("windows", "Glass", &all, &all), None);
    }

    #[test]
    fn the_sounds_have_a_table_of_their_own() {
        use crate::tomlcfg::{from_path, to_path};
        assert_eq!(
            to_path("showcase.sound.needs"),
            ["showcase", "sound", "needs"]
        );
        assert_eq!(
            from_path(&["showcase", "sound", "needs"]).as_deref(),
            Some("showcase.sound.needs")
        );
        assert_eq!(to_path(SETTING), ["showcase", "sounds"]);
        assert_eq!(from_path(&["showcase", "sounds"]).as_deref(), Some(SETTING));
        assert_eq!(
            from_path(&["showcase", "palette"]).as_deref(),
            Some("showcase.palette")
        );
    }

    #[test]
    fn never_a_sound_in_tests() {
        assert_eq!(hushed(), Some("tests"));
        let cfg = Config::in_memory(&["showcase.sound.done=Hero"]);
        match play(&cfg, Cue::Done, false) {
            Played::Hushed(why, _) => assert_eq!(why, "tests"),
            Played::Nothing => {} // a machine without a player
            p => panic!("{p:?}"),
        }
        let off = Config::in_memory(&["showcase.sound.done=off"]);
        assert_eq!(play(&off, Cue::Done, false), Played::Off);
        assert!(
            !wanted(&Config::in_memory(&["showcase.sounds=on"])),
            "showcase is off"
        );
    }
}
