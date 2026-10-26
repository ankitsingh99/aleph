//! Web Push to paired browsers (the web console, installed as a PWA): the
//! `webpush` notification sink.
//!
//! * VAPID (RFC 8292): an ES256 key pair made once and kept with the other
//!   secrets (`aleph-vapid`). Every delivery carries `Authorization: vapid
//!   t=<JWT>, k=<public key>`; the JWT names the push service's origin and
//!   lasts 12 hours.
//! * Payloads are encrypted for each subscription (RFC 8291: ECDH on P-256,
//!   HKDF-SHA-256 and AES-128-GCM in the `aes128gcm` coding of RFC 8188), so
//!   the push service only ever sees ciphertext.
//! * Subscriptions are kept per paired device in `state/push.json` (0600).
//!   Revoking the device drops them; a push service answering 404 or 410
//!   drops that one; five refusals in a row drop it too.
//! * Delivery is one `curl` per subscription. Its config goes on stdin
//!   (`-K -`): the endpoint is a capability URL and the JWT a credential, so
//!   neither shows in `ps` or the debug log. The encrypted body comes from a
//!   0600 temp file.
//!
//! Questions carry Approve / Deny / Reply actions when remote control allows
//! that session: signed single-use tokens (`actions.rs`) that the service
//! worker posts back to `/a/<token>` with the device's own token.

use crate::config::{state_dir, write_private, Config, FileLock};
use crate::events::{Event, Kind};
use hkdf::Hkdf;
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// The secret holding the VAPID private key (hex).
const SECRET: &str = "aleph-vapid";
/// RFC 8291 §4: one record of at most 4096 bytes, so at most 3993 bytes of
/// plaintext. We stay well under it.
pub const MAX_PLAIN: usize = 3800;
const RECORD: u32 = 4096;
const T_SEND: Duration = Duration::from_secs(20);
/// The default `sub` claim (push services want a mailto: or https: contact).
pub const CONTACT: &str = "mailto:aleph@users.noreply.github.com";
/// Push services a subscription may point at (the endpoint is a URL the
/// daemon will POST to: never an arbitrary one).
const PUSH_HOSTS: &[&str] = &[
    "fcm.googleapis.com",
    "android.googleapis.com",
    "push.services.mozilla.com",
    "push.apple.com",
    "notify.windows.com",
];

