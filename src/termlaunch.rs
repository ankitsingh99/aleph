//! Run a command in a new tab (or window) of the terminal you're using —
//! Warp, iTerm2, Terminal.app, Ghostty, kitty or WezTerm — so Aleph can open
//! a session in herdr next to itself instead of handing its own screen over.
//!
//! Which terminal: `ALEPH_TAB_CMD` (a program run as `<prog> <title>
//! <command line>`: a terminal Aleph doesn't know, and the fake world's
//! stand-in so tests never open a real one), `ALEPH_TERMINAL` / the
//! `terminal` setting, else the one this process runs in (`TERM_PROGRAM`,
//! kitty/WezTerm/Ghostty variables), else Terminal.app. `aleph open-tab --
//! <command…>` does the same from a shell (and the Raycast extension uses it).

use crate::proc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum App {
    Warp,
    Iterm,
    Terminal,
    Ghostty,
    Kitty,
    WezTerm,
    /// `ALEPH_TAB_CMD`
    Command,
}

impl App {
    pub fn name(self) -> &'static str {
        match self {
            App::Warp => "warp",
            App::Iterm => "iterm",
            App::Terminal => "terminal",
            App::Ghostty => "ghostty",
            App::Kitty => "kitty",
            App::WezTerm => "wezterm",
            App::Command => "ALEPH_TAB_CMD",
        }
    }
    /// How it reads in a sentence (`a new Warp tab`).
    pub fn title(self) -> &'static str {
        match self {
            App::Warp => "Warp",
            App::Iterm => "iTerm2",
            App::Terminal => "Terminal",
            App::Ghostty => "Ghostty",
            App::Kitty => "kitty",
            App::WezTerm => "WezTerm",
            App::Command => "ALEPH_TAB_CMD",
        }
    }
    pub fn parse(s: &str) -> Option<App> {
        match s.to_lowercase().as_str() {
            "warp" | "warpterminal" => Some(App::Warp),
            "iterm" | "iterm2" | "iterm.app" => Some(App::Iterm),
            "terminal" | "apple_terminal" | "terminal.app" => Some(App::Terminal),
            "ghostty" => Some(App::Ghostty),
            "kitty" | "xterm-kitty" => Some(App::Kitty),
            "wezterm" => Some(App::WezTerm),
            _ => None,
        }
    }
}

/// `ALEPH_TAB_CMD`, when it's set.
fn tab_cmd() -> Option<String> {
    crate::brand::env("TAB_CMD").filter(|v| !v.trim().is_empty())
}

/// The terminal to open tabs in.
pub fn which(setting: &str) -> App {
    if tab_cmd().is_some() {
        return App::Command;
    }
    if let Some(t) = crate::brand::env("TERMINAL").and_then(|v| App::parse(&v)) {
        return t;
    }
    if let Some(t) = App::parse(setting) {
        return t;
    }
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return App::Kitty;
    }
    if std::env::var_os("WEZTERM_PANE").is_some() {
        return App::WezTerm;
    }
    if std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some() {
        return App::Ghostty;
    }
    std::env::var("TERM_PROGRAM")
        .ok()
        .and_then(|p| App::parse(&p.replace(".app", "")))
        .unwrap_or(App::Terminal)
}

fn apple(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The process to start for `cmd` (a shell command line) in `t`.
/// Pure (tested): (program, args, optional file to write first).
pub fn plan(
    t: App,
    cmd: &str,
    title: &str,
) -> (String, Vec<String>, Option<(std::path::PathBuf, String)>) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    match t {
        App::Warp => {
            let dir = crate::config::home().join(".warp/tab_configs");
            let body = format!(
                "{} — rewritten on every launch\nname = {}\n\n[[panes]]\nid = \"main\"\ntype = \"terminal\"\ncommands = [{}]\nis_focused = true\n",
                crate::brand::MANAGED,
                toml(title),
                toml(cmd)
            );
            (
                "/usr/bin/open".into(),
                vec!["warp://tab_config/aleph_tab".into()],
                Some((dir.join("aleph_tab.toml"), body)),
            )
        }
        App::Iterm => (
            "/usr/bin/osascript".into(),
            vec![
                "-e".into(),
                format!(
                    "tell application \"iTerm\"\nactivate\nif (count of windows) = 0 then\ncreate window with default profile\nelse\ntell current window to create tab with default profile\nend if\ntell current session of current window to write text {}\nend tell",
                    apple(cmd)
                ),
            ],
            None,
        ),
        App::Terminal => (
            "/usr/bin/osascript".into(),
            vec![
                "-e".into(),
                format!("tell application \"Terminal\"\nactivate\ndo script {}\nend tell", apple(cmd)),
            ],
            None,
        ),
        App::Ghostty => (
            "/usr/bin/open".into(),
            vec![
                "-na".into(),
                "Ghostty".into(),
                "--args".into(),
                "-e".into(),
                shell.clone(),
                "-lc".into(),
                format!("{cmd}; exec {shell} -l"),
            ],
            None,
        ),
        App::Kitty => (
            "kitty".into(),
            vec![
                "@".into(),
                "launch".into(),
                "--type=tab".into(),
                "--tab-title".into(),
                title.into(),
                shell.clone(),
                "-lc".into(),
                format!("{cmd}; exec {shell} -l"),
            ],
            None,
        ),
        App::WezTerm => (
            "wezterm".into(),
            vec![
                "cli".into(),
                "spawn".into(),
                "--".into(),
                shell.clone(),
                "-lc".into(),
                format!("{cmd}; exec {shell} -l"),
            ],
            None,
        ),
        App::Command => (
            tab_cmd().unwrap_or_default(),
            vec![title.into(), cmd.into()],
            None,
        ),
    }
}

