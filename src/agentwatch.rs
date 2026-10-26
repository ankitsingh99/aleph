//! Which agents each machine has, kept fresh in the background for the
//! dashboard: [`AgentWatch::start`] with the fleet, then read
//! [`AgentWatch::snapshot`] / [`AgentWatch::agent`] from the UI thread — they
//! only lock a map, never touch the network.
//!
//! * One probe per machine ([`probe::run`]: one ssh round trip), each on its
//!   own thread, so a slow or dead machine never holds up the others.
//! * The on-disk answer ([`probe::cached`]) shows at once — even an old one,
//!   marked by its age — and is looked at again once it's older than
//!   [`probe::TTL`]. A newer answer written by another process (`claude
//!   agents install` in the terminal, `aleph agents --refresh`) is picked up
//!   within a couple of seconds.
//! * Dropping the last handle stops the watch and kills probes in flight.
//!
//! [`badges`] renders a machine's agents as brand glyphs.
#![allow(dead_code)] // start/badges/refresh are wired into the dashboard by its owner

use crate::icons;
use crate::probe::{self, Found, HostProbe, IState};
use ratatui::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, SystemTime};

/// An answer older than this isn't shown at all (a re-look starts instead).
const SHOW_FOR: Duration = Duration::from_secs(7 * 86400);
/// How often the watch checks for expired or newer answers.
const TICK: Duration = Duration::from_secs(2);
/// A machine that didn't answer is asked again after this.
const RETRY: Duration = Duration::from_secs(60);

/// What the watch knows about a machine.
#[derive(Debug, Clone)]
pub enum Status {
    /// not watched
    Unknown,
    /// a first look is under way
    Checking,
    /// an answer (`refreshing`: a newer look is under way)
    Ready {
        probe: Arc<HostProbe>,
        refreshing: bool,
    },
    /// couldn't look, and there's no earlier answer
    Failed(String),
}

/// Whether one agent is on one machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Avail {
    /// still looking
    Checking,
    Installed(Found),
    /// not there (`broken`: an install that doesn't run, and why)
    Missing {
        broken: Option<String>,
    },
    /// can't tell (why)
    Unknown(String),
}

#[derive(Default)]
struct Entry {
    probe: Option<Arc<HostProbe>>,
    err: Option<String>,
    busy: bool,
    /// when `probe` was taken
    at: Option<SystemTime>,
}

struct Inner {
    hosts: Mutex<HashMap<String, Entry>>,
    cancel: AtomicBool,
    gen: AtomicU64,
    /// never looks by itself ([`AgentWatch::fixed`])
    fixed: bool,
}

/// Lives as long as some [`AgentWatch`] does; stops everything when dropped.
struct Life {
    inner: Arc<Inner>,
}

impl Drop for Life {
    fn drop(&mut self) {
        self.inner.cancel.store(true, Ordering::Relaxed);
    }
}

/// A handle on the watch (cheap to clone; the watch stops with the last one).
#[derive(Clone)]
pub struct AgentWatch {
    life: Arc<Life>,
}

static CURRENT: OnceLock<Mutex<Weak<Life>>> = OnceLock::new();

impl AgentWatch {
    /// Watch `hosts` (more can be added with [`ensure`](Self::ensure)), and
    /// make this the watch [`current`](Self::current) returns.
    pub fn start(hosts: &[String]) -> AgentWatch {
        let w = AgentWatch::standalone(hosts);
        *CURRENT
            .get_or_init(|| Mutex::new(Weak::new()))
            .lock()
            .unwrap() = Arc::downgrade(&w.life);
        w
    }

    /// A watch of its own, not shared through [`current`](Self::current).
    pub fn standalone(hosts: &[String]) -> AgentWatch {
        let inner = Arc::new(Inner {
            hosts: Mutex::new(HashMap::new()),
            cancel: AtomicBool::new(false),
            gen: AtomicU64::new(0),
            fixed: false,
        });
        let weak = Arc::downgrade(&inner);
        thread::spawn(move || ticker(weak));
        let w = AgentWatch {
            life: Arc::new(Life { inner }),
        };
        for h in hosts {
            w.ensure(h);
        }
        w
    }

