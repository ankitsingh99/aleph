//! The demo world: deterministic, steered only by its action log, alive on
//! screen, and never reaching anything real.

use super::world::{self, Act, Action, Phase, World};
use super::{screens, state};

const T0: u64 = 1_793_000_000;

fn key(w: &World) -> Vec<(String, String, String, &'static str, u64)> {
    w.sessions
        .iter()
        .map(|s| {
            (
                s.host.clone(),
                s.pane.clone(),
                s.name.clone(),
                s.status(),
                s.at.seq,
            )
        })
        .collect()
}

fn demo_on() {
    state::force(Some(state::State {
        mode: state::Mode::Demo,
        salt: b"0123456789abcdef0123456789abcdef".to_vec(),
        epoch: 1,
        seed: 42,
        ..Default::default()
    }));
}

fn act(t: u64, host: &str, pane: &str, a: Act) -> Action {
    Action {
        t,
        host: host.into(),
        pane: pane.into(),
        act: a,
    }
}

/// The first time from `t` on when `pred` holds for the session.
fn when(seed: u64, host: &str, pane: &str, from: u64, pred: impl Fn(&world::Sess) -> bool) -> u64 {
    (from..from + 4000)
        .find(|&t| World::at(seed, t, &[]).find(host, pane).is_some_and(&pred))
        .expect("it happens within an hour")
}

#[test]
fn the_same_inputs_give_the_same_world() {
    let a = World::at(42, T0, &[]);
    let b = World::at(42, T0, &[]);
    assert_eq!(key(&a), key(&b));
    assert!(a.sessions.len() >= 10);
    // five machines, every agent of the cast and a plain shell
    for agent in ["claude", "codex", "opencode", "gemini", "pi", "shell"] {
        assert!(a.sessions.iter().any(|s| s.agent == agent), "{agent}");
    }
    assert!(a
        .sessions
        .iter()
        .any(|s| s.agent == "shell" && s.label.starts_with("shell_")));
    // another seed: other names and timings
    let c = World::at(7, T0, &[]);
    assert_ne!(key(&a), key(&c));
    // and time moves it on
    assert_ne!(key(&a), key(&World::at(42, T0 + 600, &[])));
}

