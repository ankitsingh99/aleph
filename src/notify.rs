//! Notifications that leave this computer: ntfy (phone push), Slack and a
//! generic JSON webhook. `aleph watch` keeps doing desktop notifications;
//! these are sent by the automation hub (`auto.rs`) for the event kinds in
//! `notify.remote_on` (default: only "needs you").
//!
//! Every sink is one `curl` with a 10 s limit, on a background thread.

use crate::cli::Args;
use crate::config::Config;
use crate::events::{Event, Kind};
use crate::out::{err, hint, json, ok, warn};
use serde_json::json;
use std::process::{Command, ExitCode};
use std::time::Duration;

const T_SEND: Duration = Duration::from_secs(12);

#[derive(Clone, Debug, PartialEq)]
pub enum Sink {
    /// `https://ntfy.sh/<topic>` (or your own ntfy server)
    Ntfy(String),
    /// a Slack incoming-webhook URL
    Slack(String),
    /// any URL; gets the event as JSON
    Webhook(String),
}

impl Sink {
    pub fn name(&self) -> &'static str {
        match self {
            Sink::Ntfy(_) => "ntfy",
            Sink::Slack(_) => "slack",
            Sink::Webhook(_) => "webhook",
        }
    }
}

pub fn sinks(cfg: &Config) -> Vec<Sink> {
    let mut v = Vec::new();
    let get = |k: &str| {
        cfg.get(k)
            .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
    };
    if let Some(u) = get("notify.ntfy") {
        v.push(Sink::Ntfy(u));
    }
    if let Some(u) = get("notify.slack") {
        v.push(Sink::Slack(u));
    }
    if let Some(u) = get("notify.webhook") {
        v.push(Sink::Webhook(u));
    }
    v
}

/// Should an event of this kind go out? (`notify.remote_on`)
pub fn wanted(cfg: &Config, k: Kind) -> bool {
    let on = cfg.get_or("notify.remote_on");
    on.split(',')
        .map(str::trim)
        .any(|w| w == "all" || w == k.as_str())
}

/// The one-line headline of an event (`▲ temper on b31 needs you`).
pub fn title_of(e: &Event) -> String {
    title(e)
}

fn title(e: &Event) -> String {
    let who = if e.name.is_empty() {
        e.host.clone()
    } else {
        format!("{} on {}", e.name, e.host)
    };
    match e.kind {
        Kind::Blocked => format!("▲ {who} needs you"),
        Kind::Done => format!("✓ {who} finished"),
        Kind::New => format!("+ {who} started"),
        Kind::Gone => format!("– {who} is gone"),
        Kind::Error => format!("✕ {} unreachable", e.host),
        Kind::Auto => format!("↯ {who} auto-approved"),
        Kind::Task => format!("≡ task sent to {who}"),
        Kind::Heal => format!("↻ herdr restarted on {}", e.host),
        Kind::Limit => format!("◷ {who} hit its usage limit"),
        Kind::HostUp => format!("● {} is back", e.host),
        Kind::Remote => format!("⇄ {who}: {}", e.title),
        Kind::Match => format!("◎ {who}: {}", e.title),
    }
}

