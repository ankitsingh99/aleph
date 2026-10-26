//! Projects: one repo is one project, wherever it was worked on — a folder
//! on two machines, a worktree, a subfolder (`projects.json` in the usage
//! store).
//!
//! * **Which repo a folder is in**: one `sh -s` per machine over the folders
//!   its requests ran in that aren't known yet (or not for a week): git's
//!   toplevel and common dir (worktrees share it), the remote (origin, else
//!   upstream, else the first) with any user and password taken off **on the
//!   machine**, and the first commit (cached per repo). A folder that's gone
//!   keeps its last answer, and is never entered.
//! * **The project's id**: the remote, normalised (`github.com/owner/repo`:
//!   scheme, user, port, `.git` and case gone; scp form folded; an ssh host
//!   alias resolved by `ssh -G` there); with no remote, the first commit —
//!   which joins it to the one remote project with that first commit; a
//!   folder outside git is its own. Forks (other remotes) stay apart.
//! * **Rules** (`analytics.projects.<name>` = remotes, `machine:folder`s,
//!   `root:<sha>`) put any of these under one name.
//!
//! The answers are looked up when a query runs, so a rule or a new answer
//! applies at once.

use super::store;
use crate::config::Config;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

/// What git said of a folder on a machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folder {
    /// the repo's top folder ("" not in a repo)
    pub top: String,
    /// the repo's common git dir (a worktree's is its main repo's)
    pub common: String,
    /// the remote as the machine had it, user and password taken off
    pub remote: String,
    /// the first commit ("" none known: shallow, empty, not a repo)
    pub root: String,
    /// when it was asked (unix seconds)
    pub at: u64,
    /// the folder wasn't there
    pub gone: bool,
}

/// Every folder's answer: `host\tfolder` → what git said.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    pub folders: BTreeMap<String, Folder>,
}

fn path() -> std::path::PathBuf {
    store::dir().join("projects.json")
}

fn key(host: &str, folder: &str) -> String {
    format!("{host}\t{folder}")
}

impl Book {
    pub fn load() -> Book {
        let v: Value = std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({}));
        let gs = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let folders = v
            .get("folders")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, f)| {
                (
                    k.clone(),
                    Folder {
                        top: gs(f, "top"),
                        common: gs(f, "common"),
                        remote: gs(f, "remote"),
                        root: gs(f, "root"),
                        at: f.get("at").and_then(Value::as_u64).unwrap_or(0),
                        gone: f.get("gone").and_then(Value::as_bool).unwrap_or(false),
                    },
                )
            })
            .collect();
        Book { folders }
    }

    pub fn save(&self) {
        let folders: serde_json::Map<String, Value> = self
            .folders
            .iter()
            .map(|(k, f)| {
                (
                    k.clone(),
                    json!({"top": f.top, "common": f.common, "remote": f.remote, "root": f.root,
                           "at": f.at, "gone": f.gone}),
                )
            })
            .collect();
        let v = json!({"v": 1, "folders": folders});
        let _ = crate::config::write_atomic(&path(), v.to_string().as_bytes());
    }

    pub fn get(&self, host: &str, folder: &str) -> Option<&Folder> {
        self.folders.get(&key(host, folder))
    }

    /// A machine's folders worth asking about: never asked, or a week ago.
    pub fn due(&self, host: &str, folders: &BTreeSet<String>, now: u64) -> Vec<String> {
        folders
            .iter()
            .filter(|f| is_path(f))
            .filter(|f| {
                self.get(host, f)
                    .is_none_or(|x| now.saturating_sub(x.at) > 7 * 86_400)
            })
            .take(200)
            .cloned()
            .collect()
    }

    /// The first commit known for a repo's common dir on a machine (it never changes).
    fn root_of(&self, host: &str, common: &str) -> Option<&str> {
        self.folders
            .iter()
            .filter(|(k, _)| k.split('\t').next() == Some(host))
            .map(|(_, f)| f)
            .find(|f| f.common == common && !f.root.is_empty())
            .map(|f| f.root.as_str())
    }

    /// What a run said: answers noted; a folder that's gone keeps its last answer.
    pub fn apply(&mut self, host: &str, answers: Vec<(String, Folder)>) {
        for (folder, mut f) in answers {
            let k = key(host, &folder);
            if f.gone {
                if let Some(old) = self.folders.get_mut(&k) {
                    old.gone = true;
                    old.at = f.at;
                    continue;
                }
            }
            if f.root.is_empty() && !f.common.is_empty() {
                if let Some(r) = self.root_of(host, &f.common) {
                    f.root = r.to_string();
                }
            }
            self.folders.insert(k, f);
        }
    }
}

