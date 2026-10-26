//! Step 4, herdr on each machine: installed, the same version as this
//! computer (herdr wants both ends to match), saved in herdr, its server
//! running, and which integrations it has. Installing, updating or saving a
//! machine is `herdr machine add` — queued for Review, where herdr asks once
//! more before it downloads anything. Optionally the size of panes herdr
//! creates while nobody's attached (`server.headless_cols/rows`).

use super::rows::{Row, St};
use super::{Action, Ev, Hand, Msg, Wiz};
use crate::probe::IState;
use crate::sshcfg;
use crate::update::{self, Herdr};
use ratatui::prelude::*;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Default)]
pub(crate) struct State {
    /// is herdr's server running there (None: couldn't tell)
    pub server: HashMap<String, Option<bool>>,
}

pub(crate) const SIZES: &[(u16, u16)] = &[(120, 40), (160, 48), (200, 60), (240, 64)];

pub(crate) fn enter(w: &mut Wiz) {
    super::agent_setup::ensure(w, false);
    for h in w.machines() {
        if !w.hd.server.contains_key(&h) {
            server(w, &h);
        }
    }
}

fn server(w: &mut Wiz, h: &str) {
    if w.no_network() && !sshcfg::is_local(h) {
        return;
    }
    let host = h.to_string();
    w.spawn(
        &format!("server:{h}"),
        format!("Asking herdr's server on {h}"),
        move || {
            let cmd = format!(
                "{}; herdr status server --json 2>/dev/null",
                crate::chan::PATH_PRELUDE
            );
            let o = crate::proc::run(
                &mut sshcfg::shell(&host, &cmd),
                Duration::from_secs(15),
                None,
            );
            let up = o.ok().and_then(|o| {
                let t = o.text();
                match crate::herdr::ServerStatus::parse(&t) {
                    Some(s) => Some(s.running),
                    None if t.contains("not running") => Some(false),
                    None if t.contains("running") => Some(true),
                    None => None,
                }
            });
            Msg::Server(host, up)
        },
    );
}

