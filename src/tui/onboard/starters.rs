//! Step 11, Presets: starting points — one session or several, on one
//! machine or many. A few starters to add, and import from a file or URL:
//! what it would start is shown first and adding them needs a yes. Nothing
//! starts now; presets are saved in Review (`aleph preset launch NAME` or P
//! in Fleet starts one).

use super::rows::Row;
use super::{Aside, Ev, Hand, Msg, Step, Wiz};
use crate::presets::{self, Preset};
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::time::Duration;

#[derive(Default)]
pub(crate) struct State {
    /// presets read from a file or URL, waiting for a yes: (source, presets)
    pub import: Option<(String, Vec<Preset>)>,
    /// the preset the panel shows
    pub shown: Option<String>,
}

/// The starters, as `aleph.toml` would hold them.
pub(crate) fn starter_toml(machines: &[String]) -> String {
    let mut t = String::from(
        "[presets.solo]\ndescription = \"One Claude Code session in a folder\"\nvars = { dir = \"~\" }\n\
         [[presets.solo.panes]]\nagent = \"claude\"\ndir = \"${dir}\"\n\n\
         [presets.pair]\ndescription = \"Claude Code and Codex side by side, same folder\"\nlayout = \"columns\"\nvars = { dir = \"~\" }\n\
         [[presets.pair.panes]]\nname = \"maker\"\nagent = \"claude\"\ndir = \"${dir}\"\n\
         [[presets.pair.panes]]\nname = \"checker\"\nagent = \"codex\"\ndir = \"${dir}\"\n\n\
         [presets.review]\ndescription = \"A second opinion on the current branch\"\nvars = { dir = \"~\" }\n\
         [[presets.review.panes]]\nname = \"review\"\nagent = \"codex\"\ndir = \"${dir}\"\n\
         prompt = \"Review the changes on this branch against main: bugs first, then anything risky. Don't change any files.\"\n",
    );
    if machines.len() > 1 {
        t.push_str("\n[presets.fanout]\ndescription = \"The same task on every machine\"\nvars = { dir = \"~\" }\n");
        for h in machines {
            t.push_str(&format!(
                "[[presets.fanout.panes]]\nhost = {h:?}\nagent = \"claude\"\ndir = \"${{dir}}\"\n"
            ));
        }
    }
    t
}

pub(crate) fn starters(machines: &[String]) -> Vec<Preset> {
    starter_toml(machines)
        .parse::<toml_edit::DocumentMut>()
        .map(|d| presets::from_doc(&d, "setup").0)
        .unwrap_or_default()
}

pub(crate) fn imported(w: &mut Wiz, src: &str, r: Result<Vec<Preset>, String>) {
    match r {
        Err(e) => w.toast(true, e),
        Ok(v) if v.is_empty() => w.toast(true, format!("no presets in {src}")),
        Ok(v) => {
            let mut lines = vec![Line::raw(format!(
                "{} preset{} in {src}. Nothing starts now; they're saved in Review.",
                v.len(),
                if v.len() == 1 { "" } else { "s" }
            ))];
            let tags = presets::fleet_tags(&w.cfg);
            for p in &v {
                lines.push(Line::raw(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        "@{} — {} pane{}",
                        p.name,
                        p.panes.len(),
                        if p.panes.len() == 1 { "" } else { "s" }
                    ),
                    ui::bold(),
                )));
                lines.extend(plan_lines(w, p).into_iter().take(6));
                for x in presets::validate(p, &tags).into_iter().take(3) {
                    lines.push(Line::from(Span::styled(
                        format!("  ! {x}"),
                        ui::fg(c::YELLOW()),
                    )));
                }
            }
            w.ps.import = Some((src.to_string(), v));
            w.confirm("Add these presets?", lines, "import-yes");
        }
    }
}

/// What a preset would start, one line per session.
fn plan_lines(w: &Wiz, p: &Preset) -> Vec<Line<'static>> {
    let tags = presets::fleet_tags(&w.cfg);
    let def = w.value("ssh_host");
    let pl = presets::plan(p, &tags, (!def.is_empty()).then_some(def.as_str()), &[]);
    let mut out: Vec<Line<'static>> = pl
        .launches
        .iter()
        .map(|l| {
            Line::from(vec![
                Span::styled(
                    "  ▌",
                    Style::new().fg(crate::theme::machine(&w.cfg, &l.host)),
                ),
                Span::styled(format!("{} ", l.host), ui::bold()),
                Span::raw(crate::agents::label(l.agent.as_deref().unwrap_or("claude"))),
                Span::styled(
                    format!(
                        "  {}{}",
                        l.dir.clone().unwrap_or_else(|| "~".into()),
                        if l.split_of.is_some() {
                            " · same tab".to_string()
                        } else {
                            String::new()
                        }
                    ),
                    ui::faint(),
                ),
            ])
        })
        .collect();
    for s in &pl.skipped {
        out.push(Line::from(Span::styled(format!("  – {s}"), ui::faint())));
    }
    out
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let existing = presets::all(&w.cfg);
    let mut v = vec![Row::head("Starters")];
    for p in starters(&w.machines()) {
        let planned = w.plan.presets.iter().any(|(_, x)| x.name == p.name);
        let have = existing.iter().any(|x| x.name == p.name);
        v.push(
            Row::pick(format!("starter:{}", p.name), format!("@{}", p.name), planned || have)
                .note(if have { "you have one by this name".to_string() } else { p.description.clone() })
                .why("A starting point you launch with aleph preset launch NAME (--var dir=/srv/x), or P in Fleet. Its folder is a variable, ~ until you say."),
        );
    }
    v.push(Row::head("Import"));
    v.push(Row::button("import", "import from a file or URL…").why(
        "A TOML file with [presets.NAME] tables (what aleph preset export writes). What each would start is shown first, and adding them needs a yes; nothing starts.",
    ));
    for (_, p) in &w.plan.presets {
        if starters(&w.machines()).iter().any(|s| s.name == p.name) {
            continue;
        }
        v.push(
            Row::pick(format!("planned:{}", p.name), format!("@{}", p.name), true).note(format!(
                "{} pane{} · saved in Review · space takes it out",
                p.panes.len(),
                if p.panes.len() == 1 { "" } else { "s" }
            )),
        );
    }
    if !existing.is_empty() {
        v.push(Row::head("You have"));
        let names: Vec<String> = existing.iter().map(|p| format!("@{}", p.name)).collect();
        v.push(Row::info(ui::trunc(&names.join(" · "), 200)));
    }
    v
}

