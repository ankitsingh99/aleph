//! Aleph's names in one place, and the one-time move from claude-herdr.
//!
//! Aleph used to be claude-herdr: the binary `claude-herdr`, usually typed
//! through a shell function (`cc`, `claude`). The first run of `aleph` moves
//! everything over:
//!
//! * the config, state and cache folders. The old paths become symlinks, so
//!   anything still running the old build keeps working;
//! * the launchd / systemd service (`com.claude-herdr.watch` → `dev.aleph.daemon`);
//! * the old binary name, which becomes a symlink to `aleph`;
//! * the managed block in `~/.zshrc`. `al` is added and the old names are kept,
//!   with a backup taken first.
//!
//! Every step is idempotent and safe to repeat. Old env var names
//! (`CLAUDE_HERDR_*`, `CC_*`) are still read for one release.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

pub const NAME: &str = "Aleph";
pub const BIN: &str = "aleph";
pub const ALIAS: &str = "al";
/// The 1-cell mark: U+2135 ALEF SYMBOL (left-to-right, unlike Hebrew א).
pub const MARK: &str = "ℵ";
/// Folder name under ~/.config, ~/.cache and ~/.local/state.
pub const DIR: &str = "aleph";
pub const LEGACY: &str = "claude-herdr";
pub const LABEL: &str = "dev.aleph.daemon";
pub const LEGACY_LABEL: &str = "com.claude-herdr.watch";
pub const UNIT: &str = "aleph.service";
pub const LEGACY_UNIT: &str = "claude-herdr-watch.service";
/// herdr keeps pane metadata per source. Running panes carry tokens reported
/// under this name, so it stays until they can be moved over.
pub const HERDR_SOURCE: &str = "claude-herdr";
pub const RC_BEGIN: &str = "# >>> aleph >>>";
pub const RC_END: &str = "# <<< aleph <<<";
pub const LEGACY_RC_BEGIN: &str = "# >>> claude-herdr >>>";
pub const LEGACY_RC_END: &str = "# <<< claude-herdr <<<";
/// Marker in files we generate (Warp tab configs, Raycast scripts, …).
pub const MANAGED: &str = "# managed by aleph";
pub const LEGACY_MANAGED: &str = "# managed by claude-herdr";

/// `ALEPH_<key>`, else the names from before the rename (`CLAUDE_HERDR_<key>`,
/// and `CC_<key>` for the few claude-herdr used). Empty values count as unset.
pub fn env_os(key: &str) -> Option<OsString> {
    let get = |name: String| std::env::var_os(name).filter(|v| !v.is_empty());
    get(format!("ALEPH_{key}"))
        // only the `CC_` names claude-herdr really read: C toolchains own the rest
        .or_else(|| {
            LEGACY_CC
                .contains(&key)
                .then(|| get(format!("CC_{key}")))
                .flatten()
        })
        .or_else(|| get(format!("CLAUDE_HERDR_{key}")))
}

/// The `CC_*` variables claude-herdr read (still honoured for one release).
const LEGACY_CC: &[&str] = &[
    "SSH_HOST",
    "MACHINE",
    "SSH_BIN",
    "SSH_REUSE",
    "SSH_CONFIG",
    "REFRESH",
    "NERD_FONT",
    "HERDR_VIA",
    "HERDR_BIN",
    "BROWSE_TTL",
    "DIR",
];

pub fn env(key: &str) -> Option<String> {
    env_os(key).map(|v| v.to_string_lossy().into_owned())
}

/// Is `ALEPH_<key>` (or an old name) set to something truthy?
pub fn env_flag(key: &str) -> bool {
    env(key).is_some_and(|v| !matches!(v.as_str(), "0" | "false" | "no" | "off"))
}

/// The name this process was started as (`aleph`, `claude-herdr`, `claude`, …).
pub fn invoked_as() -> String {
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
        .unwrap_or_else(|| BIN.into())
}

/// Wrapper mode: this binary stands in for an agent's own command, so whatever
/// isn't an Aleph command goes to that agent (the `wraps` setting). It's on
/// when the opt-in `claude` shell function sets `ALEPH_WRAP=1`, or when the
/// binary was started under an agent's name (a `claude` symlink).
pub fn wrapper_mode() -> bool {
    if let Some(v) = std::env::var_os("ALEPH_WRAP") {
        return !v.is_empty() && v != "0";
    }
    let me = invoked_as();
    me != BIN && me != LEGACY && me != ALIAS && is_agent_program(&me)
}

