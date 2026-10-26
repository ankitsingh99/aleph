//! Acting on every marked session at once (Fleet's `space` marks): approve,
//! send, queue and close for all of them, with an action bar on the hint
//! line while anything is marked. And closing with a grace
//! (`crate::closing`): the row greys out "closing in 5s · u undo" until
//! then, and `u` keeps it.
//!
//! Marked sessions can be on different machines. Each machine's share runs
//! in a thread of its own (its sessions in turn, over its one connection),
//! off the UI thread, and one toast sums them all up.

use super::{fleet, Act, Bg, Dash, Level};
use crate::closing::{self, Entry, Sel};
use crate::herdr::{Herdr, Session};
use crate::ui::{self, c};
use ratatui::prelude::*;
use std::sync::Arc;
use std::thread;

/// How one session went.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Res {
    Done,
    /// left alone, and why (`not asking`)
    Skipped(String),
    Failed(String),
}

/// The marked sessions still in the fleet.
pub(crate) fn marked(d: &Dash) -> Vec<Session> {
    d.fleet
        .sessions()
        .into_iter()
        .filter(|s| d.fv.marked.contains(&s.key()))
        .cloned()
        .collect()
}

/// The sessions by machine, machines in the order first seen.
pub(crate) fn by_host(v: &[Session]) -> Vec<(String, Vec<Session>)> {
    let mut out: Vec<(String, Vec<Session>)> = Vec::new();
    for s in v {
        match out.iter_mut().find(|(h, _)| *h == s.host) {
            Some((_, g)) => g.push(s.clone()),
            None => out.push((s.host.clone(), vec![s.clone()])),
        }
    }
    out
}

/// Names, or how many when there are more than three.
fn names(v: &[&str], what: &str) -> String {
    if v.len() <= 3 {
        v.join(", ")
    } else {
        format!("{} {what}", v.len())
    }
}

/// One toast for a batch — pure: what was done (`verb`: "approved", "sent
/// to"), what failed, what was skipped and why. `ok`: something was done
/// and nothing failed.
pub(crate) fn summary(verb: &str, out: &[(String, Res)]) -> (bool, String) {
    let done: Vec<&str> = out
        .iter()
        .filter(|(_, r)| *r == Res::Done)
        .map(|(n, _)| n.as_str())
        .collect();
    let failed: Vec<(&str, &str)> = out
        .iter()
        .filter_map(|(n, r)| match r {
            Res::Failed(e) => Some((n.as_str(), e.as_str())),
            _ => None,
        })
        .collect();
    // skipped, grouped by why (in the order first seen)
    let mut why: Vec<(&str, Vec<&str>)> = Vec::new();
    for (n, r) in out {
        if let Res::Skipped(w) = r {
            match why.iter_mut().find(|(x, _)| *x == w.as_str()) {
                Some((_, v)) => v.push(n),
                None => why.push((w, vec![n])),
            }
        }
    }
    let mut parts = Vec::new();
    if !done.is_empty() {
        parts.push(format!("{verb} {}", names(&done, "sessions")));
    }
    match failed.as_slice() {
        [] => {}
        [(n, e)] => parts.push(format!("{n} failed: {e}")),
        [(n, e), ..] => parts.push(format!("{} failed ({n}: {e})", failed.len())),
    }
    if !why.is_empty() {
        let n: usize = why.iter().map(|(_, v)| v.len()).sum();
        let each: Vec<String> = why
            .iter()
            .map(|(w, v)| format!("{w}: {}", names(v, "of them")))
            .collect();
        parts.push(format!("skipped {n} ({})", each.join("; ")));
    }
    if parts.is_empty() {
        parts.push(format!("nothing {verb}"));
    }
    (!done.is_empty() && failed.is_empty(), parts.join(" · "))
}