/// One value in a curl config file (`-K`): quoted, with `\\ \" \n \r \t` escaped.
pub(crate) fn cq(v: &str) -> String {
    let mut o = String::with_capacity(v.len() + 2);
    o.push('"');
    for c in v.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The curl config for one sink (pure; tested). It goes to curl on stdin
/// (`curl -K -`), so the URL — whose path is the secret for ntfy topics and
/// Slack webhooks — never shows in `ps` or the debug log.
#[cfg(test)]
pub fn curl_config(s: &Sink, e: &Event) -> String {
    let m = crate::message::compose(e, &crate::message::Ctx::default());
    curl_config_msg(s, e, &m, None).unwrap_or_default()
}

/// [`curl_config`] for a composed message (and ntfy buttons). ntfy gets its
/// JSON form, posted to the server with the topic inside: titles keep their
/// Unicode, tags show as an emoji plus the machine and the agent, a tap opens
/// the session in the web console when it's reachable.
pub fn curl_config_msg(
    s: &Sink,
    e: &Event,
    m: &crate::message::Msg,
    actions: Option<&serde_json::Value>,
) -> Option<String> {
    let mut c = String::new();
    let mut opt = |k: &str, v: &str| {
        c.push_str(k);
        c.push_str(" = ");
        c.push_str(&cq(v));
        c.push('\n');
    };
    match s {
        Sink::Ntfy(url) => {
            let (base, topic) = crate::actions::split_topic(url)?;
            let mut v = json!({
                "topic": topic,
                "title": m.title,
                "message": if m.body.is_empty() { m.title.clone() } else { m.body.clone() },
                "priority": m.priority,
                "tags": m.tags,
            });
            if let Some(u) = &m.click {
                v["click"] = json!(u);
            }
            let mut acts: Vec<serde_json::Value> = actions
                .and_then(|a| a.as_array().cloned())
                .unwrap_or_default();
            if let Some(u) = &m.click {
                if acts.len() < 3 {
                    acts.push(json!({"action": "view", "label": "Open", "url": u}));
                }
            }
            if !acts.is_empty() {
                v["actions"] = json!(acts);
            }
            opt("header", "Content-Type: application/json");
            opt("data-binary", &v.to_string());
            opt("url", &base);
        }
        Sink::Slack(url) => {
            opt("header", "Content-Type: application/json");
            let link = m
                .click
                .as_ref()
                .map(|u| format!("\n<{u}|Open in Aleph>"))
                .unwrap_or_default();
            opt(
                "data-binary",
                &json!({"text": format!("*{}*\n{}{link}", m.title, m.body)}).to_string(),
            );
            opt("url", url);
        }
        Sink::Webhook(url) => {
            let mut v = e.to_json();
            v["summary"] = json!(m.title);
            v["message"] = json!(m.body);
            if let Some(u) = &m.click {
                v["open"] = json!(u);
            }
            opt("header", "Content-Type: application/json");
            opt("data-binary", &v.to_string());
            opt("url", url);
        }
    }
    Some(c)
}

/// Send a composed message to one sink (blocking). Err has the reason.
pub fn send_msg(
    s: &Sink,
    e: &Event,
    m: &crate::message::Msg,
    actions: Option<&serde_json::Value>,
) -> Result<(), String> {
    // showcase's demo world reaches no network
    if let Some(e) = crate::showcase::refused() {
        return Err(e);
    }
    // showcase mode: the phone sees the same space references as the screen
    let masked;
    let m = if crate::showcase::on() {
        masked = crate::message::Msg {
            title: crate::showcase::text(&m.title).into_owned(),
            body: crate::showcase::text(&m.body).into_owned(),
            tags: m
                .tags
                .iter()
                .map(|t| crate::showcase::text(t).into_owned())
                .collect(),
            ..m.clone()
        };
        &masked
    } else {
        m
    };
    let cfg = curl_config_msg(s, e, m, actions).ok_or("notify.ntfy isn't a topic URL")?;
    let o = crate::proc::run(
        Command::new("curl").args(["-fsS", "-m", "10", "-o", "/dev/null", "-K", "-"]),
        T_SEND,
        Some(cfg.as_bytes()),
    )
    .map_err(|x| format!("curl: {x}"))?;
    if o.ok() {
        Ok(())
    } else {
        Err(o.reason())
    }
}

/// The value after `--name` in `rest`.
fn opt<'a>(rest: &'a [String], name: &str) -> Option<&'a str> {
    rest.iter()
        .position(|x| x == name)
        .and_then(|i| rest.get(i + 1))
        .map(String::as_str)
}

