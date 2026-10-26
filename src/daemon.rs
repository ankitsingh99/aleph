//! The daemon's front door: `state/daemon.sock` (0600, in a 0700 folder).
//!
//! Binding it is the single-instance lock. A second daemon finds it
//! answering and stays out; a socket nobody answers on is stale, so it's
//! removed and bound again — under a short file lock, so two daemons
//! starting together never both do that.
//!
//! The protocol is herdr's style of newline-delimited JSON:
//! `{"id","method","params"}` in, `{"id","result"}` or
//! `{"id","error":{"code","message"}}` out. `events.subscribe` keeps the
//! connection open and pushes `{"id","event"}` and `{"id","state"}` lines
//! (plus `{"id","ping"}` every 15 s, so a dead client is noticed). The
//! methods themselves are `api.rs`'s; `aleph api --stdio` bridges a terminal
//! (or `ssh vps aleph api --stdio`) to the socket.

use crate::api;
use crate::config::{state_dir, Config};
use crate::events::{self, Event};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// This process serves the socket (a daemon, or a dashboard with no daemon).
static OURS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// At most this many connections at once (each is a thread).
const MAX_CONNS: usize = 64;
/// A request line longer than this is refused (and the connection closed).
const MAX_LINE: u64 = 1 << 20;

/// Where the socket lives. Unix socket paths are short (104 bytes on macOS),
/// so a long state folder falls back to a per-user folder under /tmp.
pub fn sock_path() -> PathBuf {
    let p = state_dir().join("daemon.sock");
    if p.as_os_str().len() < 100 {
        return p;
    }
    let mut x: u64 = 0xcbf2_9ce4_8422_2325;
    for b in p.as_os_str().as_encoded_bytes() {
        x = (x ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    PathBuf::from(format!("/tmp/al-{}", uid())).join(format!("d-{:08x}.sock", x as u32))
}

fn uid() -> String {
    std::env::var("UID")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| crate::proc::cap("id", &["-u"], Duration::from_secs(2)).text())
        .trim()
        .to_string()
}

fn pid_path(sock: &Path) -> PathBuf {
    sock.with_extension("pid")
}

/// The pid the running daemon wrote next to its socket.
pub fn pid() -> Option<u32> {
    std::fs::read_to_string(pid_path(&sock_path()))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

// ------------------------------------------------------------------ binding

/// The bound socket: ours while this process lives.
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
}

pub enum Bind {
    Ours(Server),
    /// another daemon answers there (its pid, when it wrote one)
    Taken(Option<u32>),
    Failed(String),
}

/// Take the daemon's socket (see the module docs).
pub fn bind() -> Bind {
    bind_at(&sock_path())
}

pub fn bind_at(path: &Path) -> Bind {
    if let Some(d) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(d) {
            return Bind::Failed(format!("{}: {e}", d.display()));
        }
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
    }
    // only one process at a time looks at (and replaces) a stale socket
    let guard = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"));
    let _held = guard.as_ref().ok().map(|f| {
        let t0 = Instant::now();
        while f.try_lock().is_err() && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let r = match UnixListener::bind(path) {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Bind::Taken(
                    std::fs::read_to_string(pid_path(path))
                        .ok()
                        .and_then(|s| s.trim().parse().ok()),
                );
            }
            // nobody answers: a daemon that died without cleaning up
            let _ = std::fs::remove_file(path);
            UnixListener::bind(path)
        }
        Err(e) => Err(e),
    };
    match r {
        Ok(listener) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            let _ = std::fs::write(pid_path(path), format!("{}\n", std::process::id()));
            Bind::Ours(Server {
                listener,
                path: path.to_path_buf(),
            })
        }
        Err(e) => Bind::Failed(format!("{}: {e}", path.display())),
    }
}

