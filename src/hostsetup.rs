//! `aleph host setup <machine>`: everything a machine needs to run agents,
//! in one pass, asking before anything is installed —
//!
//! 1. key login and herdr: installed (or updated to this computer's
//!    version) and the machine saved in herdr (`onboard.rs`);
//! 2. its agents: a checklist of every agent Aleph knows, the ones there
//!    ticked; installed one after another, each validated and verified,
//!    failures explained with retry / skip / the log; herdr's integrations
//!    for them offered at the end (`agentsetup.rs`);
//! 3. the running Aleph is told (`refresh.rs`): the daemon and any open
//!    dashboard look at the machine again — its sessions, herdr's version,
//!    its agents — and the dashboard says `box3 set up · herdr 0.9.5 · 2 agents`.
//!
//! Adding a machine — the dashboard's `+`, `aleph host add` / `include` —
//! opens this in a new terminal tab when the machine has no herdr or no
//! agent yet ([`needs`], [`open_tab`]), so Aleph stays where it is.

use crate::agentsetup;
use crate::config::Config;
use crate::onboard::{self, Onboard};
use crate::out::*;
use crate::probe::{self, HostProbe};
use crate::refresh::{self, Note};
use crate::ui::{self, c};
use crate::{agents, herdr, sshcfg, termlaunch};
use ratatui::prelude::*;
use std::io::{self, IsTerminal};
use std::process::ExitCode;

/// What a machine is missing for agent sessions (pure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Needs {
    pub herdr: bool,
    /// none of the registry's agents is there
    pub agents: bool,
}

impl Needs {
    pub fn any(&self) -> bool {
        self.herdr || self.agents
    }
    /// `no herdr and no agents yet`
    pub fn text(&self) -> String {
        match (self.herdr, self.agents) {
            (true, true) => "no herdr and no agents yet".into(),
            (true, false) => "no herdr yet".into(),
            (false, true) => "no agents yet".into(),
            (false, false) => "everything it needs".into(),
        }
    }
}

/// What the machine is missing, from a probe of it.
pub fn needs(p: &HostProbe) -> Needs {
    Needs {
        herdr: p.herdr.is_none(),
        agents: p.installed().is_empty(),
    }
}

/// The command a setup tab runs.
pub fn tab_cmd(alias: &str) -> String {
    termlaunch::self_cmd(&["host", "setup", alias, "--tab"])
}

/// Open `aleph host setup <alias>` in a new tab of this terminal.
pub fn open_tab(cfg: &Config, alias: &str) -> Result<termlaunch::App, String> {
    termlaunch::open(
        &cfg.get_or("terminal"),
        &tab_cmd(alias),
        &format!("Aleph · set up {alias}"),
    )
}

#[derive(Debug, Clone, Default)]
pub struct Opts {
    /// yes to every install it would ask about (`--yes`)
    pub yes: bool,
    /// these agents, no checklist (`--agents a,b`)
    pub agents: Option<Vec<&'static str>>,
    /// leave herdr alone (`--no-herdr`) / the agents (`--no-agents`)
    pub no_herdr: bool,
    pub no_agents: bool,
    /// no Node.js for the npm ones, whatever `agents.node` says (`--no-node`)
    pub no_node: bool,
    /// running in a tab Aleph opened (`--tab`)
    pub tab: bool,
}

const USAGE: &str =
    "usage: aleph host setup [<machine>] [--agents a,b] [--yes] [--no-herdr] [--no-agents] [--no-node]";

