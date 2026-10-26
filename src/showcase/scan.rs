//! Finding private words in text and swapping them: the dictionary's words
//! (at word starts, longest first, any case), a word cut short before `…`,
//! every folder under a home folder, IPv4/IPv6 addresses, emails, versions
//! (`0.9.5`), long hex tokens. No regex: one pass over the characters,
//! testing only where a word starts. `full`: words get whole names (any
//! length); else stand-ins of the same width.

use super::{dict, fake};
use crate::showcase::Kind;

/// (start, end, replacement) in characters (as many, unless `full`).
pub type Edit = (usize, usize, Vec<char>);

fn word(c: char) -> bool {
    c.is_alphanumeric()
}

/// The text's case pattern on the fake: `B31` → `M42`, `Hunt` → `Dent`.
fn shaped(real_in_text: &[char], fake: &str) -> Vec<char> {
    let f: Vec<char> = fake.chars().collect();
    f.iter()
        .enumerate()
        .map(|(i, c)| match real_in_text.get(i) {
            Some(r) if r.is_ascii_uppercase() => c.to_ascii_uppercase(),
            Some(r) if r.is_ascii_lowercase() => c.to_ascii_lowercase(),
            _ => *c,
        })
        .collect()
}

fn eq_ci(a: &[char], b: &str) -> bool {
    let mut it = b.chars();
    for c in a {
        match it.next() {
            Some(d) if d.to_lowercase().eq(c.to_lowercase()) => {}
            _ => return false,
        }
    }
    it.next().is_none()
}

/// A dictionary word at `i` (whole: the next character doesn't go on with
/// letters or digits): its length and fake.
fn dict_at(d: &dict::Dict, s: &[char], i: usize, full: bool) -> Option<(usize, Vec<char>)> {
    let a = s.get(i)?.to_ascii_lowercase();
    let b = s.get(i + 1)?.to_ascii_lowercase();
    if !a.is_ascii() || !b.is_ascii() {
        return None;
    }
    for &k in d.starting(a as u8, b as u8) {
        let e = &d.entries[k];
        let n = e.real.chars().count();
        if i + n > s.len() || !eq_ci(&s[i..i + n], &e.real) {
            continue;
        }
        if s.get(i + n).is_some_and(|c| word(*c)) {
            continue;
        }
        if full {
            let seen: String = s[i..i + n].iter().collect();
            return Some((n, fake::styled(&seen, &e.full).chars().collect()));
        }
        return Some((n, shaped(&s[i..i + n], &e.fit)));
    }
    // cut short (`claude-he…`, `magrath…`): the same start of the fake
    let run = (i..s.len())
        .take_while(|&j| word(s[j]) || s[j] == '-' || s[j] == '_')
        .count();
    if run >= 3 && s.get(i + run) == Some(&'…') {
        for &k in d.starting(a as u8, b as u8) {
            let e = &d.entries[k];
            let real: Vec<char> = e.real.chars().collect();
            if real.len() > run && eq_ci(&s[i..i + run], &real[..run].iter().collect::<String>()) {
                // as cut short as the real one was, so the layout holds
                let f: Vec<char> = if full { &e.full } else { &e.fit }
                    .chars()
                    .take(run)
                    .collect();
                return Some((run, shaped(&s[i..i + run], &f.iter().collect::<String>())));
            }
        }
    }
    None
}

/// `1.2.3.4` at `i`: its length.
fn ipv4_at(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    for part in 0..4 {
        let st = j;
        while j < s.len() && s[j].is_ascii_digit() && j - st < 3 {
            j += 1;
        }
        if j == st {
            return None;
        }
        let n: u32 = s[st..j].iter().collect::<String>().parse().ok()?;
        if n > 255 {
            return None;
        }
        if part < 3 {
            if s.get(j) != Some(&'.') {
                return None;
            }
            j += 1;
        }
    }
    if s.get(j).is_some_and(|c| {
        c.is_ascii_digit() || *c == '.' && s.get(j + 1).is_some_and(|d| d.is_ascii_digit())
    }) {
        return None;
    }
    Some(j - i)
}

/// A version (`0.9.5`, `2.1.309`, `v1.2.3`) at `i`: three or more numbers.
fn version_at(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    let mut parts = 0;
    loop {
        let st = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j == st {
            return None;
        }
        parts += 1;
        if s.get(j) == Some(&'.') && s.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
            j += 1;
            continue;
        }
        break;
    }
    (parts >= 3 && !s.get(j).is_some_and(|c| word(*c))).then_some(j - i)
}

