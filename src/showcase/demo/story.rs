//! Story mode (`showcase.story`): every `showcase.story_every` of the world's
//! time, the demo world plays its beats (`showcase.story_beats`), in order:
//!
//! 1. **ask** — europa's Claude stops to ask before running a command. Every
//!    other cycle it's a soak test the world's own auto-approve rule
//!    (`story`, `*relay-soak*`) answers within a poll; in between it's a
//!    rollout no rule matches, which waits for you (y in the dashboard,
//!    Raycast, the web console) until the cycle ends;
//! 2. **drop** — vogsphere's herdr stops; the hub's self-heal restarts it and
//!    its sessions come back (or it comes back by itself at the beat's end);
//! 3. **limit** — magrathea's Codex hits its plan's 5-hour limit, and later
//!    the window resets;
//! 4. **finish** — magrathea's Claude finishes a big task.
//!
//! Each beat takes the place of its own part of the world's old schedule
//! (vogsphere's link dropping a minute in ten, Codex's limit 4 minutes in
//! 20); a beat left out of `showcase.story_beats` leaves that part as it was.
//!
//! The beats are part of the world's script — pure, from (seed, time,
//! actions, these settings) — so every process plays the same beat in the
//! same second. And each reaches you through the product's own paths: the
//! fleet's transitions, the hub's rules and self-heal, the inbox, desktop
//! notifications, the menu bar's push, Raycast and the web console.

use super::world::{Act, Action, Ask, At, Phase, Step, Task, K};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const ASK: u8 = 1;
pub const DROP: u8 = 2;
pub const LIMIT: u8 = 4;
pub const FINISH: u8 = 8;
pub const ALL: u8 = ASK | DROP | LIMIT | FINISH;
/// (name in `showcase.story_beats`, bit)
pub const BEATS: &[(&str, u8)] = &[
    ("ask", ASK),
    ("drop", DROP),
    ("limit", LIMIT),
    ("finish", FINISH),
];

/// Who plays each beat: (machine, pane).
pub const ASKER: (&str, &str) = ("europa", "w1:p2");
pub const DROPPER: &str = "vogsphere";
pub const LIMITED: (&str, &str) = ("magrathea", "w1:p2");
pub const FINISHER: (&str, &str) = ("magrathea", "w1:p1");
/// The world's own auto-approve rule: (name, pattern). It matches the soak
/// test and nothing else the world asks.
pub const RULE: (&str, &str) = ("story", "*relay-soak*");
/// How long herdr takes to come back once the hub restarts it (the world's
/// seconds).
pub const RESTART: u64 = 4;
/// The shortest cycle, in the world's seconds (each beat must outlast a
/// poll or two).
pub const MIN_EVERY: u64 = 60;
/// Cycles count from here (any fixed moment: every process agrees).
pub const ORIGIN: u64 = super::world::EPOCH as u64;

/// The story as the settings have it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Story {
    /// one cycle, in the world's seconds
    pub every: u64,
    /// which beats play (`ASK | DROP | …`)
    pub beats: u8,
}

impl Default for Story {
    fn default() -> Self {
        Story {
            every: 120,
            beats: ALL,
        }
    }
}

impl Story {
    pub fn has(&self, b: u8) -> bool {
        self.beats & b != 0
    }
    pub fn every(&self) -> u64 {
        self.every.max(MIN_EVERY)
    }
    /// (cycle, seconds into it) at `t`.
    pub fn place(&self, t: u64) -> (u64, u64) {
        let r = t.saturating_sub(ORIGIN);
        (r / self.every(), r % self.every())
    }
    /// When the question comes up.
    pub fn ask_at(&self) -> u64 {
        (self.every() / 20).max(3)
    }
    /// How long it works on once answered, then shows as done.
    pub fn after_for(&self) -> u64 {
        self.every() * 3 / 10
    }
    pub fn done_for(&self) -> u64 {
        self.every() * 3 / 20
    }
    /// The drop's window: [from, until) into the cycle.
    pub fn drop_window(&self) -> (u64, u64) {
        let a = self.every() / 4;
        (a, a + (self.every() / 5).max(20))
    }
    /// The limit's window: [from, until) into the cycle.
    pub fn limit_window(&self) -> (u64, u64) {
        let e = self.every();
        let a = e / 2;
        (a, (a + (e * 7 / 20).max(30)).min(e - 1))
    }
    /// When the big task finishes.
    pub fn finish_at(&self) -> u64 {
        self.every() * 17 / 20
    }
    /// The beats this cycle, in order: (name, starts, ends) in seconds into it.
    pub fn plan(&self) -> Vec<(&'static str, u64, u64)> {
        let e = self.every();
        let (d0, d1) = self.drop_window();
        let (l0, l1) = self.limit_window();
        [
            (ASK, "ask", self.ask_at(), d0),
            (DROP, "drop", d0, d1),
            (LIMIT, "limit", l0, l1),
            (FINISH, "finish", self.finish_at(), e),
        ]
        .into_iter()
        .filter(|(b, ..)| self.has(*b))
        .map(|(_, n, a, z)| (n, a, z))
        .collect()
    }
    /// What's on at `t`: (beat name, seconds left in it), the latest that
    /// has started (for `showcase status`).
    pub fn beat_now(&self, t: u64) -> Option<(&'static str, u64)> {
        let (_, p) = self.place(t);
        self.plan()
            .into_iter()
            .filter(|(_, a, z)| (*a..*z).contains(&p))
            .max_by_key(|(_, a, _)| *a)
            .map(|(n, _, z)| (n, z - p))
    }
}

