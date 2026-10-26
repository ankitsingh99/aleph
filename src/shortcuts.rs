//! `aleph shortcuts [ls | install] [--dry-run] [--json]` — Warp tab configs
//! and Raycast script commands for every fleet host.
//!
//! * Warp: `~/.warp/tab_configs/aleph_<host>.toml` runs `aleph --host <host>`.
//! * Raycast (first `raycast_dirs` folder): `aleph-<host>.sh` opens that tab
//!   config; `aleph-status.sh` shows `status --short` inline (every 30 s);
//!   `aleph-new.sh` asks for a folder and opens a `aleph_new` tab config
//!   running `aleph new --dir …`.
//!
//! Only files we wrote (they contain `# managed by aleph`) are ever
//! overwritten; anything else with the same name is left alone. Files from
//! before the rename (`claude_<host>.toml`, `claude-<host>.sh`, marked
//! `# managed by claude-herdr`) are removed by `install` once the new ones are written.

use crate::cli::Args;
use crate::config::{home, Config};
use crate::doctor::tilde;
use crate::out::*;
use crate::proc::sh_quote;
use crate::ui::{self, c};
use ratatui::prelude::*;
use serde_json::json;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub const MARK: &str = crate::brand::MANAGED;

#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub kind: &'static str,
    pub path: PathBuf,
    pub content: String,
    pub exec: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Missing,
    /// ours, identical
    Current,
    /// ours, but different from what we'd write now
    Outdated,
    /// someone else's file — never overwritten
    Unmanaged,
}

impl State {
    fn word(self) -> &'static str {
        match self {
            State::Missing => "missing",
            State::Current => "managed",
            State::Outdated => "outdated",
            State::Unmanaged => "unmanaged",
        }
    }
}

pub fn state(existing: Option<&str>, want: &str) -> State {
    match existing {
        None => State::Missing,
        Some(t) if !t.contains(MARK) => State::Unmanaged,
        Some(t) if t == want => State::Current,
        Some(_) => State::Outdated,
    }
}

