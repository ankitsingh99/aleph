//! One small log for everything Aleph does on its own: machines going down and
//! coming back, reconnects, answers sent (by rules, from a phone), sink
//! failures, the daemon starting. Not every poll.
//!
//! * `state/aleph.log`, `key=value` lines, rotated at 1 MB (3 kept).
//! * Level: `ALEPH_LOG=error|warn|info|debug` (or `--debug`), else the
//!   `log.level` setting (info). Debug adds every external command (what
//!   `--debug` always did).
//! * Secrets never land here: URLs of notification sinks, tokens and
//!   passwords are masked (`redact`).
//! * A panic writes `state/crash-<time>.log` (message, place, backtrace, the
//!   last log lines); the next start says so once.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.trim().to_lowercase().as_str() {
            "error" | "err" => Some(Level::Error),
            "warn" | "warning" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" | "1" | "true" | "on" => Some(Level::Debug),
            _ => None,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);
static LOCK: Mutex<()> = Mutex::new(());
const MAX: u64 = 1024 * 1024;
const KEEP: usize = 3;

/// Set the level: `ALEPH_LOG` wins, then `--debug`, then the setting.
pub fn init(debug_flag: bool, setting: &str) {
    let l = crate::brand::env("LOG")
        .and_then(|v| Level::parse(&v))
        .or(debug_flag.then_some(Level::Debug))
        .or_else(|| Level::parse(setting))
        .unwrap_or(Level::Info);
    LEVEL.store(l as u8, Ordering::Relaxed);
}

pub fn enabled(l: Level) -> bool {
    (l as u8) <= LEVEL.load(Ordering::Relaxed)
}

pub fn path() -> PathBuf {
    crate::config::state_dir().join("aleph.log")
}

/// Secret values this process has read from the keystore (an API key): masked
/// wherever text is written down, whatever their shape.
static KNOWN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Remember a secret's value (`secrets::get` does, for every one it reads),
/// so [`redact`] — and with it this log, the debug log, `sentlog::scrub` —
/// masks it wherever it turns up. Values under 8 characters aren't kept (too
/// likely to be an ordinary word).
pub fn mask(v: &str) {
    let v = v.trim();
    if v.chars().count() < 8 {
        return;
    }
    if let Ok(mut k) = KNOWN.lock() {
        if !k.iter().any(|x| x == v) {
            k.push(v.to_string());
        }
    }
}

/// `s` with every remembered secret value replaced by `…`.
pub fn mask_known(s: &str) -> std::borrow::Cow<'_, str> {
    let Ok(k) = KNOWN.lock() else {
        return s.into();
    };
    if !k.iter().any(|v| s.contains(v.as_str())) {
        return s.into();
    }
    let mut out = s.to_string();
    for v in k.iter() {
        out = out.replace(v.as_str(), "…");
    }
    out.into()
}