/// `showcase.story_beats`: a comma list of beat names (`all`; `off` or
/// none: no beat, so no story).
pub fn parse_beats(s: &str) -> u8 {
    let mut b = 0;
    for w in s.split(',').map(str::trim) {
        if w.eq_ignore_ascii_case("all") {
            b |= ALL;
        }
        if let Some((_, x)) = BEATS.iter().find(|(n, _)| n.eq_ignore_ascii_case(w)) {
            b |= x;
        }
    }
    b
}

/// The story the settings ask for (None: off, or no beat left).
pub fn of(cfg: &crate::config::Config) -> Option<Story> {
    let on = cfg.get_or("showcase.story").to_lowercase();
    if matches!(on.trim(), "off" | "false" | "no" | "0") {
        return None;
    }
    let every = crate::log::parse_duration(&cfg.get_or("showcase.story_every"))
        .map(|d| d.as_secs())
        .unwrap_or(120)
        .max(MIN_EVERY);
    let beats = parse_beats(&cfg.get_or("showcase.story_beats"));
    (beats != 0).then_some(Story { every, beats })
}

thread_local! {
    /// tests: a story of their own (Some(None): none)
    static FORCED: std::cell::Cell<Option<Option<Story>>> = const { std::cell::Cell::new(None) };
}

/// Tests: run with this story (Some(None): off; None: back to the default,
/// which in tests is none).
#[cfg(test)]
pub fn force(s: Option<Option<Story>>) {
    FORCED.with(|f| f.set(s));
}

/// The story now: the settings file, looked at again every 2 s (so every
/// process picks a change up within that).
pub fn current() -> Option<Story> {
    if let Some(s) = FORCED.with(|f| f.get()) {
        return s;
    }
    if cfg!(test) {
        return None;
    }
    type Cache = Option<(Instant, Option<Story>)>;
    static C: Mutex<Cache> = Mutex::new(None);
    if let Some((at, s)) = *C.lock().unwrap_or_else(|e| e.into_inner()) {
        if at.elapsed() < Duration::from_secs(2) {
            return s;
        }
    }
    // (read without the lock held: the settings ask the demo world back)
    let s = of(&crate::config::Config::load());
    *C.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), s));
    s
}

// ------------------------------------------------------------------ what's asked

const fn st(k: K, arg: &'static str, out: &'static str) -> Step {
    Step { k, arg, out }
}

