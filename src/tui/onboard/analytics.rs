//! Step 7, Analytics: tokens, active time and cost from the agents' own
//! logs on each machine (`aleph usage`). Which machines and agents are read
//! (a matrix: `analytics.include` / `exclude`), how far back the first read
//! goes, how long it's kept, whether $ estimates show, and a preview of the
//! sources: what's read where, and what's been read already.

use super::rows::Row;
use super::{Aside, Ev, Hand, Msg, Wiz};
use crate::ui::{self, c};
use ratatui::prelude::*;

/// What the usage store has seen of a machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Seen {
    pub host: String,
    pub files: usize,
    pub bytes: u64,
    pub ok_at: u64,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct State {
    pub seen: Option<Vec<Seen>>,
}

pub(crate) fn enter(w: &mut Wiz) {
    w.spawn("usage", "Reading what analytics has seen", || {
        let st = crate::usage::store::Offsets::load();
        Msg::Usage(
            st.hosts
                .iter()
                .map(|(h, s)| Seen {
                    host: h.clone(),
                    files: s.files.len(),
                    bytes: s.files.values().map(|f| f.size).sum(),
                    ok_at: s.ok_at,
                    error: s.run.as_ref().and_then(|r| r.error.clone()),
                })
                .collect(),
        )
    });
    super::agent_setup::ensure(w, false);
}

/// The reader agents the matrix has columns for: those installed somewhere
/// in the fleet (Claude Code and Codex until the machines answer).
pub(crate) fn columns(w: &Wiz) -> Vec<String> {
    let readers = crate::usage::extract::READERS;
    let mut v: Vec<String> = readers
        .iter()
        .filter(|r| w.probes.values().any(|p| p.agent(r).is_some()))
        .map(|r| r.to_string())
        .collect();
    if v.is_empty() {
        v = vec!["claude".into(), "codex".into()];
    }
    v
}

/// Pure: the include / exclude values for a matrix. Everything on: both
/// empty (every machine and agent, the ones added later too); nothing on:
/// exclude everything; else include what's ticked.
pub(crate) fn settings_for(
    hosts: &[String],
    agents: &[String],
    on: &[Vec<bool>],
    keep: &[String],
) -> (String, String) {
    crate::permit::include_exclude(hosts, agents, on, keep)
}

fn matrix(w: &Wiz, hosts: &[String], agents: &[String]) -> Vec<Vec<bool>> {
    let inc = super::split(&w.value("analytics.include"));
    let exc = super::split(&w.value("analytics.exclude"));
    crate::permit::grid(&inc, &exc, hosts, agents, true)
}