/// Is `name` an agent's own program (`claude`, `codex`) — not just an alias
/// like `cc`?
pub fn is_agent_program(name: &str) -> bool {
    crate::agents::ALL.iter().any(|a| a.exes.contains(&name))
}

fn xdg(var: &str, fallback: &[&str]) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = crate::config::home();
            for f in fallback {
                p.push(f);
            }
            p
        })
}

/// The three base folders that hold our config, state and cache.
fn bases() -> [PathBuf; 3] {
    [
        xdg("XDG_CONFIG_HOME", &[".config"]),
        xdg("XDG_STATE_HOME", &[".local", "state"]),
        xdg("XDG_CACHE_HOME", &[".cache"]),
    ]
}

/// What the move did, for the one-line notice.
#[derive(Default, Debug)]
pub struct Moved {
    pub dirs: Vec<String>,
    pub service: Option<String>,
    pub binary: Option<String>,
    pub rc: Option<String>,
    pub problems: Vec<String>,
}

impl Moved {
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty() && self.service.is_none() && self.binary.is_none() && self.rc.is_none()
    }
}

/// Move `base/claude-herdr` to `base/aleph` and leave a symlink behind.
/// Returns what happened (None when there was nothing to do).
pub fn move_dir(base: &Path) -> Result<Option<String>, String> {
    let (old, new) = (base.join(LEGACY), base.join(DIR));
    let Ok(meta) = fs::symlink_metadata(&old) else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        return Ok(None); // already moved
    }
    if !meta.is_dir() {
        return Ok(None);
    }
    if new.exists() {
        // both exist (the new one was created before the move ran): copy over
        // whatever the new folder doesn't have yet, keep the old one as a backup
        merge(&old, &new)?;
        let bak = base.join(format!("{LEGACY}.moved-to-aleph"));
        if !bak.exists() {
            fs::rename(&old, &bak).map_err(|e| format!("{}: {e}", old.display()))?;
            let _ = std::os::unix::fs::symlink(DIR, &old);
        }
        return Ok(Some(format!("{} (merged)", tilde(&new))));
    }
    fs::rename(&old, &new).map_err(|e| format!("{} → {}: {e}", old.display(), new.display()))?;
    std::os::unix::fs::symlink(DIR, &old).map_err(|e| format!("{}: {e}", old.display()))?;
    Ok(Some(tilde(&new)))
}

/// Copy entries from `from` that `to` doesn't have (recursively, files only).
fn merge(from: &Path, to: &Path) -> Result<(), String> {
    let rd = fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for e in rd.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let ft = match e.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            fs::create_dir_all(&dst).map_err(|e| format!("{}: {e}", dst.display()))?;
            merge(&src, &dst)?;
        } else if ft.is_file() && !dst.exists() {
            fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        }
    }
    Ok(())
}

fn tilde(p: &Path) -> String {
    crate::doctor::tilde(p)
}

/// `<dir of this binary>/claude-herdr` → a symlink to `aleph`, so Raycast,
/// scripts and the old service keep running the new code.
pub fn link_legacy_binary(exe: &Path) -> Result<Option<String>, String> {
    if exe.file_name().and_then(|f| f.to_str()) != Some(BIN) {
        return Ok(None);
    }
    let Some(dir) = exe.parent() else {
        return Ok(None);
    };
    let old = dir.join(LEGACY);
    match fs::symlink_metadata(&old) {
        Ok(m) if m.file_type().is_symlink() => Ok(None),
        Ok(m) if m.is_file() => {
            let tmp = dir.join(format!(".{LEGACY}.link.{}", std::process::id()));
            let _ = fs::remove_file(&tmp);
            std::os::unix::fs::symlink(BIN, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
            fs::rename(&tmp, &old).map_err(|e| format!("{}: {e}", old.display()))?;
            Ok(Some(format!("{} → {BIN}", tilde(&old))))
        }
        _ => Ok(None),
    }
}

/// The function names a managed shell block defines (`cc() { … }` → `cc`).
pub fn rc_functions(block: &str) -> Vec<String> {
    block
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let (name, _) = t.split_once("()")?;
            let name = name.trim_start_matches("function ").trim();
            (!name.is_empty() && !name.contains(char::is_whitespace) && !name.starts_with('#'))
                .then(|| name.to_string())
        })
        .collect()
}