/// `aleph host setup [<machine>] [--agents a,b] [--yes] [--no-herdr] [--no-agents] [--tab]`.
pub fn cmd(mut cfg: Config, flag: Option<String>, rest: &[String]) -> ExitCode {
    let mut o = Opts::default();
    let mut alias: Option<String> = None;
    let mut i = 1;
    while i < rest.len() {
        let a = rest[i].as_str();
        match a {
            "--yes" | "-y" => o.yes = true,
            "--no-herdr" => o.no_herdr = true,
            "--no-agents" => o.no_agents = true,
            "--no-node" => o.no_node = true,
            "--tab" => o.tab = true,
            "--agents" | "--agent" => {
                i += 1;
                let Some(v) = rest.get(i) else {
                    return fail(USAGE);
                };
                match parse_agents(v) {
                    Ok(v) => o.agents = Some(v),
                    Err(e) => return fail(&e),
                }
            }
            _ if a.starts_with("--agents=") => match parse_agents(&a["--agents=".len()..]) {
                Ok(v) => o.agents = Some(v),
                Err(e) => return fail(&e),
            },
            _ if a.starts_with('-') => return fail(USAGE),
            _ if alias.is_none() => alias = Some(a.to_string()),
            _ => return fail(USAGE),
        }
        i += 1;
    }
    let alias = match alias.or(flag).or_else(|| cfg.get("ssh_host")) {
        Some(a) => a,
        None => return crate::cli::no_host(),
    };
    if !o.yes && !io::stdin().is_terminal() {
        return fail(
            "aleph host setup asks before installing — run it in a terminal, or pass --yes (--agents a,b says which)",
        );
    }
    if o.tab {
        crate::term::set_title(&format!("{} — set up {alias}", crate::brand::BIN));
    }
    if run(&mut cfg, &alias, &o) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `claude,codex` → registry ids (did-you-mean for a typo).
fn parse_agents(v: &str) -> Result<Vec<&'static str>, String> {
    let mut out = Vec::new();
    for q in v.split([',', ' ']).map(str::trim).filter(|q| !q.is_empty()) {
        match agents::resolve(q)? {
            agents::SHELL => return Err("shell isn't an agent — there's nothing to install".into()),
            id if !out.contains(&id) => out.push(id),
            _ => {}
        }
    }
    Ok(out)
}