    /// A watch that knows only these answers and never looks by itself
    /// (tests, previews); `pending` hosts show as still checking.
    pub fn fixed(probes: Vec<HostProbe>, pending: &[&str]) -> AgentWatch {
        let mut m = HashMap::new();
        for p in probes {
            m.insert(
                p.host.clone(),
                Entry {
                    probe: Some(Arc::new(p)),
                    at: Some(SystemTime::now()),
                    ..Default::default()
                },
            );
        }
        for h in pending {
            m.insert(
                h.to_string(),
                Entry {
                    busy: true,
                    ..Default::default()
                },
            );
        }
        AgentWatch {
            life: Arc::new(Life {
                inner: Arc::new(Inner {
                    hosts: Mutex::new(m),
                    cancel: AtomicBool::new(false),
                    gen: AtomicU64::new(0),
                    fixed: true,
                }),
            }),
        }
    }

    /// The running watch the dashboard started, if any.
    pub fn current() -> Option<AgentWatch> {
        CURRENT
            .get()?
            .lock()
            .unwrap()
            .upgrade()
            .map(|life| AgentWatch { life })
    }

    fn inner(&self) -> &Arc<Inner> {
        &self.life.inner
    }

    /// Start watching `host` (no-op when it already is).
    pub fn ensure(&self, host: &str) {
        let fixed = self.inner().fixed;
        let new = {
            let mut m = self.inner().hosts.lock().unwrap();
            if m.contains_key(host) {
                false
            } else {
                m.insert(
                    host.to_string(),
                    Entry {
                        busy: !fixed,
                        err: fixed.then(|| "not checked".to_string()),
                        ..Default::default()
                    },
                );
                !fixed
            }
        };
        if new {
            let (inner, h) = (self.inner().clone(), host.to_string());
            thread::spawn(move || first_look(&inner, &h));
        }
    }

    /// Look at `host` again now, ignoring the cache (after an install).
    pub fn refresh(&self, host: &str) {
        self.ensure(host);
        let go = {
            let mut m = self.inner().hosts.lock().unwrap();
            match m.get_mut(host) {
                Some(e) if !e.busy && !self.inner().fixed => {
                    e.busy = true;
                    true
                }
                _ => false,
            }
        };
        if go {
            bump(self.inner());
            let (inner, h) = (self.inner().clone(), host.to_string());
            thread::spawn(move || look(&inner, &h));
        }
    }

    /// Take the answer on disk now (another process just looked: `aleph
    /// host setup`), or look again when there's none.
    pub fn reload(&self, host: &str) {
        self.ensure(host);
        if self.inner().fixed {
            return;
        }
        match probe::cached(host, probe::TTL) {
            Some(p) => {
                let at = cached_at(host);
                set(self.inner(), host, |e| {
                    e.probe = Some(Arc::new(p));
                    e.at = at;
                    e.err = None;
                });
            }
            None => self.refresh(host),
        }
    }

    pub fn status(&self, host: &str) -> Status {
        let m = self.inner().hosts.lock().unwrap();
        match m.get(host) {
            None => Status::Unknown,
            Some(Entry {
                probe: Some(p),
                busy,
                ..
            }) => Status::Ready {
                probe: p.clone(),
                refreshing: *busy,
            },
            Some(Entry { busy: true, .. }) => Status::Checking,
            Some(Entry { err, .. }) => Status::Failed(err.clone().unwrap_or_default()),
        }
    }

    /// The machine's last answer (None while the first look runs, or when
    /// it couldn't be reached and nothing is cached).
    pub fn probe(&self, host: &str) -> Option<Arc<HostProbe>> {
        match self.status(host) {
            Status::Ready { probe, .. } => Some(probe),
            _ => None,
        }
    }

    /// The agents installed on `host`, in registry order.
    pub fn snapshot(&self, host: &str) -> Option<Vec<Found>> {
        self.probe(host).map(|p| p.agents.clone())
    }

    /// Is agent `id` on `host`?
    pub fn agent(&self, host: &str, id: &str) -> Avail {
        match self.status(host) {
            Status::Unknown => Avail::Unknown("not checked".into()),
            Status::Checking => Avail::Checking,
            Status::Failed(e) => Avail::Unknown(e),
            Status::Ready { probe, .. } => avail(&probe, id),
        }
    }

    /// Changes whenever any answer does (redraw when it moves).
    pub fn generation(&self) -> u64 {
        self.inner().gen.load(Ordering::Relaxed)
    }