/// The new managed block for `exe`. `al` always; the old names stay as
/// deprecated aliases. A `claude` function becomes the opt-in wrapper
/// (`ALEPH_WRAP=1`), so it keeps passing Claude's own commands through.
pub fn rc_block(exe: &str, old_names: &[String], completions: bool) -> String {
    let q = exe
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    let mut s = format!("{RC_BEGIN}\n{ALIAS}() {{ \"{q}\" \"$@\"; }}\n");
    for n in old_names {
        if n == ALIAS || n == BIN {
            continue;
        }
        if is_agent_program(n) {
            s.push_str(&format!(
                "{n}() {{ ALEPH_WRAP=1 \"{q}\" \"$@\"; }}   # Aleph, and {n}'s own commands pass through\n"
            ));
        } else {
            s.push_str(&format!(
                "{n}() {{ ALEPH_VIA={n} \"{q}\" \"$@\"; }}   # old name — use {ALIAS} (or {BIN})\n"
            ));
        }
    }
    if completions {
        s.push_str(&format!(
            "[[ -o interactive && -z $HERDR_ENV ]] && source <(\"{q}\" completions zsh --name {ALIAS} </dev/null 2>/dev/null)\n"
        ));
    }
    s.push_str(RC_END);
    s.push('\n');
    s
}

