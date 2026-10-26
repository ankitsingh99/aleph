//! `aleph showcase on [--demo [--seed N] [--fresh]] | off | status`: every
//! window — the dashboard, Raycast, the web console, the CLI, notifications
//! — switches together (they all read `state/showcase.json`); the daemon and
//! open dashboards start again by themselves when the demo world goes on or
//! off.

use super::state::{self, Mode};
use crate::out::{err, head, hint, ok, warn};
use std::process::ExitCode;
use std::time::Duration;

/// What the app's palette offers: (mode, label, what it does).
pub const MODES: &[(&str, &str, &str)] = &[
    ("off", "Showcase: off", "everything shows as it is"),
    (
        "masked",
        "Showcase: masked",
        "names, folders, addresses and versions as space references, everywhere",
    ),
    (
        "demo",
        "Showcase: the demo world",
        "a made-up fleet that works, waits and finishes on a script — nothing real is reached",
    ),
];

/// The palette's switch: the words for a toast.
pub fn switch_to(mode: &str) -> Result<String, String> {
    let m = match mode {
        "off" => Mode::Off,
        "masked" => Mode::Masked,
        "demo" => Mode::Demo,
        _ => return Err(format!("no showcase mode {mode}")),
    };
    let was = state::mode();
    // switching on (or between masked and demo): new stand-ins every time
    state::set(m, m != Mode::Off).map_err(|e| format!("couldn't switch: {e}"))?;
    Ok(match m {
        Mode::Off => "showcase off: everything shows as it is".into(),
        Mode::Masked => {
            "showcase on: space references everywhere (the daemon and Raycast too)".into()
        }
        Mode::Demo if was != Mode::Demo => {
            "the demo world: a made-up fleet, nothing real is reached".into()
        }
        Mode::Demo => "the demo world is on".into(),
    })
}

pub fn run(name: &str, rest: &[String]) -> Option<ExitCode> {
    if name != "showcase" {
        return refuse(name, rest);
    }
    let sub = rest.first().map(String::as_str).unwrap_or("status");
    let demo = rest.iter().any(|a| a == "--demo");
    let fresh = rest.iter().any(|a| a == "--fresh");
    let seed = rest.iter().enumerate().find_map(|(i, a)| {
        if a == "--seed" {
            Some(rest.get(i + 1).cloned().unwrap_or_default())
        } else {
            a.strip_prefix("--seed=").map(String::from)
        }
    });
    let seed = match seed.map(|s| s.parse::<u64>()) {
        Some(Ok(n)) => Some(n),
        Some(Err(_)) => {
            err("--seed needs a whole number");
            return Some(ExitCode::from(2));
        }
        None => None,
    };
    Some(match sub {
        "on" => switch(if demo { Mode::Demo } else { Mode::Masked }, seed, fresh),
        "off" => switch(Mode::Off, None, false),
        "status" => status(&rest[1.min(rest.len())..]),
        // the worlds names come from, and which are on
        "universes" => universes(),
        // delight: play the sound cues, to choose them
        "sound" | "sounds" => super::sound::cmd(&rest[1..]),
        // capture: a poster of a view, the recordings made
        "snap" | "poster" => super::capture::cmd::snap(&rest[1..]),
        "casts" => super::capture::cmd::casts(&rest[1..]),
        // a recording's MP4 and GIF (after a stop, or again)
        "render" => super::capture::cmd::render(&rest[1..]),
        // safety: the pause, from a shell (the panic key's switch)
        "panic" => super::safety::pause::cmd(rest.get(1).map(String::as_str)),
        // what's treated as private, and its stand-ins (this computer's own words)
        "words" => {
            let d = super::dict::get();
            for e in &d.entries {
                println!(
                    "{:<10} {:<28} {:<24} {}",
                    format!("{:?}", e.kind),
                    e.real,
                    e.full,
                    e.fit
                );
            }
            ExitCode::SUCCESS
        }
        _ => {
            err(&format!(
                "unknown: aleph showcase {sub} (on [--demo [--seed N] [--fresh]] | off | status | sound [event…] | snap [--view V] | casts | render FILE.cast | panic [on|off])"
            ));
            ExitCode::from(2)
        }
    })
}

