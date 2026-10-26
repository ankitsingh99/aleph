//! A private word's stand-in: chosen by HMAC over (salt, kind, word), so it
//! can't be worked back from the fake. Two of them for every word:
//!
//! * `full` — a whole name from the word's pool (a ship for a machine, an AI
//!   for a user, anything for a folder: `universe.rs`, picked by Settings ›
//!   Showcase), in the word's own case and separator (`b31` → `normandy`,
//!   `ox_color_check` → `black_mesa`);
//! * `fit` — shaped like the word: the same length and case, digits for
//!   digits, separators kept, so a table drawn with the real one lines up
//!   with the fake (`b31` → `ds9`, `ox_color_check` → `io_comet_orbit`).

use super::universe::{self, Cat};
use crate::config::Config;
use crate::showcase::Kind;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// 8 bytes of HMAC-SHA256(salt, kind | word | n).
pub fn seed(salt: &[u8], kind: Kind, word: &str, n: u32) -> u64 {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(salt)
        .unwrap_or_else(|_| <Hmac<Sha256> as Mac>::new_from_slice(b"aleph").expect("a fixed key"));
    m.update(format!("{kind:?}|{word}|{n}").as_bytes());
    let b = m.finalize().into_bytes();
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// What one field's words become: whole names, and the single words they're
/// made of (for `fit`).
#[derive(Clone, Debug, Default)]
pub struct Pool {
    pub names: Vec<String>,
    /// letters-only words, for runs of letters
    words: Vec<String>,
    /// whole names without their dashes (digits kept), for a machine's name
    /// swapped whole
    whole: Vec<String>,
}

impl Pool {
    pub fn new(names: Vec<String>) -> Pool {
        let mut words: Vec<String> = Vec::new();
        let mut whole: Vec<String> = Vec::new();
        for n in &names {
            let joined: String = n.chars().filter(|c| *c != '-').collect();
            if joined.len() >= 3 && !whole.contains(&joined) {
                whole.push(joined.clone());
            }
            let parts = n.split('-').map(str::to_string).chain([joined]);
            for w in parts {
                if w.len() >= 2 && w.bytes().all(|b| b.is_ascii_alphabetic()) && !words.contains(&w)
                {
                    words.push(w);
                }
            }
        }
        Pool {
            names,
            words,
            whole,
        }
    }
}

/// The three fields' pools.
#[derive(Clone, Debug, Default)]
pub struct Pools {
    pub host: Pool,
    pub user: Pool,
    pub thing: Pool,
}

/// A comma list from a setting, trimmed and lowercased.
fn list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Your own word as a name: lowercase, spaces and underscores as dashes.
fn own(w: &str) -> Option<String> {
    let n: String = w
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '_' { '-' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let n = n.trim_matches('-').to_string();
    (n.len() >= 2).then_some(n)
}

impl Pools {
    /// The pools Settings › Showcase asks for (`salt` picks the universe when
    /// one rules each launch).
    pub fn from_config(cfg: &Config, salt: &[u8]) -> Pools {
        let get = |k: &str| cfg.get(k).unwrap_or_default();
        let on = |k: &str, d: &str| {
            let v = get(k);
            if v.trim().is_empty() {
                d.to_string()
            } else {
                v
            }
        };
        Pools::build(
            &on("showcase.universes", "all"),
            &on("showcase.theme", "mixed"),
            [
                (
                    on("showcase.machines", "ships"),
                    get("showcase.words.machines"),
                ),
                (on("showcase.users", "ais"), get("showcase.words.users")),
                (on("showcase.things", "any"), get("showcase.words.things")),
            ],
            salt,
        )
    }

    /// Pure (tested): universes, theme, and per field (kinds, own words).
    pub fn build(
        universes: &str,
        theme: &str,
        fields: [(String, String); 3],
        salt: &[u8],
    ) -> Pools {
        let all: Vec<&'static str> = universe::ALL.iter().map(|u| u.id).collect();
        let mut on: Vec<&'static str> = {
            let l = list(universes);
            if l.is_empty() || l.iter().any(|x| x == "all") {
                all.clone()
            } else {
                all.iter()
                    .copied()
                    .filter(|id| l.iter().any(|x| x == id))
                    .collect()
            }
        };
        if on.is_empty() {
            on = all.clone();
        }
        let theme = theme.trim().to_lowercase();
        let ids: Vec<&'static str> = match theme.as_str() {
            "" | "mixed" => on.clone(),
            "per-launch" | "launch" => {
                let x = salt.iter().take(8).fold(0u64, |a, b| a << 8 | *b as u64);
                vec![on[(x % on.len() as u64) as usize]]
            }
            id => match universe::find(id) {
                Some(u) => vec![u.id],
                None => on.clone(),
            },
        };
        let field = |(kinds, mine): &(String, String), cap: usize| -> Pool {
            let mine: Vec<String> = list(mine).iter().filter_map(|w| own(w)).collect();
            let kinds = list(kinds);
            let only_mine = kinds.len() == 1 && kinds[0] == "mine";
            let cats: Vec<Cat> = if kinds.iter().any(|k| k == "any" || k == "all") {
                Cat::ALL.to_vec()
            } else {
                Cat::ALL
                    .iter()
                    .copied()
                    .filter(|c| kinds.iter().any(|k| k == c.id()))
                    .collect()
            };
            let pick = |ids: &[&str], cats: &[Cat]| -> Vec<String> {
                let v: Vec<String> = universe::names(ids, cats)
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                // short enough to read in a column, when there are enough of those
                let short: Vec<String> = v.iter().filter(|n| n.len() <= cap).cloned().collect();
                if short.len() >= 8 {
                    short
                } else {
                    v
                }
            };
            let mut names = if only_mine && !mine.is_empty() {
                Vec::new()
            } else {
                // never empty: these kinds anywhere, then any kind at all
                let mut v = pick(&ids, if cats.is_empty() { &Cat::ALL } else { &cats });
                if v.len() < 4 {
                    v.extend(pick(&ids, &Cat::ALL));
                }
                if v.len() < 4 {
                    v.extend(pick(&[], if cats.is_empty() { &Cat::ALL } else { &cats }));
                }
                v
            };
            for w in mine {
                if !names.contains(&w) {
                    names.push(w);
                }
            }
            Pool::new(names)
        };
        Pools {
            host: field(&fields[0], 12),
            user: field(&fields[1], 14),
            thing: field(&fields[2], 24),
        }
    }

    /// Every universe, the default kinds (tests).
    #[cfg(test)]
    pub fn defaults() -> &'static Pools {
        static P: std::sync::OnceLock<Pools> = std::sync::OnceLock::new();
        P.get_or_init(|| {
            Pools::build(
                "all",
                "mixed",
                [
                    ("ships".into(), String::new()),
                    ("ais".into(), String::new()),
                    ("any".into(), String::new()),
                ],
                &[],
            )
        })
    }

    pub fn of(&self, kind: Kind) -> &Pool {
        match kind {
            Kind::Host => &self.host,
            Kind::User => &self.user,
            _ => &self.thing,
        }
    }
}