/// The word just before `i` (past spaces) names an operating system.
fn os_before(s: &[char], i: usize) -> bool {
    let mut j = i;
    while j > 0 && s[j - 1] == ' ' {
        j -= 1;
    }
    let end = j;
    while j > 0 && s[j - 1].is_alphanumeric() {
        j -= 1;
    }
    let w: String = s[j..end].iter().collect::<String>().to_lowercase();
    matches!(
        w.as_str(),
        "ubuntu"
            | "debian"
            | "fedora"
            | "macos"
            | "linux"
            | "darwin"
            | "rocky"
            | "centos"
            | "alpine"
            | "rhel"
            | "almalinux"
            | "opensuse"
            | "suse"
            | "mint"
            | "pop"
            | "arch"
            | "kernel"
            | "windows"
            | "ios"
            | "sonoma"
            | "sequoia"
            | "tahoe"
            | "raspbian"
            | "nixos"
            | "amazon"
            | "oracle"
            | "kali"
    )
}

/// An email at `i` (`name@host.tld`): the local part and the domain's labels
/// but its last one swap.
fn email_at(d: &dict::Dict, s: &[char], i: usize, full: bool) -> Option<(usize, Vec<char>)> {
    let local = (i..s.len())
        .take_while(|&j| word(s[j]) || matches!(s[j], '.' | '_' | '-' | '+'))
        .count();
    if local == 0 || s.get(i + local) != Some(&'@') {
        return None;
    }
    let at = i + local;
    let dom = (at + 1..s.len())
        .take_while(|&j| word(s[j]) || matches!(s[j], '.' | '-'))
        .count();
    let domain: String = s[at + 1..at + 1 + dom].iter().collect();
    let domain = domain.trim_end_matches('.');
    // a real domain ends in letters (`user@10.0.0.49` is ssh's, not an email)
    let tld = domain.rsplit('.').next().unwrap_or("");
    if !domain.contains('.') || tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let lp: String = s[i..at].iter().collect();
    let labels: Vec<&str> = domain.split('.').collect();
    let mut out: Vec<char> = d.stand_in(Kind::User, &lp, full).chars().collect();
    out.push('@');
    for (k, l) in labels.iter().enumerate() {
        if k > 0 {
            out.push('.');
        }
        if k + 1 == labels.len() {
            out.extend(l.chars());
        } else {
            out.extend(d.stand_in(Kind::Host, l, full).chars());
        }
    }
    Some((local + 1 + domain.chars().count(), out))
}

/// A path under a home folder (`/Users/x/…`, `/home/x/…`, `~/…`) at `i`:
/// the user and every folder after it, known or not (files keep their names).
fn home_path_at(d: &dict::Dict, s: &[char], i: usize, full: bool) -> Option<(usize, Vec<char>)> {
    let rest: String = s[i..].iter().collect();
    let (pre, user_end) = if rest.starts_with("~/") {
        (1, 1)
    } else {
        let r = rest
            .strip_prefix("/Users/")
            .or_else(|| rest.strip_prefix("/home/"))?;
        let u = r
            .chars()
            .take_while(|c| word(*c) || matches!(c, '.' | '-' | '_'))
            .count();
        if u == 0 {
            return None;
        }
        let pre = rest.len() - r.len();
        (pre, pre + u)
    };
    let len = rest
        .chars()
        .take_while(|c| {
            !c.is_whitespace()
                && !matches!(
                    c,
                    '"' | '\'' | '`' | ')' | ']' | '>' | ',' | ';' | '│' | '╯' | '╮'
                )
        })
        .count();
    let path: Vec<char> = s[i..i + len].to_vec();
    let mut out: Vec<char> = path[..pre].to_vec();
    // the demo world's own users and folders are made up already
    let made_up = |w: &str| super::demo() && super::demo::owns(w);
    // the user
    if user_end > pre {
        let u: String = path[pre..user_end].iter().collect();
        let f = if made_up(&u) {
            u.clone()
        } else {
            d.stand_in(Kind::User, &u, full)
        };
        out.extend(f.chars());
    }
    // then folder by folder
    let tail: String = path[user_end..].iter().collect();
    let comps: Vec<&str> = tail.split('/').collect();
    for (k, c) in comps.iter().enumerate() {
        if k > 0 {
            out.push('/');
        }
        let last = k + 1 == comps.len();
        let file = last && c.contains('.');
        if c.is_empty() || file || c.starts_with('.') || dict_stop(c) || made_up(c) {
            out.extend(c.chars());
        } else {
            out.extend(d.stand_in(Kind::Project, c, full).chars());
        }
    }
    // same-length stand-ins differ only in letters: as long by construction
    (full || out.len() == len).then_some((len, out))
}

fn dict_stop(c: &str) -> bool {
    let l = c.to_lowercase();
    matches!(
        l.as_str(),
        "work"
            | "projects"
            | "project"
            | "code"
            | "src"
            | "dev"
            | "repos"
            | "documents"
            | "desktop"
            | "downloads"
            | "library"
            | "applications"
            | "go"
            | "bin"
            | "lib"
            | "tmp"
            | "local"
            | "share"
            | "cache"
            | "config"
            | "herdr"
            | "aleph"
            | "pictures"
            | "movies"
            | "music"
            | "videos"
            | "public"
            | "sites"
            // Aleph's own folders (its data, state and cache, the links applet)
            | "state"
            | "backups"
            | "journal"
            | "profiles"
    ) || l.ends_with('…')
        // an agent's executable (`~/.local/bin/claude`) says nothing private
        || crate::agents::ALL
            .iter()
            .any(|a| a.id == l || a.exes.contains(&l.as_str()))
}