    /// Stop watching now (also what dropping the last handle does).
    pub fn stop(&self) {
        self.inner().cancel.store(true, Ordering::Relaxed);
    }
}

/// [`AgentWatch::agent`] for one answer.
pub fn avail(p: &HostProbe, id: &str) -> Avail {
    let id = crate::agents::find(id).map_or(id, |a| a.id);
    if let Some(f) = p.agent(id) {
        return Avail::Installed(f.clone());
    }
    Avail::Missing {
        broken: p
            .skipped
            .iter()
            .find(|s| s.id == id && s.broken)
            .map(|s| s.why.clone()),
    }
}

/// When the cached answer for `host` was written.
fn cached_at(host: &str) -> Option<SystemTime> {
    std::fs::metadata(probe::cache_file(host))
        .and_then(|m| m.modified())
        .ok()
}

fn bump(inner: &Inner) {
    inner.gen.fetch_add(1, Ordering::Relaxed);
}

fn set(inner: &Inner, host: &str, f: impl FnOnce(&mut Entry)) {
    if let Some(e) = inner.hosts.lock().unwrap().get_mut(host) {
        f(e);
    }
    bump(inner);
}

/// The cache first (shown even when old), then a real look when it's stale.
fn first_look(inner: &Arc<Inner>, host: &str) {
    if let Some(p) = probe::cached(host, SHOW_FOR) {
        let age = p.age.unwrap_or_default();
        let at = SystemTime::now().checked_sub(age);
        let fresh = age <= probe::TTL;
        set(inner, host, |e| {
            e.probe = Some(Arc::new(p));
            e.at = at;
            e.busy = !fresh;
        });
        if fresh {
            return;
        }
    }
    look(inner, host);
}

fn look(inner: &Arc<Inner>, host: &str) {
    if inner.cancel.load(Ordering::Relaxed) {
        return;
    }
    let p = probe::run_cancel(host, Some(&inner.cancel));
    if inner.cancel.load(Ordering::Relaxed) {
        return;
    }
    set(inner, host, |e| {
        e.busy = false;
        if p.ok() {
            e.probe = Some(Arc::new(p));
            e.at = Some(SystemTime::now());
            e.err = None;
        } else {
            // keep showing the last good answer; try again in a minute
            e.err = p.error.clone();
            e.at = SystemTime::now().checked_sub(probe::TTL.saturating_sub(RETRY));
        }
    });
}

/// Every [`TICK`]: re-look at expired answers, pick up newer cache files.
fn ticker(weak: Weak<Inner>) {
    loop {
        thread::sleep(TICK);
        let Some(inner) = weak.upgrade() else { return };
        if inner.cancel.load(Ordering::Relaxed) {
            return;
        }
        let mut relook = Vec::new();
        let mut reload = Vec::new();
        {
            let mut m = inner.hosts.lock().unwrap();
            for (h, e) in m.iter_mut() {
                if e.busy {
                    continue;
                }
                let disk = cached_at(h);
                if let (Some(d), Some(at)) = (disk, e.at) {
                    if d > at + Duration::from_millis(500) {
                        reload.push(h.clone());
                        continue;
                    }
                }
                let old =
                    e.at.and_then(|t| t.elapsed().ok())
                        .is_none_or(|a| a > probe::TTL);
                if old {
                    e.busy = true;
                    relook.push(h.clone());
                }
            }
        }
        for h in reload {
            if let Some(p) = probe::cached(&h, SHOW_FOR) {
                let at = cached_at(&h);
                set(&inner, &h, |e| {
                    e.probe = Some(Arc::new(p));
                    e.at = at;
                    e.err = None;
                });
            }
        }
        for h in relook {
            let i = inner.clone();
            thread::spawn(move || look(&i, &h));
        }
    }
}

// ------------------------------------------------------------------ drawing

