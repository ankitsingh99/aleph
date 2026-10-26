//! Background services Aleph installs for you: the daemon (`aleph daemon
//! install`) and the Cloudflare tunnel `aleph daemon expose cloudflare` sets
//! up. Both get the same shape:
//!
//! * macOS: a launchd agent, `~/Library/LaunchAgents/<label>.plist`, loaded
//!   into `gui/<uid>` (`launchctl bootstrap` / `bootout` / `kickstart`).
//!   KeepAlive, so launchd starts it again whenever it stops; its output goes
//!   to `~/Library/Logs`.
//! * Linux: a systemd user unit, `~/.config/systemd/user/<unit>`
//!   (`systemctl --user`), `Restart=on-failure`, its output in the journal.
//!   Without linger a user's services stop when they log out: that's checked
//!   and said (`loginctl enable-linger`), never changed for you.
//! * `ALEPH_INIT=launchd|systemd` picks the manager by hand (tests, odd setups).
//!
//! The files, the commands and the parsing of what the managers answer are
//! pure (tested). [`sys`] is the one place a manager runs, and never from a
//! unit test.

use crate::config::home;
use crate::doctor::tilde;
use crate::out::*;
use crate::proc::{self, sh_quote, Out};
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const LABEL: &str = crate::brand::LABEL;
pub const UNIT: &str = crate::brand::UNIT;
const T_CTL: Duration = Duration::from_secs(20);

/// Which service manager this computer has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Init {
    Launchd,
    Systemd,
    Unsupported,
}

impl Init {
    pub fn here() -> Init {
        Init::pick(crate::brand::env("INIT").as_deref(), std::env::consts::OS)
    }

    /// (pure) `over`: `ALEPH_INIT`; `os`: `std::env::consts::OS`.
    pub fn pick(over: Option<&str>, os: &str) -> Init {
        match over.map(str::trim) {
            Some("launchd") => return Init::Launchd,
            Some("systemd") => return Init::Systemd,
            _ => {}
        }
        match os {
            "macos" => Init::Launchd,
            "linux" => Init::Systemd,
            _ => Init::Unsupported,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Init::Launchd => "launchd",
            Init::Systemd => "systemd --user",
            Init::Unsupported => "none",
        }
    }
}

/// One service: what runs, with which environment, where its output goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// launchd label (`dev.aleph.daemon`)
    pub label: String,
    /// systemd unit (`aleph.service`)
    pub unit: String,
    pub description: String,
    /// the program, then its arguments
    pub args: Vec<String>,
    pub path: String,
    pub env: Vec<(String, String)>,
    /// launchd: where stdout and stderr go (systemd: the journal)
    pub log: PathBuf,
    /// the command that manages it, for the comment at the top of the file
    pub by: String,
}

impl Spec {
    /// The daemon: `aleph daemon run`, from this binary, at login.
    pub fn daemon(exe: &Path) -> Spec {
        Spec {
            label: LABEL.into(),
            unit: UNIT.into(),
            description:
                "Aleph daemon: agent sessions on your machines (notifications, automation, the API)"
                    .into(),
            args: vec![
                exe.to_string_lossy().into_owned(),
                "daemon".into(),
                "run".into(),
            ],
            path: service_path(&std::env::var("PATH").unwrap_or_default()),
            env: kept_env(),
            log: daemon_log(),
            by: "aleph daemon install / uninstall".into(),
        }
    }

    /// The service's file for `init`.
    pub fn file(&self, init: Init) -> PathBuf {
        match init {
            Init::Systemd => unit_path(&self.unit),
            _ => plist_path(&self.label),
        }
    }

    /// The file's content for `init`.
    pub fn content(&self, init: Init) -> String {
        match init {
            Init::Systemd => unit(self),
            _ => plist(self),
        }
    }

    /// `dev.aleph.daemon` / `aleph.service`, for messages.
    pub fn name(&self, init: Init) -> &str {
        match init {
            Init::Systemd => &self.unit,
            _ => &self.label,
        }
    }
}

/// Where launchd writes the daemon's own output.
pub fn daemon_log() -> PathBuf {
    home().join("Library/Logs/aleph.log")
}

/// The binary a service should run: this one, symlinks resolved.
pub fn exe() -> PathBuf {
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_else(|_| PathBuf::from(crate::brand::BIN))
}

/// A `cargo build` output rather than an installed binary.
pub fn dev_build(exe: &Path) -> bool {
    let s = exe.to_string_lossy();
    s.contains("/target/debug/") || s.contains("/target/release/")
}

