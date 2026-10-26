//! On-disk listing cache, one file per host:
//!   $XDG_CACHE_HOME/aleph/<host>.cache  (default ~/.cache/…)
//!
//! Entries are shown instantly when a folder is opened, then validated against
//! the remote (existence + mtime) before being trusted. Entries older than
//! MAX_AGE are dropped on load/save; the file is capped at MAX_DIRS folders.
//!
//! Deep-search indexes (sub-folder paths a few levels below a root) live in
//! the same file as `R` records followed by `d` lines, which older readers
//! skip.
//!
//! Several browsers may run at once (one per terminal), so `save` takes a lock
//! file, re-reads what's on disk, merges newest-`fetched`-wins (respecting our
//! own removals) and writes atomically.

use crate::path;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAGIC: &str = "aleph-browse cache v1";
const MAX_AGE: u64 = 7 * 24 * 3600;
const MAX_DIRS: usize = 3000;
const MAX_DEEP: usize = 24;
/// A lock older than this is from a crashed process and is taken over.
const LOCK_STALE: Duration = Duration::from_secs(10);
const LOCK_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq)]
pub struct Dir {
    pub name: Vec<u8>,
    pub link: bool,
    pub denied: bool,
    pub git: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Remote folder mtime (seconds); 0 when the remote has no usable `stat`.
    pub mtime: i64,
    /// Unix time this listing was fetched.
    pub fetched: u64,
    pub dirs: Vec<Dir>,
}

/// Deep-search index of one root: sub-folder paths relative to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Deep {
    pub fetched: u64,
    /// levels indexed
    pub depth: u64,
    /// the server stopped at its line cap
    pub truncated: bool,
    pub paths: Vec<Vec<u8>>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn sort_dirs(d: &mut [Dir]) {
    d.sort_by_cached_key(|x| (path::show(&x.name).to_lowercase(), x.name.clone()));
}

#[derive(Default)]
pub struct Cache {
    pub home: Option<Vec<u8>>,
    map: HashMap<Vec<u8>, Entry>,
    deep: HashMap<Vec<u8>, Deep>,
    /// (path, when, whole tree) — so a merge doesn't resurrect what we dropped
    removed: Vec<(Vec<u8>, u64, bool)>,
    file: Option<PathBuf>,
}

impl Cache {
    pub fn load(host: &str) -> Cache {
        let file = cache_file(host);
        let mut c = Cache {
            file: file.clone(),
            ..Default::default()
        };
        if let Some(f) = file {
            if let Ok(fh) = fs::File::open(f) {
                c.parse(BufReader::new(fh));
            }
        }
        c
    }

    fn parse<R: BufRead>(&mut self, r: R) {
        let mut lines = r.lines();
        match lines.next() {
            Some(Ok(l)) if l == MAGIC => {}
            _ => return, // unknown/old format: start fresh
        }
        let cutoff = now().saturating_sub(MAX_AGE);
        let mut cur: Option<(Vec<u8>, Entry)> = None;
        let mut deep: Option<(Vec<u8>, Deep)> = None;
        for l in lines.map_while(Result::ok) {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            match f.as_slice() {
                ["H", h] => self.home = path::unesc(h),
                ["P", p, m, t] => {
                    self.keep(cur.take(), cutoff);
                    self.keep_deep(deep.take(), cutoff);
                    if let (Some(p), Ok(m), Ok(t)) = (path::unesc(p), m.parse(), t.parse()) {
                        cur = Some((
                            p,
                            Entry {
                                mtime: m,
                                fetched: t,
                                dirs: Vec::new(),
                            },
                        ));
                    }
                }
                ["D", fl, n] => {
                    if let (Some((_, e)), Some(n)) = (cur.as_mut(), path::unesc(n)) {
                        e.dirs.push(Dir {
                            name: n,
                            link: fl.contains('l'),
                            denied: fl.contains('d'),
                            git: fl.contains('g'),
                        });
                    }
                }
                ["R", p, t, d, tr] => {
                    self.keep(cur.take(), cutoff);
                    self.keep_deep(deep.take(), cutoff);
                    if let (Some(p), Ok(t), Ok(d)) = (path::unesc(p), t.parse(), d.parse()) {
                        deep = Some((
                            p,
                            Deep {
                                fetched: t,
                                depth: d,
                                truncated: *tr == "1",
                                paths: Vec::new(),
                            },
                        ));
                    }
                }
                ["d", n] => {
                    if let (Some((_, e)), Some(n)) = (deep.as_mut(), path::unesc(n)) {
                        e.paths.push(n);
                    }
                }
                _ => {}
            }
        }
        self.keep(cur, cutoff);
        self.keep_deep(deep, cutoff);
    }

