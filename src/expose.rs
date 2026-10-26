//! `aleph daemon expose tailscale | cloudflare | off [tailscale | cloudflare] | status`
//!
//! Puts the daemon's HTTP API (`api.listen`, 127.0.0.1:7878) where a phone
//! can reach it without opening a port: detect → plan → confirm → run.
//!
//! * **tailscale**: `tailscale serve --bg --https=443 http://127.0.0.1:7878`
//!   (the `serve` CLI of tailscale 1.52 and later), so only devices on your
//!   tailnet reach `https://<this machine>.<tailnet>.ts.net`. `off` removes
//!   only that entry (`tailscale serve --https=443 off`), never
//!   `tailscale serve reset`.
//! * **cloudflare**: a named tunnel (`cloudflared tunnel create aleph`), a
//!   DNS route for your hostname, `~/.cloudflared/aleph.yml` with one ingress
//!   rule, and a launchd / systemd unit of our own that runs it
//!   (`cloudflared service install` would want sudo). The URL is public, so
//!   put Cloudflare Access in front of it. Quick tunnels (trycloudflare.com)
//!   are refused: anyone who finds the URL would reach an API whose control
//!   scope sends prompts and approves commands.
//!
//! Every step asks first (`--yes` doesn't), `--dry-run` only prints. The
//! names go into `api.hosts` (the names the API answers to), and turning on
//! `api.http` is offered. What was set up is kept in `state/expose.json`, so
//! `off` takes back only what `expose` added. Detection and planning are pure
//! functions of what the tools said (tested); the tools themselves only run
//! through `service::sys_t`, never from a unit test.

use crate::config::{home, state_dir, Config};
use crate::daemonctl::ApiCfg;
use crate::doctor::tilde;
use crate::out::*;
use crate::proc::{self, sh_quote};
use crate::service::{self, Init, Op, Spec};
use crate::ui;
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// The oldest tailscale whose `serve` takes `--bg --https=N <target>`.
pub const TS_MIN: &str = "1.52.0";
const T_TOOL: Duration = Duration::from_secs(15);
/// `tailscale serve` may wait while you approve HTTPS certificates.
const T_SERVE: Duration = Duration::from_secs(300);
pub const CF_LABEL: &str = "dev.aleph.cloudflared";
pub const CF_UNIT: &str = "aleph-cloudflared.service";

// ------------------------------------------------------------------ tailscale

/// What tailscale says about this machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ts {
    /// the CLI
    pub bin: String,
    /// `1.76.6`
    pub version: Option<String>,
    /// `BackendState`: `Running`, `NeedsLogin`, `Stopped`, …
    pub backend: String,
    /// this machine's MagicDNS name, without the trailing dot
    pub dns_name: String,
    /// HTTPS certificates are on for the tailnet (None: can't tell)
    pub https: Option<bool>,
    /// what `tailscale serve` already serves
    pub serve: Vec<Serve>,
    /// ports `tailscale serve` uses for raw TCP
    pub tcp_ports: Vec<u16>,
}

/// One web handler of `tailscale serve`.
#[derive(Clone, Debug, PartialEq)]
pub struct Serve {
    /// the MagicDNS name it's for
    pub host: String,
    pub https_port: u16,
    pub path: String,
    pub proxy: String,
}

impl Serve {
    pub fn url(&self) -> String {
        let port = if self.https_port == 443 {
            String::new()
        } else {
            format!(":{}", self.https_port)
        };
        let path = if self.path == "/" { "" } else { &self.path };
        format!("https://{}{port}{path}", self.host)
    }
}

/// (pure) The version from `tailscale version` (first line, `1.76.6`).
pub fn parse_ts_version(text: &str) -> Option<String> {
    let l = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let v: String = l
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (v.contains('.') && v.starts_with(|c: char| c.is_ascii_digit())).then_some(v)
}

/// (pure) Does this tailscale have the 1.52+ `serve` CLI?
pub fn ts_version_ok(v: &str) -> bool {
    crate::update::cmp_versions(v, TS_MIN) != std::cmp::Ordering::Less
}

