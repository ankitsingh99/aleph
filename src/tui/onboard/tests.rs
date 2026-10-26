//! The wizard in a TestBackend: every step at 80×24, 120×32 and 200×50
//! (and a 20×5 that must not break), resume from setup.json, `--redo`,
//! skip / back / retry, the review's diff.

use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn state(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aleph-onboard-{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.join("setup.json")
}

fn wiz(lines: &[&str], opts: Opts, name: &str) -> Wiz {
    let cfg = Config::in_memory(lines);
    let mut w = Wiz::new(cfg, opts, state(name), true);
    // what the probes would have said: two machines, codex older on gpu2
    w.probes.insert(
        "b31".into(),
        crate::probe::parse(
            "b31",
            "CCA\tclaude\t/u/claude\t2.1.200\nCCA\tcodex\t/u/codex\t0.46.0\nCCH\t/u/herdr\therdr 0.9.5\nCCI\tclaude: current (v11) (/x)\nCCI\tcodex: not installed (/x)\nCCO\tLinux x86_64\nCCR\tUbuntu 24.04 LTS\n",
        ),
    );
    w.probes.insert(
        "gpu2".into(),
        crate::probe::parse(
            "gpu2",
            "CCA\tcodex\t/u/codex\t0.40.1\nCCH\t/u/herdr\therdr 0.8.0\nCCI\tcodex: not installed (/x)\nCCO\tLinux x86_64\n",
        ),
    );
    w.local_herdr = Some(Some("0.9.5".into()));
    w.saved = Some(vec!["b31".into()]);
    w.mach.found = true;
    w.mach.cands = vec![
        machines::Cand {
            alias: "b31".into(),
            hostname: "10.0.0.49".into(),
            addr: "selene@10.0.0.49".into(),
            port: None,
            from: vec!["ssh config"],
            note: String::new(),
            new: false,
        },
        machines::Cand {
            alias: "dex42".into(),
            hostname: "dex42.tail.ts.net".into(),
            addr: "dex42.tail.ts.net".into(),
            port: None,
            from: vec!["tailscale"],
            note: "linux · online".into(),
            new: true,
        },
    ];
    w.mach.login.insert(
        "b31".into(),
        machines::Login::NewKey {
            fps: vec!["ED25519  SHA256:abcdef".into()],
            lines: vec!["b31 ssh-ed25519 AAAA".into()],
        },
    );
    w.ac.facts = Some(access::Facts::default());
    w.ig.facts = Some(integrations::Facts {
        shell: "zsh".into(),
        rc: Some(PathBuf::from("/h/.zshrc")),
        herdr: true,
        ..Default::default()
    });
    w.nt.moshi = Some(None);
    w.an.seen = Some(vec![]);
    w
}

fn render(w: &mut Wiz, cols: u16, rows: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    t.draw(|f| w.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    (0..rows)
        .map(|y| {
            (0..cols)
                .map(|x| b[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(w: &mut Wiz, c: KeyCode) -> Option<Exit> {
    w.key(KeyEvent::new(c, KeyModifiers::NONE))
}

fn base() -> Vec<&'static str> {
    vec!["hosts=b31,gpu2", "ssh_host=b31", "local=false"]
}

#[test]
fn every_step_renders_at_every_size() {
    let mut w = wiz(&base(), Opts::default(), "sizes");
    for s in Step::ALL {
        w.cur = s;
        for (c, r) in [(80, 24), (120, 32), (200, 50), (20, 5), (50, 12)] {
            let t = render(&mut w, c, r);
            if c >= 80 {
                assert!(t.contains("aleph setup"), "{s:?} {c}×{r}\n{t}");
                assert!(t.contains(s.title()), "{s:?} {c}×{r} has no title\n{t}");
            }
        }
    }
}

fn fresh(name: &str) -> PathBuf {
    let p = state(name);
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn a_stopped_run_picks_up_where_it_was() {
    let p = fresh("resume");
    let mut w = Wiz::new(Config::in_memory(&base()), Opts::default(), p.clone(), true);
    w.go(Step::Look);
    w.set("theme", "dark");
    w.plan.want(Step::Integrations, Action::Completions, true);
    w.go(Step::Notify);
    drop(w);
    let w = Wiz::new(Config::in_memory(&base()), Opts::default(), p.clone(), true);
    assert_eq!(w.cur, Step::Notify);
    assert_eq!(w.value("theme"), "dark");
    assert!(w.plan.has(&Action::Completions));
    // one step: there, with the choices kept
    let only = Opts {
        only: Some(Step::Agents),
        ..Default::default()
    };
    let w = Wiz::new(Config::in_memory(&base()), only, p.clone(), true);
    assert_eq!(w.cur, Step::Agents);
    assert!(w.pending("theme"));
    // --redo: from the settings as they are, at the start
    let redo = Opts {
        redo: true,
        ..Default::default()
    };
    let w = Wiz::new(Config::in_memory(&base()), redo, p, true);
    assert_eq!(w.cur, Step::Preflight);
    assert!(w.plan.is_empty());
}

#[test]
fn redo_shows_every_current_value() {
    let mut lines = base();
    lines.extend([
        "theme=light",
        "accent=claude",
        "notify.quiet=22:00-07:00",
        "remote.allow=b31:*",
    ]);
    let redo = Opts {
        redo: true,
        ..Default::default()
    };
    let mut w = wiz(&lines, redo, "redo");
    let cur = |w: &Wiz, s: Step, key: &str| {
        w.rows_of(s).into_iter().find_map(|r| match r.kind {
            rows::Kind::Choice { key: k, cur, .. } | rows::Kind::Input { key: k, cur, .. }
                if k == key =>
            {
                Some(cur)
            }
            _ => None,
        })
    };
    assert_eq!(cur(&w, Step::Look, "theme").as_deref(), Some("light"));
    assert_eq!(cur(&w, Step::Look, "accent").as_deref(), Some("claude"));
    assert_eq!(
        cur(&w, Step::Notify, "notify.quiet").as_deref(),
        Some("22:00-07:00")
    );
    assert_eq!(cur(&w, Step::Machines, "ssh_host").as_deref(), Some("b31"));
    // the fleet's machines are picked; the matrix shows what's allowed
    let rs = w.rows_of(Step::Machines);
    assert!(rs.iter().any(|r| r.kind
        == rows::Kind::Pick {
            id: "m:b31".into(),
            on: true
        }));
    assert!(rs.iter().any(|r| r.kind
        == rows::Kind::Pick {
            id: "m:dex42".into(),
            on: false
        }));
    let m = w
        .rows_of(Step::Notify)
        .into_iter()
        .find_map(|r| match r.kind {
            rows::Kind::Matrix { id, on, .. } if id == "remote:b31" => Some(on),
            _ => None,
        });
    assert_eq!(m, Some(vec![true, true]));
    w.cur = Step::Look;
    let t = render(&mut w, 120, 32);
    // every theme to pick from: the row steps through them (‹ light ›)
    assert!(t.contains("[light]") || t.contains("‹ light ›"), "{t}");
}

fn preflight_check() -> preflight::Check {
    preflight::Check {
        group: "Tools",
        id: "git".into(),
        label: "git".into(),
        st: St::Ok,
        text: "2.4".into(),
        fix: None,
        why: String::new(),
        load: Vec::new(),
    }
}

#[test]
fn next_skip_back_and_retry() {
    let mut w = wiz(&base(), Opts::default(), "moves");
    w.state_path = fresh("moves");
    w.go(Step::Look);
    // ←→ change a choice: a change in the plan, nothing written
    key(&mut w, KeyCode::Down);
    key(&mut w, KeyCode::Right);
    assert!(w.pending("accent"), "{:?}", w.plan.sets);
    // enter on a setting changes it again, and stays on the step
    let accent = w.value("accent");
    key(&mut w, KeyCode::Enter);
    assert_ne!(w.value("accent"), accent);
    assert_eq!(w.cur, Step::Look);
    // Continue: done, the next step
    key(&mut w, KeyCode::End);
    key(&mut w, KeyCode::Enter);
    assert_eq!(w.cur, Step::Machines);
    assert_eq!(w.marks[Step::Look.index()], Mark::Done);
    // esc: back
    key(&mut w, KeyCode::Esc);
    assert_eq!(w.cur, Step::Look);
    // s: skipped, and what it chose goes
    key(&mut w, KeyCode::Char('s'));
    assert_eq!(w.cur, Step::Machines);
    assert_eq!(w.marks[Step::Look.index()], Mark::Skipped);
    assert!(!w.pending("accent"));
    // r: looks again (no threads here, but the step starts over)
    w.go(Step::Preflight);
    w.pre.checks.push(preflight_check());
    key(&mut w, KeyCode::Char('r'));
    assert!(w.pre.checks.is_empty() && w.pre.started);
    // esc on the first step asks before leaving
    key(&mut w, KeyCode::Esc);
    assert!(matches!(w.mode, Overlay::Confirm { yes: Yes::Quit, .. }));
    assert!(matches!(key(&mut w, KeyCode::Char('y')), Some(Exit::Quit)));
}

/// The row under the cursor, as what it is (empty: none).
fn on(w: &Wiz) -> String {
    w.selected().map(|r| r.ident()).unwrap_or_default()
}

/// Put the cursor on the row that is `id`, as a key would.
fn select(w: &mut Wiz, id: &str) {
    let rs = w.sync_sel();
    let j = rs
        .iter()
        .position(|r| r.ident() == id)
        .unwrap_or_else(|| panic!("no row {id}"));
    w.put_sel(&rs, j);
}

fn first_stop(w: &Wiz, s: Step) -> String {
    w.rows_of(s)
        .iter()
        .find(|r| r.selectable())
        .map(Row::ident)
        .unwrap_or_default()
}

const CONTINUE: &str = "id:wiz:next";

#[test]
fn arrows_run_across_steps_without_marking_them() {
    let mut w = wiz(&base(), Opts::default(), "across");
    w.state_path = fresh("across");
    w.go(Step::Look);
    key(&mut w, KeyCode::End);
    assert_eq!(on(&w), CONTINUE);
    // ↓ past the last row: the next step opens on its first row, nothing marked
    key(&mut w, KeyCode::Down);
    assert_eq!(w.cur, Step::Machines);
    assert_eq!(on(&w), first_stop(&w, Step::Machines));
    assert_eq!(w.marks[Step::Look.index()], Mark::Todo);
    assert_eq!(w.marks[Step::Machines.index()], Mark::Todo);
    let t = render(&mut w, 120, 32);
    assert!(t.contains("┃●  3 Machines"), "the rail follows\n{t}");
    // ↑ on the first row: back, on the previous step's last row
    key(&mut w, KeyCode::Char('k'));
    assert_eq!(w.cur, Step::Look);
    assert_eq!(on(&w), CONTINUE);
    assert_eq!(w.marks[Step::Look.index()], Mark::Todo);
    key(&mut w, KeyCode::Char('j'));
    assert_eq!(w.cur, Step::Machines);
    // at either end of setup it stays
    w.go(Step::Preflight);
    key(&mut w, KeyCode::Home);
    key(&mut w, KeyCode::Up);
    assert_eq!(w.cur, Step::Preflight);
    w.go(Step::Review);
    key(&mut w, KeyCode::End);
    let last = on(&w);
    key(&mut w, KeyCode::Down);
    assert_eq!((w.cur, on(&w)), (Step::Review, last));
    assert!(w.marks.iter().all(|m| *m == Mark::Todo), "{:?}", w.marks);
}

#[test]
fn enter_changes_a_value_and_continue_moves_on() {
    let mut w = wiz(&base(), Opts::default(), "enter");
    w.state_path = fresh("enter");
    w.go(Step::Look);
    // a toggle flips, and the cursor stays on it
    select(&mut w, "set:mouse");
    let was = w.flag("mouse");
    key(&mut w, KeyCode::Enter);
    assert_eq!(w.flag("mouse"), !was);
    assert!(w.pending("mouse"));
    assert_eq!((w.cur, on(&w)), (Step::Look, "set:mouse".to_string()));
    let t = render(&mut w, 120, 32);
    assert!(t.contains("↑↓  move") && t.contains("⏎  toggle"), "{t}");
    assert!(t.contains("⇥  next step"), "{t}");
    // a choice takes its next value
    select(&mut w, "set:theme");
    let theme = w.value("theme");
    key(&mut w, KeyCode::Enter);
    assert_ne!(w.value("theme"), theme);
    assert_eq!(w.cur, Step::Look);
    // each step ends in Continue, named for the next step
    key(&mut w, KeyCode::End);
    let t = render(&mut w, 120, 32);
    assert!(t.contains("Continue → Machines"), "{t}");
    assert!(t.contains("⏎  continue"), "{t}");
    key(&mut w, KeyCode::Enter);
    assert_eq!(w.cur, Step::Machines);
    assert_eq!(w.marks[Step::Look.index()], Mark::Done);
    // the last step before Review goes there; Review has its own buttons
    let rs = w.rows_of(Step::Presets);
    assert_eq!(
        rs.last().map(|r| r.label.as_str()),
        Some("Continue → Review & apply")
    );
    assert!(!w
        .rows_of(Step::Review)
        .iter()
        .any(|r| r.ident() == CONTINUE));
    // one step (aleph setup look): Continue goes to Review
    let only = Opts {
        only: Some(Step::Look),
        ..Default::default()
    };
    let w = wiz(&base(), only, "enter-only");
    assert_eq!(
        w.rows_of(Step::Look).last().map(|r| r.label.clone()),
        Some("Continue → Review & apply".to_string())
    );
}

#[test]
fn home_end_and_half_pages() {
    let mut w = wiz(&base(), Opts::default(), "pages");
    w.go(Step::Look);
    render(&mut w, 80, 24);
    let i = Step::Look.index();
    let half = (w.view.0 / 2).max(1);
    assert!(w.view.0 >= 4, "{:?}", w.view);
    key(&mut w, KeyCode::Home);
    assert_eq!(on(&w), "set:theme");
    key(&mut w, KeyCode::End);
    assert_eq!(on(&w), CONTINUE);
    key(&mut w, KeyCode::Char('g'));
    assert_eq!(on(&w), "set:theme");
    // page down: the farthest stop within half the lines shown (one line a row here)
    let rs = w.rows_of(Step::Look);
    let from = w.sel[i];
    let want = (from + 1..=from + half)
        .rev()
        .find(|&j| rs[j].selectable())
        .unwrap();
    key(&mut w, KeyCode::PageDown);
    assert_eq!(w.sel[i], want);
    key(&mut w, KeyCode::PageUp);
    assert_eq!(on(&w), "set:theme");
    key(&mut w, KeyCode::Char('G'));
    assert_eq!(on(&w), CONTINUE);
    // the end in view, the top out of it
    let t = render(&mut w, 80, 24);
    assert!(t.contains("Continue → Machines") && t.contains("↑ "), "{t}");
}

#[test]
fn checks_with_nothing_to_do_are_passed_by() {
    let mut w = wiz(&base(), Opts::default(), "readonly");
    w.go(Step::Agents);
    let rs = w.rows_of(Step::Agents);
    assert!(rs
        .iter()
        .any(|r| r.ident() == "id:agents:b31" && !r.selectable()));
    key(&mut w, KeyCode::Home);
    loop {
        let r = w.selected().expect("on a row");
        assert!(
            !matches!(r.kind, Kind::Check { fix: None, .. }),
            "stopped on {}",
            r.ident()
        );
        if r.ident() == CONTINUE {
            break;
        }
        key(&mut w, KeyCode::Down);
        assert_eq!(w.cur, Step::Agents);
    }
    // a check with a fix is still a stop
    w.go(Step::Preflight);
    w.pre.checks.push(preflight_check());
    let mut jq = preflight_check();
    jq.id = "jq".into();
    jq.label = "jq".into();
    jq.st = St::Warn;
    jq.fix = Some(preflight::Fix::Copy("brew install jq".into()));
    w.pre.checks.push(jq);
    key(&mut w, KeyCode::Home);
    // the first row is quick start; the first check to act on is next
    assert_eq!(on(&w), "id:quick");
    key(&mut w, KeyCode::Down);
    assert_eq!(on(&w), "id:jq");
}

#[test]
fn the_heading_over_the_cursor_stays_in_view() {
    let mut w = wiz(&base(), Opts::default(), "heading");
    w.go(Step::Look);
    render(&mut w, 80, 24);
    key(&mut w, KeyCode::End);
    render(&mut w, 80, 24);
    // up to the first row under "Logos", from below
    while on(&w) != "set:icons" {
        key(&mut w, KeyCode::Up);
        render(&mut w, 80, 24);
    }
    let t = render(&mut w, 80, 24);
    let line = |s: &str| t.lines().position(|l| l.contains(s));
    let (more, head, row) = (line("↑ "), line("Logos ─"), line("┃ icons"));
    assert!(more.is_some(), "scrolled\n{t}");
    assert!(
        more < head && head < row && row.is_some(),
        "the more line, the heading, then the cursor\n{t}"
    );
}

#[test]
fn the_wheel_scrolls_the_view_and_brings_the_cursor_along() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let mut w = wiz(&base(), Opts::default(), "wheel");
    w.go(Step::Look);
    key(&mut w, KeyCode::Home);
    render(&mut w, 80, 24);
    let i = Step::Look.index();
    let wheel = |w: &mut Wiz, kind| {
        w.mouse(MouseEvent {
            kind,
            column: 40,
            row: 10,
            modifiers: KeyModifiers::NONE,
        })
    };
    wheel(&mut w, MouseEventKind::ScrollDown);
    wheel(&mut w, MouseEventKind::ScrollDown);
    let top = w.top[i];
    assert!(top > 0, "the view moved");
    // the cursor was scrolled out of sight: it's on the first row in view now
    let t = render(&mut w, 80, 24);
    assert_eq!(w.top[i], top, "the view stays where the wheel left it\n{t}");
    assert_ne!(on(&w), "set:theme");
    let sel = w.selected().unwrap();
    assert!(t.contains(&format!("┃ {}", sel.label)), "{t}");
    // a key brings the view back to the cursor's way of working
    key(&mut w, KeyCode::Home);
    render(&mut w, 80, 24);
    assert_eq!(w.top[i], 0);
}

#[test]
fn the_cursor_stays_on_its_row_when_rows_arrive() {
    let mut w = wiz(&base(), Opts::default(), "follow");
    w.go(Step::Machines);
    select(&mut w, "id:m:dex42");
    render(&mut w, 120, 32);
    let i = Step::Machines.index();
    let was = w.sel[i];
    // discovery answers with one more machine, sorted in above it
    let mut c = w.mach.cands[0].clone();
    c.alias = "box7".into();
    c.new = false;
    w.mach.cands.insert(0, c);
    let t = render(&mut w, 120, 32);
    assert_eq!(on(&w), "id:m:dex42", "{t}");
    assert_eq!(w.sel[i], was + 1);
    // its row goes: the nearest one instead
    w.mach.cands.retain(|c| c.alias != "dex42");
    render(&mut w, 120, 32);
    assert!(w.selected().is_some());
    assert!(
        w.sel[i].abs_diff(was + 1) <= 1,
        "{} vs {}",
        w.sel[i],
        was + 1
    );
}

#[test]
fn an_untouched_cursor_waits_on_the_first_row_as_rows_arrive() {
    let mut w = wiz(&base(), Opts::default(), "untouched");
    w.go(Step::Preflight);
    // nothing to do yet: only Continue
    assert_eq!(on(&w), CONTINUE);
    let fixable = |id: &str, group: &'static str| {
        let mut c = preflight_check();
        (c.id, c.label, c.group, c.st) = (id.into(), id.into(), group, St::Warn);
        c.fix = Some(preflight::Fix::Copy(format!("brew install {id}")));
        c
    };
    w.pre.checks.push(fixable("jq", "Tools"));
    render(&mut w, 120, 32);
    assert_eq!(on(&w), "id:jq", "not moved yet: on the first row");
    // moved (or used) once, it stays on its row when one arrives above it
    key(&mut w, KeyCode::Down);
    key(&mut w, KeyCode::Up);
    w.pre.checks.push(fixable("mosh", "This Mac"));
    render(&mut w, 120, 32);
    assert_eq!(on(&w), "id:jq");
}

#[test]
fn a_matrix_keeps_its_column_between_its_rows() {
    let mut w = wiz(&base(), Opts::default(), "matrix");
    w.go(Step::Notify);
    select(&mut w, "id:remote:b31");
    key(&mut w, KeyCode::Right);
    assert_eq!(w.cell, 1);
    key(&mut w, KeyCode::Down);
    assert_eq!((on(&w), w.cell), ("id:remote:gpu2".to_string(), 1));
    key(&mut w, KeyCode::Up);
    assert_eq!((on(&w), w.cell), ("id:remote:b31".to_string(), 1));
    // anything else starts at the first cell again
    key(&mut w, KeyCode::Down);
    key(&mut w, KeyCode::Down);
    assert_eq!((on(&w), w.cell), (CONTINUE.to_string(), 0));
    key(&mut w, KeyCode::Up);
    assert_eq!((on(&w), w.cell), ("id:remote:gpu2".to_string(), 0));
}

#[test]
fn the_view_never_hides_a_line_under_a_more_marker() {
    for n in 0..30 {
        for h in 0..12 {
            for top in 0..32 {
                let (a, b, up, down) = window(n, h, top);
                let shown = (b - a) + usize::from(up) + usize::from(down);
                assert!(shown <= h, "{n} {h} {top}");
                assert_eq!(up, a > 0 && h >= 4, "{n} {h} {top}");
                assert_eq!(down, b < n && h >= 4, "{n} {h} {top}");
                if h >= 4 && n > 0 {
                    assert!(b > a, "something shows: {n} {h} {top}");
                }
            }
            // every line can be followed into view
            for at in 0..n {
                for top in [0, n / 2, usize::MAX] {
                    let t = follow(n, h.max(1), top, at, at);
                    let (a, b, ..) = window(n, h.max(1), t);
                    assert!((a..b).contains(&at), "{n} {h} {top} {at}: {a}..{b}");
                }
            }
        }
    }
}

#[test]
fn text_is_checked_before_it_goes_in_the_plan() {
    let mut w = wiz(&base(), Opts::default(), "edit");
    w.go(Step::Notify);
    w.select_label("quiet hours");
    key(&mut w, KeyCode::Enter);
    assert!(matches!(w.mode, Overlay::Edit { .. }));
    for c in "late".chars() {
        key(&mut w, KeyCode::Char(c));
    }
    key(&mut w, KeyCode::Enter);
    assert!(
        matches!(&w.mode, Overlay::Edit { err: Some(_), .. }),
        "a bad value stays in the editor"
    );
    assert!(!w.pending("notify.quiet"));
    w.mode = Overlay::None;
    w.select_label("quiet hours");
    key(&mut w, KeyCode::Enter);
    for c in "23:00-08:00".chars() {
        key(&mut w, KeyCode::Char(c));
    }
    key(&mut w, KeyCode::Enter);
    assert_eq!(w.value("notify.quiet"), "23:00-08:00");
    let t = render(&mut w, 120, 32);
    assert!(t.contains("23:00-08:00"), "{t}");
}

#[test]
fn apply_carries_on_past_a_failure_and_keeps_it() {
    let mut w = wiz(&base(), Opts::default(), "apply");
    w.state_path = fresh("apply");
    w.cur = Step::Look;
    w.set("theme", "dark");
    let a = Action::Integration {
        host: "gpu2".into(),
        name: "codex".into(),
    };
    w.plan.want(Step::Agents, a.clone(), true);
    w.plan.want(Step::Integrations, Action::Completions, true);
    w.go(Step::Review);
    let t = render(&mut w, 120, 32);
    assert!(
        t.contains("apply 3 changes") && t.contains("on gpu2"),
        "{t}"
    );
    // the diff: what the settings file will say (a key=value one here)
    assert!(t.contains("+ theme=dark"), "{t}");
    key(&mut w, KeyCode::Enter);
    assert!(matches!(
        w.mode,
        Overlay::Confirm {
            yes: Yes::Apply,
            ..
        }
    ));
    key(&mut w, KeyCode::Char('y'));
    assert_eq!(w.rv.items.len(), 3);
    // settings first, then the quiet actions; threads don't run here, so
    // their answers are handed in
    assert!(review::pump(&mut w).is_none());
    review::applied(&mut w, 0, Ok("1 setting".into()));
    review::pump(&mut w);
    review::applied(&mut w, 1, Err("gpu2: boom".into()));
    review::pump(&mut w);
    review::applied(&mut w, 2, Ok("zsh".into()));
    review::pump(&mut w);
    assert!(w.rv.done);
    assert!(!w.finished);
    assert!(w.plan.sets.is_empty(), "written settings leave the plan");
    assert!(w.plan.has(&a), "a failure stays for next time");
    assert!(!w.plan.has(&Action::Completions));
    assert_eq!(w.marks[Step::Agents.index()], Mark::Warn);
    let t = render(&mut w, 120, 32);
    assert!(t.contains("gpu2: boom") && t.contains("1 failed"), "{t}");
}

#[test]
fn has_sub_reads_the_command_table() {
    assert!(has_sub("setup", "--apply"));
    assert!(!has_sub("setup", "frobnicate"));
    assert!(has_sub("daemon", "status"));
}

/// A bare machine: herdr, curl and bash, no Node.js, no agents.
const BARE: &str = "CCT\tcurl\t/usr/bin/curl\nCCT\tbash\t/bin/bash\nCCH\t/u/herdr\therdr 0.9.5\nCCO\tLinux x86_64\nCCW\t/home/t\n";

#[test]
fn a_bare_machine_ticks_codex_too_with_nodejs_first() {
    let lines = ["hosts=box3,box4", "ssh_host=box3", "local=false"];
    let mut w = wiz(&lines, Opts::default(), "nodefirst");
    w.state_path = fresh("nodefirst");
    w.probes
        .insert("box3".into(), crate::probe::parse("box3", BARE));
    agent_setup::probed(&mut w, "box3");
    let inst = |h: &str, a: &str| Action::AgentInstall {
        host: h.into(),
        agent: a.into(),
    };
    assert!(w.plan.has(&inst("box3", "claude")) && w.plan.has(&inst("box3", "codex")));
    let rows = w.rows_of(Step::Agents);
    let note: String = rows
        .iter()
        .find(|r| matches!(&r.kind, rows::Kind::Pick { id, .. } if id == "inst:box3:gemini"))
        .map(|r| r.note.iter().map(|s| s.content.to_string()).collect())
        .unwrap();
    assert!(
        note.contains("npm i -g @google/gemini-cli · Node.js LTS first"),
        "{note}"
    );
    assert!(rows
        .iter()
        .any(|r| r.label.contains("Node.js (the current LTS) goes on first")));
    // each machine has its own list: box4 (with Codex) gets nothing ticked
    w.probes.insert(
        "box4".into(),
        crate::probe::parse("box4", &format!("CCA\tcodex\t/u/codex\t0.46.0\n{BARE}")),
    );
    agent_setup::probed(&mut w, "box4");
    assert!(!w.plan.has(&inst("box4", "claude")));
    // ticking one on box3 leaves box4 alone
    agent_setup::act(&mut w, Ev::Pick("inst:box3:gemini".into()));
    assert!(w.plan.has(&inst("box3", "gemini")) && !w.plan.has(&inst("box4", "gemini")));
    let rows = w.rows_of(Step::Agents);
    assert!(
        rows.iter().any(|r| r.label.contains(
            "Each machine has its own list: box3 → Claude Code, Codex, Gemini CLI · box4 → nothing new"
        )),
        "{:?}",
        rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn the_agents_step_is_a_checklist_of_every_agent() {
    // agents.node = off: npm agents need Node.js there first, and say so
    let lines = [
        "hosts=box3",
        "ssh_host=box3",
        "local=false",
        "agents.node=off",
    ];
    let mut w = wiz(&lines, Opts::default(), "checklist");
    w.state_path = fresh("checklist");
    w.probes
        .insert("box3".into(), crate::probe::parse("box3", BARE));
    agent_setup::probed(&mut w, "box3");
    let inst = |a: &str| Action::AgentInstall {
        host: "box3".into(),
        agent: a.into(),
    };
    // nothing there: the default agent is ticked; Codex isn't (no Node.js), and that's said
    assert!(w.plan.has(&inst("claude")));
    assert!(!w.plan.has(&inst("codex")));
    let rows = w.rows_of(Step::Agents);
    let picks: Vec<&rows::Row> = rows
        .iter()
        .filter(|r| matches!(&r.kind, rows::Kind::Pick { id, .. } if id.starts_with("inst:box3:")))
        .collect();
    assert_eq!(picks.len(), crate::agents::ALL.len());
    let note = |id: &str| -> String {
        let want = format!("inst:box3:{id}");
        picks
            .iter()
            .find(|r| matches!(&r.kind, rows::Kind::Pick { id: x, .. } if *x == want))
            .map(|r| r.note.iter().map(|s| s.content.to_string()).collect())
            .unwrap_or_default()
    };
    assert!(note("claude")
        .contains("installs in Review · curl -fsSL https://claude.ai/install.sh | bash"));
    assert!(note("codex").contains("needs Node.js"));
    assert!(note("grok").contains("x.ai/cli/install.sh"));
    assert!(rows.iter().any(|r| r.label.contains(
        "Nothing is installed on box3 yet: ticked Claude Code (the default agent). Not ticked: Codex needs Node.js here."
    )));
    // ticking: a blocked one goes in (with a word)
    agent_setup::act(&mut w, Ev::Pick("inst:box3:codex".into()));
    assert!(w.plan.has(&inst("codex")));
    agent_setup::act(&mut w, Ev::Pick("inst:box3:claude".into()));
    assert!(!w.plan.has(&inst("claude")), "space takes it out again");
    // one that's there stays ticked, and can't be taken out
    let mut w2 = wiz(&base(), Opts::default(), "checklist2");
    agent_setup::probed(&mut w2, "b31");
    agent_setup::act(&mut w2, Ev::Pick("inst:b31:claude".into()));
    assert!(!w2.plan.has(&Action::AgentInstall {
        host: "b31".into(),
        agent: "claude".into()
    }));
    w2.cur = Step::Agents;
    let t = render(&mut w2, 120, 40);
    assert!(t.contains("Agents to install"), "{t}");
    assert!(t.contains("installed 2.1.200"), "{t}");
}

#[test]
fn review_installs_each_machines_agents_in_one_run() {
    let mut w = wiz(&base(), Opts::default(), "installs");
    w.state_path = fresh("installs");
    let inst = |h: &str, a: &str| Action::AgentInstall {
        host: h.into(),
        agent: a.into(),
    };
    w.plan.want(Step::Agents, inst("gpu2", "cursor"), true);
    w.plan.want(Step::Agents, inst("b31", "gemini"), true);
    w.plan.want(Step::Agents, inst("gpu2", "claude"), true);
    w.plan.want(
        Step::Herdr,
        Action::HerdrSave {
            host: "gpu2".into(),
            why: "updates herdr 0.8.0 → 0.9.5".into(),
        },
        true,
    );
    review::start(&mut w);
    let labels: Vec<&str> = w.rv.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels[0].starts_with("save gpu2 in herdr"), "{labels:?}");
    // herdr's own question first, in the terminal
    let h = review::pump(&mut w).unwrap();
    assert_eq!(h.id, "apply:0");
    review::applied(&mut w, 0, Ok("done".into()));
    // then every agent at once, each its own run (Gemini's npm, Cursor's and
    // Claude Code's scripts), on the install screen
    use review::ItemSt::Running;
    assert!(review::pump(&mut w).is_none());
    assert!(
        (1..=3).all(|k| w.rv.items[k].st == Running),
        "{:?}",
        w.rv.items.iter().map(|i| &i.st).collect::<Vec<_>>()
    );
    assert!(w.rv.items[2].label.contains("Cursor") && w.rv.items[3].label.contains("Claude"));
    // each is ticked on its own, as it finishes
    review::agent_done(&mut w, 1, Ok("Gemini CLI 0.65.0 installed".into()));
    assert!(matches!(w.rv.items[1].st, review::ItemSt::Ok(_)));
    assert_eq!(w.rv.items[2].st, Running);
    review::pump(&mut w);
    assert!(!w.rv.done, "the others are still going");
    review::agent_done(&mut w, 3, Ok("Claude Code installed".into()));
    review::agent_done(&mut w, 2, Err("exited with status 1".into()));
    review::pump(&mut w);
    // then the tidy-up of both machines, last
    assert!(
        w.rv.items[4].label.contains("tidy"),
        "{:?}",
        w.rv.items[4].label
    );
    assert!(!w.rv.done);
    review::applied(&mut w, 4, Ok("freed 1.2 GB".into()));
    review::pump(&mut w);
    assert!(w.rv.done && !w.finished);
    assert!(
        w.plan.has(&inst("gpu2", "cursor")),
        "what failed stays in the plan"
    );
    assert!(!w.plan.has(&inst("b31", "gemini")));
}

#[test]
fn an_agents_report_line_becomes_its_items_state() {
    use review::{item_of, ItemSt};
    let ok: Result<String, String> = Ok("done".into());
    let j = serde_json::json!({"id": "claude", "status": "installed", "version": "2.1.200"});
    assert_eq!(
        item_of(Some(&j), &ok),
        ItemSt::Ok("installed 2.1.200".into())
    );
    let j = serde_json::json!({"id": "codex", "status": "failed", "text": "Codex installs with npm, and box3 has no Node.js", "fix": "nvm install --lts"});
    assert_eq!(
        item_of(Some(&j), &ok),
        ItemSt::Failed(
            "Codex installs with npm, and box3 has no Node.js — nvm install --lts".into()
        )
    );
    assert_eq!(
        item_of(None, &Err("exited with status 1".into())),
        ItemSt::Failed("exited with status 1".into())
    );
}

#[test]
fn the_queue_puts_what_needs_you_first_then_herdr_tools_agents_integrations() {
    use review::order;
    let inst = |h: &str| Action::AgentInstall {
        host: h.into(),
        agent: "claude".into(),
    };
    let integ = Action::Integration {
        host: "local".into(),
        name: "claude".into(),
    };
    let mut v = vec![
        Action::Completions,
        integ.clone(),
        inst("local"),
        Action::notifier(),
        Action::HerdrLocal,
        Action::SshKey,
        Action::Service,
    ];
    v.sort_by_key(order);
    assert_eq!(
        v,
        vec![
            Action::SshKey,
            Action::HerdrLocal,
            Action::notifier(),
            inst("local"),
            integ,
            Action::Completions,
            Action::Service,
        ]
    );
}

#[test]
fn the_install_screen_shows_the_queue_its_output_and_one_that_needs_you() {
    let mut w = wiz(&base(), Opts::default(), "install-screen");
    w.state_path = fresh("install-screen");
    w.plan.want(Step::Preflight, Action::HerdrLocal, true);
    w.plan.want(Step::Integrations, Action::nerd_font(), true);
    w.plan.want(
        Step::Preflight,
        Action::SysPkg {
            mgr: "apt-get".into(),
            pkg: "git".into(),
            ask: true,
        },
        true,
    );
    w.go(Step::Review);
    review::start(&mut w);
    assert!(install::shown(&w));
    // sudo's password first: tests hand it over at once (no threads here),
    // but on the real screen it waits for enter
    w.quiet = false;
    assert!(review::pump(&mut w).is_none());
    assert_eq!(w.rv.waiting, Some(0));
    let t = render(&mut w, 120, 32);
    assert!(
        t.contains("this one needs you") && t.contains("password"),
        "{t}"
    );
    assert!(t.contains("hand over the terminal"), "{t}");
    key(&mut w, KeyCode::Enter);
    let h = review::pump(&mut w).expect("enter hands it over");
    assert!(
        matches!(&h.run, Run::Cmd { args, .. } if args.join(" ").contains("sudo apt-get install -y git"))
    );
    w.quiet = true;
    review::applied(&mut w, 0, Ok("git installed".into()));
    // herdr running, with output and a percentage from it
    review::pump(&mut w);
    assert_eq!(w.rv.items[1].st, review::ItemSt::Running);
    install::line(&mut w, 1, "\x1b[1m==>\x1b[0m Downloading herdr");
    install::line(
        &mut w,
        1,
        "######################                     52.0%",
    );
    assert_eq!(w.rv.items[1].pct, Some(0.52));
    for (c, r) in [(120, 32), (80, 24), (60, 18)] {
        let t = render(&mut w, c, r);
        assert!(
            t.contains("installing") && t.contains("Queue"),
            "{c}×{r}\n{t}"
        );
        if c >= 80 {
            assert!(
                t.contains("==> Downloading herdr") && t.contains("52%"),
                "{c}×{r}\n{t}"
            );
        }
    }
    review::applied(&mut w, 1, Ok("herdr 0.9.6".into()));
    review::pump(&mut w);
    review::applied(&mut w, 2, Ok("JetBrains Mono Nerd Font installed".into()));
    review::pump(&mut w);
    review::applied(&mut w, 3, Ok("freed 310 MB".into()));
    review::pump(&mut w);
    assert!(w.rv.done && w.rv.celebrate.is_some());
    let t = render(&mut w, 120, 32);
    assert!(
        t.contains("You're all set") && t.contains("open aleph"),
        "{t}"
    );
    assert!(t.contains("4 done"), "{t}");
    // enter opens Aleph
    assert!(matches!(
        key(&mut w, KeyCode::Enter),
        Some(Exit::Dashboard { .. })
    ));
}

#[test]
fn a_failure_can_be_run_again_from_the_install_screen() {
    let mut w = wiz(&base(), Opts::default(), "install-retry");
    w.state_path = fresh("install-retry");
    w.plan.want(Step::Integrations, Action::notifier(), true);
    w.plan.want(Step::Integrations, Action::Completions, true);
    review::start(&mut w);
    review::pump(&mut w);
    review::applied(&mut w, 0, Err("brew: no network".into()));
    review::pump(&mut w);
    review::applied(&mut w, 1, Ok("zsh".into()));
    review::pump(&mut w);
    review::applied(&mut w, 2, Ok("nothing to tidy".into()));
    review::pump(&mut w);
    assert!(w.rv.done && w.rv.celebrate.is_none());
    let t = render(&mut w, 120, 32);
    assert!(
        t.contains("brew: no network") && t.contains("retry the failed"),
        "{t}"
    );
    key(&mut w, KeyCode::Char('r'));
    assert!(!w.rv.done);
    review::pump(&mut w);
    assert_eq!(w.rv.items[0].st, review::ItemSt::Running);
    assert!(
        matches!(w.rv.items[1].st, review::ItemSt::Ok(_)),
        "what went in stays"
    );
    review::applied(&mut w, 0, Ok("terminal-notifier installed".into()));
    review::pump(&mut w);
    assert!(w.rv.done && w.rv.celebrate.is_some() && w.finished);
}

#[test]
fn quick_start_queues_what_this_computer_is_missing() {
    let lines = ["local=true"];
    let mut w = wiz(&lines, Opts::default(), "quick");
    w.state_path = fresh("quick");
    // no herdr here, git there, Homebrew, no terminal-notifier, no agents
    let mut herdr = preflight_check();
    (herdr.id, herdr.group, herdr.st) = ("herdr".into(), "Apps and herdr", St::Bad);
    w.pre.checks = vec![preflight_check(), herdr];
    w.ig.facts = Some(integrations::Facts {
        shell: "zsh".into(),
        rc: Some(PathBuf::from("/h/.zshrc")),
        brew: true,
        ..Default::default()
    });
    w.probes.insert(
        "local".into(),
        crate::probe::parse(
            "local",
            "CCT\tcurl\t/usr/bin/curl\nCCT\tbash\t/bin/bash\nCCO\tDarwin arm64\nCCW\t/Users/t\n",
        ),
    );
    quick::plan(&mut w);
    let has = |a: &Action| w.plan.has(a);
    assert!(has(&Action::HerdrLocal));
    assert!(!has(&Action::tool("git")), "git is there");
    assert!(has(&Action::AgentInstall {
        host: "local".into(),
        agent: "claude".into()
    }));
    // Claude Code's herdr integration with it (herdr is on its way)
    assert!(has(&Action::Integration {
        host: "local".into(),
        name: "claude".into()
    }));
    assert!(has(&Action::Completions));
    if cfg!(target_os = "macos") {
        assert!(has(&Action::notifier()));
    }
    // nothing needs you: it runs straight through
    assert!(w.plan.actions.iter().all(|(_, a)| !a.tty()));
}

#[test]
fn agents_install_side_by_side_but_one_npm_at_a_time() {
    let mut w = wiz(&base(), Opts::default(), "par-lanes");
    w.state_path = fresh("par-lanes");
    let inst = |a: &str| Action::AgentInstall {
        host: "b31".into(),
        agent: a.into(),
    };
    for a in ["gemini", "codex", "claude"] {
        w.plan.want(Step::Agents, inst(a), true);
    }
    review::start(&mut w);
    review::pump(&mut w);
    use review::ItemSt::{Running, Todo};
    let st = |w: &Wiz, a: &str| {
        w.rv.items
            .iter()
            .find(|i| matches!(&i.what, review::What::Act(Action::AgentInstall { agent, .. }) if agent == a))
            .map(|i| i.st.clone())
            .unwrap()
    };
    // Gemini and Codex both install with npm: one waits; Claude Code's
    // script installer goes beside them
    assert_eq!(
        (st(&w, "gemini"), st(&w, "codex"), st(&w, "claude")),
        (Running, Todo, Running)
    );
    let g =
        w.rv.items
            .iter()
            .position(|i| i.label.contains("Gemini"))
            .unwrap();
    review::agent_done(&mut w, g, Ok("Gemini CLI installed".into()));
    review::pump(&mut w);
    assert_eq!(st(&w, "codex"), Running);
}

#[test]
fn herdr_s_missing_folder_is_read_off_its_message() {
    use review::missing_dir;
    assert_eq!(
        missing_dir(
            "omp extension directory not found at /home/x/.omp/agent/extensions. install omp first"
        )
        .as_deref(),
        Some("/home/x/.omp/agent/extensions")
    );
    assert_eq!(
        missing_dir(
            "qwen code config directory not found at /home/x/.qwen. install qwen code first"
        )
        .as_deref(),
        Some("/home/x/.qwen")
    );
    assert_eq!(missing_dir("integration install failed: network"), None);
}

#[test]
fn the_tidy_up_takes_only_links_a_failed_install_left_broken() {
    let dir = fresh("tidy-links").with_extension("d");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib/node_modules/kept/bin")).unwrap();
    let kept = dir.join("lib/node_modules/kept/bin/kept");
    std::fs::write(&kept, "").unwrap();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let link =
        |to: &std::path::Path, name: &str| std::os::unix::fs::symlink(to, bin.join(name)).unwrap();
    link(&kept, "kept");
    link(&dir.join("lib/node_modules/gone/bin/gone"), "gone");
    // a broken link of your own, outside node_modules, stays
    link(&dir.join("elsewhere/mine"), "mine");
    assert_eq!(review::broken_links(&bin), vec![bin.join("gone")]);
    assert_eq!(review::human(512_000), "512 KB");
    assert_eq!(review::human(310_000_000), "310 MB");
    assert_eq!(review::human(1_240_000_000), "1.2 GB");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_openrouter_key_goes_to_the_keystore_never_the_plan() {
    const KEY: &str = "sk-or-v1-0ddba11c0ffee0ddba11c0ffee0ddba11c0ffee0ddba11c0ffee00aa";
    let off = [base(), vec!["openrouter.key=off"]].concat();
    for (cfg, points) in [(base(), false), (off, true)] {
        let mut w = wiz(&cfg, Opts::default(), &format!("openrouter-{points}"));
        w.go(Step::Agents);
        let rs = w.rows_of(Step::Agents);
        let row = rs
            .iter()
            .find(|r| r.ident() == "id:openrouter")
            .expect("the row");
        assert!(row.selectable(), "{row:?}");
        assert!(rs.iter().any(|r| r.ident() == "set:openrouter.agents"));
        select(&mut w, "id:openrouter");
        key(&mut w, KeyCode::Enter);
        assert!(
            matches!(&w.mode, Overlay::Edit { secret: true, to: EditFor::Step(Step::Agents, id), .. } if id == "openrouter-key"),
            "a masked input of the step's own"
        );
        for ch in KEY.chars() {
            key(&mut w, KeyCode::Char(ch));
        }
        // dots on the screen, never the key
        let s = render(&mut w, 120, 32);
        assert!(
            !s.contains(KEY) && !s.contains("0ddba11") && s.contains("•••••"),
            "{s}"
        );
        key(&mut w, KeyCode::Enter);
        assert!(matches!(w.mode, Overlay::None));
        // the plan holds where it lives (only when the setting said otherwise), never the key
        let plan = w.plan.to_json().to_string();
        assert!(!plan.contains(KEY), "{plan}");
        assert_eq!(plan.contains("secret:openrouter"), points, "{plan}");
        w.save();
        let saved = std::fs::read_to_string(&w.state_path).unwrap_or_default();
        assert!(!saved.is_empty() && !saved.contains(KEY), "{saved}");
        let s = render(&mut w, 160, 40);
        assert!(s.contains("OpenRouter key"), "{s}");
    }
}

#[test]
fn a_failed_installs_reason_is_its_own_error_never_alephs_note() {
    // Hermes stopped by `aleph agents install`'s limit (its error on stderr,
    // the note for an open dashboard last)
    let text = "✓ uv ready (uv 0.13.5)\n ffmpeg:   3.9%  4.0 MiB / 103.7 MiB\n\
        error: installing Hermes Agent took longer than 15 minutes, so it was stopped\n\
        hint: retry on a better connection\n  log: ~/.local/state/aleph/installs/local-hermes-1.log\n\
        ✗ left a note for Aleph — an open dashboard looks at local again within a second\n";
    assert_eq!(
        review::failure_reason(text).as_deref(),
        Some("installing Hermes Agent took longer than 15 minutes, so it was stopped")
    );
    // no error line: its own ✗, else its last line — never the note
    let text = "✗ npm ERR! code EACCES\n× left a note for Aleph — …\n";
    assert_eq!(
        review::failure_reason(text).as_deref(),
        Some("npm ERR! code EACCES")
    );
    let text = "Cloning into 'x'...\nfatal: unable to access the repository\n✗ left a note for Aleph — …\n";
    assert_eq!(
        review::failure_reason(text).as_deref(),
        Some("fatal: unable to access the repository")
    );
    assert_eq!(review::failure_reason("✗ left a note for Aleph\n"), None);
}

#[test]
fn x_stops_only_a_running_agent_install() {
    let mut w = wiz(&base(), Opts::default(), "install-stop");
    w.state_path = fresh("install-stop");
    w.plan.want(
        Step::Agents,
        Action::AgentInstall {
            host: "local".into(),
            agent: "hermes".into(),
        },
        true,
    );
    w.plan.want(Step::Integrations, Action::Completions, true);
    review::start(&mut w);
    let k =
        w.rv.items
            .iter()
            .position(|i| matches!(i.what, review::What::Act(Action::AgentInstall { .. })))
            .unwrap();
    let other =
        w.rv.items
            .iter()
            .position(|i| matches!(i.what, review::What::Act(Action::Completions)))
            .unwrap();
    // not running yet: nothing to stop
    assert!(!review::stop_one(&mut w, k));
    w.rv.items[k].st = review::ItemSt::Running;
    w.rv.items[other].st = review::ItemSt::Running;
    assert!(!review::stop_one(&mut w, other), "only an agent's install");
    assert!(review::stop_one(&mut w, k));
    assert!(w.rv.items[k]
        .stop
        .load(std::sync::atomic::Ordering::Relaxed));
    // the screen offers it while that one is shown and running
    w.rv.look = Some(k);
    let t = render(&mut w, 140, 32);
    assert!(t.contains("stop this one"), "{t}");
}