// ------------------------------------------------------------------ base64url

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Unpadded base64url (RFC 4648 §5), as Web Push and JWTs use it.
pub fn b64url(b: &[u8]) -> String {
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16
            | (*ch.get(1).unwrap_or(&0) as u32) << 8
            | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..=ch.len() {
            o.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    o
}

/// base64url or standard base64, padded or not (whitespace is skipped).
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ------------------------------------------------------------------ VAPID

/// The application server's signing key (RFC 8292).
pub struct Vapid {
    key: SigningKey,
}

impl Vapid {
    pub fn from_bytes(b: &[u8]) -> Result<Vapid, String> {
        SigningKey::from_slice(b)
            .map(|key| Vapid { key })
            .map_err(|_| "not a P-256 private key".to_string())
    }

    /// The key from the keystore, made (once) when there's none yet.
    pub fn load() -> Result<Vapid, String> {
        static CACHE: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);
        let mut c = CACHE.lock().unwrap();
        if let Some(b) = c.as_ref() {
            return Vapid::from_bytes(b);
        }
        let parse = |h: &str| -> Option<Vec<u8>> {
            let b: Vec<u8> = (0..h.len() / 2)
                .filter_map(|i| u8::from_str_radix(h.get(i * 2..i * 2 + 2)?, 16).ok())
                .collect();
            (b.len() == 32 && SigningKey::from_slice(&b).is_ok()).then_some(b)
        };
        // two processes making one at the same moment would leave them with
        // different keys: one makes it, under a lock, and the other reads it
        let _l = FileLock::take(&state_dir().join(".vapid.lock"));
        let b = match crate::secrets::get(SECRET).as_deref().and_then(parse) {
            Some(b) => b,
            None => {
                let b = loop {
                    let r = crate::devices::random(32)?;
                    if SigningKey::from_slice(&r).is_ok() {
                        break r;
                    }
                };
                crate::secrets::set(SECRET, &crate::devices::hex(&b))?;
                crate::log::info("webpush", "made the VAPID key (kept with the secrets)");
                b
            }
        };
        *c = Some(b.clone());
        Vapid::from_bytes(&b)
    }

    /// The uncompressed public key (65 bytes, 0x04 first).
    pub fn public(&self) -> Vec<u8> {
        self.key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec()
    }

    /// What `PushManager.subscribe` takes as `applicationServerKey`.
    pub fn public_b64(&self) -> String {
        b64url(&self.public())
    }

    /// An ES256 JWT for one push service (`aud`: its origin).
    pub fn jwt(&self, aud: &str, exp: u64, sub: &str) -> String {
        use p256::ecdsa::signature::Signer;
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let input = format!(
            "{}.{}",
            b64url(br#"{"typ":"JWT","alg":"ES256"}"#),
            b64url(format!(r#"{{"aud":{},"exp":{exp},"sub":{}}}"#, q(aud), q(sub)).as_bytes())
        );
        let sig: p256::ecdsa::Signature = self.key.sign(input.as_bytes());
        format!("{input}.{}", b64url(&sig.to_bytes()))
    }

    /// The `Authorization` header for a delivery to `endpoint`.
    pub fn authorization(&self, endpoint: &str, now: u64, sub: &str) -> Option<String> {
        let aud = origin(endpoint)?;
        Some(format!(
            "vapid t={}, k={}",
            self.jwt(&aud, now + 12 * 3600, sub),
            self.public_b64()
        ))
    }
}

/// `https://fcm.googleapis.com/fcm/send/…` → `https://fcm.googleapis.com`.
pub fn origin(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.contains('@')).then(|| format!("https://{host}"))
}

// ------------------------------------------------------------------ RFC 8291

/// The keys RFC 8291 §3.4 derives (kept apart so the tests can check each
/// one against Appendix A).
#[derive(Debug)]
#[allow(dead_code)] // prk_key, ikm, prk: the intermediate values the tests check
pub struct Keys {
    pub prk_key: Vec<u8>,
    pub ikm: [u8; 32],
    pub prk: Vec<u8>,
    pub cek: [u8; 16],
    pub nonce: [u8; 12],
}

pub fn derive(ecdh: &[u8], auth: &[u8], ua_public: &[u8], as_public: &[u8], salt: &[u8]) -> Keys {
    let mut info = Vec::with_capacity(14 + 130);
    info.extend_from_slice(b"WebPush: info\0");
    info.extend_from_slice(ua_public);
    info.extend_from_slice(as_public);
    let (prk_key, hk) = Hkdf::<Sha256>::extract(Some(auth), ecdh);
    let mut ikm = [0u8; 32];
    hk.expand(&info, &mut ikm)
        .expect("32 bytes is a valid length");
    let (prk, hk) = Hkdf::<Sha256>::extract(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .expect("16 bytes is a valid length");
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .expect("12 bytes is a valid length");
    Keys {
        prk_key: prk_key.to_vec(),
        ikm,
        prk: prk.to_vec(),
        cek,
        nonce,
    }
}

/// Encrypt a push message for a subscription (`ua_public`: its `p256dh`,
/// `auth`: its auth secret) with this sender key and salt. Pure — the
/// RFC 8291 test vectors check it.
pub fn encrypt_with(
    ua_public: &[u8],
    auth: &[u8],
    plaintext: &[u8],
    as_key: &p256::SecretKey,
    salt: &[u8; 16],
) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    if auth.len() != 16 {
        return Err("the subscription's auth secret isn't 16 bytes".into());
    }
    if plaintext.len() > MAX_PLAIN {
        return Err(format!(
            "a push message of {} bytes is too long",
            plaintext.len()
        ));
    }
    let ua = p256::PublicKey::from_sec1_bytes(ua_public)
        .map_err(|_| "the subscription's p256dh isn't a P-256 public key".to_string())?;
    let ua_raw = ua.to_encoded_point(false);
    let as_raw = as_key.public_key().to_encoded_point(false);
    let shared = p256::ecdh::diffie_hellman(as_key.to_nonzero_scalar(), ua.as_affine());
    let k = derive(
        shared.raw_secret_bytes(),
        auth,
        ua_raw.as_bytes(),
        as_raw.as_bytes(),
        salt,
    );
    // one record, the last: the plaintext, then the 0x02 delimiter
    let mut rec = Vec::with_capacity(plaintext.len() + 1);
    rec.extend_from_slice(plaintext);
    rec.push(2);
    let ct = aes_gcm::Aes128Gcm::new_from_slice(&k.cek)
        .map_err(|_| "AES key".to_string())?
        .encrypt(aes_gcm::Nonce::from_slice(&k.nonce), rec.as_slice())
        .map_err(|_| "AES-GCM failed".to_string())?;
    // the aes128gcm header: salt, record size, the sender's key as key id
    let mut out = Vec::with_capacity(86 + ct.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD.to_be_bytes());
    out.push(65);
    out.extend_from_slice(as_raw.as_bytes());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// [`encrypt_with`] a fresh sender key and salt (every message has its own).
pub fn encrypt(ua_public: &[u8], auth: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let key = loop {
        if let Ok(k) = p256::SecretKey::from_slice(&crate::devices::random(32)?) {
            break k;
        }
    };
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&crate::devices::random(16)?);
    encrypt_with(ua_public, auth, plaintext, &key, &salt)
}

// ------------------------------------------------------------------ subscriptions

#[derive(Clone, Debug, PartialEq)]
pub struct Sub {
    pub device: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub created: u64,
    /// refusals in a row (4xx other than 404 / 410 / 429)
    pub fails: u32,
}

impl Sub {
    fn to_json(&self) -> Value {
        json!({"device": self.device, "endpoint": self.endpoint, "p256dh": self.p256dh, "auth": self.auth, "created": self.created, "fails": self.fails})
    }
    fn from_json(v: &Value) -> Option<Sub> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Sub {
            device: s("device")?,
            endpoint: s("endpoint")?,
            p256dh: s("p256dh")?,
            auth: s("auth")?,
            created: v.get("created").and_then(Value::as_u64).unwrap_or(0),
            fails: v.get("fails").and_then(Value::as_u64).unwrap_or(0) as u32,
        })
    }
}