/// (pure) `tailscale status --json` → (backend, DNS name, HTTPS certs on).
pub fn parse_ts_status(v: &Value) -> (String, String, Option<bool>) {
    let backend = v
        .get("BackendState")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let me = v.get("Self");
    let dns = me
        .and_then(|s| s.get("DNSName"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_end_matches('.')
        .to_lowercase();
    let in_caps = me
        .and_then(|s| s.get("Capabilities"))
        .and_then(Value::as_array)
        .map(|a| a.iter().any(|c| c.as_str() == Some("https")));
    let in_map = me
        .and_then(|s| s.get("CapMap"))
        .and_then(Value::as_object)
        .map(|m| m.contains_key("https"));
    let https = match (in_caps, in_map) {
        (None, None) => None,
        (a, b) => Some(a == Some(true) || b == Some(true)),
    };
    (backend, dns, https)
}

/// (pure) `tailscale serve status --json` → web handlers and raw TCP ports.
pub fn parse_serve_status(v: &Value) -> (Vec<Serve>, Vec<u16>) {
    let mut web = Vec::new();
    if let Some(m) = v.get("Web").and_then(Value::as_object) {
        for (hp, conf) in m {
            let Some((host, port)) = hp.rsplit_once(':') else {
                continue;
            };
            let Ok(port) = port.parse::<u16>() else {
                continue;
            };
            let Some(hs) = conf.get("Handlers").and_then(Value::as_object) else {
                continue;
            };
            for (path, h) in hs {
                let proxy = h
                    .get("Proxy")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                web.push(Serve {
                    host: host.to_lowercase(),
                    https_port: port,
                    path: path.clone(),
                    proxy,
                });
            }
        }
    }
    let mut tcp = Vec::new();
    if let Some(m) = v.get("TCP").and_then(Value::as_object) {
        for (p, h) in m {
            let raw = h.get("TCPForward").is_some()
                || h.get("TerminateTLS").is_some()
                || (h.get("HTTPS").is_none() && h.get("HTTP").is_none());
            if let (Ok(p), true) = (p.parse::<u16>(), raw) {
                tcp.push(p);
            }
        }
    }
    web.sort_by(|a, b| (a.https_port, &a.path).cmp(&(b.https_port, &b.path)));
    (web, tcp)
}

/// (pure) Does a proxy target point at `port` on this computer?
pub fn same_target(proxy: &str, port: u16) -> bool {
    let p = proxy
        .trim()
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if let Ok(n) = p.parse::<u16>() {
        return n == port;
    }
    crate::daemonctl::split_listen(p).is_some_and(|(h, n)| {
        n == port && matches!(h.as_str(), "127.0.0.1" | "localhost" | "::1" | "0.0.0.0")
    })
}

/// What an expose would do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// what was found, a line each
    pub found: Vec<String>,
    /// commands, in order
    pub run: Vec<Vec<String>>,
    /// `api.hosts` afterwards (None: already has the name)
    pub hosts: Option<String>,
    pub url: String,
    /// worth saying before asking
    pub notes: Vec<String>,
    /// it's exposed like this already
    pub done: bool,
}

/// (pure) The plan for tailscale. `port`: the API's (api.listen);
/// `https_port`: where on the tailnet name; `hosts`: `api.hosts` now.
pub fn ts_plan(t: &Ts, port: u16, https_port: u16, hosts: &[String]) -> Result<Plan, String> {
    let Some(v) = t.version.as_deref() else {
        return Err("couldn't read tailscale's version (tailscale version)".into());
    };
    if !ts_version_ok(v) {
        return Err(format!(
            "tailscale {v} is older than {TS_MIN}, whose `tailscale serve` Aleph uses — update tailscale first"
        ));
    }
    if t.backend != "Running" {
        return Err(format!(
            "tailscale isn't connected here ({}) — tailscale up",
            if t.backend.is_empty() {
                "no state"
            } else {
                &t.backend
            }
        ));
    }
    if t.dns_name.is_empty() {
        return Err(
            "this machine has no MagicDNS name — turn MagicDNS on (Tailscale admin console › DNS)"
                .into(),
        );
    }
    let here = |s: &&Serve| s.host == t.dns_name && s.https_port == https_port && s.path == "/";
    let ours = t
        .serve
        .iter()
        .filter(here)
        .find(|s| same_target(&s.proxy, port));
    if let Some(other) = t
        .serve
        .iter()
        .filter(here)
        .find(|s| !same_target(&s.proxy, port))
    {
        return Err(format!(
            "https port {https_port} already serves {} there — pick another: --https-port 8443",
            other.proxy
        ));
    }
    if t.tcp_ports.contains(&https_port) {
        return Err(format!(
            "tailscale serve already forwards raw TCP on port {https_port} — pick another: --https-port 8443"
        ));
    }
    let url = Serve {
        host: t.dns_name.clone(),
        https_port,
        path: "/".into(),
        proxy: String::new(),
    }
    .url();
    let mut p = Plan {
        found: vec![format!(
            "tailscale {v} · {} · HTTPS certificates {}",
            t.dns_name,
            match t.https {
                Some(true) => "on",
                Some(false) => "off",
                None => "?",
            }
        )],
        url,
        done: ours.is_some(),
        ..Default::default()
    };
    if ours.is_none() {
        p.run.push(vec![
            t.bin.clone(),
            "serve".into(),
            "--bg".into(),
            format!("--https={https_port}"),
            format!("http://127.0.0.1:{port}"),
        ]);
    }
    if !hosts.iter().any(|h| h.eq_ignore_ascii_case(&t.dns_name)) {
        let mut h = hosts.to_vec();
        h.push(t.dns_name.clone());
        p.hosts = Some(h.join(","));
    }
    if t.https == Some(false) {
        p.notes.push("HTTPS certificates are off for this tailnet: tailscale prints a link to turn them on — open it, approve, and it carries on".into());
    }
    Ok(p)
}

/// (pure) The commands that take back exactly our entries (those that
/// proxy to `port`), never `tailscale serve reset`.
pub fn ts_off(t: &Ts, port: u16) -> Vec<Vec<String>> {
    t.serve
        .iter()
        .filter(|s| s.host == t.dns_name && same_target(&s.proxy, port))
        .map(|s| {
            let mut c = vec![
                t.bin.clone(),
                "serve".into(),
                format!("--https={}", s.https_port),
            ];
            if s.path != "/" {
                c.push(format!("--set-path={}", s.path));
            }
            c.push("off".into());
            c
        })
        .collect()
}

/// The tailscale CLI: on the PATH, or inside the macOS app.
pub fn ts_bin() -> Option<String> {
    if let Some(p) = proc::which("tailscale") {
        return Some(p.to_string_lossy().into_owned());
    }
    let app = Path::new("/Applications/Tailscale.app/Contents/MacOS/Tailscale");
    (cfg!(target_os = "macos") && app.exists()).then(|| app.to_string_lossy().into_owned())
}

fn tool(bin: &str, args: &[&str]) -> proc::Out {
    service::sys_t(bin, args, T_TOOL)
}

/// Ask tailscale (None: it isn't installed here).
pub fn ts_detect() -> Option<Result<Ts, String>> {
    let bin = ts_bin()?;
    let ver = tool(&bin, &["version"]);
    let mut t = Ts {
        bin: bin.clone(),
        version: parse_ts_version(&ver.text()),
        ..Default::default()
    };
    let st = tool(&bin, &["status", "--json"]);
    match serde_json::from_slice::<Value>(&st.stdout) {
        Ok(v) => {
            let (b, d, h) = parse_ts_status(&v);
            t.backend = b;
            t.dns_name = d;
            t.https = h;
        }
        Err(_) => {
            return Some(Err(format!(
                "tailscale status: {}",
                if st.ok() {
                    "not JSON".to_string()
                } else {
                    st.reason()
                }
            )))
        }
    }
    let sv = tool(&bin, &["serve", "status", "--json"]);
    if let Ok(v) = serde_json::from_slice::<Value>(&sv.stdout) {
        let (w, tcp) = parse_serve_status(&v);
        t.serve = w;
        t.tcp_ports = tcp;
    }
    Some(Ok(t))
}

// ------------------------------------------------------------------ cloudflare

/// What cloudflared says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cf {
    pub bin: String,
    pub version: Option<String>,
    /// `~/.cloudflared`
    pub dir: PathBuf,
    /// logged in (`cert.pem` from `cloudflared tunnel login`)
    pub cert: bool,
    /// (id, name)
    pub tunnels: Vec<(String, String)>,
    /// `tunnel list` failed
    pub error: Option<String>,
}

/// (pure) `cloudflared --version` → `2025.8.1`.
pub fn parse_cf_version(text: &str) -> Option<String> {
    let l = text.lines().next()?;
    let w = l
        .split_whitespace()
        .skip_while(|w| *w != "version")
        .nth(1)?;
    Some(w.to_string())
}

