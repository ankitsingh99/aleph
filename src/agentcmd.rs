//! `aleph agents [--host H] [--local] [--all] [--json] [--refresh]` — which
//! agents are on each machine (OS, GPUs, where each agent is, its version,
//! herdr integration, whether its sessions survive a herdr restart and
//! whether herdr's shell finds it); `aleph agents install <agent>` installs
//! one (prerequisites checked, asked once, output streamed);
//! `aleph agents setup [agent…]` installs or refreshes the integrations (one
//! ssh per machine, asked once).

use crate::agentinstall::{self, Method};
use crate::agents::{self, Agent};
use crate::cli::Args;
use crate::config::Config;
use crate::out::*;
use crate::probe::{self, HostProbe, IState, Persist, LOCAL};
use crate::ui::{self, c};
use crate::{cli, icons, theme};
use ratatui::prelude::*;
use serde_json::json;
use std::io::{self, IsTerminal};
use std::process::ExitCode;
use std::time::Duration;

/// Hosts a command works on: `--host H` (global or after the command), else
/// the fleet; `--local` adds this machine.
pub fn hosts_for(a: &Args) -> Vec<String> {
    pick_hosts(explicit_hosts(a), || a.cfg.hosts(), has_local(a))
}

/// The machines `aleph agents` lists: `--host H`, else the fleet as the
/// dashboard shows it (`local`, this computer, included when it's on);
/// `--local` adds this machine either way.
fn list_hosts(a: &Args) -> Vec<String> {
    pick_hosts(explicit_hosts(a), || a.cfg.fleet(), has_local(a))
}

fn has_local(a: &Args) -> bool {
    a.rest.iter().any(|x| x == "--local")
}

/// The machines named, else `default()`; `local` added (once) when asked for.
fn pick_hosts(
    named: Vec<String>,
    default: impl FnOnce() -> Vec<String>,
    local: bool,
) -> Vec<String> {
    let mut v = if named.is_empty() { default() } else { named };
    if local && !v.iter().any(|h| h == LOCAL) {
        v.push(LOCAL.into());
    }
    v
}

/// Only the hosts named with `--host` (global or after the command).
fn explicit_hosts(a: &Args) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut i = 0;
    while i < a.rest.len() {
        if (a.rest[i] == "--host" || a.rest[i] == "-H") && i + 1 < a.rest.len() {
            v.push(a.rest[i + 1].clone());
            i += 1;
        } else if let Some(h) = a.rest[i].strip_prefix("--host=") {
            v.push(h.to_string());
        }
        i += 1;
    }
    if let Some(f) = &a.flag {
        if !v.contains(f) {
            v.insert(0, f.clone());
        }
    }
    v
}

/// Positional words after the subcommand (skipping flag values).
fn words(rest: &[String]) -> Vec<String> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if a == "--host" || a == "-H" {
            i += 2;
            continue;
        }
        if !a.starts_with('-') {
            v.push(a.clone());
        }
        i += 1;
    }
    v
}

fn host_list(hosts: &[String]) -> String {
    hosts
        .iter()
        .map(|h| host_title(h))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A fresh look at every host, with a spinner.
pub fn probe_with_spinner(hosts: &[String]) -> Vec<HostProbe> {
    let sp = Spinner::start(format!("Looking for agents on {}", host_list(hosts)));
    let v = probe::run_all(hosts);
    sp.stop();
    v
}

/// The cached answers where they're fresh (unless `refresh`), with a spinner.
fn probe_cached_with_spinner(hosts: &[String], refresh: bool) -> Vec<HostProbe> {
    let sp = Spinner::start(format!("Looking for agents on {}", host_list(hosts)));
    let v = probe::run_all_cached(hosts, refresh);
    sp.stop();
    v
}

const USAGE: &str = "aleph agents [--host H] [--local] [--all] [--json] [--refresh] | aleph agents install <agent> [--host H] [--yes] [--dry-run] | aleph agents setup [agent…] [--yes]";

pub fn cmd(a: Args) -> ExitCode {
    let sub = words(a.rest);
    match sub.first().map(String::as_str) {
        None | Some("ls") | Some("list") => list(a),
        Some("install") | Some("add") => install_cmd(a, &sub),
        Some("setup") | Some("integrations") => {
            let only: Vec<String> = sub[1..].to_vec();
            setup(a, &only)
        }
        Some(x) => fail(&format!("unknown subcommand '{x}' — usage: {USAGE}")),
    }
}

fn host_color(cfg: &Config, h: &str) -> Color {
    if h == LOCAL {
        c::ACCENT()
    } else {
        theme::machine(cfg, h)
    }
}

fn host_title(h: &str) -> String {
    if h == LOCAL {
        "this machine".into()
    } else {
        h.to_string()
    }
}

fn list(a: Args) -> ExitCode {
    let hosts = list_hosts(&a);
    if hosts.is_empty() {
        return cli::no_host();
    }
    let all = a.rest.iter().any(|x| x == "--all");
    let refresh = a.rest.iter().any(|x| x == "--refresh" || x == "-r");
    let probes = probe_cached_with_spinner(&hosts, refresh);
    if a.rest.iter().any(|x| x == "--json") {
        json(&to_json(&probes));
    } else if !io::stdout().is_terminal() {
        for p in &probes {
            for (ag, f) in p.installed() {
                let (i, _) = integ_cell(p, ag);
                crate::out::raw(&format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    p.host,
                    ag.id,
                    f.version,
                    i,
                    if matches!(p.persist(ag), Persist::Yes | Persist::Outdated) {
                        "yes"
                    } else {
                        "no"
                    },
                    f.path,
                    match f.on_path {
                        Some(true) => "on_path",
                        Some(false) => "off_path",
                        None => "unknown",
                    },
                    f.check.as_str()
                ));
            }
        }
    } else {
        print_matrix(&a.cfg, &probes, all);
        footer(&probes);
    }
    if probes.iter().all(|p| !p.ok()) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn method_name(m: &Method) -> &'static str {
    match m {
        Method::Npm(_) => "npm",
        Method::Script { .. } => "script",
        Method::Uv(_) => "uv",
        Method::Brew(_) => "brew",
        Method::Manual(_) => "manual",
    }
}