/// Replace the old managed block (or an existing Aleph block) in `rc` text.
/// Returns the new text and whether anything changed.
pub fn rewrite_rc(rc: &str, exe: &str) -> Option<String> {
    let (b, e) = (LEGACY_RC_BEGIN, LEGACY_RC_END);
    let start = rc.lines().position(|l| l.trim() == b)?;
    let lines: Vec<&str> = rc.lines().collect();
    let end = lines[start..]
        .iter()
        .position(|l| l.trim() == e)
        .map(|i| start + i)?;
    let block = lines[start..=end].join("\n");
    let names = rc_functions(&block);
    let completions = block.contains("completions zsh");
    let mut out = String::new();
    for l in &lines[..start] {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&rc_block(exe, &names, completions));
    for l in &lines[end + 1..] {
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}

fn zshrc() -> PathBuf {
    std::env::var_os("ZDOTDIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::config::home)
        .join(".zshrc")
}

fn migrate_rc(exe: &Path) -> Result<Option<String>, String> {
    let rc = zshrc();
    let Ok(text) = fs::read_to_string(&rc) else {
        return Ok(None);
    };
    let Some(new) = rewrite_rc(&text, &exe.to_string_lossy()) else {
        return Ok(None);
    };
    let stamp = crate::cache::now();
    let bak = rc.with_file_name(format!(".zshrc.bak.aleph.{stamp}"));
    fs::write(&bak, &text).map_err(|e| format!("{}: {e}", bak.display()))?;
    crate::config::write_atomic(&rc, new.as_bytes())
        .map_err(|e| format!("{}: {e}", rc.display()))?;
    Ok(Some(format!("{} (backup {})", tilde(&rc), tilde(&bak))))
}

/// The one-time move. Cheap when there's nothing to do (a few `lstat`s), so it
/// runs at the start of every command; `ALEPH_NO_MIGRATE=1` skips it.
pub fn migrate() -> Moved {
    let mut m = Moved::default();
    if env_flag("NO_MIGRATE") || crate::service::running_as_legacy_service() {
        return m;
    }
    // nothing old left? (the common case: three lstats and we're done)
    let legacy_left = bases()
        .iter()
        .any(|b| fs::symlink_metadata(b.join(LEGACY)).is_ok_and(|md| md.is_dir()));
    let exe = std::env::current_exe().ok();
    let bin_left = exe.as_ref().is_some_and(|e| {
        e.file_name().and_then(|f| f.to_str()) == Some(BIN)
            && e.parent()
                .and_then(|d| fs::symlink_metadata(d.join(LEGACY)).ok())
                .is_some_and(|md| md.is_file())
    });
    let rc_left = fs::read_to_string(zshrc()).is_ok_and(|t| t.contains(LEGACY_RC_BEGIN));
    let svc_left = crate::config::home()
        .join("Library/LaunchAgents")
        .join(format!("{LEGACY_LABEL}.plist"))
        .exists()
        || xdg("XDG_CONFIG_HOME", &[".config"])
            .join("systemd/user")
            .join(LEGACY_UNIT)
            .exists();
    if !(legacy_left || bin_left || rc_left || svc_left) {
        return m;
    }
    // one migration at a time (a re-exec'd old watch could race us)
    let lock =
        std::env::temp_dir().join(format!("aleph-migrate-{}.lock", crate::sshcfg::uid_str()));
    if let Ok(meta) = fs::metadata(&lock) {
        let fresh = meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() < 120);
        if fresh {
            return m;
        }
        let _ = fs::remove_file(&lock);
    }
    if fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .is_err()
    {
        return m;
    }
    for base in bases() {
        match move_dir(&base) {
            Ok(Some(d)) => m.dirs.push(d),
            Ok(None) => {}
            Err(e) => m.problems.push(e),
        }
    }
    if let Some(exe) = &exe {
        match link_legacy_binary(exe) {
            Ok(b) => m.binary = b,
            Err(e) => m.problems.push(e),
        }
        match migrate_rc(exe) {
            Ok(r) => m.rc = r,
            Err(e) => m.problems.push(e),
        }
    }
    match crate::service::migrate_legacy() {
        Ok(s) => m.service = s,
        Err(e) => m.problems.push(e),
    }
    let _ = fs::remove_file(&lock);
    if !m.is_empty() || !m.problems.is_empty() {
        crate::log::info("rename", &format!("claude-herdr → Aleph: {m:?}"));
        let _ = fs::create_dir_all(crate::config::state_dir());
        let note = crate::config::state_dir().join("renamed-from-claude-herdr");
        let _ = fs::write(&note, format!("{m:?}\n"));
    }
    m
}

/// One stderr line about the move, the first time (only on a terminal).
pub fn announce(m: &Moved) {
    use std::io::IsTerminal;
    if m.is_empty() && m.problems.is_empty() {
        return;
    }
    if !std::io::stderr().is_terminal() {
        return;
    }
    let mut what: Vec<String> = Vec::new();
    if !m.dirs.is_empty() {
        what.push("config, state and cache".into());
    }
    if m.service.is_some() {
        what.push("the background service".into());
    }
    if m.rc.is_some() {
        what.push(format!("your ~/.zshrc block (`{ALIAS}` added)"));
    }
    if m.binary.is_some() {
        what.push(format!("`{LEGACY}` now runs `{BIN}`"));
    }
    if !what.is_empty() {
        crate::out::ok(&format!(
            "{MARK} claude-herdr is now {NAME} — moved {} (old paths still work)",
            what.join(", ")
        ));
    }
    for p in &m.problems {
        crate::out::warn(&format!("rename: {p}"));
    }
}

/// A deprecation nudge when started under an old name (at most once a day).
pub fn nudge_old_name() {
    use std::io::IsTerminal;
    let via = std::env::var("ALEPH_VIA").ok().filter(|v| !v.is_empty());
    let me = invoked_as();
    let old = via.or_else(|| (me == LEGACY).then(|| me.clone()));
    let Some(old) = old else {
        return;
    };
    if !std::io::stderr().is_terminal() {
        return;
    }
    let stamp = crate::config::state_dir().join("old-name-nudged");
    let today = crate::cache::now() / 86_400;
    let last: u64 = fs::read_to_string(&stamp)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if last == today {
        return;
    }
    let _ = fs::create_dir_all(crate::config::state_dir());
    let _ = fs::write(&stamp, today.to_string());
    crate::out::hint(&format!(
        "`{old}` is an old name for {NAME} — type `{ALIAS}` (or `{BIN}`); the old name goes away in the next release"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_block_keeps_old_names_and_adds_al() {
        let rc = "export X=1\n# >>> claude-herdr >>>\ncc() { \"/u/.cargo/bin/claude-herdr\" \"$@\"; }\n[[ -o interactive && -z $HERDR_ENV ]] && source <(\"/u/.cargo/bin/claude-herdr\" completions zsh --name cc </dev/null 2>/dev/null)\n# <<< claude-herdr <<<\nalias ll=ls\n";
        let new = rewrite_rc(rc, "/u/.cargo/bin/aleph").unwrap();
        assert!(new.starts_with(
            "export X=1\n# >>> aleph >>>\nal() { \"/u/.cargo/bin/aleph\" \"$@\"; }\n"
        ));
        assert!(new.contains("cc() { ALEPH_VIA=cc \"/u/.cargo/bin/aleph\" \"$@\"; }   # old name"));
        assert!(new.contains("completions zsh --name al"));
        assert!(new.ends_with("# <<< aleph <<<\nalias ll=ls\n"));
        assert!(!new.contains("claude-herdr"));
        // a second run finds nothing to do
        assert!(rewrite_rc(&new, "/u/.cargo/bin/aleph").is_none());
    }

    #[test]
    fn a_claude_function_becomes_the_wrapper() {
        let rc = "# >>> claude-herdr >>>\nclaude() { \"/b/claude-herdr\" \"$@\"; }\n# <<< claude-herdr <<<\n";
        let new = rewrite_rc(rc, "/b/aleph").unwrap();
        assert!(new.contains("claude() { ALEPH_WRAP=1 \"/b/aleph\" \"$@\"; }"));
        assert!(new.contains("al() { \"/b/aleph\" \"$@\"; }"));
        assert!(!new.contains("completions"));
    }

    #[test]
    fn env_falls_back_to_the_old_names_only() {
        std::env::set_var("CC_SSH_REUSE", "7");
        std::env::set_var("CC_FLAGS_X_ALEPH_TEST", "1");
        assert_eq!(env("SSH_REUSE").as_deref(), Some("7"));
        assert_eq!(env("FLAGS_X_ALEPH_TEST"), None);
        std::env::set_var("ALEPH_SSH_REUSE", "9");
        assert_eq!(env("SSH_REUSE").as_deref(), Some("9"));
        for k in ["CC_SSH_REUSE", "CC_FLAGS_X_ALEPH_TEST", "ALEPH_SSH_REUSE"] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn functions_in_a_block() {
        assert_eq!(
            rc_functions("cc() { x; }\nfunction foo() { y; }\n# bar() comment\n"),
            vec!["cc", "foo"]
        );
    }

    #[test]
    fn dirs_move_once_and_leave_a_link() {
        let d = std::env::temp_dir().join(format!("aleph-mv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(LEGACY).join("sub")).unwrap();
        fs::write(d.join(LEGACY).join("config"), "a=1\n").unwrap();
        fs::write(d.join(LEGACY).join("sub/x"), "x").unwrap();
        assert!(move_dir(&d).unwrap().is_some());
        assert_eq!(
            fs::read_to_string(d.join(DIR).join("config")).unwrap(),
            "a=1\n"
        );
        assert!(fs::symlink_metadata(d.join(LEGACY))
            .unwrap()
            .file_type()
            .is_symlink());
        // through the link, old paths still work
        assert_eq!(
            fs::read_to_string(d.join(LEGACY).join("sub/x")).unwrap(),
            "x"
        );
        assert!(move_dir(&d).unwrap().is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn both_dirs_existing_merges_without_overwriting() {
        let d = std::env::temp_dir().join(format!("aleph-mg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(LEGACY)).unwrap();
        fs::create_dir_all(d.join(DIR)).unwrap();
        fs::write(d.join(LEGACY).join("events.jsonl"), "old\n").unwrap();
        fs::write(d.join(LEGACY).join("config"), "old=1\n").unwrap();
        fs::write(d.join(DIR).join("config"), "new=1\n").unwrap();
        assert!(move_dir(&d).unwrap().unwrap().contains("merged"));
        assert_eq!(
            fs::read_to_string(d.join(DIR).join("config")).unwrap(),
            "new=1\n"
        );
        assert_eq!(
            fs::read_to_string(d.join(DIR).join("events.jsonl")).unwrap(),
            "old\n"
        );
        assert!(d.join(format!("{LEGACY}.moved-to-aleph")).is_dir());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn old_binary_becomes_a_link() {
        let d = std::env::temp_dir().join(format!("aleph-bin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(BIN), "new").unwrap();
        fs::write(d.join(LEGACY), "old").unwrap();
        assert!(link_legacy_binary(&d.join(BIN)).unwrap().is_some());
        assert_eq!(fs::read_to_string(d.join(LEGACY)).unwrap(), "new");
        assert!(link_legacy_binary(&d.join(BIN)).unwrap().is_none());
        // not run as `aleph`: leave things alone
        assert!(link_legacy_binary(&d.join("other")).unwrap().is_none());
        let _ = fs::remove_dir_all(&d);
    }
}
