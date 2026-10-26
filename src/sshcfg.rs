//! ssh side: aliases from ~/.ssh/config (incl. Include), effective settings via
//! `ssh -G`, non-interactive auth probes with failure diagnosis, adding a host
//! alias, and one-time key setup so nothing ever asks for a password again.

use crate::config::{expand, home};
use crate::proc::{self, Out};
use std::fs;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Options for every non-interactive ssh we run: never prompt, fail fast.
pub const BATCH: &[&str] = &[
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=8",
    "-o",
    "ConnectionAttempts=1",
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=2",
];

/// Softer keepalives for an interactive attach (a short stall shouldn't end
/// it; polls use `BATCH`'s quick ones to notice a dead machine fast).
pub const INTERACTIVE: &[&str] = &[
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=4",
];

/// ssh's words for a shared connection that died under us (the next call
/// must open a new one).
pub fn master_broken(err: &str) -> bool {
    [
        "mux_client",
        "Control socket connect",
        "master hello",
        "ControlSocket",
        "read from master failed",
    ]
    .iter()
    .any(|m| err.contains(m))
}

/// The uid that owns HOME, as text (for per-user /tmp paths).
pub fn uid_str() -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(m) = std::fs::metadata(home()) {
            return m.uid().to_string();
        }
    }
    "0".into()
}

/// BATCH plus optional connection sharing (`ssh_reuse=N` seconds): every ssh
/// we start then rides one authenticated connection per host.
/// `batch()` for an interactive session: ssh keeps the first value it
/// reads for each option, so the softer keepalives go first.
pub fn interactive() -> Vec<String> {
    let mut v: Vec<String> = INTERACTIVE.iter().map(|s| s.to_string()).collect();
    v.extend(batch());
    v
}

pub fn batch() -> Vec<String> {
    let mut v: Vec<String> = BATCH.iter().map(|s| s.to_string()).collect();
    v.extend(reuse_opts());
    v
}

pub fn reuse_opts() -> Vec<String> {
    use std::sync::OnceLock;
    static SECS: OnceLock<u64> = OnceLock::new();
    let secs = *SECS.get_or_init(|| {
        crate::brand::env("SSH_REUSE")
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| crate::config::Config::load().num("ssh_reuse"))
    });
    if secs == 0 {
        return vec![];
    }
    let Some(dir) = control_dir() else {
        return vec![];
    };
    vec![
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}/%C", dir.display()),
        "-o".into(),
        format!("ControlPersist={secs}"),
        // the shared connection carries polls and live frames (base64 JSON):
        // compression pays for itself on a slow link and costs little on a fast one
        "-o".into(),
        "Compression=yes".into(),
    ]
}

/// Where shared connections' sockets live. A unix socket path is limited to
/// ~104 bytes and ssh adds `/<40-hex %C>.<16 random>` while creating one, so
/// a long cache path (a long user name, a custom XDG_CACHE_HOME) falls back
/// to a private `/tmp/al-<uid>`; None turns sharing off.
fn control_dir() -> Option<PathBuf> {
    const ROOM: usize = 104 - 1 - 40 - 17 - 1;
    let mut dirs = vec![crate::config::cache_dir().join("cm")];
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = std::fs::metadata(home()).map(|m| m.uid()).unwrap_or(0);
        dirs.push(PathBuf::from(format!("/tmp/al-{uid}")));
    }
    for dir in dirs {
        if dir.as_os_str().len() > ROOM || std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            // someone else's /tmp dir: don't share sockets through it
            use std::os::unix::fs::MetadataExt;
            let mine = std::fs::metadata(home()).map(|m| m.uid()).ok();
            if std::fs::metadata(&dir).map(|m| m.uid()).ok() != mine {
                continue;
            }
        }
        return Some(dir);
    }
    None
}

/// Close the shared connection to `host` (`ssh -O exit`), so the next call
/// opens a fresh one: a reconnect. Quiet when there's none.
pub fn drop_master(host: &str) {
    if is_local(host) || reuse_opts().is_empty() {
        return;
    }
    let mut a = reuse_opts();
    a.extend([
        "-o".into(),
        "BatchMode=yes".into(),
        "-O".into(),
        "exit".into(),
        host.into(),
    ]);
    let _ = crate::proc::cap(&ssh_bin(), &a, std::time::Duration::from_secs(5));
}

pub fn ssh_bin() -> String {
    // showcase's demo world: nothing real is ever reached
    if let Some(dead) = crate::showcase::dead_bin() {
        return dead;
    }
    crate::brand::env("SSH_BIN").unwrap_or_else(|| "ssh".into())
}

pub fn user_config() -> PathBuf {
    crate::brand::env_os("SSH_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".ssh").join("config"))
}

fn ssh_dir() -> PathBuf {
    user_config()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home().join(".ssh"))
}

// ------------------------------------------------------------------ config parsing

/// (lower-cased keyword, rest) for a config line.
fn kv(line: &str) -> Option<(String, &str)> {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') {
        return None;
    }
    let i = l.find(|c: char| c.is_whitespace() || c == '=')?;
    let rest = l[i..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    Some((l[..i].to_ascii_lowercase(), rest))
}

/// Whitespace-separated words, honouring "double quotes".
fn words(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut q) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        match c {
            '"' => q = !q,
            c if c.is_whitespace() && !q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn wild(p: &[char], s: &[char]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some('*'), _) => wild(&p[1..], s) || (!s.is_empty() && wild(p, &s[1..])),
        (Some('?'), Some(_)) => wild(&p[1..], &s[1..]),
        (Some(a), Some(b)) if a == b => wild(&p[1..], &s[1..]),
        _ => false,
    }
}