/// State on disk; `aleph_new.toml` is rewritten by aleph-new.sh on every
/// run, so any managed version of it counts as current.
fn file_state(f: &File) -> State {
    match state(read(&f.path).as_deref(), &f.content) {
        State::Outdated if f.path.ends_with("aleph_new.toml") => State::Current,
        s => s,
    }
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A Warp tab config running `command`.
pub fn warp_tab(title: &str, url_name: &str, command: &str) -> String {
    format!(
        "{MARK} — aleph shortcuts install rewrites this file; delete this line to keep your edits\n\
         # open with: open 'warp://tab_config/{url_name}'\n\
         name = {}\n\n\
         [[panes]]\n\
         id = \"main\"\n\
         type = \"terminal\"\n\
         commands = [{}]\n\
         is_focused = true\n",
        toml_str(title),
        toml_str(command)
    )
}

fn raycast_header(title: &str, mode: &str, extra: &[&str]) -> String {
    let mut s = format!(
        "#!/bin/bash\n{MARK} — aleph shortcuts install rewrites this file; delete this line to keep your edits\n\
         # @raycast.schemaVersion 1\n\
         # @raycast.title {title}\n\
         # @raycast.mode {mode}\n\
         # @raycast.icon 🤖\n\
         # @raycast.packageName aleph\n"
    );
    for e in extra {
        s.push_str(&format!("# @raycast.{e}\n"));
    }
    s
}

/// Everything `install` would write. `exe` is this binary (Raycast runs
/// scripts without the user's shell functions).
pub fn plan(hosts: &[String], exe: &Path, warp: &Path, raycast: &Path) -> Vec<File> {
    let mut v = Vec::new();
    let bin = sh_quote(&exe.to_string_lossy());
    for h in hosts {
        let name = format!("aleph_{}", safe(h));
        v.push(File {
            kind: "warp",
            path: warp.join(format!("{name}.toml")),
            content: warp_tab(
                &format!("Aleph @ {h}"),
                &name,
                &format!("aleph --host {}", sh_quote(h)),
            ),
            exec: false,
        });
        v.push(File {
            kind: "raycast",
            path: raycast.join(format!("aleph-{}.sh", safe(h))),
            content: format!(
                "{}open 'warp://tab_config/{name}'\n",
                raycast_header(&format!("Aleph on {h}"), "silent", &[])
            ),
            exec: true,
        });
    }
    v.push(File {
        kind: "raycast",
        path: raycast.join("aleph-status.sh"),
        content: format!(
            "{}export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"\n{bin} status --short </dev/null\n",
            raycast_header("Agent status", "inline", &["refreshTime 30s"])
        ),
        exec: true,
    });
    // the tab config aleph-new.sh rewrites on every run; installed up front
    // (picking a folder) so the URL always resolves
    let tab = warp.join("aleph_new.toml");
    let initial = warp_tab("New agent session", "aleph_new", "aleph -i");
    let body =
        warp_tab("New agent session", "aleph_new", "CH_CMD").replace("\"CH_CMD\"", "\"$cmd\"");
    v.push(File {
        kind: "warp",
        path: tab.clone(),
        content: initial,
        exec: false,
    });
    v.push(File {
        kind: "raycast",
        path: raycast.join("aleph-new.sh"),
        content: format!(
            r#"{}dir=$1
if [ -n "$dir" ]; then
  cmd="aleph new --dir '$(printf '%s' "$dir" | sed "s/'/'\\\\''/g")'"
else
  cmd="aleph -i"
fi
cmd=$(printf '%s' "$cmd" | sed 's/\\/\\\\/g; s/"/\\"/g')
mkdir -p {warp_dir}
cat > {tab} <<EOF
{body}EOF
open 'warp://tab_config/aleph_new'
"#,
            raycast_header(
                "New agent session",
                "silent",
                &[r#"argument1 { "type": "text", "placeholder": "folder", "optional": true }"#]
            ),
            warp_dir = sh_quote(&warp.to_string_lossy()),
            tab = sh_quote(&tab.to_string_lossy()),
        ),
        exec: true,
    });
    v
}

/// Host aliases in file names / URLs: keep `[A-Za-z0-9._-]`.
fn safe(h: &str) -> String {
    h.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn warp_dir() -> PathBuf {
    home().join(".warp").join("tab_configs")
}

fn raycast_dir(cfg: &Config) -> PathBuf {
    cfg.raycast_dirs()
        .into_iter()
        .next()
        .unwrap_or_else(|| home().join(".raycast-scripts"))
}

fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("aleph"))
}

pub fn cmd(a: Args) -> ExitCode {
    let has = |f: &str| a.rest.iter().any(|x| x == f);
    let dry = has("--dry-run") || has("-n");
    let js = has("--json");
    let hosts: Vec<String> = match &a.flag {
        Some(h) => vec![h.clone()],
        None => a.cfg.hosts(),
    };
    let files = plan(&hosts, &exe(), &warp_dir(), &raycast_dir(&a.cfg));
    match a
        .rest
        .iter()
        .find(|x| !x.starts_with('-'))
        .map(String::as_str)
    {
        None | Some("ls") | Some("list") => ls(&a.cfg, &files, js),
        Some("install") => {
            if hosts.is_empty() {
                warn("no fleet hosts yet — only the status/new scripts will be written");
                hint("aleph host include <alias>");
            }
            install(&files, dry, js)
        }
        Some(x) => fail(&format!(
            "unknown shortcuts command '{x}' — aleph shortcuts [ls | install] [--dry-run]"
        )),
    }
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn ls(cfg: &Config, files: &[File], js: bool) -> ExitCode {
    let mut rows: Vec<(String, PathBuf, State)> = files
        .iter()
        .map(|f| (f.kind.to_string(), f.path.clone(), file_state(f)))
        .collect();
    // other managed files we no longer plan (e.g. a host left the fleet)
    for (kind, dir) in [("warp", warp_dir()), ("raycast", raycast_dir(cfg))] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut extra: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| !rows.iter().any(|r| r.1 == *p))
            .filter(|p| read(p).is_some_and(|t| t.contains(MARK)))
            .collect();
        extra.sort();
        for p in extra {
            rows.push((kind.into(), p, State::Outdated));
        }
    }
    if js {
        json(&json!(rows
            .iter()
            .map(|(k, p, s)| json!({"kind": k, "path": p.to_string_lossy(), "state": s.word()}))
            .collect::<Vec<_>>()));
        return ExitCode::SUCCESS;
    }
    if !io::stdout().is_terminal() {
        for (k, p, s) in &rows {
            println!("{k}\t{}\t{}", p.display(), s.word());
        }
        return ExitCode::SUCCESS;
    }
    for (k, p, s) in &rows {
        let (glyph, col) = match s {
            State::Current => ("✓", c::GREEN()),
            State::Outdated => ("↻", c::YELLOW()),
            State::Missing => ("·", c::FAINT()),
            State::Unmanaged => ("!", c::BLUE()),
        };
        out(Line::from(vec![
            Span::styled(format!("  {glyph} "), ui::fg(col)),
            Span::styled(format!("{k:<8}"), ui::muted()),
            Span::raw(format!("{:<48}", tilde(p))),
            Span::styled(s.word().to_string(), ui::fg(col)),
        ]));
    }
    if rows
        .iter()
        .any(|r| matches!(r.2, State::Missing | State::Outdated))
    {
        hint("aleph shortcuts install  (--dry-run shows what it would write)");
    }
    if rows.iter().any(|r| r.2 == State::Unmanaged) {
        hint("unmanaged files are never overwritten — add `# managed by aleph` to let us");
    }
    ExitCode::SUCCESS
}