/// May the daemon POST to this endpoint? (https, a known push service.)
pub fn check_endpoint(u: &str) -> Result<(), String> {
    if u.len() > 2048
        || u.chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '"')
    {
        return Err("that endpoint isn't a URL".into());
    }
    let Some(o) = origin(u) else {
        return Err("push endpoints are https:// URLs".into());
    };
    let host = o
        .trim_start_matches("https://")
        .split(':')
        .next()
        .unwrap_or("")
        .to_lowercase();
    if PUSH_HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
    {
        Ok(())
    } else {
        Err(format!(
            "{host} isn't a push service Aleph knows (FCM, Mozilla, Apple, Windows)"
        ))
    }
}

/// A browser's `PushSubscription.toJSON()` → a subscription for `device`.
pub fn parse_sub(device: &str, v: &Value, now: u64) -> Result<Sub, String> {
    let endpoint = v
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or("endpoint: missing")?;
    check_endpoint(endpoint)?;
    let keys = v.get("keys").ok_or("keys: missing")?;
    let p256dh = keys
        .get("p256dh")
        .and_then(Value::as_str)
        .ok_or("keys.p256dh: missing")?;
    let auth = keys
        .get("auth")
        .and_then(Value::as_str)
        .ok_or("keys.auth: missing")?;
    let pk = b64url_decode(p256dh).ok_or("keys.p256dh isn't base64url")?;
    if pk.len() != 65 || p256::PublicKey::from_sec1_bytes(&pk).is_err() {
        return Err("keys.p256dh isn't a P-256 public key".into());
    }
    if b64url_decode(auth).is_none_or(|a| a.len() != 16) {
        return Err("keys.auth isn't 16 bytes of base64url".into());
    }
    Ok(Sub {
        device: device.to_string(),
        endpoint: endpoint.to_string(),
        p256dh: b64url(&pk),
        auth: auth.trim_end_matches('=').to_string(),
        created: now,
        fails: 0,
    })
}

/// What a push service said about one delivery.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Sent,
    /// 404 / 410: the subscription is gone (unsubscribed, expired)
    Gone,
    /// any other 4xx: this message (or our credentials) refused
    Refused(u16),
    /// 429, 5xx or no answer: try again later
    Retry(String),
}

pub fn outcome(code: u16) -> Outcome {
    match code {
        200..=299 => Outcome::Sent,
        404 | 410 => Outcome::Gone,
        429 => Outcome::Retry("rate limited (429)".into()),
        400..=499 => Outcome::Refused(code),
        0 => Outcome::Retry("no answer".into()),
        c => Outcome::Retry(format!("the push service answered {c}")),
    }
}

/// `state/push.json`.
pub struct Store {
    path: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Store::new()
    }
}

impl Store {
    pub fn new() -> Store {
        Store {
            path: state_dir().join("push.json"),
        }
    }

    #[cfg(test)]
    pub fn at(path: PathBuf) -> Store {
        Store { path }
    }

    fn lock(&self) -> FileLock {
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        FileLock::take(&self.path.with_file_name(".push.lock"))
    }

