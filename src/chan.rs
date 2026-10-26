//! A persistent per-host command channel: one `ssh <host> sh -s` whose stdin
//! we keep open and feed `herdr …` commands — so typing into a tile, quick
//! answers and broadcasts cost ~10 ms on the remote instead of a new ssh each.
//! Opened lazily by the views that need it, closed (ssh killed) on drop.
//!
//! Also resolves where herdr lives on a host (login-shell PATH + common
//! install dirs), cached for the session and on disk.

use crate::config::cache_dir;
use crate::proc::{self, sh_quote};
use crate::sshcfg;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// A command unanswered this long means the link stalled.
const STALL: Duration = Duration::from_secs(10);

/// Shell prelude that gives a non-login `sh` the user's PATH.
pub const PATH_PRELUDE: &str = r#"P=$("${SHELL:-/bin/sh}" -lc 'printf "\nCCP:%s\n" "$PATH"' </dev/null 2>/dev/null | sed -n 's/^CCP://p' | tail -n 1); [ -n "$P" ] && PATH="$P:$PATH"; PATH="$PATH:$HOME/.local/bin:$HOME/.cargo/bin:$HOME/bin:/opt/homebrew/bin:/usr/local/bin"; export PATH"#;

static PATHS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn disk(host: &str) -> std::path::PathBuf {
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
    cache_dir().join("hosts").join(format!("{safe}.herdr"))
}

/// herdr's named session per machine (`host.<a>.session`).
static SESSIONS: OnceLock<std::sync::RwLock<HashMap<String, String>>> = OnceLock::new();

/// Read `host.<a>.session` for the whole fleet (at start, and whenever it
/// changes in Settings).
pub fn set_sessions(cfg: &crate::config::Config) {
    let mut m = HashMap::new();
    for h in cfg.fleet() {
        if let Some(s) = cfg.host_get(&h, "session") {
            let s = s.trim();
            if !s.is_empty() {
                m.insert(h, s.to_string());
            }
        }
    }
    if let Ok(mut g) = SESSIONS.get_or_init(Default::default).write() {
        *g = m;
    }
}

/// The named herdr session Aleph uses on `host` (None: the default one).
pub fn session(host: &str) -> Option<String> {
    SESSIONS.get()?.read().ok()?.get(host).cloned()
}

/// `export HERDR_SESSION=<name>; ` to put before a shell line that runs
/// herdr on `host` — herdr's own way to pick a named session's socket (its
/// CLI, `status server`, `terminal session …`); empty for the default one.
pub fn herdr_env(host: &str) -> String {
    session(host)
        .map(|s| format!("export HERDR_SESSION={}; ", sh_quote(&s)))
        .unwrap_or_default()
}

/// Absolute path of `herdr` on `host` (falls back to plain `herdr`).
pub fn herdr_path(host: &str) -> String {
    // showcase's demo world: there's no herdr to find (or to remember)
    if let Some(dead) = crate::showcase::dead_bin() {
        return dead;
    }
    let m = PATHS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(p) = m.lock().unwrap().get(host) {
        return p.clone();
    }
    if let Ok(p) = std::fs::read_to_string(disk(host)) {
        let p = p.trim().to_string();
        if !p.is_empty() {
            m.lock().unwrap().insert(host.into(), p.clone());
            return p;
        }
    }
    if sshcfg::is_local(host) {
        let p = proc::which(&crate::herdr::bin())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(crate::herdr::bin);
        m.lock().unwrap().insert(host.into(), p.clone());
        return p;
    }
    let script = format!("{PATH_PRELUDE}; command -v herdr");
    let o = proc::run(
        &mut sshcfg::shell(host, "sh -s"),
        Duration::from_secs(25),
        Some(script.as_bytes()),
    );
    let p = o
        .ok()
        .filter(|o| o.ok())
        .and_then(|o| {
            o.text()
                .lines()
                .rev()
                .find(|l| l.starts_with('/'))
                .map(str::to_string)
        })
        .unwrap_or_else(|| "herdr".into());
    if p.starts_with('/') {
        let _ = crate::config::write_atomic(&disk(host), p.as_bytes());
    }
    m.lock().unwrap().insert(host.into(), p.clone());
    p
}

/// Forget a cached path (it stopped working).
pub fn forget(host: &str) {
    if let Some(m) = PATHS.get() {
        m.lock().unwrap().remove(host);
    }
    let _ = std::fs::remove_file(disk(host));
}

pub struct Chan {
    pub host: String,
    stdin: Option<ChildStdin>,
    child: Option<Child>,
    herdr: String,
    seq: u64,
    /// errors reported by the remote (command id, message)
    pub errors: Arc<Mutex<Vec<(u64, String)>>>,
    /// commands sent and not yet answered (each acks, ok or not)
    pending: Arc<Mutex<HashMap<u64, Instant>>>,
}

impl Chan {
    pub fn open(host: &str) -> std::io::Result<Chan> {
        if let Some(e) = crate::showcase::not_here() {
            return Err(std::io::Error::other(e));
        }
        let herdr = herdr_path(host);
        let env = herdr_env(host);
        let mut c = sshcfg::shell(host, "sh -s")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = c.stdin.take();
        // the named session, for every command this channel runs
        if let (false, Some(si)) = (env.is_empty(), stdin.as_mut()) {
            let _ = si.write_all(format!("{env}\n").as_bytes());
        }
        let errors: Arc<Mutex<Vec<(u64, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let pending: Arc<Mutex<HashMap<u64, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        if let Some(out) = c.stdout.take() {
            let (e, p) = (errors.clone(), pending.clone());
            thread::spawn(move || {
                for l in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(id) = l.strip_prefix("CCO\t") {
                        p.lock().unwrap().remove(&id.trim().parse().unwrap_or(0));
                    } else if let Some(rest) = l.strip_prefix("CCE\t") {
                        let (id, msg) = rest.split_once('\t').unwrap_or(("0", rest));
                        let id = id.parse().unwrap_or(0);
                        p.lock().unwrap().remove(&id);
                        let mut g = e.lock().unwrap();
                        g.push((id, msg.to_string()));
                        if g.len() > 50 {
                            g.remove(0);
                        }
                    }
                }
            });
        }
        let mut ch = Chan {
            host: host.into(),
            stdin,
            child: Some(c),
            herdr,
            seq: 0,
            errors,
            pending,
        };
        ch.raw("export LC_ALL=C")?;
        Ok(ch)
    }