/// Run `f` on every session in `v`: a thread per machine (its sessions in
/// turn), all off the UI thread; then one toast (`pre`: results known
/// already, like those skipped up front), and each machine polled again.
pub(crate) fn fan_out(
    d: &Dash,
    verb: &'static str,
    v: Vec<Session>,
    pre: Vec<(String, Res)>,
    f: impl Fn(&Herdr, &Session) -> Res + Send + Sync + 'static,
) {
    let groups: Vec<(String, crate::config::Target, Vec<Session>)> = by_host(&v)
        .into_iter()
        .map(|(h, ss)| {
            let t = d.target_of(&h);
            (h, t, ss)
        })
        .collect();
    let tx = d.tx.clone();
    let f = Arc::new(f);
    thread::spawn(move || {
        let jobs: Vec<_> = groups
            .into_iter()
            .map(|(h, t, ss)| {
                let f = f.clone();
                thread::spawn(move || {
                    let herdr = Herdr::new(&t);
                    let r: Vec<(String, Res)> = ss
                        .iter()
                        .map(|s| (s.display().to_string(), f(&herdr, s)))
                        .collect();
                    (h, r)
                })
            })
            .collect();
        let mut out = pre;
        let mut hosts = Vec::new();
        for j in jobs {
            if let Ok((h, r)) = j.join() {
                hosts.push(h);
                out.extend(r);
            }
        }
        let (ok, msg) = summary(verb, &out);
        // every machine looked at again (an empty message toasts nothing)
        for h in hosts.iter().skip(1) {
            let _ = tx.send(Bg::Done {
                ok: true,
                msg: String::new(),
                refresh: Some(h.clone()),
            });
        }
        let _ = tx.send(Bg::Done {
            ok,
            msg,
            refresh: hosts.first().cloned(),
        });
    });
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

// ------------------------------------------------------------------ the actions

/// `y` with sessions marked: each one asking gets its question's yes
/// option (`herdr::approve_keys`, never a blind 1). Those not asking, those
/// whose question is on `approve_deny`, and those without a clear yes are
/// skipped, and the toast says so.
pub(crate) fn approve(d: &mut Dash, v: Vec<Session>) {
    let n = v.len();
    let (asking, not): (Vec<Session>, Vec<Session>) = v.into_iter().partition(|s| s.blocked());
    if asking.is_empty() {
        d.toast(
            Level::Info,
            if n == 1 {
                "the marked session isn't asking anything".to_string()
            } else {
                format!("none of the {n} marked is asking anything")
            },
        );
        return;
    }
    let pre: Vec<(String, Res)> = not
        .iter()
        .map(|s| (s.display().to_string(), Res::Skipped("not asking".into())))
        .collect();
    d.toast(
        Level::Info,
        format!(
            "approving {} session{}…",
            asking.len(),
            plural(asking.len())
        ),
    );
    let cfg = d.cfg.clone();
    fan_out(d, "approved", asking, pre, move |h, s| {
        approve_one(&cfg, h, s)
    });
}

/// One approve: its question read, the deny list checked, the yes option sent.
fn approve_one(cfg: &crate::config::Config, h: &Herdr, s: &Session) -> Res {
    let q = match h.read(&s.pane_id, "detection", 0, false) {
        Ok(q) => q,
        Err(e) => return Res::Failed(e),
    };
    // a batch never answers what approve_deny keeps for a terminal
    if let Some(d) = crate::permit::terminal_only(cfg, &q) {
        return Res::Skipped(format!("on approve_deny ({d})"));
    }
    let Some(keys) = crate::herdr::approve_keys(&q) else {
        return Res::Skipped("no clear yes option".into());
    };
    let k: Vec<&str> = keys.iter().map(String::as_str).collect();
    match h.send_keys(&s.pane_id, &k) {
        Ok(()) => Res::Done,
        Err(e) => Res::Failed(e),
    }
}

/// One prompt to every session (`s` and `b` with sessions marked).
pub(crate) fn send(d: &mut Dash, v: Vec<Session>, text: String) {
    fan_out(d, "sent to", v, Vec::new(), move |h, s| {
        match h.prompt(&s.pane_id, &text) {
            Ok(()) => Res::Done,
            Err(e) => Res::Failed(e),
        }
    });
}

/// The same task queued for each session, sent once it's free.
pub(crate) fn queue(d: &mut Dash, v: &[Session], text: &str) {
    for s in v {
        crate::queue::add(crate::queue::Task {
            host: s.host.clone(),
            prompt: text.to_string(),
            to: Some(s.name.clone()),
            ..Default::default()
        });
    }
    d.note(match v {
        [s] => format!("queued for {} — sent once it's free", s.display()),
        _ => format!(
            "queued for {} sessions — each gets it once it's free",
            v.len()
        ),
    });
}

/// The send form for these sessions (`s` and `b` with sessions marked):
/// enter sends to each, tab queues it for each.
pub(crate) fn send_form(d: &mut Dash, v: Vec<Session>) {
    let title = match v.as_slice() {
        [s] => format!("Prompt {}", s.display()),
        _ => format!("Send to {} sessions", v.len()),
    };
    let hint = if v.len() == 1 {
        "enter sends it as if you typed it · tab queues it for when it's free"
    } else {
        "enter sends it to each now · tab queues it for each, sent once it's free"
    };
    d.mode = super::Mode::Input(super::forms::Input::new(
        title,
        hint,
        String::new(),
        super::forms::For::Send(v),
    ));
}

/// The queue form for these sessions (`Q` with sessions marked).
pub(crate) fn queue_form(d: &mut Dash, v: Vec<Session>) {
    d.mode = super::Mode::Input(super::forms::Input::new(
        format!("Queue for {} session{}", v.len(), plural(v.len())),
        "each gets it once it's free (idle or done)",
        String::new(),
        super::forms::For::QueueEach(v),
    ));
}

// ------------------------------------------------------------------ closing

/// Close them (a confirm said yes): after `close.grace`, through the hub
/// (`closing.json`) — the rows grey out until then and `u` keeps them; at
/// once when the grace is 0.
pub(crate) fn close(d: &mut Dash, v: Vec<Session>) {
    d.fv.marked.clear();
    if v.is_empty() {
        return;
    }
    let g = closing::grace(&d.cfg);
    if g.is_zero() {
        fan_out(d, "closed", v, Vec::new(), |h, s| {
            let _ = closing::undo(Sel::Session(&s.host, &s.pane_id));
            match h.close_pane(&s.pane_id) {
                Ok(()) => {
                    crate::restore::mark_closed(&s.host, &s.name);
                    Res::Done
                }
                Err(e) => Res::Failed(e),
            }
        });
        return;
    }
    let e = closing::request(&v, g, false, "dashboard");
    let who = match e.as_slice() {
        [one] => one.display.clone(),
        _ => format!("{} sessions", e.len()),
    };
    d.toast(
        Level::Info,
        format!(
            "closing {who} in {} · {} undo",
            closing::grace_text(g),
            d.keys.key_for(Act::Undo)
        ),
    );
}

/// `u`: keep the selected session if it's closing, else the last close
/// asked (all of a batch).
pub(crate) fn undo(d: &mut Dash) {
    let pending = closing::current();
    let sel = fleet::selected_session(d).filter(|s| {
        pending
            .iter()
            .any(|e| e.host == s.host && e.pane == s.pane_id)
    });
    let (kept, late) = match &sel {
        Some(s) => closing::undo(Sel::Session(&s.host, &s.pane_id)),
        None => closing::undo(Sel::Latest),
    };
    match (kept.as_slice(), late.first()) {
        ([e], _) => d.note(format!("kept {} — it won't close", e.display)),
        ([_, ..], _) => d.note(format!("kept {} sessions — they won't close", kept.len())),
        ([], Some(e)) => d.toast(
            Level::Warn,
            format!("too late: {} is closing already", e.display),
        ),
        ([], None) => d.toast(Level::Info, "nothing is closing"),
    }
}

/// Is the last close asked one this dashboard asked, still in its grace?
/// (Then `u` keeps it from any view, as its toast said.)
pub(crate) fn undo_window(_d: &Dash) -> bool {
    closing::current()
        .iter()
        .filter(|e| e.claimed.is_none())
        .max_by_key(|e| e.batch)
        .is_some_and(|e| e.pid == std::process::id())
}

/// A closing row's note instead of its folder and title: "closing in 4s ·
/// u undo", or "closing…" once it's under way.
pub(crate) fn closing_note(d: &Dash, e: &Entry) -> Vec<Span<'static>> {
    let now = closing::now_ms();
    if e.claimed.is_some() || e.left(now) == 0 {
        return vec![Span::styled("closing…", ui::fg(c::PEACH()))];
    }
    vec![
        Span::styled(format!("closing in {}s", e.left(now)), ui::fg(c::PEACH())),
        Span::styled(" · ", ui::faint()),
        Span::styled(
            d.keys.key_for(Act::Undo),
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" undo", ui::muted()),
    ]
}