fn step(n: usize, of: usize, title: &str, note: &str) {
    say(Line::raw(""));
    say(Line::from(vec![
        Span::styled(
            format!("[{n}/{of}] "),
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

/// Set the machine up, printed; asks on a terminal. True when herdr is
/// ready there and every chosen agent installed.
pub fn run(cfg: &mut Config, alias: &str, o: &Opts) -> bool {
    if let Some(e) = crate::showcase::refused() {
        crate::out::err(&e);
        return false;
    }
    say(Line::from(vec![
        Span::styled(
            format!("{} ", crate::brand::MARK),
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("Setting up {}", agentsetup::on(alias)), ui::bold()),
        Span::styled(
            "  · herdr, then its agents · nothing is installed without asking",
            ui::faint(),
        ),
    ]));
    let of = usize::from(!o.no_herdr) + usize::from(!o.no_agents);
    let mut n = 0;
    let mut herdr_ok = true;
    if !o.no_herdr {
        n += 1;
        step(n, of, "herdr", "key login, herdr there, saved in herdr");
        let machine = cfg.target_for(alias, herdr::machine_for).machine;
        match onboard::run_with(cfg, alias, &machine, o.yes) {
            Onboard::Ready => {}
            Onboard::NotReady => herdr_ok = false,
            Onboard::NoLogin => {
                hint(&format!(
                    "aleph host setup {alias} picks up from here once it answers"
                ));
                return false;
            }
        }
    }
    let sp = Spinner::start(format!("Looking at {}", agentsetup::on(alias)));
    let p = probe::run(alias);
    sp.stop();
    if let Some(e) = &p.error {
        err(&format!("{alias}: {e}"));
        return false;
    }
    let (agents_ok, note) = if o.no_agents {
        (true, Note::from_probe("setup", &p))
    } else {
        n += 1;
        step(n, of, "Agents", "choose, install, check each one runs");
        let rep = agentsetup::run(
            cfg,
            p,
            o.agents.clone(),
            agentsetup::Opts {
                yes: o.yes,
                dry_run: false,
                integrations: o.yes.then_some(true),
                node: o.no_node.then_some(false),
            },
        );
        (
            rep.ok() || rep.results.is_empty() && !rep.declined,
            Note::from_report("setup", &rep),
        )
    };
    // tell the running Aleph: it looks at the machine again now
    say(Line::raw(""));
    let t = refresh::signal(&note);
    say(Line::from(Span::styled(
        format!("  {}", refresh::told_line(alias, &t)),
        ui::faint(),
    )));
    let (good, text) = note.toast();
    let all = good && herdr_ok && agents_ok;
    if all {
        ok(&text);
    } else {
        warn(&format!(
            "{text} — aleph host setup {alias} picks up where this left off"
        ));
    }
    if o.tab {
        hint("you can close this tab");
    }
    all
}

/// How [`after_join`] should set a machine up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    /// a new tab when there's a terminal, else the checklist here
    Auto,
    /// on this terminal (`--here`)
    Here,
}

/// A machine just joined the fleet (`aleph host add` / `include`). One look
/// at it: with herdr and an agent there, the usual checklist (and saving it
/// in herdr); without, its setup opens in a new tab (or runs here).
pub fn after_join(cfg: &mut Config, alias: &str, w: Where) {
    let machine = cfg.target_for(alias, herdr::machine_for).machine;
    if sshcfg::is_local(alias) {
        onboard::run(cfg, alias, &machine);
        return;
    }
    let sp = Spinner::start(format!("Looking at {alias}"));
    let p = probe::run(alias);
    sp.stop();
    let n = needs(&p);
    if !p.ok() || !n.any() {
        // it doesn't answer (onboard explains, offers key login), or it has
        // what it needs (onboard saves it in herdr when it isn't yet)
        onboard::run(cfg, alias, &machine);
        return;
    }
    let tty = io::stdin().is_terminal() && io::stderr().is_terminal();
    if w == Where::Auto && tty {
        match open_tab(cfg, alias) {
            Ok(app) => {
                ok(&format!(
                    "{alias} has {} — its setup is open in a new {} tab",
                    n.text(),
                    app.title()
                ));
                hint(&format!(
                    "it asks before installing anything; Aleph looks at {alias} again when it's done (aleph host setup {alias} runs it in this terminal)"
                ));
                return;
            }
            Err(e) => warn(&format!(
                "couldn't open a new tab ({e}) — setting {alias} up here"
            )),
        }
    }
    if !tty {
        onboard::run(cfg, alias, &machine);
        hint(&format!(
            "{alias} has {}: aleph host setup {alias} installs herdr and agents (it asks first)",
            n.text()
        ));
        return;
    }
    run(cfg, alias, &Opts::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_from_a_probe() {
        let bare = probe::parse("box3", "CCO\tLinux x86_64\nCCW\t/home/t\n");
        assert_eq!(
            needs(&bare),
            Needs {
                herdr: true,
                agents: true
            }
        );
        assert_eq!(needs(&bare).text(), "no herdr and no agents yet");
        let half = probe::parse("box3", "CCH\t/b/herdr\therdr 0.9.5\nCCO\tLinux\n");
        assert_eq!(needs(&half).text(), "no agents yet");
        let full = probe::parse(
            "b31",
            "CCA\tclaude\t/b/claude\t2.1.0\nCCH\t/b/herdr\therdr 0.9.5\nCCO\tLinux\n",
        );
        assert!(!needs(&full).any());
    }

    #[test]
    fn agents_are_parsed_and_checked() {
        assert_eq!(
            parse_agents("claude, codex").unwrap(),
            vec!["claude", "codex"]
        );
        assert_eq!(parse_agents("cc,claude").unwrap(), vec!["claude"]);
        assert!(parse_agents("shell").is_err());
        assert!(parse_agents("claud3").unwrap_err().contains("claude"));
    }

    #[test]
    fn the_tab_runs_host_setup() {
        let s = tab_cmd("box3");
        assert!(s.ends_with(" host setup box3 --tab"), "{s}");
    }
}
