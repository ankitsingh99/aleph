//! Open a session's folder in your editor: `o` in Fleet, the palette, or
//! `aleph edit <session | host:/path>`. A folder on this computer opens as it
//! is; one on another machine opens over the editor's own remote support
//! (VS Code and Cursor: Remote-SSH, `--remote ssh-remote+<alias>`; Zed:
//! `zed ssh://<alias>/<path>`), with the machine's ssh alias.
//!
//! `editor` picks it: auto (the first of Cursor, VS Code, Zed on this
//! computer: its command, else its app), code, cursor, zed, windsurf, or
//! your own command line with `{path}`, `{host}` and `{remote}` (`ssh-remote+<alias>`).

use crate::cli::{self, Args};
use crate::config::Config;
use crate::out::*;
use std::path::PathBuf;
use std::process::ExitCode;

/// (setting value, name, commands to look for, its app's command inside the bundle)
const EDITORS: &[(&str, &str, &[&str], &str)] = &[
    (
        "cursor",
        "Cursor",
        &["cursor"],
        "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
    ),
    (
        "code",
        "VS Code",
        &["code"],
        "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
    ),
    (
        "zed",
        "Zed",
        &["zed"],
        "/Applications/Zed.app/Contents/MacOS/cli",
    ),
    (
        "windsurf",
        "Windsurf",
        &["windsurf"],
        "/Applications/Windsurf.app/Contents/Resources/app/bin/windsurf",
    ),
];

/// An editor found on this computer: (setting value, name, the program).
pub fn find(id: &str) -> Option<(&'static str, &'static str, PathBuf)> {
    let (v, name, cmds, app) = EDITORS.iter().find(|e| e.0 == id)?;
    for c in *cmds {
        if let Some(p) = crate::proc::which(c) {
            return Some((v, name, p));
        }
    }
    let app = PathBuf::from(app);
    (cfg!(target_os = "macos") && app.exists()).then_some((*v, *name, app))
}

/// What to run: (program, args), and the editor's name. `host` is None for
/// this computer.
pub fn plan(
    setting: &str,
    host: Option<&str>,
    path: &str,
) -> Result<(String, Vec<String>, String), String> {
    let s = setting.trim();
    // your own command line
    if s.contains(' ') || s.contains('{') {
        let remote = host.map(|h| format!("ssh-remote+{h}")).unwrap_or_default();
        let line = s
            .replace("{path}", &crate::proc::sh_quote(path))
            .replace("{host}", &crate::proc::sh_quote(host.unwrap_or("")))
            .replace("{remote}", &crate::proc::sh_quote(&remote));
        return Ok(("sh".into(), vec!["-c".into(), line], "your editor".into()));
    }
    let found = if s.is_empty() || s == "auto" {
        EDITORS.iter().find_map(|e| find(e.0))
    } else {
        Some(find(s).ok_or_else(|| format!("{s} isn't on this computer (editor = auto, code, cursor, zed, windsurf, or a command line)"))?)
    };
    let Some((id, name, prog)) = found else {
        return Err("no editor found (Cursor, VS Code, Zed): set editor to one, or to your own command line".into());
    };
    let prog = prog.to_string_lossy().into_owned();
    let args = match (id, host) {
        (_, None) => vec![path.to_string()],
        ("zed", Some(h)) => vec![format!(
            "ssh://{h}{}{path}",
            if path.starts_with('/') { "" } else { "/" }
        )],
        (_, Some(h)) => vec![
            "--remote".into(),
            format!("ssh-remote+{h}"),
            path.to_string(),
        ],
    };
    Ok((prog, args, name.to_string()))
}

/// Open `path` (on `host`'s machine, or here); what it said.
pub fn open(cfg: &Config, host: &str, path: &str) -> Result<String, String> {
    if path.is_empty() || path == "-" {
        return Err("it has no folder to open".into());
    }
    let remote = (!crate::sshcfg::is_local(host)).then_some(host);
    // tests (the fake world): a stand-in that records the call, never a real app
    let setting = crate::brand::env("EDITOR_CMD").unwrap_or_else(|| cfg.get_or("editor"));
    if cfg!(test) && crate::brand::env("EDITOR_CMD").is_none() {
        return Err("tests don't open editors".into());
    }
    let (prog, args, name) = plan(&setting, remote, path)?;
    std::process::Command::new(&prog)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(format!(
        "opened {path}{} in {name}",
        remote.map(|h| format!(" on {h}")).unwrap_or_default()
    ))
}

pub fn cmd(a: Args) -> ExitCode {
    let pos = a.positional();
    let Some(q) = pos.first() else {
        return fail("usage: aleph edit <session | host:/path>");
    };
    // host:/a/path, or a session's folder
    let (host, path) = match q.split_once(':') {
        Some((h, p)) if p.starts_with('/') || p.starts_with('~') => (h.to_string(), p.to_string()),
        _ => match cli::find(&a.cfg, &a.flag, q) {
            Ok((_, s)) => (s.host.clone(), s.cwd.clone()),
            Err(c) => return c,
        },
    };
    match open(&a.cfg, &host, &path) {
        Ok(m) => {
            ok(&m);
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn your_own_line_and_remote_forms() {
        let (p, a, n) = plan("myedit --wait {remote} {path}", Some("b31"), "/srv/px x").unwrap();
        assert_eq!((p.as_str(), n.as_str()), ("sh", "your editor"));
        assert_eq!(a[1], "myedit --wait ssh-remote+b31 '/srv/px x'");
        let (_, a, _) = plan("vim {path}", None, "/tmp/x").unwrap();
        assert_eq!(a[1], "vim /tmp/x");
        assert!(plan("nosuch-editor", None, "/x").is_err());
    }
}
