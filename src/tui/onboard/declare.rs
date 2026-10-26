//! `aleph setup --apply FILE.toml`: setup with no questions. The file is
//! what you want, in `aleph.toml`'s own layout (settings, `[presets.NAME]`),
//! plus an optional `[setup]` table of things to do:
//!
//! ```toml
//! [ui]
//! theme = "dark"
//!
//! [setup]
//! machines = ["b31", "gpu2"]   # added to the fleet (never removes)
//! default = "b31"
//! integrations = true          # herdr's integrations for installed agents
//! hooks = ["b31"]              # Aleph's agent hooks there (true: every machine)
//! service = true               # the daemon at login
//! completions = true
//! wrapper = true               # the opt-in `claude` function
//! herdr_plugin = true
//! tour = "done"
//! ```
//!
//! Every value is checked before anything is written; `--dry-run` shows the
//! plan only. What needs a terminal (herdr's own install prompt) is skipped
//! with a hint, and remote actions wait when there's no network.

use super::plan::{Action, Set};
use super::{check_value, Step};
use crate::config::Config;
use crate::out::{err, fail, head, hint, json, ok, say, warn};
use crate::presets::Preset;
use crate::ui::{self, c};
use ratatui::prelude::*;
use serde_json::json;
use std::process::ExitCode;

/// What a file asks for.
#[derive(Debug, Default)]
pub(crate) struct Decl {
    pub sets: Vec<(String, String)>,
    pub presets: Vec<Preset>,
    pub machines: Vec<String>,
    pub default: Option<String>,
    pub integrations: bool,
    /// None: no hooks; Some(empty): every machine
    pub hooks: Option<Vec<String>>,
    pub service: bool,
    pub completions: bool,
    pub wrapper: bool,
    pub plugin: bool,
    pub tour: bool,
    /// values that don't check out, and `[setup]` keys nobody knows
    pub bad: Vec<String>,
}

fn truthy(i: &toml_edit::Item) -> bool {
    i.as_bool().unwrap_or(false)
        || i.as_str()
            .is_some_and(|s| matches!(s, "true" | "yes" | "on" | "done"))
}

fn strings(i: &toml_edit::Item) -> Vec<String> {
    match i.as_array() {
        Some(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        None => i.as_str().map(super::split).unwrap_or_default(),
    }
}

/// Pure: read a declaration (nothing is checked against the machines).
pub(crate) fn parse(text: &str, src: &str) -> Result<Decl, String> {
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|e| format!("{src} isn't TOML: {e}"))?;
    let mut d = Decl::default();
    if let Some(t) = doc.remove("setup") {
        let Some(t) = t.as_table() else {
            return Err("[setup] must be a table".into());
        };
        for (k, v) in t.iter() {
            match k {
                "machines" => d.machines = strings(v),
                "default" => d.default = v.as_str().map(String::from),
                "integrations" => d.integrations = truthy(v),
                "hooks" => {
                    d.hooks = if v.as_array().is_some()
                        || v.as_str()
                            .is_some_and(|s| s.contains(',') || !matches!(s, "true" | "false"))
                    {
                        Some(strings(v))
                    } else if truthy(v) {
                        Some(vec![])
                    } else {
                        None
                    }
                }
                "service" => d.service = truthy(v),
                "completions" => d.completions = truthy(v),
                "wrapper" => d.wrapper = truthy(v),
                "herdr_plugin" => d.plugin = truthy(v),
                "tour" => d.tour = truthy(v),
                other => d
                    .bad
                    .push(format!("[setup] {other}: not something setup does")),
            }
        }
    }
    let (presets, problems) = crate::presets::from_doc(&doc, src);
    d.bad.extend(problems);
    let has_presets = !presets.is_empty();
    d.presets = presets;
    for (k, v) in crate::tomlcfg::entries(&doc) {
        if has_presets && k.starts_with("preset.") {
            continue;
        }
        if let Err(e) = check_value(&k, &v) {
            d.bad.push(format!("{k}: {e}"));
        }
        d.sets.push((k, v));
    }
    if let Some(def) = &d.default {
        d.sets.push(("ssh_host".into(), def.clone()));
    }
    if d.tour {
        d.sets.push(("tour".into(), "done".into()));
    }
    Ok(d)
}

