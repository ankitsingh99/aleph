//! `aleph daemon install | uninstall | start | stop | restart | status | logs
//! | doctor` — the daemon as a service: a launchd agent on macOS, a systemd
//! user unit on Linux (`service.rs` has the files and the commands).
//! `aleph watch --install` / `--uninstall` are the same thing.
//!
//! * `install` writes the service file for this binary and (re)loads it;
//!   on Linux it says when linger is off (the daemon would stop at logout)
//!   and how to turn it on — never doing that for you.
//! * `start` / `stop` / `restart` go through the service manager (launchd:
//!   `kickstart`, `bootout` / `bootstrap`; systemd: `systemctl --user`). With
//!   no service installed they say so; a daemon started by hand can still be
//!   stopped.
//! * `status` puts together what the service manager, the socket, the hub
//!   lock, the logs and the HTTP API say ([`Facts`] → [`status_json`], pure);
//!   `doctor` turns the same facts into checks with fixes ([`checks`], pure),
//!   which `aleph doctor` and Settings › Daemon & API show too.
//! * `logs` is the daemon's own lines from `aleph.log`; `--service` what the
//!   service manager kept (the launchd log file, or the journal).
//! * `--dry-run` shows the files and the commands, and changes nothing.

use crate::config::Config;
use crate::doctor::{self, tilde, Fix, Line as Check, St};
use crate::out::*;
use crate::proc;
use crate::service::{self, Init, Op, Spec};
use crate::ui::{self, c};
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// A launchd log bigger than this is rotated (to `aleph.log.1`) whenever
/// the daemon (re)starts through us.
const LOG_ROTATE: u64 = 5 * 1024 * 1024;
/// The doctor warns about a service log bigger than this.
const LOG_BIG: u64 = 10 * 1024 * 1024;

/// `aleph daemon <sub> …` for the subcommands this module owns.
pub fn cmd(a: crate::cli::Args) -> ExitCode {
    let p = a.positional();
    let dry = a.has("--dry-run");
    let assume = a.has("--yes") || a.has("-y");
    match p.first().copied() {
        Some("install") => install(dry),
        Some("uninstall") => uninstall(dry),
        Some("start") => control(Op::Start, dry, assume),
        Some("stop") => control(Op::Stop, dry, assume),
        Some("restart") => control(Op::Restart, dry, assume),
        Some("logs") => logs(&a),
        Some("doctor") => doctor_cmd(&a),
        Some("expose") => crate::expose::cmd(a),
        Some("deploy") => crate::deploy::cmd(a),
        None | Some("status") | Some("ping") => status(&a),
        Some(x) => {
            err(&format!("aleph daemon {x}: no such subcommand"));
            ExitCode::from(2)
        }
    }
}

/// The daemon's service for this binary.
fn spec() -> Spec {
    Spec::daemon(&service::exe())
}

fn unsupported() -> ExitCode {
    err(&format!(
        "Aleph doesn't know this system's service manager ({})",
        std::env::consts::OS
    ));
    hint("run `aleph daemon run` under your own supervisor");
    ExitCode::from(2)
}

/// Where the service's own output goes, for messages.
fn log_where(init: Init, s: &Spec) -> String {
    match init {
        Init::Systemd => format!("log: journalctl --user -u {}", s.unit),
        _ => format!("log {}", tilde(&s.log)),
    }
}

// ------------------------------------------------------------------ install

/// Write the service file for this binary and (re)load it.
fn do_install(dry: bool, quiet: bool) -> Result<(Init, Spec), String> {
    let init = Init::here();
    if init == Init::Unsupported {
        return Err(format!(
            "Aleph doesn't know this system's service manager ({})",
            std::env::consts::OS
        ));
    }
    let s = spec();
    if init == Init::Launchd {
        rotate_log(&s.log, dry, quiet);
    }
    service::write_file(dry, &s.file(init), &s.content(init))?;
    service::run_cmds(
        dry,
        &service::commands(init, Op::Install, &s, &service::uid(), false),
    )?;
    Ok((init, s))
}

/// `aleph daemon install` (and `aleph watch --install`).
pub fn install(dry: bool) -> ExitCode {
    if Init::here() == Init::Unsupported {
        return unsupported();
    }
    let exe = service::exe();
    if service::dev_build(&exe) {
        warn(&format!(
            "{} is a development build — the service runs that file (cargo install --path . gives it a steady home)",
            tilde(&exe)
        ));
    }
    // a daemon someone started by hand keeps the socket: the service's waits
    let by_hand = if dry {
        None
    } else {
        let svc = service::state(Init::here(), &spec());
        ping().map(|(pid, _)| pid).filter(|p| svc.pid != Some(*p))
    };
    let (init, s) = match do_install(dry, false) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if dry {
        hint("nothing done (--dry-run)");
        return ExitCode::SUCCESS;
    }
    ok(&format!(
        "the daemon runs at login: {} {} · {}",
        init.word(),
        s.name(init),
        log_where(init, &s)
    ));
    if init == Init::Systemd {
        linger_note();
    }
    if let Some(pid) = by_hand {
        warn(&format!(
            "a daemon started by hand already runs here (pid {pid}): the service's waits until it stops"
        ));
        return ExitCode::SUCCESS;
    }
    report_up(None);
    ExitCode::SUCCESS
}

/// `aleph daemon uninstall` (and `aleph watch --uninstall`).
pub fn uninstall(dry: bool) -> ExitCode {
    let init = Init::here();
    if init == Init::Unsupported {
        return unsupported();
    }
    let s = spec();
    let file = s.file(init);
    let was = file.exists();
    let r = (|| {
        service::run_cmds(
            dry,
            &service::commands(init, Op::Uninstall, &s, &service::uid(), true),
        )?;
        service::remove_file(dry, &file)?;
        service::run_cmds(dry, &service::after_uninstall(init))
    })();
    if let Err(e) = r {
        return fail(&e);
    }
    if dry {
        hint("nothing done (--dry-run)");
    } else if was {
        ok(&format!(
            "removed the {} service {} — the daemon no longer starts at login",
            init.word(),
            s.name(init)
        ));
    } else {
        ok(&format!(
            "the daemon wasn't installed as a service ({} {})",
            init.word(),
            s.name(init)
        ));
    }
    ExitCode::SUCCESS
}

// ------------------------------------------------------------------ start / stop

/// Carry out `op` through the service manager (Err: not installed, or it failed).
fn do_control(op: Op, dry: bool, quiet: bool) -> Result<(Init, Spec, bool), String> {
    let init = Init::here();
    let s = spec();
    let st = service::state(init, &s);
    if !st.installed {
        return Err("the daemon isn't installed as a service".into());
    }
    let cmds = service::commands(init, op, &s, &service::uid(), st.loaded);
    if cmds.is_empty() {
        return Ok((init, s, false));
    }
    if init == Init::Launchd && op != Op::Stop {
        rotate_log(&s.log, dry, quiet);
    }
    service::run_cmds(dry, &cmds)?;
    Ok((init, s, true))
}

/// `aleph daemon start | stop | restart`.
fn control(op: Op, dry: bool, assume: bool) -> ExitCode {
    let init = Init::here();
    if init == Init::Unsupported {
        return unsupported();
    }
    if !spec().file(init).exists() {
        err("the daemon isn't installed as a service");
        if let Some((pid, _)) = ping() {
            say(Line::from(Span::styled(
                format!("  a daemon started by hand runs here (pid {pid})"),
                ui::muted(),
            )));
            if op == Op::Stop {
                return stop_by_hand(pid, dry, assume);
            }
        }
        hint("aleph daemon install — runs it at login (or aleph daemon run, in a terminal)");
        return ExitCode::from(1);
    }
    let before = if dry { None } else { ping().map(|(p, _)| p) };
    let (init, s, did) = match do_control(op, dry, false) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if !did {
        ok("the daemon isn't running");
        return ExitCode::SUCCESS;
    }
    if dry {
        hint("nothing done (--dry-run)");
        return ExitCode::SUCCESS;
    }
    match op {
        Op::Stop => {
            if wait_until(Duration::from_secs(5), || ping().is_none()) {
                ok(&format!(
                    "stopped · it starts again at login (aleph daemon start: now) · {}",
                    log_where(init, &s)
                ));
            } else {
                warn("the service manager stopped it, but something still answers on the socket");
                hint("aleph daemon status");
            }
        }
        _ => report_up(if op == Op::Restart { before } else { None }),
    }
    ExitCode::SUCCESS
}