/// `aleph notify test --host H --agent A [--kind needs_you|done|limit]
/// [--to ntfy,moshi,raycast,desktop]`: a test alert about that machine and
/// agent, composed the way a real one is (marked as a test), to each
/// channel. Raycast shows it from the Inbox (the menu bar is refreshed); the
/// hub never sends a test event again.
fn test_one(a: &Args) -> ExitCode {
    use crate::message::{compose, open_url, Ctx};
    let host = a
        .flag
        .clone()
        .unwrap_or_else(|| crate::sshcfg::LOCAL.to_string());
    let agent = match crate::agents::resolve(opt(a.rest, "--agent").unwrap_or("claude")) {
        Ok(id) => id.to_string(),
        Err(e) => return crate::out::fail(&e),
    };
    let kind = match opt(a.rest, "--kind").unwrap_or("needs_you") {
        "needs_you" | "blocked" => Kind::Blocked,
        "done" | "finished" => Kind::Done,
        "limit" => Kind::Limit,
        k => return crate::out::fail(&format!("--kind {k}: needs_you, done or limit")),
    };
    let to: Vec<String> = opt(a.rest, "--to")
        .unwrap_or("ntfy,moshi,raycast")
        .split(',')
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
        .collect();
    let label = crate::agents::label(&agent);
    // a real session of that agent there lends its folder and task
    let t = a.cfg.target_for(&host, crate::herdr::machine_for);
    let live = crate::herdr::Herdr::new(&t)
        .sessions()
        .unwrap_or_default()
        .into_iter()
        .find(|s| crate::agents::same(&s.agent, &agent));
    let task = live
        .as_ref()
        .map(|s| s.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_default();
    let e = Event {
        t: crate::cache::now(),
        host: host.clone(),
        name: live
            .as_ref()
            .map(|s| s.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| label.clone()),
        kind,
        title: if task.is_empty() {
            "Test alert: does it reach you?".into()
        } else {
            task.clone()
        },
        agent: agent.clone(),
        source: "test".into(),
        ..Default::default()
    };
    let question = "Does this reach you?".to_string();
    let c = Ctx {
        agent: label.clone(),
        folder: live
            .as_ref()
            .map(|s| crate::ui::tilde_path(&s.cwd))
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| "~".into()),
        task,
        question: if kind == Kind::Blocked {
            question.clone()
        } else {
            String::new()
        },
        options: if kind == Kind::Blocked {
            vec![(1, "Yes".into()), (2, "No".into())]
        } else {
            vec![]
        },
        since: live
            .as_ref()
            .and_then(|s| s.since)
            .map(|t| e.t.saturating_sub(t)),
        open_url: open_url(&a.cfg, &e),
        ..Default::default()
    };
    let m = compose(&e, &c);
    let mut bad = 0;
    let mut report = |t: &str, r: Result<String, String>| match r {
        Ok(note) => ok(&format!("{t}: {note}")),
        Err(x) => {
            bad += 1;
            err(&format!("{t}: {x}"));
        }
    };
    let sent = format!("“{}” · {label} on {host}", m.title);
    // Moshi waits for an answer: everything else goes first
    let mut order = to.clone();
    order.sort_by_key(|t| t == "moshi");
    for t in &order {
        match t.as_str() {
            "ntfy" => {
                report(
                    t,
                    match sinks(&a.cfg)
                        .into_iter()
                        .find(|s| matches!(s, Sink::Ntfy(_)))
                    {
                        Some(s) => {
                            // buttons that come back the real way (signed, checked), touch nothing
                            let b = crate::actions::test_actions(
                                &a.cfg,
                                &host,
                                &agent,
                                e.t,
                                ("✓ 1. Yes", "✕ 2. No"),
                            );
                            send_msg(&s, &e, &m, b.as_ref()).map(|_| {
                            if b.is_some() {
                                format!("{sent} — tap a button: the daemon confirms it on your phone")
                            } else {
                                format!("{sent} (no signing key: no buttons)")
                            }
                        })
                        }
                        None => Err("notify.ntfy isn't set".into()),
                    },
                )
            }
            "moshi" => report(t, moshi_test(a, &host, &agent, &label, &m, &question, kind)),
            // the menu bar lists the Inbox: put it there, then refresh it
            "raycast" => report(t, {
                crate::events::append(e.clone());
                if cfg!(target_os = "macos") {
                    let link = a.cfg.get_or("notify.raycast.deeplink");
                    let o = crate::proc::cap("open", &["-g", &link], Duration::from_secs(5));
                    if o.ok() {
                        Ok(format!("{sent} — in Raycast's menu bar and the Inbox"))
                    } else {
                        Err(o.reason())
                    }
                } else {
                    Err("Raycast is macOS only".into())
                }
            }),
            "desktop" => report(
                t,
                crate::watch::deliver_event(&a.cfg, &e)
                    .then(|| sent.clone())
                    .ok_or_else(|| "no terminal-notifier / osascript / notify-send".into()),
            ),
            other => report(
                other,
                Err(format!("{other}? (ntfy, moshi, raycast, desktop)")),
            ),
        }
    }
    ExitCode::from(if bad > 0 { 1 } else { 0 })
}