pub fn to_json(probes: &[HostProbe]) -> serde_json::Value {
    json!({"hosts": probes.iter().map(|p| json!({
        "alias": p.host,
        "reachable": p.ok(),
        "error": p.error,
        "os": p.os,
        "os_name": p.os_name,
        "gpu": p.gpu_summary(),
        "gpus": p.gpus,
        "home": p.home,
        "cached": p.age.is_some(),
        "age_secs": p.age.map(|a| a.as_secs()),
        "herdr": p.herdr.as_ref().map(|(path, v)| json!({"path": path, "version": v})),
        "login_path": p.login_path,
        "shell_path": p.shell_path,
        "searched": p.searched,
        "tools": p.tools.iter().map(|t| json!({
            "name": t.name, "path": t.path,
            "version": (!t.version.is_empty()).then_some(&t.version),
            "prefix": (!t.prefix.is_empty()).then_some(&t.prefix),
            "writable": t.writable,
        })).collect::<Vec<_>>(),
        "skipped": p.skipped.iter().map(|s| json!({
            "id": s.id, "path": s.path, "why": s.why,
            "kind": if s.broken { "broken" } else { "other_program" },
        })).collect::<Vec<_>>(),
        "agents": agents::ALL.iter().map(|ag| {
            let f = p.agent(ag.id);
            let integ = ag.integration.and_then(|n| p.integ(n));
            let (glyph, color) = icons::brand(ag.id);
            let pl = agentinstall::plan(ag, p);
            let mut o = json!({
                "id": ag.id, "name": ag.display_name(), "kind": ag.kind,
                "glyph": glyph, "color": ui::hex(color),
                "command": ag.exe(),
                "installed": f.is_some(),
                "path": f.map(|f| f.path.clone()),
                "version": f.map(|f| f.version.clone()),
                "check": f.map(|f| f.check.as_str()),
                "on_path": f.and_then(|f| f.on_path),
                "shadowed_by": f.and_then(|f| f.shadowed_by.clone()),
                "start": probe::start_of(Some(ag)),
            });
            let more = json!({
                "integration": ag.integration(),
                "detect": ag.detect().as_str(),
                "session_logs": ag.logs().map(|l| format!("~/{}", l.dir)),
                "integration_state": integ.map(|i| i.state.as_str()),
                "persistence": matches!(p.persist(ag), Persist::Yes | Persist::Outdated),
                "state": ag.state_note(),
                "resume": ag.resume_args(),
                "install_method": method_name(&pl.method),
                "install_command": pl.command,
                "install_ready": pl.ready(),
                "install_blocker": pl.blocker,
                "prerequisites": pl.prereqs.iter().map(|x| json!({
                    "what": x.what, "ok": x.ok, "detail": x.detail, "fix": x.fix,
                })).collect::<Vec<_>>(),
            });
            if let (Some(m), serde_json::Value::Object(x)) = (o.as_object_mut(), more) {
                m.extend(x);
            }
            o
        }).collect::<Vec<_>>(),
        "integrations": p.integs.iter().map(|i| json!({
            "name": i.name,
            "agent": agents::by_integration(&i.name).map(|a| a.id),
            "state": i.state.as_str(),
            "path": i.path,
        })).collect::<Vec<_>>(),
    })).collect::<Vec<_>>()})
}

/// (plain text, colour) for the integration column.
fn integ_cell(p: &HostProbe, ag: &Agent) -> (String, Color) {
    let i = ag.integration.and_then(|n| p.integ(n));
    match i.map(|i| &i.state) {
        Some(IState::Current(v)) => (format!("✓ installed {v}").trim().to_string(), c::GREEN()),
        Some(IState::Outdated { have, want }) => {
            (format!("↑ outdated {have} → {want}"), c::YELLOW())
        }
        Some(IState::Missing) => ("– none".into(), c::MUTED()),
        Some(IState::Other(s)) => (ui::trunc(s, 20), c::MUTED()),
        None => ("n/a".into(), c::FAINT()),
    }
}

fn persist_cell(p: &HostProbe, ag: &Agent) -> (String, Color) {
    match p.persist(ag) {
        Persist::Yes | Persist::Outdated => ("yes".into(), c::GREEN()),
        Persist::Missing if ag.needs_integration => (
            "no · aleph agents setup (state needs it too)".into(),
            c::YELLOW(),
        ),
        Persist::Missing => ("no · aleph agents setup".into(), c::YELLOW()),
        Persist::None => ("no · aleph restore".into(), c::MUTED()),
    }
}

