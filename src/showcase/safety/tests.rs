//! The safety nets: what a redacted screen keeps and swaps, the pause's
//! frame and answers, the clipboard's three ways, money scaled and hidden.

use super::redact::{self, GLYPHS};
use super::*;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Style};
use ratatui::Terminal;
use unicode_width::UnicodeWidthStr;

fn with(mode: &str, s: Settings) {
    crate::showcase::force(mode);
    force(Some(s));
}

fn done() {
    crate::showcase::force("off");
    force(None);
}

fn cfg(pairs: &[&str]) -> Settings {
    Settings::from_config(&Config::in_memory(pairs))
}

fn red(l: &str) -> String {
    redact::text(l, &[])
}

/// No letter or digit of `l` is left.
fn no_words(l: &str) -> bool {
    !l.chars().any(char::is_alphanumeric)
}

#[test]
fn the_script_is_one_cell_a_symbol_and_never_a_word() {
    for g in GLYPHS {
        assert_eq!(unicode_width::UnicodeWidthChar::width(g), Some(1), "{g}");
        assert!(!g.is_alphanumeric(), "{g}");
    }
    // the same word, the same symbols; another word, others
    let a = redact::alien(&"pytest".chars().collect::<Vec<_>>(), 1);
    assert_eq!(a, redact::alien(&"pytest".chars().collect::<Vec<_>>(), 1));
    assert_ne!(a, redact::alien(&"pytesx".chars().collect::<Vec<_>>(), 1));
    assert_ne!(a, redact::alien(&"pytest".chars().collect::<Vec<_>>(), 2));
    assert_eq!(a.chars().count(), 6);
    // a wide character keeps its two cells
    let w = redact::alien(&"日本".chars().collect::<Vec<_>>(), 1);
    assert_eq!(UnicodeWidthStr::width(w.as_str()), 4);
}

#[test]
fn a_claude_screen_keeps_its_frame_and_loses_its_words() {
    let screen = [
        "╭────────────────────────────────────────────╮",
        "│ ✻ Welcome to Claude Code!                  │",
        "│   /help for help, /status for your setup   │",
        "│   cwd: ~/work/ox_color_check                │",
        "╰────────────────────────────────────────────╯",
        "> fix the flaky tests in the auth module",
        "⏺ I'll profile the layout engine build first.",
        "⏺ Bash(cargo bench --bench parse_layout)",
        "  ⎿  Finished `bench` in 41.2 s",
        "✻ Pondering… (12s · ↑ 1.2k tokens · esc to interrupt)",
        "│ Do you want to proceed?                    │",
        "│ ❯ 1. Yes                                    │",
        "│   2. Yes, and don't ask again for pytest commands │",
        "│   3. No, and tell Claude what to do differently (esc) │",
        "  ⏵⏵ accept edits on (shift+tab to cycle)",
        "  claude-opus-4-1 · 45% context left",
    ];
    for l in screen {
        let r = red(l);
        assert_eq!(
            UnicodeWidthStr::width(r.as_str()),
            UnicodeWidthStr::width(l),
            "{l} → {r}"
        );
        // spaces and the frame where they were
        for (a, b) in l.chars().zip(r.chars()) {
            if a.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&a) {
                assert_eq!(a, b, "{l} → {r}");
            }
        }
    }
    // the agent's own lines stay as they are
    for keep in [0, 1, 2, 3, 4, 9, 10, 11, 12, 13, 14, 15] {
        assert_eq!(red(screen[keep]), screen[keep]);
    }
    // the conversation goes, its marks stay
    let prompt = red(screen[5]);
    assert!(prompt.starts_with("> ") && no_words(&prompt), "{prompt}");
    let said = red(screen[6]);
    assert!(
        said.starts_with("⏺ ") && no_words(&said) && said.ends_with('.'),
        "{said}"
    );
    // a tool call keeps the tool's name, not what it ran
    let call = red(screen[7]);
    assert!(call.starts_with("⏺ Bash(") && call.ends_with(')'), "{call}");
    assert!(
        !call.contains("cargo bench") && !call.contains("parse_layout"),
        "{call}"
    );
    let out = red(screen[8]);
    assert!(out.starts_with("  ⎿  [") && no_words(&out), "{out}");
}

