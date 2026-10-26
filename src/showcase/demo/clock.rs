//! The demo world's clock (`showcase.warp`): the world's time runs `x` times
//! as fast as real time, from an anchor every process reads —
//! `<state>/showcase/clock.json`, `{real_ms, world_ms, x}` — so the daemon,
//! the dashboard and each CLI run tell the same time. Changing the warp
//! moves the anchor to that very millisecond (`rewarp`): the world only
//! changes speed, it never jumps. No file is the plain clock (the world's
//! time is the real time), so a fresh world (`--fresh` empties the sandbox)
//! starts on time again.
//!
//! What the world *does* runs on this clock (its sessions' scripts, the
//! story's beats, a machine's outage, the action log); what it *keeps* —
//! usage records, history, events — stays on real time.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// The speeds `showcase.warp` offers.
pub const WARPS: &[u64] = &[1, 2, 5, 10];
const SETTING: &str = "showcase.warp";
/// how often the file is looked at again (its mtime), and the setting
const LOOK: Duration = Duration::from_millis(500);
const CHECK: Duration = Duration::from_secs(2);

/// `world = world₀ + (real − real₀) · x`, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub real_ms: u64,
    pub world_ms: u64,
    pub x: u64,
}

impl Default for Anchor {
    /// the plain clock: the world's time is the real time
    fn default() -> Self {
        PLAIN
    }
}

const PLAIN: Anchor = Anchor {
    real_ms: 0,
    world_ms: 0,
    x: 1,
};

impl Anchor {
    /// The world's time (ms) at the real `real_ms`.
    pub fn at_ms(&self, real_ms: u64) -> u64 {
        self.world_ms + real_ms.saturating_sub(self.real_ms) * self.x.max(1)
    }
    /// The same clock, `x` times as fast from `real_ms` on (no jump).
    pub fn rewarp(&self, real_ms: u64, x: u64) -> Anchor {
        Anchor {
            real_ms,
            world_ms: self.at_ms(real_ms),
            x: x.max(1),
        }
    }
    fn to_json(self) -> String {
        serde_json::json!({"real_ms": self.real_ms, "world_ms": self.world_ms, "x": self.x})
            .to_string()
    }
    fn parse(t: &str) -> Option<Anchor> {
        let v: serde_json::Value = serde_json::from_str(t).ok()?;
        let n = |k: &str| v.get(k).and_then(serde_json::Value::as_u64);
        Some(Anchor {
            real_ms: n("real_ms")?,
            world_ms: n("world_ms")?,
            x: n("x")?.max(1),
        })
    }
}

/// `showcase.warp`'s value as a speed (`5`, `5x`, `×5`; anything else is 1).
pub fn parse_warp(s: &str) -> u64 {
    let t = s.trim().trim_matches(|c| c == 'x' || c == '×').trim();
    t.parse::<u64>()
        .ok()
        .filter(|x| WARPS.contains(x))
        .unwrap_or(1)
}

pub fn real_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn file() -> PathBuf {
    super::sandbox(crate::config::state_dir()).join("clock.json")
}

fn read() -> Anchor {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|t| Anchor::parse(&t))
        .unwrap_or_default()
}

fn stamp() -> Option<SystemTime> {
    std::fs::metadata(file()).and_then(|m| m.modified()).ok()
}

struct Cache {
    looked: Option<Instant>,
    checked: Option<Instant>,
    stamp: Option<SystemTime>,
    anchor: Anchor,
}

static C: Mutex<Cache> = Mutex::new(Cache {
    looked: None,
    checked: None,
    stamp: None,
    anchor: PLAIN,
});

thread_local! {
    /// tests: a clock of their own (the file is never read)
    static FORCED: std::cell::Cell<Option<Anchor>> = const { std::cell::Cell::new(None) };
}

/// Tests: run on this clock (None: the plain one).
#[cfg(test)]
pub fn force(a: Option<Anchor>) {
    FORCED.with(|f| f.set(a));
}

/// The anchor now: the file (looked at twice a second, read again when it
/// changed), moved to this moment when `showcase.warp` asks for another
/// speed (the setting checked every 2 s).
pub fn anchor() -> Anchor {
    if let Some(a) = FORCED.with(|f| f.get()) {
        return a;
    }
    if cfg!(test) {
        return PLAIN;
    }
    let check = {
        let mut c = C.lock().unwrap_or_else(|e| e.into_inner());
        if c.looked.is_none_or(|t| t.elapsed() >= LOOK) {
            c.looked = Some(Instant::now());
            let s = stamp();
            if s != c.stamp {
                c.stamp = s;
                c.anchor = read();
            }
        }
        let due = c.checked.is_none_or(|t| t.elapsed() >= CHECK);
        if due {
            c.checked = Some(Instant::now());
        }
        if !due {
            return c.anchor;
        }
        c.anchor
    };
    // (the settings read without the lock held)
    let want = parse_warp(&crate::config::Config::load().get_or(SETTING));
    if want == check.x {
        return check;
    }
    let Some(a) = rewarp(want) else {
        return check;
    };
    let mut c = C.lock().unwrap_or_else(|e| e.into_inner());
    c.anchor = a;
    c.stamp = stamp();
    a
}

