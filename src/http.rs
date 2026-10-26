//! The daemon's HTTP API and web console (`api.http = on`, `api.listen`).
//!
//! The same methods as the socket, through the same table (`api.rs`):
//!
//! * `POST /api/v1/<method>` with a JSON body, or `GET /api/v1/<method>?k=v`
//!   for read-scope methods. The answer is `{"result": …}`, or
//!   `{"error": {code, message}}` with a matching status.
//! * `GET /api/v1/events`: Server-Sent Events, mirroring `events.subscribe`
//!   (`state` and `event` messages, a `: ping` every 15 s; a thread per
//!   client, at most 16).
//! * `GET /metrics`: Prometheus text (sessions by machine and state, machines
//!   up, Inbox events by kind, the queue, the hub, uptime).
//! * Devices: `POST /api/v1/pair` (a pairing code for a token), `GET
//!   /api/v1/me`, `POST /api/v1/login` / `logout` (the cookie), `GET
//!   /api/v1/push/key`, `POST /api/v1/push/subscribe|unsubscribe|test`.
//! * `POST /a/<token>` (or the token as the body): a signed action token from
//!   a notification button (`actions.rs`).
//! * `/` and its files: the web console, embedded in the binary.
//!
//! Every caller is remote (`Ctx::remote`): held to `remote.allow` and
//! audited in the Inbox by `api.rs`. Protections, in order: the Host header
//! must name this computer (localhost, `api.url`, `api.hosts`: no DNS
//! rebinding), a budget per client address, every POST must carry
//! `X-Aleph: 1` and no foreign `Origin` (no CORS headers are ever sent, so
//! another site can't add the header), a device token (`Authorization:
//! Bearer`, or the HttpOnly SameSite=Strict cookie set at pairing — what
//! EventSource uses), a budget per token, a tighter one for wrong tokens and
//! pairing attempts, and bodies of at most 256 KB.
//!
//! tiny_http has no socket timeouts (it accepts connections itself) and
//! would allocate a whole unread `Content-Length` when dropping a request:
//! those requests are never dropped (they're left unanswered), and every
//! request gets a thread of its own, so a slow client only holds itself up.
//! Keep `api.listen` on 127.0.0.1 and put `tailscale serve` or `cloudflared`
//! in front: they enforce header and body limits for anything from outside.

use crate::api::{self, Scope};
use crate::config::Config;
use crate::daemon::{Msg, Shared, Sub};
use crate::devices::{self, Device};
use crate::webpush;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Request bodies above this are refused.
pub const MAX_BODY: usize = 256 * 1024;
/// Open event streams at once.
pub const MAX_SSE: usize = 16;
const MAX_URL: usize = 8 * 1024;
const COOKIE: &str = "aleph_token";

// ------------------------------------------------------------------ budgets

/// A token bucket's size and refill rate.
#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub burst: f64,
    pub per_sec: f64,
}

/// Any request, per client address.
pub const PER_IP: Rate = Rate {
    burst: 240.0,
    per_sec: 8.0,
};
/// Any request, per device token.
pub const PER_TOKEN: Rate = Rate {
    burst: 120.0,
    per_sec: 4.0,
};
/// Wrong tokens, per client address: ten, then one every 30 s.
pub const FAILS: Rate = Rate {
    burst: 10.0,
    per_sec: 1.0 / 30.0,
};
/// Pairing attempts, per client address: five, then one a minute.
pub const PAIRING: Rate = Rate {
    burst: 5.0,
    per_sec: 1.0 / 60.0,
};
/// Pairing attempts from everywhere together.
pub const PAIRING_ALL: Rate = Rate {
    burst: 20.0,
    per_sec: 1.0 / 15.0,
};

/// Token buckets by key (`ip:…`, `tok:…`, `fail:…`, `pair:…`).
#[derive(Default)]
pub struct Limits {
    b: HashMap<String, (f64, Instant)>,
}

impl Limits {
    fn level(&mut self, key: &str, r: Rate, now: Instant) -> &mut (f64, Instant) {
        if self.b.len() > 4096 {
            // forget clients idle long enough to be full again
            self.b
                .retain(|_, (_, t)| now.saturating_duration_since(*t) < Duration::from_secs(600));
        }
        let e = self.b.entry(key.to_string()).or_insert((r.burst, now));
        let dt = now.saturating_duration_since(e.1).as_secs_f64();
        e.0 = (e.0 + dt * r.per_sec).min(r.burst);
        e.1 = now;
        e
    }

    /// Spend one (false: the budget is used up).
    pub fn take(&mut self, key: &str, r: Rate, now: Instant) -> bool {
        let e = self.level(key, r, now);
        if e.0 >= 1.0 {
            e.0 -= 1.0;
            true
        } else {
            false
        }
    }

    /// Is there one left (without spending it)?
    pub fn has(&mut self, key: &str, r: Rate, now: Instant) -> bool {
        self.level(key, r, now).0 >= 1.0
    }
}

// ------------------------------------------------------------------ state

pub struct State {
    pub shared: Arc<Shared>,
    pub devices: devices::Store,
    pub push: webpush::Store,
    /// the address it listens on (`127.0.0.1:7878`)
    pub bound: Mutex<String>,
    limits: Mutex<Limits>,
    sse: AtomicUsize,
    oversized: AtomicUsize,
}

impl State {
    pub fn new(shared: Arc<Shared>, bound: String) -> State {
        State {
            shared,
            devices: devices::Store::new(),
            push: webpush::Store::new(),
            bound: Mutex::new(bound),
            limits: Mutex::new(Limits::default()),
            sse: AtomicUsize::new(0),
            oversized: AtomicUsize::new(0),
        }
    }

    fn bound(&self) -> String {
        self.bound.lock().unwrap().clone()
    }
}

// ------------------------------------------------------------------ starting

/// `api.listen`, checked: this computer only, unless `api.listen_any`.
pub fn listen_addr(cfg: &Config) -> Result<SocketAddr, String> {
    let raw = cfg.get_or("api.listen");
    let s = raw.trim();
    let s = if let Some(p) = s.strip_prefix(':') {
        format!("127.0.0.1:{p}")
    } else if s.parse::<u16>().is_ok() {
        format!("127.0.0.1:{s}")
    } else if let Some(p) = s.strip_prefix("localhost:") {
        format!("127.0.0.1:{p}")
    } else {
        s.to_string()
    };
    let a: SocketAddr = s
        .parse()
        .map_err(|_| format!("api.listen = {raw}: give an address like 127.0.0.1:7878"))?;
    if !a.ip().is_loopback() && !cfg.flag("api.listen_any") {
        return Err(format!(
            "api.listen = {raw} can be reached from other machines: set api.listen_any = true to allow it (better: keep 127.0.0.1 and put tailscale serve in front)"
        ));
    }
    Ok(a)
}

fn listen(addr: SocketAddr) -> Result<(tiny_http::Server, String), String> {
    let server = tiny_http::Server::http(addr).map_err(|e| e.to_string())?;
    let bound = server
        .server_addr()
        .to_ip()
        .map(|a| a.to_string())
        .unwrap_or_else(|| addr.to_string());
    Ok((server, bound))
}

/// Serve the API and console when `api.http = on` (the daemon calls this
/// once it holds its socket). Problems are logged and said; the daemon
/// carries on without HTTP.
pub fn start(cfg: &Config, shared: &Arc<Shared>) {
    if !cfg.flag("api.http") {
        return;
    }
    let (server, bound) = match listen_addr(cfg).and_then(listen) {
        Ok(x) => x,
        Err(e) => {
            crate::log::warn("http", &format!("no HTTP API: {e}"));
            crate::out::warn(&format!("no HTTP API: {e}"));
            return;
        }
    };
    let addr: SocketAddr = bound
        .parse()
        .unwrap_or_else(|_| "127.0.0.1:7878".parse().unwrap());
    crate::log::info("http", &format!("API and web console on http://{bound}"));
    shared.set_http(Some(bound.clone()));
    let st = Arc::new(State::new(shared.clone(), bound));
    let _ = std::thread::Builder::new()
        .name("http".into())
        .spawn(move || run(server, addr, st));
}