/// A long hex run (a token, a hash: 24+ hex digits) at `i`.
fn hex_at(s: &[char], i: usize) -> Option<usize> {
    let n = (i..s.len())
        .take_while(|&j| s[j].is_ascii_hexdigit())
        .count();
    let has_digit = s[i..i + n].iter().any(|c| c.is_ascii_digit());
    let has_alpha = s[i..i + n].iter().any(|c| c.is_ascii_alphabetic());
    (n >= 24 && has_digit && has_alpha && !s.get(i + n).is_some_and(|c| word(*c))).then_some(n)
}

/// Every swap in `s` (`full`: whole names, of any length).
pub fn edits(d: &dict::Dict, s: &[char], full: bool) -> Vec<Edit> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        let start = i == 0 || !word(s[i - 1]);
        if !start || c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' || c == '~' {
            if let Some((n, r)) = home_path_at(d, s, i, full) {
                out.push((i, i + n, r));
                i += n;
                continue;
            }
            i += 1;
            continue;
        }
        if !word(c) {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            if let Some(n) = ipv4_at(s, i) {
                let ip: String = s[i..i + n].iter().collect();
                if !ip.starts_with("127.") && ip != "0.0.0.0" {
                    let x = fake::seed(&d.salt, Kind::Host, &ip, 0);
                    out.push((i, i + n, fake::ipv4(&ip, x).chars().collect()));
                }
                i += n;
                continue;
            }
            if let Some(n) = version_at(s, i) {
                // an operating system's version isn't private (Ubuntu 22.04.5,
                // macOS 26.0.1, Linux 6.8.0): only the software's are
                if !os_before(s, i) {
                    let v: String = s[i..i + n].iter().collect();
                    out.push((i, i + n, fake::version(&v).chars().collect()));
                }
                i += n;
                continue;
            }
        }
        if let Some(n) = hex_at(s, i) {
            let h: String = s[i..i + n].iter().collect();
            let x = fake::seed(&d.salt, Kind::Other, &h, 0);
            let r: Vec<char> = (0..n)
                .map(|k| {
                    let v = ((x.rotate_left((k % 64) as u32) >> 3) % 16) as u32;
                    char::from_digit(v, 16).unwrap_or('0')
                })
                .collect();
            out.push((i, i + n, r));
            i += n;
            continue;
        }
        if let Some((n, r)) = email_at(d, s, i, full) {
            if full || r.len() == n {
                out.push((i, i + n, r));
                i += n;
                continue;
            }
        }
        if let Some((n, r)) = dict_at(d, s, i, full) {
            if full || r.len() == n {
                out.push((i, i + n, r));
            }
            i += n;
            continue;
        }
        // on to the next word
        while i < s.len() && word(s[i]) {
            i += 1;
        }
    }
    out
}

/// `s` with every private word swapped.
pub fn scrub(s: &str) -> String {
    let d = dict::get();
    scrub_with(&d, s)
}

pub fn scrub_with(d: &dict::Dict, s: &str) -> String {
    scrub_as(d, s, d.names.text_full())
}

/// `s` with every private word swapped for a whole name (`full`) or one of
/// the same length.
pub fn scrub_as(d: &dict::Dict, s: &str, full: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let e = edits(d, &chars, full);
    if e.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    for (a, b, r) in e {
        out.extend(&chars[i..a]);
        out.extend(r);
        i = b;
    }
    out.extend(&chars[i..]);
    out
}

/// A printed line's spans, masked across them (see `showcase::spans`).
pub fn spans(parts: &[&str]) -> Option<Vec<String>> {
    let d = dict::get();
    let row: Vec<super::reflow::Slot<usize>> = parts
        .iter()
        .enumerate()
        .flat_map(|(i, p)| p.chars().map(move |c| Some((c, i))))
        .collect();
    let chars: Vec<char> = row.iter().flatten().map(|x| x.0).collect();
    let e = edits(&d, &chars, d.names.text_full());
    if e.is_empty() {
        return None;
    }
    let out = super::reflow::reflow(&row, &e, true);
    let mut v = vec![String::new(); parts.len()];
    for (c, i) in out.into_iter().flatten() {
        v[i].push(c);
    }
    Some(v)
}

/// The real word `q` stands for, when it's the scrubbed form of one of `reals`.
pub fn unmask(q: &str, reals: &[String]) -> Option<String> {
    let d = dict::get();
    if let Some(r) = d.real_of(q) {
        if reals.iter().any(|x| x == r) {
            return Some(r.to_string());
        }
    }
    // what the dashboard showed, or what a client saw (whole or same-length)
    reals
        .iter()
        .find(|r| scrub_as(&d, r, true) == q || scrub_as(&d, r, false) == q)
        .cloned()
}
