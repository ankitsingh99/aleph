//! Step 5, Agents on each machine: what's installed (with versions and
//! logos); a checklist of every agent Aleph knows to install (the ones
//! there ticked; on a machine with none, the default agent and Claude Code /
//! Codex where they can install — said so; the exact command and what it
//! needs in the panel). The ticked ones install in Review, one terminal run
//! per machine (`aleph agents install …`: each checked, retry / skip / the
//! log on a failure). Then the default agent and how sessions are named,
//! an OpenRouter key (a masked input stored in the keystore the moment
//! enter is pressed — `secrets::set` off the UI thread — so setup's plan,
//! saved as plain JSON, never holds it; the plan only ever says
//! `openrouter.key = secret:openrouter`) and which agents get it,
//! herdr's integrations (so sessions survive a herdr restart) and Aleph's
//! own hooks — opt-in per machine, with the diff of the agent's settings
//! file they'd write.
//!
//! Also home to the machine probe both this step and herdr's use: one
//! `sh -s` per machine (`probe.rs`), cached.

use super::rows::{Kind, Row, St};
use super::{Action, Aside, Ev, Hand, Msg, Step, Wiz};
use crate::agents::{self, Agent};
use crate::probe::{self, Persist};
use crate::sshcfg;
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// What `hooks install` would change on a machine: (agent, diff) each.
pub(crate) type HookDiff = Result<Vec<(String, Vec<String>)>, String>;

#[derive(Default)]
pub(crate) struct State {
    pub inst_host: usize,
    /// what was ticked on each machine before you chose, and why
    pub picked: HashMap<String, String>,
    /// what `hooks install` would change, per machine
    pub hooks: HashMap<String, HookDiff>,
    /// machines whose missing integrations were put in the plan already
    pub planned: HashSet<String>,
    /// is an OpenRouter key in the keystore (None: not looked yet)
    pub openrouter: Option<bool>,
}

/// Look at this computer's herdr and every machine (cached probes).
pub(crate) fn ensure(w: &mut Wiz, refresh: bool) {
    if refresh || w.local_herdr.is_none() {
        w.spawn("herdr:local", "Looking at herdr here", || {
            let b = crate::herdr::bin();
            let local = crate::proc::which(&b).map(|_| {
                let o = crate::proc::cap(&b, &["--version"], Duration::from_secs(10));
                crate::doctor::version(&o.text())
            });
            // herdr 0.9.5 prints a bare array: read it whatever its shape
            let saved = crate::herdr::saved_names().unwrap_or_default();
            Msg::Herdr { local, saved }
        });
    }
    for h in w.fleet() {
        if refresh || !w.probes.contains_key(&h) {
            probe(w, &h, refresh);
        }
    }
}

pub(crate) fn probe(w: &mut Wiz, h: &str, refresh: bool) {
    if w.no_network() && !sshcfg::is_local(h) {
        return;
    }
    let host = h.to_string();
    w.spawn(
        &format!("probe:{h}"),
        format!("Looking at {h}"),
        move || {
            let p = probe::run_cached(&host, refresh);
            Msg::Probe(host, Box::new(p))
        },
    );
}

pub(crate) fn retry(w: &mut Wiz) {
    w.hd.server.clear();
    ensure(w, true);
    if w.cur == Step::Herdr {
        super::herdr_setup::enter(w);
    }
}

pub(crate) fn enter(w: &mut Wiz) {
    ensure(w, false);
    if w.ag.openrouter.is_none() {
        let cfg = w.cfg.clone();
        // (the Keychain is a subprocess: off the UI thread)
        w.spawn("openrouter", "Looking for an OpenRouter key", move || {
            let have =
                crate::openrouter::configured(&cfg) && crate::openrouter::key(&cfg).is_some();
            Msg::OpenRouter(have, None)
        });
    }
}