    pub fn list(&self) -> Vec<Sub> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("subs").and_then(Value::as_array).cloned())
            .map(|a| a.iter().filter_map(Sub::from_json).collect())
            .unwrap_or_default()
    }

    fn save(&self, v: &[Sub]) -> Result<(), String> {
        let doc = json!({"version": 1, "subs": v.iter().map(Sub::to_json).collect::<Vec<_>>()});
        write_private(&self.path, (doc.to_string() + "\n").as_bytes()).map_err(|e| e.to_string())
    }

    /// Keep a subscription (an endpoint belongs to one device; at most five
    /// per device, the oldest goes).
    pub fn add(&self, s: Sub) -> Result<(), String> {
        let _l = self.lock();
        let mut v = self.list();
        v.retain(|x| x.endpoint != s.endpoint);
        let mine: Vec<usize> = (0..v.len()).filter(|i| v[*i].device == s.device).collect();
        if mine.len() >= 5 {
            v.remove(mine[0]);
        }
        v.push(s);
        self.save(&v)
    }

    pub fn remove(&self, device: &str, endpoint: &str) -> bool {
        let _l = self.lock();
        let mut v = self.list();
        let n = v.len();
        v.retain(|x| !(x.device == device && x.endpoint == endpoint));
        n != v.len() && self.save(&v).is_ok()
    }

    /// A revoked device's subscriptions go with it. How many there were.
    pub fn drop_device(&self, device: &str) -> usize {
        let _l = self.lock();
        let mut v = self.list();
        let n = v.len();
        v.retain(|x| x.device != device);
        if n != v.len() {
            let _ = self.save(&v);
        }
        n - v.len()
    }

    /// What the push services said: gone ones are dropped, refusals counted
    /// (five in a row and it's dropped too), a delivery clears the count.
    pub fn apply(&self, results: &[(String, Outcome)]) {
        if results.is_empty() {
            return;
        }
        let _l = self.lock();
        let mut v = self.list();
        let before = v.clone();
        v.retain_mut(|s| match results.iter().find(|(e, _)| *e == s.endpoint) {
            Some((_, Outcome::Gone)) => {
                crate::log::info(
                    "webpush",
                    &format!("{}: a subscription is gone; dropped it", s.device),
                );
                false
            }
            Some((_, Outcome::Refused(c))) => {
                s.fails += 1;
                if s.fails >= 5 {
                    crate::log::warn(
                        "webpush",
                        &format!(
                            "{}: the push service refused it {} times ({c}); dropped it",
                            s.device, s.fails
                        ),
                    );
                }
                s.fails < 5
            }
            Some((_, Outcome::Sent)) => {
                s.fails = 0;
                true
            }
            _ => true,
        });
        if v != before {
            let _ = self.save(&v);
        }
    }
}

/// Is anyone subscribed? (For the default routes; looked at again only when
/// the file changes.)
pub fn has_subs() -> bool {
    static C: std::sync::Mutex<Option<(Option<std::time::SystemTime>, bool)>> =
        std::sync::Mutex::new(None);
    let st = Store::new();
    let m = std::fs::metadata(&st.path).and_then(|m| m.modified()).ok();
    let mut c = C.lock().unwrap();
    if let Some((mt, v)) = *c {
        if mt == m {
            return v;
        }
    }
    let v = m.is_some() && !st.list().is_empty();
    *c = Some((m, v));
    v
}

// ------------------------------------------------------------------ messages

/// Approve / Deny tokens for a question (None: remote control doesn't
/// cover that session, or it isn't asking).
pub fn buttons(cfg: &Config, s: &crate::herdr::Session) -> Option<Value> {
    let (yes, no) = crate::actions::tokens(cfg, s)?;
    Some(json!({"approve": yes, "deny": no}))
}

/// The notification the service worker shows (pure — tested).
#[cfg(test)]
pub fn payload(e: &Event, buttons: Option<&Value>) -> Vec<u8> {
    payload_msg(e, buttons, None)
}

/// [`payload`] with the alert's composed words (title, body) when there are.
pub fn payload_msg(e: &Event, buttons: Option<&Value>, m: Option<&crate::message::Msg>) -> Vec<u8> {
    let session = if e.name.is_empty() {
        String::new()
    } else {
        format!("{}:{}", e.host, e.name)
    };
    let text = |n: usize| {
        if let Some(m) = m {
            return crate::ui::trunc(&m.body, n);
        }
        let t = if e.body.is_empty() || e.body == e.title {
            e.title.clone()
        } else if e.title.is_empty() {
            e.body.clone()
        } else {
            format!("{}\n{}", e.title, e.body)
        };
        crate::ui::trunc(&t, n)
    };
    let v = json!({
        "title": m.map(|m| m.title.clone()).unwrap_or_else(|| crate::notify::title_of(e)),
        "body": text(600),
        "tag": format!("{}|{}", e.host, e.name),
        "kind": e.kind.as_str(),
        "host": e.host,
        "name": e.name,
        "session": session,
        "agent": e.agent,
        "t": e.t,
        "id": e.id(),
        "url": if session.is_empty() { "/#/inbox".to_string() } else { format!("/#/s/{session}") },
        "urgent": matches!(e.kind, Kind::Blocked | Kind::Error | Kind::Limit),
    });
    // showcase mode: masked (the action tokens, added below, stay as they are)
    let mut v = crate::showcase::masked(v);
    if let Some(b) = buttons {
        v["actions"] = json!([
            {"action": "approve", "title": "Approve"},
            {"action": "deny", "title": "Deny"},
            {"action": "reply", "title": "Reply"},
        ]);
        v["tokens"] = b.clone();
    }
    let mut out = v.to_string().into_bytes();
    if out.len() > MAX_PLAIN {
        v["body"] = json!(text(140));
        out = v.to_string().into_bytes();
    }
    out
}

fn urgency(k: Kind) -> &'static str {
    match k {
        Kind::Blocked | Kind::Error | Kind::Limit => "high",
        Kind::Done | Kind::HostUp | Kind::Remote | Kind::Task => "normal",
        _ => "low",
    }
}

