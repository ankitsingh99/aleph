//! Paired devices for the HTTP API and web console.
//!
//! Each device has a name, a scope (`read` < `reply` < `control`, the same
//! scopes as `api.rs`) and a token. Only the token's SHA-256 is kept, in
//! `state/devices.json` (0600); the token itself is shown once.
//!
//! * `aleph daemon token new <name> [--scope read|reply|control]` prints a
//!   token for a script, Raycast or Prometheus.
//! * `aleph daemon token ls` and `aleph daemon token revoke <name>` (revoking
//!   also drops the device's Web Push subscriptions).
//! * `aleph daemon pair [--name phone] [--scope reply] [--url https://…]`
//!   makes a one-time code (5 minutes, single use) and shows the console's
//!   pairing link as a QR code. The console trades the code for a token
//!   (`POST /api/v1/pair`). Codes are kept hashed too, in `state/pairing.json`.

use crate::api::Scope;
use crate::cli::Args;
use crate::config::{state_dir, write_private, FileLock};
use crate::out::*;
use crate::ui::{self, c};
use ratatui::prelude::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

/// How long a pairing code works.
pub const PAIR_TTL: u64 = 5 * 60;
/// Every device token starts with this (so it's recognisable when pasted,
/// and in logs, where it's masked).
pub const TOKEN_PREFIX: &str = "aleph_";
/// Crockford's base32 (no I, L, O, U): codes read aloud or typed survive.
const CODE_CHARS: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub scope: Scope,
    /// hex SHA-256 of the token
    pub hash: String,
    pub created: u64,
    pub last_used: Option<u64>,
}

impl Device {
    pub fn to_json(&self) -> Value {
        json!({"name": self.name, "scope": self.scope.as_str(), "created": self.created, "last_used": self.last_used})
    }
    fn stored(&self) -> Value {
        let mut v = self.to_json();
        v["hash"] = json!(self.hash);
        v
    }
    fn from_json(v: &Value) -> Option<Device> {
        Some(Device {
            name: v.get("name")?.as_str()?.to_string(),
            scope: Scope::parse(v.get("scope")?.as_str()?)?,
            hash: v.get("hash")?.as_str()?.to_string(),
            created: v.get("created").and_then(Value::as_u64).unwrap_or(0),
            last_used: v.get("last_used").and_then(Value::as_u64),
        })
    }
}

/// Why a pairing code didn't work.
#[derive(Debug, PartialEq)]
pub enum PairError {
    Unknown,
    Used,
    Expired,
    Busy(String),
}

impl PairError {
    pub fn message(&self) -> String {
        match self {
            PairError::Unknown => {
                "that pairing code isn't valid (check it, or run aleph daemon pair again)".into()
            }
            PairError::Used => {
                "that pairing code was already used (run aleph daemon pair again)".into()
            }
            PairError::Expired => {
                "that pairing code expired: they work for 5 minutes (run aleph daemon pair again)"
                    .into()
            }
            PairError::Busy(e) => format!("couldn't save the device: {e}"),
        }
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `n` bytes from the OS's random source.
pub fn random(n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b).map_err(|e| format!("no random numbers: {e}"))?;
    Ok(b)
}

pub fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.trim().as_bytes()))
}

/// Compare without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Device names: lowercase letters, digits, `.`, `-`, `_`; 1–32 long.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 32
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        && n.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
}

/// A pairing code as typed (`abcde-fghjk`, `ABCDEFGHJK`, with O for 0 or
/// l for 1) → its canonical form, or None.
pub fn normalize_code(s: &str) -> Option<String> {
    let mut out = String::new();
    for ch in s.trim().chars() {
        let ch = match ch.to_ascii_uppercase() {
            '-' | ' ' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !CODE_CHARS.contains(&(ch as u8)) || !ch.is_ascii() {
            return None;
        }
        out.push(ch);
    }
    (out.len() == 10).then_some(out)
}

/// `ABCDEFGHJK` → `ABCDE-FGHJK`.
pub fn show_code(code: &str) -> String {
    if code.len() == 10 {
        format!("{}-{}", &code[..5], &code[5..])
    } else {
        code.to_string()
    }
}

/// The two files, in one folder (the state folder; a temp folder in tests).
pub struct Store {
    dir: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Store::new()
    }
}

