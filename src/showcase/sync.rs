//! Presenter sync (`showcase.sync`): Raycast and the web console follow the
//! dashboard you present from.
//!
//! * The dashboard ([`present`]) says what it shows — its view, the machine
//!   and the session selected (in Focus, the one there) — to the daemon on
//!   change, at most four times a second, and again every 30 s so a
//!   follower knows it's still there. Off the UI thread, over the daemon's
//!   socket (`presenter.set`); with no daemon running, nothing happens.
//! * The daemon keeps the latest ([`set`], [`get`]: `presenter.get`) and
//!   tells subscribers (`events.subscribe` topic `presenter`, `{changed:
//!   "presenter"}`).
//! * Its answers are masked like every other (the lens on the way out), so
//!   a follower's names are the ones it already shows; what it acts on
//!   comes back through the selector path that unmasks names.

use crate::config::Config;
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// At most this often to the daemon (the latest wins).
const THROTTLE: Duration = Duration::from_millis(250);
/// Said again this often while nothing changes (a follower's "still there").
const HEARTBEAT: Duration = Duration::from_secs(30);
/// A presenter not heard from for this long isn't presenting any more.
pub const STALE: u64 = 90;
const SETTING: &str = "showcase.sync";

/// Is presenter sync on (`showcase.sync`)?
pub fn wanted(cfg: &Config) -> bool {
    matches!(cfg.get_or(SETTING).trim(), "on" | "true" | "yes" | "1")
}

// ------------------------------------------------------------------ the dashboard

struct Out {
    last: Option<super::Presenting>,
    at: Option<Instant>,
    /// what was said last was "on" (an "off" goes out once when it stops)
    was_on: bool,
    tx: Option<Sender<Value>>,
}

thread_local! {
    static OUT: std::cell::RefCell<Out> = const { std::cell::RefCell::new(Out {
        last: None,
        at: None,
        was_on: false,
        tx: None,
    }) };
}

/// What `p` says to the daemon (`presenter.set`'s params).
pub fn params(p: &super::Presenting) -> Value {
    let mut v = json!({"on": true, "view": p.view});
    if let Some(h) = &p.host {
        v["host"] = json!(h);
    }
    if let (Some(pane), Some(name)) = (&p.pane, &p.name) {
        v["pane_id"] = json!(pane);
        v["name"] = json!(name);
    }
    v
}

/// The dashboard's every turn: what it shows, to the daemon when it
/// changed (throttled) or now and then; `off` once sync (or showcase mode)
/// goes off.
pub fn present(cfg: &Config, what: impl FnOnce() -> super::Presenting) {
    let on = super::on() && wanted(cfg);
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        if !on {
            if o.was_on {
                o.was_on = false;
                o.last = None;
                send(&mut o, json!({"on": false}));
            }
            return;
        }
        let p = what();
        let fresh = o.at.is_none_or(|t| t.elapsed() >= THROTTLE);
        let changed = o.last.as_ref() != Some(&p);
        let beat = o.at.is_none_or(|t| t.elapsed() >= HEARTBEAT);
        if (changed && fresh) || beat {
            let v = params(&p);
            o.last = Some(p);
            o.at = Some(Instant::now());
            o.was_on = true;
            send(&mut o, v);
        }
    });
}

/// To the daemon, off the UI thread: one worker, the latest wins.
fn send(o: &mut Out, v: Value) {
    if cfg!(test) {
        SENT.with(|s| s.borrow_mut().push(v));
        return;
    }
    let tx = o.tx.get_or_insert_with(|| {
        let (tx, rx) = channel::<Value>();
        std::thread::spawn(move || {
            while let Ok(mut v) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    v = newer;
                }
                // no daemon (nor a dashboard serving its socket): nothing
                if !crate::daemon::running() {
                    continue;
                }
                let _ = crate::daemon::request_at(
                    &crate::daemon::sock_path(),
                    "presenter.set",
                    v,
                    Duration::from_secs(2),
                );
            }
        });
        tx
    });
    let _ = tx.send(v);
}