/// The curl config for one delivery (pure — tested): everything that's a
/// secret travels here, on stdin.
pub fn curl_config(
    endpoint: &str,
    authorization: &str,
    ttl: u64,
    urgency: &str,
    topic: Option<&str>,
    body_file: &std::path::Path,
) -> String {
    let mut c = String::new();
    let mut opt = |k: &str, v: &str| {
        c.push_str(k);
        c.push_str(" = ");
        c.push_str(&crate::notify::cq(v));
        c.push('\n');
    };
    opt("url", endpoint);
    opt("request", "POST");
    opt("header", &format!("TTL: {ttl}"));
    opt("header", &format!("Urgency: {urgency}"));
    if let Some(t) = topic {
        opt("header", &format!("Topic: {t}"));
    }
    opt("header", "Content-Type: application/octet-stream");
    opt("header", "Content-Encoding: aes128gcm");
    opt("header", &format!("Authorization: {authorization}"));
    opt("data-binary", &format!("@{}", body_file.display()));
    c
}

/// One message to one subscription (blocking).
fn deliver(v: &Vapid, s: &Sub, plain: &[u8], e: &Event, contact: &str) -> Outcome {
    let (Some(ua), Some(auth)) = (b64url_decode(&s.p256dh), b64url_decode(&s.auth)) else {
        return Outcome::Refused(0);
    };
    let body = match encrypt(&ua, &auth, plain) {
        Ok(b) => b,
        Err(x) => {
            crate::log::warn("webpush", &format!("{}: {x}", s.device));
            return Outcome::Refused(0);
        }
    };
    let now = crate::cache::now();
    let Some(authz) = v.authorization(&s.endpoint, now, contact) else {
        return Outcome::Refused(0);
    };
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = state_dir()
        .join("push-out")
        .join(format!("{}-{n}.bin", std::process::id()));
    if let Err(x) = write_private(&file, &body) {
        return Outcome::Retry(format!("couldn't write the message: {x}"));
    }
    // a session's newer message replaces an undelivered older one
    let topic = (!e.name.is_empty()).then(|| {
        let h = Sha256::digest(format!("{}|{}", e.host, e.name).as_bytes());
        b64url(&h)[..22].to_string()
    });
    let conf = curl_config(
        &s.endpoint,
        &authz,
        if e.kind == Kind::Blocked {
            4 * 3600
        } else {
            3600
        },
        urgency(e.kind),
        topic.as_deref(),
        &file,
    );
    let r = crate::proc::run(
        Command::new("curl").args([
            "-sS",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-m",
            "15",
            "-K",
            "-",
        ]),
        T_SEND,
        Some(conf.as_bytes()),
    );
    let _ = std::fs::remove_file(&file);
    match r {
        Ok(o) => {
            let code: u16 = o.text().trim().parse().unwrap_or(0);
            match outcome(code) {
                Outcome::Retry(_) if !o.ok() => Outcome::Retry(o.reason()),
                x => x,
            }
        }
        Err(x) => Outcome::Retry(format!("curl: {x}")),
    }
}

/// Send `e` to these subscriptions; the outcome of each is applied to the
/// store. How many got it, or why none did.
fn send_to(
    cfg: &Config,
    subs: &[Sub],
    e: &Event,
    buttons: Option<&Value>,
    m: Option<&crate::message::Msg>,
) -> Result<usize, String> {
    // (showcase's demo world reaches no network)
    if subs.is_empty() || crate::showcase::quiet_sink(true) {
        return Ok(0);
    }
    let v = Vapid::load()?;
    let plain = payload_msg(e, buttons, m);
    let contact = cfg
        .get("api.push_contact")
        .filter(|c| c.starts_with("mailto:") || c.starts_with("https://"))
        .unwrap_or_else(|| CONTACT.to_string());
    let mut results = Vec::new();
    for s in subs {
        let o = deliver(&v, s, &plain, e, &contact);
        if let Outcome::Refused(c) = &o {
            crate::log::warn(
                "webpush",
                &format!("{}: the push service refused it ({c})", s.device),
            );
        }
        results.push((s.endpoint.clone(), o));
    }
    Store::new().apply(&results);
    let sent = results.iter().filter(|(_, o)| *o == Outcome::Sent).count();
    let retry = results.iter().find_map(|(_, o)| match o {
        Outcome::Retry(x) => Some(x.clone()),
        _ => None,
    });
    match (sent, retry) {
        (0, Some(x)) => Err(x),
        _ => Ok(sent),
    }
}

/// The router's sink: every subscription. Err only when it should be tried
/// again (no subscription got it, and one might next time).
pub fn notify(
    cfg: &Config,
    e: &Event,
    buttons: Option<&Value>,
    m: Option<&crate::message::Msg>,
) -> Result<(), String> {
    send_to(cfg, &Store::new().list(), e, buttons, m).map(|_| ())
}