/// Expand `*`/`?` per path component (enough for `Include config.d/*`).
fn glob(p: &Path) -> Vec<PathBuf> {
    let mut acc = vec![PathBuf::from("/")];
    for comp in p.components().skip(1) {
        let c = comp.as_os_str().to_string_lossy().to_string();
        let mut next = Vec::new();
        for base in acc {
            if c.contains(['*', '?']) {
                let pat: Vec<char> = c.chars().collect();
                if let Ok(rd) = fs::read_dir(&base) {
                    let mut names: Vec<String> = rd
                        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
                        .filter(|n| !n.starts_with('.') || c.starts_with('.'))
                        .filter(|n| wild(&pat, &n.chars().collect::<Vec<_>>()))
                        .collect();
                    names.sort();
                    next.extend(names.into_iter().map(|n| base.join(n)));
                }
            } else {
                next.push(base.join(&c));
            }
        }
        acc = next;
    }
    acc.into_iter().filter(|p| p.is_file()).collect()
}

/// ~/.ssh/config plus everything it Includes (recursively, cycle-safe).
pub fn config_files() -> Vec<PathBuf> {
    fn walk(f: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 8 || out.iter().any(|x| x == f) || !f.is_file() {
            return;
        }
        out.push(f.to_path_buf());
        let Ok(fh) = fs::File::open(f) else { return };
        for line in io::BufReader::new(fh).lines().map_while(Result::ok) {
            let Some((k, rest)) = kv(&line) else { continue };
            if k != "include" {
                continue;
            }
            for w in words(rest) {
                let p = if w.starts_with('~') {
                    expand(&w)
                } else if w.starts_with('/') {
                    PathBuf::from(&w)
                } else {
                    ssh_dir().join(&w)
                };
                for g in glob(&p) {
                    walk(&g, depth + 1, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&user_config(), 0, &mut out);
    out
}

/// Concrete Host aliases (no wildcards / negations), in file order, deduped.
pub fn aliases() -> Vec<String> {
    if let Some(v) = crate::showcase::aliases() {
        return v;
    }
    let mut out: Vec<String> = Vec::new();
    for f in config_files() {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        for line in text.lines() {
            let Some((k, rest)) = kv(line) else { continue };
            if k != "host" {
                continue;
            }
            for w in words(rest) {
                if !w.contains(['*', '?', '!']) && !out.contains(&w) {
                    out.push(w);
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ ssh -G

#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub hostname: String,
    pub user: String,
    pub port: String,
    pub identities: Vec<String>,
    pub proxy: Option<String>,
}

impl Resolved {
    pub fn short(&self) -> String {
        if self.port.is_empty() && self.hostname == "this computer" {
            return self.hostname.clone();
        }
        let port = if self.port == "22" || self.port.is_empty() {
            String::new()
        } else {
            format!(":{}", self.port)
        };
        format!("{}@{}{}", self.user, self.hostname, port)
    }
}

/// The pseudo-machine for this computer's own herdr: no ssh, no `--machine`.
pub const LOCAL: &str = "local";

pub fn is_local(host: &str) -> bool {
    host == LOCAL
}

/// Run `remote_cmd` on `host`: `ssh -T <batch> host remote_cmd`, or for
/// `local` the same command through `sh -c` right here.
pub fn shell(host: &str, remote_cmd: &str) -> Command {
    // the demo world: not even this computer's own shell
    if let Some(dead) = crate::showcase::dead_bin() {
        return Command::new(dead);
    }
    if is_local(host) {
        let mut c = Command::new("sh");
        c.arg("-c").arg(remote_cmd);
        c
    } else {
        let mut c = Command::new(ssh_bin());
        c.arg("-T").args(batch()).arg(host).arg(remote_cmd);
        c
    }
}

/// Effective settings for `host` (no network).
pub fn resolve(host: &str) -> Option<Resolved> {
    if let Some(r) = crate::showcase::resolve(host) {
        return r;
    }
    if is_local(host) {
        return Some(Resolved {
            hostname: "this computer".into(),
            user: std::env::var("USER").unwrap_or_default(),
            port: String::new(),
            ..Default::default()
        });
    }
    let o = proc::cap(&ssh_bin(), &["-G", host], Duration::from_secs(5));
    if !o.ok() {
        return None;
    }
    let mut r = Resolved::default();
    for l in o.text().lines() {
        let Some((k, v)) = l.split_once(' ') else {
            continue;
        };
        match k {
            "hostname" => r.hostname = v.to_string(),
            "user" => r.user = v.to_string(),
            "port" => r.port = v.to_string(),
            "identityfile" => r.identities.push(v.to_string()),
            "proxyjump" | "proxycommand" if v != "none" => r.proxy = Some(format!("{k} {v}")),
            _ => {}
        }
    }
    Some(r)
}

// ------------------------------------------------------------------ probes

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fail {
    Dns,
    Unreachable,
    Refused,
    HostKeyUnknown,
    HostKeyChanged,
    Auth,
    Timeout,
    Other,
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub fail: Option<Fail>,
    pub detail: String,
}

impl Probe {
    pub fn ok(&self) -> bool {
        self.fail.is_none()
    }
    /// One-line explanation for humans.
    pub fn explain(&self) -> String {
        match self.fail {
            None => "key login works (no password needed)".into(),
            Some(Fail::Dns) => "hostname doesn't resolve".into(),
            Some(Fail::Unreachable) => "host unreachable (network/VPN/firewall)".into(),
            Some(Fail::Refused) => "connection refused (sshd not listening on that port)".into(),
            Some(Fail::HostKeyUnknown) => "host key not trusted yet".into(),
            Some(Fail::HostKeyChanged) => "host key CHANGED since last time".into(),
            Some(Fail::Auth) => "no key accepted — would need a password".into(),
            Some(Fail::Timeout) => "timed out".into(),
            Some(Fail::Other) => self.detail.clone(),
        }
    }
}

pub fn classify(o: &Out) -> Option<Fail> {
    if o.ok() {
        return None;
    }
    if o.timed_out {
        return Some(Fail::Timeout);
    }
    let e = &o.stderr;
    let has = |s: &str| e.contains(s);
    Some(
        if has("Could not resolve hostname")
            || has("nodename nor servname")
            || has("Name or service not known")
        {
            Fail::Dns
        } else if has("REMOTE HOST IDENTIFICATION HAS CHANGED") {
            Fail::HostKeyChanged
        } else if has("Host key verification failed") || has("host key is known") {
            Fail::HostKeyUnknown
        } else if has("Connection refused") {
            Fail::Refused
        } else if has("timed out")
            || has("No route to host")
            || has("Network is unreachable")
            || has("Host is down")
        {
            Fail::Unreachable
        } else if has("Permission denied") || has("Too many authentication failures") {
            Fail::Auth
        } else {
            Fail::Other
        },
    )
}

/// Can we log in without any prompt? (what herdr --machine and `aleph -i` need)
pub fn probe(host: &str) -> Probe {
    if is_local(host) {
        return Probe {
            fail: None,
            detail: String::new(),
        };
    }
    let o = proc::run(
        Command::new(ssh_bin()).args(batch()).arg(host).arg("true"),
        Duration::from_secs(20),
        None,
    )
    .unwrap_or_else(|e| Out {
        stderr: e.to_string(),
        ..Default::default()
    });
    Probe {
        fail: classify(&o),
        detail: o.reason(),
    }
}

/// Auth methods the server offers (e.g. ["publickey","password"]), or why we couldn't ask.
pub fn methods(host: &str) -> Result<Vec<String>, Probe> {
    let o = proc::run(
        Command::new(ssh_bin())
            .args(batch())
            .args([
                "-o",
                "PreferredAuthentications=none",
                "-o",
                "PubkeyAuthentication=no",
            ])
            .arg(host)
            .arg("true"),
        Duration::from_secs(20),
        None,
    )
    .unwrap_or_else(|e| Out {
        stderr: e.to_string(),
        ..Default::default()
    });
    if let Some(i) = o.stderr.find("Permission denied (") {
        let rest = &o.stderr[i + 19..];
        let list = &rest[..rest.find(')').unwrap_or(rest.len())];
        return Ok(list
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect());
    }
    if o.ok() {
        return Ok(vec!["none".into()]);
    }
    Err(Probe {
        fail: classify(&o),
        detail: o.reason(),
    })
}

// ------------------------------------------------------------------ keys

/// Private key files that exist, from `ssh -G` identityfile entries.
pub fn identities(r: &Resolved) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = r
        .identities
        .iter()
        .map(|i| expand(i))
        .filter(|p| p.is_file())
        .collect();
    v.dedup();
    v
}

pub enum Agent {
    Keys(usize),
    Empty,
    None,
}

pub fn agent() -> Agent {
    let o = proc::cap("ssh-add", &["-l"], Duration::from_secs(5));
    match o.code {
        Some(0) => Agent::Keys(o.text().lines().filter(|l| !l.trim().is_empty()).count()),
        Some(1) => Agent::Empty,
        _ => Agent::None,
    }
}

/// Key needs a passphrase (so without an agent, every connection would prompt).
pub fn encrypted(key: &Path) -> bool {
    let o = proc::cap(
        "ssh-keygen",
        &[
            std::ffi::OsStr::new("-y"),
            "-P".as_ref(),
            "".as_ref(),
            "-f".as_ref(),
            key.as_os_str(),
        ],
        Duration::from_secs(5),
    );
    // only "ran and refused the empty passphrase" counts (missing ssh-keygen ≠ encrypted)
    o.code.is_some() && !o.ok()
}

/// ssh refuses private keys readable by group/others.
pub fn loose_perms(key: &Path) -> bool {
    key.metadata()
        .map(|m| m.permissions().mode() & 0o077 != 0)
        .unwrap_or(false)
}

// ------------------------------------------------------------------ adding a host

#[derive(Clone, Debug, PartialEq)]
pub struct NewHost {
    pub alias: String,
    pub user: Option<String>,
    pub hostname: String,
    pub port: Option<u16>,
    pub identity: Option<String>,
    /// reach it through another machine (`ProxyJump`), e.g. a bastion
    pub jump: Option<String>,
}

impl NewHost {
    /// The `Host` block for ~/.ssh/config.
    pub fn block(&self) -> String {
        let mut b = format!(
            "# added by aleph\nHost {}\n    HostName {}\n",
            self.alias, self.hostname
        );
        if let Some(u) = &self.user {
            b.push_str(&format!("    User {u}\n"));
        }
        if let Some(p) = self.port {
            b.push_str(&format!("    Port {p}\n"));
        }
        if let Some(i) = &self.identity {
            b.push_str(&format!("    IdentityFile {i}\n    IdentitiesOnly yes\n"));
        }
        if let Some(j) = &self.jump {
            b.push_str(&format!("    ProxyJump {j}\n"));
        }
        b.push_str("    ServerAliveInterval 15\n    ServerAliveCountMax 4\n");
        b
    }
}

/// What trying a host that isn't saved yet found.
#[derive(Clone, Debug)]
pub enum Precheck {
    /// logged in without a prompt
    Ok(Duration),
    /// first contact: the server's key fingerprints (`ED25519 SHA256:…`) and
    /// the known_hosts lines that would trust it
    NewKey {
        fingerprints: Vec<String>,
        lines: Vec<String>,
    },
    Failed(Fail, String),
}

fn expand_tilde(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest).to_string_lossy().into_owned(),
        None => p.to_string(),
    }
}

/// Try `h` before saving it: one BatchMode login (never a password prompt),
/// strict about host keys — an unknown key is shown to you, not accepted.
pub fn precheck(h: &NewHost) -> Precheck {
    let target = match &h.user {
        Some(u) => format!("{u}@{}", h.hostname),
        None => h.hostname.clone(),
    };
    let mut c = Command::new(ssh_bin());
    c.args(BATCH).args(["-o", "StrictHostKeyChecking=yes"]);
    if let Some(p) = h.port {
        c.args(["-p", &p.to_string()]);
    }
    if let Some(i) = &h.identity {
        c.args(["-i", &expand_tilde(i), "-o", "IdentitiesOnly=yes"]);
    }
    c.arg(&target).arg("true");
    let t0 = std::time::Instant::now();
    let o = match proc::run(&mut c, Duration::from_secs(20), None) {
        Ok(o) => o,
        Err(e) => return Precheck::Failed(Fail::Other, format!("can't run ssh: {e}")),
    };
    match classify(&o) {
        None => Precheck::Ok(t0.elapsed()),
        Some(Fail::HostKeyUnknown) => {
            let (fingerprints, lines) = host_keys(&h.hostname, h.port);
            if lines.is_empty() {
                Precheck::Failed(
                    Fail::HostKeyUnknown,
                    "its host key is unknown, and asking for it didn't work".into(),
                )
            } else {
                Precheck::NewKey {
                    fingerprints,
                    lines,
                }
            }
        }
        Some(f) => Precheck::Failed(f, o.reason()),
    }
}

/// The server's host keys (`ssh-keyscan`) and their fingerprints.
pub fn host_keys(hostname: &str, port: Option<u16>) -> (Vec<String>, Vec<String>) {
    let mut c = Command::new("ssh-keyscan");
    c.args(["-T", "6"]);
    if let Some(p) = port {
        c.args(["-p", &p.to_string()]);
    }
    c.arg(hostname);
    let Ok(o) = proc::run(&mut c, Duration::from_secs(10), None) else {
        return (vec![], vec![]);
    };
    let lines: Vec<String> = o
        .text()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        return (vec![], vec![]);
    }
    let input = lines.join("\n") + "\n";
    let fps = proc::run(
        Command::new("ssh-keygen").args(["-l", "-f", "-"]),
        Duration::from_secs(5),
        Some(input.as_bytes()),
    )
    .map(|o| {
        o.text()
            .lines()
            .filter_map(|l| {
                // "256 SHA256:abc… host (ED25519)"
                let mut w = l.split_whitespace();
                let _bits = w.next()?;
                let fp = w.next()?;
                let kind = l.rsplit_once('(').map(|x| x.1.trim_end_matches(')'))?;
                Some(format!("{kind}  {fp}"))
            })
            .collect()
    })
    .unwrap_or_default();
    (fps, lines)
}

/// Trust keys the user has seen: append them to ~/.ssh/known_hosts.
pub fn trust(lines: &[String]) -> Result<(), String> {
    let dir = home().join(".ssh");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("known_hosts");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    for l in lines {
        writeln!(f, "{l}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn valid_alias(a: &str) -> Result<(), String> {
    if a.is_empty() {
        return Err("alias is empty".into());
    }
    if !a
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        || a.starts_with('-')
    {
        return Err(format!("alias '{a}': use letters, digits, '.', '_' or '-'"));
    }
    Ok(())
}

/// `[user@]host[:port]`, IPv6 as `[::1]:22`.
pub fn parse_target(t: &str) -> Result<(Option<String>, String, Option<u16>), String> {
    let t = t.trim();
    let (user, rest) = match t.rsplit_once('@') {
        Some((u, r)) if !u.is_empty() => (Some(u.to_string()), r),
        Some(_) => return Err("empty user before '@'".into()),
        None => (None, t),
    };
    let (host, port) = if let Some(r) = rest.strip_prefix('[') {
        let (h, after) = r.split_once(']').ok_or("missing ']' in IPv6 address")?;
        (h.to_string(), after.strip_prefix(':'))
    } else if rest.matches(':').count() == 1 {
        let (h, p) = rest.split_once(':').unwrap();
        (h.to_string(), Some(p))
    } else {
        (rest.to_string(), None)
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err("hostname is empty or has spaces".into());
    }
    let port = match port {
        Some(p) => Some(
            p.parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or(format!("bad port '{p}'"))?,
        ),
        None => None,
    };
    Ok((user, host, port))
}

/// Replace a `Host <alias>` block in ~/.ssh/config with `h`'s (backup first).
/// Only blocks in the main file are edited; one that lives in an included
/// file is left alone with an error that says where to change it.
pub fn update_host(h: &NewHost) -> Result<Option<PathBuf>, String> {
    valid_alias(&h.alias)?;
    let cfg = user_config();
    let text = fs::read_to_string(&cfg).unwrap_or_default();
    let out = match replace_block(&text, h)? {
        Some(t) => t,
        None => {
            if aliases().contains(&h.alias) {
                return Err(format!(
                    "{} is defined in a file ~/.ssh/config includes — change it there",
                    h.alias
                ));
            }
            return add_host(h);
        }
    };
    let b = cfg.with_file_name(format!("config.aleph.bak.{}", crate::cache::now()));
    fs::copy(&cfg, &b).map_err(|e| format!("backup failed: {e}"))?;
    crate::config::write_private(&cfg, out.as_bytes()).map_err(|e| e.to_string())?;
    Ok(Some(b))
}

/// Pure (tested): `text` with `h.alias`'s Host block replaced by `h`'s.
/// None = the alias has no block of its own in `text`.
pub fn replace_block(text: &str, h: &NewHost) -> Result<Option<String>, String> {
    let lines: Vec<&str> = text.lines().collect();
    let is_header = |l: &str| {
        let t = l.trim_start().to_lowercase();
        t.starts_with("host ") || t.starts_with("host\t") || t.starts_with("match ")
    };
    let names = |l: &str| -> Vec<String> {
        let t = l.trim();
        if t.len() > 4 && t[..4].eq_ignore_ascii_case("host") && t[4..].starts_with([' ', '\t']) {
            t[4..].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        }
    };
    let Some(start) = lines.iter().position(|l| names(l).contains(&h.alias)) else {
        return Ok(None);
    };
    if names(lines[start]).len() > 1 {
        return Err(format!(
            "{} shares its Host line with other aliases — edit ~/.ssh/config by hand",
            h.alias
        ));
    }
    let end = lines[start + 1..]
        .iter()
        .position(|l| is_header(l))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    // our own comment line above the block goes too (we write a new one)
    let from = if start > 0 && lines[start - 1].trim() == "# added by aleph" {
        start - 1
    } else {
        start
    };
    let mut out = String::new();
    for l in &lines[..from] {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&h.block());
    if end < lines.len() {
        out.push('\n');
        for l in &lines[end..] {
            out.push_str(l);
            out.push('\n');
        }
    }
    Ok(Some(out))
}

/// A private key in ~/.ssh: where it is, its type and comment (read off its
/// `.pub`; empty when there's none).
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub path: PathBuf,
    pub kind: String,
    pub comment: String,
}

impl Key {
    /// `github_work · ed25519`
    pub fn label(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.kind.is_empty() {
            name
        } else {
            format!("{name} · {}", self.kind)
        }
    }
}

/// `ssh-ed25519 AAAA… me@x` → (`ed25519`, `me@x`).
fn pub_kind(line: &str) -> (String, String) {
    let mut w = line.split_whitespace();
    let t = w.next().unwrap_or("");
    let kind = match t {
        "ssh-ed25519" => "ed25519".to_string(),
        "ssh-rsa" => "rsa".into(),
        "ssh-dss" => "dsa".into(),
        t if t.starts_with("ecdsa-") => "ecdsa".into(),
        t if t.starts_with("sk-ssh-ed25519") => "ed25519-sk".into(),
        t if t.starts_with("sk-ecdsa") => "ecdsa-sk".into(),
        t => t.to_string(),
    };
    let _ = w.next();
    (kind, w.collect::<Vec<_>>().join(" "))
}

/// The private keys in `dir`: a file with a `.pub` beside it, or one whose
/// first line says it's a private key (only that line is read — never the
/// key). Config, known_hosts and the like aren't keys.
pub fn keys_in(dir: &Path) -> Vec<Key> {
    const NOT: &[&str] = &[
        "config",
        "known_hosts",
        "authorized_keys",
        "authorized_keys2",
        "environment",
        "rc",
    ];
    let Ok(rd) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<Key> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !p.is_file()
            || name.starts_with('.')
            || name.ends_with(".pub")
            || NOT
                .iter()
                .any(|n| name == *n || name.starts_with(&format!("{n}.")))
        {
            continue;
        }
        let pubf = p.with_file_name(format!("{name}.pub"));
        let private = pubf.is_file() || {
            use std::io::{BufRead, BufReader};
            fs::File::open(&p)
                .ok()
                .and_then(|f| BufReader::new(f).lines().next())
                .and_then(Result::ok)
                .is_some_and(|l| l.starts_with("-----BEGIN") && l.contains("PRIVATE KEY"))
        };
        if !private {
            continue;
        }
        let (kind, comment) = fs::read_to_string(&pubf)
            .map(|t| pub_kind(t.lines().next().unwrap_or("")))
            .unwrap_or_default();
        v.push(Key {
            path: p,
            kind,
            comment,
        });
    }
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

/// Does this key have a passphrase? `ssh-keygen -y -P ""` reads it with an
/// empty one (local, changes nothing): it works when there's none. None when
/// ssh-keygen couldn't tell.
pub fn needs_passphrase(key: &Path) -> Option<bool> {
    let o = proc::run(
        Command::new("ssh-keygen")
            .args(["-y", "-P", ""])
            .arg("-f")
            .arg(key),
        Duration::from_secs(5),
        None,
    )
    .ok()?;
    if o.ok() {
        return Some(false);
    }
    let e = o.stderr.to_lowercase();
    (e.contains("passphrase") || e.contains("incorrect")).then_some(true)
}

/// The private keys in ~/.ssh.
pub fn keys() -> Vec<Key> {
    keys_in(&ssh_dir())
}

/// Where a new key goes: ~/.ssh/id_ed25519 while that's free, else
/// ~/.ssh/aleph_ed25519 (then _2, _3, …) — an existing key is never touched.
pub fn new_key_path() -> PathBuf {
    let d = ssh_dir();
    let free = |p: &Path| {
        !p.exists()
            && !p
                .with_file_name(format!(
                    "{}.pub",
                    p.file_name().unwrap_or_default().to_string_lossy()
                ))
                .exists()
    };
    let first = d.join("id_ed25519");
    if free(&first) {
        return first;
    }
    let mut n = 1;
    loop {
        let p = if n == 1 {
            d.join("aleph_ed25519")
        } else {
            d.join(format!("aleph_ed25519_{n}"))
        };
        if free(&p) {
            return p;
        }
        n += 1;
    }
}

/// `ssh_key`: the key Aleph uses, when it's set and there.
pub fn chosen_key() -> Option<PathBuf> {
    let v = crate::config::Config::load().get("ssh_key")?;
    let p = PathBuf::from(expand_tilde(v.trim()));
    (!v.trim().is_empty() && p.is_file()).then_some(p)
}

/// Make ssh offer `key` to `alias`: a `Host` block with its `IdentityFile`
/// appended to ~/.ssh/config (backup first). IdentityFile adds up across
/// blocks, so the alias's own settings stay as they were.
pub fn add_identity(alias: &str, key: &Path) -> Result<Option<PathBuf>, String> {
    valid_alias(alias)?;
    let cfg = user_config();
    let backup = if cfg.exists() {
        let b = cfg.with_file_name(format!("config.aleph.bak.{}", crate::cache::now()));
        fs::copy(&cfg, &b).map_err(|e| format!("backup failed: {e}"))?;
        Some(b)
    } else {
        None
    };
    let old = fs::read(&cfg).unwrap_or_default();
    let mut block = String::new();
    if !old.is_empty() && !old.ends_with(b"\n") {
        block.push('\n');
    }
    block.push_str(&format!(
        "\n# added by aleph: the key for {alias}\nHost {alias}\n    IdentityFile {}\n",
        crate::doctor::tilde(key)
    ));
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&cfg)
        .map_err(|e| e.to_string())?;
    f.write_all(block.as_bytes()).map_err(|e| e.to_string())?;
    Ok(backup)
}

/// Append a `Host` block to ~/.ssh/config (backup first). Returns the backup path.
pub fn add_host(h: &NewHost) -> Result<Option<PathBuf>, String> {
    valid_alias(&h.alias)?;
    if aliases().contains(&h.alias) {
        return Err(format!("'{}' already exists in your ssh config", h.alias));
    }
    let cfg = user_config();
    let dir = ssh_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    let backup = if cfg.exists() {
        let b = cfg.with_file_name(format!("config.aleph.bak.{}", crate::cache::now()));
        fs::copy(&cfg, &b).map_err(|e| format!("backup failed: {e}"))?;
        Some(b)
    } else {
        None
    };
    let old = fs::read(&cfg).unwrap_or_default();
    let mut block = String::new();
    if !old.is_empty() && !old.ends_with(b"\n") {
        block.push('\n');
    }
    block.push('\n');
    block.push_str(&h.block());
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&cfg)
        .map_err(|e| e.to_string())?;
    f.write_all(block.as_bytes()).map_err(|e| e.to_string())?;
    Ok(backup)
}

// ------------------------------------------------------------------ key setup (interactive)

// styled wizard output (same language as the rest of the CLI)
fn line(spans: Vec<ratatui::text::Span<'static>>) {
    use std::io::IsTerminal;
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    println!(
        "{}",
        crate::ui::to_ansi(&ratatui::text::Line::from(spans), color)
    );
}
fn step(text: String) {
    use crate::ui::{self, c};
    line(vec![
        ratatui::text::Span::styled("⏺ ", ui::fg(c::ACCENT())),
        ratatui::text::Span::styled(text, ui::bold()),
    ]);
}
fn done(text: String) {
    use crate::ui::{self, c};
    line(vec![
        ratatui::text::Span::styled(
            "✓ ",
            ui::fg(c::GREEN()).add_modifier(ratatui::style::Modifier::BOLD),
        ),
        ratatui::text::Span::raw(text),
    ]);
}
fn note(word: &'static str, col: ratatui::style::Color, text: String) {
    use crate::ui;
    line(vec![
        ratatui::text::Span::styled(
            format!("{word}: "),
            ui::fg(col).add_modifier(ratatui::style::Modifier::BOLD),
        ),
        ratatui::text::Span::raw(text),
    ]);
}

fn ask(q: &str, default_yes: bool) -> bool {
    use crate::ui::{self, c};
    use std::io::IsTerminal;
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let l = ratatui::text::Line::from(vec![
        ratatui::text::Span::styled(
            "? ",
            ui::fg(c::GREEN()).add_modifier(ratatui::style::Modifier::BOLD),
        ),
        ratatui::text::Span::styled(q.to_string(), ui::bold()),
        ratatui::text::Span::styled(if default_yes { " (Y/n)" } else { " (y/N)" }, ui::muted()),
        ratatui::text::Span::styled(" › ", ui::faint()),
    ]);
    print!("{}", ui::to_ansi(&l, color));
    let _ = io::stdout().flush();
    let mut s = String::new();
    if io::stdin().read_line(&mut s).is_err() {
        return false;
    }
    match s.trim().to_lowercase().as_str() {
        "" => default_yes,
        a => a.starts_with('y'),
    }
}

/// One-time: make sure `host` accepts a key (copying it with a single password prompt).
pub fn setup_key(host: &str) -> Result<(), String> {
    if let Some(e) = crate::showcase::refused() {
        return Err(e);
    }
    let r = resolve(host).ok_or_else(|| format!("ssh can't parse settings for '{host}'"))?;
    step(format!("Key login for {host}  ·  {}", r.short()));
    let p = probe(host);
    if p.ok() {
        done("Already works — nothing to do".into());
        return Ok(());
    }
    match p.fail {
        Some(Fail::Auth) => {}
        Some(Fail::HostKeyUnknown) => {
            note(
                "warning",
                crate::ui::c::YELLOW(),
                format!(
                    "{host}'s host key isn't trusted yet — connecting once so you can verify it"
                ),
            );
            let _ = proc::interactive(&ssh_bin(), &["-o", "BatchMode=no", host, "true"]);
            if probe(host).ok() {
                done("Key login works now".into());
                return Ok(());
            }
        }
        _ => {
            return Err(format!(
                "can't reach {host}: {} — fix that first (aleph doctor)",
                p.explain()
            ))
        }
    }
    if let Ok(m) = methods(host) {
        if !m.iter().any(|x| x == "publickey") {
            return Err(format!(
                "{host} only allows: {} — key login is disabled on the server.\n  \
                 Ask the admin to enable PubkeyAuthentication (sshd_config), or reach it via a jump host that allows keys.",
                m.join(", ")
            ));
        }
    }
    // pick or create a key: yours (ssh_key) first, else what ssh offers
    let offered = identities(&r);
    let mut key = chosen_key().or_else(|| {
        offered
            .iter()
            .find(|k| k.with_extension("pub").is_file() || pubkey(k).is_file())
            .cloned()
    });
    if key.is_none() {
        let k = ssh_dir().join("id_ed25519");
        if k.exists() {
            key = Some(k);
        } else {
            if !ask(
                &format!("No SSH key found. Create {} now?", k.display()),
                true,
            ) {
                return Err("cancelled".into());
            }
            let comment = format!("aleph {}", std::env::var("USER").unwrap_or_default());
            if !proc::interactive(
                "ssh-keygen",
                &[
                    "-t".as_ref(),
                    "ed25519".as_ref(),
                    "-f".as_ref(),
                    k.as_os_str(),
                    "-C".as_ref(),
                    comment.as_ref(),
                ],
            )
            .map_err(|e| e.to_string())?
            {
                return Err("ssh-keygen failed".into());
            }
            key = Some(k);
        }
    }
    let key = key.unwrap();
    if loose_perms(&key) {
        let _ = fs::set_permissions(&key, fs::Permissions::from_mode(0o600));
        done(format!(
            "Fixed permissions on {} (600)",
            crate::doctor::tilde(&key)
        ));
    }
    let pubk = pubkey(&key);
    if !ask(
        &format!(
            "Copy {} to {host}? You'll type {}'s password once.",
            crate::doctor::tilde(&pubk),
            r.user
        ),
        true,
    ) {
        return Err("cancelled".into());
    }
    let ok = if proc::which("ssh-copy-id").is_some() {
        proc::interactive(
            "ssh-copy-id",
            &["-i".as_ref(), pubk.as_os_str(), host.as_ref()],
        )
        .map_err(|e| e.to_string())?
    } else {
        let f = fs::File::open(&pubk).map_err(|e| e.to_string())?;
        Command::new(ssh_bin())
            .arg(host)
            .arg("sh -c 'umask 077; mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys'")
            .stdin(f)
            .status()
            .map(|s| s.success())
            .map_err(|e| e.to_string())?
    };
    if !ok {
        return Err("copying the key failed".into());
    }
    // ssh only offers the keys its config (or the agent) names: tell it
    if !offered.iter().any(|k| k == &key) {
        match add_identity(host, &key) {
            Ok(_) => done(format!(
                "~/.ssh/config: {host} offers {} (a backup was kept)",
                crate::doctor::tilde(&key)
            )),
            Err(e) => note(
                "warning",
                crate::ui::c::YELLOW(),
                format!("couldn't add IdentityFile for {host}: {e}"),
            ),
        }
    }
    let mut p = probe(host);
    if !p.ok() && encrypted(&key) {
        step(
            "Your key has a passphrase — loading it into the ssh agent so you're not asked again"
                .into(),
        );
        let mac = cfg!(target_os = "macos");
        let mut args: Vec<&std::ffi::OsStr> = Vec::new();
        if mac {
            args.push("--apple-use-keychain".as_ref());
        }
        args.push(key.as_os_str());
        let _ = proc::interactive("ssh-add", &args);
        if mac {
            note("hint", crate::ui::c::BLUE(), "add `UseKeychain yes` and `AddKeysToAgent yes` under `Host *` in ~/.ssh/config to survive reboots".into());
        }
        p = probe(host);
    }
    if p.ok() {
        done(format!("Key login works — no more passwords for {host}"));
        Ok(())
    } else {
        Err(format!(
            "still can't log in without a password: {}",
            p.explain()
        ))
    }
}

fn pubkey(k: &Path) -> PathBuf {
    let mut s = k.as_os_str().to_owned();
    s.push(".pub");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_key_says_whether_it_has_a_passphrase() {
        let d = std::env::temp_dir().join(format!("aleph-pp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let mk = |n: &str, pp: &str| {
            proc::cap(
                "ssh-keygen",
                &[
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    pp,
                    "-f",
                    &d.join(n).to_string_lossy(),
                ],
                Duration::from_secs(20),
            )
        };
        if !mk("open", "").ok() || !mk("locked", "s3cret-pass").ok() {
            return; // no ssh-keygen here
        }
        assert_eq!(needs_passphrase(&d.join("open")), Some(false));
        assert_eq!(needs_passphrase(&d.join("locked")), Some(true));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn every_private_key_in_the_folder_is_found_whatever_its_name() {
        let d = std::env::temp_dir().join(format!("aleph-keys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let w = |n: &str, t: &str| fs::write(d.join(n), t).unwrap();
        w("github_work", "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n");
        w("github_work.pub", "ssh-ed25519 AAAAC3 me@laptop\n");
        w("work", "-----BEGIN RSA PRIVATE KEY-----\nx\n");
        w("lab.pub", "ssh-rsa AAAAB3 lab key\n");
        w("lab", "binary-ish");
        w("config", "Host x\n");
        w("known_hosts", "x ssh-ed25519 AAAA\n");
        w("known_hosts.old", "");
        w("authorized_keys", "ssh-ed25519 AAAA\n");
        w("notes.txt", "hello\n");
        let ks = keys_in(&d);
        let names: Vec<String> = ks.iter().map(|k| k.label()).collect();
        assert_eq!(names, vec!["github_work · ed25519", "lab · rsa", "work"]);
        assert_eq!(ks[0].comment, "me@laptop");
        assert_eq!(pub_kind("ecdsa-sha2-nistp256 AAAA x").0, "ecdsa");
        assert_eq!(pub_kind("sk-ssh-ed25519@openssh.com AAAA").0, "ed25519-sk");
        let _ = fs::remove_dir_all(&d);
    }
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(
            parse_target("alex@10.0.0.49:2222").unwrap(),
            (Some("alex".into()), "10.0.0.49".into(), Some(2222))
        );
        assert_eq!(
            parse_target("gpu2.lan").unwrap(),
            (None, "gpu2.lan".into(), None)
        );
        assert_eq!(
            parse_target("u@[fe80::1]:22").unwrap(),
            (Some("u".into()), "fe80::1".into(), Some(22))
        );
        assert_eq!(
            parse_target("fe80::1").unwrap(),
            (None, "fe80::1".into(), None)
        );
        assert!(
            parse_target("u@h:99999").is_err()
                && parse_target("@h").is_err()
                && parse_target("").is_err()
        );
        assert!(
            valid_alias("b31").is_ok()
                && valid_alias("a 49").is_err()
                && valid_alias("-x").is_err()
        );
    }

    #[test]
    fn config_words_and_wildcards() {
        assert_eq!(kv("  Host=b31 gpu2").unwrap(), ("host".into(), "b31 gpu2"));
        assert_eq!(kv("\tHOST\tb31").unwrap().0, "host");
        assert!(kv("# Host x").is_none());
        assert_eq!(words(r#"a "b c" d"#), vec!["a", "b c", "d"]);
        let w = |p: &str, s: &str| {
            wild(
                &p.chars().collect::<Vec<_>>(),
                &s.chars().collect::<Vec<_>>(),
            )
        };
        assert!(
            w("*", "x.conf") && w("*.conf", "x.conf") && !w("*.conf", "x.cfg") && w("h?st", "host")
        );
    }

    #[test]
    fn classify_errors() {
        let o = |e: &str| Out {
            code: Some(255),
            stderr: e.into(),
            ..Default::default()
        };
        assert_eq!(
            classify(&o(
                "ssh: Could not resolve hostname x: nodename nor servname provided"
            )),
            Some(Fail::Dns)
        );
        assert_eq!(
            classify(&o("u@h: Permission denied (publickey,password).")),
            Some(Fail::Auth)
        );
        assert_eq!(
            classify(&o("Host key verification failed.")),
            Some(Fail::HostKeyUnknown)
        );
        assert_eq!(
            classify(&o("ssh: connect to host h port 22: Connection refused")),
            Some(Fail::Refused)
        );
        assert_eq!(
            classify(&o("ssh: connect to host h port 22: Operation timed out")),
            Some(Fail::Unreachable)
        );
        assert_eq!(
            classify(&Out {
                code: Some(0),
                ..Default::default()
            }),
            None
        );
    }

    #[test]
    fn host_blocks_are_replaced_in_place() {
        let text = "Host a\n    HostName 1.1.1.1\n\n# added by aleph\nHost gpu2\n    HostName old\n    User x\n\nHost z\n    HostName z.lan\n";
        let h = NewHost {
            alias: "gpu2".into(),
            user: Some("me".into()),
            hostname: "new.lan".into(),
            port: Some(2222),
            identity: None,
            jump: Some("bastion".into()),
        };
        let out = replace_block(text, &h).unwrap().unwrap();
        assert!(out.starts_with("Host a\n    HostName 1.1.1.1\n\n# added by aleph\nHost gpu2\n    HostName new.lan\n    User me\n    Port 2222\n    ProxyJump bastion\n"), "{out}");
        assert!(out.ends_with("Host z\n    HostName z.lan\n"), "{out}");
        assert!(!out.contains("HostName old"));
        assert!(replace_block("Host other\n", &h).unwrap().is_none());
        assert!(replace_block("Host gpu2 gpu3\n", &h).is_err());
    }
}
