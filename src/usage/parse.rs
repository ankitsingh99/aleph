//! Records: what the remote extractor prints (the wire format) and what the
//! local store keeps (one TSV line each). Pure functions, no I/O.
//!
//! Wire lines (tab-separated; the extractor never prints message text):
//!
//! | tag | fields |
//! |---|---|
//! | `u` | ts, session, key, model, effort, folder, input, output, cache read, cache write 5m, cache write 1h, reasoning, cost, flags, web searches, account (optional: the agent's own id for it, or `p:<provider>`) |
//! | `t` | ts, session, key, duration ms (one agent turn) |
//! | `l` | ts, limit id, plan, then used %, window minutes, resets at, resets in s — primary, then secondary |
//!
//! `input` is fresh input (cache reads and writes are separate), `output`
//! includes reasoning (`reasoning` is the part of it that was thinking).
//! `ts` is ISO 8601 or epoch milliseconds. Flags: `s` subagent / sidechain,
//! `f` fast mode, `u` US-only inference.

/// One API request (or one agent turn, `turn_ms > 0` and no tokens).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    /// unix seconds
    pub ts: u64,
    pub host: String,
    pub agent: String,
    pub session: String,
    /// a hash of the agent's own id for the request: duplicates (streamed
    /// lines, resumed sessions, re-reads) collapse onto it
    pub key: String,
    pub model: String,
    pub effort: String,
    /// the folder it ran in (`~/…` when under the machine's home)
    pub project: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub reasoning: u64,
    /// what the agent itself reported (USD), when it does
    pub cost: Option<f64>,
    pub flags: String,
    pub web: u64,
    /// a turn record: how long the agent worked on one prompt
    pub turn_ms: u64,
    /// the account it was made on (16 hex digits, `acct_id`; "" not known:
    /// `store::build` may guess it)
    pub acct: String,
    /// which agent in the session made it: "" the main one, else a subagent's id
    pub lane: String,
    /// the log folder it was read from (`root_id`: a short hash; "" not known)
    pub root: String,
}

impl Record {
    pub fn is_turn(&self) -> bool {
        self.turn_ms > 0
    }
    pub fn sidechain(&self) -> bool {
        self.flags.contains('s')
    }
    pub fn fast(&self) -> bool {
        self.flags.contains('f')
    }
    pub fn us(&self) -> bool {
        self.flags.contains('u')
    }
    /// The provider charged nothing: a `…:free` model (OpenRouter's free
    /// variants), or OpenCode's own models where OpenCode says $0. Its cost
    /// is a known zero, never estimated from the paid model's price.
    pub fn free(&self) -> bool {
        self.flags.contains('z')
    }
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }
    pub fn price_tokens(&self) -> crate::prices::Tokens {
        crate::prices::Tokens {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write_5m: self.cache_write_5m,
            cache_write_1h: self.cache_write_1h,
            web_searches: self.web,
        }
    }
}

/// A rate-limit snapshot: from a log (Codex writes them), the agent's own
/// cache (Claude Code's), or asked of the agent live.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limit {
    pub ts: u64,
    pub id: String,
    pub plan: String,
    pub windows: Vec<Window>,
    /// "" a log, `cache` the agent's own cache, `live` asked just now
    pub source: String,
    /// a hash of the account (live answers say it): one account on two machines is one limit
    pub account: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    pub used_percent: f64,
    pub minutes: u64,
    /// unix seconds
    pub resets_at: Option<u64>,
}

/// 64-bit FNV-1a, as 16 hex digits.
pub fn hash(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h ^= 0;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        for b in p.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

fn n(s: &str) -> u64 {
    let s = s.trim();
    s.parse::<u64>()
        .ok()
        .or_else(|| {
            s.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite() && *x >= 0.0)
                .map(|x| x as u64)
        })
        .unwrap_or(0)
}