/// A machine's installed agents as brand glyphs (`✻ ◎ ⌬`), in registry
/// order; an agent whose herdr integration exists but isn't installed is
/// dimmed (herdr won't restore its sessions).
pub fn badges(found: &[Found]) -> Vec<Span<'static>> {
    let mut v: Vec<&Found> = found.iter().collect();
    v.sort_by_key(|f| {
        crate::agents::ALL
            .iter()
            .position(|a| a.id == f.id)
            .unwrap_or(usize::MAX)
    });
    let mut out = Vec::new();
    for f in v {
        if !out.is_empty() {
            out.push(Span::raw(" "));
        }
        let mut s = icons::agent(&f.id);
        if f.integration == Some(IState::Missing) {
            s.style = s
                .style
                .remove_modifier(Modifier::BOLD)
                .add_modifier(Modifier::DIM);
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Check;

    fn found(id: &str, integ: Option<IState>) -> Found {
        Found {
            id: id.into(),
            path: format!("/b/{id}"),
            version: "1.0".into(),
            check: Check::Plausible,
            on_path: Some(true),
            shadowed_by: None,
            integration: integ,
        }
    }

    #[test]
    fn badges_in_registry_order_dim_without_integration() {
        let v = badges(&[
            found("opencode", Some(IState::Missing)),
            found("claude", Some(IState::Current("v11".into()))),
            found("crush", None),
        ]);
        let text: String = v.iter().map(|s| s.content.as_ref()).collect();
        let (c, _) = icons::brand("claude");
        let (o, _) = icons::brand("opencode");
        let (k, _) = icons::brand("crush");
        assert_eq!(text, format!("{c} {o} {k}"));
        assert!(!v[0].style.add_modifier.contains(Modifier::DIM));
        assert!(v[2].style.add_modifier.contains(Modifier::DIM));
        // no integration in this herdr at all: not dimmed
        assert!(!v[4].style.add_modifier.contains(Modifier::DIM));
        assert!(badges(&[]).is_empty());
    }

    #[test]
    fn availability() {
        let p = probe::parse(
            "b31",
            "CCC\t1\tcodex\t/b/codex\t0\t0\tcodex-cli 0.46.0\n\
             CCC\t2\tgemini\t/b/gemini\t127\t0\tenv: node: No such file or directory\n\
             CCO\tLinux\n",
        );
        assert!(matches!(avail(&p, "codex"), Avail::Installed(f) if f.version == "0.46.0"));
        assert!(matches!(avail(&p, "Codex"), Avail::Installed(_)));
        assert_eq!(avail(&p, "claude"), Avail::Missing { broken: None });
        assert!(
            matches!(avail(&p, "gemini"), Avail::Missing { broken: Some(w) } if w.contains("node"))
        );
    }

    #[test]
    fn watch_answers_off_thread_and_stops() {
        // ssh that fails at once: the watch must report it, not hang
        std::env::set_var("CC_SSH_BIN", "/usr/bin/false");
        let w = AgentWatch::standalone(&["cch-test-nohost".to_string()]);
        let g0 = w.generation();
        let t = std::time::Instant::now();
        while matches!(w.status("cch-test-nohost"), Status::Checking)
            && t.elapsed() < Duration::from_secs(20)
        {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(matches!(w.status("cch-test-nohost"), Status::Failed(_)));
        assert!(w.generation() > g0);
        assert!(w.snapshot("cch-test-nohost").is_none());
        assert!(matches!(
            w.agent("cch-test-nohost", "claude"),
            Avail::Unknown(_)
        ));
        assert!(matches!(w.status("elsewhere"), Status::Unknown));
        let inner = w.inner().clone();
        drop(w);
        assert!(inner.cancel.load(Ordering::Relaxed), "dropping stops it");
        // a fixed watch answers what it was given and never looks
        let p = probe::parse(
            "b31",
            "CCC\t1\tcodex\t/b/codex\t0\t0\tcodex-cli 0.46.0\nCCO\tLinux\n",
        );
        let f = AgentWatch::fixed(vec![p], &["gpu2"]);
        assert!(matches!(f.agent("b31", "codex"), Avail::Installed(_)));
        assert_eq!(f.agent("gpu2", "codex"), Avail::Checking);
        f.ensure("other");
        f.refresh("b31");
        assert!(matches!(f.status("other"), Status::Failed(_)));
        assert!(matches!(
            f.status("b31"),
            Status::Ready {
                refreshing: false,
                ..
            }
        ));
        // current(): only a started watch
        let s = AgentWatch::start(&[]);
        assert!(AgentWatch::current().is_some());
        drop(s);
        assert!(AgentWatch::current().is_none());
    }
}