#[test]
fn codex_opencode_and_shell_lines() {
    let keep = [
        "│ >_ OpenAI Codex (v0.58.0)                 │",
        "│ model:     gpt-5.1-codex high   /model to change │",
        "│ directory: ~/work/ivy                      │",
        "◦ Working (12s • esc to interrupt)",
        "  Would you like to run the following command?",
        "› 1. Yes, proceed (y)",
        "  Press enter to confirm or esc to cancel",
        "› Ask Codex to do anything",
        "  96% context left · ? for shortcuts",
        " opencode  v1.0.62  ~/work/ivy",
        "△ Permission required",
        "  1. Allow once",
        "• Explored",
    ];
    for l in keep {
        assert_eq!(red(l), l);
    }
    let ran = red("• Ran cargo test --release");
    assert!(ran.starts_with("• Ran ") && !ran.contains("cargo"), "{ran}");
    let edited = red("• Edited src/auth.rs (+12 -3)");
    assert!(
        edited.starts_with("• Edited ") && !edited.contains("auth"),
        "{edited}"
    );
    let gutter = red(" ┃ make the login retry on a flaky network");
    assert!(gutter.starts_with(" ┃ ") && no_words(&gutter), "{gutter}");
    let shell = red("selene@b31:~/work$ ls -la");
    assert!(
        no_words(&shell) && shell.contains('@') && shell.contains(":~/"),
        "{shell}"
    );
    // a numbered list in the agent's words isn't a question's options
    let list = red("1. First I'll read the config loader");
    assert!(list.starts_with("1. ") && !list.contains("First"), "{list}");
}

#[test]
fn model_names_and_counters_stay_among_words() {
    let r = red("Using Opus with claude-sonnet-4-5 and gpt-5.1-codex for 812 tokens");
    for w in ["Opus", "claude-sonnet-4-5", "gpt-5.1-codex", "812 tokens"] {
        assert!(r.contains(w), "{w} in {r}");
    }
    assert!(
        !r.contains("Using") && !r.contains("with") && !r.contains("for "),
        "{r}"
    );
    // the settings' own phrases keep a line
    let l = "  main ✓ · deploys to prod-eu";
    assert_ne!(redact::text(l, &[]), l);
    assert_eq!(redact::text(l, &["deploys to".into()]), l);
}

#[test]
fn ansi_keeps_its_escapes_and_its_colours() {
    let l = "\x1b[38;2;217;119;87m⏺\x1b[0m I'll profile \x1b[1mthe engine\x1b[0m.\x1b]8;;http://x\x07link\x1b]8;;\x07";
    let r = red(l);
    for esc in [
        "\x1b[38;2;217;119;87m",
        "\x1b[0m",
        "\x1b[1m",
        "\x1b]8;;http://x\x07",
        "\x1b]8;;\x07",
    ] {
        assert!(r.contains(esc), "{esc:?} in {r:?}");
    }
    let plain = crate::showcase::demo::screens::plain(&r);
    assert!(plain.starts_with("⏺ ") && !plain.contains("profile") && !plain.contains("link"));
    assert_eq!(
        UnicodeWidthStr::width(plain.as_str()),
        UnicodeWidthStr::width(crate::showcase::demo::screens::plain(l).as_str())
    );
    // several lines: each on its own, the line breaks kept
    let two = red("⏺ one thing\n✻ Thinking…\n");
    let lines: Vec<&str> = two.split('\n').collect();
    assert_eq!(lines.len(), 3);
    assert!(no_words(lines[0]) && lines[1] == "✻ Thinking…" && lines[2].is_empty());
}