fn clean(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Days since 1970-01-01 for a civil date (inverse of `ui::civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2026-10-19T07:44:06.499Z`, `…+05:30`, `2026-10-19 07:44:06`, epoch
/// seconds or milliseconds → unix seconds.
pub fn parse_ts(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        let v: u64 = s.parse().ok()?;
        return Some(if v > 100_000_000_000 { v / 1000 } else { v });
    }
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        let k = r.bytes().take_while(u8::is_ascii_digit).count();
        rest = &r[k..];
    }
    let off: i64 = match rest.as_bytes().first() {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(c @ (b'+' | b'-')) => {
            let t: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            if t.len() < 4 {
                return None;
            }
            let hh: i64 = t[..2].parse().ok()?;
            let mm: i64 = t[2..4].parse().ok()?;
            let v = hh * 3600 + mm * 60;
            if *c == b'+' {
                v
            } else {
                -v
            }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo as u32, d as u32);
    let t = days * 86_400 + h * 3600 + mi * 60 + se - off;
    (t >= 0).then_some(t as u64)
}

/// A `u` or `t` wire line (tag included) → a record for `host`/`agent`.
pub fn from_wire(line: &str, host: &str, agent: &str) -> Option<Record> {
    let f: Vec<&str> = line.split('\t').collect();
    match f.first().copied()? {
        "u" if f.len() >= 16 => {
            let ts = parse_ts(f[1])?;
            let key = f[3].trim();
            if key.is_empty() || key == ":" {
                return None;
            }
            let cost = f[13]
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|c| c.is_finite() && *c > 0.0);
            let r = Record {
                ts,
                host: host.to_string(),
                agent: agent.to_string(),
                session: clean(f[2]),
                key: hash(&[agent, key]),
                model: clean(f[4]),
                effort: clean(f[5]),
                project: clean(f[6]),
                input: n(f[7]),
                output: n(f[8]),
                cache_read: n(f[9]),
                cache_write_5m: n(f[10]),
                cache_write_1h: n(f[11]),
                reasoning: n(f[12]),
                cost,
                flags: {
                    let mut fl: String =
                        f[14].chars().filter(|c| c.is_ascii_alphabetic()).collect();
                    if f[4].trim().to_ascii_lowercase().ends_with(":free") && !fl.contains('z') {
                        fl.push('z');
                    }
                    fl
                },
                web: n(f[15]),
                turn_ms: 0,
                acct: f
                    .get(16)
                    .map(|a| acct_id(agent, host, a))
                    .unwrap_or_default(),
                lane: f.get(17).map(|l| clean(l)).unwrap_or_default(),
                root: String::new(),
            };
            (r.tokens() > 0 || r.cost.is_some()).then_some(r)
        }
        "t" if f.len() >= 5 => {
            let ts = parse_ts(f[1])?;
            let ms = n(f[4]);
            let key = f[3].trim();
            if ms == 0 || key.is_empty() {
                return None;
            }
            Some(Record {
                ts,
                host: host.to_string(),
                agent: agent.to_string(),
                session: clean(f[2]),
                key: hash(&[agent, "turn", key]),
                turn_ms: ms,
                ..Default::default()
            })
        }
        _ => None,
    }
}

/// The account id for what the extractor printed: the agent's own id for the
/// account (Codex's `creator_account_id`), or `p:<provider>` (OpenCode, pi:
/// the provider it went through, on that machine). "" for nothing.
pub fn acct_id(agent: &str, host: &str, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return String::new();
    }
    if raw.starts_with("p:") {
        hash(&[agent, host, raw])
    } else {
        hash(&[agent, raw])
    }
}

/// A log folder on a machine, as records carry it: 8 hex digits.
pub fn root_id(host: &str, root: &str) -> String {
    hash(&[host, root])[..8].to_string()
}

/// An `l` wire line → a limit snapshot.
pub fn limit_from_wire(line: &str) -> Option<Limit> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.first() != Some(&"l") || f.len() < 12 {
        return None;
    }
    let ts = parse_ts(f[1])?;
    let mut windows = Vec::new();
    for w in [&f[4..8], &f[8..12]] {
        let Ok(used) = w[0].trim().parse::<f64>() else {
            continue;
        };
        let resets_at = match n(w[2]) {
            // a time (Claude's), else seconds from now
            0 => parse_ts(w[2]).or(match n(w[3]) {
                0 => None,
                inn => Some(ts + inn),
            }),
            at => Some(if at > 100_000_000_000 { at / 1000 } else { at }),
        };
        windows.push(Window {
            used_percent: used,
            minutes: n(w[1]),
            resets_at,
        });
    }
    if windows.is_empty() {
        return None;
    }
    Some(Limit {
        ts,
        id: clean(f[2]),
        plan: clean(f[3]),
        windows,
        ..Default::default()
    })
}