/// A folder as records carry it: `~/…` or absolute (Gemini's are names, not paths).
pub fn is_path(f: &str) -> bool {
    f == "~" || f.starts_with("~/") || f.starts_with('/')
}

// ------------------------------------------------------------------ the look

/// A folder for `sh`: `'/srv/x'`, `"$HOME"/'x'` for `~/x`.
pub(crate) fn sh_path(p: &str) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    match p.strip_prefix("~/") {
        Some(r) => format!("\"$HOME\"/{}", q(r)),
        None if p == "~" => "\"$HOME\"".into(),
        None => q(p),
    }
}

/// The look at these folders: one `sh -s` for a machine (pure — tested).
/// `known`: repos (common dirs) whose first commit is known already.
/// Prints `G <n> <top> <common> <remote> <root>` per folder (its index), `G
/// <n> -` for one that isn't there; nothing for one outside git.
pub fn script(folders: &[String], known: &[String]) -> String {
    let mut s = String::from(crate::chan::PATH_PRELUDE);
    s.push_str(
        "\nLC_ALL=C; export LC_ALL; GIT_OPTIONAL_LOCKS=0; export GIT_OPTIONAL_LOCKS; GIT_TERMINAL_PROMPT=0; export GIT_TERMINAL_PROMPT\n\
         command -v git >/dev/null 2>&1 || exit 0\n\
         # a Mac without the developer tools: /usr/bin/git would open an install dialog\n\
         if [ \"$(uname)\" = Darwin ] && ! xcode-select -p >/dev/null 2>&1; then exit 0; fi\n\
         T=$(printf '\\t')\n\
         KNOWN='",
    );
    for k in known
        .iter()
        .filter(|k| !k.contains('\'') && !k.contains('\n'))
    {
        s.push_str(&format!("|{k}"));
    }
    s.push_str(
        "|'\n\
         # a remote with no user or password in it, and an ssh host alias as its real name\n\
         clean() {\n\
         \x20 u=$(printf '%s' \"$1\" | sed -e 's#^\\([a-zA-Z][a-zA-Z0-9+.-]*://\\)[^/@]*@#\\1#' -e 's#[?#].*$##')\n\
         \x20 case $u in\n\
         \x20   *://*) printf '%s' \"$u\" ;;\n\
         \x20   *:*) h=${u%%:*}; h=${h#*@}; r=${u#*:}\n\
         \x20        case $h in *.*) ;; *) n=$(ssh -G \"$h\" 2>/dev/null | awk '$1==\"hostname\"{print $2; exit}'); [ -n \"$n\" ] && h=$n ;; esac\n\
         \x20        printf '%s:%s' \"$h\" \"$r\" ;;\n\
         \x20   *) printf '%s' \"$u\" ;;\n\
         \x20 esac\n\
         }\n\
         look() {\n\
         \x20 [ -d \"$2\" ] || { printf 'G\\t%s\\t-\\n' \"$1\"; return; }\n\
         \x20 cd \"$2\" 2>/dev/null || return\n\
         \x20 top=$(git rev-parse --show-toplevel 2>/dev/null) || return\n\
         \x20 # the home as a repo (dotfiles) holds everything under it: not a project\n\
         \x20 [ \"$top\" = \"$HOME\" ] && return\n\
         \x20 c=$(git rev-parse --git-common-dir 2>/dev/null); c=$(cd \"$c\" 2>/dev/null && pwd -P)\n\
         \x20 rem=$(git config --get remote.origin.url 2>/dev/null || git config --get remote.upstream.url 2>/dev/null)\n\
         \x20 [ -n \"$rem\" ] || { r1=$(git remote 2>/dev/null | head -n 1); [ -n \"$r1\" ] && rem=$(git config --get \"remote.$r1.url\" 2>/dev/null); }\n\
         \x20 rem=$(clean \"$rem\" | tr -d '\\t\\n')\n\
         \x20 root=\n\
         \x20 case $KNOWN in *\"|$c|\"*) ;; *)\n\
         \x20   if [ \"$(git rev-parse --is-shallow-repository 2>/dev/null)\" != true ] && [ ! -f \"$c/shallow\" ]; then\n\
         \x20     root=$(git rev-list --max-parents=0 --first-parent HEAD 2>/dev/null | tail -n 1)\n\
         \x20   fi ;;\n\
         \x20 esac\n\
         \x20 printf 'G\\t%s\\t%s\\t%s\\t%s\\t%s\\n' \"$1\" \"$top\" \"$c\" \"$rem\" \"$root\"\n\
         }\n",
    );
    for (i, f) in folders.iter().enumerate() {
        if f.contains('\n') {
            continue;
        }
        s.push_str(&format!("( look {i} {} )\n", sh_path(f)));
    }
    s
}

