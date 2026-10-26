//! Whether showcase mode is on: `state/showcase.json` — `{on, demo, seed,
//! salt, epoch}` — which `aleph showcase on [--demo] | off` writes and every
//! process (the daemon, the dashboard, each CLI run) reads, looking at the
//! file at most twice a second.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    /// the real fleet, private data swapped at output
    Masked,
    /// the scripted world (masked too)
    Demo,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "real",
            Mode::Masked => "masked",
            Mode::Demo => "demo",
        }
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub mode: Mode,
    pub salt: Vec<u8>,
    pub epoch: u64,
    /// the demo world's seed: its sessions' timing and names follow it
    pub seed: u64,
    /// the pause (the panic key, `aleph showcase panic`): every window blank
    pub panic: bool,
}

/// The demo world's seed unless `--seed` says otherwise.
pub const SEED: u64 = 42;

impl Default for State {
    fn default() -> Self {
        State {
            mode: Mode::Off,
            salt: Vec::new(),
            epoch: 0,
            seed: SEED,
            panic: false,
        }
    }
}

pub fn file() -> PathBuf {
    crate::config::state_dir().join("showcase.json")
}

struct Cache {
    looked: Option<Instant>,
    stamp: Option<SystemTime>,
    st: State,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| {
        Mutex::new(Cache {
            looked: None,
            stamp: None,
            st: State::default(),
        })
    })
}

thread_local! {
    /// tests: a mode of their own (the file is never read)
    static FORCED: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// Tests: run with this state (None: back to off).
#[cfg(test)]
pub fn force(st: Option<State>) {
    FORCED.with(|f| *f.borrow_mut() = st);
}

fn parse(v: &Value) -> State {
    let hex = v.get("salt").and_then(Value::as_str).unwrap_or("");
    let salt: Vec<u8> = (0..hex.len() / 2)
        .filter_map(|i| u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok())
        .collect();
    let on = v.get("on").and_then(Value::as_bool).unwrap_or(false);
    let demo = v.get("demo").and_then(Value::as_bool).unwrap_or(false);
    State {
        mode: match (on, demo) {
            (false, _) => Mode::Off,
            (true, false) => Mode::Masked,
            (true, true) => Mode::Demo,
        },
        salt,
        epoch: v.get("epoch").and_then(Value::as_u64).unwrap_or(0),
        seed: v.get("seed").and_then(Value::as_u64).unwrap_or(SEED),
        panic: v.get("panic").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// `f` of the state now (the file looked at again past half a second).
fn with<R>(f: impl FnOnce(&State) -> R) -> R {
    let forced = FORCED.with(|x| {
        x.borrow()
            .as_ref()
            .map(|s| (s.mode, s.epoch, s.seed, s.panic))
    });
    if let Some((mode, epoch, seed, panic)) = forced {
        // (the salt is only ever read through `get`)
        return f(&State {
            mode,
            epoch,
            seed,
            salt: Vec::new(),
            panic,
        });
    }
    if cfg!(test) {
        return f(&State::default());
    }
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.looked.is_none_or(|t| t.elapsed().as_millis() > 500) {
        c.looked = Some(Instant::now());
        let p = file();
        let stamp = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        if stamp != c.stamp {
            c.stamp = stamp;
            c.st = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .map(|v| parse(&v))
                .unwrap_or_default();
        }
    }
    f(&c.st)
}

/// The state now, salt and all.
pub fn get() -> State {
    if let Some(st) = FORCED.with(|f| f.borrow().clone()) {
        return st;
    }
    with(State::clone)
}

/// The mode now (cheap: asked by every hook, often).
pub fn mode() -> Mode {
    with(|s| s.mode)
}

/// The demo world's seed now.
pub fn seed() -> u64 {
    with(|s| s.seed)
}

pub fn epoch() -> u64 {
    with(|s| s.epoch)
}

/// Is the pause on? (cheap: the panic key's hooks ask every frame)
pub fn panic() -> bool {
    with(|s| s.panic)
}

/// Switch: `fresh` draws a new salt (every name and folder gets another
/// stand-in), else the last one is kept; the epoch moves so every window —
/// the daemon, Raycast, the web console — redraws with it.
pub fn set(mode: Mode, fresh: bool) -> std::io::Result<State> {
    set_seeded(mode, None, fresh)
}

/// [`set`], with the demo world's seed (None: the one before, else [`SEED`]).
pub fn set_seeded(mode: Mode, seed: Option<u64>, fresh: bool) -> std::io::Result<State> {
    let old: Value = std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    let salt = match old.get("salt").and_then(Value::as_str) {
        Some(s) if s.len() >= 32 && !fresh => s.to_string(),
        _ => {
            let mut b = [0u8; 32];
            let ok = std::fs::File::open("/dev/urandom")
                .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
                .is_ok();
            if !ok {
                let t = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                b[..16].copy_from_slice(&t.to_le_bytes());
            }
            b.iter().map(|x| format!("{x:02x}")).collect()
        }
    };
    let epoch = old.get("epoch").and_then(Value::as_u64).unwrap_or(0) + 1;
    let seed = seed
        .or_else(|| old.get("seed").and_then(Value::as_u64))
        .unwrap_or(SEED);
    let mut v = json!({
        "on": mode != Mode::Off,
        "demo": mode == Mode::Demo,
        "seed": seed,
        "salt": salt,
        "epoch": epoch,
    });
    // what other parts keep in the file (the pause, …) stays as it was
    if let (Some(o), Some(n)) = (old.as_object(), v.as_object_mut()) {
        for (k, x) in o {
            n.entry(k.clone()).or_insert_with(|| x.clone());
        }
    }
    write(&v)
}

/// The file written, and this process seeing it at once.
fn write(v: &Value) -> std::io::Result<State> {
    let _ = std::fs::create_dir_all(crate::config::state_dir());
    crate::config::write_private(&file(), v.to_string().as_bytes())?;
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    c.looked = None;
    c.stamp = None;
    Ok(parse(v))
}

/// Change some of the file's fields (`f` gets them all; the mode, salt and
/// seed are best left alone), the epoch moved on so every window redraws:
/// the pause's switch.
pub fn update(f: impl FnOnce(&mut serde_json::Map<String, Value>)) -> std::io::Result<State> {
    let mut v: Value = std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let epoch = v.get("epoch").and_then(Value::as_u64).unwrap_or(0) + 1;
    if let Some(m) = v.as_object_mut() {
        f(m);
        m.insert("epoch".into(), json!(epoch));
    }
    write(&v)
}

/// New stand-ins for everything (the dashboard does it each time it starts
/// while showcase mode is on): the same mode, a new salt.
pub fn reshuffle() {
    let m = mode();
    if m != Mode::Off {
        let _ = set(m, true);
    }
}
