//! Live machine stats for the API (`stats.get`): a sampler per machine — the
//! telemetry stream the Monitor uses, one ssh `sh -s` loop — started when a
//! client first asks for that machine and stopped a minute after the last
//! ask. Raycast's Machines reads the numbers from memory this way instead of
//! starting an ssh round (`aleph stats --json`) every few seconds.

use crate::telemetry::{Stats, Telemetry};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// How often a sampler samples.
const EVERY: u64 = 3;
/// A sampler nobody asked of this long stops (its ssh stream goes).
const IDLE: Duration = Duration::from_secs(60);
/// The first ask for a machine waits this long for its first sample.
const FIRST: Duration = Duration::from_secs(5);

type Live = Mutex<HashMap<String, (Telemetry, Instant)>>;

fn live() -> &'static Live {
    static LIVE: OnceLock<Live> = OnceLock::new();
    LIVE.get_or_init(|| {
        // the reaper: samplers nobody asks of any more stop
        thread::spawn(|| loop {
            thread::sleep(Duration::from_secs(10));
            if let Some(m) = LIVE.get() {
                m.lock().unwrap().retain(|_, (_, at)| at.elapsed() < IDLE);
            }
        });
        Mutex::new(HashMap::new())
    })
}

/// `host`'s latest sample, starting its sampler if it isn't running (and
/// then waiting a little for the first sample).
pub fn get(host: &str) -> Stats {
    let started = {
        let mut m = live().lock().unwrap();
        match m.get_mut(host) {
            Some((_, at)) => {
                *at = Instant::now();
                false
            }
            None => {
                m.insert(
                    host.to_string(),
                    (Telemetry::start(host, EVERY), Instant::now()),
                );
                true
            }
        }
    };
    let snap = || {
        live()
            .lock()
            .unwrap()
            .get(host)
            .map(|(t, _)| t.snapshot())
            .unwrap_or_default()
    };
    if started {
        let t0 = Instant::now();
        while t0.elapsed() < FIRST {
            let s = snap();
            if s.samples > 0 || s.err.is_some() {
                return s;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    snap()
}