impl Server {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ------------------------------------------------------------------ shared state

/// What the daemon's loop publishes for the API: the fleet snapshot (as
/// `fleet.json`) and a generation counter that moves on every change.
pub struct Shared {
    st: Mutex<State>,
    cv: Condvar,
    pub started: u64,
    t0: Instant,
    /// `daemon`, or `dashboard` (one serving while no daemon runs)
    pub kind: &'static str,
}

#[derive(Default)]
struct State {
    gen: u64,
    snapshot: Value,
    hub: bool,
    /// where the HTTP API listens (`api.http = on`)
    http: Option<String>,
    /// machines to look at again (`hosts.refresh`), for the loop to take
    refresh: Vec<String>,
}

impl Shared {
    pub fn new() -> Arc<Shared> {
        Shared::of("daemon")
    }

    pub fn of(kind: &'static str) -> Arc<Shared> {
        Arc::new(Shared {
            kind,
            st: Mutex::new(State {
                snapshot: json!({"version": 1, "hosts": []}),
                ..Default::default()
            }),
            cv: Condvar::new(),
            started: crate::cache::now(),
            t0: Instant::now(),
        })
    }

    /// A new fleet snapshot; `hub`: this process runs the hub. Subscribers
    /// hear of it (the generation moves) only when something they'd see
    /// changed: times and latencies alone just refresh it.
    pub fn publish(&self, snapshot: Value, hub: bool) {
        let mut s = self.st.lock().unwrap();
        let news = hub != s.hub || essence(&snapshot) != essence(&s.snapshot);
        s.snapshot = snapshot;
        s.hub = hub;
        if news {
            s.gen += 1;
            self.cv.notify_all();
        }
    }

    /// (generation, snapshot)
    pub fn get(&self) -> (u64, Value) {
        let s = self.st.lock().unwrap();
        (s.gen, s.snapshot.clone())
    }

    pub fn hub(&self) -> bool {
        self.st.lock().unwrap().hub
    }

    /// The generation alone (no copy of the snapshot).
    pub fn generation(&self) -> u64 {
        self.st.lock().unwrap().gen
    }

    pub fn set_http(&self, addr: Option<String>) {
        self.st.lock().unwrap().http = addr;
    }

    /// Where the HTTP API listens (None: it's off).
    pub fn http(&self) -> Option<String> {
        self.st.lock().unwrap().http.clone()
    }

    pub fn uptime(&self) -> Duration {
        self.t0.elapsed()
    }

    /// A machine changed (`hosts.refresh`): the loop looks at it again.
    pub fn ask_refresh(&self, host: &str) {
        let mut s = self.st.lock().unwrap();
        if !s.refresh.iter().any(|h| h == host) {
            s.refresh.push(host.to_string());
        }
    }

    /// The machines asked about since the last call.
    pub fn take_refresh(&self) -> Vec<String> {
        std::mem::take(&mut self.st.lock().unwrap().refresh)
    }

    /// Wait until the generation moves past `gen` (or `timeout`); the
    /// current generation.
    pub fn wait(&self, gen: u64, timeout: Duration) -> u64 {
        let s = self.st.lock().unwrap();
        let (s, _) = self
            .cv
            .wait_timeout_while(s, timeout, |s| s.gen == gen)
            .unwrap();
        s.gen
    }
}

/// A snapshot without what changes on every poll (pure — tested).
fn essence(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("generated_at");
    }
    for h in v
        .get_mut("hosts")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(o) = h.as_object_mut() {
            o.remove("fetched_at");
            o.remove("latency_ms");
        }
    }
    v
}

// ------------------------------------------------------------------ serving

/// Answer on the socket from a thread of its own (one more per connection).
pub fn serve(server: Server, shared: Arc<Shared>) {
    OURS.store(true, Ordering::Relaxed);
    let live = Arc::new(AtomicUsize::new(0));
    let _ = std::thread::Builder::new()
        .name("api".into())
        .spawn(move || {
            for c in server.listener.incoming() {
                let Ok(mut c) = c else { continue };
                if live.load(Ordering::Relaxed) >= MAX_CONNS {
                    let _ = writeln!(
                        c,
                        "{}",
                        json!({"id": null, "error": {"code": "busy", "message": "too many connections"}})
                    );
                    continue;
                }
                live.fetch_add(1, Ordering::Relaxed);
                let (sh, lv) = (shared.clone(), live.clone());
                let _ = std::thread::Builder::new()
                    .name("api-conn".into())
                    .spawn(move || {
                        conn(c, &sh);
                        lv.fetch_sub(1, Ordering::Relaxed);
                    });
            }
        });
}

