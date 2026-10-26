//! Telling the running Aleph that a machine changed — it was just set up:
//! herdr installed, agents added — so it looks again now instead of in ten
//! minutes: its fleet polls the machine, and what it cached about it is
//! dropped and read again (herdr's path and version, the agents report, OS
//! and GPUs, the stats sampler, the LLM-server scan).
//!
//! Two ways, so it works with a daemon or without one:
//! * the daemon's socket, `hosts.refresh {host, …}` — the daemon, or a
//!   dashboard serving the socket while no daemon runs;
//! * a marker, `state/refresh/<host>.json`, that every dashboard looks for
//!   once a second. The socket method writes it too, so a dashboard running
//!   next to a daemon hears of it as well.
//!
//! The marker carries what the setup found ([`Note`]: herdr's version, the
//! agents there now, what was installed and what failed) for the toast:
//! `b31 set up · herdr 0.9.5 · 3 agents`.

use crate::config;
use crate::probe;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What happened to a machine.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Note {
    pub host: String,
    /// when, in ms since the epoch (a dashboard acts on each one once)
    pub at: u64,
    /// `setup` (aleph host setup), `install` (aleph agents install), `refresh`
    pub what: String,
    /// herdr's version there now (None: not installed)
    pub herdr: Option<String>,
    /// the agents there now: (id, version)
    pub agents: Vec<(String, String)>,
    /// names of the agents just installed, and of those that failed
    pub installed: Vec<String>,
    pub failed: Vec<String>,
    /// the agents report on disk is fresh (no need to look again)
    pub probed: bool,
    /// per-agent results (`agentsetup::Report::to_json`)
    pub report: Option<Value>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Note {
    /// A bare "look again" (`hosts.refresh` from Raycast, a script).
    pub fn bare(host: &str) -> Note {
        Note {
            host: host.to_string(),
            at: now_ms(),
            what: "refresh".into(),
            ..Default::default()
        }
    }

    /// After an agent setup: what the machine has now.
    pub fn from_report(what: &str, rep: &crate::agentsetup::Report) -> Note {
        let p = &rep.probe;
        let names = |s: &str| -> Vec<String> {
            rep.results
                .iter()
                .filter(|(_, o)| o.status() == s)
                .map(|(a, _)| a.name.to_string())
                .collect()
        };
        Note {
            host: rep.host.clone(),
            at: now_ms(),
            what: what.to_string(),
            herdr: p.herdr_version().map(str::to_string),
            agents: p
                .installed()
                .iter()
                .map(|(a, f)| (a.id.to_string(), f.version.clone()))
                .collect(),
            installed: names("installed"),
            failed: names("failed"),
            probed: p.ok() && p.age.is_none(),
            report: Some(rep.to_json()),
        }
    }

    /// From a probe alone (a setup that didn't get to the agents).
    pub fn from_probe(what: &str, p: &probe::HostProbe) -> Note {
        Note {
            host: p.host.clone(),
            at: now_ms(),
            what: what.to_string(),
            herdr: p.herdr_version().map(str::to_string),
            agents: p
                .installed()
                .iter()
                .map(|(a, f)| (a.id.to_string(), f.version.clone()))
                .collect(),
            probed: p.ok() && p.age.is_none(),
            ..Default::default()
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "host": self.host,
            "at": self.at,
            "what": self.what,
            "herdr": self.herdr,
            "agents": self.agents.iter().map(|(id, v)| json!({"id": id, "version": v})).collect::<Vec<_>>(),
            "installed": self.installed,
            "failed": self.failed,
            "probed": self.probed,
            "report": self.report,
        })
    }

    pub fn from_json(v: &Value) -> Option<Note> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        };
        Some(Note {
            host: s("host").filter(|h| !h.is_empty())?,
            at: v.get("at").and_then(Value::as_u64).unwrap_or(0),
            what: s("what").unwrap_or_else(|| "refresh".into()),
            herdr: s("herdr"),
            agents: v
                .get("agents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|a| {
                    Some((
                        a.get("id")?.as_str()?.to_string(),
                        a.get("version")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    ))
                })
                .collect(),
            installed: list("installed"),
            failed: list("failed"),
            probed: v.get("probed").and_then(Value::as_bool).unwrap_or(false),
            report: v.get("report").filter(|r| !r.is_null()).cloned(),
        })
    }

    /// The dashboard's toast (pure): (all good, text).
    pub fn toast(&self) -> (bool, String) {
        let mut s = match self.what.as_str() {
            "setup" => format!("{} set up", self.host),
            "install" if !self.installed.is_empty() => {
                format!("{}: {} installed", self.host, self.installed.join(", "))
            }
            _ => format!("{} looked at again", self.host),
        };
        if self.what != "refresh" || self.herdr.is_some() || !self.agents.is_empty() {
            match &self.herdr {
                Some(v) => s.push_str(&format!(" · herdr {v}")),
                None => s.push_str(" · no herdr yet"),
            }
            let n = self.agents.len();
            s.push_str(&format!(" · {n} agent{}", if n == 1 { "" } else { "s" }));
        }
        if !self.failed.is_empty() {
            s.push_str(&format!(
                " · {} failed ({})",
                self.failed.len(),
                self.failed.join(", ")
            ));
        }
        (
            self.failed.is_empty() && (self.what == "refresh" || self.herdr.is_some()),
            s,
        )
    }
}