    fn raw(&mut self, line: &str) -> std::io::Result<()> {
        let s = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("channel closed"))?;
        s.write_all(line.as_bytes())?;
        s.write_all(b"\n")?;
        s.flush()
    }

    /// Run `herdr <args…>` remotely; failures come back via `errors`.
    pub fn herdr(&mut self, args: &[&str]) -> std::io::Result<u64> {
        self.seq += 1;
        let id = self.seq;
        let mut cmd = sh_quote(&self.herdr);
        for a in args {
            cmd.push(' ');
            cmd.push_str(&sh_quote(a));
        }
        self.pending.lock().unwrap().insert(id, Instant::now());
        self.raw(&format!(
            "if o=$({cmd} 2>&1 >/dev/null); then printf 'CCO\\t{id}\\n'; else printf 'CCE\\t{id}\\t%s\\n' \"$(printf '%s' \"$o\" | tail -n 1 | cut -c1-200)\"; fi"
        ))?;
        Ok(id)
    }

    pub fn send_text(&mut self, pane: &str, text: &str) -> std::io::Result<u64> {
        self.herdr(&["pane", "send-text", pane, text])
    }

    pub fn send_keys(&mut self, pane: &str, keys: &[&str]) -> std::io::Result<u64> {
        let mut a = vec!["pane", "send-keys", pane];
        a.extend_from_slice(keys);
        self.herdr(&a)
    }

    pub fn prompt(&mut self, pane: &str, text: &str) -> std::io::Result<u64> {
        self.herdr(&["agent", "prompt", pane, text])
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.as_mut().map(|c| c.try_wait()), Some(Ok(None)))
    }

    pub fn take_errors(&self) -> Vec<(u64, String)> {
        std::mem::take(&mut *self.errors.lock().unwrap())
    }

    /// A command has gone unanswered too long: the link stalled (a half-dead
    /// ssh). How many are waiting; the caller reopens the channel.
    pub fn stalled(&self) -> Option<usize> {
        let p = self.pending.lock().unwrap();
        p.values().any(|t| t.elapsed() > STALL).then_some(p.len())
    }
}

impl Drop for Chan {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// crossterm key → herdr key-combo name (`enter`, `ctrl+c`, `a`, …).
/// Returns None for keys herdr can't express.
pub fn key_name(k: &crossterm::event::KeyEvent) -> Option<String> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let base = match k.code {
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => return Some("shift+tab".into()),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char('-') => "minus".into(),
        KeyCode::Char('+') => "plus".into(),
        KeyCode::Char('`') => "backtick".into(),
        KeyCode::Char(c) => c.to_string(),
        _ => return None,
    };
    let mut m = String::new();
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        m.push_str("ctrl+");
    }
    if k.modifiers.contains(KeyModifiers::ALT) {
        m.push_str("alt+");
    }
    if k.modifiers.contains(KeyModifiers::SHIFT) && !matches!(k.code, KeyCode::Char(_)) {
        m.push_str("shift+");
    }
    Some(format!("{m}{base}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_sessions_reach_the_shell_line() {
        let cfg = crate::config::Config::in_memory(&[
            "hosts=sess-test-box,plain-test-box",
            "host.sess-test-box.session=work bench",
        ]);
        set_sessions(&cfg);
        assert_eq!(
            herdr_env("sess-test-box"),
            "export HERDR_SESSION='work bench'; "
        );
        assert_eq!(herdr_env("plain-test-box"), "");
        assert_eq!(session("sess-test-box").as_deref(), Some("work bench"));
    }
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn commands_ack_ok_and_failed() {
        // a local channel, with `true` / `false` standing in for herdr
        let mut c = Chan::open(sshcfg::LOCAL).unwrap();
        c.herdr = "true".into();
        c.herdr(&["pane", "send-text", "w1:p1", "hi"]).unwrap();
        c.herdr = "false".into();
        let bad = c.herdr(&["agent", "prompt", "w1:p1", "x"]).unwrap();
        let t0 = Instant::now();
        while !c.pending.lock().unwrap().is_empty() && t0.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(c.pending.lock().unwrap().is_empty(), "every command acked");
        assert_eq!(c.take_errors().first().map(|e| e.0), Some(bad));
        assert!(c.stalled().is_none());
    }

    #[test]
    fn key_names() {
        let k = |c, m| key_name(&KeyEvent::new(c, m)).unwrap();
        assert_eq!(k(KeyCode::Enter, KeyModifiers::NONE), "enter");
        assert_eq!(k(KeyCode::Char('c'), KeyModifiers::CONTROL), "ctrl+c");
        assert_eq!(k(KeyCode::Char('A'), KeyModifiers::SHIFT), "A");
        assert_eq!(k(KeyCode::Up, KeyModifiers::SHIFT), "shift+up");
        assert_eq!(k(KeyCode::Char('-'), KeyModifiers::NONE), "minus");
        assert_eq!(k(KeyCode::BackTab, KeyModifiers::SHIFT), "shift+tab");
    }
}