// ------------------------------------------------------------------ the files

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// PATH for the service: ours plus Homebrew's, without duplicates.
pub fn service_path(cur: &str) -> String {
    let mut v: Vec<&str> = Vec::new();
    for p in cur
        .split(':')
        .chain(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"])
    {
        if !p.is_empty() && !v.contains(&p) {
            v.push(p);
        }
    }
    v.join(":")
}

/// The environment variables a service keeps from ours: where the config,
/// cache and state live, and the log level.
pub const KEPT: &[&str] = &[
    "ALEPH_CONFIG",
    "ALEPH_LOG",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
];

/// [`KEPT`] as set in this process.
pub fn kept_env() -> Vec<(String, String)> {
    KEPT.iter()
        .filter_map(|k| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| (k.to_string(), v))
        })
        .collect()
}

pub fn plist(s: &Spec) -> String {
    let mut envs = format!(
        "        <key>PATH</key>\n        <string>{}</string>\n",
        xml(&s.path)
    );
    for (k, v) in &s.env {
        envs.push_str(&format!(
            "        <key>{}</key>\n        <string>{}</string>\n",
            xml(k),
            xml(v)
        ));
    }
    let args: String = s
        .args
        .iter()
        .map(|a| format!("        <string>{}</string>\n", xml(a)))
        .collect();
    let log = xml(&s.log.to_string_lossy());
    // an XML comment can't hold "--"
    let by = xml(&s.by).replace("--", "- -");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- managed by aleph: {by} -->
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>30</integer>
    <key>ProcessType</key>
    <string>Background</string>
    <key>EnvironmentVariables</key>
    <dict>
{envs}    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
        xml(&s.label),
    )
}

/// systemd quoting for an `Environment=` assignment (`%` is a specifier).
fn sd_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