/// Claude's rate-limit tier as its plan: `default_claude_max_5x` → `Max 5x`,
/// `default_claude_ai` → `Pro`; anything else as it is.
pub fn claude_plan(tier: &str) -> String {
    let t = tier.trim().to_lowercase();
    if let Some(i) = t.find("max_") {
        let x = t[i + 4..].split('_').next().unwrap_or("");
        return if x.is_empty() {
            "Max".into()
        } else {
            format!("Max {x}")
        };
    }
    for (k, name) in [
        ("enterprise", "Enterprise"),
        ("team", "Team"),
        ("pro", "Pro"),
        ("claude_ai", "Pro"),
    ] {
        if t.contains(k) {
            return name.into();
        }
    }
    tier.trim().to_string()
}

/// The store's line for a record (`u` or `t` first, tab-separated).
pub fn to_line(r: &Record) -> String {
    if r.is_turn() {
        return format!(
            "t\t{}\t{}\t{}\t{}\t{}\t{}",
            r.ts, r.host, r.agent, r.session, r.key, r.turn_ms
        );
    }
    format!(
        "u\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        r.ts,
        r.host,
        r.agent,
        r.session,
        r.key,
        r.model,
        r.effort,
        r.project,
        r.input,
        r.output,
        r.cache_read,
        r.cache_write_5m,
        r.cache_write_1h,
        r.reasoning,
        r.cost.map(|c| format!("{c:.6}")).unwrap_or_default(),
        r.flags,
        r.web,
        r.acct,
        r.lane,
        r.root
    )
}