/// `state/refresh/`.
pub fn dir() -> PathBuf {
    config::state_dir().join("refresh")
}

fn marker(host: &str) -> PathBuf {
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.@".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir().join(format!("{safe}.json"))
}

/// The last note about `host` (the wizard reads the per-agent results).
pub fn last(host: &str) -> Option<Note> {
    let t = std::fs::read_to_string(marker(host)).ok()?;
    Note::from_json(&serde_json::from_str(&t).ok()?)
}

/// Drop what this process and the disk cache about `host`: herdr's path and
/// version there, and (`agents`) the agents report — when it's stale.
pub fn forget(host: &str, agents: bool) {
    crate::chan::forget(host);
    crate::fleet::forget_version(host);
    if agents {
        let _ = std::fs::remove_file(probe::cache_file(host));
    }
}

/// Leave the note where dashboards look.
pub fn mark(n: &Note) -> std::io::Result<()> {
    config::write_atomic(&marker(&n.host), n.to_json().to_string().as_bytes())
}

/// Who heard of it.
#[derive(Debug, Clone, PartialEq)]
pub enum Told {
    /// the process on the daemon's socket (`daemon`, or `dashboard`)
    Socket(String),
    /// no socket answered: the marker, for dashboards
    Marker,
    Failed(String),
}

/// Tell the running Aleph about `n`: the daemon's socket when something
/// answers there (it marks too), else the marker alone.
pub fn signal(n: &Note) -> Told {
    forget(&n.host, !n.probed);
    let sock = crate::daemon::sock_path();
    if sock.exists() {
        if let Ok(v) =
            crate::daemon::request_at(&sock, "hosts.refresh", n.to_json(), Duration::from_secs(5))
        {
            let by = v
                .get("by")
                .and_then(Value::as_str)
                .unwrap_or("daemon")
                .to_string();
            return Told::Socket(by);
        }
    }
    match mark(n) {
        Ok(()) => Told::Marker,
        Err(e) => Told::Failed(e.to_string()),
    }
}

/// How [`signal`] went, as a line for the terminal.
pub fn told_line(host: &str, t: &Told) -> String {
    match t {
        Told::Socket(k) if k == "dashboard" => {
            format!("told the dashboard — it's looking at {host} again")
        }
        Told::Socket(_) => format!("told the daemon — Aleph is looking at {host} again"),
        Told::Marker => format!(
            "left a note for Aleph — an open dashboard looks at {host} again within a second"
        ),
        Told::Failed(e) => format!("couldn't tell Aleph ({e}) — it looks again within 10 minutes"),
    }
}

