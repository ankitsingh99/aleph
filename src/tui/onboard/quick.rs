//! Quick start: every default, no questions. Aleph looks at this computer
//! (the checks, its agents, the shell), lines up what's missing — herdr, the
//! default agent and its herdr integration, the integrations of the agents
//! already there, terminal-notifier, a Nerd Font, the daemon at login, tab
//! completion — and runs it at once on the install screen; then the
//! celebration, and enter opens Aleph. Settings stay at their defaults;
//! other machines are added later (`aleph setup machines`).

use super::plan::Action;
use super::rows::St;
use super::{agent_setup, integrations, preflight, review, Step, Wiz};
use crate::sshcfg::LOCAL;

/// Start it: look (what isn't looked at yet), then `tick` lines it up.
pub(crate) fn start(w: &mut Wiz) {
    if !w.rv.items.is_empty() && !w.rv.done {
        return;
    }
    w.quick = true;
    w.rv.preparing = true;
    w.rv.hidden = false;
    preflight::enter(w);
    integrations::enter(w);
    if !w.probes.contains_key(LOCAL) {
        agent_setup::probe(w, LOCAL, false);
    }
    w.go(Step::Review);
}

/// Has everything quick start reads come in?
fn ready(w: &Wiz) -> bool {
    !w.pre.checks.is_empty()
        && !w.running_any("pre:")
        && w.ig.facts.is_some()
        && (w.probes.contains_key(LOCAL) || !w.running(&format!("probe:{LOCAL}")))
}

/// Each turn of the loop: once it has looked, line it up and apply.
pub(crate) fn tick(w: &mut Wiz) {
    if !w.quick || !ready(w) {
        return;
    }
    w.quick = false;
    w.rv.preparing = false;
    plan(w);
    if w.plan.is_empty() {
        // nothing missing: straight to the celebration
        review::start(w);
        return;
    }
    review::start(w);
}

/// What's missing here, into the plan (pure but for the wizard's state).
pub(crate) fn plan(w: &mut Wiz) {
    let check = |id: &str| w.pre.checks.iter().find(|c| c.id == id).cloned();
    let facts = w.ig.facts.clone().unwrap_or_default();
    let mut want: Vec<(Step, Action)> = Vec::new();
    // herdr here
    let herdr_missing = check("herdr").is_some_and(|c| c.st == St::Bad);
    if herdr_missing {
        want.push((Step::Preflight, Action::HerdrLocal));
    }
    // the tools every machine of yours wants (git, curl), with Homebrew
    if facts.brew {
        for t in ["git", "curl"] {
            if check(t).is_some_and(|c| c.st == St::Warn) {
                want.push((Step::Preflight, Action::tool(t)));
            }
        }
        if cfg!(target_os = "macos") && !facts.notifier {
            want.push((Step::Integrations, Action::notifier()));
        }
    }
    // a Nerd Font where the terminal is this computer (Homebrew's, else the
    // release download)
    if facts.nerd != Some(true)
        && !crate::icons::nerd_font_installed()
        && !super::preflight::terminal_elsewhere()
    {
        want.push((
            Step::Integrations,
            if facts.brew {
                Action::nerd_font()
            } else {
                Action::FontDownload
            },
        ));
    }
    // the agents: the default one when there's none, integrations for all
    if let Some(p) = w.probes.get(LOCAL).filter(|p| p.ok()).cloned() {
        if p.installed().is_empty() {
            let (ticked, _) = crate::agentsetup::defaults(&w.cfg, &p);
            if let Some(id) = ticked.first() {
                want.push((
                    Step::Agents,
                    Action::AgentInstall {
                        host: LOCAL.into(),
                        agent: id.to_string(),
                    },
                ));
                if let Some(a) = crate::agents::find(id)
                    .and_then(|ag| agent_setup::integration_for(&p, ag, herdr_missing))
                {
                    want.push((Step::Agents, a));
                }
            }
        }
        for s in crate::agentcmd::plan(std::slice::from_ref(&p), &[]) {
            want.push((
                Step::Agents,
                Action::Integration {
                    host: s.host.clone(),
                    name: s.integration.to_string(),
                },
            ));
        }
    }
    if !crate::setup::watch_installed() {
        want.push((Step::Access, Action::Service));
    }
    if facts.rc.is_some() && !facts.completions {
        want.push((Step::Integrations, Action::Completions));
    }
    for (s, a) in want {
        w.plan.want(s, a, true);
    }
}