#[test]
fn two_worlds_with_the_same_log_agree() {
    let dir = std::env::temp_dir().join(format!("aleph-demo-log-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join("agree.jsonl");
    let _ = std::fs::remove_file(&log);
    super::set_log(Some(log.clone()));
    let acts = vec![
        act(T0 + 1, "europa", "w1:p2", Act::Approve),
        act(
            T0 + 2,
            "magrathea",
            "w1:p1",
            Act::Send("now with more fjords".into()),
        ),
        act(
            T0 + 3,
            "europa",
            "w1:p321",
            Act::New {
                tab: "w1:t321".into(),
                agent: "claude".into(),
                name: "drifting-heron".into(),
                label: "claude_drifting-heron:babel-fish".into(),
                dir: "/home/ford/babel-fish".into(),
                prompt: Some("teach it Klingon".into()),
            },
        ),
        act(
            T0 + 4,
            "europa",
            "w1:p321",
            Act::Rename {
                name: Some("swift-heron".into()),
                label: None,
            },
        ),
        act(T0 + 5, "vogsphere", "w1:p1", Act::Close),
    ];
    for a in &acts {
        super::log(a).unwrap();
    }
    // what another process reads back is what was written
    let back = super::actions();
    assert_eq!(*back, acts);
    for t in [T0, T0 + 3, T0 + 60, T0 + 900] {
        assert_eq!(key(&World::at(42, t, &back)), key(&World::at(42, t, &acts)));
    }
    let w = World::at(42, T0 + 10, &back);
    let s = w.find("europa", "w1:p321").expect("the new session");
    assert_eq!(
        (s.name.as_str(), s.prompt.as_str()),
        ("swift-heron", "teach it Klingon")
    );
    assert!(w.find("vogsphere", "w1:p1").is_none());
    super::set_log(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn approving_unblocks_a_session_and_unanswered_ones_answer_themselves() {
    let (host, pane) = ("europa", "w1:p2");
    let t = when(42, host, pane, T0, |s| s.at.phase == Phase::Ask);
    let s = World::at(42, t + 5, &[]);
    assert_eq!(s.find(host, pane).unwrap().status(), "blocked");
    // the question it shows has a numbered yes, so approve finds it
    let q = screens::detection(s.find(host, pane).unwrap());
    assert_eq!(crate::herdr::approve_keys(&q), Some(vec!["1".to_string()]));
    let acts = [act(t + 2, host, pane, Act::Approve)];
    let w = World::at(42, t + 5, &acts);
    assert_eq!(w.find(host, pane).unwrap().status(), "working");
    // the approval was for that question only: the next cycle asks again
    let len = world::cycle_of(42, &format!("{host}|{pane}")).len;
    assert_eq!(
        World::at(42, t + len + 5, &acts)
            .find(host, pane)
            .unwrap()
            .status(),
        "blocked"
    );
    // nobody answers: it moves on by itself after two minutes
    assert_eq!(
        World::at(42, t + world::ASK + 1, &[])
            .find(host, pane)
            .unwrap()
            .status(),
        "working"
    );
}

#[test]
fn new_sessions_start_and_closed_ones_go() {
    let new = act(
        T0,
        "tycho-base",
        "w1:p500",
        Act::New {
            tab: "w1:t500".into(),
            agent: "codex".into(),
            name: "kiting-loom".into(),
            label: "codex_kiting-loom:ring-gate".into(),
            dir: "/home/naomi/ring-gate".into(),
            prompt: None,
        },
    );
    assert!(World::at(42, T0 - 1, std::slice::from_ref(&new))
        .find("tycho-base", "w1:p500")
        .is_none());
    let w = World::at(42, T0 + 1, std::slice::from_ref(&new));
    let s = w.find("tycho-base", "kiting-loom").expect("found by name");
    assert_eq!((s.status(), s.pane.as_str()), ("working", "w1:p500"));
    let close = act(T0 + 30, "tycho-base", "w1:t500", Act::Close);
    let acts = [new, close];
    assert!(World::at(42, T0 + 29, &acts)
        .find("tycho-base", "w1:p500")
        .is_some());
    assert!(World::at(42, T0 + 31, &acts)
        .find("tycho-base", "w1:p500")
        .is_none());
}

#[test]
fn a_prompt_to_an_idle_session_makes_it_work_then_finish() {
    let (host, pane) = ("magrathea", "w1:p1");
    let t = when(42, host, pane, T0, |s| s.at.phase == Phase::Idle);
    let acts = [act(t, host, pane, Act::Send("rerender Norway".into()))];
    let w = World::at(42, t + 10, &acts);
    let s = w.find(host, pane).unwrap();
    assert_eq!(
        (s.status(), s.prompt.as_str()),
        ("working", "rerender Norway")
    );
    let s2 = World::at(42, t + world::PROMPT_WORK + 5, &acts);
    assert_eq!(s2.find(host, pane).unwrap().status(), "done");
}

#[test]
fn statuses_spread_out_over_time() {
    let mut n = std::collections::HashMap::new();
    let mut total = 0;
    for k in 0..2000u64 {
        let w = World::at(42, T0 + k * 37, &[]);
        for s in w.sessions.iter().filter(|s| s.agent != "shell") {
            *n.entry(s.status()).or_insert(0) += 1;
            total += 1;
        }
    }
    let share = |st: &str| n.get(st).copied().unwrap_or(0) as f64 / total as f64;
    assert!(share("blocked") > 0.06 && share("blocked") < 0.3, "{n:?}");
    assert!(share("working") > 0.3, "{n:?}");
    assert!(share("done") > 0.03 && share("idle") > 0.03, "{n:?}");
}

#[test]
fn a_codex_session_hits_its_limit_four_minutes_in_twenty() {
    let t = T0 - T0 % world::LIMIT_EVERY + 60;
    let w = World::at(42, t, &[]);
    let s = w
        .sessions
        .iter()
        .find(|s| s.at.phase == Phase::Limit)
        .expect("one at its limit");
    assert_eq!((s.agent.as_str(), s.status()), ("codex", "idle"));
    let m = world::machine(&s.host).unwrap();
    let text = screens::plain(&screens::lines(s, m, 0, 120).join("\n"));
    assert_eq!(
        crate::limits::detect(&text).as_deref(),
        Some("in 3 minutes")
    );
    // and later it doesn't
    let later = World::at(42, t + world::LIMIT_FOR, &[]);
    assert!(later.sessions.iter().all(|s| s.at.phase != Phase::Limit));
}

#[test]
fn one_machine_drops_out_a_minute_in_ten() {
    let m = world::machine("vogsphere").unwrap();
    let down = (0..600).filter(|k| !m.up(T0 + k)).count();
    assert_eq!(down, 60);
    assert!(world::MACHINES
        .iter()
        .filter(|m| m.alias != "vogsphere")
        .all(|m| (0..600).all(|k| m.up(T0 + k))));
}

#[test]
fn screens_fit_and_every_question_has_a_yes() {
    for k in 0..300u64 {
        let w = World::at(42, T0 + k * 13, &[]);
        for s in &w.sessions {
            let m = world::machine(&s.host).unwrap();
            for cols in [48u16, 80, 140] {
                for l in screens::lines(s, m, k * 250, cols) {
                    let p = screens::plain(&l);
                    assert!(
                        unicode_width::UnicodeWidthStr::width(p.as_str()) <= cols as usize,
                        "{} at {cols}: {p}",
                        s.agent
                    );
                }
            }
            if s.status() == "blocked" {
                let keys = crate::herdr::approve_keys(&screens::detection(s));
                assert_eq!(keys, Some(vec!["1".to_string()]), "{}", s.agent);
            }
        }
    }
}

#[test]
fn stats_and_usage_are_the_same_wherever_they_are_asked() {
    let w = World::at(42, T0, &[]);
    let m = world::machine("magrathea").unwrap();
    let (a, b) = (
        super::stats::at(m, &w, T0 as f64, 2.0),
        super::stats::at(m, &w, T0 as f64, 2.0),
    );
    assert_eq!(a.cpu, b.cpu);
    assert_eq!(a.cpu_hist, b.cpu_hist);
    assert_eq!(a.gpus.len(), 2);
    assert!(a
        .gpus
        .iter()
        .all(|g| g.hist.len() == a.cpu_hist.len() && g.util > 0.0));
    assert_eq!(a.cores.len(), 64);
    assert!(a.samples >= 2 && a.err.is_none());
}

#[test]
fn nothing_real_is_reached_in_the_demo_world() {
    demo_on();
    let dead = super::DEAD;
    assert_eq!(crate::sshcfg::ssh_bin(), dead);
    assert_eq!(crate::herdr::bin(), dead);
    assert!(crate::proc::which(&crate::herdr::bin()).is_none());
    // no ssh, and not even this computer's own shell
    for h in ["magrathea", crate::sshcfg::LOCAL] {
        let r = crate::sshcfg::shell(h, "true").status();
        assert!(r.is_err(), "{h}: {r:?}");
    }
    // herdr calls the world doesn't answer are refused before anything runs
    let t = crate::config::Target {
        ssh: "magrathea".into(),
        machine: "magrathea".into(),
        source: "test",
    };
    let h = crate::herdr::Herdr::new(&t);
    assert_eq!(h.explain("w1:p1").err().as_deref(), Some(super::NOT_HERE));
    // the ones it does answer come from the world
    let v = h.sessions().unwrap();
    assert!(v.iter().all(|s| s.host == "magrathea") && v.len() == 3);
    // a machine that isn't the world's doesn't answer
    let other = crate::herdr::Herdr::new(&crate::config::Target {
        ssh: "b31".into(),
        ..t.clone()
    });
    assert!(other.sessions().is_err());
    // the machines are the world's, and they can't be changed
    let cfg = crate::config::Config::in_memory(&["hosts=b31,gpu2", "ssh_host=b31"]);
    assert_eq!(cfg.hosts(), super::aliases());
    assert!(!cfg.fleet().iter().any(|h| h == crate::sshcfg::LOCAL));
    assert!(super::refuse_setting("hosts").is_some() && super::refuse_setting("theme").is_none());
    // what it writes goes to its sandbox
    assert!(crate::config::world_state().ends_with("showcase"));
    assert_eq!(
        crate::config::state_dir().join("showcase"),
        crate::config::world_state()
    );
    state::force(None);
    assert_ne!(crate::sshcfg::ssh_bin(), dead);
    assert!(!crate::config::world_state().ends_with("showcase"));
}

#[test]
fn the_folder_browser_walks_a_made_up_home() {
    use crate::remote::{Ev, Msg, Op, Req};
    let (tx, rx) = std::sync::mpsc::channel();
    let (etx, erx) = std::sync::mpsc::channel();
    super::browse::serve("europa".into(), rx, etx);
    let next = || erx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let Ev::Online { home } = next() else {
        panic!("not online")
    };
    assert_eq!(home, b"/home/ford");
    let ask = |id, op, p: &[u8]| {
        tx.send(Msg::Req(Req {
            id,
            op,
            path: p.to_vec(),
        }))
        .unwrap()
    };
    ask(1, Op::List, b"/home/ford");
    let Ev::List { res: Ok(l), .. } = next() else {
        panic!("no listing")
    };
    let names: Vec<String> = l
        .dirs
        .iter()
        .map(|d| String::from_utf8_lossy(&d.name).into_owned())
        .collect();
    assert!(names.contains(&"babel-fish".into()) && names.contains(&"subspace-relay".into()));
    assert!(l.dirs.iter().any(|d| d.git));
    ask(2, Op::Info, b"/home/ford/babel-fish");
    let Ev::Info { res: Ok(i), .. } = next() else {
        panic!("no info")
    };
    assert_eq!(i.branch.as_deref(), Some("main"));
    assert!(i.markers.iter().any(|m| m == "Cargo.toml"));
    ask(3, Op::Stat, b"/home/ford/nowhere");
    assert!(matches!(next(), Ev::Stat { res: Err(_), .. }));
    ask(4, Op::Find, b"2\t\t/home/ford/babel-fish");
    let Ev::Find {
        res: Ok((l, false)),
        ..
    } = next()
    else {
        panic!("no index")
    };
    assert!(l.dirs.iter().any(|d| d.name == b"src/core"));
    assert!(!l.dirs.iter().any(|d| d.name.starts_with(b".")));
    tx.send(Msg::Shutdown).unwrap();
    assert!(matches!(next(), Ev::Closed));
}

// ------------------------------------------------------------------ demo flow

use super::clock::{self, Anchor};
use super::story::{self, Story};

/// A cycle's start (the story's cycles count from its origin).
fn cycle_start(s: &Story, k: u64) -> u64 {
    story::ORIGIN + s.every() * k
}

/// An even cycle near T0 (the soak test the world's rule answers).
fn even_cycle(s: &Story) -> u64 {
    (T0 - story::ORIGIN) / s.every() / 2 * 2
}

/// Run on the world's clock reading `world` (seconds) now, at `x`.
fn clock_at(world: u64, x: u64) {
    clock::force(Some(Anchor {
        real_ms: clock::real_ms(),
        world_ms: world * 1000,
        x,
    }));
}

#[test]
fn the_story_plays_its_beats_in_the_world() {
    let s = Story::default();
    let k = even_cycle(&s);
    let c = cycle_start(&s, k);
    let at = |t: u64| World::script(42, t, &[], Some(s));
    // ask: europa's Claude stops at its question
    let w = at(c + s.ask_at());
    let a = w.find("europa", "w1:p2").unwrap();
    assert_eq!(a.status(), "blocked");
    assert_eq!(a.task.title, story::ask_task(k).title);
    let q = screens::detection(a);
    assert!(q.contains("relay-soak"), "{q}");
    assert_eq!(crate::herdr::approve_keys(&q), Some(vec!["1".to_string()]));
    // the next cycle asks for a rollout instead
    let w2 = at(cycle_start(&s, k + 1) + s.ask_at());
    let b = w2.find("europa", "w1:p2").unwrap();
    assert!(screens::detection(b).contains("relays.sh"));
    // drop: vogsphere's herdr stops (its link stays up), then comes back
    let (d0, d1) = s.drop_window();
    let w = at(c + d0 + 1);
    assert!(w.link_up("vogsphere") && !w.herdr_up("vogsphere"));
    assert!(w.herdr_up("europa"));
    assert!(at(c + d1).herdr_up("vogsphere"));
    // healed: back RESTART seconds after the hub restarted it
    let heal = [act(c + d0 + 2, "vogsphere", "", Act::Heal)];
    let healed = |t| World::script(42, t, &heal, Some(s));
    assert!(!healed(c + d0 + 2 + story::RESTART - 1).herdr_up("vogsphere"));
    assert!(healed(c + d0 + 2 + story::RESTART).herdr_up("vogsphere"));
    // and the old outage (a minute in ten) is the story's drop now
    assert!((0..600).all(|i| at(T0 + i).link_up("vogsphere")));
    // limit: magrathea's Codex stops at its plan's limit, and it resets
    let (l0, l1) = s.limit_window();
    let w = at(c + l0 + 1);
    let lim = w.find("magrathea", "w1:p2").unwrap();
    assert_eq!(lim.at.phase, Phase::Limit);
    assert_eq!(lim.limit_left, l1 - l0 - 1);
    let m = world::machine("magrathea").unwrap();
    let text = screens::plain(&screens::lines(lim, m, 0, 120).join("\n"));
    assert!(crate::limits::detect(&text).is_some(), "{text}");
    let after = at(c + l1);
    assert_ne!(
        after.find("magrathea", "w1:p2").unwrap().at.phase,
        Phase::Limit
    );
    // finish: magrathea's Claude finishes the big task
    let f = s.finish_at();
    let (before, done) = (at(c + f - 1), at(c + f));
    let (x, y) = (
        before.find("magrathea", "w1:p1").unwrap(),
        done.find("magrathea", "w1:p1").unwrap(),
    );
    assert_eq!((x.status(), y.status()), ("working", "done"));
    assert_eq!(y.task.title, story::BIG.title);
    assert!(y.at.done_seq > x.at.done_seq);
}

#[test]
fn a_beat_left_out_leaves_the_world_as_it_was() {
    let only = Story {
        beats: story::ASK,
        ..Story::default()
    };
    // no drop: vogsphere's link drops a minute in ten again
    let down = (0..600)
        .filter(|i| !World::script(42, T0 + i, &[], Some(only)).link_up("vogsphere"))
        .count();
    assert_eq!(down, 60);
    // no limit: Codex's own, four minutes in twenty
    let t = T0 - T0 % world::LIMIT_EVERY + 60;
    let w = World::script(42, t, &[], Some(only));
    assert_eq!(w.find("magrathea", "w1:p2").unwrap().at.phase, Phase::Limit);
    // no finish: magrathea's Claude on its own task
    let own = w.find("magrathea", "w1:p1").unwrap();
    assert_ne!(own.task.title, story::BIG.title);
    // and no story at all is the world as it always was
    assert_eq!(
        key(&World::script(42, T0, &[], None)),
        key(&World::at(42, T0, &[]))
    );
}

#[test]
fn the_hub_answers_the_soak_test_and_heals_the_drop() {
    demo_on();
    let s = Story::default();
    story::force(Some(Some(s)));
    let dir = std::env::temp_dir().join(format!("aleph-demo-story-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join("story.jsonl");
    let _ = std::fs::remove_file(&log);
    super::set_log(Some(log.clone()));
    let cfg = crate::config::Config::in_memory(&["auto_approve=false"]);
    // the story's settings: its rule, its self-heal (and they can't change)
    assert!(cfg.flag("auto_approve") && cfg.flag("auto_heal"));
    assert!(super::refuse_setting("auto_approve").is_some());
    let rules = crate::auto::rules(&cfg);
    assert!(rules.iter().any(|(n, _)| n == story::RULE.0));
    assert!(crate::showcase::world_heals());
    let k = even_cycle(&s);
    let c = cycle_start(&s, k);
    // the soak test: the world's rule says yes; the rollout: no rule
    clock_at(c + s.ask_at() + 1, 1);
    let soak = super::read("europa", "w1:p2", "detection", 0, false).unwrap();
    assert_eq!(
        crate::auto::decide(&cfg, &soak).as_deref(),
        Ok(story::RULE.0)
    );
    clock_at(cycle_start(&s, k + 1) + s.ask_at() + 1, 1);
    let roll = super::read("europa", "w1:p2", "detection", 0, false).unwrap();
    assert!(crate::auto::decide(&cfg, &roll).is_err());
    // the drop: herdr isn't running (what the fleet and the hub read)…
    let (d0, _) = s.drop_window();
    clock_at(c + d0 + 1, 1);
    let e = super::sessions("vogsphere", &crate::agents::Filter::All).unwrap_err();
    assert!(crate::fleet::not_running(&e), "{e}");
    // …the hub's heal restarts it, and every process sees it come back
    let said = crate::auto::heal_now("vogsphere").unwrap();
    assert!(said.contains("restarted"), "{said}");
    assert!(super::actions().iter().any(|a| a.act == Act::Heal));
    clock_at(c + d0 + 1 + story::RESTART, 1);
    assert!(super::sessions("vogsphere", &crate::agents::Filter::All).is_ok());
    assert!(crate::auto::heal_now("vogsphere").is_err());
    // each cycle's limit is news (half a cycle, at the world's speed)
    clock_at(c, 5);
    assert_eq!(super::told_for(6 * 3600), 12);
    story::force(None);
    clock::force(None);
    super::set_log(None);
    state::force(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_world_runs_on_its_own_clock() {
    demo_on();
    // ten times as fast, from wherever the anchor put it
    let t = T0 + 10_000;
    clock_at(t, 10);
    let now = super::clock();
    assert!((t..t + 10).contains(&now), "{now}");
    let w = super::world();
    let here = World::script(42, super::clock(), &super::actions(), None);
    assert_eq!(key(&w), key(&here));
    // what it keeps stays on real time
    assert!(super::now().abs_diff(crate::cache::now()) <= 1);
    // a limit's reset is a real time: 4 world minutes are 24 real seconds
    clock_at(T0 - T0 % world::LIMIT_EVERY + 1, 10);
    let l = super::usage::codex("magrathea").unwrap();
    let left = l.windows[0]
        .resets_at
        .unwrap()
        .saturating_sub(crate::cache::now());
    assert!((20..=26).contains(&left), "{left}");
    clock::force(None);
    state::force(None);
}