/// `▌b31  🐧 Ubuntu 22.04.4 LTS · x86_64 · NVIDIA 2× GeForce RTX 4090 · herdr 0.9.5 · 3 agents · 812ms`
fn host_head(cfg: &Config, p: &HostProbe) -> Line<'static> {
    let col = host_color(cfg, &p.host);
    let mut head = vec![
        Span::styled("▌", Style::new().fg(col)),
        Span::styled(
            host_title(&p.host),
            Style::new().fg(col).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(e) = &p.error {
        head.push(Span::styled(
            format!("  ✕ {}", ui::trunc(e, 60)),
            ui::fg(c::RED()),
        ));
        return Line::from(head);
    }
    let dot = || Span::styled(" · ", ui::faint());
    head.push(Span::raw("  "));
    head.push(icons::os_glyph(&format!("{} {}", p.os_name, p.os)));
    let arch = p.os.split_whitespace().nth(1).unwrap_or("");
    let os = match (p.os_name.is_empty(), arch.is_empty()) {
        (false, false) => format!(" {} · {arch}", p.os_name),
        (false, true) => format!(" {}", p.os_name),
        _ => format!(" {}", p.os),
    };
    head.push(Span::styled(os, ui::muted()));
    head.push(dot());
    match p.gpu_summary() {
        Some(g) => {
            let vendor = p.gpus.first().cloned().unwrap_or_else(|| g.clone());
            let badge = icons::gpu(&vendor);
            // the vendor word the badge shows (after a Nerd Font glyph, if any)
            let label = badge
                .content
                .trim_start_matches(|c: char| !c.is_ascii())
                .trim()
                .to_string();
            head.push(badge);
            let rest = g
                .replacen(&label, "", 1)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !rest.is_empty() {
                head.push(Span::styled(format!(" {rest}"), ui::muted()));
            }
        }
        None => head.push(Span::styled("no GPU", ui::faint())),
    }
    head.push(dot());
    match p.herdr_version() {
        Some(v) => head.push(Span::styled(format!("herdr {v}"), ui::muted())),
        None => head.push(Span::styled("no herdr", ui::fg(c::YELLOW()))),
    }
    let n = p.agents.len();
    head.push(Span::styled(
        format!(
            " · {n} agent{} · {}",
            if n == 1 { "" } else { "s" },
            match p.age {
                Some(a) => format!("cached {} ago", ago(a)),
                None => format!("{}ms", p.took.as_millis()),
            }
        ),
        ui::faint(),
    ));
    Line::from(head)
}

fn ago(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

/// Lines under a machine's table: agents herdr's shell won't find, programs
/// that shadow them, installs that don't run, same-named programs we ignored.
pub fn notes(p: &HostProbe) -> Vec<Line<'static>> {
    let mut v = Vec::new();
    let on = host_title(&p.host);
    let warn = |t: String| {
        Line::from(vec![
            Span::styled("  ⚠ ", ui::fg(c::YELLOW())),
            Span::styled(t, ui::fg(c::YELLOW())),
        ])
    };
    for (ag, f) in p.installed() {
        if let Some(s) = &f.shadowed_by {
            v.push(warn(format!("{}: {}", ag.name, shadow_text(p, ag, s))));
        } else if f.on_path == Some(false) {
            v.push(warn(format!(
                "{}: {} isn't on your shell's PATH on {on} — herdr types `{}` into a shell, so add it to PATH",
                ag.name,
                p.tilde(f.dir()),
                ag.exe()
            )));
        }
    }
    for s in &p.skipped {
        let Some(ag) = agents::find(&s.id) else {
            continue;
        };
        if s.broken && p.agent(ag.id).is_none() {
            v.push(Line::from(vec![
                Span::styled("  ✗ ", ui::fg(c::RED())),
                Span::raw(format!(
                    "{} at {} doesn't run: {}",
                    ag.name,
                    p.tilde(&s.path),
                    ui::trunc(&s.why, 70)
                )),
                Span::styled(
                    format!("  · aleph agents install {} --host {}", ag.id, p.host),
                    ui::faint(),
                ),
            ]));
        } else if !s.broken
            && !p
                .agent(ag.id)
                .is_some_and(|f| f.shadowed_by.as_deref() == Some(&s.path))
        {
            v.push(Line::from(Span::styled(
                format!(
                    "  · ignored {}: not {} ({})",
                    p.tilde(&s.path),
                    ag.name,
                    ui::trunc(&s.why, 50)
                ),
                ui::faint(),
            )));
        }
    }
    for (ag, f) in p.installed() {
        if f.check == probe::Check::Unverified {
            v.push(Line::from(Span::styled(
                format!(
                    "  · {} didn't say its version (--version) in time; it's there",
                    ag.name
                ),
                ui::faint(),
            )));
        }
    }
    v
}

/// `/usr/bin/pi comes first on the PATH and isn't Pi — a shell there runs that one`
fn shadow_text(p: &HostProbe, ag: &Agent, s: &str) -> String {
    let broken = p.skipped.iter().any(|x| x.path == s && x.broken);
    format!(
        "{} comes first on the PATH and {} — a shell there runs that one",
        p.tilde(s),
        if broken {
            "doesn't run".to_string()
        } else {
            format!("isn't {}", ag.name)
        }
    )
}

/// One block per machine: header, then a row per installed agent (every
/// registry agent with `all`), then notes.
pub fn print_matrix(cfg: &Config, probes: &[HostProbe], all: bool) {
    let rows: Vec<(&HostProbe, &'static Agent)> = probes
        .iter()
        .flat_map(|p| {
            agents::ALL
                .iter()
                .filter(move |ag| all || p.agent(ag.id).is_some())
                .map(move |ag| (p, ag))
        })
        .collect();
    let wn = rows
        .iter()
        .map(|(_, ag)| ui::width(ag.name))
        .max()
        .unwrap_or(10)
        .max(10);
    let wv = rows
        .iter()
        .filter_map(|(p, ag)| p.agent(ag.id))
        .map(|f| ui::width(&f.version))
        .max()
        .unwrap_or(7)
        .clamp(7, 18);
    let wi = rows
        .iter()
        .map(|(p, ag)| ui::width(&integ_cell(p, ag).0))
        .max()
        .unwrap_or(11)
        .max(11);
    let ww = rows
        .iter()
        .filter_map(|(p, ag)| p.agent(ag.id).map(|f| ui::width(&p.tilde(&f.path))))
        .max()
        .unwrap_or(5)
        .clamp(5, 48);
    for p in probes {
        out(host_head(cfg, p));
        if !p.ok() {
            println!();
            continue;
        }
        let mine: Vec<&'static Agent> = rows
            .iter()
            .filter(|(q, _)| std::ptr::eq(*q, p))
            .map(|(_, ag)| *ag)
            .collect();
        if mine.is_empty() {
            out(Line::from(Span::styled(
                "  no agents found — aleph agents --all lists what herdr supports, with how to install each",
                ui::faint(),
            )));
            for l in notes(p) {
                out(l);
            }
            println!();
            continue;
        }
        out(Line::from(Span::styled(
            format!(
                "    {}  {}  {}  {}  restores after a herdr restart",
                ui::pad("agent", wn),
                ui::pad("version", wv),
                ui::pad("integration", wi),
                ui::pad("where", ww)
            ),
            ui::faint(),
        )));
        for ag in mine {
            let f = p.agent(ag.id);
            let (it, ic) = integ_cell(p, ag);
            let (pt, pc) = persist_cell(p, ag);
            let dim = f.is_none();
            let mut glyph = icons::agent(ag.id);
            if dim {
                glyph.style = ui::faint();
            }
            let mut l = vec![Span::raw("  "), glyph, Span::raw(" ")];
            l.push(Span::styled(
                ui::pad(ag.name, wn),
                if dim { ui::faint() } else { ui::bold() },
            ));
            l.push(Span::raw("  "));
            match f {
                Some(f) => {
                    let v = if f.version.is_empty() {
                        "?"
                    } else {
                        &f.version
                    };
                    l.push(Span::styled(
                        ui::pad(&ui::trunc(v, wv), wv),
                        if f.version.is_empty() {
                            ui::faint()
                        } else {
                            ui::muted()
                        },
                    ));
                    l.push(Span::raw("  "));
                    l.push(Span::styled(ui::pad(&it, wi), ui::fg(ic)));
                    l.push(Span::raw("  "));
                    let off = f.on_path == Some(false) || f.shadowed_by.is_some();
                    l.push(Span::styled(
                        ui::pad(&ui::trunc_left(&p.tilde(&f.path), ww), ww),
                        if off {
                            ui::fg(c::YELLOW())
                        } else {
                            ui::faint()
                        },
                    ));
                    l.push(Span::raw("  "));
                    l.push(Span::styled(pt, ui::fg(pc)));
                }
                None => {
                    let pl = agentinstall::plan(ag, p);
                    l.push(Span::styled(ui::pad("–", wv), ui::faint()));
                    l.push(Span::raw("  "));
                    let how = match (&pl.command, pl.ready()) {
                        (Some(_), true) => format!("aleph agents install {}", ag.id),
                        (Some(cmd), false) => format!(
                            "{cmd} · {}",
                            pl.prereqs
                                .iter()
                                .find(|x| !x.ok)
                                .map(|x| format!("needs {}", x.what))
                                .unwrap_or_else(|| "blocked".into())
                        ),
                        (None, _) => format!("by hand: {}", ag.install),
                    };
                    l.push(Span::styled(format!("not installed · {how}"), ui::faint()));
                }
            }
            out(Line::from(l));
        }
        for l in notes(p) {
            out(l);
        }
        println!();
    }
}

fn footer(probes: &[HostProbe]) {
    let todo = plan(probes, &[]);
    if !todo.is_empty() {
        hint(&format!(
            "aleph agents setup installs {} herdr integration{} so sessions survive a herdr restart",
            todo.len(),
            if todo.len() == 1 { "" } else { "s" }
        ));
    }
    let restore = probes.iter().any(|p| {
        p.installed()
            .iter()
            .any(|(ag, _)| p.persist(ag) == Persist::None)
    });
    if restore {
        hint("agents without an integration are recreated by aleph restore after a reboot");
    }
    if probes.iter().any(|p| p.age.is_some()) {
        say(Line::from(Span::styled(
            "answers from the last 10 minutes are reused — aleph agents --refresh looks again",
            ui::faint(),
        )));
    } else if todo.is_empty() && !restore {
        say(Line::from(Span::styled(
            "aleph agents --all lists every agent herdr supports · aleph agents install <agent> installs one",
            ui::faint(),
        )));
    }
}

// ------------------------------------------------------------------ setup

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub host: String,
    pub agent: &'static Agent,
    pub integration: &'static str,
    /// true = refresh an outdated one
    pub refresh: bool,
}

/// Integrations to install or refresh: installed agents whose integration
/// this host's herdr offers but which is missing or outdated. `only` (ids or
/// names) narrows it to some agents.
pub fn plan(probes: &[HostProbe], only: &[String]) -> Vec<Step> {
    let only: Vec<&str> = only
        .iter()
        .filter_map(|q| agents::find(q).map(|a| a.id))
        .collect();
    let mut v = Vec::new();
    for p in probes.iter().filter(|p| p.ok()) {
        for (ag, _) in p.installed() {
            if !only.is_empty() && !only.contains(&ag.id) {
                continue;
            }
            let Some(name) = ag.integration else { continue };
            match p.persist(ag) {
                Persist::Missing | Persist::Outdated => v.push(Step {
                    host: p.host.clone(),
                    agent: ag,
                    integration: name,
                    refresh: p.persist(ag) == Persist::Outdated,
                }),
                _ => {}
            }
        }
    }
    v
}

pub fn print_plan(cfg: &Config, steps: &[Step]) {
    let wh = steps
        .iter()
        .map(|s| ui::width(&host_title(&s.host)))
        .max()
        .unwrap_or(6);
    for s in steps {
        out(Line::from(vec![
            Span::raw("  "),
            Span::styled("▌", Style::new().fg(host_color(cfg, &s.host))),
            Span::styled(ui::pad(&host_title(&s.host), wh), ui::bold()),
            Span::raw("  "),
            Span::styled(
                if s.refresh { "refresh " } else { "install " },
                ui::fg(if s.refresh { c::YELLOW() } else { c::GREEN() }),
            ),
            Span::raw(format!("{} integration", s.agent.name)),
            Span::styled(
                format!("  herdr integration install {}", s.integration),
                ui::faint(),
            ),
        ]));
    }
}

/// Run the steps, one `sh -s` per machine in parallel. Returns (ok, failed).
pub fn apply(steps: &[Step]) -> (usize, usize) {
    let mut hosts: Vec<String> = Vec::new();
    for s in steps {
        if !hosts.contains(&s.host) {
            hosts.push(s.host.clone());
        }
    }
    let sp = Spinner::start(format!("Installing {} integration(s)", steps.len()));
    let handles: Vec<_> = hosts
        .iter()
        .map(|h| {
            let names: Vec<&'static str> = steps
                .iter()
                .filter(|s| s.host == *h)
                .map(|s| s.integration)
                .collect();
            let h = h.clone();
            std::thread::spawn(move || {
                let r = probe::sh(&h, &install_script(&names), Duration::from_secs(120));
                (h, names, r)
            })
        })
        .collect();
    let (mut good, mut bad) = (0, 0);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for hd in handles {
        let Ok((h, names, r)) = hd.join() else {
            continue;
        };
        let results = match &r {
            Ok(o) => parse_install(&o.text()),
            Err(_) => vec![],
        };
        for n in names {
            let res = results.iter().find(|x| x.0 == n);
            match res {
                Some((_, true, _)) => {
                    good += 1;
                    lines.push(Line::from(vec![
                        Span::styled("✓ ", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD)),
                        Span::raw(format!("{} · {n} integration installed", host_title(&h))),
                    ]));
                }
                other => {
                    bad += 1;
                    let why = match (other, &r) {
                        (Some((_, _, m)), _) => m.clone(),
                        (None, Ok(o)) => o.reason(),
                        (None, Err(e)) => e.clone(),
                    };
                    lines.push(prefixed(
                        "error",
                        c::RED(),
                        &format!("{} · {n}: {why}", host_title(&h)),
                    ));
                }
            }
        }
    }
    sp.stop();
    for l in lines {
        say(l);
    }
    (good, bad)
}