/// What [`script`] printed → each folder's answer (pure — tested). `home`:
/// the machine's home, so tops come back as `~/…` like the records' folders.
pub fn parse(text: &str, folders: &[String], home: &str, now: u64) -> Vec<(String, Folder)> {
    let tilde = |p: &str| -> String {
        let h = home.trim_end_matches('/');
        if !h.is_empty() && (p == h || p.starts_with(&format!("{h}/"))) {
            format!("~{}", &p[h.len()..])
        } else {
            p.to_string()
        }
    };
    let mut out: HashMap<usize, Folder> = HashMap::new();
    for l in text.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.first() != Some(&"G") || f.len() < 3 {
            continue;
        }
        let Ok(i) = f[1].parse::<usize>() else {
            continue;
        };
        if f[2] == "-" {
            out.insert(
                i,
                Folder {
                    gone: true,
                    at: now,
                    ..Default::default()
                },
            );
            continue;
        }
        let g = |n: usize| f.get(n).map(|s| s.trim().to_string()).unwrap_or_default();
        let root = g(5);
        out.insert(
            i,
            Folder {
                top: tilde(&g(2)),
                common: g(3),
                remote: g(4),
                root: if root.len() >= 7 && root.chars().all(|c| c.is_ascii_hexdigit()) {
                    root
                } else {
                    String::new()
                },
                at: now,
                gone: false,
            },
        );
    }
    // a folder git said nothing of: outside a repo (asked again in a week)
    folders
        .iter()
        .enumerate()
        .map(|(i, f)| {
            (
                f.clone(),
                out.remove(&i).unwrap_or(Folder {
                    at: now,
                    ..Default::default()
                }),
            )
        })
        .collect()
}