/// One word of `ExecStart=`: `%` and `$` doubled, quoted when it needs it.
fn sd_word(s: &str) -> String {
    let esc = s.replace('%', "%%").replace('$', "$$");
    if !esc.is_empty()
        && esc
            .chars()
            .all(|c| !c.is_whitespace() && !"\"'\\;".contains(c))
    {
        return esc;
    }
    format!("\"{}\"", esc.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn unit(s: &Spec) -> String {
    let mut envs = format!("Environment={}\n", sd_quote(&format!("PATH={}", s.path)));
    for (k, v) in &s.env {
        envs.push_str(&format!("Environment={}\n", sd_quote(&format!("{k}={v}"))));
    }
    let exec: Vec<String> = s.args.iter().map(|a| sd_word(a)).collect();
    format!(
        "# managed by aleph: {}\n\
         [Unit]\n\
         Description={}\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         RestartSec=30\n\
         {envs}\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        s.by,
        s.description.replace('%', "%%"),
        exec.join(" "),
    )
}

pub fn plist_path(label: &str) -> PathBuf {
    home()
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist"))
}

pub fn unit_path(unit: &str) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("systemd/user")
        .join(unit)
}

// ------------------------------------------------------------------ commands

/// One service-manager call.
#[derive(Clone, Debug, PartialEq)]
pub struct Cmd {
    pub prog: &'static str,
    pub args: Vec<String>,
    /// a failure here is fine (unloading what isn't loaded)
    pub quiet: bool,
}

impl Cmd {
    fn new(prog: &'static str, args: &[&str]) -> Cmd {
        Cmd {
            prog,
            args: args.iter().map(|a| a.to_string()).collect(),
            quiet: false,
        }
    }
    fn quiet(mut self) -> Cmd {
        self.quiet = true;
        self
    }
    /// As you'd type it.
    pub fn line(&self) -> String {
        std::iter::once(self.prog.to_string())
            .chain(self.args.iter().map(|a| sh_quote(a)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Install,
    Uninstall,
    Start,
    Stop,
    Restart,
}

/// (pure) The calls that carry out `op`. `loaded`: launchd already has it.
/// Install and uninstall write / remove the file around these (see
/// [`after_uninstall`]).
pub fn commands(init: Init, op: Op, s: &Spec, uid: &str, loaded: bool) -> Vec<Cmd> {
    match init {
        Init::Launchd => {
            let dom = format!("gui/{uid}");
            let target = format!("{dom}/{}", s.label);
            let file = s.file(init).to_string_lossy().into_owned();
            let boot = vec![
                Cmd::new("launchctl", &["enable", &target]).quiet(),
                Cmd::new("launchctl", &["bootstrap", &dom, &file]),
            ];
            match op {
                Op::Install => {
                    let mut v = vec![Cmd::new("launchctl", &["bootout", &target]).quiet()];
                    v.extend(boot);
                    v
                }
                Op::Uninstall => vec![Cmd::new("launchctl", &["bootout", &target]).quiet()],
                Op::Start if loaded => vec![Cmd::new("launchctl", &["kickstart", &target])],
                Op::Restart if loaded => {
                    vec![Cmd::new("launchctl", &["kickstart", "-k", &target])]
                }
                Op::Start | Op::Restart => boot,
                Op::Stop if loaded => vec![Cmd::new("launchctl", &["bootout", &target])],
                Op::Stop => vec![],
            }
        }
        Init::Systemd => {
            let u = s.unit.as_str();
            match op {
                Op::Install => vec![
                    Cmd::new("systemctl", &["--user", "daemon-reload"]),
                    Cmd::new("systemctl", &["--user", "enable", u]),
                    Cmd::new("systemctl", &["--user", "restart", u]),
                ],
                Op::Uninstall => {
                    vec![Cmd::new("systemctl", &["--user", "disable", "--now", u]).quiet()]
                }
                Op::Start => vec![Cmd::new("systemctl", &["--user", "start", u])],
                Op::Stop => vec![Cmd::new("systemctl", &["--user", "stop", u])],
                Op::Restart => vec![Cmd::new("systemctl", &["--user", "restart", u])],
            }
        }
        Init::Unsupported => vec![],
    }
}

/// (pure) What follows removing the file.
pub fn after_uninstall(init: Init) -> Vec<Cmd> {
    match init {
        Init::Systemd => vec![Cmd::new("systemctl", &["--user", "daemon-reload"]).quiet()],
        _ => vec![],
    }
}

/// Run a service manager (`launchctl`, `systemctl`, `loginctl`, …) with a
/// deadline. Unit tests never reach a real one.
pub fn sys(prog: &str, args: &[&str]) -> Out {
    sys_t(prog, args, T_CTL)
}

pub fn sys_t(prog: &str, args: &[&str], timeout: Duration) -> Out {
    if cfg!(test) {
        return Out {
            code: None,
            stderr: format!("{prog}: not run from unit tests"),
            ..Default::default()
        };
    }
    proc::cap(prog, args, timeout)
}

/// Run (or, `dry`, print) the calls in order; one that fails and isn't
/// quiet stops the rest.
pub fn run_cmds(dry: bool, cmds: &[Cmd]) -> Result<(), String> {
    for c in cmds {
        if dry {
            out(Line::from(Span::styled(
                format!("  $ {}", c.line()),
                ui::muted(),
            )));
            continue;
        }
        let args: Vec<&str> = c.args.iter().map(String::as_str).collect();
        let o = sys(c.prog, &args);
        if !o.ok() && !c.quiet {
            return Err(format!("{}: {}", c.line(), o.reason()));
        }
    }
    Ok(())
}

/// Write (or, `dry`, show) a service file.
pub fn write_file(dry: bool, path: &Path, content: &str) -> Result<(), String> {
    if dry {
        out(Line::from(Span::styled(
            format!("── {}", tilde(path)),
            ui::fg(c::ACCENT()),
        )));
        print!("{content}");
        return Ok(());
    }
    crate::config::write_atomic(path, content.as_bytes())
        .map_err(|e| format!("couldn't write {}: {e}", tilde(path)))
}

/// Remove (or, `dry`, show removing) a file; gone already is fine.
pub fn remove_file(dry: bool, p: &Path) -> Result<(), String> {
    if dry {
        out(Line::from(Span::styled(
            format!("  $ rm {}", sh_quote(&p.to_string_lossy())),
            ui::muted(),
        )));
        return Ok(());
    }
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("couldn't remove {}: {e}", tilde(p))),
    }
}

pub fn uid() -> String {
    proc::cap("id", &["-u"], Duration::from_secs(5))
        .text()
        .trim()
        .to_string()
}

pub fn user_name() -> String {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| {
            proc::cap("id", &["-un"], Duration::from_secs(5))
                .text()
                .trim()
                .to_string()
        })
}

// ------------------------------------------------------------------ state

/// What the service manager says about a service.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// its file is there
    pub installed: bool,
    /// launchd has it / systemd loaded it
    pub loaded: bool,
    pub running: bool,
    pub pid: Option<u32>,
    /// the manager's own words: `running`, `active (running)`, `failed`, …
    pub detail: String,
    pub last_exit: Option<String>,
    /// systemd: starts at login
    pub enabled: Option<bool>,
    /// the manager couldn't be asked
    pub error: Option<String>,
}