fn row(b: &Buffer, y: u16) -> String {
    (0..b.area.width)
        .map(|x| b[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn drawn_cells_keep_their_frame_and_colours() {
    let mut t = Terminal::new(TestBackend::new(40, 4)).unwrap();
    t.draw(|f| {
        let b = f.buffer_mut();
        b.set_string(0, 0, "╭──────────╮", Style::new().fg(Color::Blue));
        b.set_string(0, 1, "│ ⏺ hello  │", Style::new().fg(Color::Red));
        b.set_string(0, 2, "✻ Thinking… (esc to interrupt)", Style::new());
        b.set_string(0, 3, "日本 wide words", Style::new());
        redact::cells(b, Rect::new(0, 0, 40, 4), &[]);
    })
    .unwrap();
    let b = t.backend().buffer().clone();
    assert_eq!(row(&b, 0).trim_end(), "╭──────────╮");
    let r1 = row(&b, 1);
    assert!(r1.starts_with("│ ⏺ ") && !r1.contains("hello"), "{r1}");
    assert_eq!(b[(4, 1)].fg, Color::Red);
    assert!(GLYPHS.contains(&b[(4, 1)].symbol().chars().next().unwrap()));
    assert_eq!(row(&b, 2).trim_end(), "✻ Thinking… (esc to interrupt)");
    let r3 = row(&b, 3);
    assert!(!r3.contains("wide") && !r3.contains('日'), "{r3}");
}

#[test]
fn a_hidden_screen_is_a_card() {
    let mut t = Terminal::new(TestBackend::new(50, 9)).unwrap();
    t.draw(|f| {
        let b = f.buffer_mut();
        b.set_string(
            0,
            4,
            "secret words on the screen",
            Style::new().bg(Color::Red),
        );
        redact::card(b, Rect::new(0, 0, 50, 9));
    })
    .unwrap();
    let b = t.backend().buffer().clone();
    let all: String = (0..9).map(|y| row(&b, y)).collect();
    assert!(!all.contains("secret") && all.contains("screen hidden for the demo"));
    assert!(all.contains('╭') && all.contains('╯'));
    // the terminal's own background
    assert!((0..9).all(|y| (0..50).all(|x| b[(x, y)].bg == Color::Reset)));
    // too small for the card: the words alone
    let mut t = Terminal::new(TestBackend::new(20, 1)).unwrap();
    t.draw(|f| redact::card(f.buffer_mut(), Rect::new(0, 0, 20, 1)))
        .unwrap();
    assert!(row(t.backend().buffer(), 0).contains("◌ screen"));
}

#[test]
fn settings_and_their_defaults() {
    let d = cfg(&[]);
    assert_eq!(d.screens, Screens::Masked);
    assert_eq!(d.demo_screens, Screens::Masked);
    assert_eq!(d.panic_keys, vec!["f12".to_string()]);
    assert_eq!(d.clipboard, Clip::Masked);
    assert_eq!(d.money, Money::Real);
    let s = cfg(&[
        "showcase.screens=redact",
        "showcase.panic_key=f9, ctrl+4",
        "showcase.clipboard=off",
        "showcase.money=scaled",
        "showcase.money_scale=0.5",
        "showcase.screens_keep=Deploys To, main",
    ]);
    assert_eq!(s.screens, Screens::Redact);
    assert_eq!(s.panic_keys, vec!["f9".to_string(), "ctrl+4".to_string()]);
    assert_eq!(s.clipboard, Clip::Off);
    assert_eq!(s.money, Money::Scaled(0.5));
    assert_eq!(s.keep, vec!["deploys to".to_string(), "main".to_string()]);
    assert!(cfg(&["showcase.panic_key=off"]).panic_keys.is_empty());
    assert_eq!(
        cfg(&["showcase.money=scaled", "showcase.money_scale=junk"]).money,
        Money::Scaled(SCALE)
    );
    // every one is showcase's own (Settings › Showcase, kept out of docs/config.md)
    for k in [
        "showcase.screens",
        "showcase.demo_screens",
        "showcase.screens_keep",
        "showcase.panic_key",
        "showcase.clipboard",
        "showcase.money",
        "showcase.money_scale",
    ] {
        let known = crate::config::known(k).unwrap_or_else(|| panic!("{k} isn't in KNOWN"));
        assert!(known.help.len() > 60, "{k}: say more");
        assert!(crate::showcase::own_key(k));
    }
}

#[test]
fn screens_follow_the_mode_and_the_world() {
    let s = cfg(&["showcase.screens=redact", "showcase.demo_screens=hide"]);
    with("off", s.clone());
    assert_eq!(crate::showcase::screen_text("⏺ hello"), "⏺ hello");
    with("masked", s.clone());
    assert!(no_words(&crate::showcase::screen_text("⏺ hello")));
    with("demo", s);
    assert_eq!(crate::showcase::screen_text("⏺ hello"), redact::HIDDEN);
    // a blank screen stays blank
    assert_eq!(crate::showcase::screen_text("  \n"), "  \n");
    // the demo world's own screens show as they are by default
    with("demo", cfg(&[]));
    assert_eq!(crate::showcase::screen_text("⏺ hello"), "⏺ hello");
    done();
}

#[test]
fn the_clipboard_three_ways() {
    with("masked", cfg(&["showcase.screens=redact"]));
    let real = "ssh 172.16.200.11 · ⏺ hello";
    let masked = crate::showcase::clip(real, false).unwrap();
    assert!(
        !masked.contains("192.168") && masked.contains("hello"),
        "{masked}"
    );
    // a screen goes as the screen shows it
    let screen = crate::showcase::clip("⏺ hello there", true).unwrap();
    assert!(no_words(&screen), "{screen}");
    assert!(crate::showcase::clip_refused().is_none());
    with("masked", cfg(&["showcase.clipboard=off"]));
    assert!(crate::showcase::clip(real, false).is_none());
    assert_eq!(crate::showcase::clip_refused(), Some(CLIP_OFF));
    with("masked", cfg(&["showcase.clipboard=real"]));
    assert_eq!(crate::showcase::clip(real, true).unwrap(), real);
    // showcase off: always the real text
    with("off", cfg(&["showcase.clipboard=off"]));
    assert_eq!(crate::showcase::clip(real, false).unwrap(), real);
    done();
}

#[test]
fn money_scaled_adds_up_and_hidden_shows_nothing() {
    with(
        "masked",
        cfg(&["showcase.money=scaled", "showcase.money_scale=0.37"]),
    );
    assert_eq!(crate::showcase::money(100.0), Some(37.0));
    let rows = [12.5, 30.25, 57.25];
    let total: f64 = rows.iter().sum();
    let scaled: f64 = rows
        .iter()
        .map(|r| crate::showcase::money(*r).unwrap())
        .sum();
    assert!((scaled - crate::showcase::money(total).unwrap()).abs() < 1e-9);
    // JSON: every dollar field the same way, everything else as it was
    let v = serde_json::json!({
        "total": {"cost": 100.0, "tokens": 123456, "cost_reported": 60.0, "cost_estimated": 40.0},
        "accounts": [{"plan": "max", "billed": {"kind": "plan", "monthly": 200.0}, "windows": [{"used": 42.0}]}],
    });
    let m = crate::showcase::json(v);
    assert_eq!(m["total"]["cost"], 37.0);
    assert_eq!(m["total"]["cost_reported"], 22.2);
    assert_eq!(m["total"]["cost_estimated"], 14.8);
    assert_eq!(m["total"]["tokens"], 123456);
    assert_eq!(m["accounts"][0]["billed"]["monthly"], 74.0);
    assert_eq!(m["accounts"][0]["windows"][0]["used"], 42.0);
    // the dashboard's and the CLI's formatters
    assert_eq!(crate::usage::dash::money(100.0, false), "$37.00");
    with("masked", cfg(&["showcase.money=hidden"]));
    assert_eq!(crate::showcase::money(5.0), None);
    assert_eq!(
        crate::usage::dash::money(5.0, true),
        crate::showcase::MONEY_HIDDEN
    );
    let m = crate::showcase::json(serde_json::json!({"cost": 5.5, "tokens": 9}));
    assert_eq!(m["cost"], 0);
    assert_eq!(m["tokens"], 9);
    // showcase off: real money
    with("off", cfg(&["showcase.money=hidden"]));
    assert_eq!(crate::showcase::money(5.0), Some(5.0));
    done();
}

#[test]
fn paused_answers_carry_nothing_real() {
    let v = serde_json::json!({
        "hosts": [{"alias": "b31"}],
        "name": "claude_scan",
        "version": "2.1.0",
        "kind": "blocked",
        "count": 3,
        "load": 0.7,
        "reachable": true,
        "nothing": null,
        "showcase": "masked",
        "showcase_safety": {"paused": true, "money": "real"},
        "nested": {"cwd": "/home/selene/x", "list": [1, 2]},
    });
    let n = pause::neutral(v);
    assert_eq!(n["hosts"], serde_json::json!([]));
    assert_eq!(n["name"], "");
    assert_eq!(n["version"], "2.1.0");
    assert_eq!(n["kind"], "blocked");
    assert_eq!(n["count"], 0);
    assert_eq!(n["load"], 0.0);
    assert_eq!(n["reachable"], true);
    assert!(n["nothing"].is_null());
    assert_eq!(n["showcase"], "masked");
    assert_eq!(n["showcase_safety"]["money"], "real");
    assert_eq!(n["nested"]["cwd"], "");
    assert_eq!(n["nested"]["list"], serde_json::json!([]));
    // an error stays readable
    let e = pause::neutral(serde_json::json!({"error": {"code": "not_found", "message": "no x"}}));
    assert_eq!(e["error"]["code"], "not_found");
    // Aleph's own asking isn't blanked (a pid of 0 is everyone's)
    assert!(pause::internal("daemon.ping") && pause::internal("daemon.status"));
    assert!(!pause::internal("state.snapshot") && !pause::internal("usage.now"));
}

#[test]
fn the_web_console_follows_the_marker() {
    let js = String::from_utf8_lossy(crate::showcase::web::ASSETS[0].2);
    for w in [
        "showcase_safety",
        "be right back",
        "s.money === 'hidden'",
        "s.clipboard === 'off'",
        "route()",
    ] {
        assert!(js.contains(w), "{w}");
    }
    assert!(!js.contains("innerHTML"), "text only, never markup");
    let css = String::from_utf8_lossy(crate::showcase::web::ASSETS[1].2);
    assert!(css.contains(".sc-pause") && css.contains("prefers-reduced-motion"));
}

fn paused(on: bool) {
    paused_in(super::super::state::Mode::Off, on);
}

fn paused_in(mode: super::super::state::Mode, on: bool) {
    super::super::state::force(Some(super::super::state::State {
        mode,
        epoch: 7,
        panic: on,
        salt: b"0123456789abcdef0123456789abcdef".to_vec(),
        ..Default::default()
    }));
}

#[test]
fn nothing_lands_on_the_pause_after_the_lens() {
    // showcase on, a key just pressed (the keycast is laid in after the
    // lens), a celebration's turn, the watermark's: the stars win
    paused_in(super::super::state::Mode::Masked, true);
    force(Some(cfg(&[])));
    let c = Config::in_memory(&["showcase.watermark=made by me"]);
    crate::showcase::key(&c, "j", || Some(("down", "move down".into())));
    let mut t = Terminal::new(TestBackend::new(80, 20)).unwrap();
    t.draw(|f| {
        let b = f.buffer_mut();
        b.set_string(0, 1, "─".repeat(80), Style::new());
        b.set_string(2, 5, "b31 · claude_scan", Style::new());
        crate::showcase::buffer(b);
        crate::showcase::celebrations(b);
        crate::showcase::finish(&c, b, "Fleet");
    })
    .unwrap();
    let b = t.backend().buffer().clone();
    let all: String = (0..20).map(|y| row(&b, y)).collect();
    assert!(!all.contains("made by me") && !all.contains("down") && !all.contains("b31"));
    assert!(all.contains("be right back"));
    done();
}

#[test]
fn the_hud_reads_money_as_the_rest_does() {
    let g = crate::showcase::Gauges {
        up: 2,
        machines: 2,
        working: 1,
        needs: 0,
    };
    let text =
        |s: Vec<ratatui::text::Span>| s.iter().map(|x| x.content.to_string()).collect::<String>();
    with(
        "masked",
        cfg(&["showcase.money=scaled", "showcase.money_scale=0.5"]),
    );
    let l = text(crate::showcase::hud::line(&g, Some((40.0, false)), 0, 0));
    assert!(l.contains("$20.00 today"), "{l}");
    with("masked", cfg(&["showcase.money=hidden"]));
    let l = text(crate::showcase::hud::line(&g, Some((40.0, false)), 0, 0));
    assert!(l.contains("$— today") && !l.contains("40"), "{l}");
    done();
}

#[test]
fn the_pause_blanks_the_frame_and_every_answer() {
    paused(true);
    force(Some(cfg(&[])));
    assert!(crate::showcase::paused());
    let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
    t.draw(|f| {
        let b = f.buffer_mut();
        b.set_string(
            2,
            2,
            "b31 · claude_scan · working",
            Style::new().bg(Color::Blue),
        );
        b.set_string(2, 8, "magrathea needs you", Style::new());
        crate::showcase::buffer(b);
    })
    .unwrap();
    let b = t.backend().buffer().clone();
    let all: String = (0..16).map(|y| row(&b, y)).collect();
    assert!(!all.contains("b31") && !all.contains("magrathea") && !all.contains("needs"));
    assert!(all.contains(crate::brand::MARK) && all.contains("be right back"));
    assert!(all.contains("F12 brings it back"), "{all}");
    let stars = all
        .chars()
        .filter(|c| ['·', '✦', '˚', '⋆', '∙'].contains(c))
        .count();
    assert!(stars > 5, "{stars} stars");
    assert!((0..16).all(|y| (0..60).all(|x| b[(x, y)].bg == Color::Reset)));
    // the daemon's answers, a screen, a copy
    let a = crate::showcase::json(serde_json::json!({"sessions": [{"name": "x"}], "n": 2}));
    assert_eq!(a, serde_json::json!({"sessions": [], "n": 0}));
    assert_eq!(crate::showcase::screen_text("⏺ hi"), "");
    assert!(crate::showcase::clip("x", false).is_none());
    assert!(crate::showcase::quiet_sink(false) && !crate::showcase::quiet_sink(true));
    // Aleph's own asking, a CLI's --json and a phone's push go on as before
    let pid = serde_json::json!({"pid": 42});
    assert_eq!(
        crate::showcase::answer("daemon.ping", pid.clone())["pid"],
        42
    );
    assert_eq!(
        crate::showcase::answer("state.snapshot", pid.clone())["pid"],
        0
    );
    assert_eq!(crate::showcase::masked(pid)["pid"], 42);
    // the marker says so, even with showcase off
    let mut me = serde_json::json!({"name": "phone"});
    crate::showcase::mark(&mut me);
    assert_eq!(me["showcase_safety"]["paused"], true);
    assert!(me.get("showcase").is_none());
    // every key waits; the panic key brings it back (not pressed here: it
    // would write the state file)
    assert!(crate::showcase::panic_key("j"));
    paused(false);
    assert!(!crate::showcase::panic_key("j"));
    let mut me = serde_json::json!({});
    crate::showcase::mark(&mut me);
    assert!(me.get("showcase_safety").is_none());
    done();
}

#[test]
fn keys_as_the_pause_names_them() {
    assert_eq!(pause::pretty("f12"), "F12");
    assert_eq!(pause::pretty("ctrl+4"), "⌃4");
    assert_eq!(pause::pretty("alt+p"), "⌥P");
    assert_eq!(pause::pretty("q"), "q");
}