/// A shell line that runs this aleph with `args`, with the same config
/// (`ALEPH_CONFIG` carried over: a new tab doesn't inherit our environment).
pub fn self_cmd(args: &[&str]) -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| crate::brand::BIN.into());
    let mut s = String::new();
    if let Some(c) = std::env::var_os("ALEPH_CONFIG") {
        s.push_str(&format!(
            "ALEPH_CONFIG={} ",
            proc::sh_quote(&c.to_string_lossy())
        ));
    }
    s.push_str(&proc::sh_quote(&exe));
    for a in args {
        s.push(' ');
        s.push_str(&proc::sh_quote(a));
    }
    s
}

/// What a new tab runs to attach to one session: Aleph's own `resume`
/// (launch card, the right tab selected, then herdr), with the same config.
pub fn resume_cmd(host: &str, pane: &str) -> String {
    self_cmd(&["--host", host, "resume", pane])
}

/// Open `cmd` in a new tab. Err says why it couldn't.
pub fn open(setting: &str, cmd: &str, title: &str) -> Result<App, String> {
    // unit tests never open a real terminal (the fake world's ALEPH_TAB_CMD may)
    if cfg!(test) && tab_cmd().is_none() {
        return Err("tests don't open terminals".into());
    }
    let t = which(setting);
    let (prog, args, file) = plan(t, cmd, title);
    if let Some((p, body)) = file {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&p, body).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let o = proc::cap(&prog, &args, Duration::from_secs(15));
    if o.ok() {
        Ok(t)
    } else {
        Err(format!("{}: {}", t.name(), o.reason()))
    }
}

/// `aleph open-tab [--terminal T] [--dry-run] -- <command…>`: one word after
/// `--` is a shell command line as it is (`-- "cd x && make"`); several are
/// quoted one by one.
pub fn cmd(a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::*;
    let mut setting = a.cfg.get_or("terminal");
    let mut named: Option<App> = None;
    let mut dry = false;
    let mut i = 0;
    let mut words: Vec<String> = Vec::new();
    while i < a.rest.len() {
        match a.rest[i].as_str() {
            "--terminal" => {
                i += 1;
                setting = a.rest.get(i).cloned().unwrap_or_default();
                named = App::parse(&setting);
            }
            "--dry-run" => dry = true,
            "--" => {
                words.extend(a.rest[i + 1..].iter().cloned());
                break;
            }
            w => words.push(w.to_string()),
        }
        i += 1;
    }
    if words.is_empty() {
        return fail("usage: aleph open-tab [--terminal warp|iterm|terminal|ghostty|kitty|wezterm] [--dry-run] -- <command…>");
    }
    let line = if words.len() == 1 {
        words[0].clone()
    } else {
        let q: Vec<String> = words.iter().map(|w| proc::sh_quote(w)).collect();
        q.join(" ")
    };
    if dry {
        // a dry run with --terminal shows that terminal's plan (nothing runs)
        let t = named.unwrap_or_else(|| which(&setting));
        let (prog, args, file) = plan(t, &line, "Aleph");
        let q: Vec<String> = args.iter().map(|w| proc::sh_quote(w)).collect();
        println!("terminal\t{}", t.name());
        println!("run\t{} {}", proc::sh_quote(&prog), q.join(" "));
        if let Some((p, body)) = file {
            println!("writes\t{}", p.display());
            print!("{body}");
        }
        return std::process::ExitCode::SUCCESS;
    }
    match open(&setting, &line, "Aleph") {
        Ok(t) => {
            ok(&format!("opened in {}", t.name()));
            std::process::ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_per_terminal() {
        let (p, a, f) = plan(App::Iterm, "herdr --remote b31", "x");
        assert!(p.ends_with("osascript") && a[1].contains("write text \"herdr --remote b31\""));
        assert!(f.is_none());
        let (p, _, f) = plan(App::Warp, "say \"hi\"", "t");
        assert!(p.ends_with("open"));
        let (path, body) = f.unwrap();
        assert!(path.ends_with(".warp/tab_configs/aleph_tab.toml"));
        assert!(body.contains("commands = [\"say \\\"hi\\\"\"]"), "{body}");
        let (p, a, _) = plan(App::Kitty, "ls", "Aleph");
        assert_eq!((p.as_str(), a[1].as_str()), ("kitty", "launch"));
        assert_eq!(App::parse("iTerm.app"), Some(App::Iterm));
        assert_eq!(App::parse("Apple_Terminal"), Some(App::Terminal));
    }

    #[test]
    fn self_commands_are_quoted() {
        let s = self_cmd(&["host", "setup", "my box", "--tab"]);
        assert!(s.ends_with(" host setup 'my box' --tab"), "{s}");
        assert!(resume_cmd("b31", "w1:p3").ends_with(" --host b31 resume w1:p3"));
    }
}