    fn keep(&mut self, cur: Option<(Vec<u8>, Entry)>, cutoff: u64) {
        if let Some((p, e)) = cur {
            if e.fetched >= cutoff {
                self.map.insert(p, e);
            }
        }
    }

    fn keep_deep(&mut self, cur: Option<(Vec<u8>, Deep)>, cutoff: u64) {
        if let Some((p, e)) = cur {
            if e.fetched >= cutoff {
                self.deep.insert(p, e);
            }
        }
    }

    pub fn get(&self, p: &[u8]) -> Option<&Entry> {
        self.map.get(p)
    }

    pub fn put(&mut self, p: &[u8], mut e: Entry) {
        sort_dirs(&mut e.dirs);
        self.map.insert(p.to_vec(), e);
    }

    pub fn deep(&self, root: &[u8]) -> Option<&Deep> {
        self.deep.get(root)
    }

    pub fn put_deep(&mut self, root: &[u8], mut d: Deep) {
        d.paths.sort();
        d.paths.dedup();
        self.deep.insert(root.to_vec(), d);
    }

    /// Forget one folder's listing (it changed remotely).
    pub fn forget(&mut self, p: &[u8]) {
        self.map.remove(p);
        self.removed.push((p.to_vec(), now(), false));
    }

    /// Forget a folder and everything cached beneath it (it was removed remotely).
    pub fn remove_tree(&mut self, p: &[u8]) {
        self.map.retain(|k, _| !path::is_within(k, p));
        self.deep.retain(|k, _| !path::is_within(k, p));
        self.removed.push((p.to_vec(), now(), true));
    }

    /// Fold in what another browser saved: newest `fetched` wins, and entries
    /// we removed (at or after their fetch time) stay removed.
    fn merge(&mut self, disk: Cache) {
        let removed = std::mem::take(&mut self.removed);
        let dropped = |k: &[u8], fetched: u64| {
            removed.iter().any(|(p, t, tree)| {
                fetched <= *t && (k == p.as_slice() || (*tree && path::is_within(k, p)))
            })
        };
        if self.home.is_none() {
            self.home = disk.home;
        }
        for (k, e) in disk.map {
            let newer = self.map.get(&k).is_none_or(|m| e.fetched > m.fetched);
            if newer && !dropped(&k, e.fetched) {
                self.map.insert(k, e);
            }
        }
        for (k, e) in disk.deep {
            let newer = self.deep.get(&k).is_none_or(|m| e.fetched > m.fetched);
            if newer && !dropped(&k, e.fetched) {
                self.deep.insert(k, e);
            }
        }
        self.removed = removed;
    }

    /// Lock, merge with what's on disk, write atomically.
    pub fn save(&mut self) -> io::Result<()> {
        let Some(f) = self.file.clone() else {
            return Ok(());
        };
        if let Some(d) = f.parent() {
            fs::create_dir_all(d)?;
        }
        let _lock = Lock::take(&f.with_extension("lock"))?;
        if let Ok(fh) = fs::File::open(&f) {
            let mut disk = Cache::default();
            disk.parse(BufReader::new(fh));
            self.merge(disk);
        }
        self.write(&f)
    }