/// (pure) `launchctl print gui/<uid>/<label>`: `found` is its exit status
/// (non-zero: launchd doesn't have it).
pub fn parse_launchctl_print(found: bool, text: &str) -> State {
    let mut st = State {
        loaded: found,
        ..Default::default()
    };
    if !found {
        st.detail = "not loaded".into();
        return st;
    }
    // the service's own properties are one tab in; nested blocks are deeper
    for l in text.lines() {
        let Some(rest) = l.strip_prefix('\t') else {
            continue;
        };
        if rest.starts_with('\t') || rest.starts_with(' ') {
            continue;
        }
        let Some((k, v)) = rest.split_once(" = ") else {
            continue;
        };
        match k.trim() {
            "state" if st.detail.is_empty() => st.detail = v.trim().to_string(),
            "pid" => st.pid = v.trim().parse().ok(),
            "last exit code" => st.last_exit = Some(v.trim().to_string()),
            _ => {}
        }
    }
    st.running = st.detail == "running" || st.pid.is_some();
    if st.detail.is_empty() {
        st.detail = if st.running { "running" } else { "loaded" }.into();
    }
    st
}

/// (pure) `systemctl --user show <unit> -p …`.
pub fn parse_systemctl_show(text: &str) -> State {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('=')))
            .map(str::trim)
            .unwrap_or("")
    };
    let load = get("LoadState");
    let active = get("ActiveState");
    let sub = get("SubState");
    let pid: Option<u32> = get("MainPID").parse().ok().filter(|p| *p > 0);
    let result = get("Result");
    let file = get("UnitFileState");
    State {
        installed: !load.is_empty() && load != "not-found",
        loaded: load == "loaded",
        running: active == "active" || active == "activating" || active == "reloading",
        pid,
        detail: match (active, sub) {
            ("", _) => "unknown".into(),
            (a, "") => a.to_string(),
            (a, s) => format!("{a} ({s})"),
        },
        last_exit: (!result.is_empty() && result != "success").then(|| result.to_string()),
        enabled: (!file.is_empty()).then(|| file == "enabled" || file == "enabled-runtime"),
        error: None,
    }
}

/// (pure) `loginctl show-user <user> -p Linger`.
pub fn parse_linger(text: &str) -> Option<bool> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Linger="))
        .map(|v| v.trim() == "yes")
}

/// Ask the service manager about `s`.
pub fn state(init: Init, s: &Spec) -> State {
    let installed = s.file(init).exists();
    let mut st = match init {
        Init::Launchd => {
            let target = format!("gui/{}/{}", uid(), s.label);
            let o = sys("launchctl", &["print", &target]);
            if o.code.is_none() {
                State {
                    error: Some(o.reason()),
                    ..Default::default()
                }
            } else {
                parse_launchctl_print(o.ok(), &o.text())
            }
        }
        Init::Systemd => {
            let o = sys(
                "systemctl",
                &[
                    "--user",
                    "show",
                    &s.unit,
                    "-p",
                    "LoadState",
                    "-p",
                    "ActiveState",
                    "-p",
                    "SubState",
                    "-p",
                    "MainPID",
                    "-p",
                    "UnitFileState",
                    "-p",
                    "Result",
                ],
            );
            if o.ok() {
                parse_systemctl_show(&o.text())
            } else {
                State {
                    error: Some(o.reason()),
                    ..Default::default()
                }
            }
        }
        Init::Unsupported => State::default(),
    };
    st.installed = installed;
    st
}

/// Will this user's services keep running after they log out? (systemd's
/// linger; None: couldn't tell)
pub fn linger() -> Option<bool> {
    let user = user_name();
    let o = sys("loginctl", &["show-user", &user, "-p", "Linger"]);
    if o.ok() {
        if let Some(v) = parse_linger(&o.text()) {
            return Some(v);
        }
    }
    // logind's own record of it
    let d = Path::new("/var/lib/systemd/linger");
    if !cfg!(test) && d.is_dir() {
        return Some(d.join(&user).exists());
    }
    None
}

// ------------------------------------------------------------------ installed files

/// What an installed service file says it runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Installed {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// launchd KeepAlive (`always`, `conditional`) / systemd `Restart=`
    pub restart: Option<String>,
}

impl Installed {
    pub fn env(&self, k: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(x, _)| x == k)
            .map(|(_, v)| v.as_str())
    }
}