/// For the doctor's fixes (no terminal output).
pub fn install_quiet() -> Result<String, String> {
    let (init, s) = do_install(false, true)?;
    Ok(format!(
        "the daemon runs at login ({} {})",
        init.word(),
        s.name(init)
    ))
}

pub fn restart_quiet() -> Result<String, String> {
    do_control(Op::Restart, false, true).map(|_| "restarted the daemon".to_string())
}

fn stop_by_hand(pid: u32, dry: bool, assume: bool) -> ExitCode {
    if dry {
        out(Line::from(Span::styled(
            format!("  $ kill -TERM {pid}"),
            ui::muted(),
        )));
        hint("nothing done (--dry-run)");
        return ExitCode::SUCCESS;
    }
    if !assume && !yes(&format!("Stop it (pid {pid})?")) {
        return ExitCode::from(1);
    }
    let o = proc::cap("kill", &["-TERM", &pid.to_string()], Duration::from_secs(5));
    if !o.ok() {
        return fail(&format!("kill {pid}: {}", o.reason()));
    }
    if wait_until(Duration::from_secs(5), || ping().is_none()) {
        ok(&format!("stopped the daemon (pid {pid})"));
        ExitCode::SUCCESS
    } else {
        fail(&format!("pid {pid} still answers on the socket"))
    }
}

/// The running daemon's (pid, version), when one answers on the socket.
pub fn ping() -> Option<(u32, String)> {
    let v = crate::daemon::request_at(
        &crate::daemon::sock_path(),
        "daemon.ping",
        json!({}),
        Duration::from_secs(2),
    )
    .ok()?;
    let pid = v.get("pid").and_then(Value::as_u64)? as u32;
    let ver = v.get("version").and_then(Value::as_str).unwrap_or("");
    Some((pid, crate::showcase::own_version(ver).into_owned()))
}

fn wait_until(max: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    loop {
        if f() {
            return true;
        }
        if t0.elapsed() >= max {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// After a start: wait a moment for the daemon to answer (a new pid, after
/// a restart), and say how it went.
fn report_up(before: Option<u32>) {
    let sp = Spinner::start("Waiting for the daemon to answer".into());
    let mut got = None;
    wait_until(Duration::from_secs(4), || {
        got = ping().filter(|(p, _)| before != Some(*p));
        got.is_some()
    });
    sp.stop();
    match got {
        Some((pid, v)) => {
            ok(&format!("the daemon answers: aleph {v}, pid {pid}"));
            if v != env!("CARGO_PKG_VERSION") {
                warn(&format!(
                    "it runs aleph {v}, this binary is {}",
                    env!("CARGO_PKG_VERSION")
                ));
            }
        }
        None => hint("not answering yet — aleph daemon status shows how it's doing"),
    }
}

/// Rotate the launchd log when it's grown big: launchd opens it again when
/// it starts the daemon, so a (re)start through us is the moment.
fn rotate_log(log: &Path, dry: bool, quiet: bool) {
    let Ok(m) = std::fs::metadata(log) else {
        return;
    };
    if m.len() < LOG_ROTATE {
        return;
    }
    let old = log.with_extension("log.1");
    if dry {
        out(Line::from(Span::styled(
            format!("  $ mv {} {}", tilde(log), tilde(&old)),
            ui::muted(),
        )));
        return;
    }
    if std::fs::rename(log, &old).is_ok() && !quiet {
        say(Line::from(Span::styled(
            format!(
                "  rotated {} ({}) → {}",
                tilde(log),
                ui::bytes(m.len() as f64),
                old.file_name().unwrap_or_default().to_string_lossy()
            ),
            ui::muted(),
        )));
    }
}

/// systemd: say when the daemon would stop at logout, and how to change it.
fn linger_note() {
    if service::linger() == Some(false) {
        let user = service::user_name();
        warn("linger is off: your user's services, the daemon too, stop when you log out of this machine");
        hint(&format!(
            "loginctl enable-linger {user}   (some systems want sudo; Aleph never runs it for you)"
        ));
    }
}

// ------------------------------------------------------------------ the HTTP API's settings

/// The HTTP API's settings, read with their defaults (`api.rs` serves the
/// API; status, the doctor and `expose` only look).
#[derive(Clone, Debug, PartialEq)]
pub struct ApiCfg {
    /// `api.http`
    pub http: bool,
    /// `api.listen`
    pub listen: String,
    /// `api.listen_any`: binding every network is meant
    pub listen_any: bool,
    /// `api.hosts`: names the API answers to besides localhost
    pub hosts: Vec<String>,
}

pub const API_LISTEN: &str = "127.0.0.1:7878";

fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

impl ApiCfg {
    pub fn from(cfg: &Config) -> ApiCfg {
        ApiCfg {
            http: cfg.get("api.http").is_some_and(|v| truthy(&v)),
            listen: cfg
                .get("api.listen")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| API_LISTEN.into()),
            listen_any: cfg.get("api.listen_any").is_some_and(|v| truthy(&v)),
            hosts: split_list(&cfg.get("api.hosts").unwrap_or_default()),
        }
    }

    /// (host, port) of `api.listen`: `127.0.0.1:7878`, `[::1]:7878`, `:7878`, `7878`.
    pub fn split(&self) -> Option<(String, u16)> {
        split_listen(&self.listen)
    }

    pub fn port(&self) -> u16 {
        self.split().map(|(_, p)| p).unwrap_or(7878)
    }

    /// Where a client on this computer reaches it, as a URL.
    pub fn local_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }

    /// Listens on every network (0.0.0.0, ::).
    pub fn any(&self) -> bool {
        self.split()
            .is_some_and(|(h, _)| matches!(h.as_str(), "" | "0.0.0.0" | "::" | "*" | "[::]"))
    }
}

pub fn split_list(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

/// (pure) `host:port` → (host, port); a bare port means localhost.
pub fn split_listen(l: &str) -> Option<(String, u16)> {
    let l = l.trim();
    if let Ok(p) = l.parse::<u16>() {
        return Some(("127.0.0.1".into(), p));
    }
    if let Some(r) = l.strip_prefix('[') {
        let (h, rest) = r.split_once(']')?;
        let p = rest.strip_prefix(':')?.parse().ok()?;
        return Some((h.to_string(), p));
    }
    let (h, p) = l.rsplit_once(':')?;
    Some((h.to_string(), p.parse().ok()?))
}

/// Does something accept connections on `api.listen` here?
fn api_answers(a: &ApiCfg) -> Option<bool> {
    use std::net::ToSocketAddrs;
    let (h, p) = a.split()?;
    let h = if a.any() { "127.0.0.1".to_string() } else { h };
    let addr = (h.as_str(), p).to_socket_addrs().ok()?.next()?;
    Some(std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(800)).is_ok())
}

// ------------------------------------------------------------------ facts

/// Everything `status` and `doctor` look at, gathered once.
#[derive(Clone, Debug)]
pub struct Facts {
    /// this binary
    pub version: String,
    pub exe: PathBuf,
    pub init: Init,
    pub spec: Spec,
    pub svc: service::State,
    pub file: PathBuf,
    pub installed: Option<service::Installed>,
    /// what the service file's program is, next to this binary
    pub prog: Prog,
    pub sock: PathBuf,
    /// what `daemon.status` answered
    pub answer: Result<Value, String>,
    /// `hub.machine`
    pub hub_setting: String,
    pub this_machine: String,
    pub hub_here: bool,
    /// the pid in the hub lock, and whether it's alive
    pub holder: Option<(u32, bool)>,
    pub log: PathBuf,
    pub log_bytes: Option<u64>,
    /// launchd's file for the daemon's own output
    pub svc_log: Option<(PathBuf, u64)>,
    pub api: ApiCfg,
    pub api_answers: Option<bool>,
    pub linger: Option<bool>,
    /// where herdr is on this PATH
    pub herdr_dir: Option<String>,
    /// [`service::KEPT`] as this shell has them
    pub env_now: Vec<(String, Option<String>)>,
    /// where the API is reachable from outside (`expose.rs`)
    pub exposure: crate::expose::Exposure,
    /// device tokens the HTTP API has (None: can't tell)
    pub tokens: Option<usize>,
    /// daemons deployed elsewhere (`aleph daemon deploy`): (host, its hostname)
    pub deploys: Vec<(String, String)>,
}

impl Facts {
    pub fn running(&self) -> bool {
        self.answer.is_ok()
    }
    fn answer_str(&self, k: &str) -> Option<String> {
        self.answer
            .as_ref()
            .ok()?
            .get(k)
            .and_then(Value::as_str)
            .map(String::from)
    }
    fn answer_u64(&self, k: &str) -> Option<u64> {
        self.answer.as_ref().ok()?.get(k).and_then(Value::as_u64)
    }
    /// The running daemon's pid.
    pub fn pid(&self) -> Option<u32> {
        self.answer_u64("pid").map(|p| p as u32)
    }
    /// The running daemon's version.
    pub fn daemon_version(&self) -> Option<String> {
        self.answer_str("version")
            .map(|v| crate::showcase::own_version(&v).into_owned())
    }
}

/// The service file's program, next to this binary.
#[derive(Clone, Debug, PartialEq)]
pub enum Prog {
    /// no service file (or unreadable)
    Unknown,
    /// the file runs nothing
    Nothing,
    /// a program that isn't there any more
    Gone(String),
    /// another binary
    Other(String),
    /// this binary
    This,
}

fn prog_of(i: Option<&service::Installed>, exe: &Path) -> Prog {
    let Some(i) = i else { return Prog::Unknown };
    let Some(p) = i.args.first().filter(|p| !p.is_empty()) else {
        return Prog::Nothing;
    };
    match Path::new(p).canonicalize() {
        Err(_) => Prog::Gone(p.clone()),
        Ok(c) if c == exe => Prog::This,
        Ok(_) => Prog::Other(p.clone()),
    }
}

/// How many device tokens the HTTP API has: the daemon's `daemon.status`
/// (`tokens`: a count or a list) or its `token.list`; None when neither says.
fn tokens_of(answer: &Result<Value, String>) -> Option<usize> {
    let a = answer.as_ref().ok()?;
    let n = |v: &Value| match v {
        Value::Number(n) => n.as_u64().map(|n| n as usize),
        Value::Array(a) => Some(a.len()),
        _ => None,
    };
    if let Some(x) = a.get("tokens").and_then(n) {
        return Some(x);
    }
    crate::daemon::request_at(
        &crate::daemon::sock_path(),
        "token.list",
        json!({}),
        Duration::from_secs(3),
    )
    .ok()
    .as_ref()
    .and_then(n)
}

fn alive(pid: u32) -> bool {
    proc::cap("kill", &["-0", &pid.to_string()], Duration::from_secs(2)).ok()
}

/// Ask everything (the service manager, the socket, the hub lock, the API).
pub fn gather(cfg: &Config) -> Facts {
    let exe = service::exe();
    let init = Init::here();
    let spec = Spec::daemon(&exe);
    let svc = if init == Init::Unsupported {
        service::State::default()
    } else {
        service::state(init, &spec)
    };
    let installed = service::installed(init, &spec);
    let prog = prog_of(installed.as_ref(), &exe);
    let sock = crate::daemon::sock_path();
    let answer =
        crate::daemon::request_at(&sock, "daemon.status", json!({}), Duration::from_secs(5));
    let holder = crate::auto::HubLock::holder().map(|p| (p, alive(p)));
    let log = crate::log::path();
    let log_bytes = std::fs::metadata(&log).ok().map(|m| m.len());
    let svc_log = (init == Init::Launchd)
        .then(|| {
            std::fs::metadata(&spec.log)
                .ok()
                .map(|m| (spec.log.clone(), m.len()))
        })
        .flatten();
    let api = ApiCfg::from(cfg);
    let api_answers = api_answers(&api);
    let linger = (init == Init::Systemd).then(service::linger).flatten();
    let herdr_dir = proc::which(&crate::herdr::bin())
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()));
    let env_now = service::KEPT
        .iter()
        .map(|k| {
            (
                k.to_string(),
                std::env::var(k).ok().filter(|v| !v.is_empty()),
            )
        })
        .collect();
    let exposure = crate::expose::current(cfg);
    let tokens = if exposure.any() {
        tokens_of(&answer)
    } else {
        None
    };
    let deploys = crate::deploy::deploys()
        .into_iter()
        .map(|(h, v)| {
            let hn = v
                .get("hostname")
                .and_then(Value::as_str)
                .unwrap_or(&h)
                .to_lowercase();
            (h, hn)
        })
        .collect();
    Facts {
        version: env!("CARGO_PKG_VERSION").into(),
        file: spec.file(init),
        exe,
        init,
        spec,
        svc,
        installed,
        prog,
        sock,
        answer,
        hub_setting: cfg.get("hub.machine").unwrap_or_default(),
        this_machine: crate::auto::this_machine(),
        hub_here: crate::auto::hub_here(cfg),
        holder,
        log,
        log_bytes,
        svc_log,
        api,
        api_answers,
        linger,
        herdr_dir,
        env_now,
        exposure,
        tokens,
        deploys,
    }
}