/// Ask a machine about its new folders (blocking: call it off the UI thread).
pub fn resolve(
    host: &str,
    folders: &[String],
    book: &Book,
    home: &str,
) -> Option<Vec<(String, Folder)>> {
    if folders.is_empty() {
        return None;
    }
    if let Some(r) = crate::showcase::projects(host, folders) {
        return Some(r);
    }
    let known: Vec<String> = book
        .folders
        .iter()
        .filter(|(k, f)| k.split('\t').next() == Some(host) && !f.root.is_empty())
        .map(|(_, f)| f.common.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let s = script(folders, &known);
    let o = crate::proc::run(
        &mut crate::sshcfg::shell(host, "sh -s"),
        Duration::from_secs(40),
        Some(s.as_bytes()),
    )
    .ok()?;
    o.ok()
        .then(|| parse(&o.text(), folders, home, crate::cache::now()))
}

// ------------------------------------------------------------------ identity

/// A remote as an id: `github.com/owner/repo` — scheme, user, port, `.git`
/// and trailing slashes gone, the host lowercased (the path too on the big
/// forges, which ignore case); a path on the machine stays that machine's.
pub fn normalize(remote: &str, host: &str) -> Option<String> {
    let r = remote.trim();
    if r.is_empty() {
        return None;
    }
    let (h, path) = if let Some((_, rest)) = r.split_once("://") {
        if r.starts_with("file://") {
            return Some(format!("path:{host}:{}", rest.trim_end_matches('/')));
        }
        let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
        let hostport = auth.rsplit('@').next().unwrap_or(auth);
        let h = match hostport.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
            _ => hostport,
        };
        (h.to_string(), path.to_string())
    } else if r.starts_with('/') || r.starts_with('.') || r.starts_with('~') {
        return Some(format!("path:{host}:{}", r.trim_end_matches('/')));
    } else if let Some((hpart, path)) = r.split_once(':') {
        (
            hpart.rsplit('@').next().unwrap_or(hpart).to_string(),
            path.to_string(),
        )
    } else {
        return None;
    };
    let h = h.to_lowercase();
    if h.is_empty() {
        return None;
    }
    let mut p = path.trim_matches('/').to_string();
    if let Some(x) = p.strip_suffix(".git") {
        p = x.to_string();
    }
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        return None;
    }
    let forge = matches!(
        h.as_str(),
        "github.com" | "gitlab.com" | "bitbucket.org" | "codeberg.org"
    );
    Some(format!(
        "{h}/{}",
        if forge {
            p.to_lowercase()
        } else {
            p.to_string()
        }
    ))
}

/// One project: its id, the name it's shown by, and where it was worked on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    /// `r:github.com/o/r`, `c:<first commit>`, `n:<rule>`, `d:<machine>:<folder>`
    pub id: String,
    pub name: String,
    pub remote: String,
    /// machine → its folders (tops)
    pub on: BTreeMap<String, BTreeSet<String>>,
}

/// Every (machine, folder) → its project, and the projects (built per query).
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub of: HashMap<(String, String), String>,
    pub projects: BTreeMap<String, Project>,
}

/// A rule's matchers: `github.com/x/y` (a remote; `*` globs), `machine:folder`
/// (that folder or under it; `*:folder` any machine), `root:<sha>` (a prefix).
fn rule_hits(
    rule: &str,
    host: &str,
    folder: &str,
    f: Option<&Folder>,
    remote: Option<&str>,
) -> bool {
    crate::usage::list_of(rule).iter().any(|m| {
        if let Some(sha) = m.strip_prefix("root:") {
            return f.is_some_and(|f| !f.root.is_empty() && f.root.starts_with(sha));
        }
        if let Some((h, dir)) = m.split_once(':').filter(|(h, d)| {
            !h.contains('/') && !h.contains('.') && (d.starts_with('/') || d.starts_with('~'))
        }) {
            let top = f
                .map(|f| f.top.as_str())
                .filter(|t| !t.is_empty())
                .unwrap_or(folder);
            return (h == "*" || h == host)
                && [folder, top]
                    .iter()
                    .any(|p| *p == dir || p.starts_with(&format!("{dir}/")));
        }
        remote.is_some_and(|r| crate::permit::matches(&[format!("{}:*", m.to_lowercase())], r, "x"))
    })
}