/// A run of `len` letters from `pool`'s words: one of that length, else two
/// that make it up, else the start of a longer one.
fn letters(pool: &Pool, len: usize, mut x: u64) -> String {
    let l: Vec<&str> = pool.words.iter().map(String::as_str).collect();
    let l = l.as_slice();
    let mut next = || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (x >> 33) as usize
    };
    if len == 1 {
        return ((b'a' + (next() % 26) as u8) as char).to_string();
    }
    let exact: Vec<&str> = l.iter().copied().filter(|w| w.len() == len).collect();
    if !exact.is_empty() {
        return exact[next() % exact.len()].to_string();
    }
    let pairs: Vec<(&str, &str)> = l
        .iter()
        .filter(|a| a.len() < len)
        .flat_map(|a| {
            l.iter()
                .filter(move |b| a.len() + b.len() == len)
                .map(move |b| (*a, *b))
        })
        .collect();
    if !pairs.is_empty() {
        let (a, b) = pairs[next() % pairs.len()];
        return format!("{a}{b}");
    }
    let longer: Vec<&str> = l.iter().copied().filter(|w| w.len() > len).collect();
    if !longer.is_empty() {
        return longer[next() % longer.len()][..len].to_string();
    }
    (0..len)
        .map(|_| (b'a' + (next() % 26) as u8) as char)
        .collect()
}

fn digits(len: usize, mut x: u64) -> String {
    (0..len)
        .map(|i| {
            x = x.rotate_left(7) ^ 0x9e37_79b9_7f4a_7c15;
            let d = (x % 10) as u8;
            // a number keeps its first digit non-zero when it had one
            (b'0' + if i == 0 && d == 0 { 4 } else { d }) as char
        })
        .collect()
}

/// `word`'s same-length stand-in from the default pools (tests).
#[cfg(test)]
pub fn fake(salt: &[u8], kind: Kind, word: &str, n: u32) -> String {
    fit(salt, Pools::defaults(), kind, word, n)
}