// ------------------------------------------------------------------ status

/// (pure) `aleph daemon status --json`: the daemon's own answer at the top
/// (`running`, `version`, `pid`, `uptime_s`, `hub`, …), then the service,
/// this binary, the hub, the logs and the HTTP API.
pub fn status_json(f: &Facts) -> Value {
    let mut v = match &f.answer {
        Ok(a) => {
            let mut a = a.clone();
            a["running"] = json!(true);
            a
        }
        Err(e) => json!({"running": false, "error": e, "socket": f.sock}),
    };
    v["binary"] = json!({"version": f.version, "path": f.exe});
    v["version_match"] = match f.daemon_version() {
        Some(d) => json!(d == f.version),
        None => Value::Null,
    };
    v["service"] = json!({
        "manager": f.init.word(),
        "name": f.spec.name(f.init),
        "file": f.file,
        "installed": f.svc.installed,
        "loaded": f.svc.loaded,
        "running": f.svc.running,
        "pid": f.svc.pid,
        "state": f.svc.detail,
        "last_exit": f.svc.last_exit,
        "enabled": f.svc.enabled,
        "error": f.svc.error,
        "program": f.installed.as_ref().map(|i| i.args.clone()),
        "linger": f.linger,
    });
    v["hub_machine"] = json!({
        "setting": f.hub_setting,
        "this_machine": f.this_machine,
        "here": f.hub_here,
        "holder": f.holder.map(|h| h.0),
        "holder_alive": f.holder.map(|h| h.1),
    });
    v["logs"] = json!({
        "path": f.log,
        "bytes": f.log_bytes,
        "service_log": f.svc_log.as_ref().map(|x| &x.0),
        "service_log_bytes": f.svc_log.as_ref().map(|x| x.1),
        "journal": (f.init == Init::Systemd).then(|| format!("journalctl --user -u {}", f.spec.unit)),
    });
    v["api"] = json!({
        "http": f.api.http,
        "listen": f.api.listen,
        "listen_any": f.api.listen_any,
        "hosts": f.api.hosts,
        "answers": f.api_answers,
        "tokens": f.tokens,
    });
    v["exposure"] = f.exposure.to_json();
    v["deploys"] = json!(f
        .deploys
        .iter()
        .map(|(h, hn)| json!({"host": h, "hostname": hn}))
        .collect::<Vec<_>>());
    v
}

