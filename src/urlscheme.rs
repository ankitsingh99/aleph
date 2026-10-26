//! `aleph://` links, for Shortcuts, Stream Deck, scripts and notes:
//!
//! | link | does |
//! |---|---|
//! | `aleph://open[?view=fleet\|monitor\|inbox\|settings]` | the dashboard, in a new tab |
//! | `aleph://open?workspace=gpu-watch` | a saved workspace, in a new tab |
//! | `aleph://focus?session=host:name` | one session in Focus, in a new tab |
//! | `aleph://new?host=H&dir=D&agent=A&prompt=P` | starts a session (asks first) |
//! | `aleph://send?session=host:name&text=T` | prompts a session (asks) |
//! | `aleph://approve?session=host:name` | answers its question yes (asks) |
//! | `aleph://close?session=host:name` | closes a session (asks) |
//! | `aleph://queue?text=T[&host=H][&dir=D]` | queues a task (asks) |
//!
//! Any web page or app can open a link, so the ones that act ask first, every
//! time: a dialog naming the session and what will happen, and for approve
//! the last lines of its screen. That's osascript on macOS, zenity or kdialog
//! on Linux; with neither, acting links are refused. Opening and focusing
//! only show things.
//!
//! `aleph url install` registers the scheme. On macOS a tiny applet made by
//! `osacompile` (`~/Applications/Aleph Links.app`) hands each link to
//! `aleph url <link>`; on Linux it's a .desktop file and xdg-mime. Nothing is
//! downloaded.

use crate::cli::Args;
use crate::config::Config;
use crate::out::*;
use crate::proc;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: aleph url <aleph://…> | install | uninstall | status | links";
/// The longest text a link may carry (a prompt), and a session's selector.
const MAX_TEXT: usize = 10_000;
const MAX_SEL: usize = 200;
const VIEWS: &[&str] = &["fleet", "monitor", "inbox", "settings"];

#[derive(Debug, PartialEq, Eq)]
pub enum Link {
    Open {
        view: Option<String>,
    },
    Focus {
        session: String,
    },
    New {
        host: Option<String>,
        dir: Option<String>,
        agent: Option<String>,
        prompt: Option<String>,
    },
    Send {
        session: String,
        text: String,
    },
    Approve {
        session: String,
    },
    Close {
        session: String,
    },
    Queue {
        text: String,
        host: Option<String>,
        dir: Option<String>,
    },
}

/// `%41` and `+` decoded (invalid escapes stay as they are).
fn decode(s: &str) -> String {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A link's action and its parameters (decoded).
fn split(url: &str) -> Result<(String, Vec<(String, String)>), String> {
    let rest = url
        .strip_prefix("aleph:")
        .ok_or_else(|| "not an aleph:// link".to_string())?
        .trim_start_matches('/');
    let (what, query) = rest.split_once('?').unwrap_or((rest, ""));
    let what = what.trim_end_matches('/').to_lowercase();
    let params = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k).to_lowercase(), decode(v))
        })
        .collect();
    Ok((what, params))
}

fn clean(v: &str, max: usize, what: &str) -> Result<String, String> {
    if v.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    // a prompt may have lines; nothing else in it may be a control character
    if v.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(format!("{what} has control characters"));
    }
    Ok(v.to_string())
}