/// One request line → its answer line (pure apart from the call — tested).
pub fn answer(
    line: &str,
    call: &mut dyn FnMut(&str, &Value) -> Result<Value, api::Error>,
) -> Value {
    let req: Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(e) => {
            return json!({"id": null, "error": {"code": "parse", "message": format!("not JSON: {e}")}})
        }
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return json!({"id": id, "error": {"code": "invalid", "message": "no method"}});
    };
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
    match call(method, &params) {
        Ok(v) => json!({"id": id, "result": v}),
        Err(e) => json!({"id": id, "error": {"code": e.code, "message": e.message}}),
    }
}

fn conn(s: UnixStream, shared: &Arc<Shared>) {
    let Ok(mut w) = s.try_clone() else { return };
    let mut r = BufReader::new(s);
    loop {
        let mut line = String::new();
        match r.by_ref().take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if !line.ends_with('\n') && line.len() as u64 >= MAX_LINE => {
                let _ = writeln!(
                    w,
                    "{}",
                    json!({"id": null, "error": {"code": "too_long", "message": "request line over 1 MB"}})
                );
                return;
            }
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        // a subscription (or a screen stream) keeps this connection to itself
        if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
            let m = v.get("method").and_then(Value::as_str).unwrap_or("");
            if m == "events.subscribe" || m == "stream.open" {
                let id = v.get("id").cloned().unwrap_or(Value::Null);
                let p = v.get("params").cloned().unwrap_or_else(|| json!({}));
                if m == "events.subscribe" {
                    subscribe(&mut w, id, &p, shared);
                } else {
                    stream(&mut w, id, &p, shared);
                }
                return;
            }
        }
        let cfg = Config::load();
        let ctx = api::Ctx::local(&cfg, Some(shared));
        let mut method = String::new();
        let mut a = answer(&line, &mut |m, p| {
            method = m.to_string();
            api::call(&ctx, m, p)
        });
        // showcase mode: the answer masked by field (a no-op in other builds;
        // while paused, nothing real at all — but for Aleph's own asking)
        if crate::showcase::on() || crate::showcase::paused() {
            if let Some(r) = a.get_mut("result") {
                *r = crate::showcase::answer(&method, r.take());
            }
        }
        if writeln!(w, "{a}").is_err() {
            return;
        }
    }
}

/// New events since the last look, oldest first: the newest `WINDOW` ids are
/// remembered, so events written out of order (or a rewritten log) are
/// neither lost nor sent twice.
pub struct Tail {
    seen: HashSet<String>,
    stamp: Option<(u64, std::time::SystemTime)>,
}

const WINDOW: usize = 300;

impl Tail {
    /// Start at the end: only events from now on.
    pub fn new() -> Tail {
        let mut t = Tail {
            seen: HashSet::new(),
            stamp: None,
        };
        let _ = t.next_from(&events::load(WINDOW));
        t
    }

    /// Everything after `t` (unix seconds) first, then what's new.
    pub fn since(t: u64) -> Tail {
        let mut tail = Tail {
            seen: HashSet::new(),
            stamp: None,
        };
        tail.seen = events::load(WINDOW)
            .iter()
            .filter(|e| e.t <= t)
            .map(Event::id)
            .collect();
        tail
    }

    /// New events in the log (only read when it changed).
    pub fn next(&mut self) -> Vec<Event> {
        let st = std::fs::metadata(events::file())
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?)));
        if st.is_some() && st == self.stamp {
            return Vec::new();
        }
        self.stamp = st;
        self.next_from(&events::load(WINDOW))
    }

    /// (pure — tested) `newest_first`: the log's newest events.
    pub fn next_from(&mut self, newest_first: &[Event]) -> Vec<Event> {
        let mut fresh: Vec<Event> = newest_first
            .iter()
            .filter(|e| !self.seen.contains(&e.id()))
            .cloned()
            .collect();
        fresh.reverse();
        self.seen = newest_first.iter().map(Event::id).collect();
        fresh
    }
}

impl Default for Tail {
    fn default() -> Self {
        Tail::new()
    }
}