/// (pure) The rows of `aleph daemon status`: (label, text, colour).
pub fn status_rows(f: &Facts) -> Vec<(&'static str, String, Color)> {
    let mut v: Vec<(&'static str, String, Color)> = Vec::new();
    let name = f.spec.name(f.init).to_string();
    let svc = match (f.init, f.svc.installed) {
        (Init::Unsupported, _) => (
            "no service manager Aleph knows here".to_string(),
            c::MUTED(),
        ),
        (_, false) => (
            "not installed — aleph daemon install runs it at login".to_string(),
            c::MUTED(),
        ),
        (i, true) => {
            let pid = f.svc.pid.map(|p| format!(" (pid {p})")).unwrap_or_default();
            let col = if f.svc.running {
                c::GREEN()
            } else {
                c::YELLOW()
            };
            (
                format!(
                    "{} {name} · {}{pid} · {}",
                    i.word(),
                    if f.svc.detail.is_empty() {
                        "?"
                    } else {
                        f.svc.detail.as_str()
                    },
                    tilde(&f.file)
                ),
                col,
            )
        }
    };
    v.push(("service", svc.0, svc.1));
    v.push((
        "socket",
        format!(
            "{} · {}",
            tilde(&f.sock),
            if f.running() {
                "answers"
            } else {
                "nothing answers"
            }
        ),
        if f.running() { c::GREEN() } else { c::MUTED() },
    ));
    if let Some(d) = f.daemon_version() {
        if d == f.version {
            v.push(("version", format!("{d} · this binary too"), c::GREEN()));
        } else {
            v.push((
                "version",
                format!("the daemon runs {d}, this binary is {}", f.version),
                c::YELLOW(),
            ));
        }
    }
    let hub = if !f.hub_here {
        (
            format!(
                "on {} (hub.machine) — this computer only watches",
                f.hub_setting
            ),
            c::MUTED(),
        )
    } else {
        match f.holder {
            Some((h, true)) if Some(h) == f.pid() => {
                (format!("here, in the daemon (pid {h})"), c::GREEN())
            }
            Some((h, true)) => (format!("here, in pid {h} (an open dashboard?)"), c::GREEN()),
            _ if f.running() => ("nobody runs it right now".to_string(), c::YELLOW()),
            _ => (
                "not running — the daemon or an open dashboard runs it".to_string(),
                c::MUTED(),
            ),
        }
    };
    v.push(("hub", hub.0, hub.1));
    let size = |b: Option<u64>| b.map(|b| ui::bytes(b as f64)).unwrap_or_else(|| "–".into());
    let mut logs = format!("{} ({})", tilde(&f.log), size(f.log_bytes));
    match (&f.svc_log, f.init) {
        (Some((p, b)), _) => logs.push_str(&format!(
            " · service output {} ({})",
            tilde(p),
            size(Some(*b))
        )),
        (None, Init::Systemd) => logs.push_str(&format!(
            " · service output: journalctl --user -u {}",
            f.spec.unit
        )),
        _ => {}
    }
    v.push(("logs", logs, c::MUTED()));
    let api = match (f.api.http, f.api_answers) {
        (false, Some(true)) => (
            format!(
                "off (api.http) · but something else listens on {}",
                f.api.listen
            ),
            c::YELLOW(),
        ),
        (false, _) => (format!("off (api.http) · {}", f.api.listen), c::MUTED()),
        (true, Some(true)) => (format!("answers on {}", f.api.listen), c::GREEN()),
        (true, _) if f.running() => (
            format!("on, but nothing answers on {}", f.api.listen),
            c::YELLOW(),
        ),
        (true, _) => (
            format!("on · {} (starts with the daemon)", f.api.listen),
            c::MUTED(),
        ),
    };
    v.push(("http api", api.0, api.1));
    v.push(if f.exposure.any() {
        ("exposure", f.exposure.urls().join(" · "), c::GREEN())
    } else {
        (
            "exposure",
            "none — aleph daemon expose tailscale | cloudflare".to_string(),
            c::MUTED(),
        )
    });
    if f.init == Init::Systemd {
        match f.linger {
            Some(true) => v.push((
                "linger",
                "on: it keeps running after you log out".into(),
                c::GREEN(),
            )),
            Some(false) => v.push((
                "linger",
                "off: it stops when you log out (loginctl enable-linger)".into(),
                c::YELLOW(),
            )),
            None => {}
        }
    }
    v
}

/// `aleph daemon status [--json]`.
fn status(a: &crate::cli::Args) -> ExitCode {
    let f = gather(&a.cfg);
    let code = if f.running() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };
    if a.has("--json") {
        json(&status_json(&f));
        return code;
    }
    let rows = status_rows(&f);
    if !std::io::stdout().is_terminal() {
        // piped: field, text (TSV)
        println!("running\t{}", if f.running() { "yes" } else { "no" });
        for (k, t, _) in &rows {
            println!("{k}\t{t}");
        }
        return code;
    }
    let head = match &f.answer {
        Ok(_) => {
            let up = f.answer_u64("uptime_s").unwrap_or(0);
            format!(
                "running · aleph {} · pid {} · up {}",
                f.daemon_version().unwrap_or_default(),
                f.pid().unwrap_or(0),
                ui::dur(up)
            )
        }
        Err(_) => "not running".to_string(),
    };
    out(Line::from(vec![
        Span::styled(
            "⏺ ",
            ui::fg(if f.running() { c::GREEN() } else { c::MUTED() }),
        ),
        Span::styled("Aleph daemon", ui::bold()),
        Span::styled(format!("   {head}"), ui::muted()),
    ]));
    for (k, t, col) in &rows {
        out(Line::from(vec![
            Span::raw("  "),
            Span::styled(ui::pad(k, 9), ui::faint()),
            Span::raw(" "),
            Span::styled(t.clone(), ui::fg(*col)),
        ]));
    }
    if let (Some(d), true) = (f.daemon_version(), f.running()) {
        if d != f.version {
            warn(&format!(
                "the daemon runs aleph {d}, this binary is {} — restart it (aleph daemon restart), or it picks up the new binary by itself (it re-execs when the binary changes)",
                f.version
            ));
        }
    }
    if !f.running() {
        hint(if f.svc.installed {
            "aleph daemon start · aleph daemon logs --service shows why it stopped"
        } else {
            "aleph daemon install — runs it at login (or aleph daemon run, in a terminal)"
        });
    }
    code
}

// ------------------------------------------------------------------ doctor

fn check(
    st: St,
    label: &str,
    text: impl Into<String>,
    fix: &[String],
    action: Option<Fix>,
) -> Check {
    Check {
        st,
        label: label.into(),
        text: text.into(),
        fix: fix.to_vec(),
        action,
    }
}

fn s(x: &str) -> String {
    x.to_string()
}

