//! Which words are private, and what each becomes. Built from what this
//! computer knows — the fleet's aliases and their ssh settings, `$USER` and
//! `$HOME`, git's name and email, the last fleet snapshot (session names and
//! folders), the history, the analytics' machine names — plus what's learned
//! at runtime (`learn`: the folders of agents running outside herdr). Rebuilt when the settings, the snapshot or the mode change.
//! Every process builds the same one: fakes are keyed by the install's salt.

use super::{fake, state};
use crate::showcase::Kind;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

#[derive(Clone, Debug)]
pub struct Entry {
    pub real: String,
    /// its whole-name stand-in, and its same-length one
    pub full: String,
    pub fit: String,
    pub kind: Kind,
}

/// How long stand-ins may be (`showcase.names`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Names {
    /// whole names everywhere; the dashboard lays them into its rows
    Full,
    /// whole names in printed output, JSON, Raycast and the web; the
    /// dashboard keeps the same length
    OutsideTui,
    /// the same length as the real word everywhere
    #[default]
    Fit,
}

impl Names {
    pub fn of(v: &str) -> Names {
        match v.trim() {
            "outside-tui" => Names::OutsideTui,
            "same-length" => Names::Fit,
            _ => Names::Full,
        }
    }
    /// do text and JSON get whole names?
    pub fn text_full(self) -> bool {
        self != Names::Fit
    }
    /// does the dashboard?
    pub fn tui_full(self) -> bool {
        self == Names::Full
    }
}

#[derive(Default, Debug)]
pub struct Dict {
    /// longest first
    pub entries: Vec<Entry>,
    /// lowercase real → entry
    by_real: HashMap<String, usize>,
    /// lowercase fake → real
    by_fake: HashMap<String, String>,
    /// lowercase first two bytes → entries, longest first
    index: HashMap<[u8; 2], Vec<usize>>,
    pub salt: Vec<u8>,
    /// this computer's user and home
    pub user: String,
    pub home: String,
    /// what each field's words become, and how long they may be
    pub pools: fake::Pools,
    pub names: Names,
}

/// Words that say nothing about anyone: never masked.
#[rustfmt::skip]
const STOP: &[&str] = &[
    // folders everyone has
    "src", "lib", "bin", "test", "tests", "testing", "docs", "doc", "app", "apps", "home", "users", "user",
    "tmp", "var", "etc", "opt", "usr", "work", "projects", "project", "code", "dev", "repos", "repo", "git",
    "main", "master", "local", "root", "data", "build", "target", "dist", "node_modules", "vendor",
    "scripts", "config", "configs", "packages", "crates", "cmd", "internal", "pkg", "web", "api", "server",
    "client", "assets", "public", "private", "share", "cache", "state", "log", "logs", "library", "desktop",
    "documents", "downloads", "applications", "volumes", "mnt", "media", "srv", "folder", "folders", "new",
    "old", "tmp", "temp", "backup", "backups", "archive", "notes", "readme", "examples", "example", "demo",
    "fake", "e2e", "fixtures", "samples", "sample", "experiments", "scratch", "playground", "sandbox",
    // machines and what's in them
    "gpu", "gpus", "cpu", "cpus", "ram", "mem", "memory", "disk", "disks", "net", "network", "vram", "core",
    "cores", "power", "fan", "temp", "load", "usage", "procs", "process", "processes", "threads", "model",
    "models", "box", "pc", "mac", "macbook", "imac", "air", "pro", "mini", "studio", "node", "worker", "host",
    "hosts", "lab", "rig", "vm", "docker", "cloud", "cluster", "port", "pool", "train", "training", "eval",
    "inference", "run", "runs", "job", "jobs", "prod", "staging", "stage", "office", "research",
    // systems and tools
    "shell", "herdr", "aleph", "ssh", "linux", "ubuntu", "debian", "fedora", "macos", "darwin", "nvidia",
    "amd", "intel", "apple", "windows", "arch", "alpine", "centos", "rocky", "tmux", "github", "gitlab",
    "issues", "pull", "this", "computer", "the", "and", "for", "with", "com", "net", "org", "io", "ts",
    "lan", "localhost", "none", "true", "false", "null", "x86_64", "arm64", "aarch64",
    // agents and their words
    "claude", "codex", "opencode", "gemini", "cursor", "copilot", "agent", "agents", "session", "sessions",
    "workspace", "worktree", "worktrees", "tab", "pane", "window", "default",
];

