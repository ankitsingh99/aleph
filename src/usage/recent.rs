//! Which conversation a pane holds, when herdr doesn't say: Codex 0.164 runs
//! its hooks in its app-server daemon, away from the pane's environment, so
//! herdr's session hook never reports the thread id and every view keyed by
//! it (turns, timeline, live numbers, row bars) stayed empty. The usage store
//! knows each conversation's machine, agent and folder: the one a pane holds
//! is the newest main conversation of its agent in its folder there.
//!
//! Read from the newest part of the store only (the last few MB of this
//! month's records and steps), again only when they change. Helper threads
//! (subagents: `sl` lanes; Codex's guardian reviews, model
//! `codex-auto-review`) never stand for a pane. Two panes of one agent in
//! one folder are left alone: which is which isn't known.

use super::store;
use crate::herdr::Session;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How much of the newest records and steps files is read.
const TAIL: u64 = 4 << 20;
/// A conversation quiet longer than this isn't taken for a pane's.
const FRESH: u64 = 2 * 86_400;

/// One conversation as the newest records show it.
#[derive(Clone, Debug, PartialEq)]
pub struct Conv {
    pub id: String,
    pub last: u64,
}

/// (host, agent, folder as recorded) → its main conversations.
#[derive(Default)]
pub struct Index {
    by: HashMap<(String, String, String), Vec<Conv>>,
}

impl Index {
    /// From record lines and step lines (pure — tested).
    pub fn build(records: &str, steps: &str) -> Index {
        let mut child: HashSet<&str> = HashSet::new();
        for l in steps.lines() {
            let f: Vec<&str> = l.split('\t').collect();
            if f.first() == Some(&"sl") && f.len() > 5 {
                child.insert(f[5]);
            }
        }
        let mut by: HashMap<(String, String, String), HashMap<String, u64>> = HashMap::new();
        for l in records.lines() {
            let f: Vec<&str> = l.split('\t').collect();
            if f.first() != Some(&"u") || f.len() < 9 {
                continue;
            }
            let (ts, host, agent, id, model, folder) = (f[1], f[2], f[3], f[4], f[6], f[8]);
            if id.is_empty()
                || folder.is_empty()
                || child.contains(id)
                || model == "codex-auto-review"
            {
                continue;
            }
            let ts: u64 = ts.parse().unwrap_or(0);
            let e = by
                .entry((host.into(), agent.into(), folder.into()))
                .or_default()
                .entry(id.into())
                .or_insert(0);
            *e = (*e).max(ts);
        }
        Index {
            by: by
                .into_iter()
                .map(|(k, m)| {
                    let mut v: Vec<Conv> =
                        m.into_iter().map(|(id, last)| Conv { id, last }).collect();
                    v.sort_by(|a, b| b.last.cmp(&a.last).then(a.id.cmp(&b.id)));
                    (k, v)
                })
                .collect(),
        }
    }

    /// The newest main conversation of `agent` in `cwd` on `host` (a
    /// recorded `~/x` folder matches any home's `…/x`), if it's fresh.
    pub fn newest(&self, host: &str, agent: &str, cwd: &str, now: u64) -> Option<&Conv> {
        self.by
            .iter()
            .filter(|((h, a, f), _)| h == host && a == agent && same_folder(f, cwd))
            .flat_map(|(_, v)| v.first())
            .filter(|c| now.saturating_sub(c.last) <= FRESH)
            .max_by_key(|c| c.last)
    }
}

/// A recorded folder (`~/src/x` or `/srv/x`) against a pane's cwd.
fn same_folder(recorded: &str, cwd: &str) -> bool {
    let cwd = cwd.trim_end_matches('/');
    let r = recorded.trim_end_matches('/');
    match r.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            // the home is the part before: /home/u, /Users/u, /root
            cwd.strip_suffix(rest).is_some_and(|home| {
                let parts: Vec<&str> = home.split('/').filter(|p| !p.is_empty()).collect();
                matches!(
                    parts.as_slice(),
                    ["root"] | ["home", _] | ["Users", _] | ["var", "home", _]
                )
            })
        }
        _ => r == cwd,
    }
}

/// Fill in `native_id` for the sessions herdr gave none, from the store.
pub fn fill(host: &str, v: &mut [Session]) {
    if v.iter().all(|s| s.native_id.is_some()) {
        return;
    }
    let now = crate::cache::now();
    with_index(|ix| {
        // two panes of one agent in one folder: which is which isn't known
        let mut n: HashMap<(String, String), usize> = HashMap::new();
        for s in v.iter() {
            *n.entry((s.agent.clone(), s.cwd.clone())).or_default() += 1;
        }
        for s in v
            .iter_mut()
            .filter(|s| s.native_id.is_none() && !s.cwd.is_empty())
        {
            if n.get(&(s.agent.clone(), s.cwd.clone())) != Some(&1) {
                continue;
            }
            if let Some(c) = ix.newest(host, &s.agent, &s.cwd, now) {
                s.native_id = Some(c.id.clone());
            }
        }
    });
}

