//! The cross-machine event listener: one live subscription per machine to
//! herdr's socket API (`events.subscribe`), so the fleet hears about a session
//! that starts asking, finishes, appears or closes the moment it happens —
//! across every host, session and agent — instead of at the next poll.
//!
//! * Remote machines: one dedicated `ssh -N -L <local.sock>:<herdr.sock>`
//!   forwards the machine's herdr socket here (OpenSSH stream-local
//!   forwarding; nothing runs on the machine but sshd). `local` connects to
//!   this computer's socket directly.
//! * Protocol: newline-delimited JSON. We subscribe to pane/tab lifecycle
//!   events plus `pane.agent_status_changed` for every known agent pane, and
//!   resubscribe when the pane set changes (herdr rejects a subscription that
//!   names a pane that's gone).
//! * Every event is a nudge: the fleet re-reads that machine right away (the
//!   poll stays the source of truth; events only say "look now").
//! * Errors (`events_lost`, EOF, ssh gone) reconnect with backoff; a machine
//!   whose herdr isn't running is retried every few seconds.

use crate::config::cache_dir;
use crate::{herdr, proc, sshcfg};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Lifecycle events every subscription asks for.
const LIFECYCLE: &[&str] = &[
    "pane.created",
    "pane.closed",
    "pane.exited",
    "pane.moved",
    "pane.agent_detected",
    "pane.updated",
    "tab.created",
    "tab.closed",
    "tab.renamed",
    "workspace.created",
    "workspace.closed",
    "worktree.created",
    "worktree.removed",
];

/// One thing that happened on a machine.
#[derive(Clone, Debug)]
pub struct Ev {
    pub host: String,
    /// `pane.agent_status_changed`, `pane.created`, … (`subscription_started`
    /// when a subscription goes live, `error` when it breaks)
    pub kind: String,
    pub data: Value,
    pub at: std::time::SystemTime,
}