/// A dashboard's eye on the markers (once a second at most). Only notes
/// written after it started count, each once.
pub struct Watch {
    since: u64,
    seen: HashMap<String, u64>,
    at: Instant,
}

impl Default for Watch {
    fn default() -> Self {
        Watch::new()
    }
}

impl Watch {
    pub fn new() -> Watch {
        Watch {
            since: now_ms(),
            seen: HashMap::new(),
            at: Instant::now(),
        }
    }

    /// New notes (`now`: look even if it's been under a second).
    pub fn poll(&mut self, now: bool) -> Vec<Note> {
        self.poll_in(&dir(), now)
    }

    fn poll_in(&mut self, d: &std::path::Path, now: bool) -> Vec<Note> {
        if !now && self.at.elapsed() < Duration::from_secs(1) {
            return vec![];
        }
        self.at = Instant::now();
        let Ok(rd) = std::fs::read_dir(d) else {
            return vec![];
        };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(n) = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| Note::from_json(&v))
            else {
                continue;
            };
            if n.at <= self.since || self.seen.get(&n.host).is_some_and(|t| *t >= n.at) {
                continue;
            }
            self.seen.insert(n.host.clone(), n.at);
            out.push(n);
        }
        out.sort_by_key(|n| n.at);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Note {
        Note {
            host: "b31".into(),
            at: 1_700_000_000_000,
            what: "setup".into(),
            herdr: Some("0.9.5".into()),
            agents: vec![
                ("claude".into(), "2.1.200".into()),
                ("codex".into(), "0.50.0".into()),
                ("gemini".into(), String::new()),
            ],
            installed: vec!["Codex".into()],
            failed: vec![],
            probed: true,
            report: None,
        }
    }

    #[test]
    fn notes_round_trip() {
        let n = note();
        assert_eq!(Note::from_json(&n.to_json()), Some(n));
        assert_eq!(Note::from_json(&json!({"host": ""})), None);
        let b = Note::from_json(&json!({"host": "gpu2"})).unwrap();
        assert_eq!((b.what.as_str(), b.probed, b.at), ("refresh", false, 0));
    }

    #[test]
    fn toasts_say_what_the_machine_has() {
        assert_eq!(
            note().toast(),
            (true, "b31 set up · herdr 0.9.5 · 3 agents".to_string())
        );
        let mut n = note();
        n.failed = vec!["Cursor CLI".into()];
        n.agents.truncate(1);
        assert_eq!(
            n.toast(),
            (
                false,
                "b31 set up · herdr 0.9.5 · 1 agent · 1 failed (Cursor CLI)".to_string()
            )
        );
        n.herdr = None;
        n.failed.clear();
        assert!(!n.toast().0 && n.toast().1.contains("no herdr yet"));
        let mut i = note();
        i.what = "install".into();
        assert_eq!(i.toast().1, "b31: Codex installed · herdr 0.9.5 · 3 agents");
        assert_eq!(
            Note::bare("box3").toast(),
            (true, "box3 looked at again".to_string())
        );
    }

    #[test]
    fn the_watch_sees_each_new_note_once() {
        let d = std::env::temp_dir().join(format!("aleph-refresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut w = Watch::new();
        let write = |n: &Note| {
            std::fs::write(d.join(format!("{}.json", n.host)), n.to_json().to_string()).unwrap()
        };
        let read = |w: &mut Watch| -> Vec<String> {
            w.poll_in(&d, true).into_iter().map(|n| n.host).collect()
        };
        std::fs::write(d.join("junk.json"), "not json").unwrap();
        let mut old = note();
        old.at = w.since - 1;
        write(&old);
        assert!(
            read(&mut w).is_empty(),
            "notes from before it started don't count"
        );
        let mut n = note();
        n.at = w.since + 5;
        write(&n);
        assert_eq!(read(&mut w), vec!["b31"]);
        assert!(read(&mut w).is_empty(), "once");
        n.at += 1;
        write(&n);
        assert_eq!(read(&mut w), vec!["b31"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
