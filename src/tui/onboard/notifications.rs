//! Step 6, Notifications: who tells you (herdr's view, the agents' own
//! words, or both), where it goes (the desktop, your phone through ntfy —
//! with a test send and Approve / Deny buttons — Moshi, Raycast, Slack, a
//! webhook, Web Push once the web console pairs phones), when to keep quiet
//! (route presets, quiet hours) and what phones, Raycast and the web may act
//! on (remote control, machines × agents).

use super::rows::{Kind, Row, St};
use super::{Action, Ev, Hand, Msg, Run, Step, Wiz};
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct State {
    /// moshi-hook here (None: not looked; Some(None): not installed)
    pub moshi: Option<Option<crate::moshi::Status>>,
    /// test sends: sink → how it went
    pub sent: HashMap<String, Result<(), String>>,
    /// the ntfy topic is shown (it's a secret: hidden until asked)
    pub reveal: bool,
}

/// Route presets: (label, notify.remote_on, notify_done, notify.quiet).
pub(crate) const PRESETS: &[(&str, &str, &str, &str)] = &[
    ("only when I'm needed", "blocked,limit,error", "false", ""),
    ("everything", "all", "true", ""),
    (
        "quiet nights",
        "blocked,limit,error",
        "false",
        "23:00-08:00",
    ),
];

pub(crate) fn enter(w: &mut Wiz) {
    if w.nt.moshi.is_none() {
        w.spawn("moshi", "Looking for moshi-hook", || {
            Msg::Moshi(crate::moshi::status())
        });
    }
    super::agent_setup::ensure(w, false);
}

pub(crate) fn retry(w: &mut Wiz) {
    w.nt.moshi = None;
    w.nt.sent.clear();
    enter(w);
}

pub(crate) fn sent(w: &mut Wiz, sink: &str, r: Result<(), String>) {
    match &r {
        Ok(()) => w.toast(false, format!("{sink}: sent — check it arrived")),
        Err(e) => w.toast(true, format!("{sink}: {e}")),
    }
    w.nt.sent.insert(sink.to_string(), r);
}

/// The agents a remote-control matrix has columns for: the ones installed
/// on the machines (Claude Code and Codex at least).
pub(crate) fn matrix_agents(w: &Wiz) -> Vec<String> {
    let mut v: Vec<String> = vec!["claude".into(), "codex".into()];
    for p in w.probes.values() {
        for (ag, _) in p.installed() {
            if !v.iter().any(|x| x == ag.id) {
                v.push(ag.id.to_string());
            }
        }
    }
    v
}

/// Pure: a machines × agents matrix as `host:agent` globs — `*:*` when
/// everything is on, `*:<agent>` for a whole column, `<host>:*` for a whole
/// row. `keep`: entries about machines outside the matrix stay.
pub(crate) fn to_globs(
    hosts: &[String],
    agents: &[String],
    on: &[Vec<bool>],
    keep: &[String],
) -> Vec<String> {
    crate::permit::to_globs(hosts, agents, on, keep)
}

/// The matrix `key` (remote.allow, analytics.include) describes now.
pub(crate) fn matrix(
    w: &Wiz,
    allow: &str,
    deny: &str,
    hosts: &[String],
    agents: &[String],
) -> Vec<Vec<bool>> {
    let a = super::split(&w.value(allow));
    let d = super::split(&w.value(deny));
    crate::permit::grid(&a, &d, hosts, agents, false)
}

fn which_preset(w: &Wiz) -> usize {
    let on = w.value("notify.remote_on");
    let done = w.flag("notify_done").to_string();
    let quiet = w.value("notify.quiet");
    PRESETS
        .iter()
        .position(|(_, r, d, q)| *r == on && *d == done && *q == quiet)
        .map(|i| i + 1)
        .unwrap_or(0)
}