/// A store line → a record.
pub fn from_line(l: &str) -> Option<Record> {
    let f: Vec<&str> = l.split('\t').collect();
    match f.first().copied()? {
        "u" if f.len() >= 18 => Some(Record {
            ts: f[1].parse().ok()?,
            host: f[2].to_string(),
            agent: f[3].to_string(),
            session: f[4].to_string(),
            key: f[5].to_string(),
            model: f[6].to_string(),
            effort: f[7].to_string(),
            project: f[8].to_string(),
            input: n(f[9]),
            output: n(f[10]),
            cache_read: n(f[11]),
            cache_write_5m: n(f[12]),
            cache_write_1h: n(f[13]),
            reasoning: n(f[14]),
            cost: f[15].parse::<f64>().ok().filter(|c| *c > 0.0),
            flags: f[16].to_string(),
            web: n(f[17]),
            turn_ms: 0,
            // added later: older lines have none
            acct: f.get(18).map(|a| a.trim().to_string()).unwrap_or_default(),
            lane: f.get(19).map(|a| a.trim().to_string()).unwrap_or_default(),
            root: f.get(20).map(|a| a.trim().to_string()).unwrap_or_default(),
        }),
        "t" if f.len() >= 7 => Some(Record {
            ts: f[1].parse().ok()?,
            host: f[2].to_string(),
            agent: f[3].to_string(),
            session: f[4].to_string(),
            key: f[5].to_string(),
            turn_ms: n(f[6]),
            ..Default::default()
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn claude_plans() {
        assert_eq!(claude_plan("default_claude_max_5x"), "Max 5x");
        assert_eq!(claude_plan("default_claude_max_20x"), "Max 20x");
        assert_eq!(claude_plan("default_claude_ai"), "Pro");
        assert_eq!(claude_plan("weird"), "weird");
    }

    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(parse_ts("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_ts("2026-10-19T07:44:06.499Z"), Some(1_792_395_846));
        assert_eq!(parse_ts("2026-10-19T13:14:06+05:30"), Some(1_792_395_846));
        assert_eq!(parse_ts("2026-10-19T00:44:06-0700"), Some(1_792_395_846));
        assert_eq!(parse_ts("2026-10-19 07:44:06"), Some(1_792_395_846));
        assert_eq!(parse_ts("1792395846499"), Some(1_792_395_846));
        assert_eq!(parse_ts("1792395846"), Some(1_792_395_846));
        assert_eq!(parse_ts("yesterday"), None);
        assert_eq!(parse_ts("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_ts(""), None);
        assert_eq!(days_from_civil(2026, 10, 22), 20_748);
        assert_eq!(
            crate::ui::civil(days_from_civil(2024, 2, 29)),
            (2024, 2, 29)
        );
    }

    #[test]
    fn wire_and_store_lines_round_trip() {
        let w = "u\t2026-10-22T09:00:00.000Z\tsess-1\tmsg_01:req_01\tclaude-opus-5-5\txhigh\t~/work/app\t12\t340\t5000\t200\t0\t40\t\ts\t1";
        let r = from_wire(w, "b31", "claude").unwrap();
        assert_eq!(r.ts, parse_ts("2026-10-22T09:00:00Z").unwrap());
        assert_eq!(
            (
                r.input,
                r.output,
                r.cache_read,
                r.cache_write_5m,
                r.reasoning
            ),
            (12, 340, 5000, 200, 40)
        );
        assert_eq!(r.key, hash(&["claude", "msg_01:req_01"]));
        assert!(r.sidechain() && !r.fast() && r.cost.is_none() && r.web == 1);
        let back = from_line(&to_line(&r)).unwrap();
        assert_eq!(back, r);
        let t = from_wire(
            "t\t2026-10-22T09:05:00Z\tsess-1\tuuid-9\t61000",
            "b31",
            "claude",
        )
        .unwrap();
        assert!(t.is_turn() && t.turn_ms == 61_000);
        assert_eq!(from_line(&to_line(&t)).unwrap(), t);
        // a reported cost survives; empty and junk lines don't parse
        let c = from_wire(
            "u\t1792395846000\ts\tk\tgpt-5\t\t~\t1\t2\t0\t0\t0\t0\t0.0125\t\t0",
            "h",
            "pi",
        )
        .unwrap();
        assert_eq!(c.cost, Some(0.0125));
        assert_eq!(from_line(&to_line(&c)).unwrap().cost, Some(0.0125));
        assert!(from_wire(
            "u\t2026-10-22T09:00:00Z\ts\tk\tm\t\t\t0\t0\t0\t0\t0\t0\t\t\t0",
            "h",
            "a"
        )
        .is_none());
        assert!(from_wire("u\tnot-a-time\ts\tk\tm", "h", "a").is_none());
        assert!(from_line("garbage").is_none());
        // an account (and a lane) from the wire; the provider kind is per machine
        let a = from_wire(&format!("{w}\tacc-uuid-1\tagent-7"), "b31", "codex").unwrap();
        assert_eq!(a.acct, hash(&["codex", "acc-uuid-1"]));
        assert_eq!(a.lane, "agent-7");
        let p1 = from_wire(&format!("{w}\tp:anthropic"), "b31", "opencode").unwrap();
        let p2 = from_wire(&format!("{w}\tp:anthropic"), "gpu2", "opencode").unwrap();
        assert!(!p1.acct.is_empty() && p1.acct != p2.acct);
        let mut s = a.clone();
        s.root = root_id("b31", "/home/u/.claude/projects");
        assert_eq!(from_line(&to_line(&s)).unwrap(), s);
        // a line written before accounts: 18 fields, still read (no account)
        let old = "u\t1792395846\tb31\tclaude\ts\tk\tm\t\t~\t1\t2\t0\t0\t0\t0\t\t\t0";
        let o = from_line(old).unwrap();
        assert!(o.acct.is_empty() && o.lane.is_empty() && o.root.is_empty() && o.output == 2);
    }

    #[test]
    fn limits_from_the_wire() {
        let l = limit_from_wire(
            "l\t2026-10-19T07:44:06.499Z\tcodex\tpro\t2.0\t10080\t1792957763\t\t37.5\t300\t\t600",
        )
        .unwrap();
        assert_eq!((l.id.as_str(), l.plan.as_str()), ("codex", "pro"));
        assert_eq!(l.windows.len(), 2);
        assert_eq!(l.windows[0].resets_at, Some(1_792_957_763));
        assert_eq!(l.windows[1].resets_at, Some(1_792_395_846 + 600));
        assert!(limit_from_wire("l\tx").is_none());
    }
}