/// Commands that would reach (or change) something real: refused while the
/// demo world is on.
fn refuse(name: &str, rest: &[String]) -> Option<ExitCode> {
    if !super::demo() {
        return None;
    }
    let sub = rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("");
    let no = match name {
        "wake" | "update" | "self-update" | "setup" | "connect" | "clone" | "restore" | "theme"
        | "moshi" => true,
        "daemon" => matches!(sub, "deploy" | "expose"),
        "host" => !matches!(sub, "" | "show" | "list" | "ls"),
        "agents" => matches!(sub, "install" | "setup"),
        "hooks" => matches!(sub, "install" | "uninstall"),
        _ => false,
    };
    if !no {
        return None;
    }
    err(&format!("aleph {name}: {}", super::demo::REFUSED));
    Some(ExitCode::from(1))
}

fn switch(m: Mode, seed: Option<u64>, fresh: bool) -> ExitCode {
    let was = state::mode();
    if m == Mode::Demo && fresh {
        // a new world: nothing done to it yet, no history
        for d in [crate::config::state_dir(), crate::config::cache_dir()] {
            let _ = std::fs::remove_dir_all(super::demo::sandbox(d));
        }
    }
    match state::set_seeded(m, seed, m != Mode::Off) {
        Ok(st) => {
            match m {
                Mode::Off => ok("showcase off: everything shows as it is"),
                Mode::Masked => {
                    ok("showcase on: names, folders, addresses and versions are space references now");
                    hint("every window switches within a second: the dashboard, Raycast, the web console, notifications");
                }
                Mode::Demo => {
                    ok(&format!(
                        "showcase on, with the demo world (seed {}): a made-up fleet, nothing real is reached",
                        st.seed
                    ));
                    seed_world();
                }
            }
            if (was == Mode::Demo) != (m == Mode::Demo) {
                hint("the daemon and open dashboards start again by themselves in the other world (a few seconds)");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            err(&format!("couldn't switch: {e}"));
            ExitCode::from(1)
        }
    }
}

/// The demo world's past: 30 days of usage (read the way the daemon reads a
/// machine, from the world's own logs), so Usage has history at once.
fn seed_world() {
    let cfg = crate::config::Config::load();
    let hosts = cfg.fleet();
    let sp = crate::out::Spinner::start("Writing the demo world's history".into());
    let reps = crate::usage::sync(&cfg, &hosts, false, Duration::from_secs(10), |_| {});
    sp.stop();
    // the sessions it started lately (History, recent folders, names)
    if crate::history::load(1).is_empty() {
        let now = super::demo::now();
        let w = super::demo::world();
        for (i, s) in w.sessions.iter().enumerate().rev() {
            crate::history::append(&crate::history::Entry {
                t: now - 3_600 * (2 + 7 * i as u64),
                host: s.host.clone(),
                dir: s.cwd.clone(),
                name: s.name.clone(),
                agent: s.agent.clone(),
                prompt: (s.agent != crate::agents::SHELL).then(|| s.task.prompt.to_string()),
                ..Default::default()
            });
        }
    }
    let recs: u64 = reps.iter().map(|r| r.records).sum();
    let down: Vec<&str> = reps
        .iter()
        .filter(|r| r.error.is_some())
        .map(|r| r.host.as_str())
        .collect();
    ok(&format!(
        "{} machines, {} sessions · {recs} usage records added",
        hosts.len(),
        super::demo::world().sessions.len()
    ));
    if !down.is_empty() {
        hint(&format!(
            "{} is down for a minute every ten (on purpose): its history comes with the next read",
            down.join(", ")
        ));
    }
}

/// demo flow: the world's clock (time warp) and its story, in words.
fn demo_flow_lines(v: &serde_json::Value) {
    use crate::ui::dur;
    let n = |x: &serde_json::Value, k: &str| x.get(k).and_then(serde_json::Value::as_u64);
    let x = n(v, "warp").unwrap_or(1);
    if x > 1 {
        ok(&format!(
            "time warp: the world runs {x}× (showcase.warp); what it keeps stays on your clock"
        ));
    }
    let Some(s) = v.get("story") else {
        hint("the story is off (showcase.story on plays its beats)");
        return;
    };
    let beats: Vec<&str> = s["beats"]
        .as_array()
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let every = n(s, "every").unwrap_or(0);
    let real = n(s, "every_real").unwrap_or(every);
    let yours = if real == every {
        String::new()
    } else {
        format!(" ({} of yours)", dur(real))
    };
    let now = match s.get("now").filter(|x| !x.is_null()) {
        Some(b) => format!(
            " — now: {} ({} left)",
            b["beat"].as_str().unwrap_or(""),
            dur(n(b, "left").unwrap_or(0))
        ),
        None => " — between beats".into(),
    };
    ok(&format!(
        "the story: every {} of the world's time{yours}: {}{now}",
        dur(every),
        beats.join(", ")
    ));
}

/// `aleph showcase universes`: every world, by group, what it holds and
/// whether it's on.
fn universes() -> ExitCode {
    use super::universe::{Cat, ALL};
    let cfg = crate::config::Config::load();
    let on = cfg.get("showcase.universes").unwrap_or_default();
    let on: Vec<String> = on.split(',').map(|x| x.trim().to_lowercase()).collect();
    let all = on.iter().all(|x| x.is_empty()) || on.iter().any(|x| x == "all");
    let theme = cfg.get_or("showcase.theme");
    head(&format!("showcase universes · theme {theme}"));
    for group in ["classic", "space", "games", "films"] {
        println!();
        println!("  {group}");
        for u in ALL.iter().filter(|u| u.group == group) {
            let lit = all || on.iter().any(|x| x == u.id);
            let counts: Vec<String> = Cat::ALL
                .iter()
                .map(|c| (c.id(), u.names(*c).len()))
                .filter(|(_, n)| *n > 0)
                .map(|(c, n)| format!("{n} {c}"))
                .collect();
            println!(
                "  {} {:<12} {}  ({})",
                if lit { "●" } else { "○" },
                u.id,
                u.title,
                counts.join(", ")
            );
        }
    }
    println!();
    hint("aleph settings set showcase.universes trek,valve,scifi — or all; showcase.theme mixed | per-launch | <one>");
    ExitCode::SUCCESS
}

fn status(rest: &[String]) -> ExitCode {
    let st = state::get();
    // demo flow: the mode, and the demo world's clock and story, as JSON
    if rest.iter().any(|a| a == "--json") {
        let mut v = serde_json::json!({"mode": st.mode.name(), "paused": st.panic});
        if st.mode == Mode::Demo {
            v["seed"] = st.seed.into();
            v["world"] = super::demo::flow_json();
        }
        crate::out::json(&v);
        return ExitCode::SUCCESS;
    }
    head(&format!("showcase: {}", st.mode.name()));
    if st.mode == Mode::Demo {
        let w = super::demo::world();
        ok(&format!(
            "the demo world, seed {}: {} machines, {} sessions, {} things done to it",
            st.seed,
            super::demo::world::MACHINES.len(),
            w.sessions.len(),
            super::demo::actions().len()
        ));
        demo_flow_lines(&super::demo::flow_json());
        hint("aleph showcase on --demo --fresh starts it over");
    }
    if st.mode != Mode::Off {
        let d = super::dict::get();
        ok(&format!(
            "{} private words masked (and paths, addresses, emails, versions)",
            d.entries.len()
        ));
        let cfg = crate::config::Config::load();
        if !crate::auto::hub_here(&cfg) {
            let h = cfg.get("hub.machine").unwrap_or_default();
            warn(&format!(
                "the hub runs on {h}: its notifications aren't masked"
            ));
        }
        warn("attaching through herdr's own screen isn't masked: Aleph opens Focus instead (A attaches, after a warning)");
    }
    // safety: the pause, screens, the clipboard, money
    for (fine, l) in super::safety::status_lines(st.mode != Mode::Off) {
        if fine {
            ok(&l);
        } else {
            warn(&l);
        }
    }
    ExitCode::SUCCESS
}