/// The OpenRouter key's row was answered: whether one is stored now, and
/// what storing it said.
pub(crate) fn openrouter(w: &mut Wiz, stored: bool, said: Option<(bool, String)>) {
    w.ag.openrouter = Some(stored);
    if let Some((bad, t)) = said {
        w.toast(bad, t);
    }
}

/// What the OpenRouter key's input asks (it's stored the moment you press
/// enter: setup's plan, saved as plain JSON, never holds it).
const OR_HINT: &str = "Paste your OpenRouter API key (openrouter.ai/settings/keys). It goes into the keychain now — never into setup's plan or any file of Aleph's — and sessions of the agents below get it as OPENROUTER_API_KEY.";

/// Store a pasted key (off the UI thread); the setting points at it.
fn store_openrouter(w: &mut Wiz, key: String) {
    let key = key.trim().to_string();
    if key.is_empty() {
        w.toast(false, "nothing stored — enter on the row pastes a key");
        return;
    }
    if key
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
    {
        w.toast(
            true,
            "that doesn't look like a key (spaces or quotes in it) — nothing stored",
        );
        return;
    }
    // the setting says where it lives (`secret:openrouter`, the default):
    // that, not the key, is what the plan holds
    let own = format!("secret:{}", crate::openrouter::SECRET);
    if w.value("openrouter.key") != own {
        w.set("openrouter.key", &own);
    }
    w.spawn(
        "openrouter",
        format!("Storing the key in {}", crate::secrets::store_name()),
        move || match crate::secrets::set(crate::openrouter::SECRET, &key) {
            Ok(where_) => {
                crate::openrouter::forget();
                Msg::OpenRouter(
                    true,
                    Some((
                        false,
                        format!("OpenRouter key stored in {where_} — not in setup's plan"),
                    )),
                )
            }
            Err(e) => Msg::OpenRouter(false, Some((true, format!("couldn't store the key: {e}")))),
        },
    );
}

/// A probe arrived: the first time, its missing integrations go in the plan
/// (Review lists them; untick any here) — and on a machine with no agent
/// yet, the sensible ones (the default agent and Claude Code / Codex, where
/// they can install), said so above the checklist.
pub(crate) fn probed(w: &mut Wiz, h: &str) {
    if w.ag.planned.contains(h) {
        return;
    }
    w.ag.planned.insert(h.to_string());
    if let Some(p) = w.probes.get(h).filter(|p| p.ok()) {
        let (ticked, note) = crate::agentsetup::defaults(&w.cfg, p);
        let restored = w
            .plan
            .actions
            .iter()
            .any(|(_, a)| matches!(a, Action::AgentInstall { host, .. } if host == h));
        if p.installed().is_empty() && !restored {
            for id in ticked {
                w.plan.want(
                    Step::Agents,
                    Action::AgentInstall {
                        host: h.to_string(),
                        agent: id.to_string(),
                    },
                    true,
                );
            }
        }
        w.ag.picked.insert(h.to_string(), note);
    }
    let restored = w.plan.actions.iter().any(|(s, a)| {
        *s == Step::Agents && matches!(a, Action::Integration { host, .. } if host == h)
    });
    if restored {
        return;
    }
    if let Some(p) = w.probes.get(h) {
        for s in crate::agentcmd::plan(std::slice::from_ref(p), &[]) {
            w.plan.want(
                Step::Agents,
                Action::Integration {
                    host: s.host.clone(),
                    name: s.integration.to_string(),
                },
                true,
            );
        }
    }
}

/// The herdr integration that comes with installing `ag` on `p`'s machine:
/// when herdr there offers it and doesn't have it (or herdr itself is on its
/// way: it'll have none).
pub(crate) fn integration_for(
    p: &probe::HostProbe,
    ag: &'static Agent,
    herdr_coming: bool,
) -> Option<Action> {
    let name = ag.integration?;
    let wanted = herdr_coming || matches!(p.persist(ag), Persist::Missing | Persist::Outdated);
    wanted.then(|| Action::Integration {
        host: p.host.clone(),
        name: name.to_string(),
    })
}

