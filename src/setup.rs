//! `aleph setup` — the first-run wizard (printed, not full-screen). Also runs
//! from the bare `claude` when there are no machines yet.
//!
//! Six steps, each skippable and safe to re-run (it only adds what's missing):
//!  1. machines      pick ssh aliases (or add a new one)
//!  2. connect       key login, herdr on each machine, saved in herdr (onboard.rs)
//!  3. agents        which agents each machine has; the default agent
//!  4. persistence   herdr integrations, so sessions survive a herdr restart
//!  5. notifications `aleph watch --install`
//!  6. completions   tab completion in the shell

use crate::agentcmd;
use crate::agents;
use crate::cli::Args;
use crate::config::{self, Config};
use crate::out::*;
use crate::pick::{self, Item};
use crate::probe::{HostProbe, Persist};
use crate::ui::{self, c};
use crate::{herdr, onboard, sshcfg, term, theme};
use ratatui::prelude::*;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

const STEPS: usize = 6;
const ADD: &str = "+ add a new host…";

#[derive(Default)]
struct Summary {
    /// (label, ok, text)
    rows: Vec<(String, Option<bool>, String)>,
}

impl Summary {
    fn add(&mut self, label: &str, ok: Option<bool>, text: impl Into<String>) {
        self.rows.push((label.to_string(), ok, text.into()));
    }
}

fn step(n: usize, title: &str, note: &str) {
    say(Line::raw(""));
    say(Line::from(vec![
        Span::styled(
            format!("[{n}/{STEPS}] "),
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(title.to_string(), ui::bold()),
        Span::styled(
            if note.is_empty() {
                String::new()
            } else {
                format!("  {note}")
            },
            ui::faint(),
        ),
    ]));
}

fn welcome(first: bool) {
    let row = |k: &str, v: &str| {
        Line::from(vec![
            Span::styled(format!("   {}", ui::pad(k, 14)), ui::fg(c::ACCENT())),
            Span::raw(v.to_string()),
        ])
    };
    let body = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", crate::brand::MARK),
                ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if first {
                    "Welcome to Aleph"
                } else {
                    "aleph setup"
                },
                ui::bold(),
            ),
            Span::styled(format!(" {}", env!("CARGO_PKG_VERSION")), ui::muted()),
        ]),
        Line::raw(""),
        Line::from(Span::raw(
            "  Coding agents in herdr, on all your ssh machines.",
        )),
        Line::from(Span::styled(
            "  About a minute. Nothing is installed without asking.",
            ui::muted(),
        )),
        Line::raw(""),
        row("machines", "pick hosts from your ssh config"),
        row("connect", "key login + herdr on each machine"),
        row("agents", "find installed agents, choose a default"),
        row("persistence", "sessions survive a herdr restart"),
        row("notifications", "tell you when a session needs you"),
        row("completions", "tab completion in your shell"),
        Line::raw(""),
        Line::from(Span::styled(
            "  esc or n skips a step · aleph setup runs it again",
            ui::faint(),
        )),
    ];
    let w = body.iter().map(ui::line_width).max().unwrap_or(40) + 6;
    for l in boxed(w.max(64).min(ui::print_width()), body) {
        say(l);
    }
}

/// `aleph setup [<step>] [--redo] [--plain] [--defaults] [--apply FILE [--dry-run] [--json]]`:
/// the full-screen wizard (`tui::onboard`) on a terminal, this line mode
/// with `--plain` or without one. `--defaults`: quick start — every default,
/// what this computer is missing installed at once, then the dashboard.
pub fn cmd(a: Args) -> ExitCode {
    use crate::tui::onboard::{self, Step};
    if a.has("--defaults") {
        if !term::interactive() {
            return crate::out::fail(
                "--defaults installs on the install screen: run it in a terminal",
            );
        }
        return onboard::run(
            a.cfg,
            onboard::Opts {
                first: true,
                defaults: true,
                ..Default::default()
            },
        );
    }
    if let Some(i) = a.rest.iter().position(|x| x == "--apply") {
        let Some(f) = a.rest.get(i + 1).filter(|f| !f.starts_with('-')) else {
            return crate::out::fail(
                "--apply needs a file: aleph setup --apply FILE.toml [--dry-run]",
            );
        };
        let (dry, js) = (a.has("--dry-run"), a.has("--json"));
        let f = f.clone();
        return onboard::declare::run(a.cfg, &f, dry, js);
    }
    let only = match a.positional().first() {
        None => None,
        Some(s) => match Step::parse(s) {
            Some(st) => Some(st),
            None => {
                err(&format!("no setup step '{s}'"));
                hint(&format!(
                    "steps: {}",
                    Step::ALL
                        .iter()
                        .map(|s| s.id())
                        .collect::<Vec<_>>()
                        .join(" · ")
                ));
                return ExitCode::from(2);
            }
        },
    };
    let redo = a.has("--redo");
    if a.has("--plain") || !term::interactive() {
        return match only {
            None => wizard(a.cfg, false).0,
            Some(s) => plain_step(a.cfg, s),
        };
    }
    onboard::run(
        a.cfg,
        onboard::Opts {
            redo,
            only,
            ..Default::default()
        },
    )
}