fn run(mut server: tiny_http::Server, addr: SocketAddr, st: Arc<State>) {
    loop {
        match server.recv() {
            Ok(rq) => {
                let st = st.clone();
                // a thread per request: a slow client holds up only itself
                let _ = std::thread::Builder::new()
                    .name("http-req".into())
                    .spawn(move || handle(rq, &st));
            }
            Err(e) => {
                // tiny_http's accept loop stopped (out of file descriptors,
                // …): listen again once that's possible
                crate::log::warn("http", &format!("stopped accepting ({e}); listening again"));
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    if let Ok((s, b)) = listen(addr) {
                        server = s;
                        st.shared.set_http(Some(b.clone()));
                        *st.bound.lock().unwrap() = b;
                        break;
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ requests

/// A request as the router sees it (tests build these by hand).
#[derive(Clone, Debug, Default)]
pub struct Head {
    pub method: String,
    /// as sent (not percent-decoded)
    pub path: String,
    pub query: String,
    /// names in lower case
    pub headers: Vec<(String, String)>,
    pub peer: Option<IpAddr>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn of(rq: &tiny_http::Request) -> Head {
        let (path, query) = match rq.url().split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (rq.url().to_string(), String::new()),
        };
        Head {
            method: rq.method().as_str().to_uppercase(),
            path,
            query,
            headers: rq
                .headers()
                .iter()
                .map(|h| {
                    (
                        h.field.as_str().as_str().to_lowercase(),
                        h.value.as_str().to_string(),
                    )
                })
                .collect(),
            peer: rq.remote_addr().map(|a| a.ip()),
        }
    }

    /// Came over https (to the proxy in front of us)?
    fn https(&self) -> bool {
        self.header("x-forwarded-proto")
            .is_some_and(|p| p.eq_ignore_ascii_case("https"))
            || self
                .header("forwarded")
                .is_some_and(|f| f.to_lowercase().contains("proto=https"))
    }
}

/// An answer. `sse`: hand the connection to an event stream with these
/// parameters instead.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub sse: Option<Value>,
}

impl Reply {
    fn with(mut self, k: &str, v: impl Into<String>) -> Reply {
        self.headers.push((k.to_string(), v.into()));
        self
    }
    /// The JSON body (tests).
    #[cfg(test)]
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

fn reply_json(status: u16, v: Value) -> Reply {
    Reply {
        status,
        headers: vec![
            (
                "Content-Type".into(),
                "application/json; charset=utf-8".into(),
            ),
            ("Cache-Control".into(), "no-store".into()),
        ],
        // showcase mode: masked by field (a no-op in other builds)
        body: crate::showcase::json(v).to_string().into_bytes(),
        sse: None,
    }
}

fn ok(v: Value) -> Reply {
    reply_json(200, json!({"result": v}))
}

pub fn error(status: u16, code: &str, message: impl Into<String>) -> Reply {
    reply_json(
        status,
        json!({"error": {"code": code, "message": message.into()}}),
    )
}

fn limited(what: &str) -> Reply {
    error(429, "rate", what).with("Retry-After", "30")
}

/// The HTTP status for an API error code.
pub fn status_of(code: &str) -> u16 {
    match code {
        "invalid" | "parse" => 400,
        "unauthorized" => 401,
        "forbidden" | "csrf" => 403,
        "unknown" | "not_found" => 404,
        "method" => 405,
        "failed" => 409,
        "too_large" => 413,
        "rate" => 429,
        "busy" => 503,
        _ => 500,
    }
}

/// One request, start to finish (its own thread).
fn handle(mut rq: tiny_http::Request, st: &State) {
    // tiny_http would allocate a whole unread Content-Length when the request
    // is dropped (a crash for a huge one): leave such a request unanswered
    if rq.body_length().is_some_and(|n| n > MAX_BODY) {
        if st
            .oversized
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(100)
        {
            crate::log::warn(
                "http",
                &format!(
                    "refused a {} byte request from {:?} (unanswered)",
                    rq.body_length().unwrap_or(0),
                    rq.remote_addr()
                ),
            );
        }
        std::mem::forget(rq);
        return;
    }
    if rq.url().len() > MAX_URL || rq.headers().len() > 100 {
        respond(rq, error(431, "too_large", "request header too large"));
        return;
    }
    let head = Head::of(&rq);
    let mut body = Vec::new();
    if head.method == "POST" || head.method == "PUT" || head.method == "PATCH" {
        if rq
            .as_reader()
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut body)
            .is_err()
        {
            respond(rq, error(400, "invalid", "couldn't read the body"));
            return;
        }
        if body.len() > MAX_BODY {
            respond(rq, error(413, "too_large", "bodies are at most 256 KB"));
            return;
        }
    }
    let cfg = Config::load();
    let r = route(st, &cfg, &head, &body, Instant::now());
    match r.sse {
        Some(p) => sse(rq, st, p),
        None => respond(rq, r),
    }
}

/// Headers on everything.
const SAFE: &[(&str, &str)] = &[
    ("Server", "aleph"),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("X-Frame-Options", "DENY"),
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
];

fn respond(rq: tiny_http::Request, r: Reply) {
    let mut resp = tiny_http::Response::from_data(r.body).with_status_code(r.status);
    for (k, v) in SAFE
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .chain(r.headers)
    {
        if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp.add_header(h);
        }
    }
    let _ = rq.respond(resp);
}

// ------------------------------------------------------------------ routing

/// The name in a Host header (or an Origin's authority): no port, no
/// brackets, lower case.
pub fn host_name(h: &str) -> String {
    let h = h.trim();
    let h = h
        .strip_prefix("http://")
        .or_else(|| h.strip_prefix("https://"))
        .unwrap_or(h);
    let h = h.split('/').next().unwrap_or("");
    let name = if let Some(rest) = h.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        match h.rsplit_once(':') {
            Some((a, p)) if !a.contains(':') && p.chars().all(|c| c.is_ascii_digit()) => a,
            _ => h,
        }
    };
    name.trim_end_matches('.').to_lowercase()
}

/// May a request name this host? This computer's own names, the address
/// it listens on, `api.url`'s host and `api.hosts` — anything else is a
/// page elsewhere pointing a name at us (DNS rebinding).
pub fn host_ok(cfg: &Config, bound: &str, host: &str) -> bool {
    let name = host_name(host);
    if name.is_empty() {
        return false;
    }
    if matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    let b = host_name(bound);
    if b == name && b != "0.0.0.0" && b != "::" {
        return true;
    }
    if cfg
        .get("api.url")
        .is_some_and(|u| !u.trim().is_empty() && host_name(&u) == name)
    {
        return true;
    }
    cfg.get_or("api.hosts")
        .split(',')
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .any(|p| match p.strip_prefix("*.") {
            Some(suffix) => name.ends_with(&format!(".{suffix}")),
            None => host_name(&p) == name,
        })
}

/// Who a client is, for its budget. Behind tailscale serve or cloudflared
/// everyone comes from 127.0.0.1, so there it's what the proxy says.
fn client(h: &Head) -> String {
    if h.peer.is_none_or(|p| p.is_loopback()) {
        let fwd = h.header("cf-connecting-ip").or_else(|| {
            h.header("x-forwarded-for")
                .and_then(|x| x.split(',').next())
        });
        if let Some(f) = fwd.map(str::trim).filter(|f| !f.is_empty()) {
            return format!("fwd:{}", crate::ui::trunc(f, 64));
        }
    }
    h.peer.map(|p| p.to_string()).unwrap_or_else(|| "?".into())
}

/// Every POST: `X-Aleph: 1` (a browser only adds it cross-site after a CORS
/// preflight, which is never granted) and no other site's Origin.
pub fn csrf(cfg: &Config, bound: &str, h: &Head) -> Result<(), Reply> {
    if h.header("x-aleph").map(str::trim) != Some("1") {
        return Err(error(
            403,
            "csrf",
            "POSTs need the X-Aleph: 1 header (the console sends it)",
        ));
    }
    if let Some(o) = h.header("origin") {
        if o.trim() == "null" || !host_ok(cfg, bound, o) {
            return Err(error(403, "csrf", "that request came from another site"));
        }
    }
    if h.header("sec-fetch-site") == Some("cross-site") {
        return Err(error(403, "csrf", "that request came from another site"));
    }
    Ok(())
}

fn bearer(h: &Head) -> Option<String> {
    let a = h.header("authorization")?.trim();
    let (scheme, tok) = a.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| tok.trim().to_string())
}