/// Files written before the rename (`claude_<host>.toml`, `claude-<host>.sh`,
/// marked `# managed by claude-herdr`) in the folders we write to.
fn legacy_files(files: &[File]) -> Vec<PathBuf> {
    let mut dirs: Vec<&Path> = files.iter().filter_map(|f| f.path.parent()).collect();
    dirs.sort();
    dirs.dedup();
    let mut v = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let old = (name.starts_with("claude_") && name.ends_with(".toml"))
                || (name.starts_with("claude-") && name.ends_with(".sh"));
            if old && read(&e.path()).is_some_and(|t| t.contains(crate::brand::LEGACY_MANAGED)) {
                v.push(e.path());
            }
        }
    }
    v
}

fn install(files: &[File], dry: bool, js: bool) -> ExitCode {
    let mut rows = Vec::new();
    let mut bad = 0;
    for f in files {
        let st = file_state(f);
        let action = match st {
            State::Unmanaged => "skipped",
            State::Current => "unchanged",
            State::Missing | State::Outdated if dry => "would write",
            State::Missing | State::Outdated => match write(f) {
                Ok(()) => "written",
                Err(e) => {
                    bad += 1;
                    err(&format!("{}: {e}", tilde(&f.path)));
                    "failed"
                }
            },
        };
        if dry && !js && matches!(st, State::Missing | State::Outdated) {
            out(Line::from(Span::styled(
                format!("── {} ({})", tilde(&f.path), st.word()),
                ui::fg(c::ACCENT()),
            )));
            for l in f.content.lines() {
                out(Line::from(Span::raw(format!("   {l}"))));
            }
        }
        if !js {
            match action {
                "written" => ok(&format!("{} {}", f.kind, tilde(&f.path))),
                "skipped" => warn(&format!(
                    "{} exists and isn't ours — left it alone",
                    tilde(&f.path)
                )),
                "unchanged" => say(Line::from(Span::styled(
                    format!("  · {} unchanged", tilde(&f.path)),
                    ui::faint(),
                ))),
                _ => {}
            }
        }
        rows.push(json!({"kind": f.kind, "path": f.path.to_string_lossy(), "action": action}));
    }
    // the pre-rename files are replaced by the ones above: remove ours only
    for p in legacy_files(files) {
        let action = if dry {
            "would remove (old name)"
        } else {
            match std::fs::remove_file(&p) {
                Ok(()) => "removed (old name)",
                Err(e) => {
                    bad += 1;
                    err(&format!("{}: {e}", tilde(&p)));
                    "failed"
                }
            }
        };
        if !js && action != "failed" {
            say(Line::from(Span::styled(
                format!("  · {} {action}", tilde(&p)),
                ui::faint(),
            )));
        }
        rows.push(json!({"kind": "old", "path": p.to_string_lossy(), "action": action}));
    }
    if js {
        json(&json!(rows));
    } else if dry {
        hint("nothing written (--dry-run)");
    } else {
        hint("Raycast: add the folder under Settings → Extensions → + → Add Script Directory");
    }
    if bad > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn write(f: &File) -> io::Result<()> {
    crate::config::write_atomic(&f.path, f.content.as_bytes())?;
    if f.exec {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_contents() {
        let hosts = vec!["b31".to_string(), "gpu 2".to_string()];
        let p = plan(
            &hosts,
            Path::new("/opt/my bin/aleph"),
            Path::new("/h/.warp/tab_configs"),
            Path::new("/h/rc"),
        );
        let names: Vec<String> = p.iter().map(|f| f.path.display().to_string()).collect();
        assert_eq!(
            names,
            vec![
                "/h/.warp/tab_configs/aleph_b31.toml",
                "/h/rc/aleph-b31.sh",
                "/h/.warp/tab_configs/aleph_gpu_2.toml",
                "/h/rc/aleph-gpu_2.sh",
                "/h/rc/aleph-status.sh",
                "/h/.warp/tab_configs/aleph_new.toml",
                "/h/rc/aleph-new.sh",
            ]
        );
        assert!(p.iter().all(|f| f.content.contains(MARK)));
        assert!(p[0].content.contains("commands = [\"aleph --host b31\"]"));
        assert!(p[2]
            .content
            .contains("commands = [\"aleph --host 'gpu 2'\"]"));
        assert!(p[1].content.starts_with("#!/bin/bash\n"));
        assert!(p[1].content.contains("@raycast.mode silent"));
        assert!(p[1]
            .content
            .ends_with("open 'warp://tab_config/aleph_b31'\n"));
        assert!(p
            .iter()
            .all(|f| f.exec == f.path.extension().unwrap().eq("sh")));
        let st = &p[4].content;
        assert!(st.contains("@raycast.mode inline") && st.contains("@raycast.refreshTime 30s"));
        assert!(st.contains("'/opt/my bin/aleph' status --short"));
        let nw = &p[6].content;
        assert!(nw.contains("@raycast.argument1") && nw.contains("aleph_new"));
    }

    #[test]
    fn states() {
        assert_eq!(state(None, "x"), State::Missing);
        assert_eq!(state(Some("mine"), "x"), State::Unmanaged);
        let w = format!("{MARK}\nA");
        assert_eq!(state(Some(&w), &w), State::Current);
        assert_eq!(state(Some(&format!("{MARK}\nB")), &w), State::Outdated);
    }

    #[test]
    fn raycast_new_script_writes_a_tab_config() {
        if crate::proc::which("bash").is_none() {
            return;
        }
        let d = std::env::temp_dir().join(format!("chs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let warp = d.join("warp");
        let p = plan(&[], Path::new("/x/aleph"), &warp, &d.join("rc"));
        let f = p.iter().find(|f| f.path.ends_with("aleph-new.sh")).unwrap();
        write(f).unwrap();
        // stub `open` so nothing launches
        let bin = d.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let open = bin.join("open");
        std::fs::write(
            &open,
            "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/opened\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let o = std::process::Command::new(&f.path)
            .arg("/srv/it's \"here\"")
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let toml = std::fs::read_to_string(warp.join("aleph_new.toml")).unwrap();
        assert!(
            toml.contains(r#"commands = ["aleph new --dir '/srv/it'\\''s \"here\"'"]"#),
            "{toml}"
        );
        assert_eq!(
            std::fs::read_to_string(bin.join("opened")).unwrap().trim(),
            "warp://tab_config/aleph_new"
        );
        let o = std::process::Command::new(&f.path)
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(o.status.success());
        let toml = std::fs::read_to_string(warp.join("aleph_new.toml")).unwrap();
        assert_eq!(
            toml,
            p.iter()
                .find(|f| f.path.ends_with("aleph_new.toml"))
                .unwrap()
                .content
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