impl Index {
    /// Build it from the answers, the rules, and the (machine, folder) pairs the rows hold.
    pub fn build(cfg: &Config, book: &Book, pairs: &BTreeSet<(String, String)>) -> Index {
        let rules: Vec<(String, String)> = cfg
            .with_prefix("analytics.projects.")
            .into_iter()
            .filter(|(_, v)| !v.trim().is_empty())
            .collect();
        // each folder's own id where it has one: a rule's, else its remote's
        let mut own: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
        for (host, folder) in pairs {
            let f = book.get(host, folder).filter(|f| !f.top.is_empty());
            let remote = f.and_then(|f| normalize(&f.remote, host));
            let rule = rules
                .iter()
                .find(|(_, r)| rule_hits(r, host, folder, f, remote.as_deref()))
                .map(|(n, _)| n.clone());
            match (rule, &remote) {
                (Some(n), _) => {
                    own.insert((host.clone(), folder.clone()), (format!("n:{n}"), n));
                }
                (None, Some(r)) => {
                    own.insert(
                        (host.clone(), folder.clone()),
                        (format!("r:{r}"), r.clone()),
                    );
                }
                _ => {}
            }
        }
        // the first commit → the projects that have it (to join remote-less clones)
        let mut by_root: HashMap<String, BTreeSet<(String, String)>> = HashMap::new();
        for (k, f) in &book.folders {
            let mut it = k.splitn(2, '\t');
            let (host, folder) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            if f.root.is_empty() {
                continue;
            }
            let id = own
                .get(&(host.to_string(), folder.to_string()))
                .cloned()
                .or_else(|| {
                    let f = book.get(host, folder)?;
                    let r = normalize(&f.remote, host)?;
                    Some((format!("r:{r}"), r))
                });
            if let Some(id) = id {
                by_root.entry(f.root.clone()).or_default().insert(id);
            }
        }
        let mut ix = Index::default();
        for (host, folder) in pairs {
            let f = book.get(host, folder).filter(|f| !f.top.is_empty());
            let remote = f.and_then(|f| normalize(&f.remote, host));
            let (id, name_from) = match (own.get(&(host.clone(), folder.clone())), f) {
                (Some(x), _) => x.clone(),
                (None, Some(f)) if !f.root.is_empty() => match by_root.get(&f.root) {
                    Some(ids) if ids.len() == 1 => ids.iter().next().cloned().unwrap_or_default(),
                    _ => (
                        format!("c:{}", &f.root[..12.min(f.root.len())]),
                        f.top.clone(),
                    ),
                },
                (None, Some(f)) => (format!("d:{host}:{}", f.top), f.top.clone()),
                (None, None) => {
                    if folder.is_empty() {
                        continue;
                    }
                    (format!("d:{host}:{folder}"), folder.clone())
                }
            };
            let p = ix.projects.entry(id.clone()).or_insert_with(|| Project {
                id: id.clone(),
                name: name_from.clone(),
                remote: match id.strip_prefix("r:") {
                    Some(r) => r.to_string(),
                    None => remote.clone().unwrap_or_default(),
                },
                ..Default::default()
            });
            p.on.entry(host.clone())
                .or_default()
                .insert(f.map(|f| f.top.clone()).unwrap_or_else(|| folder.clone()));
            ix.of.insert((host.clone(), folder.clone()), id);
        }
        // names: the repo's own (owner/repo when two share it)
        let short = |p: &Project| -> String {
            if p.id.starts_with("n:") {
                return p.name.clone();
            }
            p.name
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(&p.name)
                .to_string()
        };
        let mut count: HashMap<String, usize> = HashMap::new();
        for p in ix.projects.values() {
            *count.entry(short(p)).or_default() += 1;
        }
        let names: Vec<(String, String)> = ix
            .projects
            .values()
            .map(|p| {
                let s = short(p);
                // a rule's name is the user's: never changed
                let n = if p.id.starts_with("n:") {
                    s
                } else if count[&s] > 1 && !p.remote.is_empty() {
                    p.remote
                        .split_once('/')
                        .map_or(p.remote.as_str(), |x| x.1)
                        .to_string()
                } else if count[&s] > 1 {
                    let host = p.on.keys().next().cloned().unwrap_or_default();
                    format!("{s} ({host})")
                } else {
                    s
                };
                (p.id.clone(), n)
            })
            .collect();
        for (id, n) in names {
            if let Some(p) = ix.projects.get_mut(&id) {
                p.name = n;
            }
        }
        ix
    }