fn cookie(h: &Head, name: &str) -> Option<String> {
    h.headers
        .iter()
        .filter(|(k, _)| k == "cookie")
        .flat_map(|(_, v)| v.split(';'))
        .find_map(|c| {
            let (k, v) = c.trim().split_once('=')?;
            (k == name).then(|| v.trim().to_string())
        })
}

/// The cookie the console (and its EventSource) signs in with.
pub fn set_cookie(token: &str, secure: bool) -> (String, String) {
    (
        "Set-Cookie".into(),
        format!(
            "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=34560000{}",
            if secure { "; Secure" } else { "" }
        ),
    )
}

fn clear_cookie() -> (String, String) {
    (
        "Set-Cookie".into(),
        format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    )
}

/// The device behind a request (a Bearer token, else the cookie).
fn auth(st: &State, h: &Head, now: Instant) -> Result<Device, Reply> {
    let from_cookie = bearer(h).is_none();
    let Some(tok) = bearer(h).or_else(|| cookie(h, COOKIE)) else {
        return Err(error(
            401,
            "unauthorized",
            "sign in first: pair this device (aleph daemon pair, on the computer), or send Authorization: Bearer <token>",
        )
        .with("WWW-Authenticate", "Bearer realm=\"aleph\""));
    };
    let key = format!("fail:{}", client(h));
    if !st.limits.lock().unwrap().has(&key, FAILS, now) {
        return Err(limited(
            "too many wrong tokens from here: wait a few minutes",
        ));
    }
    match st.devices.check(&tok) {
        Some(d) => {
            st.devices.touch(&d.name, crate::cache::now());
            Ok(d)
        }
        None => {
            st.limits.lock().unwrap().take(&key, FAILS, now);
            let mut r = error(
                401,
                "unauthorized",
                "that token isn't valid (revoked, or from another computer): pair this device again",
            )
            .with("WWW-Authenticate", "Bearer realm=\"aleph\", error=\"invalid_token\"");
            if from_cookie {
                r.headers.push(clear_cookie());
            }
            Err(r)
        }
    }
}

/// `web · pixel`, plus the tailnet user behind tailscale serve when
/// `api.trust_tailscale` (for the audit line only).
fn who(cfg: &Config, h: &Head, d: &Device) -> String {
    let mut w = format!("web · {}", d.name);
    if cfg.flag("api.trust_tailscale") && h.peer.is_some_and(|p| p.is_loopback()) {
        if let Some(u) = h
            .header("tailscale-user-login")
            .map(str::trim)
            .filter(|u| !u.is_empty() && u.len() <= 100 && !u.chars().any(char::is_control))
        {
            w.push_str(&format!(" ({u} via tailscale)"));
        }
    }
    w
}

/// `%3A` → `:`, `+` → a space (a query string's encoding).
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |x: u8| (x as char).to_digit(16);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h * 16 + l) as u8);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `?n=20&unread=true&session=b31:temper` → params (numbers, booleans and
/// JSON arrays as such; `params=<json>` for anything else).
pub fn query_params(q: &str) -> Result<Value, Reply> {
    let mut m = serde_json::Map::new();
    for kv in q.split('&').filter(|x| !x.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        let (k, v) = (pct_decode(k), pct_decode(v));
        if k == "params" {
            match serde_json::from_str::<Value>(&v) {
                Ok(Value::Object(o)) => m.extend(o),
                _ => return Err(error(400, "invalid", "params: a JSON object")),
            }
            continue;
        }
        let looks = v == "true"
            || v == "false"
            || v == "null"
            || v.starts_with('[')
            || v.starts_with('{')
            || (v.parse::<f64>().is_ok() && !v.starts_with('+') && !v.ends_with('.'));
        let val = if looks {
            serde_json::from_str(&v).unwrap_or(Value::String(v))
        } else {
            Value::String(v)
        };
        m.insert(k, val);
    }
    Ok(Value::Object(m))
}

/// Where a request goes (pure apart from the method it calls — tested).
pub fn route(st: &State, cfg: &Config, h: &Head, body: &[u8], now: Instant) -> Reply {
    let bound = st.bound();
    // 1. the Host header names this computer (DNS rebinding)
    let Some(host) = h.header("host") else {
        return error(400, "invalid", "no Host header");
    };
    if !host_ok(cfg, &bound, host) {
        return error(
            421,
            "host",
            format!(
                "{} doesn't answer to {}: add it to api.hosts (or set api.url) on the computer",
                crate::brand::NAME,
                host_name(host)
            ),
        );
    }
    // 2. a budget per client
    if !st
        .limits
        .lock()
        .unwrap()
        .take(&format!("ip:{}", client(h)), PER_IP, now)
    {
        return limited("too many requests: slow down");
    }
    let path = h.path.as_str();
    let get = matches!(h.method.as_str(), "GET" | "HEAD");
    let api_path =
        path.starts_with("/api/") || path == "/metrics" || path == "/a" || path.starts_with("/a/");
    // 3. the console's own files need no token
    if !api_path {
        return if get {
            asset(path, h)
        } else {
            error(405, "method", "the console's files are GET only").with("Allow", "GET, HEAD")
        };
    }
    match h.method.as_str() {
        "GET" | "HEAD" => {}
        // 4. every POST: the CSRF header, no foreign Origin
        "POST" => {
            if let Err(r) = csrf(cfg, &bound, h) {
                return r;
            }
        }
        // no CORS: a preflight is never granted
        _ => return error(405, "method", "GET or POST").with("Allow", "GET, POST"),
    }
    // no token needed: pairing (a code), signing out, notification buttons
    match path {
        "/api/v1/pair" if !get => return pair(st, h, body, now),
        "/api/v1/logout" if !get => {
            let mut r = ok(json!({"signed_out": true}));
            r.headers.push(clear_cookie());
            return r;
        }
        "/a" | "/a/" if !get => {
            return action(st, cfg, h, String::from_utf8_lossy(body).trim(), now)
        }
        p if p.starts_with("/a/") && !get => return action(st, cfg, h, &p[3..], now),
        _ => {}
    }
    // 5. everything else: a device
    let dev = match auth(st, h, now) {
        Ok(d) => d,
        Err(r) => return r,
    };
    if !st
        .limits
        .lock()
        .unwrap()
        .take(&format!("tok:{}", &dev.hash[..16]), PER_TOKEN, now)
    {
        return limited("too many requests from this device: slow down");
    }
    let who = who(cfg, h, &dev);
    let unix = crate::cache::now();
    match (path, get) {
        ("/metrics", true) => Reply {
            status: 200,
            headers: vec![
                (
                    "Content-Type".into(),
                    "text/plain; version=0.0.4; charset=utf-8".into(),
                ),
                ("Cache-Control".into(), "no-store".into()),
            ],
            body: metrics(st, cfg).into_bytes(),
            sse: None,
        },
        ("/api/v1/me", true) => {
            let mut me = json!({
                "name": dev.name,
                "scope": dev.scope.as_str(),
                "version": env!("CARGO_PKG_VERSION"),
                "computer": crate::auto::this_machine(),
                "http": bound,
                "url": cfg.get("api.url"),
                "push": st.push.list().iter().filter(|s| s.device == dev.name).count(),
                "remote": {"allow": cfg.get_or("remote.allow"), "deny": cfg.get_or("remote.deny")},
                "who": who,
            });
            // showcase mode says so (the console's banner); nothing otherwise
            crate::showcase::mark(&mut me);
            // ... with your watermark, when it's shown there
            crate::showcase::stamp(cfg, &mut me);
            ok(me)
        }
        ("/api/v1/login", false) => {
            // the cookie for this token (EventSource can't send headers)
            let mut r = ok(json!({"name": dev.name, "scope": dev.scope.as_str()}));
            if let Some(t) = bearer(h) {
                r.headers.push(set_cookie(&t, h.https()));
            }
            r
        }
        ("/api/v1/events" | "/api/v1/events.subscribe", true) => {
            let mut p = match query_params(&h.query) {
                Ok(p) => p,
                Err(r) => return r,
            };
            // a reconnecting EventSource says where it was
            if p.get("since").is_none() {
                if let Some(t) = h
                    .header("last-event-id")
                    .and_then(|t| t.trim().parse::<u64>().ok())
                {
                    p["since"] = json!(t);
                }
            }
            Reply {
                status: 200,
                headers: vec![],
                body: vec![],
                sse: Some(p),
            }
        }
        ("/api/v1/push/key", true) => match webpush::Vapid::load() {
            Ok(v) => ok(json!({"key": v.public_b64()})),
            Err(e) => error(500, "failed", format!("no Web Push key: {e}")),
        },
        ("/api/v1/push/subscribe", false) => {
            let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            match webpush::parse_sub(&dev.name, &v, unix) {
                Ok(s) => match st.push.add(s) {
                    Ok(()) => {
                        crate::log::info(
                            "webpush",
                            &format!("{} turned notifications on", dev.name),
                        );
                        ok(json!({"subscribed": true, "device": dev.name}))
                    }
                    Err(e) => error(500, "failed", e),
                },
                Err(e) => error(400, "invalid", e),
            }
        }
        ("/api/v1/push/unsubscribe", false) => {
            let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            let ep = v.get("endpoint").and_then(Value::as_str).unwrap_or("");
            ok(json!({"removed": st.push.remove(&dev.name, ep)}))
        }
        ("/api/v1/push/test", false) => {
            let e = crate::events::Event {
                t: unix,
                host: crate::brand::NAME.into(),
                kind: crate::events::Kind::Remote,
                title: format!("notifications work on {}", dev.name),
                source: "aleph".into(),
                ..Default::default()
            };
            match webpush::notify_device(cfg, &dev.name, &e) {
                Ok(n) => ok(json!({"sent": n})),
                Err(e) => error(409, "failed", e),
            }
        }
        (p, _) if p.starts_with("/api/v1/") => call(st, cfg, h, body, &dev, who, &p[8..]),
        _ => error(404, "not_found", format!("nothing at {path}")),
    }
}

/// One API method (the same table as the socket's).
fn call(
    st: &State,
    cfg: &Config,
    h: &Head,
    body: &[u8],
    dev: &Device,
    who: String,
    name: &str,
) -> Reply {
    let Some(m) = api::method(name) else {
        return error(404, "unknown", format!("no such method: {name}"));
    };
    let params = if matches!(h.method.as_str(), "GET" | "HEAD") {
        if m.scope > Scope::Read {
            return error(405, "method", format!("{name} changes things: POST it"))
                .with("Allow", "POST");
        }
        match query_params(&h.query) {
            Ok(p) => p,
            Err(r) => return r,
        }
    } else if body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        match serde_json::from_slice::<Value>(body) {
            Ok(v @ Value::Object(_)) => v,
            _ => return error(400, "invalid", "the body must be a JSON object"),
        }
    };
    let ctx = api::Ctx {
        cfg,
        shared: Some(&st.shared),
        scope: dev.scope,
        remote: true,
        who,
    };
    match api::call(&ctx, name, &params) {
        Ok(v) => ok(v),
        Err(e) => error(status_of(e.code), e.code, e.message),
    }
}