/// One subscriber's view of the daemon (`events.subscribe` on the socket,
/// Server-Sent Events over HTTP): the fleet state when it changes, new
/// events (only `kinds`, when given), and a ping every 15 s so a dead client
/// is noticed.
pub struct Sub {
    want_state: bool,
    kinds: Option<Vec<String>>,
    tail: Tail,
    gen: u64,
    /// showcase mode's epoch when the state was last sent (a switch re-sends it)
    epoch: u64,
    ping: Instant,
    /// the topics followed (`keep::TOPICS`) and where each was last
    topics: Vec<(String, u64)>,
}

/// What a subscriber is sent.
#[derive(Debug)]
pub enum Msg {
    /// (generation, snapshot)
    State(u64, Value),
    Event(Event),
    /// unix time
    Ping(u64),
    /// what a followed topic is about changed: ask for it again
    Changed(String),
}

pub const PING: Duration = Duration::from_secs(15);

impl Sub {
    /// `{state?: bool, kinds?: [kind] | "a,b", since?: t}`
    pub fn new(p: &Value) -> Sub {
        let kinds: Option<Vec<String>> = match p.get("kinds") {
            Some(Value::Array(a)) => Some(
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            ),
            Some(Value::String(s)) if !s.trim().is_empty() => Some(
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect(),
            ),
            _ => None,
        };
        // `topics: ["usage", …]` or "all"
        let topics: Vec<String> = match p.get("topics") {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| crate::keep::TOPICS.contains(t))
                .map(str::to_string)
                .collect(),
            Some(Value::String(s)) if s == "all" => {
                crate::keep::TOPICS.iter().map(|t| t.to_string()).collect()
            }
            _ => Vec::new(),
        };
        Sub {
            topics: topics
                .into_iter()
                .map(|t| {
                    let at = crate::keep::stamp(&t);
                    (t, at)
                })
                .collect(),
            want_state: p.get("state").and_then(Value::as_bool).unwrap_or(true),
            kinds,
            tail: match p.get("since").and_then(Value::as_u64) {
                Some(t) => Tail::since(t),
                None => Tail::new(),
            },
            gen: u64::MAX,
            epoch: crate::showcase::epoch(),
            ping: Instant::now(),
        }
    }

    /// What's new; with nothing new, waits up to a second for a change first.
    pub fn next(&mut self, shared: &Shared) -> Vec<Msg> {
        let mut out = Vec::new();
        if self.want_state {
            let (g, snap) = shared.get();
            let ep = crate::showcase::epoch();
            if g != self.gen || ep != self.epoch {
                self.gen = g;
                self.epoch = ep;
                out.push(Msg::State(g, snap));
            }
        }
        for e in self.tail.next() {
            if self
                .kinds
                .as_ref()
                .is_some_and(|k| !k.iter().any(|x| x == e.kind.as_str()))
            {
                continue;
            }
            out.push(Msg::Event(e));
        }
        for (t, at) in &mut self.topics {
            let now = crate::keep::stamp(t);
            if now != *at {
                *at = now;
                out.push(Msg::Changed(t.clone()));
            }
        }
        if self.ping.elapsed() > PING {
            self.ping = Instant::now();
            out.push(Msg::Ping(crate::cache::now()));
        }
        if out.is_empty() {
            // (a subscriber without state still sleeps here between looks)
            let g = if self.want_state {
                self.gen
            } else {
                shared.generation()
            };
            shared.wait(g, Duration::from_secs(1));
        }
        out
    }
}

/// A fleet snapshot on its way to a subscriber: masked in showcase mode,
/// with a marker saying so (the web console's banner).
pub fn masked_state(snap: Value) -> Value {
    // (while paused: a snapshot with nothing real in it, marked so)
    if !crate::showcase::on() && !crate::showcase::paused() {
        return snap;
    }
    let mut v = crate::showcase::json(snap);
    crate::showcase::mark(&mut v);
    v
}