    /// A (machine, folder)'s project id ("" none).
    pub fn id_of(&self, host: &str, folder: &str) -> &str {
        self.of
            .get(&(host.to_string(), folder.to_string()))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// An id, a name (any case), or a remote → a project's id.
    pub fn find(&self, q: &str) -> Option<String> {
        let ql = q.trim().to_lowercase();
        self.projects
            .values()
            .find(|p| p.id == q || p.name.to_lowercase() == ql || p.remote == ql)
            .or_else(|| {
                let hits: Vec<&Project> = self
                    .projects
                    .values()
                    .filter(|p| p.remote.ends_with(&format!("/{ql}")))
                    .collect();
                (hits.len() == 1).then(|| hits[0])
            })
            .map(|p| p.id.clone())
    }

    pub fn name(&self, id: &str) -> String {
        self.projects
            .get(id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| {
                if id.is_empty() {
                    "(no folder)".into()
                } else {
                    id.into()
                }
            })
    }
}

/// The names of the projects the latest queries saw (for a breakdown's
/// labels, where the index isn't at hand).
static NAMES: std::sync::Mutex<Option<HashMap<String, String>>> = std::sync::Mutex::new(None);

/// Keep an index's names for [`name_of`].
pub fn remember(ix: &Index) {
    let mut g = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    for (id, p) in &ix.projects {
        m.insert(id.clone(), p.name.clone());
    }
}

/// A project's name, as the latest query named it (its id if none has).
pub fn name_of(id: &str) -> String {
    if id.is_empty() {
        return "(no folder)".into();
    }
    NAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(id).cloned())
        .unwrap_or_else(|| {
            id.trim_start_matches(|c| c != ':')
                .trim_start_matches(':')
                .to_string()
        })
}