/// (pure) The daemon's checks, each with how to fix it.
pub fn checks(f: &Facts) -> Vec<Check> {
    let mut v = Vec::new();
    let install_fix = || {
        vec![s(
            "aleph daemon install — writes the service for this binary, from this shell",
        )]
    };
    let name = f.spec.name(f.init).to_string();
    // the service
    match (f.init, f.svc.installed) {
        (Init::Unsupported, _) => v.push(check(
            St::Info,
            "service",
            "no service manager Aleph knows here — run `aleph daemon run` under your own",
            &[],
            None,
        )),
        (_, false) => {
            let text = match f.pid() {
                Some(p) => format!("not installed; a daemon started by hand runs (pid {p})"),
                None => "not installed — nothing starts the daemon at login".into(),
            };
            v.push(check(
                St::Info,
                "service",
                text,
                &[s(
                    "aleph daemon install — runs it at login and starts it again if it stops",
                )],
                Some(Fix::DaemonInstall),
            ));
        }
        (i, true) if f.svc.running => v.push(check(
            St::Ok,
            "service",
            format!(
                "{} {name} · {}{}",
                i.word(),
                f.svc.detail,
                f.svc.pid.map(|p| format!(" (pid {p})")).unwrap_or_default()
            ),
            &[],
            None,
        )),
        (i, true) => {
            let why = match (&f.svc.error, &f.svc.last_exit) {
                (Some(e), _) => format!("{} didn't answer: {e}", i.word()),
                (None, Some(x)) => format!("{} · last exit {x}", f.svc.detail),
                (None, None) => f.svc.detail.clone(),
            };
            v.push(check(
                St::Warn,
                "service",
                format!("{name} is installed but not running ({why})"),
                &[s(
                    "aleph daemon start · aleph daemon logs --service shows why it stopped",
                )],
                Some(Fix::DaemonRestart),
            ));
        }
    }
    // what the service file runs, with what
    if f.svc.installed {
        match &f.installed {
            None => v.push(check(
                St::Warn,
                "program",
                format!("couldn't read {}", tilde(&f.file)),
                &install_fix(),
                Some(Fix::DaemonInstall),
            )),
            Some(i) => {
                let rest: Vec<&str> = i.args.iter().skip(1).map(String::as_str).collect();
                let daemon_args = rest.first() == Some(&"watch")
                    || (rest.first() == Some(&"daemon") && rest.get(1) == Some(&"run"));
                match &f.prog {
                    Prog::Nothing | Prog::Unknown => v.push(check(
                        St::Bad,
                        "program",
                        "the service file runs nothing",
                        &install_fix(),
                        Some(Fix::DaemonInstall),
                    )),
                    Prog::Gone(p) => v.push(check(
                        St::Bad,
                        "program",
                        format!("the service runs {p}, which isn't there any more"),
                        &install_fix(),
                        Some(Fix::DaemonInstall),
                    )),
                    Prog::Other(p) => v.push(check(
                        St::Warn,
                        "program",
                        format!(
                            "the service runs {}; this binary is {}",
                            tilde(Path::new(p)),
                            tilde(&f.exe)
                        ),
                        &[s("aleph daemon install — points it at this binary")],
                        Some(Fix::DaemonInstall),
                    )),
                    Prog::This if !daemon_args => v.push(check(
                        St::Warn,
                        "program",
                        format!("the service runs `{}`, not the daemon", i.args.join(" ")),
                        &install_fix(),
                        Some(Fix::DaemonInstall),
                    )),
                    Prog::This => v.push(check(
                        St::Ok,
                        "program",
                        format!("this binary ({})", tilde(&f.exe)),
                        &[],
                        None,
                    )),
                }
                // the same config / state as this shell, and herdr on its PATH
                let mut diff: Vec<String> = Vec::new();
                for (k, here) in &f.env_now {
                    let there = i.env(k).map(String::from);
                    if there != *here {
                        diff.push(match (here, there) {
                            (Some(h), Some(t)) => format!("{k} is {t} there, {h} here"),
                            (Some(h), None) => format!("{k} ({h}) isn't set there"),
                            (None, Some(t)) => format!("{k} is {t} there, unset here"),
                            (None, None) => continue,
                        });
                    }
                }
                if let (Some(d), Some(p)) = (&f.herdr_dir, i.env("PATH")) {
                    if !p.split(':').any(|x| x == d) {
                        diff.push(format!("herdr's folder ({d}) isn't on its PATH"));
                    }
                }
                if diff.is_empty() {
                    v.push(check(
                        St::Ok,
                        "environment",
                        "the settings, state and PATH this shell has",
                        &[],
                        None,
                    ));
                } else {
                    v.push(check(
                        St::Warn,
                        "environment",
                        diff.join(" · "),
                        &[s(
                            "aleph daemon install — from this shell, so the service gets them",
                        )],
                        Some(Fix::DaemonInstall),
                    ));
                }
                let keeps = match f.init {
                    Init::Launchd => i.restart.is_some(),
                    _ => i.restart.as_deref().is_some_and(|r| r != "no"),
                };
                if keeps {
                    v.push(check(
                        St::Ok,
                        "restart",
                        match f.init {
                            Init::Launchd => {
                                "KeepAlive: launchd starts it again if it stops".to_string()
                            }
                            _ => format!(
                                "Restart={}: systemd starts it again if it fails",
                                i.restart.clone().unwrap_or_default()
                            ),
                        },
                        &[],
                        None,
                    ));
                } else {
                    v.push(check(
                        St::Warn,
                        "restart",
                        "nothing starts it again if it stops (no KeepAlive / Restart=)",
                        &install_fix(),
                        Some(Fix::DaemonInstall),
                    ));
                }
            }
        }
    }
    // the socket and the daemon's version
    match &f.answer {
        Ok(_) => {
            v.push(check(
                St::Ok,
                "socket",
                format!(
                    "answers on {} · pid {} · up {}",
                    tilde(&f.sock),
                    f.pid().unwrap_or(0),
                    ui::dur(f.answer_u64("uptime_s").unwrap_or(0))
                ),
                &[],
                None,
            ));
            let d = f.daemon_version().unwrap_or_default();
            if d == f.version {
                v.push(check(
                    St::Ok,
                    "version",
                    format!("aleph {d}, like this binary"),
                    &[],
                    None,
                ));
            } else {
                v.push(check(
                    St::Warn,
                    "version",
                    format!("the daemon runs aleph {d}, this binary is {}", f.version),
                    &[s("aleph daemon restart — or leave it: it re-execs when its own binary changes")],
                    Some(Fix::DaemonRestart),
                ));
            }
        }
        Err(_) if f.svc.installed && f.svc.running => v.push(check(
            St::Bad,
            "socket",
            format!(
                "the service runs, but nothing answers on {}",
                tilde(&f.sock)
            ),
            &[s("aleph daemon logs --service · aleph daemon restart")],
            Some(Fix::DaemonRestart),
        )),
        Err(_) => v.push(check(
            St::Info,
            "socket",
            format!(
                "no daemon answers on {} — while none runs, an open dashboard does its work",
                tilde(&f.sock)
            ),
            &[],
            None,
        )),
    }
    // the hub: auto-approve, the queue, notifications — exactly one process
    if !f.hub_here {
        v.push(check(
            St::Info,
            "hub",
            format!(
                "runs on {} (hub.machine) — this computer only watches",
                f.hub_setting
            ),
            &[],
            None,
        ));
    } else {
        match (f.holder, f.pid()) {
            (Some((h, true)), Some(d)) if h == d => v.push(check(
                St::Ok,
                "hub",
                format!("auto-approve, the queue and notifications run in the daemon (pid {d})"),
                &[],
                None,
            )),
            (Some((h, true)), Some(_)) => v.push(check(
                St::Info,
                "hub",
                format!(
                    "pid {h} runs it (an open dashboard?) — the daemon takes over when that exits"
                ),
                &[],
                None,
            )),
            (Some((h, true)), None) => v.push(check(
                St::Ok,
                "hub",
                format!("an open dashboard runs it (pid {h})"),
                &[],
                None,
            )),
            (_, Some(_)) => v.push(check(
                St::Warn,
                "hub",
                "nobody runs auto-approve, the queue and notifications right now",
                &[s("aleph daemon restart")],
                Some(Fix::DaemonRestart),
            )),
            (_, None) => v.push(check(
                St::Info,
                "hub",
                "not running — the daemon (or an open dashboard) runs it",
                &[],
                None,
            )),
        }
    }
    let hm = f.hub_setting.trim().to_lowercase();
    let deployed = f.deploys.iter().find(|(_, hn)| *hn == hm);
    if hm.is_empty() {
        if let Some((h, hn)) = f.deploys.first() {
            v.push(check(
                St::Warn,
                "hub.machine",
                format!("a daemon runs on {h} too (aleph daemon deploy): with hub.machine empty both run automation — double notifications, double approvals"),
                &[format!(
                    "aleph settings set hub.machine {hn} — here and on {h} (or {} to keep it here)",
                    f.this_machine
                )],
                None,
            ));
        }
    } else if let Some((h, _)) = deployed {
        v.push(check(
            St::Ok,
            "hub.machine",
            format!("{h}'s daemon runs automation ({hm}); this computer only watches"),
            &[],
            None,
        ));
    } else if hm == "local" {
        v.push(check(
            St::Warn,
            "hub.machine",
            "local means every computer that reads this config: copied to another machine, both run automation",
            &[format!("aleph settings set hub.machine {}", f.this_machine)],
            None,
        ));
    } else if !hm.is_empty() && hm != f.this_machine {
        v.push(check(
            St::Info,
            "hub.machine",
            format!(
                "{hm} runs automation, not this computer ({}) — a daemon must run there",
                f.this_machine
            ),
            &[],
            None,
        ));
    } else if !hm.is_empty() {
        v.push(check(
            St::Ok,
            "hub.machine",
            format!("this computer ({hm})"),
            &[],
            None,
        ));
    }
    // linger (systemd)
    if f.init == Init::Systemd {
        let user = service::user_name();
        match f.linger {
            Some(true) => v.push(check(
                St::Ok,
                "linger",
                "on: the daemon keeps running after you log out",
                &[],
                None,
            )),
            Some(false) => v.push(check(
                St::Warn,
                "linger",
                "off: your services, the daemon too, stop when you log out",
                &[format!(
                    "loginctl enable-linger {user}   (some systems want sudo; Aleph never runs it)"
                )],
                None,
            )),
            None => v.push(check(
                St::Info,
                "linger",
                "couldn't tell (loginctl show-user)",
                &[],
                None,
            )),
        }
    }
    // logs
    let size = |b: u64| ui::bytes(b as f64);
    match &f.svc_log {
        Some((p, b)) if *b > LOG_BIG => v.push(check(
            St::Warn,
            "logs",
            format!("{} is {}", tilde(p), size(*b)),
            &[s("aleph daemon restart — rotates it to aleph.log.1")],
            Some(Fix::DaemonRestart),
        )),
        _ => v.push(check(
            St::Ok,
            "logs",
            format!(
                "aleph.log {} (rotated at 1 MB){}",
                f.log_bytes.map(size).unwrap_or_else(|| "–".into()),
                f.svc_log
                    .as_ref()
                    .map(|(_, b)| format!(" · service output {}", size(*b)))
                    .unwrap_or_default()
            ),
            &[],
            None,
        )),
    }
    // the HTTP API: never every network unless that's meant
    if f.api.any() && !f.api.listen_any {
        v.push(check(
            St::Bad,
            "http api",
            format!("api.listen = {} listens on every network", f.api.listen),
            &[
                format!("aleph settings set api.listen {API_LISTEN} — and reach it through aleph daemon expose"),
                s("or, if every network is meant: aleph settings set api.listen_any true"),
            ],
            None,
        ));
    } else if !f.api.http {
        if f.api_answers == Some(true) {
            v.push(check(
                St::Warn,
                "http api",
                format!(
                    "off (api.http), and something else already listens on {}",
                    f.api.listen
                ),
                &[s(
                    "another port: aleph settings set api.listen 127.0.0.1:7879",
                )],
                None,
            ));
        } else {
            v.push(check(
                St::Info,
                "http api",
                format!("off (api.http) · {}", f.api.listen),
                &[],
                None,
            ));
        }
    } else {
        match (f.api_answers, f.running()) {
            (Some(true), _) => v.push(check(
                St::Ok,
                "http api",
                format!("answers on {}", f.api.listen),
                &[],
                None,
            )),
            (_, true) => v.push(check(
                St::Warn,
                "http api",
                format!("api.http is on, but nothing answers on {}", f.api.listen),
                &[s("aleph daemon restart · aleph daemon logs")],
                Some(Fix::DaemonRestart),
            )),
            (_, false) => v.push(check(
                St::Info,
                "http api",
                format!("on · {} (starts with the daemon)", f.api.listen),
                &[],
                None,
            )),
        }
    }
    exposure_checks(f, &mut v);
    v
}