fn pair(st: &State, h: &Head, body: &[u8], now: Instant) -> Reply {
    {
        let mut l = st.limits.lock().unwrap();
        if !l.take(&format!("pair:{}", client(h)), PAIRING, now)
            || !l.take("pair:*", PAIRING_ALL, now)
        {
            return limited("too many pairing attempts: wait a minute");
        }
    }
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code = v.get("code").and_then(Value::as_str).unwrap_or("");
    let want = v
        .get("name")
        .and_then(Value::as_str)
        .map(|n| n.trim().to_lowercase().replace(' ', "-"))
        .filter(|n| devices::valid_name(n))
        .unwrap_or_else(|| devices::guess_name(h.header("user-agent").unwrap_or("")).to_string());
    match st.devices.pair_redeem(code, &want, crate::cache::now()) {
        Ok((d, tok)) => {
            devices::audit(format!("paired {} ({} scope)", d.name, d.scope.as_str()));
            crate::log::info("http", &format!("paired {} ({})", d.name, d.scope.as_str()));
            let mut r = ok(json!({"token": tok, "name": d.name, "scope": d.scope.as_str()}));
            r.headers.push(set_cookie(&tok, h.https()));
            r
        }
        Err(devices::PairError::Busy(e)) => error(500, "failed", e),
        Err(e) => error(403, "pair", e.message()),
    }
}

/// A notification button (Web Push's service worker, or ntfy): the signed
/// token itself is the permission; a device token, when sent, must be
/// allowed to reply, and names who did it.
fn action(st: &State, cfg: &Config, h: &Head, token: &str, now: Instant) -> Reply {
    let mut by = "a notification button".to_string();
    if bearer(h).is_some() || cookie(h, COOKIE).is_some() {
        match auth(st, h, now) {
            Ok(d) if d.scope >= Scope::Reply => by = who(cfg, h, &d),
            Ok(d) => {
                return error(
                    403,
                    "forbidden",
                    format!(
                        "{} can only look (read scope): pair it again with --scope reply",
                        d.name
                    ),
                )
            }
            Err(r) => return r,
        }
    }
    let key = format!("fail:{}", client(h));
    if !st.limits.lock().unwrap().has(&key, FAILS, now) {
        return limited("too many bad buttons from here: wait a few minutes");
    }
    let sessions = api::sessions_of(&st.shared.get().1);
    match crate::actions::handle_from(cfg, &sessions, token, &by) {
        Ok(m) => ok(json!({"done": m})),
        Err(e) => {
            if e.contains("forged") || e.contains("not an Aleph") || e.contains("bad token") {
                st.limits.lock().unwrap().take(&key, FAILS, now);
            }
            error(409, "failed", e)
        }
    }
}

// ------------------------------------------------------------------ events

/// One Server-Sent Events message (`data` split on newlines, as SSE wants).
pub fn sse_frame(event: Option<&str>, id: Option<&str>, data: &str) -> String {
    let mut s = String::new();
    if let Some(e) = event {
        s.push_str("event: ");
        s.push_str(e);
        s.push('\n');
    }
    if let Some(i) = id {
        s.push_str("id: ");
        s.push_str(i);
        s.push('\n');
    }
    for l in data.split('\n') {
        s.push_str("data: ");
        s.push_str(l.trim_end_matches('\r'));
        s.push('\n');
    }
    s.push('\n');
    s
}

/// A subscriber message as SSE: `state` (the snapshot, with `gen`),
/// `event` (with its time as the id, so a reconnect resumes there), pings
/// as comments.
pub fn sse_msg(m: &Msg) -> String {
    match m {
        Msg::State(g, snap) => {
            let mut v = crate::daemon::masked_state(snap.clone());
            v["gen"] = json!(g);
            sse_frame(Some("state"), None, &v.to_string())
        }
        Msg::Event(e) => sse_frame(
            Some("event"),
            Some(&e.t.to_string()),
            &crate::showcase::json(e.to_json()).to_string(),
        ),
        Msg::Ping(t) => format!(": ping {t}\n\n"),
        Msg::Changed(t) => sse_frame(Some("changed"), None, &json!({"topic": t}).to_string()),
    }
}