thread_local! {
    /// tests: what would have gone to the daemon
    static SENT: std::cell::RefCell<Vec<Value>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub fn sent() -> Vec<Value> {
    SENT.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

// ------------------------------------------------------------------ the daemon

struct Kept {
    gen: u64,
    v: Value,
    /// unix time it was last said
    at: u64,
}

fn kept() -> &'static Mutex<Kept> {
    static K: OnceLock<Mutex<Kept>> = OnceLock::new();
    K.get_or_init(|| {
        Mutex::new(Kept {
            gen: 0,
            v: json!({"on": false}),
            at: 0,
        })
    })
}

/// `presenter.set`: a dashboard on this computer says what it shows (only
/// one here: a device elsewhere can't present).
pub fn set(p: &Value, remote: bool) -> Result<Value, String> {
    if remote {
        return Err("only a dashboard on this computer presents".into());
    }
    let s = |k: &str| p.get(k).and_then(Value::as_str).filter(|x| !x.is_empty());
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    let v = if on {
        let mut v = json!({"on": true, "view": s("view").unwrap_or("fleet")});
        if let Some(h) = s("host") {
            v["host"] = json!(h);
        }
        if let (Some(h), Some(pane), Some(name)) = (s("host"), s("pane_id"), s("name")) {
            v["pane_id"] = json!(pane);
            v["name"] = json!(name);
            v["session"] = json!(format!("{h}:{name}"));
        }
        v
    } else {
        json!({"on": false})
    };
    let now = crate::cache::now();
    let mut k = kept().lock().unwrap_or_else(|e| e.into_inner());
    // a heartbeat moves `at`; only a change is news to the followers
    if k.v != v {
        k.gen += 1;
        k.v = v;
    }
    k.at = now;
    Ok(json!({"gen": k.gen}))
}

/// `presenter.get`: what the presenter shows (`on: false` while sync is
/// off, or nobody has said anything for a while).
pub fn get(cfg: &Config) -> Value {
    let k = kept().lock().unwrap_or_else(|e| e.into_inner());
    let now = crate::cache::now();
    let live = wanted(cfg) && super::on() && now.saturating_sub(k.at) <= STALE;
    let mut v = if live {
        k.v.clone()
    } else {
        json!({"on": false})
    };
    v["gen"] = json!(k.gen);
    v["at"] = json!(k.at);
    v
}

/// The `presenter` topic's stamp (it moves when the presenter does).
pub fn stamp() -> u64 {
    kept().lock().unwrap_or_else(|e| e.into_inner()).gen
}

#[cfg(test)]
mod tests {
    use super::super::Presenting;
    use super::*;

    fn fleet(name: &str) -> Presenting {
        Presenting {
            view: "fleet".into(),
            host: Some("b31".into()),
            pane: Some("w1:p2".into()),
            name: Some(name.into()),
        }
    }

    #[test]
    fn the_dashboard_says_what_changed_at_most_four_times_a_second() {
        crate::showcase::force("masked");
        let on = Config::in_memory(&["showcase.sync=on"]);
        sent();
        present(&on, || fleet("temper"));
        let v = sent();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0]["view"], "fleet");
        assert_eq!(
            (v[0]["host"].as_str(), v[0]["name"].as_str()),
            (Some("b31"), Some("temper"))
        );
        // the same again: nothing; a change at once: held back (throttled)
        present(&on, || fleet("temper"));
        present(&on, || fleet("ignite"));
        assert!(sent().is_empty());
        OUT.with(|o| o.borrow_mut().at = Some(Instant::now() - THROTTLE));
        present(&on, || fleet("ignite"));
        assert_eq!(sent()[0]["name"], "ignite");
        // nothing new for a while: said again (still there)
        OUT.with(|o| o.borrow_mut().at = Some(Instant::now() - HEARTBEAT));
        present(&on, || fleet("ignite"));
        assert_eq!(sent().len(), 1);
        // sync off: one "off", then quiet (and the closure isn't asked)
        let off = Config::in_memory(&[]);
        present(&off, || panic!("not asked while off"));
        assert_eq!(sent(), vec![json!({"on": false})]);
        present(&off, || panic!("not asked while off"));
        assert!(sent().is_empty());
        // showcase off: nothing at all
        crate::showcase::force("off");
        present(&on, || panic!("not asked with showcase off"));
        assert!(sent().is_empty());
    }

    #[test]
    fn the_daemon_keeps_the_latest_and_follows_the_setting() {
        crate::showcase::force("masked");
        let on = Config::in_memory(&["showcase.sync=on"]);
        let g0 = stamp();
        set(&params(&fleet("temper")), false).unwrap();
        let v = get(&on);
        assert_eq!(v["on"], true);
        assert_eq!(v["session"], "b31:temper");
        assert_eq!(v["pane_id"], "w1:p2");
        assert!(stamp() > g0);
        // a heartbeat isn't news
        let g1 = stamp();
        set(&params(&fleet("temper")), false).unwrap();
        assert_eq!(stamp(), g1);
        // a device elsewhere can't present
        assert!(set(&json!({"view": "inbox"}), true).is_err());
        // sync off here: nobody presents
        assert_eq!(get(&Config::in_memory(&[]))["on"], false);
        set(&json!({"on": false}), false).unwrap();
        assert_eq!(get(&on)["on"], false);
        // through the API (the socket's own scope; a device elsewhere can't),
        // and its topic moves with it
        let ctx = crate::api::Ctx::local(&on, None);
        let before = crate::keep::stamp("presenter");
        crate::api::call(&ctx, "presenter.set", &json!({"view": "inbox"})).unwrap();
        assert_ne!(crate::keep::stamp("presenter"), before);
        let v = crate::api::call(&ctx, "presenter.get", &json!({})).unwrap();
        assert_eq!(
            (v["on"].as_bool(), v["view"].as_str()),
            (Some(true), Some("inbox"))
        );
        assert!(crate::keep::TOPICS.contains(&"presenter"));
        let mut far = crate::api::Ctx::local(&on, None);
        far.remote = true;
        assert!(crate::api::call(&far, "presenter.set", &json!({"view": "fleet"})).is_err());
        let mut look = crate::api::Ctx::local(&on, None);
        look.scope = crate::api::Scope::Read;
        assert!(crate::api::call(&look, "presenter.get", &json!({})).is_ok());
        assert!(crate::api::call(&look, "presenter.set", &json!({"view": "x"})).is_err());
        crate::showcase::force("off");
        assert_eq!(get(&on)["on"], false);
    }
}
