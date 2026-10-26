//! What the daemon keeps for its clients (Raycast, the web console, the
//! dashboard), so none of them runs a probe or a scan of its own: the agents
//! on each machine (`AgentWatch`, re-looking past its 10 min TTL), the LLM
//! servers (`services::Watcher`s, kept while someone asks), the last Doctor
//! report and the daemon's own facts. Each answers at once from memory or
//! the disk cache and refreshes in the background; `stamp` tells a
//! subscriber which of them moved (`events.subscribe {topics}`).

use crate::agentwatch::AgentWatch;
use crate::config::Config;
use crate::probe::{self, HostProbe};
use crate::services;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

// ------------------------------------------------------------------ agents

fn watch() -> &'static AgentWatch {
    static W: OnceLock<AgentWatch> = OnceLock::new();
    W.get_or_init(|| AgentWatch::start(&[]))
}

/// `agents.list`: `aleph agents --json`'s shape for `hosts` (and this
/// computer with `local`). From the watch, else the disk cache (however
/// old: it says its age), else — the first time ever — a look now.
/// `refresh`: a fresh look at each, now.
pub fn agents(hosts: &[String], refresh: bool) -> Value {
    let w = watch();
    for h in hosts {
        w.ensure(h);
    }
    if refresh {
        let v = probe::run_all_cached(hosts, true);
        for h in hosts {
            w.reload(h);
        }
        return crate::agentcmd::to_json(&v);
    }
    let mut out: Vec<Option<HostProbe>> = hosts
        .iter()
        .map(|h| {
            w.probe(h)
                .map(|p| (*p).clone())
                .or_else(|| probe::cached(h, Duration::from_secs(30 * 86400)))
        })
        .collect();
    let missing: Vec<String> = hosts
        .iter()
        .zip(&out)
        .filter(|(_, p)| p.is_none())
        .map(|(h, _)| h.clone())
        .collect();
    if !missing.is_empty() {
        let mut got = probe::run_all_cached(&missing, false).into_iter();
        for p in out.iter_mut().filter(|p| p.is_none()) {
            *p = got.next();
        }
    }
    crate::agentcmd::to_json(&out.into_iter().flatten().collect::<Vec<_>>())
}

// ------------------------------------------------------------------ LLM servers

/// A machine's watcher and when someone last asked (it stops after `IDLE`).
struct Svc {
    w: services::Watcher,
    asked: Instant,
}

const IDLE: Duration = Duration::from_secs(600);

static SVCS: OnceLock<Mutex<HashMap<String, Svc>>> = OnceLock::new();

fn svc_map() -> &'static Mutex<HashMap<String, Svc>> {
    SVCS.get_or_init(|| {
        // the reaper: watchers nobody asked about for a while stop
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(30));
            if let Some(m) = SVCS.get() {
                m.lock().unwrap().retain(|_, s| s.asked.elapsed() < IDLE);
            }
        });
        Mutex::new(HashMap::new())
    })
}