impl Store {
    pub fn new() -> Store {
        Store { dir: state_dir() }
    }

    #[cfg(test)]
    pub fn at(dir: PathBuf) -> Store {
        Store { dir }
    }

    fn devices_file(&self) -> PathBuf {
        self.dir.join("devices.json")
    }

    fn pairing_file(&self) -> PathBuf {
        self.dir.join("pairing.json")
    }

    fn lock(&self) -> FileLock {
        let _ = std::fs::create_dir_all(&self.dir);
        FileLock::take(&self.dir.join(".devices.lock"))
    }

    pub fn list(&self) -> Vec<Device> {
        read_json(&self.devices_file())
            .get("devices")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Device::from_json).collect())
            .unwrap_or_default()
    }

    fn save(&self, v: &[Device]) -> Result<(), String> {
        let doc =
            json!({"version": 1, "devices": v.iter().map(Device::stored).collect::<Vec<_>>()});
        write_private(
            &self.devices_file(),
            (serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n").as_bytes(),
        )
        .map_err(|e| e.to_string())
    }

    /// A new device; its token (shown once, never stored).
    pub fn create(&self, name: &str, scope: Scope, now: u64) -> Result<String, String> {
        if !valid_name(name) {
            return Err(format!(
                "{name:?} isn't a device name: lowercase letters, digits, - _ . (up to 32)"
            ));
        }
        let _l = self.lock();
        let mut v = self.list();
        if v.iter().any(|d| d.name == name) {
            return Err(format!(
                "there's already a device called {name} (aleph daemon token revoke {name} first)"
            ));
        }
        let token = format!("{TOKEN_PREFIX}{}", hex(&random(32)?));
        v.push(Device {
            name: name.to_string(),
            scope,
            hash: hash(&token),
            created: now,
            last_used: None,
        });
        self.save(&v)?;
        Ok(token)
    }

    /// Forget a device (its token stops working at once). False: no such device.
    pub fn revoke(&self, name: &str) -> bool {
        let _l = self.lock();
        let mut v = self.list();
        let n = v.len();
        v.retain(|d| d.name != name);
        n != v.len() && self.save(&v).is_ok()
    }

    /// The device a token belongs to.
    pub fn check(&self, token: &str) -> Option<Device> {
        let t = token.trim();
        if !t.starts_with(TOKEN_PREFIX) || t.len() > 128 {
            return None;
        }
        let h = hash(t);
        // look at every entry: no early exit on the first match
        self.list().into_iter().fold(
            None,
            |found, d| if same(&d.hash, &h) { Some(d) } else { found },
        )
    }

    /// Note a device was just used (written at most once a minute per device).
    pub fn touch(&self, name: &str, now: u64) {
        static LAST: std::sync::Mutex<Vec<(String, u64)>> = std::sync::Mutex::new(Vec::new());
        {
            let mut l = LAST.lock().unwrap();
            if l.iter()
                .any(|(n, t)| n == name && now.saturating_sub(*t) < 60)
            {
                return;
            }
            l.retain(|(n, _)| n != name);
            l.push((name.to_string(), now));
        }
        let _l = self.lock();
        let mut v = self.list();
        if let Some(d) = v.iter_mut().find(|d| d.name == name) {
            d.last_used = Some(now);
            let _ = self.save(&v);
        }
    }

    // -------------------------------------------------------------- pairing

    fn codes(&self) -> Vec<Value> {
        read_json(&self.pairing_file())
            .get("codes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    fn save_codes(&self, v: Vec<Value>) -> Result<(), String> {
        write_private(
            &self.pairing_file(),
            (json!({"codes": v}).to_string() + "\n").as_bytes(),
        )
        .map_err(|e| e.to_string())
    }

    /// A one-time pairing code for a device of this scope (`name`: what to
    /// call it; empty lets the console name it).
    pub fn pair_new(&self, name: &str, scope: Scope, now: u64) -> Result<String, String> {
        if !name.is_empty() && !valid_name(name) {
            return Err(format!(
                "{name:?} isn't a device name: lowercase letters, digits, - _ . (up to 32)"
            ));
        }
        let code: String = random(10)?
            .iter()
            .map(|b| CODE_CHARS[(*b & 31) as usize] as char)
            .collect();
        let _l = self.lock();
        let mut v = self.codes();
        // expired ones go (used ones stay until then, to say "already used")
        v.retain(|c| c.get("exp").and_then(Value::as_u64).unwrap_or(0) > now);
        v.push(json!({"hash": hash(&code), "name": name, "scope": scope.as_str(), "created": now, "exp": now + PAIR_TTL}));
        self.save_codes(v)?;
        Ok(code)
    }

    /// Is this code still waiting to be used?
    pub fn pair_pending(&self, code: &str, now: u64) -> bool {
        let h = hash(code);
        self.codes().iter().any(|c| {
            c.get("hash").and_then(Value::as_str) == Some(h.as_str())
                && c.get("used").is_none()
                && c.get("exp").and_then(Value::as_u64).unwrap_or(0) > now
        })
    }

    /// Trade a code for a new device and its token. `want`: the name the
    /// console asked for (the one `aleph daemon pair --name` gave wins).
    pub fn pair_redeem(
        &self,
        code: &str,
        want: &str,
        now: u64,
    ) -> Result<(Device, String), PairError> {
        let code = normalize_code(code).ok_or(PairError::Unknown)?;
        let h = hash(&code);
        let _l = self.lock();
        let mut v = self.codes();
        let Some(i) = v.iter().position(|c| {
            c.get("hash")
                .and_then(Value::as_str)
                .is_some_and(|x| same(x, &h))
        }) else {
            return Err(PairError::Unknown);
        };
        if v[i].get("used").is_some() {
            return Err(PairError::Used);
        }
        if v[i].get("exp").and_then(Value::as_u64).unwrap_or(0) <= now {
            return Err(PairError::Expired);
        }
        let scope = v[i]
            .get("scope")
            .and_then(Value::as_str)
            .and_then(Scope::parse)
            .unwrap_or(Scope::Read);
        let given = v[i].get("name").and_then(Value::as_str).unwrap_or("");
        let base = [given, want]
            .into_iter()
            .find(|n| valid_name(n))
            .unwrap_or("device")
            .to_string();
        let mut devs = self.list();
        // names are ASCII: slicing by bytes is safe
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{}-{n}", &base[..base.len().min(28)])
                }
            })
            .find(|n| !devs.iter().any(|d| d.name == *n))
            .unwrap_or(base);
        let token = format!(
            "{TOKEN_PREFIX}{}",
            hex(&random(32).map_err(PairError::Busy)?)
        );
        let d = Device {
            name,
            scope,
            hash: hash(&token),
            created: now,
            last_used: Some(now),
        };
        devs.push(d.clone());
        self.save(&devs).map_err(PairError::Busy)?;
        // single use: marked (kept until it would have expired, so a second
        // try says "already used")
        v[i]["used"] = json!(now);
        v[i]["device"] = json!(d.name);
        let _ = self.save_codes(v);
        Ok((d, token))
    }

    /// The device a used code made (for `aleph daemon pair`, waiting).
    pub fn paired_by(&self, code: &str) -> Option<String> {
        let h = hash(code);
        self.codes().iter().find_map(|c| {
            (c.get("hash").and_then(Value::as_str) == Some(h.as_str()))
                .then(|| c.get("device").and_then(Value::as_str).map(str::to_string))
                .flatten()
        })
    }
}