pub fn parse(url: &str) -> Result<Link, String> {
    let (what, params) = split(url.trim())?;
    let get = |k: &str| {
        params
            .iter()
            .find(|(x, _)| x == k)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    };
    let need = |k: &str, max: usize| -> Result<String, String> {
        clean(
            get(k).ok_or_else(|| format!("aleph://{what} needs {k}="))?,
            max,
            k,
        )
    };
    let opt = |k: &str, max: usize| -> Result<Option<String>, String> {
        get(k).map(|v| clean(v, max, k)).transpose()
    };
    let dir = |d: Option<String>| -> Result<Option<String>, String> {
        match d {
            Some(d) if !(d.starts_with('/') || d.starts_with('~')) => {
                Err(format!("dir= must be a full path, not {d}"))
            }
            d => Ok(d),
        }
    };
    Ok(match what.as_str() {
        "" | "open" => {
            // a saved workspace opens like a tab (`aleph ui NAME`)
            if let Some(w) = opt("workspace", 40)? {
                if !w
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err(format!("workspace={w}: a workspace's name (gpu-watch)"));
                }
                return Ok(Link::Open {
                    view: Some(w.to_lowercase()),
                });
            }
            let view = opt("view", 20)?.map(|v| v.to_lowercase());
            if let Some(v) = &view {
                if !VIEWS.contains(&v.as_str()) {
                    return Err(format!("view={v}: one of {}", VIEWS.join(", ")));
                }
            }
            Link::Open { view }
        }
        "focus" => Link::Focus {
            session: need("session", MAX_SEL)?,
        },
        "new" => Link::New {
            host: opt("host", MAX_SEL)?,
            dir: dir(opt("dir", 1024)?)?,
            agent: opt("agent", 40)?,
            prompt: opt("prompt", MAX_TEXT)?,
        },
        "send" => Link::Send {
            session: need("session", MAX_SEL)?,
            text: need("text", MAX_TEXT)?,
        },
        "approve" => Link::Approve {
            session: need("session", MAX_SEL)?,
        },
        "close" => Link::Close {
            session: need("session", MAX_SEL)?,
        },
        "queue" => Link::Queue {
            text: need("text", MAX_TEXT)?,
            host: opt("host", MAX_SEL)?,
            dir: dir(opt("dir", 1024)?)?,
        },
        w => return Err(format!("aleph://{w}: unknown (aleph url links lists them)")),
    })
}

impl Link {
    /// Does it change something (and so ask first)?
    pub fn acts(&self) -> bool {
        !matches!(self, Link::Open { .. } | Link::Focus { .. })
    }

    /// Our own command line that does it.
    pub fn argv(&self) -> Vec<String> {
        let s = |x: &str| x.to_string();
        match self {
            Link::Open { view } => {
                let mut v = vec![s("ui")];
                v.extend(view.clone());
                v
            }
            Link::Focus { session } => vec![s("focus"), session.clone()],
            Link::New {
                host,
                dir,
                agent,
                prompt,
            } => {
                let mut v = Vec::new();
                if let Some(h) = host {
                    v.extend([s("--host"), h.clone()]);
                }
                v.push(s("new"));
                if let Some(d) = dir {
                    v.extend([s("--dir"), d.clone()]);
                }
                if let Some(a) = agent {
                    v.extend([s("--agent"), a.clone()]);
                }
                if let Some(p) = prompt {
                    v.extend([s("-m"), p.clone()]);
                }
                v.push(s("--detach"));
                v
            }
            Link::Send { session, text } => vec![s("send"), session.clone(), text.clone()],
            Link::Approve { session } => vec![s("approve"), session.clone()],
            Link::Close { session } => vec![s("close"), session.clone(), s("--yes")],
            Link::Queue { text, host, dir } => {
                let mut v = vec![s("queue"), s("add")];
                v.extend(host.clone());
                v.push(text.clone());
                if let Some(d) = dir {
                    v.extend([s("--dir"), d.clone()]);
                }
                v
            }
        }
    }