fn install_script(names: &[&str]) -> String {
    format!(
        "LC_ALL=C; export LC_ALL\nfor n in {}; do if o=$(herdr integration install \"$n\" </dev/null 2>&1); then r=ok; else r=fail; fi; printf 'CCX\\t%s\\t%s\\t%s\\n' \"$n\" \"$r\" \"$(printf '%s\\n' \"$o\" | grep . | tail -n 1 | tr -d '\\t')\"; done\n",
        names.join(" ")
    )
}

/// `CCX\t<name>\tok|fail\t<last line>` → (name, ok, message).
pub fn parse_install(text: &str) -> Vec<(String, bool, String)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(4, '\t').collect();
            (f.len() >= 3 && f[0] == "CCX").then(|| {
                (
                    f[1].to_string(),
                    f[2] == "ok",
                    f.get(3).unwrap_or(&"").to_string(),
                )
            })
        })
        .collect()
}

/// Plan, show, ask once, apply. Shared with `aleph setup`.
/// Some(true) = all in place (or installed), Some(false) = something failed,
/// None = declined.
pub fn setup_flow(
    cfg: &Config,
    probes: &[HostProbe],
    only: &[String],
    assume_yes: bool,
) -> Option<bool> {
    for q in only {
        if agents::find(q).is_none() {
            err(&agents::resolve(q).unwrap_err());
            return Some(false);
        }
    }
    let steps = plan(probes, only);
    if steps.is_empty() {
        let n: usize = probes
            .iter()
            .map(|p| {
                p.installed()
                    .iter()
                    .filter(|(ag, _)| p.persist(ag) == Persist::Yes)
                    .count()
            })
            .sum();
        ok(&if n > 0 {
            format!("every installed agent with a herdr integration has it ({n} in all)")
        } else {
            "nothing to install — no installed agent has a missing or outdated integration".into()
        });
        return Some(true);
    }
    say(Line::from(vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled(
            format!(
                "{} herdr integration{} to set up",
                steps.len(),
                if steps.len() == 1 { "" } else { "s" }
            ),
            ui::bold(),
        ),
        Span::styled(
            "  · herdr then restores these agents' sessions after a restart",
            ui::faint(),
        ),
    ]));
    print_plan(cfg, &steps);
    if !assume_yes && !yes("Install them now?") {
        hint("aleph agents setup does it later");
        return None;
    }
    let (_, bad) = apply(&steps);
    // an integration install may rewrite the agent's settings: where Aleph's
    // own hooks are opted in, put them back (and say so)
    let mut hosts: Vec<String> = probes.iter().map(|p| p.host.clone()).collect();
    hosts.dedup();
    for h in hosts
        .iter()
        .filter(|h| cfg.host_get(h, "hooks").as_deref() == Some("on"))
    {
        for m in crate::hooks::ensure(h) {
            ok(&m);
        }
    }
    Some(bad == 0)
}