/// Mask what must never be written down: secret values read from the
/// keystore ([`mask`]), notification URLs (the topic or webhook path *is* the
/// secret), bearer tokens, `token=`/`password=` values.
pub fn redact(s: &str) -> String {
    let s = mask_known(s);
    let mut out = String::with_capacity(s.len());
    for word in s.split_inclusive(char::is_whitespace) {
        let (w, ws) = match word.char_indices().last() {
            Some((i, c)) if c.is_whitespace() => (&word[..i], &word[i..]),
            _ => (word, ""),
        };
        out.push_str(&redact_word(w));
        out.push_str(ws);
    }
    // `Bearer xyz` → `Bearer …`
    let mut r = out;
    for key in ["Bearer ", "bearer "] {
        if let Some(i) = r.find(key) {
            let rest = &r[i + key.len()..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(rest.len());
            r = format!("{}{key}…{}", &r[..i], &rest[end..]);
        }
    }
    r
}

fn redact_word(w: &str) -> String {
    let lower = w.to_lowercase();
    // secret-bearing URLs: keep scheme + host, hide the path
    // device tokens (the HTTP API): the prefix and 64 hex digits
    let pre = crate::devices::TOKEN_PREFIX;
    if let Some(i) = lower.find(pre) {
        let rest = &lower.as_bytes()[i + pre.len()..];
        if rest
            .get(..64)
            .is_some_and(|b| b.iter().all(u8::is_ascii_hexdigit))
        {
            return format!("{}{pre}…", &w[..i]);
        }
    }
    let secret_hosts = [
        "ntfy.sh",
        "hooks.slack.com",
        "discord.com/api/webhooks",
        "api.telegram.org",
        "api.getmoshi.app",
        // Web Push endpoints are capability URLs
        "fcm.googleapis.com",
        "push.services.mozilla.com",
        "push.apple.com",
        "notify.windows.com",
    ];
    if let Some(i) = lower.find("://") {
        let host_end = lower[i + 3..].find('/').map(|j| i + 3 + j);
        let is_secret = secret_hosts.iter().any(|h| lower.contains(h))
            || lower.contains("token=")
            || lower.contains("key=");
        if let (true, Some(he)) = (is_secret, host_end) {
            return format!("{}/…", &w[..he]);
        }
    }
    for k in [
        "token=",
        "password=",
        "passwd=",
        "secret=",
        "apikey=",
        "api_key=",
    ] {
        if let Some(i) = lower.find(k) {
            return format!("{}…", &w[..i + k.len()]);
        }
    }
    w.to_string()
}

fn rotate(p: &std::path::Path) {
    if fs::metadata(p).map(|m| m.len()).unwrap_or(0) < MAX {
        return;
    }
    for i in (1..KEEP).rev() {
        let _ = fs::rename(
            p.with_extension(format!("log.{i}")),
            p.with_extension(format!("log.{}", i + 1)),
        );
    }
    let _ = fs::rename(p, p.with_extension("log.1"));
}

fn stamp() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    iso_utc_ms(t)
}

/// Write one line (when `level` is enabled).
pub fn write(level: Level, comp: &str, msg: &str) {
    if !enabled(level) {
        return;
    }
    let line = format!(
        "{} {:<5} [{}] {comp}: {}\n",
        stamp(),
        level.word(),
        std::process::id(),
        redact(msg).replace('\n', " ⏎ ")
    );
    let _g = LOCK.lock();
    let p = path();
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    rotate(&p);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
        }
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn error(comp: &str, msg: &str) {
    write(Level::Error, comp, msg)
}
pub fn warn(comp: &str, msg: &str) {
    write(Level::Warn, comp, msg)
}
pub fn info(comp: &str, msg: &str) {
    write(Level::Info, comp, msg)
}
pub fn debug(comp: &str, msg: &str) {
    write(Level::Debug, comp, msg)
}

/// The last `n` lines of the log (oldest first).
pub fn tail(n: usize) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for p in [path().with_extension("log.1"), path()] {
        if let Ok(t) = fs::read_to_string(&p) {
            v.extend(t.lines().map(String::from));
        }
    }
    let skip = v.len().saturating_sub(n);
    v.split_off(skip)
}

/// `2026-10-22T06:40:01.123Z` for milliseconds since the epoch.
pub fn iso_utc_ms(ms: u128) -> String {
    let secs = (ms / 1000) as i64;
    let (y, m, d) = crate::ui::civil(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        s / 3600,
        s % 3600 / 60,
        s % 60,
        ms % 1000
    )
}

/// `90s`, `30m`, `2h`, `3d` (a bare number is seconds).
pub fn parse_duration(s: &str) -> Option<std::time::Duration> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = n.parse().ok()?;
    if unit == "ms" {
        return Some(std::time::Duration::from_millis(n));
    }
    let mult = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        _ => return None,
    };
    Some(std::time::Duration::from_secs(n * mult))
}

// ------------------------------------------------------------------ crashes

