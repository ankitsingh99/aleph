//! What the demo world's panes show: Claude Code-, Codex-, OpenCode-,
//! Gemini- and Pi-like screens drawn from a session's script, and a shell.
//! Lines are ANSI (truecolor); `plain` strips them for herdr's plain reads.

use super::world::{Machine, Phase, Sess, Step, K, SHELL_CMDS};
use unicode_width::UnicodeWidthStr;

type Rgb = (u8, u8, u8);

const ORANGE: Rgb = (215, 119, 87);
const GREY: Rgb = (136, 136, 136);
const GREEN: Rgb = (78, 186, 101);
const RED: Rgb = (255, 107, 128);
const BLUE: Rgb = (177, 185, 249);
const CYAN: Rgb = (86, 182, 194);
const MAGENTA: Rgb = (198, 120, 221);
const YELLOW: Rgb = (229, 192, 123);
const PEACH: Rgb = (250, 178, 131);
const GEM: Rgb = (71, 150, 228);
const TEAL: Rgb = (94, 205, 180);

fn fg(c: Rgb, s: &str) -> String {
    format!("\x1b[38;2;{};{};{}m{s}\x1b[0m", c.0, c.1, c.2)
}
fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[0m")
}
fn dim(s: &str) -> String {
    format!("\x1b[2m{s}\x1b[0m")
}

/// The text without its escape sequences.
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn vis(s: &str) -> usize {
    UnicodeWidthStr::width(plain(s).as_str())
}

/// Fit to `w` columns (an over-long line loses its colours and gets `…`).
fn clip(s: &str, w: usize) -> String {
    if vis(s) <= w {
        return s.to_string();
    }
    crate::ui::trunc(&plain(s), w)
}

fn pad(s: &str, w: usize) -> String {
    let s = clip(s, w);
    let n = w.saturating_sub(vis(&s));
    format!("{s}{}", " ".repeat(n))
}

/// A rounded box `w` wide around `inner`.
fn boxed(w: usize, inner: &[String], c: Rgb) -> Vec<String> {
    let iw = w.saturating_sub(4);
    let mut v = vec![fg(c, &format!("╭{}╮", "─".repeat(w.saturating_sub(2))))];
    for l in inner {
        v.push(format!("{} {} {}", fg(c, "│"), pad(l, iw), fg(c, "│")));
    }
    v.push(fg(c, &format!("╰{}╯", "─".repeat(w.saturating_sub(2)))));
    v
}