/// Read and parse `s`'s file (None: not there, or not readable).
pub fn installed(init: Init, s: &Spec) -> Option<Installed> {
    let text = std::fs::read_to_string(s.file(init)).ok()?;
    match init {
        Init::Systemd => Some(parse_unit(&text)),
        Init::Launchd => parse_plist(&text),
        Init::Unsupported => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
enum P {
    Str(String),
    Bool(bool),
    Arr(Vec<P>),
    Dict(Vec<(String, P)>),
    Other,
}

fn unxml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `<tag>` / text pieces of a plist (comments, `<?xml` and `<!DOCTYPE` dropped).
fn plist_tokens(t: &str) -> Vec<(bool, String)> {
    let mut v = Vec::new();
    let mut rest = t;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map_or("", |i| &r[i + 3..]);
            continue;
        }
        if let Some(r) = rest.strip_prefix('<') {
            let end = r.find('>').unwrap_or(r.len());
            let tag = r[..end].trim().to_string();
            rest = r.get(end + 1..).unwrap_or("");
            if !tag.starts_with('?') && !tag.starts_with('!') {
                v.push((true, tag));
            }
            continue;
        }
        let end = rest.find('<').unwrap_or(rest.len());
        let text = &rest[..end];
        if !text.trim().is_empty() {
            v.push((false, unxml(text)));
        }
        rest = &rest[end..];
    }
    v
}

fn plist_value(toks: &[(bool, String)], i: &mut usize) -> Option<P> {
    let (is_tag, tag) = toks.get(*i)?.clone();
    *i += 1;
    if !is_tag {
        return Some(P::Other);
    }
    let text_until = |i: &mut usize, close: &str| -> String {
        let mut s = String::new();
        while let Some((t, x)) = toks.get(*i) {
            *i += 1;
            if *t && x == close {
                break;
            }
            if !*t {
                s.push_str(x);
            }
        }
        s
    };
    match tag.as_str() {
        "true/" => Some(P::Bool(true)),
        "false/" => Some(P::Bool(false)),
        "string/" => Some(P::Str(String::new())),
        "dict/" => Some(P::Dict(vec![])),
        "array/" => Some(P::Arr(vec![])),
        "string" => Some(P::Str(text_until(i, "/string"))),
        "array" => {
            let mut v = Vec::new();
            while let Some((t, x)) = toks.get(*i) {
                if *t && x == "/array" {
                    *i += 1;
                    break;
                }
                v.push(plist_value(toks, i)?);
            }
            Some(P::Arr(v))
        }
        "dict" => {
            let mut v = Vec::new();
            while let Some((t, x)) = toks.get(*i) {
                if *t && x == "/dict" {
                    *i += 1;
                    break;
                }
                if *t && x == "key" {
                    *i += 1;
                    let k = text_until(i, "/key");
                    let val = plist_value(toks, i)?;
                    v.push((k, val));
                } else {
                    *i += 1;
                }
            }
            Some(P::Dict(v))
        }
        t if t.starts_with("plist") => plist_value(toks, i),
        t if !t.starts_with('/') && !t.ends_with('/') => {
            // <integer>, <real>, <date>, <data>: skip to its end
            let _ = text_until(i, &format!("/{t}"));
            Some(P::Other)
        }
        _ => Some(P::Other),
    }
}