/// Counts an open stream for as long as it lives.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn sse(rq: tiny_http::Request, st: &State, p: Value) {
    if st.sse.fetch_add(1, Ordering::Relaxed) >= MAX_SSE {
        st.sse.fetch_sub(1, Ordering::Relaxed);
        respond(
            rq,
            error(503, "busy", "too many open event streams").with("Retry-After", "10"),
        );
        return;
    }
    let _slot = Slot(&st.sse);
    // the stream is ours from here: no length, closed when it ends
    let mut w = rq.into_writer();
    let mut head = String::from("HTTP/1.1 200 OK\r\n");
    for (k, v) in SAFE.iter().chain(&[
        ("Content-Type", "text/event-stream; charset=utf-8"),
        ("Cache-Control", "no-store"),
        ("Connection", "close"),
        ("X-Accel-Buffering", "no"),
    ]) {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\nretry: 3000\n\n");
    if w.write_all(head.as_bytes())
        .and_then(|_| w.flush())
        .is_err()
    {
        return;
    }
    let mut sub = Sub::new(&p);
    loop {
        let out: String = sub.next(&st.shared).iter().map(sse_msg).collect();
        if !out.is_empty() && w.write_all(out.as_bytes()).and_then(|_| w.flush()).is_err() {
            return;
        }
    }
}

// ------------------------------------------------------------------ metrics

/// `name{labels} value` lines with their HELP / TYPE.
struct Prom(String);

impl Prom {
    fn family(&mut self, name: &str, kind: &str, help: &str) {
        self.0
            .push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n"));
    }
    fn sample(&mut self, name: &str, labels: &[(&str, &str)], v: f64) {
        self.0.push_str(name);
        if !labels.is_empty() {
            let l: Vec<String> = labels
                .iter()
                .map(|(k, v)| format!("{k}=\"{}\"", prom_escape(v)))
                .collect();
            self.0.push('{');
            self.0.push_str(&l.join(","));
            self.0.push('}');
        }
        if v.fract() == 0.0 && v.abs() < 1e15 {
            self.0.push_str(&format!(" {}\n", v as i64));
        } else {
            self.0.push_str(&format!(" {v}\n"));
        }
    }
    fn one(&mut self, name: &str, kind: &str, help: &str, v: f64) {
        self.family(name, kind, help);
        self.sample(name, &[], v);
    }
}

/// A Prometheus label value: `\`, `"` and newlines escaped.
pub fn prom_escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// The Prometheus text for this daemon (pure apart from reading the state
/// files — tested).
pub fn metrics(st: &State, _cfg: &Config) -> String {
    let (_, snap) = st.shared.get();
    let hosts = snap
        .get("hosts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut p = Prom(String::new());
    p.family("aleph_build_info", "gauge", "The daemon's version.");
    p.sample(
        "aleph_build_info",
        &[("version", env!("CARGO_PKG_VERSION"))],
        1.0,
    );
    p.one(
        "aleph_uptime_seconds",
        "gauge",
        "How long the daemon has been running.",
        st.shared.uptime().as_secs() as f64,
    );
    p.one(
        "aleph_hub",
        "gauge",
        "1 when this daemon runs the hub (automation, notifications).",
        if st.shared.hub() { 1.0 } else { 0.0 },
    );
    p.one(
        "aleph_hub_pid",
        "gauge",
        "The pid of the process running the hub (0: none).",
        crate::auto::HubLock::holder().unwrap_or(0) as f64,
    );
    let up = |h: &Value| h.get("reachable").and_then(Value::as_bool) == Some(true);
    p.one(
        "aleph_hosts",
        "gauge",
        "Machines in the fleet.",
        hosts.len() as f64,
    );
    p.one(
        "aleph_hosts_up",
        "gauge",
        "Machines that answer.",
        hosts.iter().filter(|h| up(h)).count() as f64,
    );
    let alias = |h: &Value| {
        h.get("alias")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    p.family("aleph_host_up", "gauge", "1 when the machine answers.");
    for h in &hosts {
        p.sample(
            "aleph_host_up",
            &[("host", &alias(h))],
            if up(h) { 1.0 } else { 0.0 },
        );
    }
    p.family(
        "aleph_host_latency_seconds",
        "gauge",
        "The machine's last round trip.",
    );
    for h in &hosts {
        if let Some(ms) = h.get("latency_ms").and_then(Value::as_u64) {
            p.sample(
                "aleph_host_latency_seconds",
                &[("host", &alias(h))],
                ms as f64 / 1000.0,
            );
        }
    }
    p.family("aleph_sessions", "gauge", "Sessions by machine and state.");
    for h in &hosts {
        let ss = h
            .get("sessions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut states: Vec<String> = ["working", "blocked", "done", "idle"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for s in &ss {
            if let Some(x) = s.get("status").and_then(Value::as_str) {
                if !states.iter().any(|y| y == x) {
                    states.push(x.to_string());
                }
            }
        }
        for state in &states {
            let n = ss
                .iter()
                .filter(|s| s.get("status").and_then(Value::as_str) == Some(state.as_str()))
                .count();
            p.sample(
                "aleph_sessions",
                &[("host", &alias(h)), ("state", state)],
                n as f64,
            );
        }
    }
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for e in crate::events::load(crate::events::CAP) {
        match kinds.iter_mut().find(|(k, _)| k == e.kind.as_str()) {
            Some(x) => x.1 += 1,
            None => kinds.push((e.kind.as_str().to_string(), 1)),
        }
    }
    kinds.sort();
    p.family(
        "aleph_inbox_events",
        "gauge",
        "Events in the Inbox log (its last 2000) by kind.",
    );
    for (k, n) in &kinds {
        p.sample("aleph_inbox_events", &[("kind", k)], *n as f64);
    }
    let q = crate::queue::load();
    p.one(
        "aleph_queue_length",
        "gauge",
        "Tasks waiting in the queue.",
        q.iter().filter(|t| t.state == "queued").count() as f64,
    );
    p.one(
        "aleph_devices",
        "gauge",
        "Paired devices (the HTTP API).",
        st.devices.list().len() as f64,
    );
    p.one(
        "aleph_push_subscriptions",
        "gauge",
        "Web Push subscriptions.",
        st.push.list().len() as f64,
    );
    p.one(
        "aleph_sse_clients",
        "gauge",
        "Open event streams.",
        st.sse.load(Ordering::Relaxed) as f64,
    );
    p.0
}

// ------------------------------------------------------------------ the console

/// The console's files, embedded (no build step, nothing from a CDN).
const ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "/index.html",
        "text/html; charset=utf-8",
        include_bytes!("../web/index.html"),
    ),
    (
        "/app.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../web/app.js"),
    ),
    (
        "/style.css",
        "text/css; charset=utf-8",
        include_bytes!("../web/style.css"),
    ),
    (
        "/sw.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../web/sw.js"),
    ),
    (
        "/manifest.webmanifest",
        "application/manifest+json",
        include_bytes!("../web/manifest.webmanifest"),
    ),
    (
        "/icons/aleph-192.png",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-app-192.png"),
    ),
    (
        "/icons/aleph-512.png",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-app-512.png"),
    ),
    (
        "/icons/aleph-maskable-192.png",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-maskable-192.png"),
    ),
    (
        "/icons/aleph-maskable-512.png",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-maskable-512.png"),
    ),
    (
        "/icons/aleph-180.png",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-app-180.png"),
    ),
    (
        "/icons/aleph-32.png",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-app-32.png"),
    ),
    (
        "/icons/aleph-16.png",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-app-16.png"),
    ),
    (
        "/favicon.ico",
        "image/png",
        include_bytes!("../assets/brand/png/aleph-app-32.png"),
    ),
];

/// The console's build: the version and a hash of its files. The service
/// worker carries it, so a new build replaces the cached console.
pub fn build_id() -> &'static str {
    static B: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for (p, _, b) in ASSETS.iter().chain(crate::showcase::web_assets()) {
            h.update(p.as_bytes());
            h.update(b);
        }
        format!(
            "{}-{}",
            env!("CARGO_PKG_VERSION"),
            &devices::hex(&h.finalize())[..10]
        )
    })
}

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; manifest-src 'self'; worker-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