/// `word`'s stand-in in its own shape (try `n` = 0, 1, … until it's not
/// someone else's real).
pub fn fit(salt: &[u8], pools: &Pools, kind: Kind, word: &str, n: u32) -> String {
    let pool = pools.of(kind);
    // a machine's name (one word: `b31`, `dex42`, `q7x`) is swapped whole for
    // a ship or station of the same length — `ds9`, `kestrel`
    if kind == Kind::Host && word.len() >= 3 && word.chars().all(|c| c.is_ascii_alphanumeric()) {
        let len = word.len();
        let same: Vec<&String> = pool.whole.iter().filter(|w| w.len() == len).collect();
        if !same.is_empty() {
            let x = seed(salt, kind, word, n);
            let f = same[(x % same.len() as u64) as usize];
            let shout = word.chars().any(|c| c.is_ascii_uppercase())
                && !word.chars().any(|c| c.is_ascii_lowercase());
            return if shout {
                f.to_uppercase()
            } else {
                f.to_string()
            };
        }
    }
    let mut out = String::with_capacity(word.len());
    let chars: Vec<char> = word.chars().collect();
    let mut i = 0;
    let mut run = 0u32;
    while i < chars.len() {
        let c = chars[i];
        let j = (i..chars.len())
            .find(|&k| {
                let d = chars[k];
                !(d.is_ascii_alphabetic() && c.is_ascii_alphabetic()
                    || d.is_ascii_digit() && c.is_ascii_digit())
            })
            .unwrap_or(chars.len())
            .max(i + 1);
        let seg: String = chars[i..j].iter().collect();
        let x = seed(salt, kind, &format!("{word}#{run}"), n);
        run += 1;
        if c.is_ascii_alphabetic() {
            let f = letters(pool, seg.len(), x);
            let upper = seg.chars().all(|c| c.is_ascii_uppercase());
            let title = !upper && seg.chars().next().is_some_and(|c| c.is_ascii_uppercase());
            if upper && seg.len() > 1 {
                out.push_str(&f.to_uppercase());
            } else if title || (upper && seg.len() == 1) {
                let mut cs = f.chars();
                if let Some(h) = cs.next() {
                    out.push(h.to_ascii_uppercase());
                    out.extend(cs);
                }
            } else {
                out.push_str(&f);
            }
        } else if c.is_ascii_digit() {
            out.push_str(&digits(seg.len(), x));
        } else {
            // separators and anything not ASCII stay as they are
            out.push_str(&seg);
        }
        i = j;
    }
    out
}

/// `word`'s whole-name stand-in, in the word's case and with its separator
/// (a word with none: machines and people take `-`, folders and sessions `_`).
/// Account tags and the like keep their shape (`fit`).
pub fn full(salt: &[u8], pools: &Pools, kind: Kind, word: &str, n: u32) -> String {
    let pool = pools.of(kind);
    if kind == Kind::Other || pool.names.is_empty() {
        return fit(salt, pools, kind, word, n);
    }
    // any case of a word gets the same name (in that case)
    let x = seed(salt, kind, &word.to_lowercase(), n);
    let name = &pool.names[(x % pool.names.len() as u64) as usize];
    let sep = if word.contains('_') {
        '_'
    } else if word.contains('-') || matches!(kind, Kind::Host | Kind::User) {
        '-'
    } else {
        '_'
    };
    styled(word, &name.replace('-', &sep.to_string()))
}

/// `name` in `word`'s case: SHOUTED, Title (each part), or as it is.
pub fn styled(word: &str, name: &str) -> String {
    let letters = || word.chars().filter(|c| c.is_alphabetic());
    let any = letters().next().is_some();
    if any && letters().all(|c| c.is_uppercase()) && letters().count() > 1 {
        return name.to_uppercase();
    }
    if word.chars().next().is_some_and(|c| c.is_uppercase()) {
        let mut out = String::with_capacity(name.len());
        let mut up = true;
        for c in name.chars() {
            out.push(if up { c.to_ascii_uppercase() } else { c });
            up = !c.is_ascii_alphanumeric();
        }
        return out;
    }
    name.to_string()
}

/// A version's stand-in of the same width (`0.9.5` → `4.2.0`).
pub fn version(v: &str) -> String {
    let len = v.chars().count();
    // two different versions read differently (`0.9.5` vs `0.8.1` stay two)
    let h = v.bytes().fold(0xcbf2_9ce4_8422_2325u64, |a, b| {
        (a ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    let same: Vec<&&str> = super::words::VERSIONS
        .iter()
        .filter(|x| x.len() == len)
        .collect();
    if !same.is_empty() {
        return same[(h % same.len() as u64) as usize].to_string();
    }
    let mut x = h;
    v.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                x = x.rotate_left(5);
                char::from(b'0' + (x % 10) as u8)
            } else {
                c
            }
        })
        .collect()
}

/// An IPv4 address's stand-in in 127.0.0.0/8, the same width (7–15).
pub fn ipv4(ip: &str, x: u64) -> String {
    let w = ip.len();
    if w < 9 {
        return format!("0.0.0.{}", "4".repeat(w.saturating_sub(6).max(1)));
    }
    // 127. + three octets that fill w - 4 characters (with two dots)
    let rest = w - 4 - 2;
    let mut parts = [1usize, 1, 1];
    let mut left = rest.saturating_sub(3);
    let mut k = 0;
    while left > 0 {
        if parts[k] < 3 {
            parts[k] += 1;
            left -= 1;
        }
        k = (k + 1) % 3;
    }
    let oct = |n: usize, s: u64| -> String {
        match n {
            1 => ((s % 9) + 1).to_string(),
            2 => (10 + s % 90).to_string(),
            _ => (100 + s % 155).to_string(),
        }
    };
    format!(
        "127.{}.{}.{}",
        oct(parts[0], x),
        oct(parts[1], x >> 8),
        oct(parts[2], x >> 16)
    )
}