    /// The dialog's (title, question, button) for a link that acts.
    pub fn ask(&self) -> (String, String, String) {
        let quoted = |t: &str| {
            let n = t.chars().count();
            let shown: String = t.chars().take(600).collect();
            if n > 600 {
                format!("“{shown}…” (+{} more characters)", n - 600)
            } else {
                format!("“{shown}”")
            }
        };
        let (t, q, b) = match self {
            Link::New {
                host,
                dir,
                agent,
                prompt,
            } => (
                "Start a session",
                format!(
                    "Start {} on {}{}?{}",
                    agent.as_deref().unwrap_or("the default agent"),
                    host.as_deref().unwrap_or("the default machine"),
                    dir.as_deref()
                        .map(|d| format!(" in {d}"))
                        .unwrap_or_default(),
                    prompt
                        .as_deref()
                        .map(|p| format!("\n\nIts first prompt:\n{}", quoted(p)))
                        .unwrap_or_default()
                ),
                "Start",
            ),
            Link::Send { session, text } => (
                "Send a prompt",
                format!("Send this to {session}?\n\n{}", quoted(text)),
                "Send",
            ),
            Link::Approve { session } => (
                "Approve",
                format!("Answer yes to {session}'s question?"),
                "Approve",
            ),
            Link::Close { session } => (
                "Close a session",
                format!("Close {session}? Its agent stops."),
                "Close",
            ),
            Link::Queue { text, host, dir } => (
                "Queue a task",
                format!(
                    "Queue this for {}{}?\n\n{}",
                    host.as_deref()
                        .map(|h| format!("the next idle agent on {h}"))
                        .unwrap_or_else(|| "the next idle agent".into()),
                    dir.as_deref()
                        .map(|d| format!(" in {d}"))
                        .unwrap_or_default(),
                    quoted(text)
                ),
                "Queue",
            ),
            Link::Open { .. } | Link::Focus { .. } => ("Aleph", String::new(), "OK"),
        };
        (format!("Aleph · {t}"), q, b.to_string())
    }
}

/// Ask with a dialog: true only when the button was pressed (Cancel, a
/// closed dialog, two minutes without an answer and no dialog tool are all no).
fn confirm(title: &str, question: &str, button: &str) -> Result<bool, String> {
    // showcase mode: what the dialog says is masked like everything else on screen
    let (title, question) = (
        crate::showcase::text(title),
        crate::showcase::text(question),
    );
    let t = Duration::from_secs(150);
    if cfg!(target_os = "macos") {
        let script = [
            "on run argv",
            "activate",
            "set r to display dialog (item 2 of argv) with title (item 1 of argv) buttons {\"Cancel\", (item 3 of argv)} default button (item 3 of argv) cancel button \"Cancel\" with icon caution giving up after 120",
            "if gave up of r then return \"\"",
            "return button returned of r",
            "end run",
        ];
        let mut args: Vec<String> = script
            .iter()
            .flat_map(|l| ["-e".to_string(), l.to_string()])
            .collect();
        args.extend([
            title.into_owned(),
            question.into_owned(),
            button.to_string(),
        ]);
        let o = proc::cap("osascript", &args, t);
        return Ok(o.ok() && o.text().trim() == button);
    }
    if proc::which("zenity").is_some() {
        let o = proc::cap(
            "zenity",
            &[
                "--question",
                "--title",
                &title,
                "--text",
                &question,
                "--ok-label",
                button,
                "--cancel-label",
                "Cancel",
                "--timeout",
                "120",
            ],
            t,
        );
        return Ok(o.ok());
    }
    if proc::which("kdialog").is_some() {
        return Ok(proc::cap("kdialog", &["--title", &title, "--yesno", &question], t).ok());
    }
    Err("no dialog to ask with (zenity or kdialog): links that act are refused".into())
}

/// A banner saying how it went (the link came from outside a terminal, so
/// there's nowhere else to say it).
fn banner(title: &str, body: &str) {
    let (title, body) = (crate::showcase::text(title), crate::showcase::text(body));
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let _ = if cfg!(target_os = "macos") {
        proc::cap(
            "osascript",
            &[
                "-e",
                &format!("display notification {} with title {}", q(&body), q(&title)),
            ],
            Duration::from_secs(10),
        )
    } else {
        proc::cap(
            "notify-send",
            &["-a", "aleph", &title, &body],
            Duration::from_secs(10),
        )
    };
}

/// The last lines of a session's screen, for the approve dialog.
fn screen_of(session: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let o = proc::cap(
        &exe.to_string_lossy(),
        &["peek", session, "-n", "10"],
        Duration::from_secs(15),
    );
    let t = o.text();
    (o.ok() && !t.trim().is_empty()).then(|| t.trim_end().to_string())
}