fn read_json(p: &std::path::Path) -> Value {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

/// A guess at what's pairing, from its User-Agent (`iphone`, `android`, …).
pub fn guess_name(user_agent: &str) -> &'static str {
    let ua = user_agent.to_lowercase();
    if ua.contains("iphone") {
        "iphone"
    } else if ua.contains("ipad") {
        "ipad"
    } else if ua.contains("android") {
        "android"
    } else if ua.contains("macintosh") {
        "mac"
    } else if ua.contains("windows") {
        "windows"
    } else if ua.contains("linux") {
        "linux"
    } else {
        "browser"
    }
}

/// An Inbox line for pairing and revoking (`remote` events: the audit trail).
pub fn audit(title: String) {
    crate::events::append(crate::events::Event {
        t: crate::cache::now(),
        host: crate::brand::NAME.into(),
        kind: crate::events::Kind::Remote,
        title,
        source: "aleph".into(),
        ..Default::default()
    });
}

// ------------------------------------------------------------------ CLI

/// `--flag value` or `--flag=value`.
fn opt<'a>(rest: &'a [String], flag: &str) -> Option<&'a str> {
    let mut it = rest.iter();
    while let Some(x) = it.next() {
        if x == flag {
            return it.next().map(String::as_str);
        }
        if let Some(v) = x.strip_prefix(flag).and_then(|r| r.strip_prefix('=')) {
            return Some(v);
        }
    }
    None
}