fn asset(path: &str, h: &Head) -> Reply {
    let p = if path == "/" { "/index.html" } else { path };
    // (a showcase build has a few files of its own)
    let Some((p, ctype, body)) = ASSETS
        .iter()
        .chain(crate::showcase::web_assets())
        .find(|(x, _, _)| *x == p)
    else {
        // the console routes with #fragments: anything else is missing
        return error(404, "not_found", format!("nothing at {path}"));
    };
    let body: Vec<u8> = if *p == "/sw.js" {
        String::from_utf8_lossy(body)
            .replace("__ALEPH_BUILD__", build_id())
            .into_bytes()
    } else if *p == "/index.html" {
        crate::showcase::web_index(String::from_utf8_lossy(body).into_owned())
            .replace("__ALEPH_BUILD__", build_id())
            .into_bytes()
    } else {
        body.to_vec()
    };
    let etag = format!("\"{}\"", build_id());
    let mut r = Reply {
        status: 200,
        headers: vec![
            ("Content-Type".into(), ctype.to_string()),
            ("ETag".into(), etag.clone()),
            (
                "Cache-Control".into(),
                if p.starts_with("/icons/") || *p == "/favicon.ico" {
                    "public, max-age=86400".into()
                } else {
                    "no-cache".into()
                },
            ),
        ],
        body,
        sse: None,
    };
    if *p == "/index.html" {
        r.headers
            .push(("Content-Security-Policy".into(), CSP.into()));
        r.headers.push((
            "Permissions-Policy".into(),
            "camera=(), microphone=(), geolocation=()".into(),
        ));
    }
    if *p == "/sw.js" {
        r.headers
            .push(("Service-Worker-Allowed".into(), "/".into()));
    }
    if h.header("if-none-match").map(str::trim) == Some(etag.as_str()) {
        r.status = 304;
        r.body.clear();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(tag: &str) -> State {
        let d = std::env::temp_dir().join(format!("al-http-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let sh = Shared::new();
        sh.publish(
            json!({"version": 1, "hosts": [
                {"alias": "b31", "reachable": true, "latency_ms": 12, "sessions": [
                    {"host": "b31", "name": "temper", "pane_id": "w1:p2", "agent": "claude", "status": "blocked"},
                    {"host": "b31", "name": "ignite", "pane_id": "w1:p3", "agent": "claude", "status": "working"},
                ]},
                {"alias": "gpu\"2", "reachable": false, "sessions": []},
            ]}),
            true,
        );
        State {
            shared: sh,
            devices: devices::Store::at(d.clone()),
            push: webpush::Store::at(d.join("push.json")),
            bound: Mutex::new("127.0.0.1:7878".into()),
            limits: Mutex::new(Limits::default()),
            sse: AtomicUsize::new(0),
            oversized: AtomicUsize::new(0),
        }
    }

    fn req(method: &str, url: &str, headers: &[(&str, &str)]) -> Head {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let mut h: Vec<(String, String)> = vec![("host".into(), "127.0.0.1:7878".into())];
        for (k, v) in headers {
            h.retain(|(x, _)| x != k);
            h.push((k.to_string(), v.to_string()));
        }
        Head {
            method: method.into(),
            path: path.into(),
            query: query.into(),
            headers: h,
            peer: Some("127.0.0.1".parse().unwrap()),
        }
    }

    fn cfg() -> Config {
        Config::in_memory(&[
            "remote.allow=b31:codex",
            "api.hosts=mac.tail1234.ts.net,*.example.org",
        ])
    }

    #[test]
    fn auth_and_scopes() {
        let st = state("auth");
        let c = cfg();
        let now = Instant::now();
        let read = st.devices.create("dash", Scope::Read, 1).unwrap();
        let reply = st.devices.create("phone", Scope::Reply, 1).unwrap();
        let b = |t: &str| format!("Bearer {t}");
        // no token, a wrong one
        let r = route(
            &st,
            &c,
            &req("GET", "/api/v1/state.snapshot", &[]),
            b"",
            now,
        );
        assert_eq!(r.status, 401);
        assert!(r.headers.iter().any(|(k, _)| k == "WWW-Authenticate"));
        let r = route(
            &st,
            &c,
            &req(
                "GET",
                "/api/v1/state.snapshot",
                &[("authorization", "Bearer aleph_nope")],
            ),
            b"",
            now,
        );
        assert_eq!(r.status, 401);
        // a read token reads
        let r = route(
            &st,
            &c,
            &req(
                "GET",
                "/api/v1/state.snapshot",
                &[("authorization", &b(&read))],
            ),
            b"",
            now,
        );
        assert_eq!(r.status, 200, "{:?}", r.json());
        assert_eq!(r.json()["result"]["hosts"][0]["alias"], "b31");
        // …and the cookie works the same
        let r = route(
            &st,
            &c,
            &req(
                "GET",
                "/api/v1/hosts.list",
                &[("cookie", &format!("x=1; {COOKIE}={read}"))],
            ),
            b"",
            now,
        );
        assert_eq!(r.status, 200);
        // but can't send
        let post = |t: &str, m: &str, body: &str| {
            route(
                &st,
                &c,
                &req(
                    "POST",
                    &format!("/api/v1/{m}"),
                    &[("authorization", &b(t)), ("x-aleph", "1")],
                ),
                body.as_bytes(),
                now,
            )
        };
        let r = post(
            &read,
            "session.send",
            r#"{"session":"b31:temper","text":"hi"}"#,
        );
        assert_eq!(r.status, 403);
        assert!(r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("reply scope"));
        // a reply token gets past the scope, to remote.allow (b31:codex only)
        let r = post(
            &reply,
            "session.send",
            r#"{"session":"b31:temper","text":"hi"}"#,
        );
        assert_eq!(r.status, 403);
        assert!(
            r.json()["error"]["message"]
                .as_str()
                .unwrap()
                .contains("remote.allow"),
            "{:?}",
            r.json()
        );
        // control needs control
        let r = post(&reply, "config.set", r#"{"key":"theme","value":"dark"}"#);
        assert_eq!(r.status, 403);
        // writes are POST only; GET of a reply method says so
        let r = route(
            &st,
            &c,
            &req(
                "GET",
                "/api/v1/session.send?session=b31:temper&text=hi",
                &[("authorization", &b(&reply))],
            ),
            b"",
            now,
        );
        assert_eq!(r.status, 405);
        // unknown method; me
        assert_eq!(post(&read, "no.such", "{}").status, 404);
        let r = route(
            &st,
            &c,
            &req("GET", "/api/v1/me", &[("authorization", &b(&reply))]),
            b"",
            now,
        );
        assert_eq!(r.json()["result"]["scope"], "reply");
        assert_eq!(r.json()["result"]["name"], "phone");
        // a revoked device is out at once
        st.devices.revoke("dash");
        let r = route(
            &st,
            &c,
            &req(
                "GET",
                "/api/v1/hosts.list",
                &[("cookie", &format!("{COOKIE}={read}"))],
            ),
            b"",
            now,
        );
        assert_eq!(r.status, 401);
        assert!(r
            .headers
            .iter()
            .any(|(k, v)| k == "Set-Cookie" && v.contains("Max-Age=0")));
    }

    #[test]
    fn host_allowlist() {
        let c = cfg();
        let b = "127.0.0.1:7878";
        for ok in [
            "localhost:7878",
            "127.0.0.1:7878",
            "[::1]:7878",
            "LOCALHOST",
            "mac.tail1234.ts.net",
            "a.example.org:443",
        ] {
            assert!(host_ok(&c, b, ok), "{ok}");
        }
        for bad in [
            "evil.com",
            "evil.com:7878",
            "127.0.0.1.evil.com",
            "example.org",
            "",
            "ts.net",
        ] {
            assert!(!host_ok(&c, b, bad), "{bad}");
        }
        // api.url's host; the address it listens on (not 0.0.0.0)
        let c2 = Config::in_memory(&["api.url=https://Mac.Tail1234.ts.net/"]);
        assert!(host_ok(&c2, b, "mac.tail1234.ts.net"));
        assert!(host_ok(&c2, "100.64.0.9:7878", "100.64.0.9:7878"));
        assert!(!host_ok(&c2, "0.0.0.0:7878", "0.0.0.0"));
        // through route: a rebinding page gets nothing, not even the console
        let st = state("host");
        let r = route(
            &st,
            &c,
            &req("GET", "/", &[("host", "evil.com:7878")]),
            b"",
            Instant::now(),
        );
        assert_eq!(r.status, 421);
        let r = route(&st, &c, &req("GET", "/", &[]), b"", Instant::now());
        assert_eq!(r.status, 200);
        assert!(r
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Security-Policy" && v.contains("script-src 'self'")));
    }

    #[test]
    fn csrf_header_and_origin() {
        let st = state("csrf");
        let c = cfg();
        let tok = st.devices.create("phone", Scope::Reply, 1).unwrap();
        let a = format!("Bearer {tok}");
        let now = Instant::now();
        let r = route(
            &st,
            &c,
            &req("POST", "/api/v1/inbox.read", &[("authorization", &a)]),
            b"{}",
            now,
        );
        assert_eq!(r.status, 403);
        assert_eq!(r.json()["error"]["code"], "csrf");
        let r = route(
            &st,
            &c,
            &req(
                "POST",
                "/api/v1/inbox.read",
                &[
                    ("authorization", &a),
                    ("x-aleph", "1"),
                    ("origin", "https://evil.com"),
                ],
            ),
            b"{}",
            now,
        );
        assert_eq!(r.json()["error"]["code"], "csrf");
        let r = route(
            &st,
            &c,
            &req(
                "POST",
                "/api/v1/inbox.read",
                &[
                    ("authorization", &a),
                    ("x-aleph", "1"),
                    ("sec-fetch-site", "cross-site"),
                ],
            ),
            b"{}",
            now,
        );
        assert_eq!(r.json()["error"]["code"], "csrf");
        // the console's own requests pass
        let r = route(
            &st,
            &c,
            &req(
                "POST",
                "/api/v1/daemon.ping",
                &[
                    ("authorization", &a),
                    ("x-aleph", "1"),
                    ("origin", "http://127.0.0.1:7878"),
                    ("sec-fetch-site", "same-origin"),
                ],
            ),
            b"",
            now,
        );
        assert_eq!(r.status, 200, "{:?}", r.json());
        assert_eq!(r.json()["result"]["pong"], true);
        // pairing is a POST too; no CORS preflights, ever
        assert_eq!(
            route(&st, &c, &req("POST", "/api/v1/pair", &[]), b"{}", now).status,
            403
        );
        let r = route(
            &st,
            &c,
            &req("OPTIONS", "/api/v1/daemon.ping", &[]),
            b"",
            now,
        );
        assert_eq!(r.status, 405);
        assert!(!r
            .headers
            .iter()
            .any(|(k, _)| k.starts_with("Access-Control")));
    }

    #[test]
    fn rate_limits() {
        let mut l = Limits::default();
        let t0 = Instant::now();
        let r = Rate {
            burst: 3.0,
            per_sec: 1.0,
        };
        assert!(l.take("k", r, t0) && l.take("k", r, t0) && l.take("k", r, t0));
        assert!(!l.take("k", r, t0));
        assert!(!l.has("k", r, t0 + Duration::from_millis(500)));
        assert!(l.take("k", r, t0 + Duration::from_millis(1100)));
        // others have their own
        assert!(l.take("other", r, t0));
        // full again after a while, never more than the burst
        let later = t0 + Duration::from_secs(60);
        assert!((0..3).all(|_| l.take("k", r, later)));
        assert!(!l.take("k", r, later));
        // wrong tokens: ten, then 429 even for a good one
        let st = state("rate");
        let c = cfg();
        let good = st.devices.create("phone", Scope::Read, 1).unwrap();
        let now = Instant::now();
        let get = |t: &str| {
            route(
                &st,
                &c,
                &req(
                    "GET",
                    "/api/v1/daemon.ping",
                    &[("authorization", &format!("Bearer {t}"))],
                ),
                b"",
                now,
            )
            .status
        };
        for _ in 0..10 {
            assert_eq!(get("aleph_wrong"), 401);
        }
        assert_eq!(get("aleph_wrong"), 429);
        assert_eq!(get(&good), 429);
        // pairing attempts: five per client
        let pair = || {
            route(
                &st,
                &c,
                &req("POST", "/api/v1/pair", &[("x-aleph", "1")]),
                br#"{"code":"AAAAA-AAAAA"}"#,
                now,
            )
            .status
        };
        for _ in 0..5 {
            assert_eq!(pair(), 403);
        }
        assert_eq!(pair(), 429);
        // behind a proxy, clients are told apart by what it forwards
        let other = route(
            &st,
            &c,
            &req(
                "POST",
                "/api/v1/pair",
                &[("x-aleph", "1"), ("x-forwarded-for", "100.64.0.7")],
            ),
            br#"{"code":"AAAAA-AAAAA"}"#,
            now,
        );
        assert_eq!(other.status, 403);
    }

    #[test]
    fn pairing_gives_a_token_and_a_cookie() {
        let st = state("pair");
        let c = cfg();
        let code = st
            .devices
            .pair_new("", Scope::Reply, crate::cache::now())
            .unwrap();
        let h = req(
            "POST",
            "/api/v1/pair",
            &[
                ("x-aleph", "1"),
                ("user-agent", "Mozilla/5.0 (Linux; Android 14)"),
                ("x-forwarded-proto", "https"),
            ],
        );
        let r = route(
            &st,
            &c,
            &h,
            format!("{{\"code\":\"{}\"}}", devices::show_code(&code)).as_bytes(),
            Instant::now(),
        );
        assert_eq!(r.status, 200, "{:?}", r.json());
        let tok = r.json()["result"]["token"].as_str().unwrap().to_string();
        assert_eq!(r.json()["result"]["name"], "android");
        let cookie = &r.headers.iter().find(|(k, _)| k == "Set-Cookie").unwrap().1;
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Strict")
                && cookie.ends_with("; Secure")
        );
        assert!(cookie.contains(&tok));
        assert_eq!(st.devices.check(&tok).unwrap().scope, Scope::Reply);
        // the code works once
        let r = route(
            &st,
            &c,
            &h,
            format!("{{\"code\":\"{code}\"}}").as_bytes(),
            Instant::now(),
        );
        assert_eq!(r.status, 403);
        assert!(r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already used"));
        // a plain-http login cookie isn't Secure
        let (_, v) = set_cookie("aleph_x", false);
        assert!(!v.contains("Secure"));
    }

    #[test]
    fn sse_framing() {
        assert_eq!(
            sse_frame(Some("state"), None, r#"{"a":1}"#),
            "event: state\ndata: {\"a\":1}\n\n"
        );
        assert_eq!(
            sse_frame(Some("event"), Some("17"), "one\ntwo\r\nthree"),
            "event: event\nid: 17\ndata: one\ndata: two\ndata: three\n\n"
        );
        assert_eq!(sse_msg(&Msg::Ping(5)), ": ping 5\n\n");
        let s = sse_msg(&Msg::State(3, json!({"hosts": []})));
        assert!(s.starts_with("event: state\ndata: {") && s.ends_with("}\n\n"));
        assert!(s.contains("\"gen\":3"));
        let e = crate::events::Event {
            t: 99,
            host: "b31".into(),
            name: "temper".into(),
            kind: crate::events::Kind::Blocked,
            title: "line one\nline two".into(),
            ..Default::default()
        };
        let m = sse_msg(&Msg::Event(e));
        // JSON never has a raw newline: one data line
        assert_eq!(m.matches("\ndata: ").count(), 1, "{m}");
        assert!(m.starts_with("event: event\nid: 99\ndata: {"));
        // the stream's parameters, and Last-Event-ID resuming it
        let st = state("sse");
        let c = cfg();
        let tok = st.devices.create("phone", Scope::Read, 1).unwrap();
        let r = route(
            &st,
            &c,
            &req(
                "GET",
                "/api/v1/events?kinds=blocked,done&state=false",
                &[
                    ("cookie", &format!("{COOKIE}={tok}")),
                    ("last-event-id", "1234"),
                ],
            ),
            b"",
            Instant::now(),
        );
        let p = r.sse.unwrap();
        assert_eq!(p["kinds"], "blocked,done");
        assert_eq!(p["state"], false);
        assert_eq!(p["since"], 1234);
    }

    #[test]
    fn metrics_format() {
        let st = state("metrics");
        let m = metrics(&st, &cfg());
        // every sample belongs to a family announced before it
        let mut families: Vec<String> = Vec::new();
        for l in m.lines() {
            if let Some(r) = l.strip_prefix("# TYPE ") {
                let mut w = r.split(' ');
                families.push(w.next().unwrap().to_string());
                assert!(matches!(w.next(), Some("gauge" | "counter")), "{l}");
                continue;
            }
            if l.starts_with("# HELP ") {
                continue;
            }
            let name = l.split(['{', ' ']).next().unwrap();
            assert!(families.iter().any(|f| f == name), "{l}");
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{l}"
            );
            let v = l.rsplit(' ').next().unwrap();
            assert!(v.parse::<f64>().is_ok(), "{l}");
        }
        assert!(
            m.contains("aleph_sessions{host=\"b31\",state=\"blocked\"} 1\n"),
            "{m}"
        );
        assert!(m.contains("aleph_sessions{host=\"b31\",state=\"done\"} 0\n"));
        assert!(m.contains("aleph_hosts_up 1\n"));
        assert!(
            m.contains("aleph_host_up{host=\"gpu\\\"2\"} 0\n"),
            "label values are escaped"
        );
        assert!(m.contains("aleph_host_latency_seconds{host=\"b31\"} 0.012\n"));
        assert!(m.contains("aleph_hub 1\n"));
        assert!(m.contains("aleph_queue_length "));
        assert_eq!(prom_escape("a\\b\"c\nd"), "a\\\\b\\\"c\\nd");
        // through route: a token needed
        let r = route(
            &st,
            &cfg(),
            &req("GET", "/metrics", &[]),
            b"",
            Instant::now(),
        );
        assert_eq!(r.status, 401);
    }

    #[test]
    fn listen_is_local_unless_allowed() {
        let a = |v: &str| Config::in_memory(&[&format!("api.listen={v}")]);
        assert_eq!(
            listen_addr(&Config::in_memory(&[])).unwrap().to_string(),
            "127.0.0.1:7878"
        );
        assert_eq!(
            listen_addr(&a(":9000")).unwrap().to_string(),
            "127.0.0.1:9000"
        );
        assert_eq!(
            listen_addr(&a("9001")).unwrap().to_string(),
            "127.0.0.1:9001"
        );
        assert_eq!(
            listen_addr(&a("localhost:9002")).unwrap().to_string(),
            "127.0.0.1:9002"
        );
        assert_eq!(
            listen_addr(&a("[::1]:9003")).unwrap().to_string(),
            "[::1]:9003"
        );
        let e = listen_addr(&a("0.0.0.0:7878")).unwrap_err();
        assert!(e.contains("api.listen_any"), "{e}");
        assert!(listen_addr(&a("100.64.0.9:7878")).is_err());
        let any = Config::in_memory(&["api.listen=0.0.0.0:7878", "api.listen_any=true"]);
        assert!(listen_addr(&any).is_ok());
        assert!(listen_addr(&a("nonsense")).is_err());
    }

    #[test]
    fn queries_and_paths() {
        let p = query_params(
            "n=20&unread=true&session=b31%3Atemper&host=10.0.0.49&kinds=%5B%22a%22%5D&q=two+words",
        )
        .unwrap();
        assert_eq!(p["n"], 20);
        assert_eq!(p["unread"], true);
        assert_eq!(p["session"], "b31:temper");
        assert_eq!(p["host"], "10.0.0.49");
        assert_eq!(p["kinds"][0], "a");
        assert_eq!(p["q"], "two words");
        let p = query_params("params=%7B%22range%22%3A%227d%22%7D").unwrap();
        assert_eq!(p["range"], "7d");
        assert!(query_params("params=1").is_err());
        assert_eq!(host_name("[::1]:7878"), "::1");
        assert_eq!(host_name("https://Mac.ts.net:443/x"), "mac.ts.net");
        assert_eq!(status_of("forbidden"), 403);
        assert_eq!(status_of("failed"), 409);
    }

    #[test]
    fn the_console_is_served() {
        let st = state("assets");
        let c = cfg();
        let now = Instant::now();
        for (p, ct) in [
            ("/", "text/html"),
            ("/app.js", "text/javascript"),
            ("/style.css", "text/css"),
            ("/sw.js", "text/javascript"),
            ("/manifest.webmanifest", "application/manifest+json"),
            ("/icons/aleph-192.png", "image/png"),
        ] {
            let r = route(&st, &c, &req("GET", p, &[]), b"", now);
            assert_eq!(r.status, 200, "{p}");
            assert!(
                r.headers
                    .iter()
                    .any(|(k, v)| k == "Content-Type" && v.starts_with(ct)),
                "{p}"
            );
        }
        let sw = route(&st, &c, &req("GET", "/sw.js", &[]), b"", now);
        assert!(String::from_utf8_lossy(&sw.body).contains(build_id()));
        // cached: 304 with the build as the ETag
        let etag = format!("\"{}\"", build_id());
        let r = route(
            &st,
            &c,
            &req("GET", "/app.js", &[("if-none-match", &etag)]),
            b"",
            now,
        );
        assert_eq!(r.status, 304);
        assert_eq!(
            route(&st, &c, &req("GET", "/nope.js", &[]), b"", now).status,
            404
        );
        assert_eq!(
            route(&st, &c, &req("POST", "/", &[("x-aleph", "1")]), b"", now).status,
            405
        );
    }

    #[test]
    fn push_subscriptions_per_device() {
        let st = state("push");
        let c = cfg();
        let tok = st.devices.create("phone", Scope::Read, 1).unwrap();
        let h = req(
            "POST",
            "/api/v1/push/subscribe",
            &[
                ("authorization", &format!("Bearer {tok}")),
                ("x-aleph", "1"),
            ],
        );
        let sub = json!({"endpoint": "https://fcm.googleapis.com/fcm/send/abc", "keys": {
            "p256dh": "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
            "auth": "BTBZMqHH6r4Tts7J_aSIgg"}});
        let r = route(&st, &c, &h, sub.to_string().as_bytes(), Instant::now());
        assert_eq!(r.status, 200, "{:?}", r.json());
        assert_eq!(st.push.list()[0].device, "phone");
        let mut bad = sub.clone();
        bad["endpoint"] = json!("https://169.254.169.254/latest");
        let r = route(&st, &c, &h, bad.to_string().as_bytes(), Instant::now());
        assert_eq!(r.status, 400);
        // unsubscribing takes only this device's
        let h = req(
            "POST",
            "/api/v1/push/unsubscribe",
            &[
                ("authorization", &format!("Bearer {tok}")),
                ("x-aleph", "1"),
            ],
        );
        let r = route(
            &st,
            &c,
            &h,
            br#"{"endpoint":"https://fcm.googleapis.com/fcm/send/abc"}"#,
            Instant::now(),
        );
        assert_eq!(r.json()["result"]["removed"], true);
        assert!(st.push.list().is_empty());
    }

    #[test]
    fn action_buttons_check_the_device() {
        let st = state("action");
        let c = cfg();
        let tok = st.devices.create("dash", Scope::Read, 1).unwrap();
        // a read-only device can't press buttons
        let r = route(
            &st,
            &c,
            &req(
                "POST",
                "/a/v1.x",
                &[
                    ("authorization", &format!("Bearer {tok}")),
                    ("x-aleph", "1"),
                ],
            ),
            b"",
            Instant::now(),
        );
        assert_eq!(r.status, 403);
        // without a device: the token must check out (this one doesn't)
        let r = route(
            &st,
            &c,
            &req("POST", "/a", &[("x-aleph", "1")]),
            b"v1.b31.w1:p2.1.approve.2.ff",
            Instant::now(),
        );
        assert_eq!(r.status, 409);
        assert!(r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("forged"));
        // and buttons are POSTs with the CSRF header too
        assert_eq!(
            route(&st, &c, &req("POST", "/a/v1.x", &[]), b"", Instant::now()).status,
            403
        );
    }
}