/// Move the anchor to this moment at speed `x` — under a lock, from the
/// file as it is now, so two processes noticing together write one anchor
/// (the second finds the first's speed there and keeps it).
fn rewarp(x: u64) -> Option<Anchor> {
    let p = file();
    let dir = p.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("clock.lock"))
        .ok()?;
    let _ = lock.lock();
    let now = read();
    if now.x == x {
        return Some(now);
    }
    let a = now.rewarp(real_ms(), x);
    crate::config::write_atomic(&p, a.to_json().as_bytes()).ok()?;
    crate::log::info(
        "showcase",
        &format!("the demo world's clock runs {x}× from now"),
    );
    Some(a)
}

/// The world's time now, in seconds.
pub fn now() -> u64 {
    anchor().at_ms(real_ms()) / 1000
}

/// The speed the world runs at now.
pub fn warp() -> u64 {
    anchor().x.max(1)
}

/// The real time (unix seconds) when the world's clock reads `world` (what
/// clients show as a time: a limit's reset).
pub fn real_at(world: u64) -> u64 {
    let a = anchor();
    let r = real_ms();
    let now_w = a.at_ms(r);
    let w = world * 1000;
    let x = a.x.max(1);
    let real = if w >= now_w {
        r + (w - now_w).div_ceil(x)
    } else {
        r.saturating_sub((now_w - w) / x)
    };
    real / 1000
}

/// How many real seconds `world` seconds of the world's time take.
pub fn real_secs(world: u64) -> u64 {
    world.div_ceil(warp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plain_clock_is_real_time() {
        let a = Anchor::default();
        assert_eq!(a.at_ms(1_793_000_000_123), 1_793_000_000_123);
    }

    #[test]
    fn a_new_warp_changes_the_speed_not_the_time() {
        let a = Anchor::default();
        let t = 1_793_000_000_000;
        // mid-second: not a whole second's worth lost or gained
        let b = a.rewarp(t + 730, 10);
        assert_eq!(b.at_ms(t + 730), a.at_ms(t + 730));
        assert_eq!(b.at_ms(t + 1730), t + 730 + 10_000);
        // back to 1×: still no jump, the world stays ahead
        let c = b.rewarp(t + 6_000, 1);
        assert_eq!(c.at_ms(t + 6_000), b.at_ms(t + 6_000));
        assert_eq!(c.at_ms(t + 7_000), b.at_ms(t + 6_000) + 1_000);
        // continuous across every change: one millisecond either side
        for (from, to) in [(1, 2), (2, 10), (10, 5), (5, 1)] {
            let p = Anchor {
                real_ms: t,
                world_ms: t,
                x: from,
            };
            let at = t + 12_345;
            let q = p.rewarp(at, to);
            assert_eq!(q.at_ms(at), p.at_ms(at));
            assert_eq!(q.at_ms(at + 1) - p.at_ms(at - 1), from + to);
        }
        // what's written is what's read back
        assert_eq!(Anchor::parse(&c.to_json()), Some(c));
        assert_eq!(Anchor::parse("{}"), None);
    }

    #[test]
    fn the_setting_reads_as_a_speed() {
        assert_eq!(parse_warp("1"), 1);
        assert_eq!(parse_warp(" 5 "), 5);
        assert_eq!(parse_warp("10x"), 10);
        assert_eq!(parse_warp("×2"), 2);
        // only the offered speeds
        assert_eq!(parse_warp("3"), 1);
        assert_eq!(parse_warp("fast"), 1);
        assert_eq!(parse_warp(""), 1);
    }

    #[test]
    fn a_forced_clock_is_the_worlds() {
        let t = 1_793_000_000_000;
        force(Some(Anchor {
            real_ms: real_ms(),
            world_ms: t,
            x: 5,
        }));
        let w = now();
        assert!((t / 1000..t / 1000 + 5).contains(&w), "{w}");
        assert_eq!(warp(), 5);
        assert_eq!(real_secs(25), 5);
        // ten world seconds from now are two real ones away
        let r = real_at(w + 10);
        let real = real_ms() / 1000;
        assert!((real + 1..=real + 3).contains(&r), "{r} {real}");
        force(None);
        assert_eq!(warp(), 1);
    }
}