fn subscribe(w: &mut UnixStream, id: Value, p: &Value, shared: &Arc<Shared>) {
    let mut sub = Sub::new(p);
    if writeln!(w, "{}", json!({"id": id, "result": {"subscribed": true}})).is_err() {
        return;
    }
    loop {
        let mut out = String::new();
        for m in sub.next(shared) {
            let line = match m {
                Msg::State(g, snap) => json!({"id": id, "state": masked_state(snap), "gen": g}),
                Msg::Event(e) => json!({"id": id, "event": crate::showcase::json(e.to_json())}),
                Msg::Ping(t) => json!({"id": id, "ping": t}),
                Msg::Changed(t) => json!({"id": id, "changed": t}),
            };
            out.push_str(&line.to_string());
            out.push('\n');
        }
        if !out.is_empty() && w.write_all(out.as_bytes()).is_err() {
            return;
        }
    }
}

/// `stream.open`: a session's screen, sent again whenever it changes (read
/// every half second; once a second while it's quiet), until the client
/// goes or the session ends.
fn stream(w: &mut UnixStream, id: Value, p: &Value, shared: &Arc<Shared>) {
    let cfg = Config::load();
    let ctx = api::Ctx::local(&cfg, Some(shared));
    let (t, s) = match api::resolve(&ctx, p) {
        Ok(x) => x,
        Err(e) => {
            let _ = writeln!(
                w,
                "{}",
                json!({"id": id, "error": {"code": e.code, "message": e.message}})
            );
            return;
        }
    };
    let lines = p
        .get("lines")
        .and_then(Value::as_u64)
        .unwrap_or(80)
        .clamp(1, 2000) as usize;
    let hello = json!({"id": id, "result": {"streaming": true, "host": s.host, "name": s.name, "pane_id": s.pane_id}});
    if writeln!(w, "{hello}").is_err() {
        return;
    }
    let h = crate::herdr::Herdr::new(&t);
    let mut last = String::new();
    let mut quiet = 0u32;
    let mut fails = 0u32;
    let mut ping = Instant::now();
    loop {
        match h.read_pane(&s.pane_id, lines) {
            Ok(text) => {
                fails = 0;
                if text != last {
                    quiet = 0;
                    // showcase: redacted or hidden, then masked
                    let shown = crate::showcase::screen_text(&text);
                    if writeln!(
                        w,
                        "{}",
                        json!({"id": id, "screen": crate::showcase::text(&shown)})
                    )
                    .is_err()
                    {
                        return;
                    }
                    last = text;
                } else {
                    quiet += 1;
                }
            }
            Err(e) => {
                fails += 1;
                if fails >= 3 {
                    let _ = writeln!(
                        w,
                        "{}",
                        json!({"id": id, "ended": true, "why": crate::showcase::text(&e)})
                    );
                    return;
                }
            }
        }
        if ping.elapsed() > Duration::from_secs(15) {
            ping = Instant::now();
            if writeln!(w, "{}", json!({"id": id, "ping": crate::cache::now()})).is_err() {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(if quiet > 6 { 1000 } else { 500 }));
    }
}

// ------------------------------------------------------------------ clients

/// Does this process serve the socket?
pub fn ours() -> bool {
    OURS.load(Ordering::Relaxed)
}

/// Does another process (a daemon, or a dashboard serving while none runs)
/// answer on the socket?
pub fn other_running() -> bool {
    !ours() && running()
}

/// Is a daemon answering on this computer? (cached for 5 s)
pub fn running() -> bool {
    static C: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut c = C.lock().unwrap();
    if let Some((at, v)) = *c {
        if at.elapsed() < Duration::from_secs(5) {
            return v;
        }
    }
    let v = UnixStream::connect(sock_path()).is_ok();
    *c = Some((Instant::now(), v));
    v
}

/// One request to the running daemon; its result.
pub fn request(method: &str, params: Value) -> Result<Value, String> {
    request_at(&sock_path(), method, params, Duration::from_secs(30))
}

pub fn request_at(
    path: &Path,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let mut s = UnixStream::connect(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            "no daemon is running (aleph daemon start, or aleph watch)".to_string()
        }
        _ => format!("the daemon's socket: {e}"),
    })?;
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
    let id = format!("cli-{}", std::process::id());
    let line = json!({"id": id, "method": method, "params": params}).to_string() + "\n";
    s.write_all(line.as_bytes())
        .map_err(|e| format!("the daemon's socket: {e}"))?;
    let mut r = BufReader::new(s);
    let mut buf = String::new();
    r.read_line(&mut buf)
        .map_err(|e| format!("the daemon didn't answer: {e}"))?;
    let v: Value = serde_json::from_str(buf.trim()).map_err(|e| format!("the daemon: {e}"))?;
    if let Some(e) = v.get("error") {
        return Err(e
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("the daemon refused it")
            .to_string());
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

/// `aleph api --stdio` (a bridge: stdin → the socket → stdout), `aleph api
/// methods`, or `aleph api <method> [json params]` (one call, the result).
pub fn api_cmd(a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::{err, fail, json as print_json};
    use std::process::ExitCode;
    let p = a.positional();
    if a.has("--stdio") {
        return match stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                err(&e);
                ExitCode::from(1)
            }
        };
    }
    match p.first().copied() {
        None | Some("methods") => {
            if p.is_empty() && !a.has("--json") && std::io::stdout().is_terminal_like() {
                // a table for people
                crate::out::head("Aleph's API (aleph api <method> '{json}')");
                for m in api::METHODS {
                    println!(
                        "  {:<18} {:<8} {}{}",
                        m.name,
                        m.scope.as_str(),
                        m.help,
                        if m.params.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", m.params)
                        }
                    );
                }
                return ExitCode::SUCCESS;
            }
            print_json(&api::methods_json());
            ExitCode::SUCCESS
        }
        Some(m) => {
            let params = match p.get(1) {
                Some(j) => match serde_json::from_str::<Value>(j) {
                    Ok(v) => v,
                    Err(e) => return fail(&format!("params must be JSON: {e}")),
                },
                None => json!({}),
            };
            match request(m, params) {
                Ok(v) => {
                    print_json(&v);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    err(&e);
                    ExitCode::from(1)
                }
            }
        }
    }
}