/// (pure) A launchd plist: the program, its environment, KeepAlive.
pub fn parse_plist(text: &str) -> Option<Installed> {
    let toks = plist_tokens(text);
    let mut i = 0;
    let P::Dict(root) = plist_value(&toks, &mut i)? else {
        return None;
    };
    let get = |k: &str| root.iter().find(|(x, _)| x == k).map(|(_, v)| v);
    let args = match get("ProgramArguments") {
        Some(P::Arr(a)) => a
            .iter()
            .filter_map(|x| match x {
                P::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => match get("Program") {
            Some(P::Str(s)) => vec![s.clone()],
            _ => vec![],
        },
    };
    let env = match get("EnvironmentVariables") {
        Some(P::Dict(d)) => d
            .iter()
            .filter_map(|(k, v)| match v {
                P::Str(s) => Some((k.clone(), s.clone())),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
    let restart = match get("KeepAlive") {
        Some(P::Bool(true)) => Some("always".to_string()),
        Some(P::Dict(_)) => Some("conditional".to_string()),
        _ => None,
    };
    Some(Installed { args, env, restart })
}

/// systemd's word splitting for `ExecStart=` / `Environment=` values.
fn sd_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut it = s.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '\\' => {
                if let Some(n) = it.next() {
                    cur.push(n);
                    any = true;
                }
            }
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out.into_iter()
        .map(|w| w.replace("%%", "%").replace("$$", "$"))
        .collect()
}

/// (pure) A systemd unit: `ExecStart=`, `Environment=`, `Restart=`.
pub fn parse_unit(text: &str) -> Installed {
    let mut v = Installed::default();
    for l in text.lines().map(str::trim) {
        if let Some(r) = l.strip_prefix("ExecStart=") {
            let r = r.trim_start_matches(['-', '@', '+', '!', ':']);
            v.args = sd_words(r);
        } else if let Some(r) = l.strip_prefix("Environment=") {
            for w in sd_words(r) {
                if let Some((k, x)) = w.split_once('=') {
                    v.env.retain(|(y, _)| y != k);
                    v.env.push((k.to_string(), x.to_string()));
                }
            }
        } else if let Some(r) = l.strip_prefix("Restart=") {
            v.restart = Some(r.trim().to_string()).filter(|x| x != "no");
        }
    }
    v
}

// ------------------------------------------------------------------ the rename

/// Is this process the pre-rename background service itself? (launchd sets
/// `XPC_SERVICE_NAME`; systemd puts the unit in our cgroup.) The service must
/// never move itself — a bootout would kill it halfway.
pub fn running_as_legacy_service() -> bool {
    if std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| v == crate::brand::LEGACY_LABEL) {
        return true;
    }
    std::fs::read_to_string("/proc/self/cgroup")
        .is_ok_and(|c| c.contains(crate::brand::LEGACY_UNIT))
}

/// The rename's service step: if the old `com.claude-herdr.watch` (launchd) or
/// `claude-herdr-watch.service` (systemd) is installed, install the new one
/// for this binary first, then stop and remove the old one.
pub fn migrate_legacy() -> Result<Option<String>, String> {
    if running_as_legacy_service() {
        return Ok(None);
    }
    let init = Init::here();
    let spec = Spec::daemon(&exe());
    let (old, legacy) = match init {
        Init::Launchd => (
            plist_path(crate::brand::LEGACY_LABEL),
            crate::brand::LEGACY_LABEL,
        ),
        Init::Systemd => (
            unit_path(crate::brand::LEGACY_UNIT),
            crate::brand::LEGACY_UNIT,
        ),
        Init::Unsupported => return Ok(None),
    };
    if !old.exists() {
        return Ok(None);
    }
    write_file(false, &spec.file(init), &spec.content(init))?;
    let uid = uid();
    run_cmds(false, &commands(init, Op::Install, &spec, &uid, false))?;
    let stop_old = match init {
        Init::Launchd => {
            vec![Cmd::new("launchctl", &["bootout", &format!("gui/{uid}/{legacy}")]).quiet()]
        }
        _ => vec![Cmd::new("systemctl", &["--user", "disable", "--now", legacy]).quiet()],
    };
    run_cmds(false, &stop_old)?;
    std::fs::remove_file(&old).map_err(|e| format!("{}: {e}", tilde(&old)))?;
    run_cmds(false, &after_uninstall(init))?;
    Ok(Some(format!(
        "{} {legacy} → {}",
        if init == Init::Launchd {
            "launchd"
        } else {
            "systemd"
        },
        spec.name(init)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(exe: &str) -> Spec {
        Spec {
            label: LABEL.into(),
            unit: UNIT.into(),
            description: "Aleph daemon".into(),
            args: vec![exe.into(), "daemon".into(), "run".into()],
            path: "/usr/bin:/opt/homebrew/bin".into(),
            env: vec![("ALEPH_CONFIG".into(), "/tmp/a&b".into())],
            log: PathBuf::from("/Users/me/Library/Logs/aleph.log"),
            by: "aleph daemon install / uninstall".into(),
        }
    }

    #[test]
    fn launchd_plist() {
        let p = plist(&spec("/Users/me/.cargo/bin/aleph"));
        assert!(p.starts_with("<?xml"));
        assert!(p.contains("<string>dev.aleph.daemon</string>"));
        assert!(p.contains(
            "<string>/Users/me/.cargo/bin/aleph</string>\n        <string>daemon</string>\n        <string>run</string>"
        ));
        assert!(p.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(p.contains("<key>KeepAlive</key>\n    <true/>"));
        assert!(p.contains("<key>PATH</key>\n        <string>/usr/bin:/opt/homebrew/bin</string>"));
        assert!(p.contains("<string>/tmp/a&amp;b</string>"));
        assert_eq!(p.matches("Logs/aleph.log").count(), 2);
        assert!(!p
            .split("<!--")
            .nth(1)
            .unwrap()
            .split("-->")
            .next()
            .unwrap()
            .contains("--"));
        // balanced tags
        for t in ["dict", "array", "plist"] {
            assert_eq!(
                p.matches(&format!("<{t}")).count(),
                p.matches(&format!("</{t}>")).count()
            );
        }
        if proc::which("plutil").is_some() {
            let f = std::env::temp_dir().join(format!("al-svc-{}.plist", std::process::id()));
            std::fs::write(&f, &p).unwrap();
            let o = proc::cap(
                "plutil",
                &[std::ffi::OsStr::new("-lint"), f.as_os_str()],
                Duration::from_secs(5),
            );
            assert!(o.ok(), "{}", o.text());
            let _ = std::fs::remove_file(f);
        }
        // and it reads back
        let i = parse_plist(&p).unwrap();
        assert_eq!(i.args, vec!["/Users/me/.cargo/bin/aleph", "daemon", "run"]);
        assert_eq!(i.env("ALEPH_CONFIG"), Some("/tmp/a&b"));
        assert_eq!(i.env("PATH"), Some("/usr/bin:/opt/homebrew/bin"));
        assert_eq!(i.restart.as_deref(), Some("always"));
    }

    #[test]
    fn plists_written_by_hand_parse() {
        let t = r#"<?xml version="1.0"?>
<!-- a comment > with a bracket -->
<plist version="1.0"><dict>
  <key>Label</key><string>x</string>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ProgramArguments</key><array><string>/usr/local/bin/aleph</string><string>watch</string></array>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>EnvironmentVariables</key><dict/>
</dict></plist>"#;
        let i = parse_plist(t).unwrap();
        assert_eq!(i.args, vec!["/usr/local/bin/aleph", "watch"]);
        assert_eq!(i.restart.as_deref(), Some("conditional"));
        assert!(i.env.is_empty());
        assert_eq!(parse_plist("not a plist"), None);
    }

    #[test]
    fn systemd_unit() {
        let mut s = spec("/home/me/my bin/aleph");
        s.env = vec![("XDG_CACHE_HOME".into(), "/c/100%".into())];
        s.path = "/usr/bin".into();
        let u = unit(&s);
        assert!(
            u.contains("ExecStart=\"/home/me/my bin/aleph\" daemon run\n"),
            "{u}"
        );
        assert!(u.contains("Environment=\"PATH=/usr/bin\"\n"));
        assert!(u.contains("Environment=\"XDG_CACHE_HOME=/c/100%%\"\n"));
        assert!(u.contains("Restart=on-failure\n"));
        assert!(u.contains("[Install]\nWantedBy=default.target\n"));
        // a plain path isn't quoted; $ and % are doubled
        let u = unit(&spec("/opt/a$b%c/aleph"));
        assert!(
            u.contains("ExecStart=/opt/a$$b%%c/aleph daemon run\n"),
            "{u}"
        );
        // and both read back
        let i = parse_unit(&unit(&s));
        assert_eq!(i.args, vec!["/home/me/my bin/aleph", "daemon", "run"]);
        assert_eq!(i.env("XDG_CACHE_HOME"), Some("/c/100%"));
        assert_eq!(i.env("PATH"), Some("/usr/bin"));
        assert_eq!(i.restart.as_deref(), Some("on-failure"));
        let old = parse_unit("[Service]\nExecStart=\"/u/aleph\" watch\nRestart=no\n");
        assert_eq!(old.args, vec!["/u/aleph", "watch"]);
        assert_eq!(old.restart, None);
    }

    #[test]
    fn service_paths() {
        assert_eq!(
            service_path("/usr/bin:/a::/usr/local/bin"),
            "/usr/bin:/a:/usr/local/bin:/opt/homebrew/bin:/bin"
        );
        assert_eq!(
            service_path(""),
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
        );
    }

    #[test]
    fn picks_the_manager() {
        assert_eq!(Init::pick(None, "macos"), Init::Launchd);
        assert_eq!(Init::pick(None, "linux"), Init::Systemd);
        assert_eq!(Init::pick(None, "freebsd"), Init::Unsupported);
        assert_eq!(Init::pick(Some("systemd"), "macos"), Init::Systemd);
        assert_eq!(Init::pick(Some("nope"), "macos"), Init::Launchd);
    }

    #[test]
    fn launchd_commands() {
        let s = spec("/u/aleph");
        let lines = |op, loaded| -> Vec<String> {
            commands(Init::Launchd, op, &s, "501", loaded)
                .iter()
                .map(Cmd::line)
                .collect()
        };
        let i = lines(Op::Install, false);
        assert_eq!(i[0], "launchctl bootout gui/501/dev.aleph.daemon");
        assert_eq!(i[1], "launchctl enable gui/501/dev.aleph.daemon");
        assert!(
            i[2].starts_with("launchctl bootstrap gui/501 ")
                && i[2].ends_with("LaunchAgents/dev.aleph.daemon.plist")
        );
        assert!(commands(Init::Launchd, Op::Install, &s, "501", false)[0].quiet);
        assert_eq!(
            lines(Op::Start, true),
            vec!["launchctl kickstart gui/501/dev.aleph.daemon"]
        );
        assert_eq!(lines(Op::Start, false).len(), 2, "not loaded: bootstrap it");
        assert_eq!(
            lines(Op::Restart, true),
            vec!["launchctl kickstart -k gui/501/dev.aleph.daemon"]
        );
        assert_eq!(
            lines(Op::Stop, true),
            vec!["launchctl bootout gui/501/dev.aleph.daemon"]
        );
        assert!(lines(Op::Stop, false).is_empty());
        assert_eq!(
            lines(Op::Uninstall, true),
            vec!["launchctl bootout gui/501/dev.aleph.daemon"]
        );
        assert!(after_uninstall(Init::Launchd).is_empty());
    }

    #[test]
    fn systemd_commands() {
        let s = spec("/u/aleph");
        let lines = |op| -> Vec<String> {
            commands(Init::Systemd, op, &s, "1000", false)
                .iter()
                .map(Cmd::line)
                .collect()
        };
        assert_eq!(
            lines(Op::Install),
            vec![
                "systemctl --user daemon-reload",
                "systemctl --user enable aleph.service",
                "systemctl --user restart aleph.service"
            ]
        );
        assert_eq!(
            lines(Op::Start),
            vec!["systemctl --user start aleph.service"]
        );
        assert_eq!(lines(Op::Stop), vec!["systemctl --user stop aleph.service"]);
        assert_eq!(
            lines(Op::Restart),
            vec!["systemctl --user restart aleph.service"]
        );
        assert_eq!(
            lines(Op::Uninstall),
            vec!["systemctl --user disable --now aleph.service"]
        );
        assert_eq!(
            after_uninstall(Init::Systemd)[0].line(),
            "systemctl --user daemon-reload"
        );
        assert!(commands(Init::Unsupported, Op::Install, &s, "1", false).is_empty());
    }

    #[test]
    fn reads_launchctl_print() {
        let t = "gui/501/dev.aleph.daemon = {\n\tactive count = 1\n\tpath = /Users/me/Library/LaunchAgents/dev.aleph.daemon.plist\n\ttype = LaunchAgent\n\tstate = running\n\n\tprogram = /Users/me/.cargo/bin/aleph\n\targuments = {\n\t\t/Users/me/.cargo/bin/aleph\n\t\tdaemon\n\t}\n\n\tendpoints = {\n\t\tstate = active\n\t}\n\tpid = 4242\n\tlast exit code = (never exited)\n}\n";
        let s = parse_launchctl_print(true, t);
        assert!(s.loaded && s.running);
        assert_eq!(s.pid, Some(4242));
        assert_eq!(s.detail, "running");
        assert_eq!(s.last_exit.as_deref(), Some("(never exited)"));
        let w = parse_launchctl_print(
            true,
            "x = {\n\tstate = not running\n\tlast exit code = 78: EX_CONFIG\n}\n",
        );
        assert!(w.loaded && !w.running && w.pid.is_none());
        assert_eq!(w.detail, "not running");
        let n = parse_launchctl_print(false, "Could not find service");
        assert!(!n.loaded && !n.running);
    }

    #[test]
    fn reads_systemctl_show() {
        let s = parse_systemctl_show("LoadState=loaded\nActiveState=active\nSubState=running\nMainPID=1234\nUnitFileState=enabled\nResult=success\n");
        assert!(s.installed && s.loaded && s.running);
        assert_eq!(s.pid, Some(1234));
        assert_eq!(s.detail, "active (running)");
        assert_eq!(s.enabled, Some(true));
        assert_eq!(s.last_exit, None);
        let f = parse_systemctl_show("LoadState=loaded\nActiveState=failed\nSubState=failed\nMainPID=0\nUnitFileState=disabled\nResult=exit-code\n");
        assert!(!f.running && f.pid.is_none());
        assert_eq!(f.enabled, Some(false));
        assert_eq!(f.last_exit.as_deref(), Some("exit-code"));
        let n = parse_systemctl_show(
            "LoadState=not-found\nActiveState=inactive\nSubState=dead\nMainPID=0\n",
        );
        assert!(!n.installed && !n.loaded);
    }

    #[test]
    fn reads_linger() {
        assert_eq!(parse_linger("Linger=yes\n"), Some(true));
        assert_eq!(parse_linger("Linger=no"), Some(false));
        assert_eq!(parse_linger("Failed to get user: No such user"), None);
    }

    #[test]
    fn never_runs_a_manager_in_tests() {
        let o = sys("launchctl", &["print", "gui/0/x"]);
        assert!(!o.ok() && o.stderr.contains("unit tests"));
    }
}