fn handle(cfg: &Config, url: &str, yes_flag: bool) -> ExitCode {
    let link = match parse(url) {
        Ok(l) => l,
        Err(e) => {
            banner("Aleph link refused", &e);
            return fail(&e);
        }
    };
    let argv = link.argv();
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| crate::brand::BIN.into());
    if !link.acts() {
        let words: Vec<&str> = argv.iter().map(String::as_str).collect();
        let cmd = crate::termlaunch::self_cmd(&words);
        return match crate::termlaunch::open(&cfg.get_or("terminal"), &cmd, "Aleph") {
            Ok(app) => {
                ok(&format!("opened in {}", app.title()));
                ExitCode::SUCCESS
            }
            Err(e) => {
                banner("Aleph link", &format!("couldn't open a terminal tab: {e}"));
                fail(&format!("couldn't open a terminal tab: {e}"))
            }
        };
    }
    if !yes_flag {
        let (title, mut question, button) = link.ask();
        if let Link::Approve { session } = &link {
            match screen_of(session) {
                Some(s) => question.push_str(&format!("\n\nIts screen now:\n{s}")),
                None => question.push_str("\n\n(its screen couldn't be read)"),
            }
        }
        match confirm(&title, &question, &button) {
            Ok(true) => {}
            Ok(false) => {
                hint("not done: the dialog wasn't confirmed");
                return ExitCode::from(1);
            }
            Err(e) => {
                banner("Aleph link refused", &e);
                return fail(&e);
            }
        }
    }
    let o = proc::cap(&exe, &argv, Duration::from_secs(60));
    let (title, _, _) = link.ask();
    let said = o.text();
    let last = said
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if o.ok() {
        banner(&title, if last.is_empty() { "done" } else { last });
        ok(&format!("done: aleph {}", argv.join(" ")));
        ExitCode::SUCCESS
    } else {
        let why = if last.is_empty() {
            o.reason()
        } else {
            last.to_string()
        };
        banner(&format!("{title}: failed"), &why);
        fail(&why)
    }
}

// ------------------------------------------------------------------ install

const APP: &str = "Aleph Links.app";
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

fn app_path() -> PathBuf {
    crate::config::home().join("Applications").join(APP)
}

fn desktop_path() -> PathBuf {
    crate::config::home().join(".local/share/applications/aleph-links.desktop")
}

/// The shell line before a link: the daemon's own environment (PATH with
/// Homebrew's, where the config and state live) and this aleph, as
/// `do shell script` starts with almost nothing.
pub fn applet_prefix(exe: &str, env: &[(String, String)]) -> String {
    let mut s = String::new();
    for (k, v) in env {
        s.push_str(&format!("{k}={} ", proc::sh_quote(v)));
    }
    s.push_str(&proc::sh_quote(exe));
    s.push_str(" url ");
    s
}

/// The applet's script: every link to `aleph url`, in the background.
pub fn applet_source(prefix: &str) -> String {
    let lit = prefix.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "on open location theURL\n\tdo shell script \"{lit}\" & quoted form of theURL & \" >/dev/null 2>&1 &\"\nend open location\n\non run\n\tdisplay dialog \"Aleph Links opens aleph:// links (aleph url links lists them). There's nothing to do here.\" buttons {{\"OK\"}} default button \"OK\" with title \"Aleph\"\nend run\n"
    )
}

/// What the applet carries over from here: the service's PATH and kept variables.
fn applet_env() -> Vec<(String, String)> {
    let mut v = vec![(
        "PATH".to_string(),
        crate::service::service_path(&std::env::var("PATH").unwrap_or_default()),
    )];
    for k in crate::service::KEPT {
        if let Ok(x) = std::env::var(k) {
            if !x.is_empty() {
                v.push((k.to_string(), x));
            }
        }
    }
    v
}

/// The `.desktop` file for Linux.
pub fn desktop_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Aleph links\nComment=Opens aleph:// links\nExec={} url %u\nNoDisplay=true\nTerminal=false\nMimeType=x-scheme-handler/aleph;\n",
        proc::sh_quote(exe)
    )
}

