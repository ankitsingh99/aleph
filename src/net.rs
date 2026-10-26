//! Is this computer online? One cheap probe on its own thread: a TCP connect
//! to `net.probe` (1.1.1.1:443 by default; no DNS) every few seconds with a
//! 1.5 s deadline. When it fails, the default route tells "no network" (Wi-Fi
//! off, cable out) apart from "the internet is down"; the machines' own polls
//! add their round trips. The dashboard shows the result in its header —
//! `● online 32 ms` · `◐ weak 480 ms` · `○ offline 14s` · `⟳ reconnecting` —
//! and toasts only when it changes.

use std::collections::VecDeque;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Probes kept for loss and round-trip figures.
const WINDOW: usize = 10;
/// Slower than this (median) is a weak link.
pub const WEAK_MS: u128 = 250;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// no probe has answered or failed yet
    Unknown,
    Online {
        ms: u128,
    },
    /// slow or lossy: (median round trip, share of probes lost in %)
    Weak {
        ms: u128,
        loss: u8,
    },
    /// `route`: a default route exists (so it's the internet, not the link)
    Offline {
        since: Instant,
        route: bool,
    },
    /// back online, while the machines catch up
    Reconnecting {
        since: Instant,
    },
}

impl State {
    pub fn is_offline(&self) -> bool {
        matches!(self, State::Offline { .. })
    }
}

#[derive(Default)]
struct Inner {
    /// newest last: Some(rtt) answered, None failed
    probes: VecDeque<Option<Duration>>,
    offline_since: Option<Instant>,
    back_at: Option<Instant>,
    route: bool,
}

impl Inner {
    fn state(&self) -> State {
        if self.probes.is_empty() {
            return State::Unknown;
        }
        if let Some(since) = self.offline_since {
            return State::Offline {
                since,
                route: self.route,
            };
        }
        if let Some(t) = self.back_at {
            if t.elapsed() < Duration::from_secs(20) {
                return State::Reconnecting { since: t };
            }
        }
        let mut rtts: Vec<u128> = self
            .probes
            .iter()
            .flatten()
            .map(|d| d.as_millis())
            .collect();
        rtts.sort_unstable();
        let ms = rtts.get(rtts.len() / 2).copied().unwrap_or(0);
        let lost = self.probes.iter().filter(|p| p.is_none()).count();
        let loss = (lost * 100 / self.probes.len()) as u8;
        if ms > WEAK_MS || loss >= 20 {
            State::Weak { ms, loss }
        } else {
            State::Online { ms }
        }
    }

    /// One probe's outcome. Two failures in a row mean offline (one alone
    /// is a blip); an answer after that means back.
    fn push(&mut self, p: Option<Duration>, route: impl FnOnce() -> bool) {
        self.probes.push_back(p);
        while self.probes.len() > WINDOW {
            self.probes.pop_front();
        }
        match p {
            Some(_) => {
                if self.offline_since.take().is_some() {
                    self.back_at = Some(Instant::now());
                }
            }
            None => {
                let n = self.probes.len();
                let twice = n >= 2 && self.probes[n - 2].is_none();
                if (twice || n == 1) && self.offline_since.is_none() {
                    self.offline_since = Some(Instant::now());
                    self.route = route();
                }
            }
        }
    }
}

/// The probe thread; dropping it stops the thread.
pub struct Net {
    inner: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
    /// fast while someone looks (the dashboard), slow in the daemon
    every: Arc<Mutex<Duration>>,
}

impl Drop for Net {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Where to probe: `net.probe` (host:port), else Cloudflare's resolver.
pub fn target(cfg: &crate::config::Config) -> String {
    cfg.get("net.probe")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "1.1.1.1:443".into())
}

fn addr(t: &str) -> Option<SocketAddr> {
    t.parse::<SocketAddr>()
        .ok()
        .or_else(|| t.to_socket_addrs().ok()?.next())
}

/// Connect once (no data sent): its round trip, or None.
pub fn probe(t: &str, timeout: Duration) -> Option<Duration> {
    let a = addr(t)?;
    let t0 = Instant::now();
    TcpStream::connect_timeout(&a, timeout).ok()?;
    Some(t0.elapsed())
}

/// Is there a default route at all? (No: Wi-Fi off, cable out.)
pub fn default_route() -> bool {
    let o = if cfg!(target_os = "macos") {
        crate::proc::cap("route", &["-n", "get", "default"], Duration::from_secs(2))
    } else {
        crate::proc::cap("ip", &["route", "show", "default"], Duration::from_secs(2))
    };
    o.ok() && (o.text().contains("gateway") || o.text().contains("default"))
}