/// The asker's two questions: a soak test its rule answers, a rollout that
/// waits for you.
pub const ASKS: [Task; 2] = [
    Task {
        title: "Soak-testing the subspace relay",
        verb: "Soaking",
        prompt: "the relay holds at warp 6 now — soak-test it at warp 9.6 before we ship",
        steps: &[
            st(K::Read, "src/relay/fec.rs", "Read 366 lines"),
            st(
                K::Run,
                "cargo test relay",
                "test result: ok. 37 passed; 0 failed",
            ),
            st(
                K::Edit,
                "examples/relay-soak.rs",
                "Wrote 58 lines to examples/relay-soak.rs",
            ),
            st(
                K::Run,
                "cargo build --examples",
                "Finished `dev` profile in 4.12s",
            ),
        ],
        ask: Ask {
            cmd: "cargo run --example relay-soak -- --warp 9.6 --minutes 10",
            why: "Soak-test the relay at warp 9.6 for ten minutes",
            family: "cargo run --example relay-soak",
            out: "warp 9.6 · 10m · 5,766,208 frames · loss 0.000%",
        },
        done: "Soaked at warp 9.6 for ten minutes: 5,766,208 frames, none lost. Ready to ship.",
    },
    Task {
        title: "Shipping the subspace relay",
        verb: "Shipping",
        prompt: "ship the relay's error correction to the fleet's relays",
        steps: &[
            st(
                K::Run,
                "cargo build --release",
                "Finished `release` profile [optimized] in 21.4s",
            ),
            st(K::Read, "deploy/relays.toml", "Read 42 lines"),
            st(
                K::Edit,
                "CHANGELOG.md",
                "Updated CHANGELOG.md with 6 additions",
            ),
            st(
                K::Run,
                "cargo test",
                "test result: ok. 112 passed; 0 failed",
            ),
        ],
        ask: Ask {
            cmd: "./deploy/relays.sh --fleet --canary 2",
            why: "Roll the relay out to every ship, two canaries first",
            family: "./deploy/relays.sh",
            out: "canaries ok · 46/46 relays updated · loss 0.000%",
        },
        done: "Shipped to 46 relays, two canaries first; loss stays at 0.000% fleet-wide.",
    },
];

/// The big task the finisher finishes.
pub const BIG: Task = Task {
    title: "Building Earth Mk II, continent by continent",
    verb: "Building",
    prompt: "build Earth Mk II from the original plans — every continent, every fjord, and Norway crinklier",
    steps: &[
        st(K::Read, "plans/earth-mk1.toml", "Read 4,118 lines"),
        st(K::Run, "python -m gen.continents --all", "7 continents · 1,904 fjords · 37,000 islands"),
        st(K::Edit, "gen/fjords.py", "Updated gen/fjords.py with 31 additions and 9 removals"),
        st(K::Run, "pytest -q", "212 passed in 41.80s"),
    ],
    ask: Ask {
        cmd: "python -m gen.render --gpus 0,1 --planet earth-mk2",
        why: "Render the whole planet on both GPUs",
        family: "python -m gen.render",
        out: "rendered 510,072,000 km² in 6m 41s → out/earth-mk2/",
    },
    done: "Earth Mk II is built: 7 continents, 1,904 fjords, 510 million km² rendered on both GPUs. 14 files, +2,318 −407; all 212 tests pass.",
};

/// The question the asker asks in cycle `k`.
pub fn ask_task(k: u64) -> &'static Task {
    &ASKS[(k % 2) as usize]
}

// ------------------------------------------------------------------ the beats

fn at(phase: Phase, into: u64, len: u64, k: u64, i: u64, done: bool) -> At {
    At {
        phase,
        into,
        len,
        seq: 5 * k + i,
        done_seq: k + u64::from(done),
        round: k,
    }
}

/// The asker at `t`: works, asks, and — once answered — works on, finishes
/// and rests; unanswered, it asks until the cycle ends.
pub fn ask(s: &Story, t: u64, approvals: &[u64]) -> At {
    let (k, p) = s.place(t);
    let start = t - p;
    let a0 = s.ask_at();
    if p < a0 {
        return at(Phase::Work, p, a0, k, 1, false);
    }
    let Some(&a) = approvals.iter().rev().find(|&&a| a >= start + a0 && a <= t) else {
        return at(Phase::Ask, p - a0, s.every() - a0, k, 2, false);
    };
    let el = t - a;
    let (w, d) = (s.after_for(), s.done_for());
    if el < w {
        at(Phase::After, el, w, k, 3, false)
    } else if el < w + d {
        at(Phase::Done, el - w, d, k, 4, true)
    } else {
        at(Phase::Idle, el - w - d, s.every(), k, 5, true)
    }
}

/// The finisher at `t`: the big task, worked on all cycle, done near its end.
pub fn finish(s: &Story, t: u64) -> At {
    let (k, p) = s.place(t);
    let f = s.finish_at();
    if p < f {
        at(Phase::Work, p, f, k, 1, false)
    } else {
        at(Phase::Done, p - f, s.every() - f, k, 4, true)
    }
}