/// Called from the panic hook: write `state/crash-<time>.log`.
pub fn crash(info: &std::panic::PanicHookInfo) {
    let bt = std::backtrace::Backtrace::force_capture();
    let place = info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()))
        .unwrap_or_default();
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into());
    let body = format!(
        "{} {} {} crashed\n\n{msg}\nat {place}\nargs: {}\n\nbacktrace:\n{bt}\n\nlast log lines:\n{}\n",
        crate::brand::NAME,
        env!("CARGO_PKG_VERSION"),
        stamp(),
        redact(&std::env::args().collect::<Vec<_>>().join(" ")),
        tail(200).join("\n"),
    );
    let d = crate::config::state_dir();
    let _ = fs::create_dir_all(&d);
    let p = d.join(format!("crash-{}.log", crate::cache::now()));
    let _ = fs::write(&p, body);
    error("crash", &format!("{msg} at {place} — {}", p.display()));
}

/// The newest crash report not reported yet (marks it seen).
pub fn unseen_crash() -> Option<PathBuf> {
    let d = crate::config::state_dir();
    let newest = latest_crash()?;
    let seen = d.join("crash-seen");
    let last = fs::read_to_string(&seen).unwrap_or_default();
    let name = newest.file_name()?.to_string_lossy().into_owned();
    if last.trim() == name {
        return None;
    }
    let _ = fs::write(&seen, &name);
    Some(newest)
}

pub fn latest_crash() -> Option<PathBuf> {
    let d = crate::config::state_dir();
    let mut v: Vec<PathBuf> = fs::read_dir(&d)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f.starts_with("crash-") && f.ends_with(".log"))
        })
        .collect();
    v.sort();
    v.pop()
}

// ------------------------------------------------------------------ aleph logs

use crate::cli::Args;
use crate::out::*;
use std::process::ExitCode;

/// `aleph logs [-f] [--level L] [--since 1h] [--host H] [-n N] [--crash] [--bundle]`
pub fn cmd(a: Args) -> ExitCode {
    let r = a.rest;
    let has = |f: &str| r.iter().any(|x| x == f);
    let val = |f: &str| {
        r.iter()
            .position(|x| x == f)
            .and_then(|i| r.get(i + 1))
            .cloned()
    };
    if has("--crash") {
        return match latest_crash() {
            Some(p) => {
                print!("{}", fs::read_to_string(&p).unwrap_or_default());
                hint(&format!("{}", p.display()));
                ExitCode::SUCCESS
            }
            None => {
                ok("no crash reports");
                ExitCode::SUCCESS
            }
        };
    }
    if has("--bundle") {
        return bundle(&a.cfg);
    }
    let min = val("--level")
        .and_then(|l| Level::parse(&l))
        .unwrap_or(Level::Info);
    let host = val("--host");
    let since = val("--since").and_then(|s| parse_duration(&s));
    let n: usize = val("-n").and_then(|v| v.parse().ok()).unwrap_or(200);
    let keep = |l: &str| -> bool {
        let lvl = l
            .split_whitespace()
            .nth(1)
            .and_then(Level::parse)
            .unwrap_or(Level::Info);
        if lvl > min {
            return false;
        }
        if let Some(h) = &host {
            if !l.contains(h.as_str()) {
                return false;
            }
        }
        if let Some(d) = since {
            let cutoff = iso_utc_ms(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|x| x.as_millis())
                    .unwrap_or(0)
                    .saturating_sub(d.as_millis()),
            );
            if l.split_whitespace()
                .next()
                .is_some_and(|t| t < cutoff.as_str())
            {
                return false;
            }
        }
        true
    };
    let lines: Vec<String> = tail(10_000).into_iter().filter(|l| keep(l)).collect();
    let skip = lines.len().saturating_sub(n);
    for l in &lines[skip..] {
        println!("{l}");
    }
    if !has("-f") {
        if lines.is_empty() {
            hint(&format!(
                "nothing at {} or above in {}",
                min.word().to_lowercase(),
                path().display()
            ));
        }
        return ExitCode::SUCCESS;
    }
    // follow
    let mut pos = fs::metadata(path()).map(|m| m.len()).unwrap_or(0);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let len = fs::metadata(path()).map(|m| m.len()).unwrap_or(0);
        if len < pos {
            pos = 0; // rotated
        }
        if len > pos {
            if let Ok(t) = fs::read(path()) {
                let s = String::from_utf8_lossy(&t[pos as usize..]).into_owned();
                for l in s.lines().filter(|l| keep(l)) {
                    println!("{l}");
                }
            }
            pos = len;
        }
    }
}