/// Moshi's part of a test: a real question with buttons (as phone approvals
/// get them), waited for and recorded; `--no-wait` sends a card instead.
fn moshi_test(
    a: &Args,
    host: &str,
    agent: &str,
    label: &str,
    m: &crate::message::Msg,
    question: &str,
    kind: Kind,
) -> Result<String, String> {
    if crate::moshi::bin().is_none() {
        return Err("moshi-hook isn't installed here".into());
    }
    let card = || {
        crate::moshi::notify(&crate::moshi::Note {
            label: format!("{host} · {label}"),
            title: m.title.clone(),
            message: m.body.clone(),
            strict: true,
            ..Default::default()
        })
        .map(|_| format!("card “{}” · {label} on {host}", m.title))
    };
    if kind != Kind::Blocked || a.rest.iter().any(|x| x == "--no-wait") {
        return card();
    }
    let place = m.body.lines().next().unwrap_or("").to_string();
    // Moshi's app answers only Approve / Deny unless `moshi.choices`
    let choices = a.cfg.flag("moshi.choices");
    let q = crate::moshi::Ask {
        label: format!("{host} · {label}"),
        question: format!(
            "{place}\n\n{question}\n\n{}",
            if choices {
                "A test: nothing is really waiting."
            } else {
                "A test: Approve or Deny, either answer comes back to Aleph. Nothing is really waiting."
            }
        ),
        options: if choices {
            vec![
                crate::moshi::button(1, "Yes"),
                crate::moshi::button(2, "No"),
            ]
        } else {
            vec![]
        },
        timeout: Duration::from_secs(300),
    };
    let child = crate::moshi::ask_spawn(&q)?;
    let sp = crate::out::Spinner::start(format!(
        "Moshi: asked “{}” — answer it on your phone (5 min)",
        m.title
    ));
    let t0 = std::time::Instant::now();
    let ans = crate::moshi::ask_wait(
        child,
        choices,
        &std::sync::atomic::AtomicBool::new(false),
        Duration::from_secs(330),
    );
    sp.stop();
    let took = crate::ui::dur(t0.elapsed().as_secs());
    let what = match &ans {
        crate::moshi::Answer::Chosen(b) => format!("answered “{b}” after {took}"),
        crate::moshi::Answer::Approved => format!("approved after {took}"),
        crate::moshi::Answer::Denied => format!("denied after {took}"),
        crate::moshi::Answer::TimedOut => return Err("no answer in 5 minutes".into()),
        crate::moshi::Answer::Unreachable(e) => return Err(format!("Moshi unreachable: {e}")),
        crate::moshi::Answer::Failed(e) => return Err(e.clone()),
    };
    crate::events::append(Event {
        t: crate::cache::now(),
        host: host.to_string(),
        name: format!("test-{agent}"),
        kind: Kind::Remote,
        title: format!("test: Moshi {what}"),
        agent: agent.to_string(),
        source: "test".into(),
        ..Default::default()
    });
    Ok(format!(
        "question “{}” · {label} on {host} — {what}",
        m.title
    ))
}

/// Send to one sink now (blocking). Err has the reason.
pub fn send(s: &Sink, e: &Event) -> Result<(), String> {
    let m = crate::message::compose(e, &crate::message::Ctx::default());
    send_msg(s, e, &m, None)
}

/// Send `e` to every configured sink, on a background thread.
#[allow(dead_code)] // the router (router.rs) sends now; kept for scripts
pub fn dispatch(cfg: &Config, e: &Event) {
    if !wanted(cfg, e.kind) {
        return;
    }
    let sinks = sinks(cfg);
    if sinks.is_empty() {
        return;
    }
    let e = e.clone();
    std::thread::spawn(move || {
        for s in &sinks {
            if let Err(x) = send(s, &e) {
                crate::log::warn("notify", &format!("{} failed: {x}", s.name()));
            }
        }
    });
}