fn stopped(w: &str) -> bool {
    let l = w.to_lowercase();
    l.len() < 2
        || STOP.contains(&l.as_str())
        || crate::agents::ALL.iter().any(|a| a.id == l)
        || l.bytes().all(|b| b.is_ascii_digit())
        || crate::names::generated(&l)
        // the demo world's own words are made up already
        || (super::demo() && super::demo::owns(&l))
}

impl Dict {
    fn add(&mut self, kind: Kind, real: &str) {
        let real = real.trim();
        if stopped(real) {
            return;
        }
        let key = real.to_lowercase();
        if let Some(&i) = self.by_real.get(&key) {
            // a word that's two kinds is the more telling one (a host over a folder)
            if kind < self.entries[i].kind {
                self.entries[i].kind = kind;
            }
            return;
        }
        // never someone else's real word (nor one in a part of it), never
        // another's fake
        let free = |d: &Dict, f: &str| {
            let lf = f.to_lowercase();
            lf != key
                && !d.by_real.contains_key(&lf)
                && !d.by_fake.contains_key(&lf)
                && !lf
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|p| p.len() > 1 && d.by_real.contains_key(p))
        };
        let pick = |d: &Dict, make: &dyn Fn(u32) -> String| {
            let mut n = 0;
            loop {
                let f = make(n);
                if free(d, &f) || n > 40 {
                    break f;
                }
                n += 1;
            }
        };
        let fit = pick(self, &|n| fake::fit(&self.salt, &self.pools, kind, real, n));
        let full = pick(self, &|n| {
            fake::full(&self.salt, &self.pools, kind, real, n)
        });
        self.by_fake.insert(fit.to_lowercase(), real.to_string());
        self.by_fake.insert(full.to_lowercase(), real.to_string());
        self.by_real.insert(key, self.entries.len());
        self.entries.push(Entry {
            real: real.to_string(),
            full,
            fit,
            kind,
        });
    }

    /// Every word in a path or a name, one by one (`claude_scan:px` → hunt, px).
    fn add_parts(&mut self, kind: Kind, s: &str) {
        // whole words only (`ox_meadow`, `claude-herdr`): their pieces are often
        // everyday words (`gpu`, `model`, `new`) that would go everywhere
        for w in s.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')) {
            // Aleph's own story names (`smoldering-heron`) say nothing about
            // anyone: neither they nor their words are private
            if w.is_empty() || crate::names::generated(w) {
                continue;
            }
            self.add(kind, w);
        }
    }

    /// A folder: the words after the home folder (or all of an absolute one's).
    fn add_path(&mut self, p: &str) {
        let rest = p
            .strip_prefix(&self.home)
            .or_else(|| p.strip_prefix("~"))
            .unwrap_or(p);
        self.add_parts(Kind::Project, rest);
    }

    /// A folder on any machine: another machine's home (`/home/<user>/…`)
    /// gives its user, the rest are a project's words.
    fn add_any_path(&mut self, p: &str) {
        let other = p
            .strip_prefix("/home/")
            .or_else(|| p.strip_prefix("/Users/"))
            .filter(|_| self.home.is_empty() || !p.starts_with(&self.home));
        match other.map(|r| r.split_once('/').unwrap_or((r, ""))) {
            Some((user, rest)) => {
                self.add(Kind::User, user);
                self.add_parts(Kind::Project, rest);
            }
            None => self.add_path(p),
        }
    }

    fn finish(&mut self) {
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.entries[i].real.len()));
        let entries: Vec<Entry> = order.iter().map(|&i| self.entries[i].clone()).collect();
        self.entries = entries;
        self.by_real.clear();
        self.index.clear();
        for (i, e) in self.entries.iter().enumerate() {
            let l = e.real.to_lowercase();
            self.by_real.insert(l.clone(), i);
            let b = l.as_bytes();
            if b.len() >= 2 {
                self.index.entry([b[0], b[1]]).or_default().push(i);
            }
        }
    }

    /// The entries that could start with these two (lowercased) bytes, longest first.
    pub fn starting(&self, a: u8, b: u8) -> &[usize] {
        self.index.get(&[a, b]).map(Vec::as_slice).unwrap_or(&[])
    }

    /// A known word's stand-in in text and JSON (whole or same-length, as
    /// `showcase.names` says).
    pub fn fake_of(&self, real: &str) -> Option<&str> {
        self.stand_in_of(real, self.names.text_full())
    }

    /// A known word's whole-name (`full`) or same-length stand-in.
    pub fn stand_in_of(&self, real: &str, full: bool) -> Option<&str> {
        self.by_real.get(&real.to_lowercase()).map(|&i| {
            let e = &self.entries[i];
            if full {
                e.full.as_str()
            } else {
                e.fit.as_str()
            }
        })
    }

    /// Any word's stand-in: the dictionary's when it knows the word, else one
    /// made up for it.
    pub fn stand_in(&self, kind: Kind, word: &str, full: bool) -> String {
        match self.stand_in_of(word, full) {
            Some(f) => f.to_string(),
            None if full => fake::full(&self.salt, &self.pools, kind, word, 0),
            None => fake::fit(&self.salt, &self.pools, kind, word, 0),
        }
    }

    pub fn real_of(&self, fake: &str) -> Option<&str> {
        self.by_fake.get(&fake.to_lowercase()).map(String::as_str)
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

type Stamp = (
    Option<Instant>,
    Option<(u64, u64, Option<SystemTime>, Option<SystemTime>)>,
);

static STAMP: Mutex<Stamp> = Mutex::new((None, None));

/// Folders seen at runtime that no file names — agents running outside
/// herdr — with a generation that moves when one is new.
static LEARNED: Mutex<(u64, Vec<String>)> = Mutex::new((0, Vec::new()));

/// A folder seen at runtime: its words get stand-ins from the next frame on,
/// like every session's folder (kept for this process only).
pub fn learn(path: &str) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    let mut l = LEARNED.lock().unwrap_or_else(|e| e.into_inner());
    if l.1.iter().any(|p| p == path) {
        return;
    }
    if l.1.len() >= 500 {
        l.1.remove(0);
    }
    l.1.push(path.to_string());
    l.0 += 1;
}