/// A redacted bundle for a bug report: logs, crash reports, doctor, version,
/// the config with secrets masked.
fn bundle(cfg: &crate::config::Config) -> ExitCode {
    let dir = std::env::temp_dir().join(format!("aleph-report-{}", crate::cache::now()));
    let _ = fs::create_dir_all(&dir);
    let mut logs = String::new();
    for l in tail(5000) {
        logs.push_str(&redact(&l));
        logs.push('\n');
    }
    let _ = fs::write(dir.join("aleph.log"), logs);
    if let Some(c) = latest_crash() {
        let _ = fs::write(
            dir.join("crash.log"),
            redact(&fs::read_to_string(&c).unwrap_or_default()),
        );
    }
    let mut conf = String::new();
    for (k, v) in cfg.entries() {
        let secret = [
            "ntfy", "slack", "webhook", "token", "secret", "password", "key",
        ]
        .iter()
        .any(|s| k.contains(s));
        conf.push_str(&format!(
            "{k}={}\n",
            if secret { "…".into() } else { redact(&v) }
        ));
    }
    let _ = fs::write(dir.join("config.txt"), conf);
    let _ = fs::write(
        dir.join("version.txt"),
        format!(
            "aleph {}\n{} {}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    let out_path = dir.with_extension("tar.gz");
    let o = crate::proc::cap(
        "tar",
        &[
            "-czf",
            &out_path.to_string_lossy(),
            "-C",
            &dir.parent().unwrap_or(&dir).to_string_lossy(),
            &dir.file_name().unwrap_or_default().to_string_lossy(),
        ],
        std::time::Duration::from_secs(30),
    );
    let _ = fs::remove_dir_all(&dir);
    if o.ok() {
        ok(&format!("bug report bundle: {}", out_path.display()));
        hint("secrets are masked; look inside before sharing it");
        ExitCode::SUCCESS
    } else {
        fail(&format!("tar: {}", o.reason()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_masked() {
        assert_eq!(
            redact("curl -d x https://ntfy.sh/my-secret-topic"),
            "curl -d x https://ntfy.sh/…"
        );
        assert_eq!(
            redact("POST https://hooks.slack.com/services/T0/B1/xyz ok"),
            "POST https://hooks.slack.com/… ok"
        );
        assert_eq!(
            redact("Authorization: Bearer abc.def end"),
            "Authorization: Bearer … end"
        );
        assert_eq!(redact("url?token=s3cret&x=1"), "url?token=…");
        assert_eq!(
            redact("ssh b31 herdr agent list"),
            "ssh b31 herdr agent list"
        );
        assert_eq!(redact("https://example.com/a/b"), "https://example.com/a/b");
    }

    #[test]
    fn secret_values_are_masked_whatever_their_shape() {
        // a key with no tell-tale prefix: only known by its value
        let k = "plainlookingvalue42";
        assert_eq!(redact(&format!("env {k} set")), format!("env {k} set"));
        mask(k);
        assert_eq!(redact(&format!("env {k} set")), "env … set");
        assert_eq!(mask_known(&format!("a{k}b")), "a…b");
        assert!(!crate::sentlog::scrub(&format!("use {k} please")).contains(k));
        // too short to be told from a word: never kept
        mask("short");
        assert_eq!(redact("a short word"), "a short word");
    }

    #[test]
    fn times_and_durations() {
        assert_eq!(iso_utc_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc_ms(1_792_630_022_123), "2026-10-22T00:47:02.123Z");
        assert_eq!(
            parse_duration("2h"),
            Some(std::time::Duration::from_secs(7200))
        );
        assert_eq!(
            parse_duration("45"),
            Some(std::time::Duration::from_secs(45))
        );
        assert_eq!(parse_duration("2x"), None);
    }

    #[test]
    fn levels_parse_and_order() {
        assert_eq!(Level::parse("WARN"), Some(Level::Warn));
        assert!(Level::Error < Level::Debug);
        assert_eq!(Level::parse("nope"), None);
    }
}