trait TermLike {
    fn is_terminal_like(&self) -> bool;
}

impl TermLike for std::io::Stdout {
    fn is_terminal_like(&self) -> bool {
        use std::io::IsTerminal;
        self.is_terminal()
    }
}

/// Copy stdin to the socket and the socket to stdout until either ends.
fn stdio() -> Result<(), String> {
    let s = UnixStream::connect(sock_path())
        .map_err(|_| "no daemon is running (aleph daemon start, or aleph watch)".to_string())?;
    let mut up = s.try_clone().map_err(|e| e.to_string())?;
    let mut down = s;
    let t = std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let _ = std::io::copy(&mut stdin, &mut up);
        let _ = up.shutdown(std::net::Shutdown::Write);
    });
    let mut stdout = std::io::stdout().lock();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match down.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if stdout.write_all(&buf[..n]).is_err() || stdout.flush().is_err() {
                    break;
                }
            }
        }
    }
    drop(t);
    Ok(())
}

/// `aleph daemon [status [--json] | run | ping | token new|ls|revoke | pair]`
/// (run = `aleph watch`); the service, its logs, doctor, exposure and
/// deploys: `daemonctl.rs`; devices and pairing: `devices.rs`.
pub fn cmd(a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::err;
    use std::process::ExitCode;
    let p = a.positional();
    match p.first().copied() {
        Some("run") => {
            let rest: Vec<String> = a.rest.iter().filter(|x| *x != "run").cloned().collect();
            crate::watch::cmd(crate::cli::Args {
                cfg: a.cfg,
                flag: a.flag,
                rest: &rest,
            })
        }
        Some(sub @ ("token" | "tokens" | "pair")) => {
            // what follows the subcommand
            let i = a.rest.iter().position(|x| x == sub).map_or(0, |i| i + 1);
            let rest = a.rest[i..].to_vec();
            if sub == "pair" {
                crate::devices::pair_cmd(&a, &rest)
            } else {
                crate::devices::token_cmd(&a, &rest)
            }
        }
        None
        | Some(
            "status" | "ping" | "install" | "uninstall" | "start" | "stop" | "restart" | "logs"
            | "doctor" | "expose" | "deploy",
        ) => crate::daemonctl::cmd(a),
        Some(x) => {
            err(&format!("aleph daemon {x}: no such subcommand"));
            crate::out::hint(
                "aleph daemon status | run | install | uninstall | start | stop | restart | logs | doctor | expose | deploy | pair | token",
            );
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(t: u64, name: &str) -> Event {
        Event {
            t,
            host: "b31".into(),
            name: name.into(),
            kind: events::Kind::Blocked,
            ..Default::default()
        }
    }

    #[test]
    fn answers_lines() {
        let mut call = |m: &str, p: &Value| -> Result<Value, api::Error> {
            match m {
                "echo" => Ok(p.clone()),
                _ => Err(api::Error::new("unknown", "no such method")),
            }
        };
        let a = answer(r#"{"id":"1","method":"echo","params":{"x":2}}"#, &mut call);
        assert_eq!(a["id"], "1");
        assert_eq!(a["result"]["x"], 2);
        let a = answer(r#"{"id":7,"method":"nope"}"#, &mut call);
        assert_eq!(a["id"], 7);
        assert_eq!(a["error"]["code"], "unknown");
        assert_eq!(answer("{not json", &mut call)["error"]["code"], "parse");
        assert_eq!(answer(r#"{"id":1}"#, &mut call)["error"]["code"], "invalid");
    }

    #[test]
    fn tail_sends_each_event_once() {
        let mut t = Tail {
            seen: HashSet::new(),
            stamp: None,
        };
        // the first look starts at the end
        let _ = t.next_from(&[ev(100, "a")]);
        // two new ones, one written out of order (older than the newest seen)
        let got = t.next_from(&[ev(101, "b"), ev(99, "late"), ev(100, "a")]);
        let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["late", "b"]);
        assert!(t
            .next_from(&[ev(101, "b"), ev(99, "late"), ev(100, "a")])
            .is_empty());
    }

    #[test]
    fn only_news_moves_the_generation() {
        let sh = Shared::new();
        let snap = |lat: u64, st: &str| {
            json!({"generated_at": lat, "hosts": [{"alias": "b31", "latency_ms": lat,
                    "sessions": [{"name": "temper", "status": st}]}]})
        };
        sh.publish(snap(10, "working"), true);
        let (g, _) = sh.get();
        sh.publish(snap(42, "working"), true);
        let (g2, v) = sh.get();
        assert_eq!(g, g2);
        assert_eq!(v["hosts"][0]["latency_ms"], 42);
        sh.publish(snap(42, "blocked"), true);
        assert_eq!(sh.get().0, g + 1);
        sh.publish(snap(42, "blocked"), false);
        assert_eq!(sh.get().0, g + 2);
    }

    #[test]
    fn binding_is_the_lock() {
        let d = std::env::temp_dir().join(format!("al-dsock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("daemon.sock");
        let Bind::Ours(s) = bind_at(&p) else {
            panic!("first bind")
        };
        // someone answers there: taken (with our pid)
        assert!(matches!(bind_at(&p), Bind::Taken(Some(x)) if x == std::process::id()));
        // the daemon died without cleaning up: stale, so it's taken over
        drop(s);
        assert!(p.exists());
        // (another test's child forked in that instant may hold the socket
        // until it execs: retry briefly)
        let mut r = bind_at(&p);
        for _ in 0..40 {
            if matches!(r, Bind::Ours(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
            r = bind_at(&p);
        }
        assert!(matches!(r, Bind::Ours(_)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn requests_round_trip() {
        let d = std::env::temp_dir().join(format!("al-dreq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("daemon.sock");
        let Bind::Ours(s) = bind_at(&p) else {
            panic!("bind")
        };
        let sh = Shared::new();
        sh.publish(json!({"version": 1, "hosts": [{"alias": "b31"}]}), true);
        serve(s, sh);
        let v = request_at(&p, "daemon.ping", json!({}), Duration::from_secs(5)).unwrap();
        assert_eq!(v["pong"], true);
        let v = request_at(&p, "state.snapshot", json!({}), Duration::from_secs(5)).unwrap();
        assert_eq!(v["hosts"][0]["alias"], "b31");
        let e = request_at(&p, "no.such", json!({}), Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("no such method"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