/// `services.list`: `aleph services --json`'s shape for `hosts`. Each
/// machine gets a watcher (rescanning every `services_interval`) that starts
/// from its last scan on disk; a machine never scanned waits for its first
/// (a few seconds) once.
pub fn services(cfg: &Config, hosts: &[String]) -> Value {
    let on = services::enabled_services(cfg);
    let every = services::interval(cfg);
    let mut fresh = false;
    {
        let mut m = svc_map().lock().unwrap();
        for h in hosts {
            match m.get_mut(h) {
                Some(s) => s.asked = Instant::now(),
                None => {
                    fresh = true;
                    m.insert(
                        h.clone(),
                        Svc {
                            w: services::Watcher::start(h, every, on.clone()),
                            asked: Instant::now(),
                        },
                    );
                }
            }
        }
    }
    let scans = |m: &HashMap<String, Svc>| -> Vec<services::Scan> {
        hosts
            .iter()
            .filter_map(|h| m.get(h).map(|s| s.w.snapshot()))
            .collect()
    };
    if fresh {
        // never scanned (no cache): give the first scan a moment
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(8) {
            let pending = scans(&svc_map().lock().unwrap())
                .iter()
                .any(|s| s.at == 0 && s.busy);
            if !pending {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    let v = scans(&svc_map().lock().unwrap());
    json!({
        "generated_at": crate::cache::now(),
        "enabled": services::enabled(cfg),
        "interval": every.as_secs(),
        "services": on.iter().map(|s| s.id).collect::<Vec<_>>(),
        "hosts": v.iter().map(|s| s.to_json(&crate::theme::machine_hex(cfg, &s.alias))).collect::<Vec<_>>(),
    })
}

/// When the kept scans last changed (for subscribers).
fn services_stamp() -> u64 {
    SVCS.get()
        .map(|m| {
            m.lock()
                .unwrap()
                .values()
                .map(|s| s.w.snapshot().at)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

// ------------------------------------------------------------------ doctor

#[derive(Default)]
struct Doc {
    /// the report (possibly partial while `running`)
    report: Option<Value>,
    at: u64,
    running: bool,
    /// bumped on every change, for subscribers
    gen: u64,
}

fn docs() -> &'static Mutex<HashMap<String, Doc>> {
    static D: OnceLock<Mutex<HashMap<String, Doc>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `doctor.get`: the last report for `host` (every machine without),
/// with `at` and `running`. One older than `doctor.every` (or `refresh`)
/// starts a new run in the background; its sections arrive as they finish.
pub fn doctor(cfg: &Config, host: Option<&str>, refresh: bool) -> Value {
    let key = host.unwrap_or("").to_string();
    let every = crate::log::parse_duration(&cfg.get_or("doctor.every"))
        .unwrap_or(Duration::from_secs(1800))
        .as_secs();
    let start = {
        let mut m = docs().lock().unwrap();
        let d = m.entry(key.clone()).or_default();
        let old = crate::cache::now().saturating_sub(d.at) >= every;
        let go = !d.running && (refresh || d.report.is_none() || old);
        if go {
            d.running = true;
            d.gen += 1;
        }
        go
    };
    if start {
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            let parts = crate::doctor::parts(&cfg, (!key.is_empty()).then_some(key.as_str()));
            let (v, _) = crate::doctor::report_json(&cfg, &parts, &mut |partial| {
                let mut m = docs().lock().unwrap();
                let d = m.entry(key.clone()).or_default();
                // the first run shows its sections as they come; later ones
                // keep the last full report until theirs is done
                if d.report.is_none() || d.report.as_ref().is_some_and(|r| r["running"] == true) {
                    d.report = Some(partial.clone());
                    d.gen += 1;
                }
            });
            let mut m = docs().lock().unwrap();
            let d = m.entry(key).or_default();
            d.report = Some(v);
            d.at = crate::cache::now();
            d.running = false;
            d.gen += 1;
        });
    }
    let m = docs().lock().unwrap();
    let d = m.get(host.unwrap_or("")).unwrap();
    let mut v = d
        .report
        .clone()
        .unwrap_or_else(|| json!({"ok": 0, "warn": 0, "fail": 0, "sections": []}));
    v["at"] = json!(d.at);
    v["running"] = json!(d.running);
    v
}

fn doctor_stamp() -> u64 {
    docs().lock().unwrap().values().map(|d| d.gen).sum()
}

/// A machine changed (it came back, was set up): its next report is new.
pub fn forget_doctor(host: &str) {
    let mut m = docs().lock().unwrap();
    for k in [host, ""] {
        if let Some(d) = m.get_mut(k) {
            d.at = 0;
        }
    }
}

// ------------------------------------------------------------------ the daemon

/// `daemon.facts`: `aleph daemon status --json`'s shape, asked of the
/// service manager at most every 10 s.
pub fn facts(cfg: &Config) -> Value {
    static F: OnceLock<Mutex<Option<(Instant, Value)>>> = OnceLock::new();
    let m = F.get_or_init(|| Mutex::new(None));
    if let Some((t, v)) = &*m.lock().unwrap() {
        if t.elapsed() < Duration::from_secs(10) {
            return v.clone();
        }
    }
    let v = crate::daemonctl::status_json(&crate::daemonctl::gather(cfg));
    *m.lock().unwrap() = Some((Instant::now(), v.clone()));
    v
}

// ------------------------------------------------------------------ topics

/// What a subscriber can follow, besides the fleet (`state`).
pub const TOPICS: &[&str] = &[
    "inbox",
    "queue",
    "notes",
    "usage",
    "agents",
    "services",
    "doctor",
    "config",
    "history",
    // a showcase build's presenter sync: what the presenter's dashboard shows
    #[cfg(feature = "showcase")]
    "presenter",
];

fn mtime(p: &std::path::Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Where `topic` is now: it moved when this changes (files by their
/// modification time, kept answers by their generation).
pub fn stamp(topic: &str) -> u64 {
    let st = crate::config::state_dir();
    match topic {
        "inbox" => {
            mtime(&crate::events::file())
                ^ mtime(&st.join("inbox_seen")).rotate_left(1)
                ^ mtime(&st.join("inbox_dismissed")).rotate_left(2)
                ^ mtime(&st.join("inbox_marks")).rotate_left(3)
        }
        "queue" => mtime(&crate::queue::file()),
        "notes" => mtime(&crate::notes::file()),
        "history" => mtime(&crate::history::file()),
        "usage" => {
            mtime(&crate::usage::store::dir().join("now.json"))
                ^ mtime(&crate::usage::store::dir().join("limits.json")).rotate_left(1)
        }
        "agents" => watch().generation(),
        "services" => services_stamp(),
        "doctor" => doctor_stamp(),
        "config" => mtime(&Config::path()),
        // (a showcase build's own: `presenter`)
        _ => crate::showcase::topic_stamp(topic),
    }
}