/// The files an index was built from: (path, mtime, size) each.
type Stamp = Vec<(PathBuf, u64, u64)>;

/// The index of the newest store files, built again when they change.
fn with_index(f: impl FnOnce(&Index)) {
    static CACHE: Mutex<Option<(Stamp, Index)>> = Mutex::new(None);
    let files: Vec<PathBuf> = ["records-", "steps-"]
        .iter()
        .filter_map(|p| newest(&store::dir(), p))
        .collect();
    let stamp: Stamp = files
        .iter()
        .map(|p| {
            let m = std::fs::metadata(p).ok();
            let mt = m
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            (p.clone(), mt, m.map_or(0, |m| m.len()))
        })
        .collect();
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|(s, _)| *s != stamp) {
        let text = |prefix: &str| {
            files
                .iter()
                .find(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
                })
                .map(|p| tail(p))
                .unwrap_or_default()
        };
        *g = Some((stamp, Index::build(&text("records-"), &text("steps-"))));
    }
    if let Some((_, ix)) = g.as_ref() {
        f(ix);
    }
}

/// `<dir>/<prefix>YYYY-MM.tsv`, the newest month.
fn newest(dir: &Path, prefix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".tsv"))
        })
        .max()
}

/// The last `TAIL` bytes of a file, from its first whole line.
fn tail(p: &Path) -> String {
    let Ok(mut f) = std::fs::File::open(p) else {
        return String::new();
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let from = len.saturating_sub(TAIL);
    if f.seek(SeekFrom::Start(from)).is_err() {
        return String::new();
    }
    let mut b = Vec::new();
    let _ = f.read_to_end(&mut b);
    let s = String::from_utf8_lossy(&b).into_owned();
    if from == 0 {
        return s;
    }
    s.split_once('\n')
        .map(|(_, r)| r.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(ts: u64, host: &str, agent: &str, id: &str, model: &str, folder: &str) -> String {
        format!("u\t{ts}\t{host}\t{agent}\t{id}\tk{ts}\t{model}\thigh\t{folder}\t1\t2\t0\t0\t0\t0\t\t\t0\n")
    }

    #[test]
    fn a_pane_gets_its_folders_newest_main_conversation() {
        let now = 1_793_000_000;
        let recs = [
            u(now - 900, "b31", "codex", "old-1", "gpt-6", "~/px"),
            u(now - 60, "b31", "codex", "main-2", "gpt-6", "~/px"),
            // its guardian review and a subagent: never a pane's
            u(
                now - 10,
                "b31",
                "codex",
                "guard-3",
                "codex-auto-review",
                "~/px",
            ),
            u(now - 5, "b31", "codex", "kid-4", "gpt-6", "~/px"),
            u(now - 30, "b31", "claude", "cl-5", "opus", "~/px"),
            u(now - 30, "gpu2", "codex", "other-6", "gpt-6", "~/px"),
        ]
        .concat();
        let steps = "sl\t1793000000000\tb31\tcodex\tmain-2\tkid-4\tk\tworker\t\t0\t0\n";
        let ix = Index::build(&recs, steps);
        let id = |h: &str, a: &str, cwd: &str| ix.newest(h, a, cwd, now).map(|c| c.id.clone());
        assert_eq!(
            id("b31", "codex", "/home/selene/px").as_deref(),
            Some("main-2")
        );
        assert_eq!(
            id("b31", "codex", "/Users/me/px/").as_deref(),
            Some("main-2")
        );
        assert_eq!(
            id("b31", "claude", "/home/selene/px").as_deref(),
            Some("cl-5")
        );
        assert_eq!(id("gpu2", "codex", "/root/px").as_deref(), Some("other-6"));
        // another folder, another machine's home shape, a stale one: none
        assert_eq!(id("b31", "codex", "/home/selene/px2"), None);
        assert_eq!(id("b31", "codex", "/opt/x/px"), None);
        assert_eq!(
            ix.newest("b31", "codex", "/home/selene/px", now + 3 * 86_400),
            None
        );
    }

    #[test]
    fn folders_match_by_home() {
        assert!(same_folder("~/work/x", "/Users/me/work/x"));
        assert!(same_folder("~", "/home/me"));
        assert!(same_folder("/srv/x", "/srv/x/"));
        assert!(!same_folder("~/x", "/srv/me/x"));
        assert!(!same_folder("~/x", "/home/me/y/x"));
    }

    #[test]
    fn two_panes_in_one_folder_are_left_alone() {
        let s = |pane: &str, agent: &str, id: Option<&str>| Session {
            pane_id: pane.into(),
            agent: agent.into(),
            cwd: "/home/u/px".into(),
            native_id: id.map(str::to_string),
            ..Default::default()
        };
        let mut v = vec![
            s("w1:p1", "claude", Some("given")),
            s("w1:p2", "codex", None),
            s("w1:p3", "codex", None),
        ];
        fill("nowhere-host", &mut v);
        assert_eq!(v[0].native_id.as_deref(), Some("given"));
        assert!(v[1].native_id.is_none() && v[2].native_id.is_none());
    }
}