/// Wrap plain text to `w` columns.
fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty()
            && UnicodeWidthStr::width(cur.as_str()) + 1 + UnicodeWidthStr::width(word) > w
        {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn dur(s: u64) -> String {
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

fn kilo(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn tilde(m: &Machine, p: &str) -> String {
    let h = m.home();
    match p.strip_prefix(&h) {
        Some(r) => format!("~{r}"),
        None => p.to_string(),
    }
}

/// How far the steps have got: (steps done before the question, the
/// question's command ran, steps after it).
fn progress(s: &Sess) -> (usize, bool, usize) {
    let n = s.task.steps.len();
    let pre = n.saturating_sub(1);
    let frac = |a: &super::world::At| {
        if a.len == 0 {
            1.0
        } else {
            (a.into as f64 / a.len as f64).clamp(0.0, 1.0)
        }
    };
    match s.at.phase {
        Phase::Work => ((1 + (frac(&s.at) * pre as f64) as usize).min(pre), false, 0),
        Phase::Ask => (pre, false, 0),
        Phase::After => (pre, true, usize::from(frac(&s.at) > 0.5) * (n - pre)),
        _ => (pre, true, n - pre),
    }
}

/// Spinner frames, by the clock.
fn spin(frames: &[&'static str], ms: u64) -> &'static str {
    frames[(ms / 120) as usize % frames.len()]
}

/// The session's screen as ANSI lines, `cols` wide.
pub fn lines(s: &Sess, m: &Machine, ms: u64, cols: u16) -> Vec<String> {
    let w = (cols as usize).clamp(40, 110);
    let v = match s.agent.as_str() {
        "shell" => shell(s, m, ms),
        "codex" => codex(s, m, ms, w),
        "opencode" => opencode(s, m, ms, w),
        "gemini" => gemini(s, m, ms, w),
        "pi" => pi(s, m, ms, w),
        _ => claude(s, m, ms, w),
    };
    v.into_iter().map(|l| clip(&l, cols as usize)).collect()
}

/// What herdr's detection reads while it's blocked: the question and its
/// numbered options (approve_keys finds the yes there).
pub fn detection(s: &Sess) -> String {
    if s.at.phase != Phase::Ask {
        return String::new();
    }
    let a = &s.task.ask;
    match s.agent.as_str() {
        "codex" => format!(
            "Would you like to run the following command?\n\n  $ {}\n\n› 1. Yes, proceed (y)\n  2. Yes, and don't ask again for this command (a)\n  3. No, and tell Codex what to do differently (esc)\n",
            a.cmd
        ),
        "opencode" => format!(
            "△ Permission required\n  $ {}\n\n  1. Allow once\n  2. Allow always\n  3. Reject\n",
            a.cmd
        ),
        "gemini" => format!(
            "? Shell {}\n\nAllow execution of: '{}'?\n\n❯ 1. Yes, allow once\n  2. Yes, allow always ...\n  3. No, suggest changes (esc)\n",
            a.cmd,
            a.cmd.split_whitespace().next().unwrap_or("")
        ),
        "pi" => format!("Run this command?\n  {}\n\n❯ 1. Yes\n  2. No\n", a.cmd),
        _ => format!(
            " Bash command\n\n   {}\n   {}\n\n Do you want to proceed?\n ❯ 1. Yes\n   2. Yes, and don't ask again for {} commands\n   3. No, and tell Claude what to do differently (esc)\n",
            a.cmd, a.why, a.family
        ),
    }
}

/// Tokens it has used this turn (they climb as it works).
fn tokens(s: &Sess) -> u64 {
    let h = super::world::hash(&[&s.pane, "tok"]);
    200 + s.at.into * (28 + h % 30) + h % 900
}

/// How long the whole turn took (for "Worked for …").
fn worked(s: &Sess) -> u64 {
    let h = super::world::hash(&[&s.pane, "worked"]);
    240 + h % 300
}

// ------------------------------------------------------------------ Claude Code

fn claude_step(st: &Step) -> Vec<String> {
    let dot = fg(GREEN, "⏺");
    let head = match st.k {
        K::Read => format!("{dot} {}({})", bold("Read"), st.arg),
        K::Edit => format!("{dot} {}({})", bold("Update"), st.arg),
        K::Run => format!("{dot} {}({})", bold("Bash"), st.arg),
        K::Find => format!("{dot} {}(pattern: \"{}\")", bold("Search"), st.arg),
    };
    vec![
        head,
        format!("  {}  {}", dim("⎿"), dim(st.out)),
        String::new(),
    ]
}

fn claude(s: &Sess, m: &Machine, ms: u64, w: usize) -> Vec<String> {
    let mut v = boxed(
        w.min(64),
        &[
            format!("{} Welcome to {}!", fg(ORANGE, "✻"), bold("Claude Code")),
            String::new(),
            dim("  /help for help, /status for your current setup"),
            String::new(),
            dim(&format!("  cwd: {}", tilde(m, &s.cwd))),
        ],
        ORANGE,
    );
    v.push(String::new());
    for (i, l) in wrap(s.task.prompt, w.saturating_sub(4)).iter().enumerate() {
        v.push(dim(&format!("{} {l}", if i == 0 { ">" } else { " " })));
    }
    v.push(String::new());
    let (pre, ran, post) = progress(s);
    v.push(format!(
        "{} I'll start with {}.",
        fg(BLUE, "⏺"),
        s.task.steps.first().map_or("the code", |x| x.arg)
    ));
    v.push(String::new());
    for st in &s.task.steps[..pre] {
        v.extend(claude_step(st));
    }
    let a = &s.task.ask;
    if ran {
        v.push(format!("{} {}({})", fg(GREEN, "⏺"), bold("Bash"), a.cmd));
        v.push(format!("  {}  {}", dim("⎿"), dim(a.out)));
        v.push(String::new());
    }
    for st in &s.task.steps[pre..pre + post] {
        v.extend(claude_step(st));
    }
    let input = |v: &mut Vec<String>| {
        v.extend(boxed(w, &[">".to_string()], GREY));
        v.push(dim("  ⏵⏵ accept edits on (shift+tab to cycle)"));
    };
    match s.at.phase {
        Phase::Work | Phase::After => {
            v.push(format!(
                "{} {}… {}",
                fg(
                    ORANGE,
                    spin(&["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"], ms)
                ),
                fg(ORANGE, s.task.verb),
                dim(&format!(
                    "({} · ↑ {} tokens · esc to interrupt)",
                    dur(s.at.into),
                    kilo(tokens(s))
                ))
            ));
            v.push(String::new());
            input(&mut v);
        }
        Phase::Ask => {
            let mut inner = vec![bold("Bash command"), String::new()];
            inner.push(format!("  {}", a.cmd));
            inner.push(dim(&format!("  {}", a.why)));
            inner.push(String::new());
            inner.push("Do you want to proceed?".into());
            inner.push(format!("{} {}", fg(BLUE, "❯"), fg(BLUE, "1. Yes")));
            inner.push(format!(
                "  2. Yes, and don't ask again for {} commands",
                bold(a.family)
            ));
            inner.push("  3. No, and tell Claude what to do differently (esc)".into());
            v.extend(boxed(w, &inner, BLUE));
        }
        Phase::Prompted | Phase::Replied => {
            v.push(format!("{} {}", fg(BLUE, "⏺"), s.task.done));
            v.push(String::new());
            v.push(dim(&format!("> {}", s.prompt)));
            v.push(String::new());
            if s.at.phase == Phase::Prompted {
                v.push(format!(
                    "{} {} {}",
                    fg(ORANGE, spin(&["·", "✢", "✳", "✶", "✻", "✽"], ms)),
                    fg(ORANGE, "Thinking…"),
                    dim(&format!("({} · esc to interrupt)", dur(s.at.into)))
                ));
            } else {
                v.push(format!(
                    "{} On it — done. The change is in place and the tests pass.",
                    fg(BLUE, "⏺")
                ));
                v.push(String::new());
                v.push(dim(&format!(
                    "✻ Worked for {}",
                    dur(super::world::PROMPT_WORK)
                )));
            }
            v.push(String::new());
            input(&mut v);
        }
        _ => {
            for l in wrap(s.task.done, w.saturating_sub(2)).iter().enumerate() {
                v.push(if l.0 == 0 {
                    format!("{} {}", fg(BLUE, "⏺"), l.1)
                } else {
                    format!("  {}", l.1)
                });
            }
            v.push(String::new());
            v.push(dim(&format!("✻ Worked for {}", dur(worked(s)))));
            v.push(String::new());
            input(&mut v);
        }
    }
    v
}

// ------------------------------------------------------------------ Codex

fn codex_step(st: &Step) -> Vec<String> {
    let dot = fg(GREY, "•");
    match st.k {
        K::Read => vec![
            format!("{dot} {}", bold("Explored")),
            format!("  └ Read {}", st.arg),
        ],
        K::Find => vec![
            format!("{dot} {}", bold("Explored")),
            format!("  └ Search {}", st.arg),
            dim(&format!("    {}", st.out)),
        ],
        K::Edit => vec![format!(
            "{dot} {} {} {}",
            bold("Edited"),
            st.arg,
            fg(GREEN, &format!("({})", st.out))
        )],
        K::Run => vec![
            format!("{dot} {} {}", bold("Ran"), st.arg),
            dim(&format!("  └ {}", st.out)),
        ],
    }
    .into_iter()
    .chain([String::new()])
    .collect()
}

fn codex(s: &Sess, m: &Machine, ms: u64, w: usize) -> Vec<String> {
    let mut v = boxed(
        w.min(60),
        &[
            format!(
                "{} {} {}",
                bold(">_"),
                bold("OpenAI Codex"),
                dim("(v0.58.0)")
            ),
            String::new(),
            format!(
                "{} gpt-5.1-codex high   {}",
                dim("model:    "),
                fg(CYAN, "/model to change")
            ),
            format!("{} {}", dim("directory:"), tilde(m, &s.cwd)),
        ],
        GREY,
    );
    v.push(String::new());
    for (i, l) in wrap(s.task.prompt, w.saturating_sub(4)).iter().enumerate() {
        v.push(format!(
            "{} {l}",
            if i == 0 { fg(CYAN, "›") } else { " ".into() }
        ));
    }
    v.push(String::new());
    let (pre, ran, post) = progress(s);
    for st in &s.task.steps[..pre] {
        v.extend(codex_step(st));
    }
    let a = &s.task.ask;
    if ran {
        v.push(format!("{} {} {}", fg(GREY, "•"), bold("Ran"), a.cmd));
        v.push(dim(&format!("  └ {}", a.out)));
        v.push(String::new());
    }
    for st in &s.task.steps[pre..pre + post] {
        v.extend(codex_step(st));
    }
    let foot = |v: &mut Vec<String>| {
        v.push(format!(
            "{} {}",
            fg(CYAN, "›"),
            dim("Ask Codex to do anything")
        ));
        v.push(String::new());
        v.push(dim(&format!(
            "  {}% context left · ? for shortcuts",
            96u64.saturating_sub(s.at.into / 20)
        )));
    };
    match s.at.phase {
        Phase::Work | Phase::After | Phase::Prompted => {
            v.push(format!(
                "{} {} {}",
                fg(CYAN, spin(&["◦", "•"], ms * 2)),
                bold("Working"),
                dim(&format!("({} • esc to interrupt)", dur(s.at.into)))
            ));
            v.push(String::new());
            foot(&mut v);
        }
        Phase::Ask => {
            v.push(bold("  Would you like to run the following command?"));
            v.push(String::new());
            v.push(format!("  $ {}", fg(YELLOW, a.cmd)));
            v.push(String::new());
            v.push(format!(
                "{} {}",
                fg(CYAN, "›"),
                fg(CYAN, "1. Yes, proceed (y)")
            ));
            v.push("  2. Yes, and don't ask again for this command (a)".into());
            v.push("  3. No, and tell Codex what to do differently (esc)".into());
            v.push(String::new());
            v.push(dim("  Press enter to confirm or esc to cancel"));
        }
        Phase::Limit => {
            v.push(format!("{} {}", fg(GREY, "•"), s.task.done));
            v.push(String::new());
            let left = s.limit_left.div_ceil(60).max(1);
            v.push(fg(
                RED,
                &format!(
                    "■ You've hit your usage limit. Upgrade to Pro or try again in {left} minute{}.",
                    if left == 1 { "" } else { "s" }
                ),
            ));
            v.push(String::new());
            foot(&mut v);
        }
        Phase::Replied => {
            v.push(format!(
                "{} Done — the change is in and `cargo test` passes.",
                fg(GREY, "•")
            ));
            v.push(String::new());
            foot(&mut v);
        }
        _ => {
            for (i, l) in wrap(s.task.done, w.saturating_sub(2)).iter().enumerate() {
                v.push(format!(
                    "{} {l}",
                    if i == 0 { fg(GREY, "•") } else { " ".into() }
                ));
            }
            v.push(String::new());
            foot(&mut v);
        }
    }
    v
}

// ------------------------------------------------------------------ OpenCode

fn opencode(s: &Sess, m: &Machine, ms: u64, w: usize) -> Vec<String> {
    let bar = fg(PEACH, "┃");
    let mut v = vec![
        format!(
            " {}  {}  {}",
            bold(&fg(PEACH, "opencode")),
            dim("v1.0.62"),
            dim(&tilde(m, &s.cwd))
        ),
        String::new(),
    ];
    for l in wrap(s.task.prompt, w.saturating_sub(6)) {
        v.push(format!(" {bar} {l}"));
    }
    v.push(String::new());
    let (pre, ran, post) = progress(s);
    let step = |st: &Step| -> Vec<String> {
        let (g, verb) = match st.k {
            K::Read => ("⚙", "read"),
            K::Edit => ("✎", "write"),
            K::Run => ("$", "bash"),
            K::Find => ("⌕", "grep"),
        };
        vec![
            format!(
                " {} {} {} {}",
                fg(GREY, "┃"),
                fg(PEACH, g),
                dim(verb),
                st.arg
            ),
            format!(" {}   {}", fg(GREY, "┃"), dim(st.out)),
        ]
    };
    for st in &s.task.steps[..pre] {
        v.extend(step(st));
    }
    let a = &s.task.ask;
    if ran {
        v.push(format!(
            " {} {} {} {}",
            fg(GREY, "┃"),
            fg(PEACH, "$"),
            dim("bash"),
            a.cmd
        ));
        v.push(format!(" {}   {}", fg(GREY, "┃"), dim(a.out)));
    }
    for st in &s.task.steps[pre..pre + post] {
        v.extend(step(st));
    }
    v.push(String::new());
    match s.at.phase {
        Phase::Work | Phase::After | Phase::Prompted => v.push(format!(
            " {} {}  {}",
            fg(PEACH, spin(&["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"], ms)),
            "Working…",
            dim(&format!(
                "claude-sonnet-4-5 · {} tokens · esc interrupt",
                kilo(tokens(s))
            ))
        )),
        Phase::Ask => {
            v.push(format!(
                " {} {}",
                fg(YELLOW, "△"),
                bold("Permission required")
            ));
            v.push(format!("   $ {}", a.cmd));
            v.push(String::new());
            v.push(format!("   {}", fg(PEACH, "1. Allow once")));
            v.push("   2. Allow always".into());
            v.push("   3. Reject".into());
        }
        _ => {
            for l in wrap(s.task.done, w.saturating_sub(4)) {
                v.push(format!(" {l}"));
            }
        }
    }
    v.push(String::new());
    v.push(format!(" {bar} {}", dim(">")));
    v.push(dim(&format!(
        "   enter send   ctrl+x h help{}Build  claude-sonnet-4-5",
        " ".repeat(w.saturating_sub(62).max(2))
    )));
    v
}

// ------------------------------------------------------------------ Gemini CLI

fn gemini(s: &Sess, m: &Machine, ms: u64, w: usize) -> Vec<String> {
    let mut v = vec![
        format!("{} {}", fg(GEM, "✦"), bold(&fg(GEM, "Gemini CLI"))),
        dim("  Tips: ask questions, edit files or run commands · /help for more"),
        String::new(),
    ];
    for (i, l) in wrap(s.task.prompt, w.saturating_sub(4)).iter().enumerate() {
        v.push(format!(
            "{} {l}",
            if i == 0 { fg(MAGENTA, ">") } else { " ".into() }
        ));
    }
    v.push(String::new());
    let (pre, ran, post) = progress(s);
    let step = |st: &Step| -> String {
        let t = match st.k {
            K::Read => "ReadFile",
            K::Edit => "WriteFile",
            K::Run => "Shell",
            K::Find => "SearchText",
        };
        format!(
            " {} {} {}  {}",
            fg(GREEN, "✓"),
            bold(t),
            st.arg,
            dim(st.out)
        )
    };
    for st in &s.task.steps[..pre] {
        v.push(step(st));
    }
    let a = &s.task.ask;
    if ran {
        v.push(format!(
            " {} {} {}  {}",
            fg(GREEN, "✓"),
            bold("Shell"),
            a.cmd,
            dim(a.out)
        ));
    }
    for st in &s.task.steps[pre..pre + post] {
        v.push(step(st));
    }
    v.push(String::new());
    match s.at.phase {
        Phase::Work | Phase::After | Phase::Prompted => v.push(format!(
            "{} {} {}",
            fg(
                GEM,
                spin(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"], ms)
            ),
            s.task.title,
            dim(&format!("(esc to cancel, {})", dur(s.at.into)))
        )),
        Phase::Ask => {
            let inner = vec![
                format!("{} {} {}", fg(YELLOW, "?"), bold("Shell"), a.cmd),
                String::new(),
                format!(
                    "Allow execution of: '{}'?",
                    a.cmd.split_whitespace().next().unwrap_or("")
                ),
                String::new(),
                format!("{} {}", fg(GREEN, "❯"), fg(GREEN, "1. Yes, allow once")),
                "  2. Yes, allow always ...".into(),
                "  3. No, suggest changes (esc)".into(),
            ];
            v.extend(boxed(w, &inner, YELLOW));
        }
        _ => {
            for (i, l) in wrap(s.task.done, w.saturating_sub(2)).iter().enumerate() {
                v.push(format!(
                    "{} {l}",
                    if i == 0 { fg(GEM, "✦") } else { " ".into() }
                ));
            }
        }
    }
    v.push(String::new());
    v.extend(boxed(
        w,
        &[format!(
            "{}   {}",
            fg(MAGENTA, ">"),
            dim("Type your message or @path/to/file")
        )],
        GREY,
    ));
    v.push(dim(&format!(
        "{} (main*)      no sandbox      gemini-2.5-pro ({}% context left)",
        tilde(m, &s.cwd),
        97u64.saturating_sub(s.at.into / 30)
    )));
    v
}

// ------------------------------------------------------------------ Pi

fn pi(s: &Sess, m: &Machine, ms: u64, w: usize) -> Vec<String> {
    let mut v = vec![
        format!(
            " {} {} {} {}",
            bold(&fg(TEAL, "π")),
            dim("pi v0.30.2 ·"),
            "claude-sonnet-4-5",
            dim(&format!("· {}", tilde(m, &s.cwd)))
        ),
        String::new(),
    ];
    for (i, l) in wrap(s.task.prompt, w.saturating_sub(7)).iter().enumerate() {
        v.push(if i == 0 {
            format!(" {} {l}", bold("You:"))
        } else {
            format!("      {l}")
        });
    }
    v.push(String::new());
    let (pre, ran, post) = progress(s);
    let step = |st: &Step| -> Vec<String> {
        let t = match st.k {
            K::Read => "read",
            K::Edit => "edit",
            K::Run => "bash",
            K::Find => "grep",
        };
        vec![
            format!(" {} {} {}", fg(TEAL, "▸"), bold(t), st.arg),
            dim(&format!("   {}", st.out)),
        ]
    };
    for st in &s.task.steps[..pre] {
        v.extend(step(st));
    }
    let a = &s.task.ask;
    if ran {
        v.push(format!(" {} {} {}", fg(TEAL, "▸"), bold("bash"), a.cmd));
        v.push(dim(&format!("   {}", a.out)));
    }
    for st in &s.task.steps[pre..pre + post] {
        v.extend(step(st));
    }
    v.push(String::new());
    match s.at.phase {
        Phase::Work | Phase::After | Phase::Prompted => v.push(format!(
            " {} {}… {}",
            fg(
                TEAL,
                spin(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"], ms)
            ),
            s.task.title,
            dim(&dur(s.at.into))
        )),
        Phase::Ask => {
            v.push(bold(" Run this command?"));
            v.push(format!("   {}", fg(YELLOW, a.cmd)));
            v.push(String::new());
            v.push(format!(" {} {}", fg(TEAL, "❯"), fg(TEAL, "1. Yes")));
            v.push("   2. No".into());
        }
        _ => {
            for l in wrap(s.task.done, w.saturating_sub(2)) {
                v.push(format!(" {l}"));
            }
        }
    }
    v.push(String::new());
    v.push(format!(" {}", fg(TEAL, ">")));
    let t = tokens(s);
    v.push(dim(&format!(
        " ↑{} ↓{}  ${:.2}  {}% of 200k",
        kilo(t),
        kilo(t / 4),
        t as f64 * 0.000_009,
        (20 + t / 1500).min(95)
    )));
    v
}

// ------------------------------------------------------------------ a shell

fn shell(s: &Sess, m: &Machine, _ms: u64) -> Vec<String> {
    let dir = s.cwd.rsplit('/').next().unwrap_or("~");
    let prompt = if m.mac() {
        format!("{}@{} {} % ", m.user, m.alias, fg(CYAN, dir))
    } else {
        format!(
            "{}:{}$ ",
            fg(GREEN, &format!("{}@{}", m.user, m.alias)),
            fg(BLUE, &tilde(m, &s.cwd))
        )
    };
    // a command every 40 s, the screen cleared after the last
    let n = SHELL_CMDS.len() as u64 + 1;
    let k = (s.at.into / 40) % n;
    let mut v = Vec::new();
    for c in &SHELL_CMDS[..k as usize] {
        v.push(format!("{prompt}{}", c.cmd));
        v.extend(c.out.iter().map(|l| l.to_string()));
    }
    v.push(prompt);
    v
}