/// What installing does on macOS, step by step: (program, args).
pub fn mac_plan(app: &Path, script: &Path) -> Vec<(String, Vec<String>)> {
    let a = app.to_string_lossy().to_string();
    let plist = app
        .join("Contents/Info.plist")
        .to_string_lossy()
        .to_string();
    let s = |x: &str| x.to_string();
    vec![
        (
            s("osacompile"),
            vec![s("-o"), a.clone(), script.to_string_lossy().to_string()],
        ),
        (
            s("plutil"),
            vec![
                s("-replace"),
                s("CFBundleIdentifier"),
                s("-string"),
                s("dev.aleph.links"),
                plist.clone(),
            ],
        ),
        (
            s("plutil"),
            vec![
                s("-replace"),
                s("CFBundleURLTypes"),
                s("-json"),
                s(r#"[{"CFBundleURLName":"Aleph links","CFBundleURLSchemes":["aleph"]}]"#),
                plist.clone(),
            ],
        ),
        (
            s("plutil"),
            vec![
                s("-replace"),
                s("LSUIElement"),
                s("-bool"),
                s("true"),
                plist,
            ],
        ),
        // the plist changed: sign it again (ad hoc, as osacompile did)
        (
            s("codesign"),
            vec![s("--force"), s("--sign"), s("-"), a.clone()],
        ),
        (s(LSREGISTER), vec![s("-f"), a]),
    ]
}

fn install() -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => return fail(&format!("where am I? {e}")),
    };
    if cfg!(target_os = "macos") {
        let app = app_path();
        let tmp = crate::config::cache_dir().join("aleph-links.applescript");
        if let Some(d) = tmp.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(&tmp, applet_source(&applet_prefix(&exe, &applet_env()))) {
            return fail(&format!("{}: {e}", tmp.display()));
        }
        if let Some(d) = app.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::remove_dir_all(&app);
        for (prog, args) in mac_plan(&app, &tmp) {
            let o = proc::cap(&prog, &args, Duration::from_secs(30));
            if !o.ok() {
                return fail(&format!("{prog}: {}", o.reason()));
            }
        }
        let _ = std::fs::remove_file(&tmp);
        ok(&format!(
            "aleph:// links open with {}",
            crate::doctor::tilde(&app)
        ));
    } else {
        let p = desktop_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(&p, desktop_entry(&exe)) {
            return fail(&format!("{}: {e}", p.display()));
        }
        let o = proc::cap(
            "xdg-mime",
            &["default", "aleph-links.desktop", "x-scheme-handler/aleph"],
            Duration::from_secs(15),
        );
        if !o.ok() {
            return fail(&format!("xdg-mime: {}", o.reason()));
        }
        if let Some(d) = p.parent() {
            let _ = proc::cap(
                "update-desktop-database",
                &[d.to_string_lossy().as_ref()],
                Duration::from_secs(15),
            );
        }
        ok(&format!(
            "aleph:// links open with {}",
            crate::doctor::tilde(&p)
        ));
    }
    hint("links that act (approve, send, close, new, queue) always ask first");
    hint("try one: open 'aleph://open?view=monitor'   ·   aleph url links lists them");
    ExitCode::SUCCESS
}

fn uninstall() -> ExitCode {
    if cfg!(target_os = "macos") {
        let app = app_path();
        if app.exists() {
            let _ = proc::cap(
                LSREGISTER,
                &["-u", app.to_string_lossy().as_ref()],
                Duration::from_secs(15),
            );
            if let Err(e) = std::fs::remove_dir_all(&app) {
                return fail(&format!("{}: {e}", app.display()));
            }
        }
    } else {
        let _ = std::fs::remove_file(desktop_path());
    }
    ok("aleph:// links no longer open Aleph");
    ExitCode::SUCCESS
}