// ------------------------------------------------------------------ the action bar

/// The hint line while sessions are marked: `◉ 3 marked · y approve · s
/// send · Q queue · x close · esc clear` (the keys as bound), fitted to `w`.
pub(crate) fn action_bar(d: &Dash, w: u16) -> Option<Line<'static>> {
    let v = marked(d);
    if v.is_empty() {
        return None;
    }
    let asking = v.iter().filter(|s| s.blocked()).count();
    let approve = if asking == 0 || asking == v.len() {
        "approve".to_string()
    } else {
        format!("approve {asking}")
    };
    // (key, label, how long it stays when room runs out: the lowest goes first)
    let mut items: Vec<(String, String, u8)> = vec![
        (d.keys.key_for(Act::Approve), approve, 3),
        (d.keys.key_for(Act::Send), "send".into(), 2),
        (d.keys.key_for(Act::Queue), "queue".into(), 1),
        (d.keys.key_for(Act::Close), "close".into(), 4),
        ("esc".into(), "clear".into(), 5),
    ];
    items.retain(|(k, _, _)| !k.is_empty());
    let pill = Span::styled(
        format!(" ◉ {} marked ", v.len()),
        if c::mono() {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::new()
                .fg(c::BASE())
                .bg(c::ACCENT())
                .add_modifier(Modifier::BOLD)
        },
    );
    let build = |items: &[(String, String, u8)]| {
        let mut s = vec![Span::raw(" "), pill.clone(), Span::raw("  ")];
        for (k, l, _) in items {
            s.push(ui::keycap(k));
            s.push(Span::styled(format!(" {l}   "), ui::muted()));
        }
        Line::from(s)
    };
    // too wide: queue goes first, then send, approve, close; esc stays
    while items.len() > 1 && ui::line_width(&build(&items)) > w as usize {
        let Some(i) = items
            .iter()
            .enumerate()
            .min_by_key(|(_, x)| x.2)
            .map(|(i, _)| i)
        else {
            break;
        };
        items.remove(i);
    }
    Some(build(&items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(host: &str, name: &str) -> Session {
        Session {
            host: host.into(),
            pane_id: format!("w1:{name}"),
            name: name.into(),
            label: format!("claude_{name}"),
            ..Default::default()
        }
    }

    #[test]
    fn machines_in_the_order_first_seen() {
        let v = vec![s("b31", "x"), s("gpu2", "y"), s("b31", "z")];
        let g = by_host(&v);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].0, "b31");
        assert_eq!(
            g[0].1.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["x", "z"]
        );
        assert_eq!(g[1].0, "gpu2");
    }

    #[test]
    fn one_toast_says_what_was_done_skipped_and_failed() {
        let r = |n: &str, x: Res| (n.to_string(), x);
        assert_eq!(
            summary("approved", &[r("ignite", Res::Done)]),
            (true, "approved ignite".into())
        );
        let (ok, m) = summary(
            "approved",
            &[
                r("ignite", Res::Done),
                r("temper", Res::Done),
                r("weave", Res::Skipped("not asking".into())),
                r("vega", Res::Skipped("not asking".into())),
                r("rigel", Res::Skipped("on approve_deny (*rm -rf*)".into())),
            ],
        );
        assert!(ok);
        assert_eq!(
            m,
            "approved ignite, temper · skipped 3 (not asking: weave, vega; on approve_deny (*rm -rf*): rigel)"
        );
        let (ok, m) = summary(
            "sent to",
            &[
                r("a", Res::Done),
                r("b", Res::Done),
                r("c", Res::Done),
                r("d", Res::Done),
                r("e", Res::Failed("timeout".into())),
            ],
        );
        assert!(!ok, "a failure is an error");
        assert_eq!(m, "sent to 4 sessions · e failed: timeout");
        let (ok, m) = summary(
            "approved",
            &[
                r("x", Res::Failed("gone".into())),
                r("y", Res::Failed("down".into())),
            ],
        );
        assert!(!ok);
        assert_eq!(m, "2 failed (x: gone)");
        let (ok, m) = summary("approved", &[r("x", Res::Skipped("not asking".into()))]);
        assert!(!ok, "nothing done");
        assert_eq!(m, "skipped 1 (not asking: x)");
        assert_eq!(summary("closed", &[]).1, "nothing closed");
    }
}