fn integ_summary(p: &crate::probe::HostProbe) -> String {
    let mut v: Vec<String> = Vec::new();
    for (ag, _) in p.installed() {
        let Some(n) = ag.integration else { continue };
        if let Some(i) = p.integ(n) {
            v.push(format!(
                "{} {}",
                ag.id,
                match &i.state {
                    IState::Current(_) => "✓",
                    IState::Outdated { .. } => "↑ outdated",
                    IState::Missing => "– missing",
                    IState::Other(_) => "?",
                }
            ));
        }
    }
    if v.is_empty() {
        "no integrations for its agents".into()
    } else {
        format!("integrations: {}", v.join(" · "))
    }
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let mut v = vec![Row::head("This computer")];
    let local = w.local_herdr.clone();
    v.push(match &local {
        None => Row::check(St::Busy, "herdr:here", "herdr here", "looking".into()),
        Some(None) => Row::check(St::Bad, "herdr:here", "herdr here", "not installed — the machines get this computer's version".into())
            .fix("copy where to get it")
            .why("herdr runs the agents, on every machine, and this computer's herdr saves the machines and installs herdr on them (the same version). Get it from https://herdr.dev."),
        Some(Some(ver)) => Row::check(St::Ok, "herdr:here", "herdr here", format!("{ver} — machines get this version")),
    });
    let machines: Vec<String> = w
        .machines()
        .into_iter()
        .filter(|h| !sshcfg::is_local(h))
        .collect();
    if machines.is_empty() {
        v.push(Row::info("No machines picked yet (step 3)."));
        return v;
    }
    v.push(Row::head("Machines"));
    let saved = w.saved.clone().unwrap_or_default();
    for h in &machines {
        let id = format!("herdr:{h}");
        let save_planned = w.plan.actions.iter().find_map(|(_, a)| match a {
            Action::HerdrSave { host, why } if host == h => Some(why.clone()),
            _ => None,
        });
        let row = match w.probes.get(h) {
            None if w.running(&format!("probe:{h}")) => {
                Row::check(St::Busy, id, h.clone(), "looking".into())
            }
            None if w.no_network() => {
                Row::check(St::Todo, id, h.clone(), "waiting for the network".into())
            }
            None => Row::check(St::Todo, id, h.clone(), "not looked at yet".into()).fix("look"),
            Some(p) => {
                let cmp = update::compare(local.clone().flatten().as_deref(), p);
                let is_saved = saved
                    .iter()
                    .any(|s| s == h || *s == crate::herdr::machine_for(h));
                let ver = p.herdr_version().unwrap_or("").to_string();
                let lv = local.clone().flatten().unwrap_or_default();
                let (st, text, fix, why) = match (&cmp, is_saved) {
                    (Herdr::Unreachable(e), _) => {
                        (St::Bad, e.clone(), Some("look again"), String::new())
                    }
                    (Herdr::Missing, _) => (
                        St::Warn,
                        "herdr isn't installed".to_string(),
                        Some("install it"),
                        format!("installs herdr {lv}"),
                    ),
                    (Herdr::Behind, _) => (
                        St::Warn,
                        format!("{ver} · this computer has {lv}"),
                        Some("update it"),
                        format!("updates herdr {ver} → {lv}"),
                    ),
                    (Herdr::Ahead, _) => (
                        St::Warn,
                        format!("{ver} · newer than this computer's {lv} — update herdr here"),
                        None,
                        String::new(),
                    ),
                    (_, false) => (
                        St::Warn,
                        format!("{ver} · not saved in herdr yet"),
                        Some("save it"),
                        String::new(),
                    ),
                    (_, true) => (
                        St::Ok,
                        format!("{ver} · saved in herdr"),
                        None,
                        String::new(),
                    ),
                };
                match (&save_planned, fix) {
                    (Some(_), Some(_)) => Row::check(St::Info, id, h.clone(), format!("{text} → in Review: {}", if why.is_empty() { "save it".to_string() } else { why.clone() }))
                        .fix("don't")
                        .why("herdr machine add runs in Review: it saves the machine in herdr and installs (or updates) herdr there to this computer's version — herdr asks once more before it downloads. Enter takes it out again."),
                    (_, Some(f)) => Row::check(st, id, h.clone(), text).fix(format!("{f} (in Review)"))
                        .why("Enter puts herdr machine add on the list for Review. It saves the machine in herdr and, when needed, installs herdr there (the same version as here); herdr asks once more before downloading."),
                    _ => Row::check(st, id, h.clone(), text),
                }
            }
        };
        v.push(row);
        if let Some(p) = w.probes.get(h).filter(|p| p.ok()) {
            let srv = match w.hd.server.get(h) {
                Some(Some(true)) => "server running",
                Some(Some(false)) => "server not running (it starts with the first session)",
                _ if w.running(&format!("server:{h}")) => "asking the server…",
                _ => "server: unknown",
            };
            v.push(Row::line(vec![
                Span::raw("      "),
                Span::styled(format!("{srv} · {}", integ_summary(p)), crate::ui::faint()),
            ]));
        }
        let cur = w
            .plan
            .actions
            .iter()
            .find_map(|(_, a)| match a {
                Action::Headless { host, cols, rows } if host == h => SIZES
                    .iter()
                    .position(|s| *s == (*cols, *rows))
                    .map(|i| i + 1),
                _ => None,
            })
            .unwrap_or(0);
        let opts: Vec<(String, String)> =
            std::iter::once(("keep".to_string(), "as it is".to_string()))
                .chain(
                    SIZES
                        .iter()
                        .map(|(c, r)| (format!("{c}x{r}"), format!("{c}×{r}"))),
                )
                .collect();
        v.push(
            Row::local(format!("headless:{h}"), format!("{h} new panes"), opts, cur)
                .why("The size herdr gives a pane while no client is attached (aleph new --detach, presets, the queue): server.headless_cols/rows in that machine's ~/.config/herdr/config.toml. herdr's default is 120×40. Applied in Review; herdr reads it at its next start (or reload-config)."),
        );
        let key = format!("host.{h}.session");
        v.push(
            Row::input(
                key.clone(),
                format!("{h} herdr session"),
                w.value(&key),
                "empty: herdr's default session",
                w.pending(&key),
            )
            .why("herdr can run several independent servers on a machine, each a named session with its own tabs and socket (herdr session list shows them there). Name one here and Aleph uses it for everything on that machine — sessions, attach, the listener, streams (herdr's --session / HERDR_SESSION). Empty: the default one."),
        );
    }
    v
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Fix(id) if id == "herdr:here" => {
            let ok = crate::tui::copy("https://herdr.dev");
            w.toast(!ok, "https://herdr.dev — install herdr, then r here");
        }
        Ev::Fix(id) => {
            let h = id.strip_prefix("herdr:")?.to_string();
            let Some(p) = w.probes.get(&h) else {
                super::agent_setup::probe(w, &h, true);
                return None;
            };
            let local = w.local_herdr.clone().flatten();
            let cmp = update::compare(local.as_deref(), p);
            if let Herdr::Unreachable(_) = cmp {
                super::agent_setup::probe(w, &h, true);
                server(w, &h);
                return None;
            }
            let why = match (&cmp, &local) {
                (Herdr::Missing, Some(l)) => format!("installs herdr {l}"),
                (Herdr::Behind, Some(l)) => {
                    format!("updates herdr {} → {l}", p.herdr_version().unwrap_or("?"))
                }
                _ => String::new(),
            };
            let a = Action::HerdrSave { host: h, why };
            let on = !w.plan.has(&a);
            w.plan.want(w.cur, a, on);
        }
        Ev::Local(id, i) => {
            let h = id.strip_prefix("headless:")?.to_string();
            w.plan
                .actions
                .retain(|(_, a)| !matches!(a, Action::Headless { host, .. } if *host == h));
            if let Some((cols, rows)) = i.checked_sub(1).and_then(|j| SIZES.get(j)) {
                w.plan.want(
                    w.cur,
                    Action::Headless {
                        host: h,
                        cols: *cols,
                        rows: *rows,
                    },
                    true,
                );
            }
        }
        _ => {}
    }
    None
}