fn learned() -> (u64, Vec<String>) {
    LEARNED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn current() -> &'static Mutex<Arc<Dict>> {
    static D: OnceLock<Mutex<Arc<Dict>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Arc::new(Dict::default())))
}

/// The dictionary now (rebuilt when what feeds it changed; looked at every 2 s).
pub fn get() -> Arc<Dict> {
    #[cfg(test)]
    {
        return TEST.with(|t| t.borrow().clone()).unwrap_or_default();
    }
    #[allow(unreachable_code)]
    let st = state::get();
    let mut s = STAMP.lock().unwrap_or_else(|e| e.into_inner());
    // a folder just learned is masked from the very next frame
    let gen = LEARNED.lock().unwrap_or_else(|e| e.into_inner()).0;
    let due = s.0.is_none_or(|t| t.elapsed().as_secs() >= 2) || s.1.is_none_or(|x| x.1 != gen);
    if due {
        s.0 = Some(Instant::now());
        let now = (
            st.epoch,
            gen,
            mtime(&crate::config::Config::path()),
            mtime(&crate::config::world_cache().join("fleet.json")),
        );
        if s.1 != Some(now) {
            s.1 = Some(now);
            *current().lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(build(&st.salt));
        }
    }
    current().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
thread_local! {
    static TEST: std::cell::RefCell<Option<Arc<Dict>>> = const { std::cell::RefCell::new(None) };
}

/// Tests: these words (and this salt) are the private ones.
#[cfg(test)]
pub fn set_test(salt: &[u8], home: &str, words: &[(Kind, &str)]) -> Arc<Dict> {
    set_test_names(salt, home, words, Names::Fit)
}

/// Tests: [`set_test`] with stand-ins as long as `names` says.
#[cfg(test)]
pub fn set_test_names(salt: &[u8], home: &str, words: &[(Kind, &str)], names: Names) -> Arc<Dict> {
    let mut d = Dict {
        salt: salt.to_vec(),
        home: home.to_string(),
        pools: fake::Pools::defaults().clone(),
        names,
        ..Default::default()
    };
    for (k, w) in words {
        d.add_parts(*k, w);
    }
    d.finish();
    let d = Arc::new(d);
    TEST.with(|t| *t.borrow_mut() = Some(d.clone()));
    d
}

/// Everything this computer knows that's private, with its fakes.
pub fn build(salt: &[u8]) -> Dict {
    let cfg = crate::config::Config::load();
    let mut d = Dict {
        salt: salt.to_vec(),
        home: std::env::var("HOME").unwrap_or_default(),
        user: std::env::var("USER").unwrap_or_default(),
        pools: fake::Pools::from_config(&cfg, salt),
        names: Names::of(&cfg.get("showcase.names").unwrap_or_default()),
        ..Default::default()
    };
    // machines first (they win a word two kinds share), in the fleet's order
    for a in cfg.fleet() {
        if crate::sshcfg::is_local(&a) {
            continue;
        }
        d.add_parts(Kind::Host, &a);
        if let Some(m) = cfg.host_get(&a, "machine") {
            d.add_parts(Kind::Host, &m);
        }
        if let Some(r) = crate::sshcfg::resolve(&a) {
            if r.hostname.parse::<std::net::IpAddr>().is_err() {
                d.add_parts(Kind::Host, &r.hostname);
            }
            d.add(Kind::User, &r.user);
            if let Some(p) = &r.proxy {
                d.add_parts(Kind::Host, p);
            }
        }
    }
    if let Some(h) = cfg.get("ssh_host") {
        d.add_parts(Kind::Host, &h);
    }
    if let Some(u) = cfg.get("api.url") {
        if let Some(host) = u
            .split("://")
            .nth(1)
            .and_then(|r| r.split(['/', ':']).next())
        {
            d.add_parts(Kind::Host, host);
        }
    }
    // this computer's user, its home's name, and git's name and email
    let user = d.user.clone();
    d.add(Kind::User, &user);
    let home = d.home.clone();
    if let Some(b) = Path::new(&home).file_name().and_then(|b| b.to_str()) {
        d.add(Kind::User, b);
    }
    if let Ok(t) = std::fs::read_to_string(Path::new(&home).join(".gitconfig")) {
        for l in t.lines() {
            let l = l.trim();
            if let Some(v) = l
                .strip_prefix("name")
                .and_then(|r| r.trim().strip_prefix('='))
            {
                d.add_parts(Kind::User, v);
            }
            if let Some(v) = l
                .strip_prefix("email")
                .and_then(|r| r.trim().strip_prefix('='))
            {
                d.add_parts(Kind::User, v.split('@').next().unwrap_or(""));
                if let Some(dom) = v.split('@').nth(1) {
                    if let Some(label) = dom.trim().split('.').next() {
                        d.add(Kind::Host, label);
                    }
                }
            }
        }
    }
    if cfg!(target_os = "macos") {
        if let Ok(h) = std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
        {
            // "Alexander's MacBook Air": its owner, not the kind of Mac
            let name = String::from_utf8_lossy(&h.stdout).trim().to_string();
            let owner = name.split(['\'', '’', ' ']).next().unwrap_or("");
            d.add(Kind::User, owner);
        }
    }
    // the analytics' machine names and homes
    for (_, s) in crate::usage::store::Offsets::load().hosts {
        if let Some(r) = s.run {
            d.add_parts(Kind::Host, &r.hostname);
            if let Some(b) = Path::new(&r.home).file_name().and_then(|b| b.to_str()) {
                d.add(Kind::User, b);
            }
        }
    }
    // plan accounts (their hashes, and the 4-character tags views show)
    let raw = crate::usage::store::raw_limits();
    for (_, agents) in raw.as_object().into_iter().flatten() {
        for (_, ids) in agents.as_object().into_iter().flatten() {
            for (_, snap) in ids.as_object().into_iter().flatten() {
                if let Some(a) = snap.get("account").and_then(|x| x.as_str()) {
                    if a.len() >= 8 {
                        d.add(Kind::Other, a);
                        d.add(Kind::Other, &a[..4]);
                    }
                }
            }
        }
    }
    // the last fleet snapshot: session names and folders
    if let Ok(t) = std::fs::read_to_string(crate::config::world_cache().join("fleet.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
            for h in v
                .get("hosts")
                .and_then(|x| x.as_object())
                .into_iter()
                .flatten()
            {
                for s in
                    h.1.get("sessions")
                        .and_then(|x| x.as_array())
                        .into_iter()
                        .flatten()
                {
                    for k in ["name", "label"] {
                        if let Some(n) = s.get(k).and_then(|x| x.as_str()) {
                            let base = crate::herdr::base_label(n);
                            let rest = base.split_once('_').map(|x| x.1).unwrap_or(base);
                            d.add_parts(Kind::Session, rest);
                        }
                    }
                    if let Some(c) = s.get("cwd").and_then(|x| x.as_str()) {
                        d.add_path(c);
                    }
                }
            }
        }
    }
    // where sessions were started
    let mut seen: HashSet<String> = HashSet::new();
    for e in crate::history::load(300) {
        if seen.insert(e.dir.clone()) {
            d.add_path(&e.dir);
        }
        d.add_parts(Kind::Session, &e.name);
    }
    // sessions the state log remembers (ended ones show in Monitor › Timeline)
    let mut names: HashSet<String> = HashSet::new();
    for st in crate::statelog::read_from(&crate::statelog::file()) {
        if names.insert(st.name.clone()) {
            d.add_parts(Kind::Session, &st.name);
        }
    }
    // folders seen at runtime (agents outside herdr)
    for p in learned().1 {
        d.add_any_path(&p);
    }
    d.finish();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_on_other_machines_give_their_user_and_words() {
        let mut d = Dict {
            salt: b"k".to_vec(),
            home: "/Users/me".into(),
            ..Default::default()
        };
        d.add_any_path("/home/selene/work/claude_scan");
        d.add_any_path("/Users/me/src/px_audit");
        d.add_any_path("~/scratch/lumen");
        d.finish();
        let kind = |w: &str| d.entries.iter().find(|e| e.real == w).map(|e| e.kind);
        assert_eq!(kind("selene"), Some(Kind::User));
        assert_eq!(kind("claude_scan"), Some(Kind::Project));
        assert_eq!(kind("px_audit"), Some(Kind::Project));
        assert_eq!(kind("lumen"), Some(Kind::Project));
        assert_eq!(
            kind("me"),
            None,
            "this computer's home isn't a project word"
        );
        assert_eq!(kind("work"), None, "everyday folders stay");
        assert_eq!(kind("scratch"), None);
    }

    #[test]
    fn learning_a_folder_moves_the_generation_once() {
        let g0 = learned().0;
        learn("/srv/x/pulsar_lab_7");
        learn("/srv/x/pulsar_lab_7");
        learn("  ");
        let (g1, v) = learned();
        assert_eq!(g1, g0 + 1);
        assert!(v.iter().any(|p| p == "/srv/x/pulsar_lab_7"));
    }
}