fn status() -> ExitCode {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (path, text) = if cfg!(target_os = "macos") {
        let app = app_path();
        let src = proc::cap(
            "osadecompile",
            &[app
                .join("Contents/Resources/Scripts/main.scpt")
                .to_string_lossy()
                .as_ref()],
            Duration::from_secs(10),
        );
        (app, src.ok().then(|| src.text()))
    } else {
        let p = desktop_path();
        let t = std::fs::read_to_string(&p).ok();
        (p, t)
    };
    match text {
        None => {
            warn("aleph:// links aren't registered");
            hint("aleph url install");
        }
        Some(t) if !t.contains(&exe) => {
            warn(&format!(
                "{} runs another aleph than this one",
                crate::doctor::tilde(&path)
            ));
            hint("aleph url install puts this one in");
        }
        Some(_) => ok(&format!(
            "aleph:// links open with {}",
            crate::doctor::tilde(&path)
        )),
    }
    ExitCode::SUCCESS
}

fn links() -> ExitCode {
    head("aleph:// links");
    for (l, what) in [
        (
            "aleph://open[?view=monitor]",
            "the dashboard (fleet, monitor, inbox, settings), in a new tab",
        ),
        (
            "aleph://open?workspace=gpu-watch",
            "a saved workspace (W in the dashboard), in a new tab",
        ),
        (
            "aleph://focus?session=b31:hunt",
            "one session in Focus, in a new tab",
        ),
        (
            "aleph://new?host=b31&dir=/srv/x&agent=codex&prompt=…",
            "start a session   · asks first",
        ),
        (
            "aleph://send?session=b31:hunt&text=…",
            "prompt a session   · asks",
        ),
        (
            "aleph://approve?session=b31:hunt",
            "answer yes         · asks, showing its screen",
        ),
        (
            "aleph://close?session=b31:hunt",
            "close a session    · asks",
        ),
        (
            "aleph://queue?text=…&host=b31[&dir=/srv/x]",
            "queue a task       · asks",
        ),
    ] {
        out(ratatui::text::Line::from(vec![
            ratatui::text::Span::raw(format!("  {}  ", crate::ui::pad(l, 52))),
            ratatui::text::Span::styled(what, crate::ui::muted()),
        ]));
    }
    hint("text is URL-encoded (%20 or + for a space); Shortcuts' URL action does it for you");
    ExitCode::SUCCESS
}