/// Every project with its machines and folders (`usage projects`, the API).
pub fn to_json(ix: &Index) -> Vec<Value> {
    ix.projects
        .values()
        .map(|p| {
            json!({
                "id": p.id, "name": p.name,
                "remote": (!p.remote.is_empty()).then_some(&p.remote),
                "machines": p.on.iter().map(|(h, fs)| json!({"host": h, "folders": fs})).collect::<Vec<_>>(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_normalise_to_one_id() {
        for r in [
            "git@github.com:Acme/App.git",
            "https://github.com/acme/app",
            "https://user:ghp_TOKEN@github.com/Acme/App.git/",
            "ssh://git@GitHub.com:22/Acme/App.git",
            "github.com:acme/app",
        ] {
            assert_eq!(
                normalize(r, "b31").as_deref(),
                Some("github.com/acme/app"),
                "{r}"
            );
        }
        // case kept off the big forges; a path stays that machine's
        assert_eq!(
            normalize("ssh://git@git.corp.example:2222/Team/Svc.git", "b31").as_deref(),
            Some("git.corp.example/Team/Svc")
        );
        assert_eq!(
            normalize("/srv/git/app.git", "b31").as_deref(),
            Some("path:b31:/srv/git/app.git")
        );
        assert_eq!(normalize("", "b31"), None);
        assert_eq!(normalize("nonsense", "b31"), None);
    }

    fn folder(top: &str, common: &str, remote: &str, root: &str) -> Folder {
        Folder {
            top: top.into(),
            common: common.into(),
            remote: remote.into(),
            root: root.into(),
            at: 1,
            gone: false,
        }
    }

    #[test]
    fn one_repo_on_two_machines_a_worktree_and_a_clone_without_a_remote() {
        let root = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
        let mut b = Book::default();
        b.apply(
            "b31",
            vec![
                (
                    "~/src/app".into(),
                    folder(
                        "~/src/app",
                        "/h/src/app/.git",
                        "git@github.com:Acme/App.git",
                        root,
                    ),
                ),
                (
                    "~/src/app/web".into(),
                    folder(
                        "~/src/app",
                        "/h/src/app/.git",
                        "git@github.com:Acme/App.git",
                        "",
                    ),
                ),
                (
                    "~/src/app-wt".into(),
                    folder(
                        "~/src/app-wt",
                        "/h/src/app/.git",
                        "git@github.com:Acme/App.git",
                        "",
                    ),
                ),
            ],
        );
        b.apply(
            "gpu2",
            vec![
                (
                    "/work/app".into(),
                    folder(
                        "/work/app",
                        "/work/app/.git",
                        "https://github.com/acme/app",
                        root,
                    ),
                ),
                (
                    "/work/copy".into(),
                    folder("/work/copy", "/work/copy/.git", "", root),
                ),
                (
                    "/work/fork".into(),
                    folder(
                        "/work/fork",
                        "/work/fork/.git",
                        "git@github.com:me/app.git",
                        "f00d",
                    ),
                ),
                (
                    "/tmp/x".into(),
                    Folder {
                        at: 1,
                        ..Default::default()
                    },
                ),
            ],
        );
        // a subfolder's first commit is its repo's (known already)
        assert_eq!(b.get("b31", "~/src/app/web").unwrap().root, root);
        let pairs: BTreeSet<(String, String)> = [
            ("b31", "~/src/app"),
            ("b31", "~/src/app/web"),
            ("b31", "~/src/app-wt"),
            ("gpu2", "/work/app"),
            ("gpu2", "/work/copy"),
            ("gpu2", "/work/fork"),
            ("gpu2", "/tmp/x"),
        ]
        .iter()
        .map(|(h, f)| (h.to_string(), f.to_string()))
        .collect();
        let ix = Index::build(&Config::in_memory(&[]), &b, &pairs);
        let app = ix.id_of("b31", "~/src/app");
        assert_eq!(app, "r:github.com/acme/app");
        for (h, f) in [
            ("b31", "~/src/app/web"),
            ("b31", "~/src/app-wt"),
            ("gpu2", "/work/app"),
            ("gpu2", "/work/copy"),
        ] {
            assert_eq!(ix.id_of(h, f), app, "{h}:{f}");
        }
        assert_ne!(ix.id_of("gpu2", "/work/fork"), app, "a fork stays apart");
        // a fork sharing the first commit: a clone without a remote can't say whose, so it's its own
        let mut b2 = b.clone();
        b2.apply(
            "gpu2",
            vec![(
                "/work/fork".into(),
                folder(
                    "/work/fork",
                    "/work/fork/.git",
                    "git@github.com:me/app.git",
                    root,
                ),
            )],
        );
        let ix2 = Index::build(&Config::in_memory(&[]), &b2, &pairs);
        assert!(ix2
            .id_of("gpu2", "/work/copy")
            .starts_with("c:a1b2c3d4e5f6"));
        assert_eq!(ix.id_of("gpu2", "/tmp/x"), "d:gpu2:/tmp/x");
        // names: the repo's, owner/repo when two share it
        assert_eq!(ix.name(app), "acme/app");
        assert_eq!(ix.name(ix.id_of("gpu2", "/work/fork")), "me/app");
        let p = &ix.projects[app];
        assert_eq!(p.on.len(), 2, "both machines");
        assert_eq!(ix.find("acme/app").as_deref(), Some(app));
        // a rule puts the fork under the same name
        let cfg =
            Config::in_memory(&["analytics.projects.app=github.com/acme/app,github.com/me/app"]);
        let ix = Index::build(&cfg, &b, &pairs);
        assert_eq!(ix.id_of("gpu2", "/work/fork"), "n:app");
        assert_eq!(ix.id_of("b31", "~/src/app"), "n:app");
        assert_eq!(ix.name("n:app"), "app");
        let cfg = Config::in_memory(&["analytics.projects.scratch=*:/tmp"]);
        assert_eq!(
            Index::build(&cfg, &b, &pairs).id_of("gpu2", "/tmp/x"),
            "n:scratch"
        );
    }

    #[test]
    fn the_look_is_posix_sh_and_its_answers_parse() {
        let s = script(
            &["~/src/app".into(), "/srv/it's".into()],
            &["/h/x/.git".into()],
        );
        assert!(s.contains("( look 0 \"$HOME\"/'src/app' )"));
        assert!(s.contains("'/srv/it'\\''s'"));
        assert!(s.contains("|/h/x/.git|"));
        let o = std::process::Command::new("sh")
            .arg("-n")
            .arg("-c")
            .arg(&s)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let text = "G\t0\t/home/u/src/app\t/home/u/src/app/.git\tgithub.com:Acme/App.git\tabcdef1234567\nG\t1\t-\n";
        let a = parse(
            text,
            &["~/src/app".into(), "/gone".into(), "/plain".into()],
            "/home/u",
            9,
        );
        assert_eq!(a[0].1.top, "~/src/app");
        assert_eq!(a[0].1.root, "abcdef1234567");
        assert!(a[1].1.gone);
        assert_eq!(
            a[2].1,
            Folder {
                at: 9,
                ..Default::default()
            }
        );
        let mut b = Book::default();
        b.apply(
            "h",
            vec![("/gone".into(), folder("/gone", "/gone/.git", "", "abc1234"))],
        );
        b.apply("h", a);
        assert_eq!(
            b.get("h", "/gone").unwrap().top,
            "/gone",
            "a gone folder keeps its answer"
        );
        assert!(b.get("h", "/gone").unwrap().gone);
    }

    #[test]
    fn the_look_against_real_repos() {
        // a repo with a password in its remote, a worktree, a clone with none, the home itself
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let home = std::env::temp_dir().join(format!("aleph-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let sh = |cmd: &str| {
            let o = std::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .env("HOME", &home)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@x")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@x")
                .current_dir(&home)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{cmd}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        std::fs::create_dir_all(&home).unwrap();
        sh("git init -q app && cd app && git commit -q --allow-empty -m one && git remote add origin https://me:SECRET-ghp@github.com/Acme/App.git && git worktree add -q ../app-wt -b wt && mkdir sub && cd .. && git clone -q app copy && cd copy && git remote remove origin");
        let folders: Vec<String> = ["~/app/sub", "~/app-wt", "~/copy", "~/gone"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = script(&folders, &[]);
        let o = std::process::Command::new("sh")
            .arg("-s")
            .env("HOME", &home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(s.as_bytes())?;
                c.wait_with_output()
            })
            .unwrap();
        let text = String::from_utf8_lossy(&o.stdout).to_string();
        assert!(
            !text.contains("SECRET"),
            "the password stays on the machine: {text}"
        );
        let home_s = home.canonicalize().unwrap().to_string_lossy().to_string();
        let a = parse(&text, &folders, &home_s, 1);
        let get = |f: &str| &a.iter().find(|(x, _)| x == f).unwrap().1;
        assert_eq!(get("~/app/sub").top, "~/app");
        assert_eq!(get("~/app/sub").remote, "https://github.com/Acme/App.git");
        assert_eq!(
            get("~/app-wt").common,
            get("~/app/sub").common,
            "a worktree shares its repo"
        );
        assert!(get("~/gone").gone);
        let root = get("~/app/sub").root.clone();
        assert_eq!(root.len(), 40);
        assert_eq!(get("~/copy").root, root);
        let mut b = Book::default();
        b.apply("h", a);
        let pairs: BTreeSet<(String, String)> = folders
            .iter()
            .map(|f| ("h".to_string(), f.clone()))
            .collect();
        let ix = Index::build(&Config::in_memory(&[]), &b, &pairs);
        let id = ix.id_of("h", "~/app/sub");
        assert_eq!(id, "r:github.com/acme/app");
        assert_eq!(ix.id_of("h", "~/app-wt"), id);
        assert_eq!(
            ix.id_of("h", "~/copy"),
            id,
            "no remote: joined by its first commit"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