/// The limit at `t`: Some((seconds into it, how long it lasts)) while it's
/// hit.
pub fn limit_at(s: &Story, t: u64) -> Option<(u64, u64)> {
    if !s.has(LIMIT) {
        return None;
    }
    let (_, p) = s.place(t);
    let (l0, l1) = s.limit_window();
    (l0..l1).contains(&p).then(|| (p - l0, l1 - l0))
}

/// The 5-hour window's use at `t` (the limit's machine): full while it's
/// hit, else filling from the reset towards the next hit. (used %, seconds
/// to its reset — the hit's end while hit, else the next one's)
pub fn window(s: &Story, t: u64) -> (f64, u64) {
    let (_, p) = s.place(t);
    let (l0, l1) = s.limit_window();
    let e = s.every();
    if (l0..l1).contains(&p) {
        return (100.0, l1 - p);
    }
    // since the last reset, out of the time to the next hit
    let since = if p >= l1 { p - l1 } else { p + e - l1 };
    let span = e - (l1 - l0);
    let used = (5.0 + 90.0 * since as f64 / span.max(1) as f64).min(99.0);
    let next = if p < l0 { l1 - p } else { e - p + l1 };
    (used.round(), next)
}

/// Is `host`'s herdr stopped at `t`? From the drop's start until the hub
/// healed it (and herdr took `RESTART` to come back), or the window's end.
pub fn stopped(s: &Story, host: &str, t: u64, actions: &[&Action]) -> bool {
    if !s.has(DROP) || host != DROPPER {
        return false;
    }
    let (_, p) = s.place(t);
    let (d0, d1) = s.drop_window();
    if !(d0..d1).contains(&p) {
        return false;
    }
    let from = t - p + d0;
    let healed = actions
        .iter()
        .filter(|a| a.host == host && a.act == Act::Heal && a.t >= from && a.t <= t)
        .map(|a| a.t)
        .min();
    healed.is_none_or(|h| t < h + RESTART)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = ORIGIN + 1_000 * 120;

    #[test]
    fn the_beats_are_in_order_and_fit_the_cycle() {
        for every in [60, 120, 300, 600, 3600] {
            let s = Story { every, beats: ALL };
            let (d0, d1) = s.drop_window();
            let (l0, l1) = s.limit_window();
            assert!(s.ask_at() < d0, "{every}");
            assert!(d0 < d1 && l0 < l1 && l1 < every, "{every}");
            assert!(s.finish_at() < every && s.finish_at() > d1, "{every}");
            assert!(l1 - l0 >= 29 && d1 - d0 >= 20, "{every}");
            // they start in order
            let starts: Vec<u64> = s.plan().iter().map(|b| b.1).collect();
            assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{every}");
        }
        // a cycle shorter than a minute is a minute
        let s = Story {
            every: 5,
            beats: ALL,
        };
        assert_eq!(s.place(ORIGIN + 61), (1, 1));
    }

    #[test]
    fn the_asker_waits_unless_answered() {
        let s = Story::default();
        let a0 = s.ask_at();
        assert_eq!(ask(&s, T, &[]).phase, Phase::Work);
        assert_eq!(ask(&s, T + a0, &[]).phase, Phase::Ask);
        // nobody answers: it asks until the cycle ends
        assert_eq!(ask(&s, T + 119, &[]).phase, Phase::Ask);
        assert_eq!(ask(&s, T + 120, &[]).phase, Phase::Work);
        // answered: works on, then finishes, then rests
        let ans = [T + a0 + 2];
        assert_eq!(ask(&s, T + a0 + 3, &ans).phase, Phase::After);
        let done = T + a0 + 2 + s.after_for();
        assert_eq!(ask(&s, done, &ans).phase, Phase::Done);
        assert_eq!(ask(&s, done + s.done_for(), &ans).phase, Phase::Idle);
        // an answer from the cycle before doesn't count
        assert_eq!(ask(&s, T + 120 + a0 + 1, &ans).phase, Phase::Ask);
        // the seqs move with each phase (herdr's state_change_seq)
        let seqs: Vec<u64> = [T, T + a0, T + a0 + 3, done, done + s.done_for()]
            .iter()
            .map(|t| ask(&s, *t, &ans).seq)
            .collect();
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    }

    #[test]
    fn only_the_soak_test_matches_the_worlds_rule() {
        let p = RULE.1;
        assert!(crate::auto::pmatch(p, ask_task(0).ask.cmd));
        assert!(!crate::auto::pmatch(p, ask_task(1).ask.cmd));
        assert!(!crate::auto::pmatch(p, BIG.ask.cmd));
        for t in super::super::world::TASKS {
            assert!(!crate::auto::pmatch(p, t.ask.cmd), "{}", t.title);
        }
        // and the deny list never stops it, nor the rollout
        let cfg = crate::config::Config::in_memory(&[]);
        for t in &ASKS {
            assert!(
                !crate::auto::deny_list(&cfg)
                    .iter()
                    .any(|d| crate::auto::pmatch(d, t.ask.cmd)),
                "{}",
                t.ask.cmd
            );
        }
    }

    #[test]
    fn the_drop_lasts_until_healed() {
        let s = Story::default();
        let (d0, d1) = s.drop_window();
        assert!(!stopped(&s, DROPPER, T + d0 - 1, &[]));
        assert!(stopped(&s, DROPPER, T + d0, &[]));
        assert!(stopped(&s, DROPPER, T + d1 - 1, &[]));
        assert!(!stopped(&s, DROPPER, T + d1, &[]));
        assert!(!stopped(&s, "europa", T + d0, &[]));
        let heal = Action {
            t: T + d0 + 2,
            host: DROPPER.into(),
            pane: String::new(),
            act: Act::Heal,
        };
        let acts = [&heal];
        assert!(stopped(&s, DROPPER, T + d0 + 2 + RESTART - 1, &acts));
        assert!(!stopped(&s, DROPPER, T + d0 + 2 + RESTART, &acts));
        // a heal from the cycle before doesn't count
        let next = T + 120 + d0;
        assert!(stopped(&s, DROPPER, next + RESTART, &acts));
        // off: never
        let off = Story {
            beats: ASK,
            ..Story::default()
        };
        assert!(!stopped(&off, DROPPER, T + d0, &[]));
    }

    #[test]
    fn the_limit_fills_hits_and_resets() {
        let s = Story::default();
        let (l0, l1) = s.limit_window();
        assert_eq!(limit_at(&s, T + l0), Some((0, l1 - l0)));
        assert_eq!(limit_at(&s, T + l1), None);
        assert_eq!(window(&s, T + l0 + 1), (100.0, l1 - l0 - 1));
        let (just, _) = window(&s, T + l1);
        let (later, next) = window(&s, T + l0 - 1);
        assert!(
            just <= 6.0 && later > 80.0 && later < 100.0,
            "{just} {later}"
        );
        assert_eq!(next, l1 - l0 + 1);
        assert_eq!(finish(&s, T + s.finish_at()).phase, Phase::Done);
        assert_eq!(finish(&s, T + s.finish_at() - 1).phase, Phase::Work);
        let no = Story {
            beats: ASK,
            ..Story::default()
        };
        assert_eq!(limit_at(&no, T + l0), None);
    }

    #[test]
    fn the_settings_pick_the_story() {
        let c = |l: &[&str]| of(&crate::config::Config::in_memory(l));
        assert_eq!(c(&[]), Some(Story::default()));
        assert_eq!(c(&["showcase.story=off"]), None);
        assert_eq!(
            c(&[
                "showcase.story_every=5m",
                "showcase.story_beats=ask, finish"
            ]),
            Some(Story {
                every: 300,
                beats: ASK | FINISH
            })
        );
        assert_eq!(c(&["showcase.story_every=10s"]).unwrap().every, MIN_EVERY);
        assert_eq!(c(&["showcase.story_beats="]).map(|s| s.beats), Some(ALL));
        assert_eq!(c(&["showcase.story_beats=off"]), None);
        assert_eq!(parse_beats("all"), ALL);
        let s = Story::default();
        assert_eq!(s.beat_now(T + s.ask_at()).map(|b| b.0), Some("ask"));
        assert_eq!(s.beat_now(T + s.drop_window().0).map(|b| b.0), Some("drop"));
        assert_eq!(
            s.beat_now(T + s.limit_window().0).map(|b| b.0),
            Some("limit")
        );
        assert_eq!(s.beat_now(T + s.finish_at()).map(|b| b.0), Some("finish"));
        assert_eq!(s.beat_now(T), None);
        let only = Story { beats: DROP, ..s };
        assert_eq!(only.plan().len(), 1);
        assert_eq!(only.beat_now(T + s.ask_at()), None);
    }
}