pub(crate) fn hooks(w: &mut Wiz, h: &str, r: Result<Vec<(String, Vec<String>)>, String>) {
    w.ag.hooks.insert(h.to_string(), r);
}

fn glyph(id: &str) -> Span<'static> {
    let (g, col) = crate::icons::brand(id);
    Span::styled(g.to_string(), ui::fg(col).add_modifier(Modifier::BOLD))
}

/// The machines the checklist can show (they answered).
fn answered(w: &Wiz) -> Vec<&probe::HostProbe> {
    w.fleet()
        .iter()
        .filter_map(|h| w.probes.get(h))
        .filter(|p| p.ok())
        .collect()
}

/// The checklist's machine.
fn shown(w: &Wiz) -> Option<&probe::HostProbe> {
    let v = answered(w);
    let i = w.ag.inst_host.min(v.len().checked_sub(1)?);
    v.get(i).copied()
}

/// One agent in the checklist: ticked when it's there or planned; its logo,
/// then its version, the command, or what it needs.
fn agent_row(w: &Wiz, p: &probe::HostProbe, x: &crate::agentsetup::Choice) -> Row {
    let a = x.agent;
    let planned = w.plan.has(&Action::AgentInstall {
        host: p.host.clone(),
        agent: a.id.to_string(),
    });
    let mut r = Row::pick(
        format!("inst:{}:{}", p.host, a.id),
        a.name,
        x.found.is_some() || planned,
    );
    let cmd = match (x.plan.command.clone().unwrap_or_default(), x.node) {
        (c, true) => format!("{c} · Node.js LTS first"),
        (c, false) => c,
    };
    let (text, col) = match (&x.found, crate::agentsetup::blocked(&w.cfg, p, &x.plan)) {
        (Some(f), _) if f.version.is_empty() => ("installed".to_string(), c::GREEN()),
        (Some(f), _) => match a.support(&f.version) {
            agents::Support::Old => (
                format!(
                    "installed {} · older than {}: untested, usage not tracked properly · {}",
                    f.version, a.min_version, a.upgrade
                ),
                c::YELLOW(),
            ),
            agents::Support::Yes => (format!("installed {} · supported", f.version), c::GREEN()),
            agents::Support::Unknown => (format!("installed {}", f.version), c::GREEN()),
        },
        (None, Some(crate::agentsetup::Why::Manual(_))) => {
            ("no install command".to_string(), c::FAINT())
        }
        (None, Some(why)) => (why.short(), c::YELLOW()),
        (None, None) if planned => (format!("installs in Review · {cmd}"), c::ACCENT()),
        (None, None) => (cmd, c::MUTED()),
    };
    r.note = vec![glyph(a.id), Span::raw(" "), Span::styled(text, ui::fg(col))];
    r.why("space ticks it: it installs in Review, in the terminal, one after another with its output; each one is checked (on your PATH, its --version runs, herdr sees it), and a failure says why, with retry / skip / the log. Ones already there stay ticked — Aleph never uninstalls. The panel shows the exact command and what it needs.")
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let fleet = w.fleet();
    let mut v = vec![Row::head("On each machine")];
    if fleet.is_empty() {
        v.push(Row::info("No machines picked yet (step 3)."));
    }
    for h in &fleet {
        let id = format!("agents:{h}");
        match w.probes.get(h) {
            None if w.running(&format!("probe:{h}")) => {
                v.push(Row::check(St::Busy, id, h.clone(), "looking".into()))
            }
            None => {
                v.push(Row::check(St::Todo, id, h.clone(), "not looked at yet".into()).fix("look"))
            }
            Some(p) if !p.ok() => v.push(
                Row::check(St::Bad, id, h.clone(), p.error.clone().unwrap_or_default())
                    .fix("look again"),
            ),
            Some(p) => {
                let inst = p.installed();
                if inst.is_empty() {
                    v.push(Row::check(
                        St::Warn,
                        id,
                        h.clone(),
                        "none of herdr's agents is installed".into(),
                    ));
                    continue;
                }
                let mut spans = Vec::new();
                let mut old = false;
                for (i, (ag, f)) in inst.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::styled(" · ", ui::faint()));
                    }
                    spans.push(glyph(ag.id));
                    spans.push(Span::raw(format!(" {}", ag.name)));
                    if ag.support(&f.version) == agents::Support::Old {
                        old = true;
                        spans.push(Span::styled(
                            format!(" {} ⚠ older than {}", f.version, ag.min_version),
                            ui::fg(c::YELLOW()),
                        ));
                    } else if !f.version.is_empty() {
                        spans.push(Span::styled(format!(" {}", f.version), ui::faint()));
                    }
                }
                let st = if old { St::Warn } else { St::Ok };
                let mut r = Row::check(st, id, h.clone(), String::new());
                r.note = spans;
                v.push(r.why("The agents found on the machine: its PATH, your shell's PATH and every folder installers use; each one's --version checked. aleph agents shows the same."));
            }
        }
    }
    // installs: every agent Aleph knows, as a checklist per machine
    v.push(Row::head("Agents to install"));
    let probed = answered(w);
    match shown(w) {
        None => v.push(Row::info(
            "Once a machine answers, every agent Aleph knows is listed here to tick.",
        )),
        Some(p) => {
            if probed.len() > 1 {
                v.push(
                    Row::local(
                        "inst-host",
                        "machine",
                        probed
                            .iter()
                            .map(|p| (p.host.clone(), p.host.clone()))
                            .collect(),
                        w.ag.inst_host.min(probed.len() - 1),
                    )
                    .why("Whose checklist this is: ←→ shows another machine's. Each machine has its own — ticking an agent here never ticks it anywhere else."),
                );
                // each machine's own choice, side by side (never one for all)
                let per: Vec<String> = probed
                    .iter()
                    .map(|q| {
                        let mine: Vec<String> = w
                            .plan
                            .actions
                            .iter()
                            .filter_map(|(_, a)| match a {
                                Action::AgentInstall { host, agent } if *host == q.host => {
                                    Some(agents::label(agent))
                                }
                                _ => None,
                            })
                            .collect();
                        format!(
                            "{} → {}",
                            q.host,
                            if mine.is_empty() {
                                "nothing new".to_string()
                            } else {
                                mine.join(", ")
                            }
                        )
                    })
                    .collect();
                v.push(Row::info(format!(
                    "Each machine has its own list: {}",
                    per.join(" · ")
                )));
            }
            if let Some(n) = w.ag.picked.get(&p.host) {
                v.push(Row::info(n.clone()));
            }
            let (ch, _) = crate::agentsetup::choices(&w.cfg, p);
            for x in &ch {
                v.push(agent_row(w, p, x));
            }
        }
    }
    // defaults
    v.push(Row::head("Defaults"));
    let cur = w.value("agent");
    // only agents that are there, or queued to be (and the one set now)
    let queued: Vec<String> = w
        .plan
        .actions
        .iter()
        .filter_map(|(_, a)| match a {
            Action::AgentInstall { agent, .. } => Some(agent.clone()),
            _ => None,
        })
        .collect();
    let there = |a: &Agent| probed.iter().any(|p| p.agent(a.id).is_some());
    let mut ids: Vec<&'static Agent> = agents::ALL
        .iter()
        .filter(|a| there(a) || queued.iter().any(|q| q == a.id) || a.id == cur)
        .collect();
    if ids.is_empty() {
        ids = agents::ALL.iter().collect();
    }
    ids.sort_by_key(|a| !there(a));
    v.push(
        Row::choice(
            "agent",
            "default agent",
            ids.iter().map(|a| (a.id.to_string(), a.name.to_string())).collect(),
            cur.clone(),
            w.pending("agent"),
        )
        .note(if probed.iter().any(|p| p.agent(&cur).is_some()) || probed.is_empty() {
            ""
        } else if queued.contains(&cur) {
            "installs when you apply"
        } else {
            "not installed on your machines"
        })
        .why("What aleph new starts when you don't say --agent: one that's installed, or ticked above to install. A machine can have its own (host.<alias>.agent)."),
    );
    v.push(
        Row::choice(
            "naming",
            "session names",
            vec![
                ("story".into(), "story · ignite, temper".into()),
                ("verb".into(), "verb · fix-tests".into()),
                ("folder".into(), "folder · ox_color_check".into()),
            ],
            w.value("naming"),
            w.pending("naming"),
        )
        .why("How new sessions are named when you don't name them: a word from the machine's land and the agent's crew (story), a verb, or the folder."),
    );
    // OpenRouter: one key for many models — in the keystore, never the plan
    v.push(Row::head("OpenRouter (optional)"));
    let (st, text, fix) = if w.running("openrouter") {
        (St::Busy, "looking in the keychain…".to_string(), None)
    } else if w.value("openrouter.key") == "off" {
        (
            St::Info,
            "off (openrouter.key) — enter pastes one and turns it on".to_string(),
            Some("paste a key"),
        )
    } else {
        match w.ag.openrouter {
            Some(true) => (
                St::Ok,
                format!("stored in {} — enter replaces it", crate::secrets::store_name()),
                Some("replace it"),
            ),
            _ => (
                St::Todo,
                "not set — enter pastes it; it's stored in the keychain at once, never in setup's plan"
                    .to_string(),
                Some("paste it"),
            ),
        }
    };
    let mut r = Row::check(st, "openrouter", "OpenRouter key", text);
    if let Some(fx) = fix {
        r = r.fix(fx);
    }
    v.push(r.why("One OpenRouter key gives agents hundreds of models. It's stored in the macOS Keychain / Secret Service the moment you press enter (setup's plan, saved as a file, never holds it); openrouter.key says secret:openrouter. Sessions Aleph starts for the agents below get it as OPENROUTER_API_KEY — through herdr's socket only, never on a command line, on disk or in a log — and Monitor › Usage shows its spend. aleph secret set openrouter does the same from a shell."));
    v.push(
        Row::input(
            "openrouter.agents",
            "for agents",
            w.value("openrouter.agents"),
            "agent ids, all, or off",
            w.pending("openrouter.agents"),
        )
        .note(format!(
            "{}: a model picker too",
            crate::openrouter::model_agents().join(", ")
        ))
        .why("The agents whose sessions get the key: the ones that read OPENROUTER_API_KEY by themselves (OpenCode, Kilo, Pi, OMP, Crush, Hermes, Maki). Others — Claude Code, Codex, Cline, Qwen Code — would need config of their own, so they don't get it unless you add them."),
    );
    v.push(
        Row::choice(
            "openrouter.models",
            "models",
            vec![
                ("all".into(), "all of them".into()),
                ("free".into(), "free only · $0".into()),
                ("paid".into(), "paid only".into()),
            ],
            w.value("openrouter.models"),
            w.pending("openrouter.models"),
        )
        .why("Which of OpenRouter's models to use. Free only: the $0 ones (the :free variants — rate-limited, and some providers may train on what you send); paid only leaves those out. The new-session form's model picker lists just those, and OpenCode sessions Aleph starts get the same as their own config (provider.openrouter.whitelist / blacklist, passed with the session — your opencode.json isn't touched). Usage counts free requests' tokens and says free."),
    );
    // integrations
    v.push(Row::head("Sessions survive a herdr restart"));
    let all: Vec<probe::HostProbe> = probed.iter().map(|p| (*p).clone()).collect();
    let steps = crate::agentcmd::plan(&all, &[]);
    if steps.is_empty() && !probed.is_empty() {
        let none: Vec<String> = probed
            .iter()
            .flat_map(|p| {
                p.installed()
                    .into_iter()
                    .filter(|(ag, _)| p.persist(ag) == Persist::None)
                    .map(|(ag, _)| ag.name.to_string())
                    .collect::<Vec<_>>()
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        v.push(Row::info(
            "Every installed agent with a herdr integration has it.",
        ));
        if !none.is_empty() {
            v.push(Row::info(format!(
                "No integration for {}: aleph restore brings those sessions back.",
                none.join(", ")
            )));
        }
    }
    for s in &steps {
        let a = Action::Integration {
            host: s.host.clone(),
            name: s.integration.to_string(),
        };
        v.push(
            Row::pick(
                format!("integ:{}:{}", s.host, s.integration),
                format!("{} · {}", s.host, s.agent.name),
                w.plan.has(&a),
            )
            .note(if s.refresh { "refresh (outdated)" } else { "install" })
            .note(format!("  herdr integration install {}", s.integration))
            .why("herdr's integration for the agent: with it, herdr restores the same agent session after its server restarts. One command on the machine; it runs in Review."),
        );
    }
    // hooks
    let hookable: Vec<&probe::HostProbe> = probed
        .iter()
        .copied()
        .filter(|p| p.agent("claude").is_some() || p.agent("codex").is_some())
        .collect();
    if !hookable.is_empty() {
        v.push(Row::head("Aleph's hooks (opt-in)"));
        for p in hookable {
            let h = &p.host;
            let on = w.plan.has(&Action::Hooks { host: h.clone() });
            let already = w.cfg.host_get(h, "hooks").as_deref() == Some("on");
            let note = match (already, w.ag.hooks.get(h)) {
                (true, _) => "already on".to_string(),
                (_, None) if w.running(&format!("hooks:{h}")) => {
                    "reading the agents' settings…".into()
                }
                (_, None) => "space: see what it changes".into(),
                (_, Some(Err(e))) => format!("can't tell: {e}"),
                (_, Some(Ok(d))) => {
                    let n: usize = d.iter().map(|(_, l)| l.len()).sum();
                    if n == 0 {
                        "nothing to change".into()
                    } else {
                        format!(
                            "{n} line{} change · the panel shows the diff",
                            if n == 1 { "" } else { "s" }
                        )
                    }
                }
            };
            v.push(
                Row::pick(format!("hooks:{h}"), h.clone(), on || already)
                    .note(note)
                    .why("Claude Code's and Codex's own hooks tell Aleph what happened in their words (a question, a finished turn), next to herdr's view. They're added beside herdr's and moshi-hook's entries in ~/.claude/settings.json and ~/.codex/config.toml, with a backup; aleph hooks remove takes them out again."),
            );
        }
    }
    v
}

pub(crate) fn aside(w: &Wiz, _width: u16) -> Option<Aside> {
    let r = w.selected()?;
    let id = match &r.kind {
        Kind::Check { id, .. } | Kind::Pick { id, .. } | Kind::Local { id, .. } => id.clone(),
        _ => return None,
    };
    if let Some(h) = id.strip_prefix("agents:") {
        let p = w.probes.get(h)?;
        let mut lines = vec![Line::from(vec![
            Span::styled("▌", Style::new().fg(crate::theme::machine(&w.cfg, h))),
            Span::styled(h.to_string(), ui::bold()),
            Span::styled(format!("  {}", p.os_name), ui::faint()),
        ])];
        for (ag, f) in p.installed() {
            let integ = match p.persist(ag) {
                Persist::Yes => ("✓ restores", c::GREEN()),
                Persist::Outdated => ("↑ outdated", c::YELLOW()),
                Persist::Missing => ("– integration", c::YELLOW()),
                Persist::None => ("· no integration", c::FAINT()),
            };
            lines.push(Line::from(vec![
                glyph(ag.id),
                Span::raw(format!(" {} ", ui::pad(ag.name, 14))),
                Span::styled(ui::pad(&f.version, 9), ui::muted()),
                Span::styled(integ.0, ui::fg(integ.1)),
            ]));
        }
        if let Some(hv) = p.herdr_version() {
            lines.push(Line::from(Span::styled(format!("herdr {hv}"), ui::faint())));
        }
        return Some(Aside {
            title: "Agents".into(),
            lines,
        });
    }
    if let Some(rest) = id.strip_prefix("inst:") {
        let (h, ag) = rest.split_once(':')?;
        let p = w.probes.get(h)?;
        let a = agents::find(ag)?;
        let mut lines = vec![Line::from(vec![
            glyph(a.id),
            Span::styled(format!(" {} on {}", a.name, p.host), ui::bold()),
        ])];
        if let Some(f) = p.agent(a.id) {
            lines.push(Line::from(vec![
                Span::styled("✓ installed ", ui::fg(c::GREEN())),
                Span::styled(f.version.clone(), ui::bold()),
            ]));
            match a.support(&f.version) {
                agents::Support::Yes => lines.push(Line::from(Span::styled(
                    format!("supported (Aleph is built for {} and up)", a.min_version),
                    ui::fg(c::GREEN()),
                ))),
                agents::Support::Old => {
                    if let Some(t) = a.old_note(&f.version) {
                        lines.push(Line::from(Span::styled(
                            format!("⚠ {t}"),
                            ui::fg(c::YELLOW()),
                        )));
                    }
                }
                agents::Support::Unknown => {}
            }
            lines.push(Line::from(Span::styled(p.tilde(&f.path), ui::faint())));
            lines.push(Line::from(Span::styled(
                probe::start_of(Some(a)),
                ui::muted(),
            )));
            return Some(Aside {
                title: "Installed".into(),
                lines,
            });
        }
        let pl = crate::agentinstall::plan(a, p);
        match &pl.command {
            Some(cmd) => lines.push(Line::from(vec![
                Span::styled("$ ", ui::faint()),
                Span::styled(cmd.clone(), ui::bold()),
            ])),
            None => lines.push(Line::from(Span::styled(a.install, ui::muted()))),
        }
        for pr in &pl.prereqs {
            lines.push(Line::from(vec![
                Span::styled(
                    if pr.ok { "✓ " } else { "✗ " },
                    ui::fg(if pr.ok { c::GREEN() } else { c::RED() }),
                ),
                Span::raw(format!("{} ", pr.what)),
                Span::styled(pr.detail.clone(), ui::faint()),
            ]));
        }
        if let Some(b) = &pl.blocker {
            for t in ui::wrap(b, 44) {
                lines.push(Line::from(Span::styled(t, ui::fg(c::YELLOW()))));
            }
        }
        if let Some(l) = &pl.lands_in {
            lines.push(Line::from(Span::styled(
                format!("lands in {l}"),
                ui::faint(),
            )));
        }
        return Some(Aside {
            title: "Install".into(),
            lines,
        });
    }
    if let Some(h) = id.strip_prefix("hooks:") {
        let mut lines = vec![];
        match w.ag.hooks.get(h) {
            None => lines.push(Line::from(Span::styled(
                "space reads what it would change",
                ui::muted(),
            ))),
            Some(Err(e)) => lines.push(Line::from(Span::styled(e.clone(), ui::fg(c::RED())))),
            Some(Ok(d)) => {
                for (agent, diff) in d {
                    let f = if agent == "claude" {
                        "~/.claude/settings.json"
                    } else {
                        "~/.codex/config.toml"
                    };
                    lines.push(Line::from(Span::styled(f.to_string(), ui::bold())));
                    if diff.is_empty() {
                        lines.push(Line::from(Span::styled("  already there", ui::faint())));
                    }
                    for l in diff.iter().take(14) {
                        let col = if l.starts_with('+') {
                            c::GREEN()
                        } else {
                            c::RED()
                        };
                        lines.push(Line::from(Span::styled(format!("  {l}"), ui::fg(col))));
                    }
                    if diff.len() > 14 {
                        lines.push(Line::from(Span::styled(
                            format!("  … {} more", diff.len() - 14),
                            ui::faint(),
                        )));
                    }
                }
            }
        }
        return Some(Aside {
            title: format!("Hooks on {h}"),
            lines,
        });
    }
    None
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Fix(id) if id == "openrouter" => {
            w.edit_secret("OpenRouter key", OR_HINT, "openrouter-key");
        }
        Ev::Input(id, text) if id == "openrouter-key" => store_openrouter(w, text),
        Ev::Fix(id) => {
            let h = id.strip_prefix("agents:")?.to_string();
            probe(w, &h, true);
        }
        Ev::Local(id, i) if id == "inst-host" => w.ag.inst_host = i,
        Ev::Pick(id) if id.starts_with("inst:") => {
            let mut it = id.splitn(3, ':').skip(1);
            let (host, agent) = (it.next()?.to_string(), it.next()?.to_string());
            let a = Action::AgentInstall {
                host: host.clone(),
                agent: agent.clone(),
            };
            let on = !w.plan.has(&a);
            if let (true, Some(p), Some(ag)) = (on, w.probes.get(&host), agents::find(&agent)) {
                if p.agent(ag.id).is_some() {
                    w.toast(
                        false,
                        format!("{} is on {host} already — Aleph never uninstalls", ag.name),
                    );
                    return None;
                }
                let pl = crate::agentinstall::plan(ag, p);
                match crate::agentsetup::blocked(&w.cfg, p, &pl) {
                    Some(crate::agentsetup::Why::Manual(how)) => {
                        w.toast(
                            true,
                            format!("{} has no install command Aleph can run — {how}", ag.name),
                        );
                        return None;
                    }
                    Some(why) => w.toast(
                        false,
                        format!(
                            "ticked — but {}: {}; Review shows how",
                            ag.name,
                            why.short()
                        ),
                    ),
                    None => {}
                }
            }
            w.plan.want(Step::Agents, a, on);
            if let (Some(p), Some(ag)) = (w.probes.get(&host), agents::find(&agent)) {
                let herdr_coming = sshcfg::is_local(&host) && w.plan.has(&Action::HerdrLocal);
                if let Some(ia) = integration_for(p, ag, herdr_coming) {
                    w.plan.want(Step::Agents, ia, on);
                }
            }
        }
        Ev::Pick(id) if id.starts_with("integ:") => {
            let mut it = id.splitn(3, ':').skip(1);
            let a = Action::Integration {
                host: it.next()?.to_string(),
                name: it.next()?.to_string(),
            };
            let on = !w.plan.has(&a);
            w.plan.want(Step::Agents, a, on);
        }
        Ev::Pick(id) if id.starts_with("hooks:") => {
            let h = id.trim_start_matches("hooks:").to_string();
            if w.cfg.host_get(&h, "hooks").as_deref() == Some("on") {
                w.toast(
                    false,
                    format!("{h} has them already — aleph hooks remove --host {h} takes them out"),
                );
                return None;
            }
            let a = Action::Hooks { host: h.clone() };
            let on = !w.plan.has(&a);
            w.plan.want(Step::Agents, a, on);
            if on && !w.ag.hooks.contains_key(&h) {
                let host = h.clone();
                w.spawn(
                    &format!("hooks:{h}"),
                    format!("Reading the agents' settings on {h}"),
                    move || {
                        let r = crate::hooks::preview(&host);
                        Msg::Hooks(host, r)
                    },
                );
            }
        }
        _ => {}
    }
    None
}