/// One step in line mode (`aleph setup machines --plain`).
fn plain_step(mut cfg: Config, s: crate::tui::onboard::Step) -> ExitCode {
    use crate::tui::onboard::Step;
    match s {
        Step::Machines | Step::Herdr => {
            let v = if s == Step::Machines {
                step(1, "Machines", "the ssh hosts your agents run on");
                match machines(&cfg) {
                    Some(v) if !v.is_empty() => v,
                    _ => {
                        say(skipped("no machines chosen"));
                        return ExitCode::SUCCESS;
                    }
                }
            } else {
                cfg.hosts()
            };
            if let Err(e) = include(&mut cfg, &v) {
                err(&format!("couldn't save the fleet: {e}"));
                return ExitCode::from(1);
            }
            step(2, "Connect", "key login, herdr, saved as a herdr machine");
            let mut bad = false;
            for alias in &v {
                let m = cfg.target_for(alias, herdr::machine_for).machine;
                bad |= !onboard::run(&mut cfg, alias, &m);
            }
            ExitCode::from(u8::from(bad))
        }
        Step::Agents => {
            step(3, "Agents", "what's installed on each machine");
            let hosts = cfg.hosts();
            if hosts.is_empty() {
                say(skipped("no machines yet — aleph setup machines"));
                return ExitCode::SUCCESS;
            }
            let probes = agentcmd::probe_with_spinner(&hosts);
            agentcmd::print_matrix(&cfg, &probes, false);
            default_agent(&mut cfg, &probes);
            step(
                4,
                "Persistence",
                "herdr restores agents that have its integration",
            );
            match agentcmd::setup_flow(&cfg, &probes, &[], false) {
                Some(false) => ExitCode::from(1),
                _ => ExitCode::SUCCESS,
            }
        }
        Step::Notify | Step::Access => {
            step(
                5,
                "Notifications",
                "the daemon at login tells you when a session needs you",
            );
            let (o, _) = notifications();
            ExitCode::from(u8::from(o == Some(false)))
        }
        Step::Integrations => {
            step(6, "Completions", "tab completion for aleph in your shell");
            let (o, _) = completions();
            ExitCode::from(u8::from(o == Some(false)))
        }
        other => {
            err(&format!("{} has no line mode", other.title()));
            hint(&format!(
                "run it in a terminal (aleph setup {}), or set its options with aleph settings set …",
                other.id()
            ));
            ExitCode::from(2)
        }
    }
}

/// The bare `aleph` with no machines: the wizard, then the dashboard.
pub fn first_run(a: Args) -> ExitCode {
    if term::interactive() {
        return crate::tui::onboard::run(
            a.cfg,
            crate::tui::onboard::Opts {
                first: true,
                ..Default::default()
            },
        );
    }
    let (code, cfg) = wizard(a.cfg, true);
    if cfg.hosts().is_empty() || !term::interactive() {
        return code;
    }
    if yes("Open the dashboard now?") {
        return crate::tui::run_dashboard(
            Config::load(),
            crate::tui::Start {
                view: crate::tui::View::Fleet,
                ..Default::default()
            },
        );
    }
    code
}