impl Ev {
    fn new(host: &str, kind: &str, data: Value) -> Ev {
        Ev {
            host: host.to_string(),
            kind: kind.to_string(),
            data,
            at: std::time::SystemTime::now(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LState {
    Connecting,
    /// subscribed; events arrive as they happen
    Live,
    /// no subscription right now (why); polling still runs
    Down(String),
}

#[derive(Default)]
struct Shared {
    /// alias → (agent pane ids, version)
    panes: HashMap<String, (Vec<String>, u64)>,
    /// alias → (watch rules' subscriptions, version): `pane.output_matched`
    watch: HashMap<String, (Vec<crate::watchrules::Plan>, u64)>,
    state: HashMap<String, LState>,
    /// one ssh stream-local forward per remote machine, reused across
    /// (re)subscriptions
    forwards: HashMap<String, Child>,
}

pub struct Listener {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Ev>,
}

impl Listener {
    /// Start listening on `hosts`; each event sends the machine's alias on `tx`.
    pub fn start(hosts: &[String], tx: Sender<Ev>) -> Listener {
        let l = Listener {
            stop: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(Shared::default())),
            tx,
        };
        for h in hosts {
            l.add(h);
        }
        l
    }

    /// Start listening on one more machine.
    pub fn add(&self, host: &str) {
        let mut g = self.shared.lock().unwrap();
        if g.state.contains_key(host) {
            return;
        }
        g.state.insert(host.to_string(), LState::Connecting);
        let (h, tx, stop, shared) = (
            host.to_string(),
            self.tx.clone(),
            self.stop.clone(),
            self.shared.clone(),
        );
        thread::spawn(move || run(&h, &tx, &stop, &shared));
    }

    /// The agent panes the last poll saw on `alias` (resubscribes on change).
    pub fn set_panes(&self, alias: &str, mut panes: Vec<String>) {
        panes.sort();
        let mut g = self.shared.lock().unwrap();
        let e = g.panes.entry(alias.to_string()).or_default();
        if e.0 != panes {
            e.0 = panes;
            e.1 += 1;
        }
    }

    /// The watch rules' subscriptions on `alias` (resubscribes on change).
    pub fn set_watch(&self, alias: &str, plans: Vec<crate::watchrules::Plan>) {
        let mut g = self.shared.lock().unwrap();
        let e = g.watch.entry(alias.to_string()).or_default();
        if e.0 != plans {
            e.0 = plans;
            e.1 += 1;
        }
    }

    pub fn state(&self, alias: &str) -> LState {
        self.shared
            .lock()
            .unwrap()
            .state
            .get(alias)
            .cloned()
            .unwrap_or(LState::Down("not started".into()))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let mut g = self.shared.lock().unwrap();
        for (_, c) in g.forwards.drain() {
            stop_forward(c);
        }
    }
}

fn set_state(shared: &Arc<Mutex<Shared>>, h: &str, s: LState) {
    shared.lock().unwrap().state.insert(h.to_string(), s);
}

/// Where herdr's socket lives on `host` (`herdr status server` prints it).
pub(crate) fn remote_socket(host: &str) -> Result<String, String> {
    let line = format!(
        "{}{} status server",
        crate::chan::herdr_env(host),
        proc::sh_quote(&crate::chan::herdr_path(host))
    );
    let o = proc::run(
        &mut sshcfg::shell(host, &line),
        Duration::from_secs(20),
        None,
    )
    .map_err(|e| e.to_string())?;
    let text = o.text();
    socket_from_status(&text).ok_or_else(|| {
        if o.ok() {
            "herdr didn't say where its socket is".into()
        } else {
            o.reason()
        }
    })
}

/// `socket: /home/u/.config/herdr/herdr.sock` → the path.
pub fn socket_from_status(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("socket:"))
        .map(|p| p.trim().to_string())
        .filter(|p| p.starts_with('/'))
}

pub(crate) fn local_forward_path(host: &str) -> PathBuf {
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_dir().join("ev").join(format!("{safe}.sock"))
}

/// Where to connect for `host`'s herdr socket: the socket itself for
/// `local`, else the local end of a dedicated ssh stream-local forward (one
/// per machine, reused while it lives).
fn endpoint(host: &str, shared: &Arc<Mutex<Shared>>, stop: &AtomicBool) -> Result<PathBuf, String> {
    // showcase's demo world: no herdr events to follow
    if let Some(e) = crate::showcase::not_here() {
        return Err(e);
    }
    if sshcfg::is_local(host) {
        let sock = socket_from_status(&herdr::local_status())
            .ok_or("herdr didn't say where its socket is")?;
        return Ok(PathBuf::from(sock));
    }
    let local = local_forward_path(host);
    {
        let mut g = shared.lock().unwrap();
        if let Some(c) = g.forwards.get_mut(host) {
            if matches!(c.try_wait(), Ok(None)) && local.exists() {
                return Ok(local);
            }
            if let Some(c) = g.forwards.remove(host) {
                stop_forward(c);
            }
        }
    }
    let remote = remote_socket(host)?;
    let _ = std::fs::create_dir_all(local.parent().unwrap_or(&cache_dir()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(d) = local.parent() {
            let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _ = std::fs::remove_file(&local);
    // a dedicated connection (forwards through a ControlMaster client would
    // die with it), under a tiny sh watchdog that ends it when we go away —
    // even if this process is killed outright and never runs its cleanup
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(WATCHDOG)
        .arg("sh")
        .arg(sshcfg::ssh_bin())
        .args(sshcfg::BATCH)
        .args(["-o", "ControlMaster=no", "-o", "ControlPath=none"])
        .args([
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "StreamLocalBindUnlink=yes",
        ])
        .arg("-N")
        .arg("-T")
        .arg("-L")
        .arg(format!("{}:{remote}", local.display()))
        .arg(host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ssh: {e}"))?;
    let t0 = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            stop_forward(child);
            return Err("stopped".into());
        }
        if local.exists() && UnixStream::connect(&local).is_ok() {
            shared
                .lock()
                .unwrap()
                .forwards
                .insert(host.to_string(), child);
            return Ok(local);
        }
        if let Ok(Some(_)) = child.try_wait() {
            let mut e = String::new();
            if let Some(mut er) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut er, &mut e);
            }
            let reason = e
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("ssh exited")
                .to_string();
            return Err(reason);
        }
        if t0.elapsed() > Duration::from_secs(15) {
            stop_forward(child);
            return Err("timed out forwarding herdr's socket".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Runs `ssh "$@"` and ends it when the parent (us) is gone, or on TERM.
/// POSIX sh; `$PPID` is fixed at start, so a reparented watchdog notices.
pub(crate) const WATCHDOG: &str =
    "trap 'kill $s 2>/dev/null; exit 0' TERM INT HUP; \"$@\" & s=$!; \
    while kill -0 $PPID 2>/dev/null && kill -0 $s 2>/dev/null; do sleep 1; done; \
    kill $s 2>/dev/null";

/// TERM the watchdog (it takes ssh with it), then make sure it's gone.
fn stop_forward(mut c: Child) {
    let _ = Command::new("kill")
        .args(["-TERM", &c.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    for _ in 0..30 {
        if matches!(c.try_wait(), Ok(Some(_))) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = c.kill();
    let _ = c.wait();
}

/// A line read from one of a machine's subscriptions: (which, text); text
/// `None` = that connection ended.
type Read = (u8, Option<String>);

/// Connect, send one `events.subscribe`, and read its lines on a thread.
/// Returns the stream (to shut it down later).
fn open(path: &PathBuf, req: String, tag: u8, tx: Sender<Read>) -> Result<UnixStream, String> {
    let mut s = UnixStream::connect(path).map_err(|e| format!("herdr's socket: {e}"))?;
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let r = s.try_clone().map_err(|e| e.to_string())?;
    thread::spawn(move || {
        let mut r = BufReader::new(r);
        loop {
            let mut buf = String::new();
            match r.read_line(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.send((tag, None));
                    return;
                }
                Ok(_) => {
                    if tx.send((tag, Some(buf))).is_err() {
                        return;
                    }
                }
            }
        }
    });
    Ok(s)
}

/// The `events.subscribe` request line: lifecycle events when `panes` is
/// None, else agent status for those panes. Two separate subscriptions, so a
/// pane that closes (herdr rejects a subscription naming it) never takes the
/// lifecycle stream down with it.
pub fn subscribe_request(id: &str, panes: Option<&[String]>) -> String {
    let subs: Vec<Value> = match panes {
        None => LIFECYCLE.iter().map(|t| json!({"type": t})).collect(),
        Some(ps) => ps
            .iter()
            .map(|p| json!({"type": "pane.agent_status_changed", "pane_id": p}))
            .collect(),
    };
    let mut s = json!({"id": id, "method": "events.subscribe", "params": {"subscriptions": subs}})
        .to_string();
    s.push('\n');
    s
}

/// What a line from the socket means.
#[derive(Debug, PartialEq)]
pub enum Line {
    /// our subscription is live
    Started,
    /// something happened: re-read the machine
    Event(String),
    /// the subscription failed (code, message)
    Error(String, String),
    Other,
}

/// herdr sends `pane_created` / `pane_agent_status_changed`; its schema says
/// `pane.created`. One spelling everywhere: the dotted one.
pub fn normalize(kind: &str) -> String {
    for p in ["pane", "tab", "workspace", "worktree", "layout"] {
        if let Some(rest) = kind.strip_prefix(&format!("{p}_")) {
            return format!("{p}.{rest}");
        }
    }
    kind.to_string()
}

pub fn classify(line: &str) -> Line {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
        return Line::Other;
    };
    if let Some(e) = v.get("error") {
        let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        return Line::Error(s("code"), s("message"));
    }
    if let Some(ev) = v.get("event").and_then(Value::as_str) {
        return Line::Event(normalize(ev));
    }
    let t = v
        .pointer("/result/type")
        .or_else(|| v.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if t == "subscription_started" || v.get("result").is_some() {
        return Line::Started;
    }
    Line::Other
}

fn run(host: &str, tx: &Sender<Ev>, stop: &Arc<AtomicBool>, shared: &Arc<Mutex<Shared>>) {
    let mut backoff = Duration::from_millis(500);
    let mut n = 0u64;
    let panes_now = || shared.lock().unwrap().panes.get(host).cloned();
    let watch_now = || shared.lock().unwrap().watch.get(host).cloned();
    while !stop.load(Ordering::SeqCst) {
        set_state(shared, host, LState::Connecting);
        let path = match endpoint(host, shared, stop) {
            Ok(p) => p,
            Err(e) => {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                set_state(shared, host, LState::Down(e));
                sleep_until(stop, backoff);
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        let (ltx, lrx) = std::sync::mpsc::channel::<Read>();
        n += 1;
        let lifecycle = match open(
            &path,
            subscribe_request(&format!("aleph-l{n}"), None),
            0,
            ltx.clone(),
        ) {
            Ok(s) => s,
            Err(e) => {
                set_state(shared, host, LState::Down(e));
                sleep_until(stop, backoff);
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        // the per-pane status subscription, reopened whenever the pane set changes
        let mut status: Option<UnixStream> = None;
        let mut status_ver: Option<u64> = None;
        // one per watch rule (tags 2…), reopened whenever the rules or panes change
        let mut watch: Vec<UnixStream> = Vec::new();
        let mut watch_ver: Option<u64> = None;
        let mut watch_rules: Vec<String> = Vec::new();
        let reason: String = loop {
            if stop.load(Ordering::SeqCst) {
                let _ = lifecycle.shutdown(std::net::Shutdown::Both);
                if let Some(s) = &status {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
                for s in &watch {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
                return;
            }
            if let Some((plans, ver)) = watch_now() {
                if watch_ver != Some(ver) {
                    for s in watch.drain(..) {
                        let _ = s.shutdown(std::net::Shutdown::Both);
                    }
                    watch_ver = Some(ver);
                    watch_rules = plans.iter().map(|p| p.rule.clone()).collect();
                    for (i, p) in plans.iter().take(40).enumerate() {
                        n += 1;
                        let req = crate::watchrules::request(&format!("aleph-w{n}"), p);
                        if let Ok(s) = open(&path, req, 2 + i as u8, ltx.clone()) {
                            watch.push(s);
                        }
                    }
                }
            }
            if let Some((panes, ver)) = panes_now() {
                if status_ver != Some(ver) {
                    if let Some(s) = status.take() {
                        let _ = s.shutdown(std::net::Shutdown::Both);
                    }
                    status_ver = Some(ver);
                    if !panes.is_empty() {
                        n += 1;
                        let req = subscribe_request(&format!("aleph-s{n}"), Some(&panes));
                        status = open(&path, req, 1, ltx.clone()).ok();
                    }
                }
            }
            match lrx.recv_timeout(Duration::from_millis(300)) {
                Ok((0, None)) => break "herdr closed the connection".to_string(),
                // the status stream ended (we closed it, or herdr rejected a pane)
                Ok((_, None)) => {}
                Ok((tag, Some(line))) => match classify(&line) {
                    Line::Started if tag == 0 => {
                        set_state(shared, host, LState::Live);
                        backoff = Duration::from_millis(500);
                        // catch up on anything that happened while we connected
                        let _ = tx.send(Ev::new(host, "subscription_started", Value::Null));
                    }
                    Line::Started | Line::Other => {}
                    Line::Event(kind) => {
                        let mut data = serde_json::from_str::<Value>(line.trim())
                            .ok()
                            .and_then(|v| v.get("data").cloned())
                            .unwrap_or(Value::Null);
                        // which watch rule saw it
                        if tag >= 2 {
                            if let (Some(r), Some(o)) =
                                (watch_rules.get((tag - 2) as usize), data.as_object_mut())
                            {
                                o.insert("rule".into(), json!(r));
                            }
                        }
                        let _ = tx.send(Ev::new(host, &kind, data));
                    }
                    Line::Error(code, msg) if tag >= 2 => {
                        // a pane a rule named is gone: wait for the next poll
                        shared.lock().unwrap().watch.remove(host);
                        watch_ver = None;
                        crate::log::warn("watch", &format!("{host}: {code}: {msg}"));
                    }
                    Line::Error(code, msg) if tag == 1 => {
                        // a pane we named is gone: wait for the next poll's pane list
                        shared.lock().unwrap().panes.remove(host);
                        status_ver = None;
                        let _ = tx.send(Ev::new(
                            host,
                            "error",
                            json!({"code": code, "message": msg}),
                        ));
                    }
                    Line::Error(code, msg) => {
                        let _ = tx.send(Ev::new(
                            host,
                            "error",
                            json!({"code": code, "message": msg}),
                        ));
                        break format!("{code}: {msg}");
                    }
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break "listener stopped".to_string(),
            }
        };
        let _ = lifecycle.shutdown(std::net::Shutdown::Both);
        if let Some(s) = status {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        for s in watch.drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        set_state(shared, host, LState::Down(reason));
        sleep_until(stop, backoff);
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

fn sleep_until(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }
}

// ------------------------------------------------------------------ aleph events

fn glyph(kind: &str, data: &Value) -> (&'static str, ratatui::style::Color, String) {
    use crate::ui::c;
    let st = data
        .get("agent_status")
        .and_then(Value::as_str)
        .unwrap_or("");
    match kind {
        "pane.agent_status_changed" => match st {
            "blocked" => ("▲", c::NEEDS(), "needs you".into()),
            "working" => ("✻", c::WORKING(), "working".into()),
            "done" => ("✓", c::GREEN(), "done".into()),
            "idle" => ("○", c::MUTED(), "idle".into()),
            s => ("·", c::MUTED(), s.to_string()),
        },
        "pane.created" | "tab.created" => ("+", c::BLUE(), kind.replace('.', " ")),
        "pane.closed" | "tab.closed" | "pane.exited" => ("–", c::MUTED(), kind.replace('.', " ")),
        "pane.agent_detected" => ("◆", c::MAUVE(), "agent detected".into()),
        "subscription_started" => ("●", c::GREEN(), "listening".into()),
        "error" => ("✕", c::RED(), "error".into()),
        k => ("·", c::FAINT(), k.to_string()),
    }
}

/// `aleph events [--host H] [--json]`: every machine's herdr events, live.
pub fn cmd(a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::{head, out};
    use crate::ui;
    use ratatui::prelude::*;
    let as_json = a.has("--json");
    let mut f = crate::fleet::Fleet::new(&a.cfg, a.flag.as_deref());
    if f.hosts.is_empty() {
        return crate::cli::no_host();
    }
    f.poll_blocking(Duration::from_secs(20));
    let (tx, rx) = std::sync::mpsc::channel();
    let hosts: Vec<String> = f.hosts.iter().map(|h| h.alias.clone()).collect();
    let l = Listener::start(&hosts, tx);
    let feed = |f: &crate::fleet::Fleet, l: &Listener| {
        for h in &f.hosts {
            if h.conn == crate::fleet::Conn::Up {
                l.set_panes(
                    &h.alias,
                    h.sessions.iter().map(|s| s.pane_id.clone()).collect(),
                );
            }
        }
    };
    feed(&f, &l);
    if !as_json {
        head(&format!(
            "listening to {} — ctrl+c to stop",
            hosts.join(", ")
        ));
    }
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(e) => {
                f.nudge(&e.host);
                let pane = e
                    .data
                    .get("pane_id")
                    .or_else(|| e.data.pointer("/pane/pane_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = f
                    .host(&e.host)
                    .and_then(|h| h.sessions.iter().find(|s| s.pane_id == pane))
                    .map(|s| s.display().to_string())
                    .unwrap_or_default();
                if as_json {
                    let t =
                        e.at.duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                    println!(
                        "{}",
                        json!({"t": t, "host": e.host, "event": e.kind, "pane_id": pane, "name": name, "data": e.data})
                    );
                    continue;
                }
                let (g, col, what) = glyph(&e.kind, &e.data);
                let title = e
                    .data
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| e.data.get("message").and_then(Value::as_str))
                    .unwrap_or("");
                let hcol = crate::theme::machine(&a.cfg, &e.host);
                let secs =
                    e.at.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                let hms = format!(
                    "{:02}:{:02}:{:02}",
                    secs / 3600 % 24,
                    secs / 60 % 60,
                    secs % 60
                );
                out(Line::from(vec![
                    Span::styled(format!("{hms} UTC  "), ui::faint()),
                    Span::styled("▌", ui::fg(hcol)),
                    Span::styled(ui::pad(&e.host, 8), ui::fg(hcol)),
                    Span::styled(format!("{g} "), ui::fg(col)),
                    Span::styled(ui::pad(&what, 16), ui::fg(col)),
                    Span::styled(ui::pad(&name, 18), ui::bold()),
                    Span::styled(ui::pad(&pane, 8), ui::faint()),
                    Span::styled(ui::trunc(title, 60), ui::muted()),
                ]));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return std::process::ExitCode::from(1),
        }
        f.tick();
        feed(&f, &l);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_socket() {
        let t = "status: running\nsocket: /home/selene/.config/herdr/herdr.sock\npid: 42\n";
        assert_eq!(
            socket_from_status(t).as_deref(),
            Some("/home/selene/.config/herdr/herdr.sock")
        );
        assert_eq!(socket_from_status("status: not running"), None);
    }

    #[test]
    fn request_and_lines() {
        // lifecycle and per-pane status are separate subscriptions
        let l = subscribe_request("l1", None);
        let v: Value = serde_json::from_str(l.trim()).unwrap();
        assert_eq!(v["method"], "events.subscribe");
        let subs = v["params"]["subscriptions"].as_array().unwrap();
        assert!(subs.iter().any(|s| s["type"] == "pane.created"));
        assert!(subs.iter().any(|s| s["type"] == "pane.moved"));
        assert!(!subs
            .iter()
            .any(|s| s["type"] == "pane.agent_status_changed"));
        let r = subscribe_request("s1", Some(&["w1:p2".into()]));
        let v: Value = serde_json::from_str(r.trim()).unwrap();
        let subs = v["params"]["subscriptions"].as_array().unwrap();
        assert_eq!(subs.len(), 1);
        assert!(subs
            .iter()
            .any(|s| s["type"] == "pane.agent_status_changed" && s["pane_id"] == "w1:p2"));
        assert_eq!(
            classify(r#"{"id":"s1","result":{"type":"subscription_started"}}"#),
            Line::Started
        );
        assert_eq!(
            classify(
                r#"{"event":"pane.agent_status_changed","data":{"pane_id":"w1:p2","workspace_id":"w1","agent_status":"blocked"}}"#
            ),
            Line::Event("pane.agent_status_changed".into())
        );
        assert_eq!(
            classify(r#"{"id":"s1","error":{"code":"events_lost","message":"fell behind"}}"#),
            Line::Error("events_lost".into(), "fell behind".into())
        );
        assert_eq!(classify("not json"), Line::Other);
        assert_eq!(
            classify(r#"{"event":"pane_created","data":{}}"#),
            Line::Event("pane.created".into())
        );
        assert_eq!(
            normalize("pane_agent_status_changed"),
            "pane.agent_status_changed"
        );
        assert_eq!(normalize("tab.closed"), "tab.closed");
    }
}