/// (pure) `cloudflared tunnel list --output json` → (id, name).
pub fn parse_tunnels(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter(|t| {
                    t.get("deleted_at")
                        .and_then(Value::as_str)
                        .is_none_or(|d| d.starts_with("0001-"))
                })
                .filter_map(|t| {
                    Some((
                        t.get("id")?.as_str()?.to_string(),
                        t.get("name")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// (pure) The id in `cloudflared tunnel create`'s answer.
pub fn parse_created(text: &str) -> Option<String> {
    let i = text.find(" with id ")?;
    let id: String = text[i + 9..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
        .collect();
    (id.len() >= 32).then_some(id)
}

/// (pure) A DNS name we can route (`aleph.example.com`).
pub fn valid_hostname(h: &str) -> bool {
    let h = h.trim();
    h.contains('.')
        && !h.starts_with('.')
        && !h.ends_with('.')
        && h.len() <= 253
        && h.split('.')
            .all(|p| !p.is_empty() && !p.starts_with('-') && !p.ends_with('-'))
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// (pure) `~/.cloudflared/aleph.yml`.
pub fn cf_config(id: &str, cred: &Path, hostname: &str, port: u16) -> String {
    format!(
        "# managed by aleph: aleph daemon expose cloudflare / off\n\
         tunnel: {id}\n\
         credentials-file: {}\n\
         ingress:\n\
         \x20 - hostname: {hostname}\n\
         \x20   service: http://127.0.0.1:{port}\n\
         \x20 - service: http_status:404\n",
        cred.display()
    )
}

/// (pure) The hostname in one of our configs.
pub fn cf_config_hostname(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("- hostname:"))
        .map(|h| h.trim().to_string())
}

/// The unit that keeps the tunnel up: launchd / systemd, like the daemon's.
pub fn cf_spec(bin: &str, config: &Path, tunnel: &str) -> Spec {
    Spec {
        label: CF_LABEL.into(),
        unit: CF_UNIT.into(),
        description: "Aleph: the Cloudflare tunnel to the daemon's API".into(),
        args: vec![
            bin.to_string(),
            "tunnel".into(),
            "--config".into(),
            config.to_string_lossy().into_owned(),
            "run".into(),
            tunnel.to_string(),
        ],
        path: service::service_path(&std::env::var("PATH").unwrap_or_default()),
        env: vec![],
        log: home().join("Library/Logs/aleph-cloudflared.log"),
        by: "aleph daemon expose cloudflare / off".into(),
    }
}

/// What a cloudflare expose would do.
#[derive(Clone, Debug, PartialEq)]
pub struct CfPlan {
    pub tunnel: String,
    /// known when the tunnel exists already
    pub id: Option<String>,
    /// `cloudflared tunnel create <name>` when it doesn't
    pub create: Option<Vec<String>>,
    pub route: Vec<String>,
    pub config_path: PathBuf,
    /// with the id, or `<tunnel id>` until it's created
    pub config: String,
    pub unit: Spec,
    pub hosts: Option<String>,
    pub url: String,
    pub found: Vec<String>,
}

/// (pure) The plan for cloudflare.
pub fn cf_plan(
    c: &Cf,
    tunnel: &str,
    hostname: &str,
    port: u16,
    hosts: &[String],
) -> Result<CfPlan, String> {
    let Some(v) = &c.version else {
        return Err("couldn't read cloudflared's version (cloudflared --version)".into());
    };
    if !c.cert {
        return Err(format!(
            "cloudflared isn't logged in to your Cloudflare account — cloudflared tunnel login (it opens a browser: pick the zone {} is in)",
            hostname.split_once('.').map_or(hostname, |x| x.1)
        ));
    }
    if let Some(e) = &c.error {
        return Err(format!("cloudflared tunnel list: {e}"));
    }
    if !valid_hostname(hostname) {
        return Err(format!(
            "{hostname} isn't a hostname — a name in your Cloudflare zone, like aleph.example.com"
        ));
    }
    let id = c
        .tunnels
        .iter()
        .find(|(_, n)| n == tunnel)
        .map(|(i, _)| i.clone());
    let create = id.is_none().then(|| {
        vec![
            c.bin.clone(),
            "tunnel".into(),
            "create".into(),
            tunnel.into(),
        ]
    });
    let id_text = id.clone().unwrap_or_else(|| "<tunnel id>".into());
    let config_path = c.dir.join(format!("{tunnel}.yml"));
    let cred = c.dir.join(format!("{id_text}.json"));
    let hosts = (!hosts.iter().any(|h| h.eq_ignore_ascii_case(hostname))).then(|| {
        let mut h = hosts.to_vec();
        h.push(hostname.to_string());
        h.join(",")
    });
    Ok(CfPlan {
        tunnel: tunnel.into(),
        create,
        route: vec![
            c.bin.clone(),
            "tunnel".into(),
            "route".into(),
            "dns".into(),
            tunnel.into(),
            hostname.into(),
        ],
        config: cf_config(&id_text, &cred, hostname, port),
        unit: cf_spec(&c.bin, &config_path, tunnel),
        config_path,
        hosts,
        url: format!("https://{hostname}"),
        found: vec![format!(
            "cloudflared {v} · logged in · tunnel {tunnel}: {}",
            match &id {
                Some(i) => format!("exists ({})", &i[..8.min(i.len())]),
                None => "to be created".into(),
            }
        )],
        id,
    })
}

/// Ask cloudflared (None: it isn't installed here).
pub fn cf_detect() -> Option<Cf> {
    let bin = proc::which("cloudflared")?.to_string_lossy().into_owned();
    let dir = home().join(".cloudflared");
    let ver = tool(&bin, &["--version"]);
    let mut c = Cf {
        version: parse_cf_version(&format!("{}{}", ver.text(), ver.stderr)),
        cert: dir.join("cert.pem").exists()
            || std::env::var_os("TUNNEL_ORIGIN_CERT").is_some_and(|p| Path::new(&p).exists()),
        dir,
        bin: bin.clone(),
        ..Default::default()
    };
    if c.cert {
        let o = tool(&bin, &["tunnel", "list", "--output", "json"]);
        match serde_json::from_slice::<Value>(&o.stdout) {
            Ok(v) if o.ok() => c.tunnels = parse_tunnels(&v),
            _ => c.error = Some(o.reason()),
        }
    }
    Some(c)
}

// ------------------------------------------------------------------ what's exposed

/// The API's exposure right now (status, the doctor).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exposure {
    /// our tailscale serve entries
    pub tailscale: Vec<Serve>,
    /// our tunnel unit: (hostname, running)
    pub cloudflare: Option<(Option<String>, bool)>,
}

impl Exposure {
    pub fn any(&self) -> bool {
        !self.tailscale.is_empty() || self.cloudflare.is_some()
    }
    /// Every name it's reachable under.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tailscale.iter().map(|s| s.host.clone()).collect();
        if let Some((Some(h), _)) = &self.cloudflare {
            v.push(h.clone());
        }
        v.dedup();
        v
    }
    pub fn urls(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tailscale.iter().map(Serve::url).collect();
        if let Some((h, _)) = &self.cloudflare {
            v.push(format!(
                "https://{}",
                h.clone().unwrap_or_else(|| "?".into())
            ));
        }
        v
    }
    pub fn to_json(&self) -> Value {
        json!({
            "tailscale": self.tailscale.iter().map(|s| json!({"url": s.url(), "host": s.host, "https_port": s.https_port, "path": s.path, "proxy": s.proxy})).collect::<Vec<_>>(),
            "cloudflare": self.cloudflare.as_ref().map(|(h, r)| json!({"hostname": h, "url": h.as_ref().map(|h| format!("https://{h}")), "running": r})),
        })
    }
}

/// Look (cheap: one `tailscale serve status`, our unit's state).
pub fn current(cfg: &Config) -> Exposure {
    let port = ApiCfg::from(cfg).port();
    let mut e = Exposure::default();
    if let Some(Ok(t)) = ts_bin().map(|bin| -> Result<Ts, String> {
        let sv = tool(&bin, &["serve", "status", "--json"]);
        let v: Value = serde_json::from_slice(&sv.stdout).map_err(|e| e.to_string())?;
        let (w, _) = parse_serve_status(&v);
        Ok(Ts {
            bin,
            serve: w,
            ..Default::default()
        })
    }) {
        e.tailscale = t
            .serve
            .into_iter()
            .filter(|s| same_target(&s.proxy, port))
            .collect();
    }
    let init = Init::here();
    let unit = cf_spec("cloudflared", Path::new(""), "aleph");
    if unit.file(init).exists() {
        let st = service::state(init, &unit);
        let host = service::installed(init, &unit)
            .and_then(|i| {
                let cfgp = i
                    .args
                    .iter()
                    .position(|a| a == "--config")
                    .and_then(|k| i.args.get(k + 1).cloned())?;
                std::fs::read_to_string(cfgp).ok()
            })
            .and_then(|t| cf_config_hostname(&t));
        e.cloudflare = Some((host, st.running));
    }
    e
}

// ------------------------------------------------------------------ state/expose.json

fn record_path() -> PathBuf {
    state_dir().join("expose.json")
}

fn record() -> Value {
    std::fs::read_to_string(record_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn save_record(v: &Value) {
    let _ = crate::config::write_atomic(&record_path(), v.to_string().as_bytes());
}

// ------------------------------------------------------------------ the command

struct Opts {
    yes: bool,
    dry: bool,
}

/// `aleph daemon expose …` (`a.rest` starts with `expose`).
pub fn cmd(a: crate::cli::Args) -> ExitCode {
    let p = a.positional();
    let val = |f: &str| {
        a.rest
            .iter()
            .position(|x| x == f)
            .and_then(|i| a.rest.get(i + 1))
            .cloned()
            .or_else(|| {
                a.rest
                    .iter()
                    .find_map(|x| x.strip_prefix(&format!("{f}=")).map(String::from))
            })
    };
    let o = Opts {
        yes: a.has("--yes") || a.has("-y"),
        dry: a.has("--dry-run"),
    };
    // positional words after `expose` (flag values aren't among them)
    let flag_vals: Vec<String> = ["--https-port", "--hostname", "--tunnel"]
        .iter()
        .filter_map(|f| val(f))
        .collect();
    let words: Vec<&str> = p
        .iter()
        .skip(1)
        .copied()
        .filter(|w| !flag_vals.iter().any(|v| v == w))
        .collect();
    match words.first().copied() {
        Some("tailscale") => {
            let https = match val("--https-port").map(|v| v.parse::<u16>()) {
                None => 443,
                Some(Ok(n)) => n,
                Some(Err(_)) => return fail("--https-port takes a port number (443, 8443, 10000)"),
            };
            tailscale(&a.cfg, https, &o)
        }
        Some("cloudflare") | Some("cloudflared") => {
            if a.has("--quick") {
                return refuse_quick();
            }
            let tunnel = val("--tunnel").unwrap_or_else(|| "aleph".into());
            cloudflare(&a.cfg, val("--hostname"), &tunnel, &o)
        }
        Some("off") => off(&a.cfg, words.get(1).copied(), &o),
        None | Some("status") => {
            print_current(&current(&a.cfg));
            ExitCode::SUCCESS
        }
        Some(x) => {
            err(&format!(
                "aleph daemon expose {x}: tailscale, cloudflare, off or status"
            ));
            ExitCode::from(2)
        }
    }
}

fn refuse_quick() -> ExitCode {
    err("quick tunnels (trycloudflare.com) aren't offered: anyone who finds the URL reaches the API, and its control scope sends prompts and approves commands");
    hint("a named tunnel with Cloudflare Access in front: aleph daemon expose cloudflare --hostname aleph.example.com");
    hint("or only your own devices: aleph daemon expose tailscale");
    ExitCode::from(2)
}

fn print_current(e: &Exposure) {
    if !e.any() {
        say(Line::from(Span::styled(
            "  not exposed — aleph daemon expose tailscale | cloudflare",
            ui::muted(),
        )));
        return;
    }
    for s in &e.tailscale {
        out(Line::from(vec![
            Span::styled("  tailscale   ", ui::faint()),
            Span::styled(s.url(), ui::bold()),
            Span::styled(format!(" → {}", s.proxy), ui::muted()),
        ]));
    }
    if let Some((h, running)) = &e.cloudflare {
        out(Line::from(vec![
            Span::styled("  cloudflare  ", ui::faint()),
            Span::styled(
                format!("https://{}", h.clone().unwrap_or_else(|| "?".into())),
                ui::bold(),
            ),
            Span::styled(
                if *running {
                    "  tunnel running"
                } else {
                    "  tunnel not running"
                },
                ui::muted(),
            ),
        ]));
    }
}

/// A command as you'd type it: the tool by its name when that's the one
/// your PATH finds.
pub fn typed(c: &[String]) -> String {
    let mut w: Vec<String> = c.iter().map(|x| sh_quote(x)).collect();
    if let Some(name) = Path::new(&c[0]).file_name().and_then(|n| n.to_str()) {
        if proc::which(name).is_some_and(|p| p == Path::new(&c[0])) {
            w[0] = name.to_string();
        }
    }
    w.join(" ")
}

fn show_cmd(c: &[String]) {
    out(Line::from(Span::styled(
        format!("  $ {}", typed(c)),
        ui::muted(),
    )));
}

fn show_found(lines: &[String]) {
    for l in lines {
        say(Line::from(vec![
            Span::styled("  ✓ ", ui::fg(ui::c::GREEN())),
            Span::raw(l.clone()),
        ]));
    }
}

/// Ask before changing anything: `--yes` says yes, no terminal says no.
fn confirm(o: &Opts, q: &str) -> bool {
    if o.yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        err("this asks before changing anything — run it in a terminal, or pass --yes (--dry-run shows the plan)");
        return false;
    }
    yes(q)
}

/// Run one step, its output indented as it comes.
fn run_step(c: &[String], timeout: Duration) -> Result<String, String> {
    show_cmd(c);
    if cfg!(test) {
        return Err("not run from unit tests".into());
    }
    let mut all = String::new();
    let r = proc::stream(
        std::process::Command::new(&c[0]).args(&c[1..]),
        timeout,
        None,
        |l| {
            all.push_str(l);
            all.push('\n');
            say(Line::from(Span::styled(format!("    {l}"), ui::faint())));
        },
    )
    .map_err(|e| format!("{}: {e}", c[0]))?;
    let what = Path::new(&c[0])
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let what = format!("{what} {}", c.get(1).map(String::as_str).unwrap_or(""));
    if r.ok() {
        Ok(all)
    } else if r.timed_out {
        Err(format!("{what} timed out"))
    } else {
        // the line that says what went wrong, else the last one
        let why = all
            .lines()
            .find(|l| {
                let l = l.to_lowercase();
                l.contains("denied") || l.contains("error") || l.contains("failed")
            })
            .map(str::to_string)
            .unwrap_or_else(|| r.reason());
        Err(format!("{what}: {why}"))
    }
}

/// After exposing: the names the API answers to, `api.http`, a token.
fn settle(cfg: &Config, hosts: Option<&str>, o: &Opts) {
    let mut c = Config::load();
    if let Some(h) = hosts {
        match c.set("api.hosts", h) {
            Ok(()) => ok(&format!("api.hosts = {h}")),
            Err(e) => warn(&format!("couldn't set api.hosts: {e}")),
        }
    }
    let api = ApiCfg::from(cfg);
    if !api.http {
        if o.yes
            || (std::io::stdin().is_terminal() && yes("Turn the HTTP API on now (api.http = on)?"))
        {
            match c.set("api.http", "on") {
                Ok(()) => {
                    ok("api.http = on");
                    if crate::daemonctl::ping().is_some() {
                        hint("aleph daemon restart — the daemon starts listening on it");
                    }
                }
                Err(e) => warn(&format!("couldn't set api.http: {e}")),
            }
        } else {
            hint("the API is still off: aleph settings set api.http on");
        }
    }
    hint("aleph daemon pair — gives a phone or laptop its token");
}

fn tailscale(cfg: &Config, https_port: u16, o: &Opts) -> ExitCode {
    let api = ApiCfg::from(cfg);
    let sp = Spinner::start("Looking at tailscale".into());
    let t = ts_detect();
    sp.stop();
    let t = match t {
        None => {
            err("tailscale isn't installed here");
            hint("https://tailscale.com/download — then tailscale up, and run this again");
            return ExitCode::from(1);
        }
        Some(Err(e)) => return fail(&e),
        Some(Ok(t)) => t,
    };
    let plan = match ts_plan(&t, api.port(), https_port, &api.hosts) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    show_found(&plan.found);
    for n in &plan.notes {
        warn(n);
    }
    if plan.done && plan.hosts.is_none() {
        ok(&format!(
            "already exposed: {} → {} (your tailnet only)",
            plan.url,
            api.local_url()
        ));
        return ExitCode::SUCCESS;
    }
    head(&format!(
        "Expose the API at {} — only devices on your tailnet reach it",
        plan.url
    ));
    for c in &plan.run {
        show_cmd(c);
    }
    if let Some(h) = &plan.hosts {
        say(Line::from(Span::styled(
            format!("  then api.hosts = {h}"),
            ui::muted(),
        )));
    }
    if o.dry {
        hint("nothing done (--dry-run)");
        return ExitCode::SUCCESS;
    }
    if !confirm(o, "Go ahead?") {
        return ExitCode::from(1);
    }
    for c in &plan.run {
        if let Err(e) = run_step(c, T_SERVE) {
            err(&e);
            if e.contains("denied") || e.contains("Access denied") || e.contains("operator") {
                hint(&format!(
                    "let your user change tailscale's serve config: sudo tailscale set --operator={}   (Aleph never runs sudo)",
                    service::user_name()
                ));
            }
            return ExitCode::from(1);
        }
    }
    let mut r = record();
    r["tailscale"] = json!({
        "url": plan.url, "https_port": https_port, "port": api.port(),
        "added_host": plan.hosts.as_ref().map(|_| t.dns_name.clone()),
        "at": crate::cache::now(),
    });
    save_record(&r);
    ok(&format!(
        "{} → {} (only devices on your tailnet)",
        plan.url,
        api.local_url()
    ));
    settle(cfg, plan.hosts.as_deref(), o);
    ExitCode::SUCCESS
}

fn cloudflare(cfg: &Config, hostname: Option<String>, tunnel: &str, o: &Opts) -> ExitCode {
    let api = ApiCfg::from(cfg);
    let sp = Spinner::start("Looking at cloudflared".into());
    let c = cf_detect();
    sp.stop();
    let Some(c) = c else {
        err("cloudflared isn't installed here");
        hint(if cfg!(target_os = "macos") {
            "brew install cloudflared — then cloudflared tunnel login"
        } else {
            "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/ — then cloudflared tunnel login"
        });
        return ExitCode::from(1);
    };
    let hostname = match hostname {
        Some(h) => h,
        None if !o.yes && std::io::stdin().is_terminal() => ask(
            "Public hostname for the API",
            "a name in your Cloudflare zone, like aleph.example.com",
        ),
        None => return fail(
            "--hostname: the public name for the API, in your Cloudflare zone (aleph.example.com)",
        ),
    };
    let hostname = hostname
        .trim()
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_lowercase();
    let plan = match cf_plan(&c, tunnel, &hostname, api.port(), &api.hosts) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let init = Init::here();
    if init == Init::Unsupported {
        return fail("no service manager Aleph knows here to keep the tunnel running");
    }
    show_found(&plan.found);
    head(&format!(
        "Expose the API at {} through the Cloudflare tunnel {tunnel}",
        plan.url
    ));
    if let Some(cr) = &plan.create {
        show_cmd(cr);
    }
    show_cmd(&plan.route);
    say(Line::from(Span::styled(
        format!("  write {}", tilde(&plan.config_path)),
        ui::muted(),
    )));
    for l in plan.config.lines() {
        say(Line::from(Span::styled(format!("    {l}"), ui::faint())));
    }
    say(Line::from(Span::styled(
        format!(
            "  a {} service of its own, {} (cloudflared tunnel --config {} run {tunnel})",
            init.word(),
            plan.unit.name(init),
            tilde(&plan.config_path)
        ),
        ui::muted(),
    )));
    if let Some(h) = &plan.hosts {
        say(Line::from(Span::styled(
            format!("  then api.hosts = {h}"),
            ui::muted(),
        )));
    }
    access_reminder(&plan.url);
    if o.dry {
        hint("nothing done (--dry-run)");
        return ExitCode::SUCCESS;
    }
    if !confirm(o, "Go ahead?") {
        return ExitCode::from(1);
    }
    let mut id = plan.id.clone();
    if let Some(cr) = &plan.create {
        match run_step(cr, Duration::from_secs(60)) {
            Ok(text) => id = parse_created(&text),
            Err(e) => return fail(&e),
        }
        if id.is_none() {
            // read it back from the list
            id = cf_detect().and_then(|c| {
                c.tunnels
                    .into_iter()
                    .find(|(_, n)| n == tunnel)
                    .map(|(i, _)| i)
            });
        }
    }
    let Some(id) = id else {
        return fail(&format!(
            "couldn't find the id of the tunnel {tunnel} (cloudflared tunnel list)"
        ));
    };
    let cred = c.dir.join(format!("{id}.json"));
    if !cred.exists() {
        warn(&format!(
            "{} isn't there: cloudflared made the tunnel elsewhere, or on another computer — copy its credentials file here",
            tilde(&cred)
        ));
    }
    if let Err(e) = run_step(&plan.route, Duration::from_secs(60)) {
        if e.contains("already exists") {
            warn(&format!(
                "a DNS record for {hostname} already exists — if it isn't this tunnel's, remove it in the Cloudflare dashboard (or: cloudflared tunnel route dns --overwrite-dns {tunnel} {hostname})"
            ));
        } else {
            return fail(&e);
        }
    }
    let config = cf_config(&id, &cred, &hostname, api.port());
    if let Err(e) = service::write_file(false, &plan.config_path, &config) {
        return fail(&e);
    }
    let unit = &plan.unit;
    let r = service::write_file(false, &unit.file(init), &unit.content(init)).and_then(|_| {
        service::run_cmds(
            false,
            &service::commands(init, Op::Install, unit, &service::uid(), false),
        )
    });
    if let Err(e) = r {
        return fail(&e);
    }
    let mut rec = record();
    rec["cloudflare"] = json!({
        "url": plan.url, "hostname": hostname, "tunnel": tunnel, "id": id,
        "config": plan.config_path, "port": api.port(),
        "added_host": plan.hosts.as_ref().map(|_| hostname.clone()),
        "at": crate::cache::now(),
    });
    save_record(&rec);
    ok(&format!(
        "{} → {} through the tunnel {tunnel} ({} {})",
        plan.url,
        api.local_url(),
        init.word(),
        unit.name(init)
    ));
    access_reminder(&plan.url);
    settle(cfg, plan.hosts.as_deref(), o);
    ExitCode::SUCCESS
}

fn access_reminder(url: &str) {
    warn(&format!(
        "anyone on the internet can reach {url} — put Cloudflare Access in front of it before you pair a phone"
    ));
    hint("Cloudflare Zero Trust › Access › Applications › Add an application › Self-hosted: that hostname, with a policy that lets only you in");
}

/// `aleph daemon expose off [tailscale | cloudflare]`.
fn off(cfg: &Config, which: Option<&str>, o: &Opts) -> ExitCode {
    let api = ApiCfg::from(cfg);
    let mut rec = record();
    let mut did = 0;
    let mut drop_hosts: Vec<String> = Vec::new();
    if which.is_none_or(|w| w == "tailscale") {
        match ts_detect() {
            Some(Ok(t)) => {
                // ours: what proxies to the API's port now, or did when we set it up
                let mut cmds = ts_off(&t, api.port());
                if let Some(p) = rec["tailscale"]["port"].as_u64() {
                    if p as u16 != api.port() {
                        cmds.extend(ts_off(&t, p as u16));
                    }
                }
                if cmds.is_empty() {
                    say(Line::from(Span::styled(
                        "  tailscale: nothing of ours in tailscale serve",
                        ui::muted(),
                    )));
                } else {
                    head("Take the API off your tailnet (only Aleph's entry; tailscale serve reset is never run)");
                    for c in &cmds {
                        show_cmd(c);
                    }
                    if !o.dry {
                        if !confirm(o, "Go ahead?") {
                            return ExitCode::from(1);
                        }
                        for c in &cmds {
                            if let Err(e) = run_step(c, T_TOOL) {
                                return fail(&e);
                            }
                        }
                        did += 1;
                        ok("tailscale no longer serves the API");
                    }
                }
                if !o.dry {
                    if let Some(h) = rec["tailscale"]["added_host"].as_str() {
                        drop_hosts.push(h.to_string());
                    }
                    if let Some(m) = rec.as_object_mut() {
                        m.remove("tailscale");
                    }
                }
            }
            Some(Err(e)) => warn(&format!("tailscale: {e}")),
            None if which.is_some() => warn("tailscale isn't installed here"),
            None => {}
        }
    }
    if which.is_none_or(|w| w == "cloudflare" || w == "cloudflared") {
        let init = Init::here();
        let unit = cf_spec("cloudflared", Path::new(""), "aleph");
        let file = unit.file(init);
        if file.exists() {
            head("Stop Aleph's Cloudflare tunnel service (the tunnel and its DNS record stay)");
            let r = service::run_cmds(
                o.dry,
                &service::commands(init, Op::Uninstall, &unit, &service::uid(), true),
            )
            .and_then(|_| service::remove_file(o.dry, &file))
            .and_then(|_| service::run_cmds(o.dry, &service::after_uninstall(init)));
            if let Err(e) = r {
                return fail(&e);
            }
            if !o.dry {
                did += 1;
                ok(&format!("removed {} {}", init.word(), unit.name(init)));
                let t = rec["cloudflare"]["tunnel"]
                    .as_str()
                    .unwrap_or("aleph")
                    .to_string();
                hint(&format!(
                    "to delete the tunnel too: cloudflared tunnel delete {t} · and its DNS record in the Cloudflare dashboard"
                ));
                if let Some(h) = rec["cloudflare"]["added_host"].as_str() {
                    drop_hosts.push(h.to_string());
                }
                if let Some(m) = rec.as_object_mut() {
                    m.remove("cloudflare");
                }
            }
        } else if which.is_some() {
            say(Line::from(Span::styled(
                "  cloudflare: no tunnel service of Aleph's here",
                ui::muted(),
            )));
        }
    }
    if o.dry {
        hint("nothing done (--dry-run)");
        return ExitCode::SUCCESS;
    }
    if !drop_hosts.is_empty() {
        let left: Vec<String> = api
            .hosts
            .iter()
            .filter(|h| !drop_hosts.iter().any(|d| d.eq_ignore_ascii_case(h)))
            .cloned()
            .collect();
        let mut c = Config::load();
        let r = if left.is_empty() {
            c.unset("api.hosts")
        } else {
            c.set("api.hosts", &left.join(","))
        };
        match r {
            Ok(()) => ok(&format!(
                "api.hosts {}",
                if left.is_empty() {
                    "unset".to_string()
                } else {
                    format!("= {}", left.join(","))
                }
            )),
            Err(e) => warn(&format!("api.hosts: {e}")),
        }
    }
    save_record(&rec);
    if did == 0 {
        ok("the API wasn't exposed by Aleph");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> Ts {
        Ts {
            bin: "tailscale".into(),
            version: Some("1.76.6".into()),
            backend: "Running".into(),
            dns_name: "mbp.tail1234.ts.net".into(),
            https: Some(true),
            serve: vec![],
            tcp_ports: vec![],
        }
    }

    #[test]
    fn reads_tailscale() {
        assert_eq!(
            parse_ts_version("1.76.6\n  tailscale commit: abc\n").as_deref(),
            Some("1.76.6")
        );
        assert_eq!(
            parse_ts_version("1.77.0-dev2024\n").as_deref(),
            Some("1.77.0")
        );
        assert_eq!(parse_ts_version("command not found"), None);
        assert!(ts_version_ok("1.52.0") && ts_version_ok("1.76.6") && ts_version_ok("1.100.1"));
        assert!(!ts_version_ok("1.50.1"));
        let st: Value = serde_json::from_str(
            r#"{"BackendState":"Running","Self":{"DNSName":"MBP.tail1234.ts.net.","Capabilities":["HTTPS","https"],"CapMap":{"https":null}},"CurrentTailnet":{"MagicDNSEnabled":true}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_ts_status(&st),
            ("Running".into(), "mbp.tail1234.ts.net".into(), Some(true))
        );
        let off: Value = serde_json::from_str(
            r#"{"BackendState":"Running","Self":{"DNSName":"x.ts.net.","Capabilities":[]}}"#,
        )
        .unwrap();
        assert_eq!(parse_ts_status(&off).2, Some(false));
        let none: Value =
            serde_json::from_str(r#"{"BackendState":"NeedsLogin","Self":{}}"#).unwrap();
        assert_eq!(
            parse_ts_status(&none),
            ("NeedsLogin".into(), "".into(), None)
        );
    }

    #[test]
    fn reads_serve_status() {
        let v: Value = serde_json::from_str(
            r#"{"TCP":{"443":{"HTTPS":true},"2222":{"TCPForward":"127.0.0.1:22"}},
                "Web":{"mbp.tail1234.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:7878"},"/grafana":{"Proxy":"http://127.0.0.1:3000"}}}}}"#,
        )
        .unwrap();
        let (w, tcp) = parse_serve_status(&v);
        assert_eq!(tcp, vec![2222]);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].path, "/");
        assert_eq!(w[0].url(), "https://mbp.tail1234.ts.net");
        assert_eq!(w[1].url(), "https://mbp.tail1234.ts.net/grafana");
        assert_eq!(parse_serve_status(&json!({})), (vec![], vec![]));
        assert!(same_target("http://127.0.0.1:7878", 7878));
        assert!(same_target("http://localhost:7878/", 7878));
        assert!(same_target("7878", 7878));
        assert!(!same_target("http://127.0.0.1:3000", 7878));
        assert!(!same_target("http://10.0.0.5:7878", 7878));
    }

    #[test]
    fn plans_tailscale_serve() {
        let p = ts_plan(&ts(), 7878, 443, &[]).unwrap();
        assert_eq!(
            p.run,
            vec![vec![
                "tailscale",
                "serve",
                "--bg",
                "--https=443",
                "http://127.0.0.1:7878"
            ]]
        );
        assert_eq!(p.url, "https://mbp.tail1234.ts.net");
        assert_eq!(p.hosts.as_deref(), Some("mbp.tail1234.ts.net"));
        assert!(!p.done && p.notes.is_empty());
        // another port, names kept
        let p = ts_plan(&ts(), 9000, 8443, &["other.example.com".into()]).unwrap();
        assert_eq!(p.run[0][3], "--https=8443");
        assert_eq!(p.run[0][4], "http://127.0.0.1:9000");
        assert_eq!(p.url, "https://mbp.tail1234.ts.net:8443");
        assert_eq!(
            p.hosts.as_deref(),
            Some("other.example.com,mbp.tail1234.ts.net")
        );
        // already there: nothing to run
        let mut t = ts();
        t.serve.push(Serve {
            host: t.dns_name.clone(),
            https_port: 443,
            path: "/".into(),
            proxy: "http://127.0.0.1:7878".into(),
        });
        let p = ts_plan(&t, 7878, 443, &["mbp.tail1234.ts.net".into()]).unwrap();
        assert!(p.done && p.run.is_empty() && p.hosts.is_none());
        // someone else's handler there: never replaced
        t.serve[0].proxy = "http://127.0.0.1:3000".into();
        assert!(ts_plan(&t, 7878, 443, &[])
            .unwrap_err()
            .contains("--https-port"));
        // raw TCP on that port
        let mut t = ts();
        t.tcp_ports.push(443);
        assert!(ts_plan(&t, 7878, 443, &[]).is_err());
        // too old, not connected, no MagicDNS, no HTTPS certs
        let mut t = ts();
        t.version = Some("1.50.0".into());
        assert!(ts_plan(&t, 7878, 443, &[])
            .unwrap_err()
            .contains("older than 1.52"));
        let mut t = ts();
        t.backend = "NeedsLogin".into();
        assert!(ts_plan(&t, 7878, 443, &[])
            .unwrap_err()
            .contains("tailscale up"));
        let mut t = ts();
        t.dns_name.clear();
        assert!(ts_plan(&t, 7878, 443, &[])
            .unwrap_err()
            .contains("MagicDNS"));
        let mut t = ts();
        t.https = Some(false);
        assert_eq!(ts_plan(&t, 7878, 443, &[]).unwrap().notes.len(), 1);
    }

    #[test]
    fn off_takes_back_only_ours() {
        let mut t = ts();
        let s = |port: u16, path: &str, proxy: &str| Serve {
            host: "mbp.tail1234.ts.net".into(),
            https_port: port,
            path: path.into(),
            proxy: proxy.into(),
        };
        t.serve = vec![
            s(443, "/", "http://127.0.0.1:7878"),
            s(443, "/grafana", "http://127.0.0.1:3000"),
            s(8443, "/aleph", "http://127.0.0.1:7878"),
        ];
        assert_eq!(
            ts_off(&t, 7878),
            vec![
                vec!["tailscale", "serve", "--https=443", "off"],
                vec![
                    "tailscale",
                    "serve",
                    "--https=8443",
                    "--set-path=/aleph",
                    "off"
                ],
            ]
        );
        assert!(ts_off(&t, 9999).is_empty());
        assert!(!ts_off(&t, 7878).iter().flatten().any(|w| w == "reset"));
    }

    fn cf() -> Cf {
        Cf {
            bin: "/opt/homebrew/bin/cloudflared".into(),
            version: Some("2025.8.1".into()),
            dir: PathBuf::from("/Users/me/.cloudflared"),
            cert: true,
            tunnels: vec![],
            error: None,
        }
    }

    #[test]
    fn reads_cloudflared() {
        assert_eq!(
            parse_cf_version("cloudflared version 2025.8.1 (built 2025-08-21-1356 UTC)").as_deref(),
            Some("2025.8.1")
        );
        let v: Value = serde_json::from_str(r#"[{"id":"6ff42ae2-765d-4adf-8112-31c55c1551ef","name":"aleph","created_at":"2026-10-22T06:00:00Z","deleted_at":"0001-01-01T00:00:00Z","connections":[]},{"id":"x","name":"old","deleted_at":"2026-01-01T00:00:00Z"}]"#).unwrap();
        assert_eq!(
            parse_tunnels(&v),
            vec![(
                "6ff42ae2-765d-4adf-8112-31c55c1551ef".into(),
                "aleph".into()
            )]
        );
        assert_eq!(
            parse_created("Tunnel credentials written to /x/6ff4.json.\n\nCreated tunnel aleph with id 6ff42ae2-765d-4adf-8112-31c55c1551ef\n").as_deref(),
            Some("6ff42ae2-765d-4adf-8112-31c55c1551ef")
        );
        assert_eq!(parse_created("nothing"), None);
        assert!(valid_hostname("aleph.example.com"));
        assert!(
            !valid_hostname("aleph")
                && !valid_hostname("https://a.b/c")
                && !valid_hostname("-a.b.com")
        );
    }

    #[test]
    fn plans_a_cloudflare_tunnel() {
        let p = cf_plan(&cf(), "aleph", "aleph.example.com", 7878, &[]).unwrap();
        assert_eq!(
            p.create,
            Some(vec![
                "/opt/homebrew/bin/cloudflared".into(),
                "tunnel".into(),
                "create".into(),
                "aleph".into()
            ])
        );
        assert_eq!(
            p.route[1..],
            ["tunnel", "route", "dns", "aleph", "aleph.example.com"]
        );
        assert_eq!(
            p.config_path,
            PathBuf::from("/Users/me/.cloudflared/aleph.yml")
        );
        assert!(p.config.contains("tunnel: <tunnel id>"));
        assert!(p.config.contains("  - hostname: aleph.example.com\n    service: http://127.0.0.1:7878\n  - service: http_status:404\n"));
        assert_eq!(p.unit.label, CF_LABEL);
        assert_eq!(
            p.unit.args[1..],
            [
                "tunnel",
                "--config",
                "/Users/me/.cloudflared/aleph.yml",
                "run",
                "aleph"
            ]
        );
        assert!(p
            .unit
            .content(Init::Launchd)
            .contains("<string>dev.aleph.cloudflared</string>"));
        assert!(p.unit.content(Init::Systemd).contains("ExecStart=/opt/homebrew/bin/cloudflared tunnel --config /Users/me/.cloudflared/aleph.yml run aleph"));
        assert_eq!(p.hosts.as_deref(), Some("aleph.example.com"));
        assert_eq!(p.url, "https://aleph.example.com");
        assert_eq!(
            cf_config_hostname(&p.config).as_deref(),
            Some("aleph.example.com")
        );
        // the tunnel exists: no create, its id in the config
        let mut c = cf();
        c.tunnels = vec![(
            "6ff42ae2-765d-4adf-8112-31c55c1551ef".into(),
            "aleph".into(),
        )];
        let p = cf_plan(
            &c,
            "aleph",
            "aleph.example.com",
            7878,
            &["aleph.example.com".into()],
        )
        .unwrap();
        assert!(p.create.is_none() && p.hosts.is_none());
        assert!(p.config.contains(
            "credentials-file: /Users/me/.cloudflared/6ff42ae2-765d-4adf-8112-31c55c1551ef.json"
        ));
        // not logged in, a bad name, list failing
        let mut c = cf();
        c.cert = false;
        assert!(cf_plan(&c, "aleph", "aleph.example.com", 7878, &[])
            .unwrap_err()
            .contains("cloudflared tunnel login"));
        assert!(cf_plan(&cf(), "aleph", "aleph", 7878, &[]).is_err());
        let mut c = cf();
        c.error = Some("boom".into());
        assert!(cf_plan(&c, "aleph", "aleph.example.com", 7878, &[]).is_err());
    }

    #[test]
    fn exposure_names_and_json() {
        let e = Exposure {
            tailscale: vec![Serve {
                host: "mbp.tail1234.ts.net".into(),
                https_port: 443,
                path: "/".into(),
                proxy: "http://127.0.0.1:7878".into(),
            }],
            cloudflare: Some((Some("aleph.example.com".into()), true)),
        };
        assert!(e.any());
        assert_eq!(e.names(), vec!["mbp.tail1234.ts.net", "aleph.example.com"]);
        assert_eq!(e.urls()[1], "https://aleph.example.com");
        let j = e.to_json();
        assert_eq!(j["tailscale"][0]["url"], "https://mbp.tail1234.ts.net");
        assert_eq!(j["cloudflare"]["running"], true);
        assert!(!Exposure::default().any());
    }
}