/// `[server] headless_cols / headless_rows` set in a herdr config.toml's text
/// (pure; the rest of the file is kept as it is).
pub(crate) fn with_headless(src: &str, cols: u16, rows: u16) -> String {
    let mut doc: toml_edit::DocumentMut = src.parse().unwrap_or_default();
    if !doc.contains_table("server") {
        doc["server"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["server"]["headless_cols"] = toml_edit::value(i64::from(cols));
    doc["server"]["headless_rows"] = toml_edit::value(i64::from(rows));
    doc.to_string()
}

/// Apply the headless size on `host` (read, edit, write back with a backup,
/// only if the file didn't change meanwhile).
pub(crate) fn apply_headless(host: &str, cols: u16, rows: u16) -> Result<String, String> {
    let f = "$HOME/.config/herdr/config.toml";
    let read = format!("f=\"{f}\"; if [ -f \"$f\" ]; then printf 'CK %s\\n' \"$(cksum < \"$f\")\"; cat \"$f\"; else echo NONE; fi");
    let o = crate::proc::run(
        &mut sshcfg::shell(host, &read),
        Duration::from_secs(20),
        None,
    )
    .map_err(|e| e.to_string())?;
    if !o.ok() {
        return Err(o.reason());
    }
    let t = o.text();
    let (ck, text) = if t.trim() == "NONE" {
        (String::new(), String::new())
    } else {
        let (first, rest) = t.split_once('\n').unwrap_or((&t, ""));
        (
            first.trim_start_matches("CK ").trim().to_string(),
            rest.to_string(),
        )
    };
    if text.parse::<toml_edit::DocumentMut>().is_err() && !text.trim().is_empty() {
        return Err("its herdr config.toml doesn't parse — left it alone".into());
    }
    let new = with_headless(&text, cols, rows);
    let stamp = crate::herdrtheme::stamp(crate::cache::now());
    let write = format!(
        "f=\"{f}\"; n=\"$f.aleph-new\"; mkdir -p \"$(dirname \"$f\")\"; cat > \"$n\" || exit 3; \
         if [ -f \"$f\" ]; then [ \"$(cksum < \"$f\")\" = \"{ck}\" ] || {{ rm -f \"$n\"; echo CHANGED; exit 4; }}; cp -p \"$f\" \"$f.aleph-bak-{stamp}\" || exit 5; fi; \
         mv \"$n\" \"$f\" && echo OK"
    );
    let o = crate::proc::run(
        &mut sshcfg::shell(host, &write),
        Duration::from_secs(20),
        Some(new.as_bytes()),
    )
    .map_err(|e| e.to_string())?;
    if o.text().contains("CHANGED") {
        return Err("the file changed while we looked — nothing written".into());
    }
    if !o.ok() || !o.text().contains("OK") {
        return Err(o.reason());
    }
    Ok(format!(
        "{host}: new panes {cols}×{rows} (herdr reads it at its next start)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_size_keeps_the_rest_of_the_file() {
        let src = "# mine\n[ui]\naccent = \"#89b4fa\"\n\n[server]\nheadless_cols = 120 # was\n";
        let out = with_headless(src, 200, 60);
        assert!(
            out.starts_with("# mine\n[ui]\naccent = \"#89b4fa\""),
            "{out}"
        );
        assert!(out.contains("headless_cols = 200"), "{out}");
        assert!(out.contains("headless_rows = 60"), "{out}");
        let fresh = with_headless("", 160, 48);
        assert!(
            fresh.contains("[server]") && fresh.contains("headless_rows = 48"),
            "{fresh}"
        );
    }
}
