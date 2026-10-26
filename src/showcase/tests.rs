//! Showcase mode: fakes keep each word's shape, nothing private gets out of
//! the text, JSON or a drawn frame, and the frame's structure is unchanged.

use super::{dict, fake, json, scan, state, tui, Kind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::json;

const SALT: &[u8] = b"0123456789abcdef0123456789abcdef";

fn on() {
    state::force(Some(state::State {
        mode: state::Mode::Masked,
        salt: SALT.to_vec(),
        epoch: 1,
        ..Default::default()
    }));
    dict::set_test(
        SALT,
        "/home/selene",
        &[
            (Kind::Host, "b31"),
            (Kind::Host, "gpu2"),
            (Kind::Host, "dex42"),
            (Kind::User, "selene"),
            (Kind::Project, "claude-herdr"),
            (Kind::Project, "ox_color_check"),
            (Kind::Session, "claude_scan"),
        ],
    );
}

#[test]
fn fakes_keep_the_shape_and_need_the_salt() {
    // a machine: a ship or station of the same length (ds9, iss, ghost…)
    let ship = |f: &str| {
        fake::Pools::defaults()
            .host
            .names
            .iter()
            .any(|n| n.replace('-', "") == f)
    };
    let f = fake::fake(SALT, Kind::Host, "b31", 0);
    assert_eq!(f.len(), 3);
    assert!(ship(&f), "{f}");
    let g = fake::fake(SALT, Kind::Host, "dex42", 0);
    assert!(ship(&g), "{g}");
    let g = fake::fake(SALT, Kind::Project, "ox_color_check", 0);
    assert_eq!(g.len(), 14);
    assert_eq!(g.chars().filter(|c| *c == '_').count(), 2);
    assert_ne!(
        f,
        fake::fake(b"a different salt, also 32 bytes!", Kind::Host, "b31", 0)
    );
    assert_eq!(
        fake::fake(SALT, Kind::Host, "GPU2", 0).to_uppercase(),
        fake::fake(SALT, Kind::Host, "GPU2", 0)
    );
    assert_eq!(fake::ipv4("172.16.200.11", 7).len(), 13);
    assert!(fake::ipv4("100.101.102.103", 9).starts_with("127."));
    assert_eq!(fake::version("0.9.5").len(), 5);
}

#[test]
fn text_loses_every_private_word_and_keeps_its_width() {
    on();
    let d = dict::get();
    let lines = [
        "b31 · claude_scan  /home/selene/work/claude-herdr/src/main.rs  ssh selene@172.16.200.11",
        "herdr 0.9.5 on dex42 · gpu2 is offline · mail alex@example.com",
        "ssh: connect to host 100.101.102.103 port 22: Operation timed out",
        "~/Projects/ox_color_check · claude-he… · GPU2",
        "selene@10.0.0.49 · tester@gpu2.lan:2222",
    ];
    // what isn't anyone's stays: GPU headings, OS versions
    for keep in [
        "GPU 0 · CPU 64%",
        "Ubuntu 22.04.5 LTS · Linux 6.8.0-45-generic · macOS 26.0.1",
    ] {
        assert_eq!(scan::scrub_with(&d, keep), keep);
    }
    for l in lines {
        let s = scan::scrub_with(&d, l);
        assert_eq!(s.chars().count(), l.chars().count(), "{l} → {s}");
        for real in [
            "b31",
            "gpu2",
            "dex42",
            "selene",
            "claude-herdr",
            "claude-he…",
            "ox_color_check",
            "hunt",
            "192.168",
            "100.101",
            "0.9.5",
            "alexander",
            "example",
            "GPU2",
            "10.0.0.49",
        ] {
            assert!(!s.contains(real), "{real} still in {s}");
        }
        // what isn't private stays
        assert!(
            !l.contains("Operation") || s.contains("Operation timed out"),
            "{s}"
        );
    }
    assert!(
        scan::scrub_with(&d, "/home/selene/work/claude-herdr/src/main.rs")
            .ends_with("/src/main.rs")
    );
    // a selector coming back: the real one it stands for
    let fake_sel = scan::scrub_with(&d, "b31:claude_scan");
    assert_eq!(
        scan::unmask(&fake_sel, &["gpu2:x".into(), "b31:claude_scan".into()]).as_deref(),
        Some("b31:claude_scan")
    );
    state::force(None);
}

#[test]
fn story_names_and_agent_programs_stay() {
    on();
    let d = dict::set_test(
        SALT,
        "/home/selene",
        &[
            (Kind::Host, "b31"),
            (Kind::User, "selene"),
            (Kind::Session, "smoldering-heron"),
        ],
    );
    // Aleph's own names, and their words, say nothing about anyone
    let s = scan::scrub_with(&d, "smoldering-heron on b31");
    assert!(
        s.starts_with("smoldering-heron on ") && !s.contains("b31"),
        "{s}"
    );
    // nor does an agent's program in a path (the user still goes)
    let p = scan::scrub_with(&d, "/home/selene/.local/bin/claude");
    assert!(
        p.ends_with("/.local/bin/claude") && !p.contains("selene"),
        "{p}"
    );
    state::force(None);
}

#[test]
fn json_keeps_ids_and_shape() {
    on();
    let v = json::mask(json!({
        "id": "b31|temper|1",
        "hosts": [{"alias": "b31", "cwd": "/home/selene/ox_color_check", "version": "0.9.5", "status": "working",
                   "account": "beef0000bbbb2222", "token": "aleph_abc"}],
        "b31": {"codex": 1},
    }));
    assert_eq!(
        v["id"], "b31|temper|1",
        "ids pass: dismiss and read marks keep working"
    );
    let h = &v["hosts"][0];
    assert_ne!(h["alias"], "b31");
    assert_eq!(h["status"], "working");
    assert!(!h["cwd"].as_str().unwrap().contains("ox_color_check"));
    assert_ne!(h["version"], "0.9.5");
    assert_ne!(h["account"], "beef0000bbbb2222");
    assert_eq!(h["account"].as_str().unwrap().len(), 16);
    assert_eq!(h["token"], "••••");
    assert!(
        v.get("b31").is_none(),
        "a map keyed by a machine gets its fake"
    );
    state::force(None);
}

#[test]
fn a_frame_changes_letters_only() {
    on();
    let area = Rect::new(0, 0, 60, 3);
    let mut b = Buffer::empty(area);
    b.set_string(
        0,
        0,
        "╭ b31 ─ claude_scan ──────────────────╮",
        ratatui::style::Style::default(),
    );
    b.set_string(
        0,
        1,
        "│ /home/selene/claude-herdr  ● 0.9.5  │",
        ratatui::style::Style::default(),
    );
    b.set_string(
        0,
        2,
        "╰ 🐧 gpu2 ─────────────────────────────╯",
        ratatui::style::Style::default(),
    );
    let before = b.clone();
    tui::buffer(&mut b);
    let text = |b: &Buffer| -> String {
        (0..3)
            .map(|y| {
                (0..60)
                    .map(|x| b[(x, y)].symbol().to_string())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    };
    let after = text(&b);
    for real in ["b31", "hunt", "selene", "claude-herdr", "0.9.5", "gpu2"] {
        assert!(!after.contains(real), "{real} in\n{after}");
    }
    // borders, spaces and symbols are exactly where they were
    for y in 0..3 {
        for x in 0..60 {
            let (o, n) = (before[(x, y)].symbol(), b[(x, y)].symbol());
            let plain = |s: &str| s.chars().all(|c| !c.is_alphanumeric());
            if plain(o) {
                assert_eq!(o, n, "({x},{y})");
            }
        }
    }
    state::force(None);
}

#[test]
fn a_fresh_salt_means_other_stand_ins() {
    // the same word under two salts (what each start of the dashboard draws)
    let a = fake::fake(SALT, Kind::Host, "magrathea-box", 0);
    let b = fake::fake(
        b"fedcba9876543210fedcba9876543210",
        Kind::Host,
        "magrathea-box",
        0,
    );
    assert_ne!(a, b);
    assert_eq!(a.len(), b.len());
}

#[test]
fn a_daemon_reporting_this_builds_stand_in_runs_this_build() {
    on();
    let ours = env!("CARGO_PKG_VERSION");
    let seen = json::mask(json!({"version": ours}))["version"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(seen, ours, "the version is masked on its way out");
    assert_eq!(crate::showcase::own_version(&seen), ours);
    assert_eq!(crate::showcase::own_version("0.9.1"), "0.9.1");
    state::force(None);
    assert_eq!(crate::showcase::own_version(&seen), seen, "off: as it came");
}

#[test]
fn whole_names_come_from_the_fields_pools_in_the_words_style() {
    let p = fake::Pools::defaults();
    let host = fake::full(SALT, p, Kind::Host, "b31", 0);
    assert!(p.host.names.contains(&host), "{host}");
    assert!(!host.contains('_'));
    // a folder keeps its separator, a SHOUTED word shouts
    let f = fake::full(SALT, p, Kind::Project, "ox_color_check", 0);
    assert!(
        !f.contains('-') && p.thing.names.contains(&f.replace('_', "-")),
        "{f}"
    );
    assert_eq!(
        fake::full(SALT, p, Kind::Host, "GPU2", 0),
        fake::full(SALT, p, Kind::Host, "gpu2", 0).to_uppercase()
    );
    assert_eq!(fake::styled("Hunt", "black_mesa"), "Black_Mesa");
    // users are AIs and computers by default, machines ships and stations
    let ais = super::universe::names(&[], &[super::universe::Cat::Ais]);
    let u = fake::full(SALT, p, Kind::User, "selene", 0);
    assert!(ais.contains(&u.as_str()), "{u}");
}

#[test]
fn settings_pick_universes_kinds_and_your_own_words() {
    let field = |k: &str, mine: &str| (k.to_string(), mine.to_string());
    // one universe, pinned: every name from it
    let p = fake::Pools::build(
        "all",
        "valve",
        [field("ships", ""), field("ais", ""), field("things", "")],
        SALT,
    );
    let valve = super::universe::find("valve").unwrap();
    assert!(p.user.names.iter().all(|n| valve.ais.contains(&n.as_str())));
    assert!(p
        .host
        .names
        .iter()
        .all(|n| valve.ships.contains(&n.as_str())));
    // only your own words
    let p = fake::Pools::build(
        "all",
        "mixed",
        [
            field("mine", "Rocinante, Tycho Station"),
            field("ais", ""),
            field("any", ""),
        ],
        SALT,
    );
    assert_eq!(p.host.names, vec!["rocinante", "tycho-station"]);
    // one per launch: the salt picks it, from the ones that are on
    let one = |salt: &[u8]| {
        fake::Pools::build(
            "trek,starwars",
            "per-launch",
            [field("people", ""), field("ais", ""), field("any", "")],
            salt,
        )
        .host
        .names
    };
    let a = one(b"\x00\x00\x00\x00\x00\x00\x00\x00");
    let b = one(b"\x00\x00\x00\x00\x00\x00\x00\x01");
    assert_ne!(a, b, "two launches, two universes");
    let trek = super::universe::find("trek").unwrap();
    let wars = super::universe::find("starwars").unwrap();
    for names in [&a, &b] {
        assert!(
            names.iter().all(|n| trek.people.contains(&n.as_str()))
                || names.iter().all(|n| wars.people.contains(&n.as_str()))
        );
    }
    // a kind a universe hasn't got: never an empty pool
    let p = fake::Pools::build(
        "cosmos",
        "mixed",
        [
            field("people", ""),
            field("people", ""),
            field("people", ""),
        ],
        SALT,
    );
    assert!(!p.user.names.is_empty());
}

#[test]
fn whole_names_in_text_and_frames_keep_every_word_out() {
    let d = dict::set_test_names(
        SALT,
        "/home/selene",
        &[
            (Kind::Host, "b31"),
            (Kind::User, "selene"),
            (Kind::Session, "pxaudit"),
        ],
        dict::Names::Full,
    );
    state::force(Some(state::State {
        mode: state::Mode::Masked,
        salt: SALT.to_vec(),
        epoch: 1,
        ..Default::default()
    }));
    let host = d.stand_in_of("b31", true).unwrap().to_string();
    let t = scan::scrub_with(&d, "approved claude_pxaudit on b31 (/home/selene/px)");
    assert!(
        t.contains(&host) && !t.contains("b31") && !t.contains("selene") && !t.contains("pxaudit"),
        "{t}"
    );
    // a frame: the whole name, laid into its row
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 2));
    buf.set_string(
        0,
        0,
        "▌b31            ⠿ working",
        ratatui::style::Style::default(),
    );
    buf.set_string(0, 1, "on b31 │ x", ratatui::style::Style::default());
    tui::buffer(&mut buf);
    let row = |y: u16| (0..40).map(|x| buf[(x, y)].symbol()).collect::<String>();
    assert!(row(0).starts_with(&format!("▌{host}")), "{}", row(0));
    assert_eq!(
        row(0).find('⠿'),
        "▌b31            ⠿".find('⠿'),
        "the next column stays"
    );
    assert!(
        !row(1).contains("b31") && row(1).contains("│ x"),
        "{}",
        row(1)
    );
    // a printed line's spans: the name across them, each span its part
    let parts = crate::showcase::spans(&["✓ approved ", "claude_pxaudit", " on b31"]).unwrap();
    assert_eq!(parts.len(), 3);
    assert!(
        parts[2].ends_with(&host) && !parts.concat().contains("pxaudit"),
        "{parts:?}"
    );
    // a name coming back in: either stand-in finds the real one
    let reals = vec!["b31".to_string()];
    assert_eq!(scan::unmask(&host, &reals).as_deref(), Some("b31"));
    state::force(None);
}