fn wizard(mut cfg: Config, first: bool) -> (ExitCode, Config) {
    welcome(first);
    let mut sum = Summary::default();

    // 1 ─ machines
    step(1, "Machines", "the ssh hosts your agents run on");
    let chosen = machines(&cfg);
    match &chosen {
        Some(v) if !v.is_empty() => sum.add("machines", Some(true), v.join(" · ")),
        _ if !cfg.hosts().is_empty() => sum.add("machines", Some(true), cfg.hosts().join(" · ")),
        _ => sum.add(
            "machines",
            None,
            "none yet — aleph host add <alias> <user@host>",
        ),
    }

    // 2 ─ connect
    step(2, "Connect", "key login, herdr, saved as a herdr machine");
    let mut ready: Vec<String> = Vec::new();
    match chosen.filter(|v| !v.is_empty()) {
        None => say(skipped("no machines chosen")),
        Some(v) => {
            if let Err(e) = include(&mut cfg, &v) {
                err(&format!("couldn't save the fleet: {e}"));
            }
            for alias in &v {
                say(Line::from(vec![
                    Span::styled("▌", Style::new().fg(theme::machine(&cfg, alias))),
                    Span::styled(
                        alias.clone(),
                        Style::new()
                            .fg(theme::machine(&cfg, alias))
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                let m = cfg.target_for(alias, herdr::machine_for).machine;
                if onboard::run(&mut cfg, alias, &m) {
                    ready.push(alias.clone());
                } else {
                    sum.add(
                        alias,
                        Some(false),
                        format!("not ready — aleph doctor --host {alias}"),
                    );
                }
            }
        }
    }
    if !ready.is_empty() {
        sum.add("connected", Some(true), ready.join(" · "));
    }

    // 3 ─ agents
    step(3, "Agents", "what's installed on each machine");
    let hosts = cfg.hosts();
    let probes: Vec<HostProbe> = if hosts.is_empty() {
        say(skipped("no machines yet"));
        vec![]
    } else {
        let p = agentcmd::probe_with_spinner(&hosts);
        agentcmd::print_matrix(&cfg, &p, false);
        p
    };
    match default_agent(&mut cfg, &probes) {
        Some(a) => sum.add(
            "agent",
            Some(true),
            format!("{} · default for aleph new", a.name),
        ),
        None => sum.add(
            "agent",
            Some(true),
            format!(
                "{} (unchanged) · aleph settings set agent <id>",
                agents::default_agent(&cfg).name
            ),
        ),
    }
    // OpenRouter: the key is never asked for here (it would sit in a
    // terminal's scrollback): `aleph secret set openrouter` reads it from stdin
    if crate::openrouter::configured(&cfg) && crate::openrouter::key(&cfg).is_some() {
        sum.add(
            "openrouter",
            Some(true),
            format!(
                "key in {} · OPENROUTER_API_KEY for {}",
                crate::secrets::store_name(),
                cfg.get_or("openrouter.agents")
            ),
        );
    } else {
        sum.add(
            "openrouter",
            None,
            "no key (optional) · aleph secret set openrouter, or the full-screen setup",
        );
    }

    // 4 ─ persistence
    step(
        4,
        "Persistence",
        "herdr restores agents that have its integration",
    );
    if probes.iter().any(HostProbe::ok) {
        let res = agentcmd::setup_flow(&cfg, &probes, &[], false);
        // what's still missing: nothing once installed, else the plan
        let left = match res {
            Some(true) => vec![],
            _ => agentcmd::plan(&probes, &[]),
        };
        let none: Vec<String> = probes
            .iter()
            .flat_map(|p| {
                p.installed()
                    .into_iter()
                    .filter(|(ag, _)| p.persist(ag) == Persist::None)
                    .map(|(ag, _)| ag.id.to_string())
                    .collect::<Vec<_>>()
            })
            .fold(Vec::new(), |mut v, x| {
                if !v.contains(&x) {
                    v.push(x);
                }
                v
            });
        if !none.is_empty() {
            say(Line::from(Span::styled(
                format!(
                    "  {} ha{} no herdr integration: aleph restore recreates those sessions after a reboot",
                    none.join(", "),
                    if none.len() == 1 { "s" } else { "ve" }
                ),
                ui::faint(),
            )));
        }
        sum.add(
            "persistence",
            res.filter(|_| left.is_empty() || res == Some(false)),
            if res == Some(false) {
                "some integrations failed · aleph agents setup".to_string()
            } else if left.is_empty() {
                "integrations in place".to_string()
            } else {
                format!("{} integration(s) offered · aleph agents setup", left.len())
            },
        );
    } else {
        say(skipped("no machine answered"));
        sum.add("persistence", None, "skipped — aleph agents setup");
    }

    // 5 ─ notifications
    step(
        5,
        "Notifications",
        "a desktop notification when a session needs you",
    );
    let (o, t) = notifications();
    sum.add("notifications", o, t);

    // 6 ─ completions
    step(6, "Completions", "tab completion for claude in your shell");
    let (o, t) = completions();
    sum.add("completions", o, t);

    // summary
    say(Line::raw(""));
    say(Line::from(vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled("Setup", ui::bold()),
    ]));
    let wl = sum.rows.iter().map(|r| ui::width(&r.0)).max().unwrap_or(8);
    for (label, okv, text) in &sum.rows {
        let (m, col) = match okv {
            Some(true) => ("✓", c::GREEN()),
            Some(false) => ("✗", c::RED()),
            None => ("○", c::YELLOW()),
        };
        say(Line::from(vec![
            Span::styled(format!("  {m} "), ui::fg(col).add_modifier(Modifier::BOLD)),
            Span::styled(ui::pad(label, wl), ui::muted()),
            Span::raw("  "),
            Span::raw(text.clone()),
        ]));
    }
    say(Line::raw(""));
    say(Line::from(Span::styled(
        "Next",
        ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
    )));
    for (k, v) in [
        ("claude", "the dashboard: every machine, live"),
        (
            "aleph new",
            "start a session (--agent codex, --dir /srv/x, -m \"…\")",
        ),
        ("aleph agents", "agents and integrations per machine"),
        ("aleph update --check", "herdr and agents across the fleet"),
        ("aleph inbox", "what needed you while you were away"),
        ("aleph restore", "bring sessions back after a reboot"),
    ] {
        say(Line::from(vec![
            Span::styled(format!("  {}", ui::pad(k, 24)), ui::bold()),
            Span::styled(v.to_string(), ui::muted()),
        ]));
    }
    let bad = sum.rows.iter().any(|r| r.1 == Some(false));
    (
        if bad {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        },
        cfg,
    )
}

fn skipped(why: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("○ ", ui::fg(c::YELLOW())),
        Span::styled(format!("skipped — {why}"), ui::muted()),
    ])
}

// ------------------------------------------------------------------ 1 machines

fn machines(cfg: &Config) -> Option<Vec<String>> {
    let fleet = cfg.hosts();
    let mut all = fleet.clone();
    for a in sshcfg::aliases() {
        if !all.contains(&a) {
            all.push(a);
        }
    }
    // resolve addresses in parallel (ssh -G, local, 5s cap each)
    let hints: Vec<String> = all
        .iter()
        .map(|a| {
            let a = a.clone();
            std::thread::spawn(move || sshcfg::resolve(&a).map(|r| r.short()).unwrap_or_default())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap_or_default())
        .collect();
    let mut items: Vec<Item> = all
        .iter()
        .zip(&hints)
        .map(|(a, h)| {
            let inf = fleet.contains(a);
            Item::new(
                a.clone(),
                if inf {
                    format!("{h} · in the fleet")
                } else {
                    h.clone()
                },
                inf,
            )
        })
        .collect();
    items.push(Item::new(ADD, "define one in ~/.ssh/config", false));
    if all.is_empty() {
        say(Line::from(Span::styled(
            format!(
                "  No Host entries in {} yet.",
                crate::doctor::tilde(&sshcfg::user_config())
            ),
            ui::muted(),
        )));
    }
    let picked = pick::multi("Machines", &items)?;
    let mut out: Vec<String> = Vec::new();
    for i in picked {
        if items[i].label == ADD {
            if let Some(a) = add_host() {
                out.push(a);
            }
        } else {
            out.push(items[i].label.clone());
        }
    }
    Some(out)
}

/// Ask for an alias and `user@host[:port]`, write the Host block.
fn add_host() -> Option<String> {
    if !io::stdin().is_terminal() {
        return None;
    }
    let alias = ask("Alias for the new host", "e.g. gpu3");
    if alias.is_empty() {
        return None;
    }
    if let Err(e) = sshcfg::valid_alias(&alias) {
        err(&e);
        return None;
    }
    if sshcfg::aliases().contains(&alias) {
        ok(&format!("{alias} is already in your ssh config"));
        return Some(alias);
    }
    let tgt = ask("Connect to", "user@hostname[:port]");
    let (user, hostname, port) = match sshcfg::parse_target(&tgt) {
        Ok(x) => x,
        Err(e) => {
            err(&e);
            return None;
        }
    };
    let h = sshcfg::NewHost {
        alias: alias.clone(),
        user,
        hostname,
        port,
        identity: None,
        jump: None,
    };
    match sshcfg::add_host(&h) {
        Ok(b) => {
            ok(&format!(
                "Added Host {alias} to {}",
                crate::doctor::tilde(&sshcfg::user_config())
            ));
            if let Some(b) = b {
                say(Line::from(Span::styled(
                    format!("  backup: {}", crate::doctor::tilde(&b)),
                    ui::faint(),
                )));
            }
            Some(alias)
        }
        Err(e) => {
            err(&e);
            None
        }
    }
}

/// Add hosts to the fleet (never removes), default host if none, colours.
fn include(cfg: &mut Config, v: &[String]) -> std::io::Result<()> {
    let mut hosts = cfg.hosts();
    let mut changed = false;
    for a in v {
        if !hosts.contains(a) {
            hosts.push(a.clone());
            changed = true;
        }
    }
    if cfg.get("ssh_host").is_none() {
        if let Some(f) = v.first() {
            cfg.set("ssh_host", f)?;
        }
    }
    if changed {
        cfg.set_hosts(&hosts)?;
    }
    theme::assign_machine_colors(cfg);
    Ok(())
}

// ------------------------------------------------------------------ 3 agents

/// Installed agents first (where), then the rest. Pure — tested.
pub fn agent_items(probes: &[HostProbe], current: &str) -> (Vec<(&'static str, Item)>, usize) {
    let mut v: Vec<(&'static str, Item)> = Vec::new();
    for ag in agents::ALL {
        let on: Vec<String> = probes
            .iter()
            .filter(|p| p.agent(ag.id).is_some())
            .map(|p| p.host.clone())
            .collect();
        if !on.is_empty() {
            v.push((
                ag.id,
                Item::new(ag.name, format!("on {}", on.join(", ")), false),
            ));
        }
    }
    for ag in agents::ALL {
        if !v.iter().any(|x| x.0 == ag.id) {
            v.push((
                ag.id,
                Item::new(ag.name, "not found on your machines", false),
            ));
        }
    }
    let i = v.iter().position(|x| x.0 == current).unwrap_or(0);
    v[i].1.hint.push_str(" · current");
    (v, i)
}

fn default_agent(cfg: &mut Config, probes: &[HostProbe]) -> Option<&'static agents::Agent> {
    let cur = cfg.get_or("agent");
    let (v, i) = agent_items(probes, &cur);
    let items: Vec<Item> = v.iter().map(|x| x.1.clone()).collect();
    let pick = pick::single("Default agent", &items, i)?;
    let id = v[pick].0;
    if id != cur {
        if let Err(e) = cfg.set("agent", id) {
            err(&format!("couldn't save: {e}"));
            return None;
        }
    }
    agents::find(id)
}

// ------------------------------------------------------------------ 5 notifications

fn planned(cmd: &str) -> bool {
    crate::PLANNED.iter().any(|p| p.0 == cmd)
}

pub(crate) fn watch_installed() -> bool {
    let h = config::home();
    let unit_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| h.join(".config"))
        .join("systemd/user");
    [crate::brand::LABEL, crate::brand::LEGACY_LABEL]
        .iter()
        .any(|l| h.join(format!("Library/LaunchAgents/{l}.plist")).exists())
        || [crate::brand::UNIT, crate::brand::LEGACY_UNIT]
            .iter()
            .any(|u| unit_dir.join(u).exists())
}

fn notifications() -> (Option<bool>, String) {
    if planned("watch") {
        say(skipped("aleph watch isn't in this build yet"));
        return (
            None,
            "not available yet — aleph watch --install later".into(),
        );
    }
    if watch_installed() {
        ok("aleph watch already runs at login");
        return (Some(true), "aleph watch runs at login".into());
    }
    if !yes("Run aleph watch at login to notify you? (launchd / systemd --user)") {
        say(skipped("aleph watch --install does it later"));
        return (None, "skipped — aleph watch --install".into());
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("aleph"));
    match crate::proc::interactive(&exe.to_string_lossy(), &["watch", "--install"]) {
        Ok(true) => (Some(true), "aleph watch runs at login".into()),
        Ok(false) => (Some(false), "aleph watch --install failed".into()),
        Err(e) => (Some(false), format!("couldn't run aleph watch: {e}")),
    }
}

// ------------------------------------------------------------------ 6 completions

/// (shell, rc file, line to add) for the user's shell.
pub fn completion_target(
    shell: &str,
    home: &std::path::Path,
    exe: &str,
) -> Option<(&'static str, PathBuf, String)> {
    let name = std::path::Path::new(shell)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.as_str() {
        "zsh" => Some((
            "zsh",
            std::env::var_os("ZDOTDIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.to_path_buf())
                .join(".zshrc"),
            format!("source <({exe} completions zsh)"),
        )),
        "bash" => Some((
            "bash",
            home.join(".bashrc"),
            format!("source <({exe} completions bash)"),
        )),
        "fish" => Some((
            "fish",
            home.join(".config/fish/conf.d/aleph.fish"),
            format!("{exe} completions fish | source"),
        )),
        _ => None,
    }
}

const C_BEGIN: &str = "# >>> aleph completions >>>";
const C_END: &str = "# <<< aleph completions <<<";

/// Pure: the rc file with our completion block added (None = already there).
pub fn with_completions(rc: &str, line: &str) -> Option<String> {
    if rc.contains(C_BEGIN)
        || rc.lines().any(|l| {
            l.contains(" completions ")
                && (l.contains("aleph") || l.contains("claude-herdr") || l.contains("claude"))
        })
    {
        return None;
    }
    let mut s = rc.to_string();
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s.push_str(&format!("{C_BEGIN}\n{line}\n{C_END}\n"));
    Some(s)
}

fn completions() -> (Option<bool>, String) {
    if planned("completions") {
        say(skipped("aleph completions isn't in this build yet"));
        return (None, "not available yet".into());
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let exe = if crate::proc::which("aleph").is_some() {
        "aleph".to_string()
    } else {
        std::env::current_exe()
            .map(|p| crate::proc::sh_quote(&p.to_string_lossy()))
            .unwrap_or_else(|_| "aleph".into())
    };
    let Some((sh, rc, line)) = completion_target(&shell, &config::home(), &exe) else {
        say(skipped(&format!(
            "unknown shell '{shell}' — aleph completions zsh|bash|fish"
        )));
        return (None, "skipped — aleph completions <shell>".into());
    };
    let cur = std::fs::read_to_string(&rc).unwrap_or_default();
    let Some(new) = with_completions(&cur, &line) else {
        ok(&format!(
            "{sh} completion is already in {}",
            crate::doctor::tilde(&rc)
        ));
        return (Some(true), format!("{sh} · {}", crate::doctor::tilde(&rc)));
    };
    if !yes(&format!(
        "Add {sh} completion to {}?",
        crate::doctor::tilde(&rc)
    )) {
        say(skipped(&format!("add this line yourself: {line}")));
        return (None, "skipped".into());
    }
    match config::write_atomic(&rc, new.as_bytes()) {
        Ok(()) => {
            ok(&format!(
                "added to {} — open a new shell to use it",
                crate::doctor::tilde(&rc)
            ));
            (Some(true), format!("{sh} · {}", crate::doctor::tilde(&rc)))
        }
        Err(e) => (Some(false), format!("couldn't write {}: {e}", rc.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe;

    #[test]
    fn installed_agents_first() {
        let p = probe::parse("b31", "CCA\tcodex\t/b/codex\t0.4\nCCO\tL\n");
        let (v, i) = agent_items(&[p], "claude");
        assert_eq!(v[0].0, "codex");
        assert_eq!(v[0].1.hint, "on b31");
        assert_eq!(v[i].0, "claude");
        assert!(v[i].1.hint.ends_with("· current"));
        assert_eq!(v.len(), agents::ALL.len());
    }

    #[test]
    fn completion_block_is_idempotent() {
        let home = std::path::Path::new("/h");
        let (sh, rc, line) = completion_target("/bin/zsh", home, "aleph").unwrap();
        assert_eq!(sh, "zsh");
        assert!(rc.ends_with(".zshrc"));
        let once = with_completions("export X=1", &line).unwrap();
        assert!(once.starts_with(
            "export X=1\n# >>> aleph completions >>>\nsource <(aleph completions zsh)\n"
        ));
        assert!(with_completions(&once, &line).is_none());
        assert!(completion_target("/usr/bin/tcsh", home, "x").is_none());
        let (_, rc, l) = completion_target("/opt/homebrew/bin/fish", home, "aleph").unwrap();
        assert!(rc.ends_with("conf.d/aleph.fish") && l.ends_with("| source"));
    }
}