/// The settings it changes from `cfg`: (key, old, new). The machines are
/// added to the fleet the file (or the settings) already has.
pub(crate) fn changes(d: &Decl, cfg: &Config) -> Vec<(String, String, String)> {
    let mut sets = d.sets.clone();
    if !d.machines.is_empty() {
        let mut hosts = match sets.iter().find(|(k, _)| k == "hosts") {
            Some((_, v)) => super::split(v),
            // as written (hosts() puts the default first)
            None => cfg
                .get_raw("hosts")
                .map(|v| super::split(&v))
                .unwrap_or_else(|| cfg.hosts()),
        };
        for m in &d.machines {
            if !hosts.contains(m) {
                hosts.push(m.clone());
            }
        }
        sets.retain(|(k, _)| k != "hosts");
        sets.push(("hosts".into(), hosts.join(",")));
        if d.default.is_none() && cfg.get("ssh_host").is_none() {
            if let Some(f) = hosts.first() {
                sets.push(("ssh_host".into(), f.clone()));
            }
        }
    }
    let mut out: Vec<(String, String, String)> = Vec::new();
    for (k, v) in sets {
        let old = cfg.get_raw(&k).unwrap_or_default();
        if old != v {
            out.retain(|(x, ..)| *x != k);
            out.push((k, old, v));
        }
    }
    out
}

fn shown(k: &str, v: &str) -> String {
    if v.is_empty() {
        "—".into()
    } else if crate::configcmd::is_secret(k) {
        super::rows::masked(v)
    } else {
        v.to_string()
    }
}