pub fn cmd(a: Args) -> ExitCode {
    let pos = a.positional();
    match pos.first().copied() {
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        Some("status") => status(),
        Some("links") | Some("ls") => links(),
        Some(u) if u.starts_with("aleph:") => handle(&a.cfg, u, a.has("--yes")),
        _ => {
            raw(USAGE);
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_parse_and_say_what_they_run() {
        assert_eq!(parse("aleph://open").unwrap(), Link::Open { view: None });
        assert_eq!(
            parse("aleph://open?view=Monitor").unwrap().argv(),
            ["ui", "monitor"]
        );
        assert!(parse("aleph://open?view=nope").is_err());
        assert_eq!(
            parse("aleph://open?workspace=gpu-watch").unwrap().argv(),
            ["ui", "gpu-watch"]
        );
        assert!(parse("aleph://open?workspace=a%20b").is_err());
        let l =
            parse("aleph://send?session=b31%3Ahunt&text=run+the%20tests%0Athen+commit").unwrap();
        assert_eq!(
            l,
            Link::Send {
                session: "b31:hunt".into(),
                text: "run the tests\nthen commit".into()
            }
        );
        assert!(l.acts());
        assert_eq!(l.argv(), ["send", "b31:hunt", "run the tests\nthen commit"]);
        assert_eq!(
            parse("aleph:///approve/?session=hunt").unwrap().argv(),
            ["approve", "hunt"]
        );
        assert_eq!(
            parse("aleph://close?session=b31:x").unwrap().argv(),
            ["close", "b31:x", "--yes"]
        );
        assert_eq!(
            parse("aleph://new?host=b31&dir=/srv/x&agent=codex&prompt=hi")
                .unwrap()
                .argv(),
            [
                "--host", "b31", "new", "--dir", "/srv/x", "--agent", "codex", "-m", "hi",
                "--detach"
            ]
        );
        assert_eq!(
            parse("aleph://queue?text=later&host=b31").unwrap().argv(),
            ["queue", "add", "b31", "later"]
        );
        assert!(!parse("aleph://focus?session=b31:x").unwrap().acts());
    }

    #[test]
    fn bad_links_are_refused() {
        for bad in [
            "https://example.com",
            "aleph://approve",
            "aleph://send?session=x",
            "aleph://rm?session=x",
            "aleph://new?dir=relative/path",
            "aleph://send?session=x&text=a%1bb",
            "aleph://approve?session=a%00b",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let long = format!("aleph://send?session=x&text={}", "a".repeat(MAX_TEXT + 1));
        assert!(parse(&long).is_err());
        // a broken escape stays as written
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("a%zzb"), "a%zzb");
        assert_eq!(decode("%E2%84%B5"), "ℵ");
    }

    #[test]
    fn the_dialog_names_what_happens() {
        let (t, q, b) = parse("aleph://close?session=b31:hunt").unwrap().ask();
        assert_eq!(
            (t.as_str(), b.as_str()),
            ("Aleph · Close a session", "Close")
        );
        assert!(q.contains("b31:hunt"));
        let text = "x".repeat(700);
        let (_, q, _) = Link::Send {
            session: "s".into(),
            text,
        }
        .ask();
        assert!(q.contains("+100 more characters"), "{q}");
    }

    #[test]
    fn the_applet_hands_links_to_this_aleph() {
        let pre = applet_prefix(
            "/Users/me/.cargo/bin/aleph",
            &[
                ("PATH".into(), "/opt/homebrew/bin:/usr/bin".into()),
                ("XDG_STATE_HOME".into(), "/x \"y\"".into()),
            ],
        );
        assert_eq!(pre, "PATH=/opt/homebrew/bin:/usr/bin XDG_STATE_HOME='/x \"y\"' /Users/me/.cargo/bin/aleph url ");
        let s = applet_source(&pre);
        assert!(s.contains("on open location theURL"));
        // an AppleScript string: its quotes escaped, the link quoted by AppleScript itself
        assert!(
            s.contains(r#"do shell script "PATH=/opt/homebrew/bin:/usr/bin XDG_STATE_HOME='/x \"y\"' /Users/me/.cargo/bin/aleph url " & quoted form of theURL"#),
            "{s}"
        );
        let p = mac_plan(
            Path::new("/A/Aleph Links.app"),
            Path::new("/t/s.applescript"),
        );
        assert_eq!(p[0].0, "osacompile");
        assert!(p.iter().any(|(_, a)| a
            .iter()
            .any(|x| x.contains("\"CFBundleURLSchemes\":[\"aleph\"]"))));
        assert_eq!(p[4].0, "codesign");
        assert!(desktop_entry("/usr/bin/aleph").contains("MimeType=x-scheme-handler/aleph;"));
    }

    /// macOS only, by hand: the applet's script compiles and decompiles as
    /// written (nothing is registered). `cargo test applet_compiles -- --ignored`
    #[test]
    #[ignore]
    fn applet_compiles() {
        let dir = std::env::temp_dir().join(format!("aleph-applet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("s.applescript");
        let pre = applet_prefix(
            "/opt/x y/aleph",
            &[("PATH".into(), "/opt/homebrew/bin:/usr/bin".into())],
        );
        std::fs::write(&src, applet_source(&pre)).unwrap();
        let app = dir.join("Aleph Links.app");
        for (prog, args) in mac_plan(&app, &src).into_iter().take(5) {
            let o = proc::cap(&prog, &args, Duration::from_secs(30));
            assert!(o.ok(), "{prog}: {}", o.reason());
        }
        let back = proc::cap(
            "osadecompile",
            &[app
                .join("Contents/Resources/Scripts/main.scpt")
                .to_string_lossy()
                .as_ref()],
            Duration::from_secs(10),
        );
        assert!(back.text().contains("/opt/x y/aleph"), "{}", back.text());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