fn days(v: &[&str]) -> Vec<(String, String)> {
    v.iter()
        .map(|d| {
            let l = match *d {
                "30d" => "30 days",
                "90d" => "90 days",
                "400d" => "13 months",
                "1y" => "a year",
                "2y" => "2 years",
                "all" => "all of it",
                o => o,
            };
            (d.to_string(), l.to_string())
        })
        .collect()
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let mut v = vec![Row::toggle(
        "analytics.enabled",
        "analytics",
        w.flag("analytics.enabled"),
        w.pending("analytics.enabled"),
    )
    .note("read in place over ssh; nothing is installed")
    .why("Aleph reads the agents' own session logs on each machine (one sh script over ssh, capped in time and size) and keeps the numbers here. Off: nothing is read.")];
    if !w.flag("analytics.enabled") {
        return v;
    }
    v.push(Row::head("Machines × agents"));
    let hosts = w.fleet();
    let agents = columns(w);
    let m = matrix(w, &hosts, &agents);
    for (i, h) in hosts.iter().enumerate() {
        v.push(
            Row::matrix(format!("an:{h}"), h.clone(), agents.clone(), m[i].clone())
                .why("A ticked cell reads that agent's logs on that machine (analytics.include / analytics.exclude, host:agent globs)."),
        );
    }
    if hosts.is_empty() {
        v.push(Row::info("No machines picked yet (step 3)."));
    }
    v.push(Row::head("History"));
    let ch = |key: &str, label: &str, o: Vec<(String, String)>| {
        let cur = w.value(key);
        let mut o = o;
        if !o.iter().any(|(x, _)| *x == cur) {
            o.push((cur.clone(), cur.clone()));
        }
        Row::choice(key, label, o, cur, w.pending(key))
    };
    v.push(
        ch(
            "analytics.backfill",
            "first read goes back",
            days(&["30d", "90d", "1y", "all"]),
        )
        .why("How far back the first read of a machine goes. Later reads only pick up what's new."),
    );
    v.push(
        ch(
            "analytics.keep",
            "keep",
            days(&["90d", "400d", "2y", "all"]),
        )
        .why("Records older than this are pruned (aleph usage prune does it now)."),
    );
    v.push(
        ch(
            "analytics.cost",
            "$ estimates",
            vec![("on".into(), "show".into()), ("off".into(), "hide".into())],
        )
        .why("Costs are estimated from list prices per model (analytics.prices.<model> overrides one). Hide shows tokens and time only."),
    );
    v.push(Row::head("Sources"));
    match &w.an.seen {
        None => v.push(Row::info("…")),
        Some(seen) => {
            for h in &hosts {
                let s = seen.iter().find(|s| s.host == *h);
                let text = match s {
                    None => format!(
                        "never read — the first sync reads {}",
                        days(&[w.value("analytics.backfill").as_str()])[0].1
                    ),
                    Some(s) if s.error.is_some() => {
                        format!("last read failed: {}", s.error.clone().unwrap_or_default())
                    }
                    Some(s) => format!(
                        "{} file{} · {} · read {}",
                        s.files,
                        if s.files == 1 { "" } else { "s" },
                        ui::bytes(s.bytes as f64),
                        if s.ok_at == 0 {
                            "never".into()
                        } else {
                            ui::ago(crate::cache::now().saturating_sub(s.ok_at))
                        }
                    ),
                };
                v.push(Row::line(vec![
                    Span::styled("▌", Style::new().fg(crate::theme::machine(&w.cfg, h))),
                    Span::styled(format!("{} ", ui::pad(h, 10)), ui::bold()),
                    Span::styled(text, ui::faint()),
                ]));
            }
        }
    }
    v
}

pub(crate) fn aside(w: &Wiz, _width: u16) -> Option<Aside> {
    if !w.flag("analytics.enabled") {
        return None;
    }
    let hosts = w.fleet();
    let agents = columns(w);
    let m = matrix(w, &hosts, &agents);
    let mut lines = Vec::new();
    for (i, h) in hosts.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled("▌", Style::new().fg(crate::theme::machine(&w.cfg, h))),
            Span::styled(h.clone(), ui::bold()),
        ]));
        for (j, a) in agents.iter().enumerate() {
            let on = m[i][j];
            let dir = crate::agents::find(a)
                .and_then(|g| g.logs())
                .map(|l| format!("~/{}", l.dir))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(
                    if on { "  ✓ " } else { "  · " },
                    ui::fg(if on { c::GREEN() } else { c::FAINT() }),
                ),
                Span::styled(
                    format!("{} ", ui::pad(a, 9)),
                    if on { Style::new() } else { ui::faint() },
                ),
                Span::styled(dir, ui::faint()),
            ]));
        }
    }
    Some(Aside {
        title: "What's read".into(),
        lines,
    })
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    if let Ev::Cell(id, col) = ev {
        let h = id.strip_prefix("an:")?.to_string();
        let hosts = w.fleet();
        let agents = columns(w);
        let mut m = matrix(w, &hosts, &agents);
        let i = hosts.iter().position(|x| *x == h)?;
        if let Some(cl) = m[i].get_mut(col) {
            *cl = !*cl;
        }
        let keep = super::split(&w.value("analytics.include"));
        let (inc, exc) = settings_for(&hosts, &agents, &m, &keep);
        w.set("analytics.include", &inc);
        w.set("analytics.exclude", &exc);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_matrix_settings() {
        let h: Vec<String> = vec!["b31".into(), "gpu2".into()];
        let a: Vec<String> = vec!["claude".into(), "codex".into()];
        assert_eq!(
            settings_for(&h, &a, &[vec![true, true], vec![true, true]], &[]),
            (String::new(), String::new())
        );
        assert_eq!(
            settings_for(&h, &a, &[vec![false, false], vec![false, false]], &[]),
            (String::new(), "*:*".into())
        );
        assert_eq!(
            settings_for(&h, &a, &[vec![true, false], vec![true, true]], &[]),
            ("*:claude,gpu2:*".into(), String::new())
        );
    }
}