/// `aleph setup --apply FILE [--dry-run] [--json]`.
pub fn run(cfg: Config, src: &str, dry: bool, js: bool) -> ExitCode {
    let text = match std::fs::read_to_string(crate::config::expand(src)) {
        Ok(t) => t,
        Err(e) => return fail(&format!("{src}: {e}")),
    };
    let d = match parse(&text, src) {
        Ok(d) => d,
        Err(e) => return fail(&e),
    };
    if !d.bad.is_empty() {
        for b in &d.bad {
            err(b);
        }
        hint("nothing was changed — fix the file and run it again");
        return ExitCode::from(2);
    }
    let ch = changes(&d, &cfg);
    // a preset that's already there as written isn't a change
    let mut d = d;
    d.presets.retain(|p| {
        crate::presets::get(&cfg, &p.name)
            .is_none_or(|e| crate::presets::to_toml(&e) != crate::presets::to_toml(p))
    });
    let fleet_after: Vec<String> = ch
        .iter()
        .find(|(k, ..)| k == "hosts")
        .map(|(_, _, v)| super::split(v))
        .unwrap_or_else(|| cfg.hosts());
    let mut acts: Vec<Action> = Vec::new();
    if let Some(h) = &d.hooks {
        let on: Vec<String> = if h.is_empty() {
            fleet_after.clone()
        } else {
            h.clone()
        };
        acts.extend(on.into_iter().map(|host| Action::Hooks { host }));
    }
    if d.service {
        acts.push(Action::Service);
    }
    if d.completions {
        acts.push(Action::Completions);
    }
    if d.wrapper {
        acts.push(Action::Wrapper);
    }
    if d.plugin {
        acts.push(Action::HerdrPlugin);
    }
    if !js {
        head(&format!("aleph setup --apply {src}"));
        if ch.is_empty() && d.presets.is_empty() {
            say(Line::from(Span::styled(
                "  no settings change",
                ui::faint(),
            )));
        }
        for (k, o, n) in &ch {
            say(Line::from(vec![
                Span::styled(
                    if o.is_empty() { "  + " } else { "  ~ " },
                    ui::fg(if o.is_empty() {
                        c::GREEN()
                    } else {
                        c::YELLOW()
                    }),
                ),
                Span::raw(if o.is_empty() {
                    format!("{k} = {}", shown(k, n))
                } else {
                    format!("{k}: {} → {}", shown(k, o), shown(k, n))
                }),
            ]));
        }
        for p in &d.presets {
            say(Line::from(vec![
                Span::styled("  + ", ui::fg(c::GREEN())),
                Span::raw(format!(
                    "preset @{} ({} pane{})",
                    p.name,
                    p.panes.len(),
                    if p.panes.len() == 1 { "" } else { "s" }
                )),
            ]));
        }
        if d.integrations {
            say(Line::from(vec![
                Span::styled("  ▲ ", ui::fg(c::ACCENT())),
                Span::raw("herdr's integrations on every machine that lacks one"),
            ]));
        }
        for a in &acts {
            say(Line::from(vec![
                Span::styled(
                    if a.host().is_some() {
                        "  ▲ "
                    } else {
                        "  · "
                    },
                    ui::fg(c::ACCENT()),
                ),
                Span::raw(a.label()),
            ]));
        }
    }
    if dry {
        if js {
            json(
                &json!({"dry_run": true, "changes": ch.iter().map(|(k, o, n)| json!({"key": k, "old": shown(k, o), "new": shown(k, n)})).collect::<Vec<_>>(), "presets": d.presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), "actions": acts.iter().map(Action::label).collect::<Vec<_>>(), "integrations": d.integrations}),
            );
        } else {
            hint("nothing changed (--dry-run)");
        }
        return ExitCode::SUCCESS;
    }
    let mut results: Vec<(String, Result<String, String>)> = Vec::new();
    if !ch.is_empty() || !d.presets.is_empty() {
        let sets: Vec<Set> = ch
            .iter()
            .map(|(k, _, n)| Set {
                step: Step::Review,
                key: k.clone(),
                val: (!n.is_empty()).then(|| n.clone()),
            })
            .collect();
        results.push((
            "settings".into(),
            super::review::apply_settings(&sets, &d.presets),
        ));
    }
    let online = crate::net::probe(
        &crate::net::target(&cfg),
        std::time::Duration::from_millis(1500),
    )
    .is_some()
        || crate::net::default_route();
    if d.integrations {
        if !online {
            results.push(("integrations".into(), Err("skipped: no network".into())));
        } else {
            let hosts: Vec<String> = Config::load().fleet();
            let probes = crate::probe::run_all(&hosts);
            let steps = crate::agentcmd::plan(&probes, &[]);
            if steps.is_empty() {
                results.push((
                    "integrations".into(),
                    Ok("every installed agent has its integration".into()),
                ));
            }
            for s in steps {
                let a = Action::Integration {
                    host: s.host.clone(),
                    name: s.integration.to_string(),
                };
                results.push((a.label(), super::review::run_quiet(&a, &|_| {})));
            }
        }
    }
    for a in acts {
        if a.host().is_some() && !online {
            results.push((a.label(), Err("skipped: no network".into())));
            continue;
        }
        results.push((a.label(), super::review::run_quiet(&a, &|_| {})));
    }
    let bad = results.iter().filter(|(_, r)| r.is_err()).count();
    if js {
        json(&json!({
            "changes": ch.iter().map(|(k, o, n)| json!({"key": k, "old": shown(k, o), "new": shown(k, n)})).collect::<Vec<_>>(),
            "presets": d.presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            "results": results.iter().map(|(w, r)| match r {
                Ok(m) => json!({"what": w, "ok": true, "message": m}),
                Err(e) => json!({"what": w, "ok": false, "message": e}),
            }).collect::<Vec<_>>(),
        }));
    } else {
        for (w, r) in &results {
            match r {
                Ok(m) => ok(&format!("{w}: {m}")),
                Err(e) if e.starts_with("skipped") => warn(&format!("{w}: {e}")),
                Err(e) => err(&format!("{w}: {e}")),
            }
        }
        if !fleet_after.is_empty() {
            hint(
                "herdr on the machines asks before it installs: aleph setup herdr (in a terminal)",
            );
        }
    }
    if bad > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declaration_reads_settings_presets_and_setup() {
        let t = "[ui]\ntheme = \"dark\"\n[notify]\nquiet = \"23:00-08:00\"\n\
                 [presets.pair]\nlayout = \"columns\"\n[[presets.pair.panes]]\nagent = \"claude\"\n\
                 [setup]\nmachines = [\"b31\", \"gpu2\"]\ndefault = \"gpu2\"\nhooks = true\ncompletions = true\ntour = \"done\"\n";
        let d = parse(t, "x.toml").unwrap();
        assert!(d.bad.is_empty(), "{:?}", d.bad);
        assert!(d.sets.contains(&("theme".into(), "dark".into())));
        assert!(d
            .sets
            .contains(&("notify.quiet".into(), "23:00-08:00".into())));
        assert!(d.sets.contains(&("ssh_host".into(), "gpu2".into())));
        assert!(d.sets.contains(&("tour".into(), "done".into())));
        assert!(
            !d.sets.iter().any(|(k, _)| k.starts_with("preset.")),
            "{:?}",
            d.sets
        );
        assert_eq!(d.presets.len(), 1);
        assert_eq!(d.machines, vec!["b31", "gpu2"]);
        assert_eq!(d.hooks, Some(vec![]));
        assert!(d.completions && !d.service);
        let cfg = Config::in_memory(&["hosts=b31", "theme=dark"]);
        let ch = changes(&d, &cfg);
        assert!(
            !ch.iter().any(|(k, ..)| k == "theme"),
            "unchanged values aren't changes: {ch:?}"
        );
        assert!(
            ch.contains(&("hosts".into(), "b31".into(), "b31,gpu2".into())),
            "{ch:?}"
        );
    }

    #[test]
    fn bad_values_are_caught_before_anything_is_written() {
        let d = parse(
            "[ui]\ntheme = \"purple\"\n[setup]\nfrobnicate = true\n",
            "x",
        )
        .unwrap();
        assert_eq!(d.bad.len(), 2, "{:?}", d.bad);
        assert!(parse("not toml = [", "x").is_err());
    }
}