    fn write(&mut self, f: &Path) -> io::Result<()> {
        let cutoff = now().saturating_sub(MAX_AGE);
        self.map.retain(|_, e| e.fetched >= cutoff);
        self.deep.retain(|_, e| e.fetched >= cutoff);
        trim_oldest(&mut self.map, MAX_DIRS, |e| e.fetched);
        trim_oldest(&mut self.deep, MAX_DEEP, |e| e.fetched);
        let tmp = f.with_extension(format!("tmp{}", std::process::id()));
        {
            let fh = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            let mut w = BufWriter::new(fh);
            writeln!(w, "{MAGIC}")?;
            if let Some(h) = &self.home {
                writeln!(w, "H\t{}", path::esc(h))?;
            }
            for (p, e) in &self.map {
                writeln!(w, "P\t{}\t{}\t{}", path::esc(p), e.mtime, e.fetched)?;
                for d in &e.dirs {
                    let mut fl = String::new();
                    if d.link {
                        fl.push('l')
                    }
                    if d.denied {
                        fl.push('d')
                    }
                    if d.git {
                        fl.push('g')
                    }
                    if fl.is_empty() {
                        fl.push('-')
                    }
                    writeln!(w, "D\t{}\t{}", fl, path::esc(&d.name))?;
                }
            }
            for (p, e) in &self.deep {
                writeln!(
                    w,
                    "R\t{}\t{}\t{}\t{}",
                    path::esc(p),
                    e.fetched,
                    e.depth,
                    u8::from(e.truncated)
                )?;
                for d in &e.paths {
                    writeln!(w, "d\t{}", path::esc(d))?;
                }
            }
            w.flush()?;
        }
        fs::rename(&tmp, f) // atomic: concurrent browsers never see a half-written file
    }
}

fn trim_oldest<V>(m: &mut HashMap<Vec<u8>, V>, max: usize, fetched: impl Fn(&V) -> u64) {
    if m.len() <= max {
        return;
    }
    let mut by_age: Vec<(u64, Vec<u8>)> = m.iter().map(|(k, e)| (fetched(e), k.clone())).collect();
    by_age.sort();
    let excess = m.len() - max;
    for (_, k) in by_age.into_iter().take(excess) {
        m.remove(&k);
    }
}

/// An exclusive `create_new` lock file, removed on drop. A lock older than
/// LOCK_STALE belongs to a crashed process and is taken over; otherwise we
/// wait up to LOCK_WAIT, then give up (the cache is only a cache).
struct Lock(PathBuf);