/// `aleph notify [test]`
pub fn cmd(mut a: Args) -> ExitCode {
    let p: Vec<String> = a.positional().iter().map(|s| s.to_string()).collect();
    match p.first().map(String::as_str) {
        // watch rules: matches in sessions' output land in the Inbox
        Some("watch") => return crate::watchrules::cmd(a, &p),
        // do not disturb: on, off, or for a while
        Some("dnd") => {
            let v = match p.get(1).map(String::as_str) {
                None | Some("on") => "on".to_string(),
                Some("off") => String::new(),
                Some(d) => match crate::log::parse_duration(d) {
                    Some(x) => (crate::cache::now() + x.as_secs()).to_string(),
                    None => {
                        return crate::out::fail("usage: aleph notify dnd [on | off | 30m | 2h]")
                    }
                },
            };
            let r = if v.is_empty() {
                a.cfg.unset("notify.dnd")
            } else {
                a.cfg.set("notify.dnd", &v)
            };
            return match r {
                Ok(()) if v.is_empty() => {
                    ok("notifications are back on");
                    ExitCode::SUCCESS
                }
                Ok(()) => {
                    ok(&format!(
                        "do not disturb{} — problems ({}) still come through",
                        v.parse::<u64>()
                            .map(|u| format!(" until {}", crate::ui::clock(u)))
                            .unwrap_or_default(),
                        a.cfg.get_or("notify.quiet_pass")
                    ));
                    ExitCode::SUCCESS
                }
                Err(e) => crate::out::fail(&format!("couldn't save: {e}")),
            };
        }
        // mute one session for a while
        Some("mute") | Some("unmute") => {
            let Some((h, n)) = p.get(1).and_then(|x| x.split_once(':')) else {
                return crate::out::fail("usage: aleph notify mute host:name [1h | off]");
            };
            let secs = match (p[0].as_str(), p.get(2).map(String::as_str)) {
                ("unmute", _) | (_, Some("off")) => 0,
                (_, Some(d)) => crate::log::parse_duration(d).map_or(3600, |x| x.as_secs()),
                _ => 3600,
            };
            if crate::router::mute(h, n, secs) {
                ok(&format!("{h}:{n} muted for {}", crate::ui::dur(secs)));
            } else {
                ok(&format!("{h}:{n} unmuted"));
            }
            return ExitCode::SUCCESS;
        }
        // a token as a phone button would send it (testing; needs the key here)
        Some("token") => {
            let (Some(q), Some(act)) = (p.get(1), p.get(2)) else {
                return crate::out::fail("usage: aleph notify token host:name approve|deny");
            };
            let Some((_, s)) = crate::cli::find_quiet(&a.cfg, &a.flag, q) else {
                return crate::out::fail(&format!("no session '{q}'"));
            };
            let Some(key) = crate::actions::key() else {
                return crate::out::fail("no signing key");
            };
            let act = if act == "deny" {
                crate::actions::Act::Deny
            } else {
                crate::actions::Act::Approve
            };
            println!(
                "{}",
                crate::actions::encode(
                    &key,
                    &crate::actions::Grant {
                        host: s.host.clone(),
                        pane: s.pane_id.clone(),
                        since: s.since.unwrap_or(0),
                        act,
                        exp: crate::cache::now() + crate::actions::TTL,
                    }
                )
            );
            return ExitCode::SUCCESS;
        }
        // where each kind of event goes
        Some("routes") => {
            let rs = crate::router::routes(&a.cfg, true);
            if a.has("--json") {
                json(&serde_json::Value::Array(
                    rs.iter()
                        .map(|r| json!({"when": r.when, "hosts": r.hosts, "agents": r.agents, "to": r.to.iter().map(|t| t.name()).collect::<Vec<_>>(), "quiet": r.quiet, "escalate_after": r.escalate_after.map(|d| d.as_secs()), "escalate_to": r.escalate_to.iter().map(|t| t.name()).collect::<Vec<_>>()}))
                        .collect(),
                ));
                return ExitCode::SUCCESS;
            }
            if rs.is_empty() {
                warn("no routes — nothing leaves this computer (desktop notifications need aleph watch)");
            }
            for r in &rs {
                let when = if r.when.is_empty() {
                    "all".into()
                } else {
                    r.when.join(", ")
                };
                let to: Vec<&str> = r.to.iter().map(|t| t.name()).collect();
                let mut extra = String::new();
                if let Some((x, y)) = r.quiet {
                    extra.push_str(&format!(
                        "  quiet {:02}:{:02}-{:02}:{:02}",
                        x / 60,
                        x % 60,
                        y / 60,
                        y % 60
                    ));
                }
                if let Some(e) = r.escalate_after {
                    extra.push_str(&format!(
                        "  escalates after {}",
                        crate::ui::dur(e.as_secs())
                    ));
                }
                crate::out::say(ratatui::text::Line::raw(format!(
                    "  {when:<28} → {}{extra}",
                    to.join(", ")
                )));
            }
            if crate::router::dnd(&a.cfg).is_some() {
                warn("do not disturb is on (aleph notify dnd off)");
            }
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let v = sinks(&a.cfg);
    let test = p.first().map(String::as_str) == Some("test");
    // one machine and agent, to chosen channels, in the real words
    if test && (a.flag.is_some() || a.rest.iter().any(|x| x == "--agent" || x == "--to")) {
        return test_one(&a);
    }
    if a.has("--json") && !test {
        json(&json!({
            "sinks": v.iter().map(|s| s.name()).collect::<Vec<_>>(),
            "remote_on": a.cfg.get_or("notify.remote_on"),
        }));
        return ExitCode::SUCCESS;
    }
    if v.is_empty() {
        warn("no notification sinks yet — desktop notifications come from aleph watch");
        hint("phone: aleph settings set notify.ntfy https://ntfy.sh/<a-secret-topic>  (then subscribe in the ntfy app)");
        hint("Slack: aleph settings set notify.slack https://hooks.slack.com/services/…");
        hint(
            "anything else: aleph settings set notify.webhook https://…  (gets the event as JSON)",
        );
        return ExitCode::SUCCESS;
    }
    if !test {
        for s in &v {
            ok(&format!("{} is set up", s.name()));
        }
        hint(&format!(
            "sent for: {}  (notify.remote_on) · aleph notify test sends one now",
            a.cfg.get_or("notify.remote_on")
        ));
        return ExitCode::SUCCESS;
    }
    let e = Event {
        t: crate::cache::now(),
        host: "aleph".into(),
        name: "test".into(),
        agent: String::new(),
        kind: Kind::Blocked,
        title: "If you can read this, phone notifications work.".into(),
        ..Default::default()
    };
    // a test says what it is, not what a question would
    let m = crate::message::Msg {
        title: format!("{} test notification", crate::brand::NAME),
        body: "If you can read this, Aleph's alerts reach this device: questions, finished turns, limits and machines going down.".into(),
        tags: vec!["white_check_mark".into(), "aleph".into()],
        priority: 4,
        click: crate::message::open_url(&a.cfg, &e),
    };
    let mut bad = 0;
    for s in &v {
        match send_msg(s, &e, &m, None) {
            Ok(()) => ok(&format!("{}: sent", s.name())),
            Err(x) => {
                bad += 1;
                err(&format!("{}: {x}", s.name()));
            }
        }
    }
    ExitCode::from(if bad > 0 { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: Kind) -> Event {
        Event {
            t: 1,
            host: "b31".into(),
            name: "ignite".into(),
            kind,
            title: "Run the benchmark?".into(),
            agent: "claude".into(),
            ..Default::default()
        }
    }

    #[test]
    fn sinks_and_filter() {
        let cfg = Config::in_memory(&[
            "notify.ntfy=https://ntfy.sh/abc",
            "notify.slack=not-a-url",
            "notify.webhook=http://localhost:9/x",
            "notify.remote_on=blocked,done",
        ]);
        assert_eq!(
            sinks(&cfg),
            vec![
                Sink::Ntfy("https://ntfy.sh/abc".into()),
                Sink::Webhook("http://localhost:9/x".into())
            ]
        );
        assert!(
            wanted(&cfg, Kind::Blocked) && wanted(&cfg, Kind::Done) && !wanted(&cfg, Kind::New)
        );
    }

    #[test]
    fn curl_lines() {
        let c = curl_config(&Sink::Ntfy("https://ntfy.sh/t".into()), &ev(Kind::Blocked));
        // ntfy's JSON form: the topic inside, posted to the server
        assert!(c.contains("\\\"topic\\\":\\\"t\\\""), "{c}");
        assert!(c.contains("\\\"priority\\\":4"), "{c}");
        assert!(c.contains("ignite needs you"), "{c}");
        assert!(c.contains("raising_hand"), "{c}");
        assert!(c.ends_with("url = \"https://ntfy.sh\"\n"), "{c}");
        let c = curl_config(&Sink::Slack("https://hooks/x".into()), &ev(Kind::Done));
        assert!(c.contains("ignite finished its turn"), "{c}");
        assert!(c.contains("\\\"text\\\""), "{c}"); // JSON quotes escaped for curl
        let c = curl_config(&Sink::Webhook("https://w".into()), &ev(Kind::Auto));
        assert!(c.contains("\\\"kind\\\":\\\"auto\\\""), "{c}");
        assert_eq!(cq("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
}