fn setup(a: Args, only: &[String]) -> ExitCode {
    let hosts = hosts_for(&a);
    if hosts.is_empty() {
        return cli::no_host();
    }
    let assume = a.rest.iter().any(|x| x == "--yes" || x == "-y");
    if !assume && !io::stdin().is_terminal() {
        return fail(
            "aleph agents setup asks before installing — run it in a terminal, or pass --yes",
        );
    }
    let probes = probe_with_spinner(&hosts);
    for p in probes.iter().filter(|p| !p.ok()) {
        warn(&format!(
            "{}: {}",
            host_title(&p.host),
            p.error.clone().unwrap_or_default()
        ));
    }
    if setup_flow(&a.cfg, &probes, only, assume) != Some(false) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

// ------------------------------------------------------------------ install

pub use crate::agentsetup::Opts as InstallOpts;

fn install_cmd(a: Args, sub: &[String]) -> ExitCode {
    if sub.len() < 2 {
        err("which agent? usage: aleph agents install <agent>… [--host H] [--yes] [--dry-run]");
        hint("aleph agents --all lists them · aleph host setup <host> picks from a checklist · aleph agents setup installs herdr integrations");
        return ExitCode::from(2);
    }
    let mut ids: Vec<&'static str> = Vec::new();
    for q in &sub[1..] {
        match agents::resolve(q) {
            Ok(agents::SHELL) => return fail("shell isn't an agent — there's nothing to install"),
            Ok(id) if !ids.contains(&id) => ids.push(id),
            Ok(_) => {}
            Err(e) => return fail(&e),
        }
    }
    let mut hosts = explicit_hosts(&a);
    if a.rest.iter().any(|x| x == "--local") && !hosts.iter().any(|h| h == LOCAL) {
        hosts.push(LOCAL.into());
    }
    if hosts.is_empty() {
        match a.cfg.get("ssh_host") {
            Some(h) => hosts.push(h),
            None => return cli::no_host(),
        }
    }
    let o = InstallOpts {
        yes: a.rest.iter().any(|x| x == "--yes" || x == "-y"),
        dry_run: a.rest.iter().any(|x| x == "--dry-run" || x == "-n"),
        integrations: if a.rest.iter().any(|x| x == "--integrations") {
            Some(true)
        } else if a.rest.iter().any(|x| x == "--no-integrations") {
            Some(false)
        } else {
            None
        },
        // Node.js first for the npm ones (agents.node), unless --no-node
        node: a.rest.iter().any(|x| x == "--no-node").then_some(false),
    };
    if !o.yes && !o.dry_run && !io::stdin().is_terminal() {
        return fail(
            "aleph agents install asks before installing — run it in a terminal, or pass --yes (--dry-run shows the plan)",
        );
    }
    if install_many(&a.cfg, &ids, &hosts, o) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Install agent `id` on `hosts` ([`install_many`] with one). The dashboard
/// runs this in the terminal for the new-session form's "install it".
pub fn install_flow(cfg: &Config, id: &str, hosts: &[String], o: InstallOpts) -> bool {
    if let Some(e) = crate::showcase::refused() {
        err(&e);
        return false;
    }
    let Some(ag) = agents::find(id) else {
        err(&agents::resolve(id).unwrap_err());
        return false;
    };
    install_many(cfg, &[ag.id], hosts, o)
}

/// Install `ids` on each of `hosts`: look at the machine, show the exact
/// commands and what they need, ask once, run them one after another with
/// the output streamed, check each runs (`agentsetup::run`: retry / skip /
/// the log on a failure), then tell the running Aleph to look again. True
/// when every one is there afterwards.
pub fn install_many(cfg: &Config, ids: &[&'static str], hosts: &[String], o: InstallOpts) -> bool {
    let names: Vec<&str> = ids
        .iter()
        .filter_map(|i| agents::find(i).map(|a| a.name))
        .collect();
    let sp = Spinner::start(format!(
        "Looking at {} on {}",
        names.join(", "),
        host_list(hosts)
    ));
    let probes = probe::run_all(hosts);
    sp.stop();
    let mut good = true;
    for p in probes {
        if let Some(e) = &p.error {
            err(&format!("{}: {e}", host_title(&p.host)));
            good = false;
            continue;
        }
        let rep = crate::agentsetup::run(cfg, p, Some(ids.to_vec()), o);
        good &= rep.ok();
        // the running Aleph looks at the machine again (its agents, now)
        if !o.dry_run && rep.count("installed") + rep.count("failed") > 0 {
            let n = crate::refresh::Note::from_report("install", &rep);
            let t = crate::refresh::signal(&n);
            say(Line::from(Span::styled(
                format!("  {}", crate::refresh::told_line(&rep.host, &t)),
                ui::faint(),
            )));
        }
    }
    good
}

/// The plan, as the user sees it before saying yes.
pub(crate) fn print_install_plan(cfg: &Config, p: &HostProbe, pl: &agentinstall::Plan) {
    let ag = pl.agent;
    let on = host_title(&p.host);
    say(Line::from(vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled(format!("Install {} on ", ag.name), ui::bold()),
        Span::styled(
            on.clone(),
            ui::fg(host_color(cfg, &p.host)).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        icons::agent(ag.id),
    ]));
    if let Some(cmd) = &pl.command {
        say(Line::from(vec![
            Span::styled("  $ ", ui::faint()),
            Span::styled(cmd.clone(), ui::bold()),
            match &pl.path_first {
                Some(d) => Span::styled(format!("   (npm from {})", p.tilde(d)), ui::faint()),
                None => Span::raw(""),
            },
        ]));
    }
    let wp = pl
        .prereqs
        .iter()
        .map(|x| ui::width(&x.what))
        .max()
        .unwrap_or(0);
    for x in &pl.prereqs {
        say(Line::from(vec![
            Span::raw("  "),
            if x.ok {
                Span::styled("✓ ", ui::fg(c::GREEN()))
            } else {
                Span::styled("✗ ", ui::fg(c::RED()))
            },
            Span::raw(ui::pad(&x.what, wp)),
            Span::raw("  "),
            Span::styled(
                x.detail.clone(),
                if x.ok { ui::muted() } else { ui::fg(c::RED()) },
            ),
        ]));
    }
    if let Some(d) = &pl.lands_in {
        say(Line::from(Span::styled(
            format!("  · {} lands in {}", ag.exe(), p.tilde(d)),
            ui::faint(),
        )));
    }
    for n in &pl.notes {
        say(Line::from(Span::styled(format!("  · {n}"), ui::faint())));
    }
    for s in p.skipped.iter().filter(|s| s.id == ag.id && s.broken) {
        warn(&format!(
            "there's a {} at {} that doesn't run ({}) — installing again usually fixes it",
            ag.name,
            p.tilde(&s.path),
            ui::trunc(&s.why, 60)
        ));
    }
    if let Some(b) = &pl.blocker {
        err(b);
        for x in pl.prereqs.iter().filter(|x| !x.ok) {
            if let Some(f) = &x.fix {
                hint(f);
            }
        }
        if pl.command.is_some() && pl.prereqs.iter().all(|x| x.ok) {
            // root-owned npm: they can still do it themselves
            hint(&format!(
                "or install it yourself: ssh -t {} {}",
                p.host,
                crate::proc::sh_quote(pl.command.as_deref().unwrap_or_default())
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(text: &str) -> HostProbe {
        probe::parse("b31", text)
    }

    #[test]
    fn plans_only_installed_agents_with_offered_integrations() {
        let p = probe(
            "CCA\tclaude\t/b/claude\t2.1.0\nCCA\tcodex\t/b/codex\t0.4.0\nCCA\tamp\t/b/amp\t1.0\nCCA\tcursor\t/b/cursor-agent\t1.0\n\
             CCH\t/b/herdr\therdr 0.9.5\n\
             CCI\tclaude: outdated (v7 < v11) (/h/c)\nCCI\tcodex: not installed (/h/x)\nCCI\topencode: not installed (/h/o)\nCCO\tLinux\n",
        );
        let v = plan(std::slice::from_ref(&p), &[]);
        let ids: Vec<(&str, bool)> = v.iter().map(|s| (s.integration, s.refresh)).collect();
        // amp has no integration; cursor's isn't offered by this herdr; opencode isn't installed
        assert_eq!(ids, vec![("claude", true), ("codex", false)]);
        let only = plan(std::slice::from_ref(&p), &["codex".into()]);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].agent.id, "codex");
    }

    #[test]
    fn install_results() {
        let r = parse_install("junk\nCCX\tclaude\tok\tinstalled\nCCX\tcodex\tfail\terror: nope\n");
        assert_eq!(
            r,
            vec![
                ("claude".into(), true, "installed".into()),
                ("codex".into(), false, "error: nope".into())
            ]
        );
        let s = install_script(&["claude", "antigravity-cli"]);
        assert!(s.contains("for n in claude antigravity-cli;"));
    }

    #[test]
    fn json_shape() {
        let p =
            probe("CCA\tclaude\t/b/claude\t2.1.0\nCCI\tclaude: current (v11) (/h/c)\nCCO\tLinux\n");
        let v = to_json(&[p]);
        let a = &v["hosts"][0]["agents"];
        let claude = a
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "claude")
            .unwrap();
        assert_eq!(claude["installed"], true);
        assert_eq!(claude["integration_state"], "current");
        assert_eq!(claude["persistence"], true);
    }

    #[test]
    fn json_has_glyphs_install_and_paths() {
        let p = probe(
            "CCW\t/home/n\nCCP\tlogin\t/usr/bin:/bin\nCCP\tshell\t/home/n/.local/bin:/usr/bin:/bin\n\
             CCC\t1\tcodex\t/home/n/.local/bin/codex\t0\t0\tcodex-cli 0.46.0\n\
             CCC\t2\topencode\t/home/n/.opencode/bin/opencode\t0\t0\t1.18.15\n\
             CCT\tcurl\t/usr/bin/curl\nCCT\tbash\t/bin/bash\n\
             CCG\tNVIDIA GeForce RTX 4090\nCCO\tLinux x86_64\nCCR\tUbuntu 22.04.4 LTS\n",
        );
        let v = to_json(std::slice::from_ref(&p));
        let h = &v["hosts"][0];
        assert_eq!(h["os_name"], "Ubuntu 22.04.4 LTS");
        assert_eq!(h["gpu"], "NVIDIA GeForce RTX 4090");
        let a = h["agents"].as_array().unwrap();
        let get = |id: &str| a.iter().find(|x| x["id"] == id).unwrap();
        let codex = get("codex");
        assert_eq!(codex["glyph"], icons::brand("codex").0);
        assert!(codex["color"].as_str().unwrap().starts_with('#'));
        assert_eq!(codex["on_path"], true);
        assert_eq!(codex["check"], "verified");
        assert_eq!(codex["start"], "herdr agent start --kind codex");
        let oc = get("opencode");
        assert_eq!(oc["on_path"], false);
        // not installed: how, and what's missing
        let gem = get("gemini");
        assert_eq!(gem["installed"], false);
        assert_eq!(gem["install_command"], "npm i -g @google/gemini-cli");
        assert_eq!(gem["install_ready"], false);
        let pre = gem["prerequisites"].as_array().unwrap();
        assert!(pre.iter().any(|x| x["what"] == "npm" && x["ok"] == false));
        let claude = get("claude");
        assert_eq!(claude["install_ready"], true);
        assert_eq!(claude["install_method"], "script");
        assert_eq!(
            get("grok")["install_command"],
            "curl -fsSL https://x.ai/cli/install.sh | bash"
        );
        // notes: opencode isn't on the shell's PATH
        let n: Vec<String> = notes(&p)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert!(
            n.iter()
                .any(|l| l.contains("OpenCode: ~/.opencode/bin isn't on your shell's PATH")),
            "{n:?}"
        );
    }

    #[test]
    fn install_hosts_and_args() {
        let cfg = Config::in_memory(&["ssh_host=b31", "hosts=b31,gpu2"]);
        let rest: Vec<String> = ["install", "codex", "--host", "gpu2", "--yes"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = Args {
            cfg,
            flag: None,
            rest: &rest,
        };
        assert_eq!(explicit_hosts(&a), vec!["gpu2".to_string()]);
        assert_eq!(words(a.rest), vec!["install", "codex"]);
        // the fleet when nothing's named (listing), but not for install
        let rest2: Vec<String> = vec!["install".into(), "codex".into()];
        let b = Args {
            cfg: Config::in_memory(&["ssh_host=b31", "hosts=b31,gpu2"]),
            flag: None,
            rest: &rest2,
        };
        assert!(explicit_hosts(&b).is_empty());
        assert_eq!(hosts_for(&b), vec!["b31".to_string(), "gpu2".to_string()]);
    }

    #[test]
    fn listing_includes_this_computer() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
        // the fleet as the dashboard shows it: `local` stays in
        assert_eq!(
            pick_hosts(vec![], || s(&["b31", "local"]), false),
            s(&["b31", "local"])
        );
        // --local adds it when the fleet doesn't (its `local` setting off, no herdr here), once
        assert_eq!(
            pick_hosts(vec![], || s(&["b31"]), true),
            s(&["b31", "local"])
        );
        assert_eq!(
            pick_hosts(vec![], || s(&["b31", "local"]), true),
            s(&["b31", "local"])
        );
        // --host H: only that one
        assert_eq!(
            pick_hosts(s(&["gpu2"]), || s(&["b31", "local"]), false),
            s(&["gpu2"])
        );
        // `aleph agents` lists from the fleet, which has `local` whenever it's on and herdr is here
        let cfg = Config::in_memory(&["hosts=b31,gpu2", "local=true"]);
        let rest: Vec<String> = vec!["--json".into()];
        let a = Args {
            cfg,
            flag: None,
            rest: &rest,
        };
        assert_eq!(list_hosts(&a), a.cfg.fleet());
        assert_eq!(&list_hosts(&a)[..2], &s(&["b31", "gpu2"])[..]);
        let rest2: Vec<String> = vec!["--json".into(), "--local".into()];
        let b = Args {
            cfg: Config::in_memory(&["hosts=b31", "local=false"]),
            flag: None,
            rest: &rest2,
        };
        assert_eq!(list_hosts(&b), s(&["b31", "local"]));
    }
}