/// (pure) Where the API is reachable from, and whether it's ready for that.
fn exposure_checks(f: &Facts, v: &mut Vec<Check>) {
    let e = &f.exposure;
    if !e.any() {
        v.push(check(
            St::Info,
            "exposure",
            "not exposed — aleph daemon expose tailscale | cloudflare puts the API on your phone",
            &[],
            None,
        ));
        return;
    }
    for t in &e.tailscale {
        v.push(check(
            St::Ok,
            "exposure",
            format!("tailscale {} → {} (your tailnet only)", t.url(), t.proxy),
            &[],
            None,
        ));
    }
    if let Some((h, running)) = &e.cloudflare {
        let url = format!("https://{}", h.clone().unwrap_or_else(|| "?".into()));
        if *running {
            v.push(check(
                St::Info,
                "exposure",
                format!("cloudflare {url} — public: keep Cloudflare Access in front of it"),
                &[s("Cloudflare Zero Trust › Access › Applications: a self-hosted app for it that lets only you in")],
                None,
            ));
        } else {
            v.push(check(
                St::Warn,
                "exposure",
                format!("Aleph's tunnel to {url} isn't running"),
                &[s("aleph daemon expose cloudflare — sets it up again (or: aleph daemon expose off cloudflare)")],
                None,
            ));
        }
    }
    if !f.api.http {
        v.push(check(
            St::Warn,
            "exposure",
            "exposed, but api.http is off: nothing answers there",
            &[s(
                "aleph settings set api.http on · then aleph daemon restart",
            )],
            None,
        ));
    }
    match f.tokens {
        Some(0) => v.push(check(
            St::Warn,
            "tokens",
            "exposed, but no device has a token yet",
            &[s("aleph daemon pair — gives a phone or laptop its token")],
            None,
        )),
        Some(n) => v.push(check(
            St::Ok,
            "tokens",
            format!("{n} device token{}", if n == 1 { "" } else { "s" }),
            &[],
            None,
        )),
        None => v.push(check(
            St::Info,
            "tokens",
            "exposed — check which devices hold a token: aleph daemon token ls",
            &[],
            None,
        )),
    }
    let missing: Vec<String> = e
        .names()
        .into_iter()
        .filter(|n| !f.api.hosts.iter().any(|h| h.eq_ignore_ascii_case(n)))
        .collect();
    if !missing.is_empty() {
        let mut all = f.api.hosts.clone();
        all.extend(missing.iter().cloned());
        v.push(check(
            St::Warn,
            "api.hosts",
            format!(
                "{} isn't in api.hosts: the API turns requests for it away",
                missing.join(", ")
            ),
            &[format!("aleph settings set api.hosts {}", all.join(","))],
            None,
        ));
    }
}

/// The daemon's checks, for `aleph doctor` and Settings › Daemon & API.
pub fn doctor_lines(cfg: &Config) -> Vec<Check> {
    checks(&gather(cfg))
}

/// `aleph daemon doctor [--json] [--fix]`.
fn doctor_cmd(a: &crate::cli::Args) -> ExitCode {
    let sp = Spinner::start("Checking the daemon".into());
    let lines = doctor_lines(&a.cfg);
    sp.stop();
    let n = |st: St| lines.iter().filter(|l| l.st == st).count();
    let (okc, wc, bc) = (n(St::Ok), n(St::Warn), n(St::Bad));
    let code = if bc > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    };
    if a.has("--json") {
        let word = |st: St| match st {
            St::Ok => "ok",
            St::Warn => "warn",
            St::Bad => "fail",
            _ => "info",
        };
        json(&json!({
            "ok": okc, "warn": wc, "fail": bc,
            "checks": lines.iter().map(|l| json!({"status": word(l.st), "label": l.label, "text": l.text, "fix": l.fix})).collect::<Vec<_>>(),
        }));
        return code;
    }
    let color = crate::term::color_out();
    out(doctor::head_line("Daemon", "", &doctor::Counts::of(&lines)));
    for l in doctor::section_rows(&lines, ui::print_width()) {
        println!("{}", ui::to_ansi(&l, color));
    }
    let mut fixes: Vec<Fix> = Vec::new();
    for l in &lines {
        if let Some(fx) = &l.action {
            if matches!(l.st, St::Bad | St::Warn) && !fixes.contains(fx) {
                fixes.push(fx.clone());
            }
        }
    }
    if a.has("--fix") {
        doctor::fix_all(&a.cfg, &fixes);
    } else if !fixes.is_empty() {
        hint("aleph daemon doctor --fix carries out the fixes, asking first");
    }
    code
}

// ------------------------------------------------------------------ logs

/// The `[pid]` of a log line.
fn line_pid(l: &str) -> Option<u32> {
    let w = l.split_whitespace().nth(2)?;
    w.strip_prefix('[')?.strip_suffix(']')?.parse().ok()
}

/// (pure) Processes that started a daemon (`daemon: started …`).
pub fn daemon_pids(lines: &[String]) -> HashSet<u32> {
    lines
        .iter()
        .filter(|l| l.contains(" daemon: started "))
        .filter_map(|l| line_pid(l))
        .collect()
}

/// (pure) The lines those processes wrote.
pub fn daemon_lines<'a>(lines: &'a [String], pids: &HashSet<u32>) -> Vec<&'a str> {
    lines
        .iter()
        .filter(|l| line_pid(l).is_some_and(|p| pids.contains(&p)))
        .map(String::as_str)
        .collect()
}

/// `aleph daemon logs [-f] [-n N] [--service]`.
fn logs(a: &crate::cli::Args) -> ExitCode {
    let r = a.rest;
    let n: usize = r
        .iter()
        .position(|x| x == "-n")
        .and_then(|i| r.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let follow = a.has("-f") || a.has("--follow");
    if a.has("--service") {
        return service_logs(n, follow);
    }
    let all = crate::log::tail(20_000);
    let mut pids = daemon_pids(&all);
    if let Some(p) = crate::daemon::pid() {
        pids.insert(p);
    }
    let mine = daemon_lines(&all, &pids);
    for l in &mine[mine.len().saturating_sub(n)..] {
        println!("{l}");
    }
    if !follow {
        if mine.is_empty() {
            hint(&format!(
                "no daemon lines in {} yet — aleph daemon logs --service shows what the service manager kept",
                tilde(&crate::log::path())
            ));
        } else if std::io::stderr().is_terminal() {
            hint(
                "aleph daemon logs --service: what the service manager kept (its output, crashes)",
            );
        }
        return ExitCode::SUCCESS;
    }
    let p = crate::log::path();
    let mut pos = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if len < pos {
            pos = 0; // rotated
        }
        if len > pos {
            if let Ok(t) = std::fs::read(&p) {
                let fresh: Vec<String> = String::from_utf8_lossy(&t[pos as usize..])
                    .lines()
                    .map(String::from)
                    .collect();
                pids.extend(daemon_pids(&fresh));
                for l in daemon_lines(&fresh, &pids) {
                    println!("{l}");
                }
            }
            pos = len;
        }
    }
}