impl Lock {
    fn take(p: &Path) -> io::Result<Lock> {
        let end = Instant::now() + LOCK_WAIT;
        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(p)
            {
                Ok(mut fh) => {
                    let _ = writeln!(fh, "{}", std::process::id());
                    return Ok(Lock(p.to_path_buf()));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(p)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = fs::remove_file(p);
                        continue;
                    }
                    if Instant::now() >= end {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "cache is locked by another browser",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn cache_file(host: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.@".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(crate::showcase::sandbox(base.join("aleph")).join(format!("{safe}.cache")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chb-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn at(file: &Path) -> Cache {
        Cache {
            file: Some(file.to_path_buf()),
            ..Default::default()
        }
    }

    fn reload(file: &Path) -> Cache {
        let mut l = at(file);
        l.parse(BufReader::new(fs::File::open(file).unwrap()));
        l
    }

    fn entry(fetched: u64, names: &[&[u8]]) -> Entry {
        Entry {
            mtime: 1,
            fetched,
            dirs: names
                .iter()
                .map(|n| Dir {
                    name: n.to_vec(),
                    link: false,
                    denied: false,
                    git: false,
                })
                .collect(),
        }
    }

    #[test]
    fn save_load_roundtrip_and_remove_tree() {
        let dir = tmpdir("rt");
        let file = dir.join("h.cache");
        let mut c = Cache {
            file: Some(file.clone()),
            home: Some(b"/home/u".to_vec()),
            ..Default::default()
        };
        let d = |n: &[u8]| Dir {
            name: n.to_vec(),
            link: false,
            denied: n == b"root",
            git: n == b"repo",
        };
        c.put(
            b"/home/u",
            Entry {
                mtime: 5,
                fetched: now(),
                dirs: vec![d(b"repo"), d(b"Alpha"), d(b"we\tird\xff"), d(b"root")],
            },
        );
        c.put(
            b"/home/u/repo",
            Entry {
                mtime: 6,
                fetched: now(),
                dirs: vec![],
            },
        );
        c.put(
            b"/old",
            Entry {
                mtime: 1,
                fetched: 1,
                dirs: vec![],
            },
        ); // older than MAX_AGE
        c.put_deep(
            b"/home/u",
            Deep {
                fetched: now(),
                depth: 3,
                truncated: true,
                paths: vec![b"repo/src".to_vec(), b"Alpha/b\tc".to_vec()],
            },
        );
        c.save().unwrap();
        assert!(!file.with_extension("lock").exists(), "lock released");

        let mut l = reload(&file);
        assert_eq!(l.home.as_deref(), Some(&b"/home/u"[..]));
        let e = l.get(b"/home/u").unwrap();
        let names: Vec<&[u8]> = e.dirs.iter().map(|d| &d.name[..]).collect();
        assert_eq!(names, vec![&b"Alpha"[..], b"repo", b"root", b"we\tird\xff"]);
        assert!(e.dirs[1].git && e.dirs[2].denied);
        assert!(l.get(b"/old").is_none());
        let dp = l.deep(b"/home/u").unwrap();
        assert!(dp.truncated && dp.depth == 3);
        assert_eq!(dp.paths, vec![b"Alpha/b\tc".to_vec(), b"repo/src".to_vec()]);
        // the listing after an R block is still a listing
        assert!(l.get(b"/home/u/repo").is_some());

        l.remove_tree(b"/home/u");
        assert!(l.get(b"/home/u/repo").is_none() && l.get(b"/home/u").is_none());
        assert!(l.deep(b"/home/u").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_saves_merge_newest_wins() {
        let dir = tmpdir("merge");
        let file = dir.join("h.cache");
        let t = now();
        // two browsers load the same (empty) file
        let mut a = at(&file);
        let mut b = at(&file);
        a.put(b"/x", entry(t - 50, &[b"old"]));
        a.put(b"/only-a", entry(t, &[b"a"]));
        b.put(b"/x", entry(t - 10, &[b"new"]));
        b.put(b"/only-b", entry(t, &[b"b"]));
        b.home = Some(b"/home/b".to_vec());
        b.save().unwrap();
        a.save().unwrap(); // a's older /x must not clobber b's newer one

        let l = reload(&file);
        assert_eq!(l.get(b"/x").unwrap().dirs[0].name, b"new");
        assert!(l.get(b"/only-a").is_some() && l.get(b"/only-b").is_some());
        assert_eq!(l.home.as_deref(), Some(&b"/home/b"[..]));

        // a removal survives a merge with an older copy on disk...
        let mut c = reload(&file);
        c.remove_tree(b"/only-b");
        c.forget(b"/x");
        c.save().unwrap();
        let l = reload(&file);
        assert!(l.get(b"/only-b").is_none() && l.get(b"/x").is_none());
        assert!(l.get(b"/only-a").is_some());
        // ...but a listing fetched after the removal wins
        let mut d = at(&file);
        d.put(b"/x", entry(t + 100, &[b"fresh"]));
        d.save().unwrap();
        let mut e = reload(&file);
        e.removed.push((b"/x".to_vec(), t, false));
        e.map.remove(&b"/x"[..]);
        e.save().unwrap();
        assert_eq!(reload(&file).get(b"/x").unwrap().dirs[0].name, b"fresh");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn lock_waits_then_takes_over_stale() {
        let dir = tmpdir("lock");
        let lock = dir.join("h.lock");
        {
            let _held = Lock::take(&lock).unwrap();
            let t0 = Instant::now();
            let e = Lock::take(&lock).err().unwrap();
            assert_eq!(e.kind(), io::ErrorKind::WouldBlock);
            assert!(t0.elapsed() >= LOCK_WAIT);
        }
        assert!(!lock.exists());
        // a crashed holder: lock file with an old mtime
        fs::write(&lock, "999999\n").unwrap();
        let ok = std::process::Command::new("touch")
            .args(["-t", "202001010000"])
            .arg(&lock)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let t0 = Instant::now();
        let _l = Lock::take(&lock).unwrap();
        assert!(t0.elapsed() < Duration::from_secs(1));
        let _ = fs::remove_dir_all(dir);
    }
}