/// Positional words after the subcommand, skipping options and their values.
fn words(rest: &[String], with_values: &[&str]) -> Vec<String> {
    let mut v = Vec::new();
    let mut skip = false;
    for x in rest {
        if skip {
            skip = false;
            continue;
        }
        if with_values.contains(&x.as_str()) {
            skip = true;
            continue;
        }
        if !x.starts_with('-') {
            v.push(x.clone());
        }
    }
    v
}

fn scope_opt(rest: &[String], default: Scope) -> Result<Scope, ExitCode> {
    match opt(rest, "--scope") {
        None => Ok(default),
        Some(s) => {
            Scope::parse(s).ok_or_else(|| fail(&format!("--scope {s}? read, reply or control")))
        }
    }
}

/// `aleph daemon token new|ls|revoke` (`rest` starts after `token`).
/// The paired devices as `aleph daemon token ls --json` lists them (each
/// with how many push subscriptions it has).
pub fn tokens_json() -> Value {
    let subs = crate::webpush::Store::new().list();
    Value::Array(
        Store::new()
            .list()
            .iter()
            .map(|d| {
                let mut j = d.to_json();
                j["push"] = json!(subs.iter().filter(|s| s.device == d.name).count());
                j
            })
            .collect(),
    )
}

pub fn token_cmd(a: &Args, rest: &[String]) -> ExitCode {
    let st = Store::new();
    let w = words(rest, &["--scope"]);
    let now = crate::cache::now();
    match w.first().map(String::as_str) {
        Some("new") => {
            let Some(name) = w.get(1) else {
                return fail("usage: aleph daemon token new <name> [--scope read|reply|control]");
            };
            let scope = match scope_opt(rest, Scope::Read) {
                Ok(s) => s,
                Err(c) => return c,
            };
            match st.create(name, scope, now) {
                Ok(tok) => {
                    audit(format!("new device token: {name} ({})", scope.as_str()));
                    if a.has("--json") {
                        json(&json!({"name": name, "scope": scope.as_str(), "token": tok}));
                    } else {
                        println!("{tok}");
                        ok(&format!(
                            "{name} can use the API with the {} scope",
                            scope.as_str()
                        ));
                        hint("it's shown only this once: send it as Authorization: Bearer <token>");
                        if scope > Scope::Read {
                            hint("sessions also need remote.allow before a device can act on them");
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        Some("ls") | None => {
            let v = st.list();
            let subs = crate::webpush::Store::new().list();
            let pushes = |n: &str| subs.iter().filter(|s| s.device == n).count();
            if a.has("--json") {
                json(&tokens_json());
                return ExitCode::SUCCESS;
            }
            if !std::io::stdout().is_terminal() {
                for d in &v {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        d.name,
                        d.scope.as_str(),
                        d.created,
                        d.last_used.map(|t| t.to_string()).unwrap_or_default(),
                        pushes(&d.name)
                    );
                }
                return ExitCode::SUCCESS;
            }
            if v.is_empty() {
                ok("no devices yet");
                hint("aleph daemon pair — a phone or browser scans a QR code");
                hint("aleph daemon token new <name> --scope read — a token for a script");
                return ExitCode::SUCCESS;
            }
            out(Line::from(Span::styled(
                format!(
                    "  {:<18} {:<8} {:<10} {:<10} {}",
                    "DEVICE", "SCOPE", "PAIRED", "LAST USED", "PUSH"
                ),
                ui::faint(),
            )));
            for d in &v {
                let sc = match d.scope {
                    Scope::Read => c::MUTED(),
                    Scope::Reply => c::BLUE(),
                    Scope::Control => c::NEEDS(),
                };
                out(Line::from(vec![
                    Span::styled("  ", ui::muted()),
                    Span::styled(ui::pad(&d.name, 19), ui::bold()),
                    Span::styled(ui::pad(d.scope.as_str(), 9), ui::fg(sc)),
                    Span::styled(
                        ui::pad(&ui::ago(now.saturating_sub(d.created)), 11),
                        ui::muted(),
                    ),
                    Span::styled(
                        ui::pad(
                            &d.last_used
                                .map(|t| ui::ago(now.saturating_sub(t)))
                                .unwrap_or_else(|| "never".into()),
                            11,
                        ),
                        ui::muted(),
                    ),
                    Span::styled(
                        match pushes(&d.name) {
                            0 => "–".to_string(),
                            n => format!("{n}"),
                        },
                        ui::muted(),
                    ),
                ]));
            }
            ExitCode::SUCCESS
        }
        Some("revoke") | Some("rm") => {
            let Some(name) = w.get(1) else {
                return fail("usage: aleph daemon token revoke <name>");
            };
            if !st.revoke(name) {
                err(&format!("no device called {name}"));
                hint("aleph daemon token ls");
                return ExitCode::from(1);
            }
            let dropped = crate::webpush::Store::new().drop_device(name);
            audit(format!("revoked device {name}"));
            ok(&format!(
                "revoked {name}: its token stops working now{}",
                if dropped > 0 {
                    format!(", and its {dropped} push subscription(s) are gone")
                } else {
                    String::new()
                }
            ));
            ExitCode::SUCCESS
        }
        Some(x) => {
            err(&format!("aleph daemon token {x}: no such subcommand"));
            hint("aleph daemon token new <name> [--scope read|reply|control] | ls | revoke <name>");
            ExitCode::from(2)
        }
    }
}

/// Where the console is, for a pairing link: `--url`, `api.url`, else the
/// running daemon's own address (this computer only).
fn base_url(cfg: &crate::config::Config, flag: Option<&str>, status: &Value) -> Option<String> {
    if let Some(u) = flag.map(str::to_string).or_else(|| cfg.get("api.url")) {
        let u = u.trim().trim_end_matches('/').to_string();
        if u.starts_with("http://") || u.starts_with("https://") {
            return Some(u);
        }
    }
    let addr = status.get("http").and_then(Value::as_str)?;
    Some(format!("http://{addr}"))
}

/// `aleph daemon pair [--name N] [--scope S] [--url U] [--json]`.
pub fn pair_cmd(a: &Args, rest: &[String]) -> ExitCode {
    let scope = match scope_opt(rest, Scope::Reply) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let name = opt(rest, "--name").unwrap_or("");
    let status = match crate::daemon::request("daemon.status", json!({})) {
        Ok(v) => v,
        Err(e) => {
            err(&e);
            hint("pairing needs the daemon with its HTTP API: aleph settings set api.http on, then aleph daemon run");
            return ExitCode::from(1);
        }
    };
    if status.get("http").and_then(Value::as_str).is_none() {
        err("the daemon's HTTP API is off");
        hint("aleph settings set api.http on, then restart the daemon");
        return ExitCode::from(1);
    }
    let Some(base) = base_url(&a.cfg, opt(rest, "--url"), &status) else {
        return fail("--url needs an http:// or https:// address");
    };
    let st = Store::new();
    let now = crate::cache::now();
    // showcase mode: a QR code to show, not one that pairs (no code is made)
    let code = if crate::showcase::on() {
        "42424242".to_string()
    } else {
        match st.pair_new(name, scope, now) {
            Ok(c) => c,
            Err(e) => return fail(&e),
        }
    };
    let base = crate::showcase::text(&base).into_owned();
    let url = format!("{base}/#/pair/{code}");
    if a.has("--json") {
        json(
            &json!({"url": url, "code": code, "name": name, "scope": scope.as_str(), "expires": now + PAIR_TTL}),
        );
        return ExitCode::SUCCESS;
    }
    if !std::io::stderr().is_terminal() {
        println!("{url}");
        return ExitCode::SUCCESS;
    }
    head(&format!(
        "Pair a device with {} ({} scope)",
        crate::brand::NAME,
        scope.as_str()
    ));
    say(Line::raw(""));
    for l in qr_lines(&url, crate::term::color_err()) {
        eprintln!("  {l}");
    }
    say(Line::raw(""));
    say(Line::from(vec![
        Span::styled("  link  ", ui::faint()),
        Span::styled(url.clone(), ui::fg(c::ACCENT())),
    ]));
    say(Line::from(vec![
        Span::styled("  code  ", ui::faint()),
        Span::styled(show_code(&code), ui::bold()),
        Span::styled("   works once, for 5 minutes", ui::muted()),
    ]));
    if base.starts_with("http://127.")
        || base.starts_with("http://localhost")
        || base.starts_with("http://[::1]")
    {
        hint("that address only works on this computer: for a phone, expose the API (tailscale serve) and pass --url https://… (or set api.url)");
    }
    hint("iPhone: open the link, Share → Add to Home Screen, then open Aleph from the Home Screen and enter the code there (that's where notifications work)");
    let sp = Spinner::start("Waiting for the device".into());
    while st.pair_pending(&code, crate::cache::now()) {
        std::thread::sleep(Duration::from_millis(400));
    }
    sp.stop();
    match st.paired_by(&code) {
        Some(d) => {
            ok(&format!("paired {d} ({})", scope.as_str()));
            hint(&format!(
                "aleph daemon token revoke {d} — when it's lost or no longer used"
            ));
            ExitCode::SUCCESS
        }
        None => {
            warn("the code expired before anyone used it (run aleph daemon pair again)");
            ExitCode::from(1)
        }
    }
}

/// A QR code in half blocks: two modules per character cell. With colour,
/// black on white whatever the terminal's own background; without, light
/// modules are drawn (dark terminals are the usual case).
pub fn qr_lines(text: &str, color: bool) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::L) else {
        return vec![];
    };
    let w = code.width();
    let colors = code.to_colors();
    let quiet = 2usize;
    let size = w + 2 * quiet;
    // true = dark module
    let dark = |x: usize, y: usize| -> bool {
        if x < quiet || y < quiet || x >= w + quiet || y >= w + quiet {
            return false;
        }
        colors[(y - quiet) * w + (x - quiet)] == qrcode::Color::Dark
    };
    let mut out = Vec::new();
    let mut y = 0;
    while y < size {
        let mut l = String::new();
        for x in 0..size {
            let top = dark(x, y);
            let bottom = y + 1 < size && dark(x, y + 1);
            if color {
                // ▀ in the top module's colour over the bottom one's
                let c = |d: bool| if d { "0;0;0" } else { "255;255;255" };
                l.push_str(&format!("\x1b[38;2;{}m\x1b[48;2;{}m▀", c(top), c(bottom)));
            } else {
                l.push(match (top, bottom) {
                    (false, false) => '█',
                    (false, true) => '▀',
                    (true, false) => '▄',
                    (true, true) => ' ',
                });
            }
        }
        if color {
            l.push_str("\x1b[0m");
        }
        out.push(l);
        y += 2;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> Store {
        let d = std::env::temp_dir().join(format!("al-dev-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Store::at(d)
    }

    #[test]
    fn only_a_hash_is_kept() {
        let st = store("hash");
        let tok = st.create("phone", Scope::Reply, 100).unwrap();
        assert!(tok.starts_with(TOKEN_PREFIX) && tok.len() == TOKEN_PREFIX.len() + 64);
        let text = std::fs::read_to_string(st.devices_file()).unwrap();
        assert!(
            !text.contains(&tok[TOKEN_PREFIX.len()..]),
            "the token itself is never written"
        );
        assert!(text.contains(&hash(&tok)));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(st.devices_file())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let d = st.check(&tok).unwrap();
        assert_eq!((d.name.as_str(), d.scope), ("phone", Scope::Reply));
        // a near miss, another prefix, junk
        let mut bad = tok.clone();
        bad.pop();
        bad.push('x');
        assert!(st.check(&bad).is_none());
        assert!(st.check(&tok.replace(TOKEN_PREFIX, "other_")).is_none());
        assert!(st.check("").is_none());
        // names are unique and checked
        assert!(st.create("phone", Scope::Read, 101).is_err());
        assert!(st.create("Bad Name", Scope::Read, 101).is_err());
        // revoked: gone at once
        assert!(st.revoke("phone"));
        assert!(st.check(&tok).is_none());
        assert!(!st.revoke("phone"));
    }

    #[test]
    fn pairing_codes_expire_and_work_once() {
        let st = store("pair");
        let code = st.pair_new("", Scope::Reply, 1000).unwrap();
        assert_eq!(code.len(), 10);
        assert!(st.pair_pending(&code, 1001));
        // typed loosely: lower case, a dash
        let typed = show_code(&code).to_lowercase();
        let (d, tok) = st.pair_redeem(&typed, "pixel", 1010).unwrap();
        assert_eq!((d.name.as_str(), d.scope), ("pixel", Scope::Reply));
        assert_eq!(st.check(&tok).unwrap().name, "pixel");
        assert_eq!(st.paired_by(&code).as_deref(), Some("pixel"));
        // single use
        assert_eq!(
            st.pair_redeem(&code, "pixel", 1011).unwrap_err(),
            PairError::Used
        );
        assert!(!st.pair_pending(&code, 1011));
        // expired after 5 minutes
        let old = st.pair_new("tablet", Scope::Read, 2000).unwrap();
        assert_eq!(
            st.pair_redeem(&old, "", 2000 + PAIR_TTL).unwrap_err(),
            PairError::Expired
        );
        // never made, or not a code at all
        assert_eq!(
            st.pair_redeem("ABCDE-FGHJK", "", 2001).unwrap_err(),
            PairError::Unknown
        );
        assert_eq!(
            st.pair_redeem("hello", "", 2001).unwrap_err(),
            PairError::Unknown
        );
        // the name from `aleph daemon pair --name` wins; clashes get a suffix
        let a = st.pair_new("pixel", Scope::Read, 3000).unwrap();
        let (d, _) = st.pair_redeem(&a, "whatever", 3001).unwrap();
        assert_eq!(d.name, "pixel-2");
        assert_eq!(d.scope, Scope::Read);
    }

    #[test]
    fn codes_and_names() {
        assert_eq!(normalize_code("abcde-fghjk").as_deref(), Some("ABCDEFGHJK"));
        assert_eq!(normalize_code("O1I2L-34567").as_deref(), Some("0112134567"));
        assert!(normalize_code("ABCDE-FGHJ").is_none());
        assert!(normalize_code("ABCDE-FGHJU").is_none());
        assert!(valid_name("pixel-8") && valid_name("mac.home") && !valid_name("-x"));
        assert!(!valid_name("") && !valid_name("Phone") && !valid_name(&"x".repeat(33)));
        assert_eq!(
            guess_name("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"),
            "iphone"
        );
        assert_eq!(
            guess_name("Mozilla/5.0 (Linux; Android 14; Pixel 8)"),
            "android"
        );
    }

    #[test]
    fn qr_is_square_half_blocks() {
        let l = qr_lines("https://mac.tail1234.ts.net/#/pair/ABCDEFGHJK", false);
        // 2 modules per row: (width + 2 × quiet zone) / 2, rounded up
        let w = l[0].chars().count();
        assert!(w >= 25 + 4, "{w}");
        assert_eq!(l.len(), w.div_ceil(2));
        assert!(l.iter().all(|r| r.chars().count() == w));
        assert!(l.iter().all(|r| r.chars().all(|c| " ▀▄█".contains(c))));
        let c = qr_lines("x", true);
        assert!(c[0].contains("\x1b[48;2;255;255;255m") && c[0].ends_with("\x1b[0m"));
    }
}