/// What the service manager kept: launchd's log file, or the journal.
fn service_logs(n: usize, follow: bool) -> ExitCode {
    let init = Init::here();
    let s = spec();
    if init == Init::Systemd {
        if proc::which("journalctl").is_none() {
            return fail("no journalctl here — systemd keeps the daemon's output in the journal");
        }
        let n = n.to_string();
        let mut args = vec![
            "--user",
            "-u",
            s.unit.as_str(),
            "-n",
            n.as_str(),
            "--no-pager",
        ];
        if follow {
            // following is open-ended by design: ctrl+c ends it
            args.push("-f");
            return match std::process::Command::new("journalctl")
                .args(&args)
                .status()
            {
                Ok(st) if st.success() => ExitCode::SUCCESS,
                Ok(_) => ExitCode::from(1),
                Err(e) => fail(&format!("journalctl: {e}")),
            };
        }
        let o = service::sys("journalctl", &args);
        print!("{}", o.text());
        if !o.ok() {
            return fail(&format!("journalctl: {}", o.reason()));
        }
        return ExitCode::SUCCESS;
    }
    let p = s.log.clone();
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    for l in &lines[lines.len().saturating_sub(n)..] {
        println!("{l}");
    }
    if !follow {
        if lines.is_empty() {
            hint(&format!("{} is empty (or not there yet)", tilde(&p)));
        }
        return ExitCode::SUCCESS;
    }
    let mut pos = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if len < pos {
            pos = 0;
        }
        if len > pos {
            if let Ok(t) = std::fs::read(&p) {
                print!("{}", String::from_utf8_lossy(&t[pos as usize..]));
            }
            pos = len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A computer with nothing installed and no daemon.
    fn bare() -> Facts {
        let exe = PathBuf::from("/usr/local/bin/aleph");
        let spec = Spec {
            label: service::LABEL.into(),
            unit: service::UNIT.into(),
            description: "d".into(),
            args: vec![exe.to_string_lossy().into(), "daemon".into(), "run".into()],
            path: "/usr/bin".into(),
            env: vec![],
            log: PathBuf::from("/Users/me/Library/Logs/aleph.log"),
            by: "x".into(),
        };
        Facts {
            version: "2.0.0".into(),
            exe,
            init: Init::Launchd,
            file: PathBuf::from("/Users/me/Library/LaunchAgents/dev.aleph.daemon.plist"),
            spec,
            svc: service::State::default(),
            installed: None,
            prog: Prog::Unknown,
            sock: PathBuf::from("/Users/me/.local/state/aleph/daemon.sock"),
            answer: Err("no daemon is running".into()),
            hub_setting: String::new(),
            this_machine: "mbp".into(),
            hub_here: true,
            holder: None,
            log: PathBuf::from("/Users/me/.local/state/aleph/aleph.log"),
            log_bytes: Some(2048),
            svc_log: None,
            api: ApiCfg {
                http: false,
                listen: API_LISTEN.into(),
                listen_any: false,
                hosts: vec![],
            },
            api_answers: Some(false),
            linger: None,
            herdr_dir: Some("/opt/homebrew/bin".into()),
            env_now: vec![("ALEPH_CONFIG".into(), None), ("ALEPH_LOG".into(), None)],
            exposure: crate::expose::Exposure::default(),
            tokens: None,
            deploys: vec![],
        }
    }

    /// The daemon running from its service, holding the hub.
    fn running() -> Facts {
        let mut f = bare();
        f.svc = service::State {
            installed: true,
            loaded: true,
            running: true,
            pid: Some(4242),
            detail: "running".into(),
            ..Default::default()
        };
        f.installed = Some(service::Installed {
            args: vec!["/usr/local/bin/aleph".into(), "daemon".into(), "run".into()],
            env: vec![("PATH".into(), "/opt/homebrew/bin:/usr/bin".into())],
            restart: Some("always".into()),
        });
        f.prog = Prog::This;
        f.answer = Ok(
            json!({"version": "2.0.0", "pid": 4242, "uptime_s": 3700, "hub": true, "hub_holder": 4242}),
        );
        f.holder = Some((4242, true));
        f
    }

    fn find<'a>(v: &'a [Check], label: &str) -> &'a Check {
        v.iter()
            .find(|l| l.label == label)
            .unwrap_or_else(|| panic!("no {label} check in {v:?}"))
    }

    #[test]
    fn status_json_keeps_the_daemons_answer_on_top() {
        let v = status_json(&running());
        assert_eq!(v["running"], true);
        assert_eq!(v["hub"], true);
        assert_eq!(v["pid"], 4242);
        assert_eq!(v["version_match"], true);
        assert_eq!(v["binary"]["version"], "2.0.0");
        assert_eq!(v["service"]["manager"], "launchd");
        assert_eq!(v["service"]["name"], "dev.aleph.daemon");
        assert_eq!(v["service"]["running"], true);
        assert_eq!(v["service"]["program"][1], "daemon");
        assert_eq!(v["hub_machine"]["holder"], 4242);
        assert_eq!(v["api"]["listen"], "127.0.0.1:7878");
        assert_eq!(v["api"]["http"], false);
        let v = status_json(&bare());
        assert_eq!(v["running"], false);
        assert!(v["error"].as_str().unwrap().contains("no daemon"));
        assert_eq!(v["version_match"], Value::Null);
        assert_eq!(v["service"]["installed"], false);
        let mut f = running();
        f.answer = Ok(json!({"version": "1.9.0", "pid": 4242}));
        assert_eq!(status_json(&f)["version_match"], false);
    }

    #[test]
    fn status_rows_say_what_matters() {
        let rows = status_rows(&bare());
        let get = |rows: &[(&str, String, Color)], k: &str| {
            rows.iter().find(|r| r.0 == k).map(|r| r.1.clone()).unwrap()
        };
        assert!(get(&rows, "service").contains("aleph daemon install"));
        assert!(get(&rows, "socket").contains("nothing answers"));
        assert!(get(&rows, "http api").starts_with("off (api.http)"));
        let rows = status_rows(&running());
        assert!(get(&rows, "service").contains("running (pid 4242)"));
        assert!(get(&rows, "hub").contains("in the daemon (pid 4242)"));
        assert!(get(&rows, "version").contains("this binary too"));
        let mut f = running();
        f.answer = Ok(json!({"version": "1.9.0", "pid": 4242}));
        assert!(get(&status_rows(&f), "version").contains("runs 1.9.0, this binary is 2.0.0"));
    }

    #[test]
    fn a_bare_computer_has_no_problems_only_notes() {
        let v = checks(&bare());
        assert!(
            v.iter().all(|l| l.st != St::Bad && l.st != St::Warn),
            "{v:?}"
        );
        assert_eq!(find(&v, "service").action, Some(Fix::DaemonInstall));
    }

    #[test]
    fn a_healthy_daemon_passes() {
        let v = checks(&running());
        for l in [
            "service",
            "program",
            "environment",
            "restart",
            "socket",
            "version",
            "hub",
            "logs",
        ] {
            assert_eq!(find(&v, l).st, St::Ok, "{l}: {:?}", find(&v, l));
        }
        assert!(
            v.iter().all(|l| l.st != St::Bad && l.st != St::Warn),
            "{v:?}"
        );
    }

    #[test]
    fn the_service_file_must_run_this_binary_with_this_env() {
        let mut f = running();
        f.prog = Prog::Gone("/nowhere/aleph".into());
        assert_eq!(find(&checks(&f), "program").st, St::Bad);
        f.prog = Prog::Other("/opt/old/aleph".into());
        let c = find(&checks(&f), "program").clone();
        assert_eq!((c.st, c.action), (St::Warn, Some(Fix::DaemonInstall)));
        let mut f = running();
        f.env_now = vec![("ALEPH_CONFIG".into(), Some("/tmp/my.toml".into()))];
        let e = find(&checks(&f), "environment").clone();
        assert_eq!(e.st, St::Warn);
        assert!(
            e.text
                .contains("ALEPH_CONFIG (/tmp/my.toml) isn't set there"),
            "{}",
            e.text
        );
        let mut f = running();
        f.herdr_dir = Some("/home/me/.cargo/bin".into());
        assert!(find(&checks(&f), "environment")
            .text
            .contains("herdr's folder"));
        let mut f = running();
        f.installed.as_mut().unwrap().restart = None;
        assert_eq!(find(&checks(&f), "restart").st, St::Warn);
        // the old `aleph watch` service is still the daemon; something else isn't
        let mut f = running();
        f.installed.as_mut().unwrap().args = vec!["/usr/local/bin/aleph".into(), "watch".into()];
        assert_eq!(find(&checks(&f), "program").st, St::Ok);
        f.installed.as_mut().unwrap().args = vec!["/usr/local/bin/aleph".into(), "ls".into()];
        assert_eq!(find(&checks(&f), "program").st, St::Warn);
    }

    #[test]
    fn socket_version_and_hub() {
        // the service runs, the socket doesn't answer
        let mut f = running();
        f.answer = Err("refused".into());
        assert_eq!(find(&checks(&f), "socket").st, St::Bad);
        // another version
        let mut f = running();
        f.answer = Ok(json!({"version": "1.9.0", "pid": 4242, "uptime_s": 5}));
        let c = find(&checks(&f), "version").clone();
        assert_eq!(c.st, St::Warn);
        assert!(c.fix[0].contains("re-execs"));
        // a dashboard holds the hub
        let mut f = running();
        f.holder = Some((777, true));
        assert_eq!(find(&checks(&f), "hub").st, St::Info);
        // nobody holds it while the daemon runs
        let mut f = running();
        f.holder = Some((777, false));
        assert_eq!(find(&checks(&f), "hub").st, St::Warn);
        // another computer runs it
        let mut f = running();
        f.hub_here = false;
        f.hub_setting = "b31".into();
        assert_eq!(find(&checks(&f), "hub").st, St::Info);
        assert!(find(&checks(&f), "hub.machine")
            .text
            .contains("b31 runs automation"));
        // "local" is every computer
        let mut f = running();
        f.hub_setting = "local".into();
        assert_eq!(find(&checks(&f), "hub.machine").st, St::Warn);
        // a daemon deployed elsewhere, hub.machine empty: two hubs
        let mut f = running();
        f.deploys = vec![("b31".into(), "b31".into())];
        let c = find(&checks(&f), "hub.machine").clone();
        assert_eq!(c.st, St::Warn);
        assert!(c.fix[0].starts_with("aleph settings set hub.machine b31"));
        // … and set to it: fine
        f.hub_setting = "b31".into();
        f.hub_here = false;
        assert_eq!(find(&checks(&f), "hub.machine").st, St::Ok);
        assert_eq!(status_json(&f)["deploys"][0]["host"], "b31");
    }

    #[test]
    fn linger_logs_and_the_api() {
        let mut f = running();
        f.init = Init::Systemd;
        f.installed.as_mut().unwrap().restart = Some("on-failure".into());
        f.linger = Some(false);
        let l = find(&checks(&f), "linger").clone();
        assert_eq!(l.st, St::Warn);
        assert!(l.fix[0].starts_with("loginctl enable-linger"));
        assert_eq!(find(&checks(&f), "restart").st, St::Ok);
        let mut f = running();
        f.svc_log = Some((
            PathBuf::from("/Users/me/Library/Logs/aleph.log"),
            30 * 1024 * 1024,
        ));
        assert_eq!(find(&checks(&f), "logs").st, St::Warn);
        // every network, not meant
        let mut f = running();
        f.api.listen = "0.0.0.0:7878".into();
        assert_eq!(find(&checks(&f), "http api").st, St::Bad);
        f.api.listen_any = true;
        assert_ne!(find(&checks(&f), "http api").st, St::Bad);
        // on, but nothing answers while the daemon runs
        let mut f = running();
        f.api.http = true;
        assert_eq!(find(&checks(&f), "http api").st, St::Warn);
        f.api_answers = Some(true);
        assert_eq!(find(&checks(&f), "http api").st, St::Ok);
        // off, but the port is taken by someone else
        let mut f = bare();
        f.api_answers = Some(true);
        assert_eq!(find(&checks(&f), "http api").st, St::Warn);
    }

    #[test]
    fn exposure_wants_the_api_on_tokens_and_its_names() {
        let ts = crate::expose::Serve {
            host: "mbp.tail1234.ts.net".into(),
            https_port: 443,
            path: "/".into(),
            proxy: "http://127.0.0.1:7878".into(),
        };
        let mut f = running();
        assert_eq!(find(&checks(&f), "exposure").st, St::Info);
        f.exposure.tailscale.push(ts);
        let v = checks(&f);
        // api.http off, tokens unknown, the name not in api.hosts
        assert!(v.iter().any(|l| l.label == "exposure"
            && l.st == St::Warn
            && l.text.contains("api.http is off")));
        assert_eq!(find(&v, "tokens").st, St::Info);
        let h = find(&v, "api.hosts");
        assert_eq!(h.st, St::Warn);
        assert_eq!(h.fix[0], "aleph settings set api.hosts mbp.tail1234.ts.net");
        // all set, but no device has a token
        f.api.http = true;
        f.api_answers = Some(true);
        f.api.hosts = vec!["mbp.tail1234.ts.net".into()];
        f.tokens = Some(0);
        let v = checks(&f);
        assert_eq!(find(&v, "tokens").st, St::Warn);
        assert!(v.iter().all(|l| l.label != "api.hosts"));
        f.tokens = Some(2);
        assert!(
            checks(&f)
                .iter()
                .all(|l| l.st != St::Warn && l.st != St::Bad),
            "{:?}",
            checks(&f)
        );
        // a public tunnel: the Access reminder; stopped: a warning
        f.exposure.cloudflare = Some((Some("aleph.example.com".into()), true));
        f.api.hosts.push("aleph.example.com".into());
        assert!(checks(&f)
            .iter()
            .any(|l| l.text.contains("Cloudflare Access")));
        f.exposure.cloudflare = Some((Some("aleph.example.com".into()), false));
        assert!(checks(&f)
            .iter()
            .any(|l| l.st == St::Warn && l.text.contains("isn't running")));
        let j = status_json(&f);
        assert_eq!(
            j["exposure"]["tailscale"][0]["url"],
            "https://mbp.tail1234.ts.net"
        );
        assert_eq!(j["api"]["tokens"], 2);
        assert!(status_rows(&f)
            .iter()
            .any(|r| r.0 == "exposure" && r.1.contains("aleph.example.com")));
    }

    #[test]
    fn listen_addresses() {
        assert_eq!(
            split_listen("127.0.0.1:7878"),
            Some(("127.0.0.1".into(), 7878))
        );
        assert_eq!(split_listen("[::1]:9000"), Some(("::1".into(), 9000)));
        assert_eq!(split_listen("7878"), Some(("127.0.0.1".into(), 7878)));
        assert_eq!(split_listen(":7878"), Some(("".into(), 7878)));
        assert_eq!(split_listen("nope"), None);
        let a = |l: &str| ApiCfg {
            http: true,
            listen: l.into(),
            listen_any: false,
            hosts: vec![],
        };
        assert!(a("0.0.0.0:7878").any() && a("[::]:7878").any() && a(":7878").any());
        assert!(!a("127.0.0.1:7878").any() && !a("100.64.0.5:7878").any());
        assert_eq!(a("[::1]:9000").local_url(), "http://127.0.0.1:9000");
        assert_eq!(a("nonsense").port(), 7878);
        let cfg = Config::in_memory(&["api.http=on", "api.hosts=a.ts.net, b.example.com"]);
        let c = ApiCfg::from(&cfg);
        assert!(c.http && c.listen == API_LISTEN);
        assert_eq!(c.hosts, vec!["a.ts.net", "b.example.com"]);
    }

    #[test]
    fn the_daemons_own_log_lines() {
        let lines: Vec<String> = [
            "2026-10-22T06:40:01.123Z INFO  [100] daemon: started 2.0.0 — watching b31",
            "2026-10-22T06:40:02.000Z WARN  [200] dashboard: a newer build is installed",
            "2026-10-22T06:40:03.000Z INFO  [100] router: sent to ntfy",
            "garbage",
            "2026-10-22T06:41:00.000Z INFO  [300] daemon: started 2.0.0 — watching b31, gpu2",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let pids = daemon_pids(&lines);
        assert_eq!(pids, [100, 300].into_iter().collect());
        let mine = daemon_lines(&lines, &pids);
        assert_eq!(mine.len(), 3);
        assert!(mine[1].contains("router: sent"));
    }
}