impl Net {
    pub fn start(target: String, every: Duration) -> Net {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let every = Arc::new(Mutex::new(every));
        let (i2, s2, e2) = (inner.clone(), stop.clone(), every.clone());
        thread::spawn(move || {
            while !s2.load(Ordering::Relaxed) {
                let p = probe(&target, Duration::from_millis(1500));
                i2.lock().unwrap().push(p, default_route);
                let wait = *e2.lock().unwrap();
                let t0 = Instant::now();
                while t0.elapsed() < wait && !s2.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(200));
                }
            }
        });
        Net { inner, stop, every }
    }

    pub fn state(&self) -> State {
        self.inner.lock().unwrap().state()
    }

    pub fn set_every(&self, d: Duration) {
        *self.every.lock().unwrap() = d;
    }

    /// For tests: a probe result as if it had just come in.
    #[cfg(test)]
    pub fn feed(&self, p: Option<Duration>, route: bool) {
        self.inner.lock().unwrap().push(p, || route);
    }

    /// Never probes on its own (tests, setup's quiet mode).
    pub fn quiet() -> Net {
        Net {
            inner: Arc::new(Mutex::new(Inner::default())),
            stop: Arc::new(AtomicBool::new(true)),
            every: Arc::new(Mutex::new(Duration::from_secs(3600))),
        }
    }
}

/// This process's own probe, for what isn't a dashboard (the daemon's
/// outage rule): started once, every 15 s. Never in tests.
static GLOBAL: std::sync::OnceLock<Net> = std::sync::OnceLock::new();

pub fn start_global(target: &str) {
    if cfg!(test) {
        return;
    }
    GLOBAL.get_or_init(|| Net::start(target.to_string(), Duration::from_secs(15)));
}

/// Is this computer offline now, as far as its own probe knows? (No probe
/// yet: no.)
pub fn offline_now() -> bool {
    GLOBAL.get().is_some_and(|n| n.state().is_offline())
}

/// The header's words for a state: (glyph, text).
pub fn label(s: &State) -> (&'static str, String) {
    match s {
        State::Unknown => ("·", String::new()),
        State::Online { ms } => ("●", format!("online {ms} ms")),
        State::Weak { loss, .. } if *loss >= 20 => ("◐", format!("weak · {loss}% lost")),
        State::Weak { ms, .. } => ("◐", format!("weak {ms} ms")),
        State::Offline { since, route } => (
            "○",
            format!(
                "{} {}",
                if *route { "internet down" } else { "offline" },
                crate::ui::dur(since.elapsed().as_secs())
            ),
        ),
        State::Reconnecting { .. } => ("⟳", "reconnecting".into()),
    }
}

/// [`label`], the machines counted in: with the internet down, machines
/// that still answer (over a local network) make it "no internet", not
/// "offline". `up` of `total` (this computer aside).
pub fn label_with(s: &State, up: usize, total: usize) -> (&'static str, String) {
    match s {
        State::Offline { .. } if up > 0 => (
            "◐",
            format!(
                "no internet · {up}/{total} machine{} answer",
                if total == 1 { "" } else { "s" }
            ),
        ),
        _ => label(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_internet_with_machines_answering_is_not_offline() {
        let off = State::Offline {
            since: Instant::now(),
            route: true,
        };
        assert_eq!(
            label_with(&off, 2, 3).1,
            "no internet · 2/3 machines answer"
        );
        assert!(label_with(&off, 0, 3).1.starts_with("internet down"));
        let on = State::Online { ms: 20 };
        assert_eq!(label_with(&on, 3, 3), label(&on));
    }

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    #[test]
    fn states_from_probes() {
        let mut i = Inner::default();
        assert_eq!(i.state(), State::Unknown);
        i.push(ms(30), || true);
        i.push(ms(40), || true);
        assert!(matches!(i.state(), State::Online { ms: 30..=40 }));
        // slow: weak
        for _ in 0..5 {
            i.push(ms(600), || true);
        }
        assert!(matches!(i.state(), State::Weak { .. }));
        // one failure is a blip (but counts as loss); two are offline
        let mut i = Inner::default();
        i.push(ms(30), || true);
        i.push(None, || true);
        assert!(!i.state().is_offline());
        i.push(None, || false);
        assert!(matches!(i.state(), State::Offline { route: false, .. }));
        // back: reconnecting for a while
        i.push(ms(30), || true);
        assert!(matches!(i.state(), State::Reconnecting { .. }));
    }

    #[test]
    fn probe_a_closed_port_fails_fast() {
        // nothing listens on port 9 of localhost (discard): refused at once
        let t0 = Instant::now();
        assert!(probe("127.0.0.1:9", Duration::from_millis(500)).is_none());
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert!(probe("not a target", Duration::from_millis(100)).is_none());
    }
}