pub(crate) fn aside(w: &Wiz, _width: u16) -> Option<Aside> {
    let r = w.selected()?;
    let name = match &r.kind {
        super::rows::Kind::Pick { id, .. } => id.split_once(':')?.1.to_string(),
        _ => return None,
    };
    let p = starters(&w.machines())
        .into_iter()
        .chain(w.plan.presets.iter().map(|(_, p)| p.clone()))
        .find(|p| p.name == name)?;
    let mut lines = vec![Line::from(Span::styled(p.description.clone(), ui::muted()))];
    lines.push(Line::raw(""));
    lines.extend(plan_lines(w, &p));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        format!("aleph preset launch {} --var dir=…", p.name),
        ui::faint(),
    )));
    Some(Aside {
        title: format!("@{}", p.name),
        lines,
    })
}

fn read_source(src: &str) -> Result<Vec<Preset>, String> {
    let text = if src.starts_with("https://") || src.starts_with("http://") {
        let o = crate::proc::cap("curl", &["-fsSL", "-m", "20", src], Duration::from_secs(25));
        if !o.ok() {
            return Err(format!("couldn't fetch {src}: {}", o.reason()));
        }
        o.text()
    } else {
        std::fs::read_to_string(crate::config::expand(src)).map_err(|e| format!("{src}: {e}"))?
    };
    let doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{src} isn't TOML: {e}"))?;
    let (v, bad) = presets::from_doc(&doc, src);
    if v.is_empty() && !bad.is_empty() {
        return Err(bad.join("; "));
    }
    Ok(v)
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Pick(id) if id.starts_with("starter:") => {
            let n = id.trim_start_matches("starter:");
            if presets::all(&w.cfg).iter().any(|x| x.name == n) {
                w.toast(
                    false,
                    format!("@{n} exists already — aleph preset show {n}"),
                );
                return None;
            }
            if w.plan.presets.iter().any(|(_, x)| x.name == n) {
                w.plan.presets.retain(|(_, x)| x.name != n);
            } else if let Some(p) = starters(&w.machines()).into_iter().find(|p| p.name == n) {
                w.plan.put_preset(Step::Presets, p);
            }
        }
        Ev::Pick(id) if id.starts_with("planned:") => {
            let n = id.trim_start_matches("planned:");
            w.plan.presets.retain(|(_, x)| x.name != n);
        }
        Ev::Button(id) if id == "import" => {
            w.edit(
                "Import presets",
                "a file (~/team/presets.toml) or a URL (https://…)",
                "",
                "import",
            );
        }
        Ev::Input(id, src) if id == "import" => {
            if src.is_empty() {
                return None;
            }
            if src.starts_with("http") && w.offline() {
                w.toast(true, "offline — importing from a URL waits for the network");
                return None;
            }
            let s = src.clone();
            w.spawn("import", format!("Reading {src}"), move || {
                let r = read_source(&s);
                Msg::Import(s, r)
            });
        }
        Ev::Confirm(id) if id == "import-yes" => {
            if let Some((_, v)) = w.ps.import.take() {
                let n = v.len();
                for p in v {
                    w.plan.put_preset(Step::Presets, p);
                }
                w.toast(
                    false,
                    format!(
                        "{n} preset{} waiting for Review",
                        if n == 1 { "" } else { "s" }
                    ),
                );
            }
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_parse() {
        let one = starters(&["b31".to_string()]);
        let names: Vec<&str> = one.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["solo", "pair", "review"]);
        assert_eq!(one[1].panes.len(), 2);
        let many = starters(&["b31".to_string(), "gpu2".to_string()]);
        let f = many.iter().find(|p| p.name == "fanout").unwrap();
        assert_eq!(f.panes.len(), 2);
        assert_eq!(f.panes[1].host.as_deref(), Some("gpu2"));
    }
}