/// One device's subscriptions (the console's "send a test").
pub fn notify_device(cfg: &Config, device: &str, e: &Event) -> Result<usize, String> {
    let subs: Vec<Sub> = Store::new()
        .list()
        .into_iter()
        .filter(|s| s.device == device)
        .collect();
    if subs.is_empty() {
        return Err("this device has no push subscription yet".into());
    }
    send_to(cfg, &subs, e, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Vec<u8> {
        b64url_decode(s).unwrap()
    }

    // RFC 8291, Section 5 and Appendix A
    const PLAIN: &str = "V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24";
    const AS_PUBLIC: &str =
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const MESSAGE: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    #[test]
    fn base64url_round_trips() {
        for n in 0..40 {
            let b: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(b64url_decode(&b64url(&b)).unwrap(), b, "{n}");
        }
        assert_eq!(b64url(b"When I grow up, I want to be a watermelon"), PLAIN);
        assert_eq!(b64url_decode("TWFu").unwrap(), b"Man");
        assert_eq!(b64url_decode("TWE=").unwrap(), b"Ma");
        assert!(b64url_decode("no!").is_none());
    }

    #[test]
    fn rfc8291_intermediate_values() {
        let as_key = p256::SecretKey::from_slice(&d(AS_PRIVATE)).unwrap();
        assert_eq!(
            b64url(as_key.public_key().to_encoded_point(false).as_bytes()),
            AS_PUBLIC
        );
        let ua = p256::PublicKey::from_sec1_bytes(&d(UA_PUBLIC)).unwrap();
        let shared = p256::ecdh::diffie_hellman(as_key.to_nonzero_scalar(), ua.as_affine());
        assert_eq!(
            b64url(shared.raw_secret_bytes()),
            "kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs"
        );
        // the receiver gets the same secret from its side
        let ua_key = p256::SecretKey::from_slice(&d(UA_PRIVATE)).unwrap();
        let back =
            p256::ecdh::diffie_hellman(ua_key.to_nonzero_scalar(), as_key.public_key().as_affine());
        assert_eq!(back.raw_secret_bytes(), shared.raw_secret_bytes());
        let k = derive(
            shared.raw_secret_bytes(),
            &d(AUTH),
            &d(UA_PUBLIC),
            &d(AS_PUBLIC),
            &d(SALT),
        );
        assert_eq!(
            b64url(&k.prk_key),
            "Snr3JMxaHVDXHWJn5wdC52WjpCtd2EIEGBykDcZW32k"
        );
        assert_eq!(
            b64url(&k.ikm),
            "S4lYMb_L0FxCeq0WhDx813KgSYqU26kOyzWUdsXYyrg"
        );
        assert_eq!(
            b64url(&k.prk),
            "09_eUZGrsvxChDCGRCdkLiDXrReGOEVeSCdCcPBSJSc"
        );
        assert_eq!(b64url(&k.cek), "oIhVW04MRdy2XN9CiKLxTg");
        assert_eq!(b64url(&k.nonce), "4h_95klXJ5E_qnoN");
        // the info strings as the RFC prints them
        assert_eq!(
            b64url(b"Content-Encoding: aes128gcm\0"),
            "Q29udGVudC1FbmNvZGluZzogYWVzMTI4Z2NtAA"
        );
        assert_eq!(
            b64url(b"Content-Encoding: nonce\0"),
            "Q29udGVudC1FbmNvZGluZzogbm9uY2UA"
        );
        let mut info = b"WebPush: info\0".to_vec();
        info.extend(d(UA_PUBLIC));
        info.extend(d(AS_PUBLIC));
        assert_eq!(
            b64url(&info),
            "V2ViUHVzaDogaW5mbwAEJXGyvs3942BVGq8e0PTNNmwRzr5VX4m8t7GGpTM5FzFo7OLr4BhZe9MEebhuPI-OztV3ylkYfpJGmQ22ggCLDgT-M_SrDepxkU21WCP3O1SUj0EwbZIHMtu5pZpTKGSCIA5Zent7wmC6HCJ5mFgJkuk5cwAvMBKiiujwa7t45ewP"
        );
    }

    #[test]
    fn rfc8291_message() {
        let as_key = p256::SecretKey::from_slice(&d(AS_PRIVATE)).unwrap();
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&d(SALT));
        let out = encrypt_with(&d(UA_PUBLIC), &d(AUTH), &d(PLAIN), &as_key, &salt).unwrap();
        assert_eq!(b64url(&out), MESSAGE);
        // the 86-byte header, then the ciphertext the RFC shows
        assert_eq!(
            b64url(&out[..86]),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"
        );
        assert_eq!(
            b64url(&out[86..]),
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ"
        );
    }

    /// What a browser does with a push message (RFC 8291 from the receiving
    /// side), to check fresh encryptions.
    fn decrypt(ua_private: &[u8], auth: &[u8], msg: &[u8]) -> Vec<u8> {
        use aes_gcm::aead::{Aead, KeyInit};
        let salt = &msg[..16];
        assert_eq!(u32::from_be_bytes(msg[16..20].try_into().unwrap()), 4096);
        assert_eq!(msg[20], 65);
        let as_public = &msg[21..86];
        let ua_key = p256::SecretKey::from_slice(ua_private).unwrap();
        let ua_public = ua_key.public_key().to_encoded_point(false);
        let as_pk = p256::PublicKey::from_sec1_bytes(as_public).unwrap();
        let shared = p256::ecdh::diffie_hellman(ua_key.to_nonzero_scalar(), as_pk.as_affine());
        let k = derive(
            shared.raw_secret_bytes(),
            auth,
            ua_public.as_bytes(),
            as_public,
            salt,
        );
        let mut p = aes_gcm::Aes128Gcm::new_from_slice(&k.cek)
            .unwrap()
            .decrypt(aes_gcm::Nonce::from_slice(&k.nonce), &msg[86..])
            .unwrap();
        assert_eq!(p.pop(), Some(2), "the last record's delimiter");
        p
    }

    #[test]
    fn fresh_messages_decrypt() {
        let plain = payload(
            &Event {
                t: 1,
                host: "b31".into(),
                name: "temper".into(),
                kind: Kind::Blocked,
                title: "Run the cargo benches?".into(),
                ..Default::default()
            },
            Some(&json!({"approve": "v1.a", "deny": "v1.b"})),
        );
        let a = encrypt(&d(UA_PUBLIC), &d(AUTH), &plain).unwrap();
        let b = encrypt(&d(UA_PUBLIC), &d(AUTH), &plain).unwrap();
        assert_ne!(a, b, "a new key and salt every time");
        assert_eq!(decrypt(&d(UA_PRIVATE), &d(AUTH), &a), plain);
        let v: Value = serde_json::from_slice(&decrypt(&d(UA_PRIVATE), &d(AUTH), &b)).unwrap();
        assert_eq!(v["session"], "b31:temper");
        assert_eq!(v["url"], "/#/s/b31:temper");
        assert_eq!(v["actions"][0]["action"], "approve");
        assert_eq!(v["tokens"]["deny"], "v1.b");
        assert!(encrypt(&d(UA_PUBLIC), &d(AUTH)[..8], &plain).is_err());
        assert!(encrypt(&d(UA_PUBLIC), &d(AUTH), &[b'x'; MAX_PLAIN + 1]).is_err());
    }

    #[test]
    fn vapid_jwt_shape() {
        // RFC 8292 §2.4: the same header and claims, byte for byte
        let v = Vapid::from_bytes(&d(AS_PRIVATE)).unwrap();
        let t = v.jwt(
            "https://push.example.net",
            1453523768,
            "mailto:push@example.com",
        );
        let parts: Vec<&str> = t.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9");
        assert_eq!(
            parts[1],
            "eyJhdWQiOiJodHRwczovL3B1c2guZXhhbXBsZS5uZXQiLCJleHAiOjE0NTM1MjM3NjgsInN1YiI6Im1haWx0bzpwdXNoQGV4YW1wbGUuY29tIn0"
        );
        // ES256: a raw 64-byte r‖s that our public key verifies
        use p256::ecdsa::signature::Verifier;
        let sig = d(parts[2]);
        assert_eq!(sig.len(), 64);
        let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&v.public()).unwrap();
        let input = format!("{}.{}", parts[0], parts[1]);
        vk.verify(
            input.as_bytes(),
            &p256::ecdsa::Signature::from_slice(&sig).unwrap(),
        )
        .unwrap();
        // and the RFC's own example token checks out the same way
        let rfc_k = d("BA1Hxzyi1RUM1b5wjxsn7nGxAszw2u61m164i3MrAIxHF6YK5h4SDYic-dRuU_RCPCfA5aq9ojSwk5Y2EmClBPs");
        let rfc_sig = d("i3CYb7t4xfxCDquptFOepC9GAu_HLGkMlMuCGSK2rpiUfnK9ojFwDXb1JrErtmysazNjjvW2L9OkSSHzvoD1oA");
        p256::ecdsa::VerifyingKey::from_sec1_bytes(&rfc_k)
            .unwrap()
            .verify(
                input.as_bytes(),
                &p256::ecdsa::Signature::from_slice(&rfc_sig).unwrap(),
            )
            .unwrap();
        // the header the push service gets
        let h = v
            .authorization("https://fcm.googleapis.com/fcm/send/abc:def", 1000, CONTACT)
            .unwrap();
        assert!(h.starts_with("vapid t=ey") && h.ends_with(&format!(", k={}", v.public_b64())));
        let claims = d(h.trim_start_matches("vapid t=").split('.').nth(1).unwrap());
        let c: Value = serde_json::from_slice(&claims).unwrap();
        assert_eq!(c["aud"], "https://fcm.googleapis.com");
        assert_eq!(c["exp"], 1000 + 12 * 3600);
        assert_eq!(v.public().len(), 65);
    }

    #[test]
    fn subscriptions_are_checked() {
        let ok = json!({"endpoint": "https://fcm.googleapis.com/fcm/send/abc", "keys": {"p256dh": UA_PUBLIC, "auth": AUTH}});
        let s = parse_sub("phone", &ok, 5).unwrap();
        assert_eq!((s.device.as_str(), s.created), ("phone", 5));
        for (ep, why) in [
            ("http://fcm.googleapis.com/x", "https"),
            ("https://evil.example.com/push", "push service"),
            ("https://fcm.googleapis.com.evil.com/x", "push service"),
            ("https://user@fcm.googleapis.com/x", "https"),
        ] {
            let mut v = ok.clone();
            v["endpoint"] = json!(ep);
            let e = parse_sub("phone", &v, 5).unwrap_err();
            assert!(e.contains(why), "{ep}: {e}");
        }
        assert!(check_endpoint("https://web.push.apple.com/QGuQ").is_ok());
        assert!(check_endpoint("https://updates.push.services.mozilla.com/wpush/v2/x").is_ok());
        assert!(check_endpoint("https://wns2-par02p.notify.windows.com/w/?token=x").is_ok());
        let mut v = ok.clone();
        v["keys"]["p256dh"] = json!("AAAA");
        assert!(parse_sub("phone", &v, 5).is_err());
        let mut v = ok.clone();
        v["keys"]["auth"] = json!("AAAA");
        assert!(parse_sub("phone", &v, 5).is_err());
    }

    #[test]
    fn gone_subscriptions_are_dropped() {
        let p = std::env::temp_dir()
            .join(format!("al-push-{}", std::process::id()))
            .join("push.json");
        let _ = std::fs::remove_file(&p);
        let st = Store::at(p.clone());
        let sub = |dev: &str, n: u32| Sub {
            device: dev.into(),
            endpoint: format!("https://fcm.googleapis.com/fcm/send/{dev}{n}"),
            p256dh: UA_PUBLIC.into(),
            auth: AUTH.into(),
            created: 1,
            fails: 0,
        };
        st.add(sub("phone", 1)).unwrap();
        st.add(sub("phone", 2)).unwrap();
        st.add(sub("ipad", 1)).unwrap();
        // the same endpoint again replaces it
        st.add(sub("phone", 2)).unwrap();
        assert_eq!(st.list().len(), 3);
        st.apply(&[
            (sub("phone", 1).endpoint, outcome(410)),
            (sub("phone", 2).endpoint, outcome(201)),
            (sub("ipad", 1).endpoint, outcome(403)),
        ]);
        // (adding phone 2 again moved it last)
        let left: Vec<String> = st.list().iter().map(|s| s.endpoint.clone()).collect();
        assert_eq!(
            left,
            vec![sub("ipad", 1).endpoint, sub("phone", 2).endpoint]
        );
        assert_eq!(st.list()[0].fails, 1);
        // 404 is gone too; retries keep it
        st.apply(&[
            (sub("ipad", 1).endpoint, outcome(503)),
            (sub("phone", 2).endpoint, outcome(404)),
        ]);
        assert_eq!(st.list().len(), 1);
        // five refusals in a row and it goes
        for _ in 0..4 {
            st.apply(&[(sub("ipad", 1).endpoint, outcome(400))]);
        }
        assert!(st.list().is_empty());
        // revoking a device drops its subscriptions
        st.add(sub("phone", 3)).unwrap();
        st.add(sub("ipad", 3)).unwrap();
        assert_eq!(st.drop_device("phone"), 1);
        assert_eq!(st.list().len(), 1);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(outcome(0), Outcome::Retry("no answer".into()));
        assert_eq!(outcome(429), Outcome::Retry("rate limited (429)".into()));
    }

    #[test]
    fn curl_gets_secrets_on_stdin() {
        let c = curl_config(
            "https://fcm.googleapis.com/fcm/send/abc",
            "vapid t=x.y.z, k=K",
            3600,
            "high",
            Some("tpc"),
            std::path::Path::new("/tmp/m.bin"),
        );
        assert!(c.contains("url = \"https://fcm.googleapis.com/fcm/send/abc\"\n"));
        assert!(c.contains("header = \"Content-Encoding: aes128gcm\"\n"));
        assert!(c.contains("header = \"TTL: 3600\"\n"));
        assert!(c.contains("header = \"Urgency: high\"\n"));
        assert!(c.contains("header = \"Topic: tpc\"\n"));
        assert!(c.contains("header = \"Authorization: vapid t=x.y.z, k=K\"\n"));
        assert!(c.contains("data-binary = \"@/tmp/m.bin\"\n"));
    }

    #[test]
    fn payloads_fit() {
        let e = Event {
            t: 1,
            host: "b31".into(),
            name: "temper".into(),
            kind: Kind::Done,
            title: "x".repeat(5000),
            ..Default::default()
        };
        let p = payload(&e, None);
        assert!(p.len() <= MAX_PLAIN);
        let v: Value = serde_json::from_slice(&p).unwrap();
        assert!(v.get("actions").is_none());
        assert_eq!(v["urgent"], false);
        assert_eq!(urgency(Kind::Blocked), "high");
    }
}