fn test_note(w: &Wiz, sink: &str) -> Vec<Span<'static>> {
    if w.running(&format!("send:{sink}")) {
        return vec![Span::styled(
            format!("  {} sending", ui::spinner(w.t0)),
            ui::fg(c::ACCENT()),
        )];
    }
    match w.nt.sent.get(sink) {
        Some(Ok(())) => vec![Span::styled("  ✓ sent", ui::fg(c::GREEN()))],
        Some(Err(e)) => vec![Span::styled(
            format!("  ✗ {}", ui::trunc(e, 50)),
            ui::fg(c::RED()),
        )],
        None => vec![],
    }
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let mut v = vec![Row::head("Who tells you")];
    v.push(
        Row::choice(
            "notify.source",
            "source",
            vec![
                ("both".into(), "both".into()),
                ("herdr".into(), "herdr's view".into()),
                ("agents".into(), "the agents' own words".into()),
            ],
            w.value("notify.source"),
            w.pending("notify.source"),
        )
        .why("herdr sees every agent's state; the agents' own hooks (step 5) say it in their words. With both, the agent's words win when both say the same thing."),
    );
    v.push(Row::head("Where it goes"));
    v.push(
        Row::toggle("notify", "desktop", w.flag("notify"), w.pending("notify"))
            .note("from the daemon (step 9): questions")
            .why("A desktop notification when a session needs you, sent by the daemon (the dashboard is already in front of you)."),
    );
    v.push(Row::toggle(
        "notify_done",
        "  finished turns too",
        w.flag("notify_done"),
        w.pending("notify_done"),
    ));
    v.push(Row::choice(
        "notify.desktop.click",
        "  a click opens",
        vec![
            ("aleph".into(), "Aleph's Focus".into()),
            ("raycast".into(), "Raycast".into()),
            ("warp".into(), "a Warp tab".into()),
        ],
        w.value("notify.desktop.click"),
        w.pending("notify.desktop.click"),
    ));
    let ntfy = w.value("notify.ntfy");
    // a secret: shown when you ask (from the keychain when it's kept there)
    let shown_ntfy = if w.nt.reveal && ntfy.starts_with("secret:") {
        crate::secrets::resolve(&ntfy).unwrap_or(ntfy.clone())
    } else {
        ntfy.clone()
    };
    let mut row = Row::input("notify.ntfy", "phone (ntfy)", shown_ntfy, "https://ntfy.sh/<a-secret-topic> — then subscribe in the ntfy app", w.pending("notify.ntfy"))
        .why("ntfy pushes to the ntfy app on your phone. Anyone who knows the topic can read it, so make it hard to guess (make one below). Questions carry Approve and Deny buttons for the sessions remote control allows.");
    if w.nt.reveal {
        row = row.revealed();
    }
    v.push(row);
    if !ntfy.is_empty() {
        v.push(
            Row::button("ntfy-show", if w.nt.reveal { "hide the topic" } else { "show the topic" })
                .why("The topic is a secret (whoever knows it can read your notifications): it's shown only while you ask, to subscribe in the ntfy app."),
        );
    }
    if ntfy.is_empty() {
        v.push(
            Row::button("ntfy-new", "make a private topic")
                .why("A topic nobody will guess: https://ntfy.sh/aleph-<16 random hex>. Subscribe to it in the ntfy app."),
        );
        v.push(
            Row::button("ntfy-name", "make one with my own name")
                .why("Your words first (e.g. sam-laptop), then 8 random hex so it stays unguessable: https://ntfy.sh/sam-laptop-3f9a1c07. A server of your own: type its whole URL in the phone row above."),
        );
    } else {
        v.push(Row::button("test:ntfy", "send a test").spans(test_note(w, "ntfy")));
        let allow = w.value("remote.allow");
        v.push(Row::line(vec![
            Span::raw("    "),
            Span::styled(
                if allow.is_empty() {
                    "Approve / Deny buttons: off until remote control allows some sessions (below)"
                        .to_string()
                } else {
                    format!(
                        "Approve / Deny buttons on questions from {allow} (never rm -rf, sudo, …)"
                    )
                },
                ui::faint(),
            ),
        ]));
    }
    let slack = w.value("notify.slack");
    v.push(Row::input(
        "notify.slack",
        "Slack",
        slack.clone(),
        "https://hooks.slack.com/services/…",
        w.pending("notify.slack"),
    ));
    if !slack.is_empty() {
        v.push(Row::button("test:slack", "send a test to Slack").spans(test_note(w, "slack")));
    }
    let hook = w.value("notify.webhook");
    v.push(
        Row::input(
            "notify.webhook",
            "webhook",
            hook.clone(),
            "https://… — gets each event as JSON",
            w.pending("notify.webhook"),
        )
        .why("Any URL: each event is POSTed as JSON (kind, host, session, title)."),
    );
    if !hook.is_empty() {
        v.push(
            Row::button("test:webhook", "send a test to the webhook")
                .spans(test_note(w, "webhook")),
        );
    }
    v.push(
        Row::toggle("notify.raycast", "Raycast menu bar", w.flag("notify.raycast"), w.pending("notify.raycast"))
            .note("refreshed the moment something happens")
            .why("Refreshes Raycast's Aleph menu bar through its deeplink when something happens (debounced), instead of every 30 s."),
    );
    v.push(
        Row::toggle("notify.herdr", "herdr toasts", w.flag("notify.herdr"), w.pending("notify.herdr"))
            .why("Questions, limits and problems also show as a toast in herdr's own window on this computer."),
    );
    // Moshi
    v.push(Row::head("Moshi"));
    let st = match &w.nt.moshi {
        None => Row::check(St::Busy, "moshi", "moshi-hook", "looking".into()),
        Some(None) if w.plan.has(&Action::moshi_hook()) => Row::check(St::Todo, "moshi", "moshi-hook", "queued: Moshi's installer runs when you apply — then moshi-hook pair".into()).fix("unqueue it"),
        Some(None) => Row::check(St::Info, "moshi", "moshi-hook", "not on this computer (https://getmoshi.app)".into()).fix("queue its install"),
        Some(Some(s)) if !s.paired => Row::check(St::Warn, "moshi", "moshi-hook", format!("{} · not paired with the app yet", s.version))
            .fix("copy the pairing command")
            .why("Pair it once: in the Moshi app, Settings → Hooks → copy the token, then run moshi-hook pair --token <token> in a terminal. The token goes straight to moshi-hook; Aleph never sees it."),
        Some(Some(s)) => Row::check(
            if s.running { St::Ok } else { St::Warn },
            "moshi",
            "moshi-hook",
            format!(
                "{} · paired{} · {}",
                s.version,
                if s.host_name.is_empty() { String::new() } else { format!(" as {}", s.host_name) },
                if s.running { "running" } else { "its service isn't running" }
            ),
        ),
    };
    v.push(st);
    v.push(
        Row::choice(
            "moshi",
            "Aleph → Moshi",
            vec![
                ("auto".into(), "auto".into()),
                ("on".into(), "on".into()),
                ("off".into(), "off".into()),
            ],
            w.value("moshi"),
            w.pending("moshi"),
        )
        .why("Fleet cards, phone approvals as buttons and live job rows in Moshi, through moshi-hook. auto = on when moshi-hook is paired here."),
    );
    if matches!(&w.nt.moshi, Some(Some(s)) if s.paired) {
        v.push(
            Row::pick("moshi-on", "turn it all on", w.plan.has(&super::Action::MoshiOn))
                .note("aleph moshi install: cards, approvals, live jobs, the `aleph` tmux session")
                .why("Runs aleph moshi install in Review: the bridge on, approvals and live jobs on, the tmux session Moshi's picker opens, and a hello card."),
        );
    }
    let opts: Vec<(String, String)> = [
        ("auto", "auto"),
        ("full", "everything"),
        ("machine-only", "machine only"),
        ("off", "off"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    for h in w.fleet() {
        let key = format!("host.{h}.moshi");
        let cur = w.value(&key);
        let i = opts.iter().position(|(v, _)| *v == cur).unwrap_or(0);
        let detect = match w.probes.get(&h) {
            Some(p) if p.tool("moshi-hook").is_some() && !p.moshi.is_empty() => {
                format!(
                    "moshi-hook there tells the phone about {}",
                    p.moshi.join(", ")
                )
            }
            Some(p) if p.tool("moshi-hook").is_some() => {
                "moshi-hook there, no agents hooked".into()
            }
            Some(_) => "no moshi-hook there".into(),
            None => String::new(),
        };
        v.push(
            Row::local(format!("moshi:{h}"), h.to_string(), opts.clone(), i)
                .note(detect)
                .why("What Aleph sends Moshi about this machine: everything; only what moshi-hook can't see (the machine itself, limits, the queue); or nothing. auto = everything, except agents moshi-hook already reports from that machine (no double cards). host.<alias>.moshi."),
        );
    }
    v.push(Row::head("Web Push"));
    if super::has_sub("daemon", "pair") {
        v.push(
            Row::button("pair", "pair a phone (aleph daemon pair)")
                .why("Shows a QR code: the web console on your phone, with push notifications."),
        );
    } else {
        v.push(Row::info(
            "Phones get push with the web console (aleph daemon pair, coming).",
        ));
    }
    // when
    v.push(Row::head("When"));
    let presets: Vec<(String, String)> =
        std::iter::once(("custom".to_string(), "as set below".to_string()))
            .chain(PRESETS.iter().map(|(l, ..)| (l.to_string(), l.to_string())))
            .collect();
    v.push(
        Row::local("preset", "routes", presets, which_preset(w))
            .why("Only when I'm needed: questions, limits and problems. Everything: finished turns, starts and ends too. Quiet nights: only when needed, held 23:00–08:00 (problems still come through)."),
    );
    v.push(
        Row::input("notify.remote_on", "phone & webhooks get", w.value("notify.remote_on"), "blocked,limit,error — or all", w.pending("notify.remote_on"))
            .why("Event kinds sent to ntfy, Slack and the webhook: blocked, done, new, gone, error, auto, task, heal, limit, or all."),
    );
    v.push(Row::input(
        "notify.quiet",
        "quiet hours",
        w.value("notify.quiet"),
        "e.g. 23:00-08:00 (empty: none)",
        w.pending("notify.quiet"),
    ));
    v.push(Row::input(
        "notify.escalate_after",
        "ask again after",
        w.value("notify.escalate_after"),
        "e.g. 10m — a question still waiting is sent again",
        w.pending("notify.escalate_after"),
    ));
    // remote control
    v.push(Row::head("Remote control"));
    v.push(Row::info(
        "What phones, Raycast and the web may answer (never the deny list).",
    ));
    let hosts = w.fleet();
    let agents = matrix_agents(w);
    let m = matrix(w, "remote.allow", "remote.deny", &hosts, &agents);
    for (i, h) in hosts.iter().enumerate() {
        v.push(
            Row::matrix(format!("remote:{h}"), h.clone(), agents.clone(), m[i].clone())
                .why("A ticked cell lets phone buttons, Raycast and the web console answer that agent's questions on that machine (remote.allow, as host:agent globs). Questions matching the deny list (rm -rf, sudo, git push, …) are only ever answered at a terminal."),
        );
    }
    v
}

fn test_event() -> crate::events::Event {
    crate::events::Event {
        t: crate::cache::now(),
        host: "aleph".into(),
        name: "test".into(),
        kind: crate::events::Kind::Blocked,
        title: "If you can read this, Aleph's notifications reach you.".into(),
        ..Default::default()
    }
}

/// A topic with your own words first, then 8 random hex (pure but for the
/// randomness): `sam laptop!` → `https://ntfy.sh/sam-laptop-3f9a1c07`.
pub(crate) fn named_topic(name: &str, hex: &str) -> Option<String> {
    let mut slug = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        return None;
    }
    let slug: String = slug.chars().take(40).collect();
    Some(format!("https://ntfy.sh/{slug}-{hex}"))
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .is_ok();
    if !ok {
        let t = crate::cache::now() ^ u64::from(std::process::id()) << 20;
        for (i, x) in b.iter_mut().enumerate() {
            *x = (t >> ((i % 8) * 8)) as u8;
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A topic nobody will guess: `https://ntfy.sh/aleph-<16 hex>`.
fn new_topic() -> String {
    let mut b = [0u8; 8];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .is_ok();
    if !ok {
        let t = crate::cache::now() ^ u64::from(std::process::id()) << 20;
        b = t.to_le_bytes();
    }
    format!(
        "https://ntfy.sh/aleph-{}",
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    )
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Button(id) if id == "ntfy-show" => w.nt.reveal = !w.nt.reveal,
        Ev::Fix(id) if id == "moshi" && matches!(w.nt.moshi, Some(None)) => {
            let a = Action::moshi_hook();
            let on = !w.plan.has(&a);
            w.plan.want(Step::Notify, a, on);
            w.toast(
                false,
                if on {
                    "queued: moshi-hook (Moshi's installer) — then pair it with the app: moshi-hook pair"
                } else {
                    "moshi-hook is out of the queue"
                },
            );
        }
        Ev::Button(id) if id == "ntfy-name" => {
            w.edit(
                "Your ntfy topic",
                "a name, e.g. sam-laptop — 8 random hex go after it so nobody guesses it",
                "",
                "ntfy-name",
            );
        }
        Ev::Input(id, name) if id == "ntfy-name" => match named_topic(&name, &random_hex(4)) {
            Some(t) => {
                w.set("notify.ntfy", &t);
                w.toast(
                    false,
                    format!(
                        "{} — subscribe to it in the ntfy app, then send a test",
                        t.trim_start_matches("https://")
                    ),
                );
                w.select_label("phone (ntfy)");
            }
            None if name.trim().is_empty() => {}
            None => w.toast(true, "letters and digits, please (e.g. sam-laptop)"),
        },
        Ev::Button(id) if id == "ntfy-new" => {
            let t = new_topic();
            w.set("notify.ntfy", &t);
            w.toast(
                false,
                "a private topic — subscribe to it in the ntfy app, then send a test",
            );
            w.select_label("phone (ntfy)");
        }
        Ev::Button(id) if id.starts_with("test:") => {
            let sink = id.trim_start_matches("test:").to_string();
            if w.offline() {
                w.toast(
                    true,
                    "offline — the test waits for the network (r when it's back)",
                );
                return None;
            }
            let key = format!("notify.{sink}");
            let url = w.value(&key);
            let url = if url.starts_with("secret:") {
                crate::secrets::resolve(&url).unwrap_or_default()
            } else {
                url
            };
            if !url.starts_with("http") {
                w.toast(true, format!("{key} isn't a URL yet"));
                return None;
            }
            let s = match sink.as_str() {
                "ntfy" => crate::notify::Sink::Ntfy(url),
                "slack" => crate::notify::Sink::Slack(url),
                _ => crate::notify::Sink::Webhook(url),
            };
            w.nt.sent.remove(&sink);
            w.spawn(
                &format!("send:{sink}"),
                format!("Sending a test to {sink}"),
                move || {
                    let r = crate::notify::send(&s, &test_event());
                    Msg::Sent(sink, r)
                },
            );
        }
        Ev::Fix(id) if id == "moshi" => {
            let cmd = "moshi-hook pair --token ";
            let ok = crate::tui::copy(cmd);
            w.toast(
                false,
                if ok {
                    "copied `moshi-hook pair --token ` — paste the app's token after it, in a terminal; then r here"
                } else {
                    "run moshi-hook pair --token <from the Moshi app> in a terminal, then r here"
                },
            );
        }
        Ev::Pick(id) if id == "moshi-on" => {
            let on = !w.plan.has(&super::Action::MoshiOn);
            w.plan.want(Step::Notify, super::Action::MoshiOn, on);
        }
        Ev::Button(id) if id == "pair" => {
            let h = Hand {
                title: "Pair a phone".into(),
                note:
                    "aleph daemon pair shows a QR code for your phone; it runs until you're done."
                        .into(),
                run: Run::aleph(&["daemon", "pair"]),
                step: Step::Notify,
                id: "pair".into(),
            };
            w.confirm_hand(h, vec![]);
        }
        Ev::Local(id, i) if id == "preset" => {
            if let Some((_, on, done, quiet)) = i.checked_sub(1).and_then(|j| PRESETS.get(j)) {
                w.set("notify.remote_on", on);
                w.set("notify_done", done);
                w.set("notify.quiet", quiet);
            }
        }
        Ev::Local(id, i) if id.starts_with("moshi:") => {
            let h = id.trim_start_matches("moshi:");
            let v = ["auto", "full", "machine-only", "off"][i.min(3)];
            w.set(&format!("host.{h}.moshi"), if v == "auto" { "" } else { v });
        }
        Ev::Cell(id, col) if id.starts_with("remote:") => {
            let h = id.trim_start_matches("remote:").to_string();
            let hosts = w.fleet();
            let agents = matrix_agents(w);
            let mut m = matrix(w, "remote.allow", "remote.deny", &hosts, &agents);
            if let Some(i) = hosts.iter().position(|x| *x == h) {
                if let Some(c) = m[i].get_mut(col) {
                    *c = !*c;
                }
            }
            let keep = super::split(&w.value("remote.allow"));
            let g = to_globs(&hosts, &agents, &m, &keep);
            w.set("remote.allow", &g.join(","));
        }
        Ev::After(..) => {}
        _ => {}
    }
    let _ = Kind::Head;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_topic_keeps_your_words_and_stays_unguessable() {
        assert_eq!(
            named_topic("Sam laptop!", "3f9a1c07").as_deref(),
            Some("https://ntfy.sh/sam-laptop-3f9a1c07")
        );
        assert_eq!(named_topic("  ", "00"), None);
        assert_eq!(named_topic("!!!", "00"), None);
        assert_eq!(random_hex(4).len(), 8);
        assert_ne!(random_hex(8), random_hex(8));
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn a_matrix_becomes_short_globs() {
        let hosts = s(&["b31", "gpu2"]);
        let agents = s(&["claude", "codex"]);
        let all = vec![vec![true, true], vec![true, true]];
        assert_eq!(to_globs(&hosts, &agents, &all, &[]), s(&["*:*"]));
        let col = vec![vec![true, false], vec![true, false]];
        assert_eq!(to_globs(&hosts, &agents, &col, &[]), s(&["*:claude"]));
        let row = vec![vec![true, true], vec![false, true]];
        assert_eq!(
            to_globs(&hosts, &agents, &row, &[]),
            s(&["*:codex", "b31:*"])
        );
        let one = vec![vec![false, true], vec![false, false]];
        assert_eq!(to_globs(&hosts, &agents, &one, &[]), s(&["b31:codex"]));
        let none = vec![vec![false, false], vec![false, false]];
        assert!(to_globs(&hosts, &agents, &none, &[]).is_empty());
        // a machine outside the matrix keeps its entry
        assert_eq!(
            to_globs(&hosts, &agents, &one, &s(&["dex42:*", "b31:claude"])),
            s(&["dex42:*", "b31:codex"])
        );
    }
}
