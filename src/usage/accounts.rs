//! Accounts: which login each request was made on — two Claude Code logins on
//! one machine, one ChatGPT account on three — so usage and its cost split by
//! account under the total (`accounts.json` in the usage store).
//!
//! Where an account comes from, never from a credential:
//!
//! | Agent | The account |
//! |---|---|
//! | Claude Code | its config folder's `.claude.json` `oauthAccount` (id, email, organisation, plan), read beside the logs (`N` lines); none there: an API key, per folder |
//! | Codex | each session's own `creator_account_id` (exact, per request), its plan from the log's rate limits, its email from the app server (`account/read`) |
//! | Gemini CLI | how `settings.json` says it signs in: a Google sign-in, `google_accounts.json`'s `active` email; an API key or Vertex AI, the key's own account per machine (the key never read) |
//! | Antigravity CLI | its Google sign-in: `google_accounts.json`'s `active` email (shared with Gemini CLI) |
//! | OpenCode, pi | the provider each request went through, per machine |
//! | the rest | one per machine |
//!
//! **Exact or guessed.** A folder's account is *sighted* at each read: the
//! account and from when to when it was seen there (or, for an agent whose
//! log names it, the requests that did). A request is stamped only when its
//! time falls inside a sighting; older ones — before Aleph knew accounts, or
//! between two different sightings — keep none, and `store::build` guesses
//! them from the folder's sightings, marked ≈ ([`Book::guess`]). A guess is
//! never written into a record, so a better sighting later changes it.

use super::extract::Output;
use super::parse::{self, Record};
use super::store;
use crate::config::Config;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One account.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    /// 16 hex digits ([`parse::acct_id`]); shown as its first 6 ([`Book::short`])
    pub id: String,
    pub agent: String,
    /// `plan` a subscription sign-in, `api` an API key or a cloud provider,
    /// `provider` a provider through OpenCode or pi, `machine` all an agent
    /// did on one machine (it doesn't say whose)
    pub kind: String,
    pub email: String,
    pub org: String,
    /// `Max 20x`, `pro`, …
    pub plan: String,
    /// the provider (OpenCode, pi)
    pub provider: String,
    pub hosts: BTreeSet<String>,
    pub first: u64,
    pub last: u64,
}

/// An account seen on a machine's log folder, from when to when.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sighting {
    pub host: String,
    /// the log folder's id (`parse::root_id`)
    pub root: String,
    pub acct: String,
    pub from: u64,
    pub to: u64,
}

/// Every account, and where each was seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    pub accounts: BTreeMap<String, Account>,
    pub seen: Vec<Sighting>,
}

fn path() -> std::path::PathBuf {
    store::dir().join("accounts.json")
}

fn gs(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn gu(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

impl Book {
    pub fn load() -> Book {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| Book::from_json(&v))
            .unwrap_or_default()
    }

    pub fn from_json(v: &Value) -> Book {
        let mut b = Book::default();
        for (id, a) in v
            .get("accounts")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            b.accounts.insert(
                id.clone(),
                Account {
                    id: id.clone(),
                    agent: gs(a, "agent"),
                    kind: gs(a, "kind"),
                    email: gs(a, "email"),
                    org: gs(a, "org"),
                    plan: gs(a, "plan"),
                    provider: gs(a, "provider"),
                    hosts: a
                        .get("hosts")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    first: gu(a, "first"),
                    last: gu(a, "last"),
                },
            );
        }
        for s in v
            .get("seen")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            b.seen.push(Sighting {
                host: gs(s, "host"),
                root: gs(s, "root"),
                acct: gs(s, "acct"),
                from: gu(s, "from"),
                to: gu(s, "to"),
            });
        }
        b
    }

    pub fn to_json(&self) -> Value {
        let accounts: serde_json::Map<String, Value> = self
            .accounts
            .iter()
            .map(|(id, a)| {
                (
                    id.clone(),
                    json!({"agent": a.agent, "kind": a.kind, "email": a.email, "org": a.org,
                           "plan": a.plan, "provider": a.provider, "hosts": a.hosts,
                           "first": a.first, "last": a.last}),
                )
            })
            .collect();
        let seen: Vec<Value> = self
            .seen
            .iter()
            .map(|s| json!({"host": s.host, "root": s.root, "acct": s.acct, "from": s.from, "to": s.to}))
            .collect();
        json!({"v": 1, "accounts": accounts, "seen": seen})
    }

    pub fn save(&self) {
        let _ = crate::config::write_atomic(&path(), self.to_json().to_string().as_bytes());
    }

    /// Note an account (made or updated): what's known of it is kept, what's new filled in.
    pub fn note(&mut self, a: Account) -> &mut Account {
        let e = self
            .accounts
            .entry(a.id.clone())
            .or_insert_with(|| Account {
                id: a.id.clone(),
                agent: a.agent.clone(),
                kind: a.kind.clone(),
                ..Default::default()
            });
        for (mine, new) in [
            (&mut e.email, &a.email),
            (&mut e.org, &a.org),
            (&mut e.plan, &a.plan),
            (&mut e.provider, &a.provider),
            (&mut e.kind, &a.kind),
        ] {
            if !new.is_empty() {
                *mine = new.clone();
            }
        }
        e.hosts.extend(a.hosts);
        if a.first > 0 && (e.first == 0 || a.first < e.first) {
            e.first = a.first;
        }
        e.last = e.last.max(a.last);
        e
    }

    /// `acct` seen on `host`'s folder `root` over [from, to]: the folder's
    /// latest sighting grows when it's the same account, else a new one starts.
    pub fn sight(&mut self, host: &str, root: &str, acct: &str, from: u64, to: u64) {
        if acct.is_empty() || root.is_empty() {
            return;
        }
        let last = self
            .seen
            .iter_mut()
            .filter(|s| s.host == host && s.root == root)
            .max_by_key(|s| s.to);
        match last {
            Some(s) if s.acct == acct => {
                s.from = s.from.min(from);
                s.to = s.to.max(to);
            }
            _ => {
                // a request inside another account's sighting doesn't move it
                if let Some(s) = self.seen.iter_mut().find(|s| {
                    s.host == host
                        && s.root == root
                        && s.acct == acct
                        && s.to >= from
                        && s.from <= to
                }) {
                    s.from = s.from.min(from);
                    s.to = s.to.max(to);
                    return;
                }
                self.seen.push(Sighting {
                    host: host.into(),
                    root: root.into(),
                    acct: acct.into(),
                    from,
                    to,
                });
            }
        }
    }

    /// The account a folder was seen on at `ts` (exact), if any.
    pub fn at(&self, host: &str, root: &str, ts: u64) -> Option<&str> {
        self.seen
            .iter()
            .find(|s| s.host == host && s.root == root && s.from <= ts && ts <= s.to)
            .map(|s| s.acct.as_str())
    }

    /// The likeliest account for a request that has none (before Aleph knew
    /// accounts): its folder's, when only one was ever seen there; else the
    /// first one seen after it (or the last one before); else its machine's
    /// agent's, when that's one account.
    pub fn guess(&self, r: &Record) -> Option<String> {
        let mut here: Vec<&Sighting> = self
            .seen
            .iter()
            .filter(|s| s.host == r.host && (r.root.is_empty() || s.root == r.root))
            .filter(|s| {
                self.accounts
                    .get(&s.acct)
                    .is_some_and(|a| a.agent == r.agent)
            })
            .collect();
        here.sort_by_key(|s| s.from);
        let distinct: BTreeSet<&str> = here.iter().map(|s| s.acct.as_str()).collect();
        match distinct.len() {
            // no folder seen on it: the machine's one account of that agent, when it has one
            0 => {
                let mine: Vec<&Account> = self
                    .accounts
                    .values()
                    .filter(|a| {
                        a.agent == r.agent && a.hosts.contains(&r.host) && a.kind != "machine"
                    })
                    .collect();
                (mine.len() == 1).then(|| mine[0].id.clone())
            }
            1 => distinct.into_iter().next().map(str::to_string),
            _ if r.root.is_empty() => None,
            _ => here
                .iter()
                .find(|s| s.from >= r.ts)
                .or(here.last())
                .map(|s| s.acct.clone()),
        }
    }

    /// What the guesses depend on (the rollup is built again when it changes):
    /// the sightings' accounts and starts, not their ends (they grow at every read).
    pub fn stamp(&self) -> String {
        let mut v: Vec<String> = self
            .seen
            .iter()
            .map(|s| format!("{}|{}|{}|{}", s.host, s.root, s.acct, s.from))
            .collect();
        v.sort();
        let parts: Vec<&str> = v.iter().map(String::as_str).collect();
        parse::hash(&parts)
    }

    /// The id as shown: its first 6 digits, more when another account shares them.
    pub fn short(&self, id: &str) -> String {
        let mut n = 6.min(id.len());
        while n < id.len()
            && self
                .accounts
                .keys()
                .any(|k| k != id && k.len() >= n && k[..n] == id[..n])
        {
            n += 2;
        }
        id[..n.min(id.len())].to_string()
    }

    /// An id or its short form (any unique prefix, 4 digits at least), an
    /// email or a nickname → the account's id.
    pub fn find(&self, cfg: &Config, q: &str) -> Option<String> {
        let q = q.trim();
        if q.is_empty() {
            return None;
        }
        let ql = q.to_lowercase();
        if q.len() >= 4 && q.chars().all(|c| c.is_ascii_hexdigit()) {
            let hits: Vec<&String> = self
                .accounts
                .keys()
                .filter(|k| k.starts_with(&ql))
                .collect();
            if hits.len() == 1 {
                return Some(hits[0].clone());
            }
        }
        self.accounts
            .values()
            .find(|a| {
                a.email.eq_ignore_ascii_case(q)
                    || nickname(cfg, self, &a.id).is_some_and(|n| n.eq_ignore_ascii_case(q))
            })
            .map(|a| a.id.clone())
    }
}

/// Its nickname (`analytics.accounts.<id>`, by its short id or any longer prefix).
pub fn nickname(cfg: &Config, b: &Book, id: &str) -> Option<String> {
    let short = b.short(id);
    [short.as_str(), &id[..6.min(id.len())], id]
        .iter()
        .find_map(|k| cfg.get(&format!("analytics.accounts.{k}")))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The word for what an account is on: its plan, else what kind it is.
fn what(a: &Account) -> String {
    if !a.plan.is_empty() {
        return a.plan.clone();
    }
    match a.kind.as_str() {
        "api" => "API key".into(),
        "provider" => a.provider.clone(),
        "machine" => a.hosts.iter().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

/// An account as usage names it (`analytics.account_label`): its nickname
/// when it has one; else its email and plan (`email`), its agent, plan and
/// short id (`plan`), or its agent's numbered account (`nickname`). An
/// account without an email or a plan says what it is: an API key, a
/// provider, a machine.
pub fn label(cfg: &Config, b: &Book, id: &str) -> String {
    if id.is_empty() {
        return "account unknown".into();
    }
    if let Some(n) = nickname(cfg, b, id) {
        return n;
    }
    let short = b.short(id);
    let Some(a) = b.accounts.get(id) else {
        return format!("account {short}");
    };
    let agent = crate::agents::label(&a.agent);
    let w = what(a);
    match cfg.get_or("analytics.account_label").as_str() {
        "nickname" => {
            let mut mine: Vec<&Account> =
                b.accounts.values().filter(|x| x.agent == a.agent).collect();
            mine.sort_by_key(|x| (x.first, x.id.clone()));
            let n = mine.iter().position(|x| x.id == a.id).unwrap_or(0) + 1;
            format!("{agent} account {n} · {short}")
        }
        "email" if !a.email.is_empty() => {
            if w.is_empty() {
                a.email.clone()
            } else {
                format!("{} · {w}", a.email)
            }
        }
        _ => {
            if w.is_empty() {
                format!("{agent} · {short}")
            } else {
                format!("{agent} {w} · {short}")
            }
        }
    }
}

/// The folder (of `roots`) a log is in: the longest that holds it.
fn root_of<'a>(roots: &'a [String], path: &str) -> Option<&'a str> {
    roots
        .iter()
        .filter(|r| path == r.as_str() || path.starts_with(&format!("{r}/")))
        .max_by_key(|r| r.len())
        .map(String::as_str)
}

/// A machine's run, its accounts sorted out (before its records are saved):
/// the folders' accounts noted and sighted, every record given its folder and
/// — when it's known for its time — its account; a Codex log's limits given
/// its account. `now` is when the machine said it read them.
pub fn stamp(b: &mut Book, host: &str, out: &mut Output, now: u64) {
    let roots: HashMap<String, Vec<String>> = out
        .agents
        .iter()
        .map(|a| (a.agent.clone(), a.roots.clone()))
        .collect();
    // the folders' own accounts (N lines): seen there now
    for n in &out.accounts {
        let root = parse::root_id(host, &n.root);
        let (id, a) = match (n.agent.as_str(), n.id.is_empty()) {
            ("claude", false) => (
                parse::hash(&["claude", &n.id]),
                Account {
                    kind: if n.billing.contains("subscription") || !n.tier.is_empty() {
                        "plan".into()
                    } else {
                        "api".into()
                    },
                    email: n.email.clone(),
                    org: n.org.clone(),
                    plan: if n.tier.is_empty() {
                        String::new()
                    } else {
                        parse::claude_plan(&n.tier)
                    },
                    ..Default::default()
                },
            ),
            // no account in its settings: an API key (or a cloud provider), that folder's own
            ("claude", true) => (
                parse::hash(&["claude", "api", host, &n.root]),
                Account {
                    kind: "api".into(),
                    ..Default::default()
                },
            ),
            // Gemini CLI on an API key or Vertex AI: the key's own account, per machine
            ("gemini", true) if n.email.is_empty() && !n.billing.is_empty() => (
                parse::hash(&["gemini", "api", host, &n.billing]),
                Account {
                    kind: "api".into(),
                    plan: if n.billing.contains("vertex") {
                        "Vertex AI".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                },
            ),
            (agent, _) if !n.email.is_empty() || !n.id.is_empty() => (
                parse::hash(&[agent, if n.id.is_empty() { &n.email } else { &n.id }]),
                Account {
                    kind: "plan".into(),
                    email: n.email.clone(),
                    ..Default::default()
                },
            ),
            _ => continue,
        };
        b.note(Account {
            id: id.clone(),
            agent: n.agent.clone(),
            hosts: [host.to_string()].into(),
            first: now,
            last: now,
            ..a
        });
        b.sight(host, &root, &id, now, now);
    }
    for f in &mut out.files {
        let root = roots
            .get(&f.agent)
            .and_then(|rs| root_of(rs, &f.path))
            .map(|r| parse::root_id(host, r))
            .unwrap_or_default();
        let mut span: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for r in &mut f.records {
            r.root = root.clone();
            if r.acct.is_empty() && !r.is_turn() {
                if let Some(a) = b.at(host, &root, r.ts) {
                    r.acct = a.to_string();
                } else if !matches!(r.agent.as_str(), "claude" | "codex" | "gemini") {
                    // an agent that never says whose: one account per machine
                    r.acct = parse::hash(&[&r.agent, "machine", host]);
                    b.note(Account {
                        id: r.acct.clone(),
                        agent: r.agent.clone(),
                        kind: "machine".into(),
                        hosts: [host.to_string()].into(),
                        ..Default::default()
                    });
                }
            }
            if !r.acct.is_empty() {
                let e = span.entry(r.acct.clone()).or_insert((r.ts, r.ts));
                e.0 = e.0.min(r.ts);
                e.1 = e.1.max(r.ts);
            }
        }
        // what the log named itself (Codex's account, a provider): exact, so sighted too
        for (acct, (lo, hi)) in &span {
            let raw = f.accts.get(acct).cloned().unwrap_or_default();
            if !raw.is_empty() || !b.accounts.contains_key(acct) {
                let provider = raw.strip_prefix("p:").unwrap_or("").to_string();
                b.note(Account {
                    id: acct.clone(),
                    agent: f.agent.clone(),
                    kind: if provider.is_empty() {
                        "plan".into()
                    } else {
                        "provider".into()
                    },
                    provider,
                    hosts: [host.to_string()].into(),
                    first: *lo,
                    last: *hi,
                    ..Default::default()
                });
            } else if let Some(a) = b.accounts.get_mut(acct) {
                a.hosts.insert(host.to_string());
                a.last = a.last.max(*hi);
                if a.first == 0 || *lo < a.first {
                    a.first = *lo;
                }
            }
            if !raw.is_empty() {
                b.sight(host, &root, acct, *lo, *hi);
            }
        }
        // a Codex log's plan windows are its account's
        if span.len() == 1 {
            let acct = span.keys().next().cloned().unwrap_or_default();
            for l in &mut f.limits {
                if l.account.is_empty() {
                    l.account = acct.clone();
                }
                if !l.plan.is_empty() {
                    if let Some(a) = b.accounts.get_mut(&acct) {
                        a.plan = l.plan.clone();
                    }
                }
            }
        }
    }
}

/// What Codex's app server said of its account (`live.rs`): its plan, and
/// its email when it gave one.
pub fn note_live(b: &mut Book, host: &str, acct: &str, plan: &str, email: &str, now: u64) {
    if acct.is_empty() {
        return;
    }
    b.note(Account {
        id: acct.to_string(),
        agent: "codex".into(),
        kind: "plan".into(),
        plan: plan.to_string(),
        email: email.to_string(),
        hosts: [host.to_string()].into(),
        last: now,
        ..Default::default()
    });
}

/// An account's label where there's no settings at hand (printed tables, the
/// API): the book and the settings read once, again when either file changes.
pub fn name(id: &str) -> String {
    use std::sync::Mutex;
    /// (the files' stamp, id → label, the book)
    type Names = (u64, HashMap<String, String>, Book);
    static CACHE: Mutex<Option<Names>> = Mutex::new(None);
    let mtime = |p: std::path::PathBuf| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    };
    let stamp = mtime(path()) ^ mtime(Config::path()).rotate_left(1);
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|(s, ..)| *s != stamp) {
        let (cfg, b) = (Config::load(), Book::load());
        let m = b
            .accounts
            .keys()
            .map(|id| (id.clone(), label(&cfg, &b, id)))
            .collect();
        *g = Some((stamp, m, b));
    }
    let (_, m, b) = g.as_ref().expect("filled above");
    match m.get(id) {
        Some(l) => l.clone(),
        None if id.is_empty() => "account unknown".into(),
        None => format!("account {}", b.short(id)),
    }
}

/// Every account, as `aleph usage accounts` and the API list them.
pub fn to_json(cfg: &Config, b: &Book) -> Vec<Value> {
    b.accounts
        .values()
        .map(|a| {
            json!({
                "id": b.short(&a.id),
                "account": a.id,
                "agent": a.agent,
                "label": label(cfg, b, &a.id),
                "kind": a.kind,
                "email": (!a.email.is_empty()).then_some(&a.email),
                "org": (!a.org.is_empty()).then_some(&a.org),
                "plan": (!a.plan.is_empty()).then_some(&a.plan),
                "provider": (!a.provider.is_empty()).then_some(&a.provider),
                "hosts": a.hosts,
                "first": a.first,
                "last": a.last,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::extract::{AgentSeen, FileRun, RootAccount};

    fn rec(agent: &str, ts: u64) -> Record {
        Record {
            ts,
            host: "b31".into(),
            agent: agent.into(),
            session: "s".into(),
            key: format!("k{ts}"),
            input: 1,
            ..Default::default()
        }
    }

    fn run(n: Vec<RootAccount>, files: Vec<FileRun>) -> Output {
        Output {
            agents: vec![
                AgentSeen {
                    agent: "claude".into(),
                    roots: vec![
                        "/h/.claude/projects".into(),
                        "/h/.claude-work/projects".into(),
                    ],
                    ..Default::default()
                },
                AgentSeen {
                    agent: "codex".into(),
                    roots: vec!["/h/.codex/sessions".into()],
                    ..Default::default()
                },
            ],
            accounts: n,
            files,
            ..Default::default()
        }
    }

    fn file(agent: &str, path: &str, recs: Vec<Record>) -> FileRun {
        FileRun {
            agent: agent.into(),
            path: path.into(),
            records: recs,
            ..Default::default()
        }
    }

    #[test]
    fn two_claude_logins_on_one_machine_are_two_accounts() {
        let mut b = Book::default();
        let mut out = run(
            vec![
                RootAccount {
                    agent: "claude".into(),
                    root: "/h/.claude/projects".into(),
                    id: "uuid-me".into(),
                    email: "me@home.dev".into(),
                    tier: "default_claude_max_20x".into(),
                    billing: "stripe_subscription".into(),
                    ..Default::default()
                },
                RootAccount {
                    agent: "claude".into(),
                    root: "/h/.claude-work/projects".into(),
                    id: "uuid-work".into(),
                    email: "me@work.com".into(),
                    tier: "default_claude_max_5x".into(),
                    ..Default::default()
                },
            ],
            vec![
                file(
                    "claude",
                    "/h/.claude/projects/-a/1.jsonl",
                    vec![rec("claude", 1000), rec("claude", 5000)],
                ),
                file(
                    "claude",
                    "/h/.claude-work/projects/-b/2.jsonl",
                    vec![rec("claude", 5000)],
                ),
            ],
        );
        stamp(&mut b, "b31", &mut out, 5000);
        let me = parse::hash(&["claude", "uuid-me"]);
        let work = parse::hash(&["claude", "uuid-work"]);
        // read now: exact; before Aleph knew the folder's account: none (guessed later)
        assert_eq!(out.files[0].records[1].acct, me);
        assert_eq!(out.files[0].records[0].acct, "");
        assert_eq!(out.files[1].records[0].acct, work);
        assert_ne!(out.files[0].records[0].root, out.files[1].records[0].root);
        assert_eq!(
            b.guess(&out.files[0].records[0]).as_deref(),
            Some(me.as_str())
        );
        assert_eq!(b.accounts[&me].plan, "Max 20x");
        assert_eq!(b.accounts[&work].plan, "Max 5x");
        let cfg = Config::in_memory(&[]);
        assert_eq!(label(&cfg, &b, &me), "me@home.dev · Max 20x");
        // the label as asked
        let c2 = Config::in_memory(&["analytics.account_label=plan"]);
        assert_eq!(
            label(&c2, &b, &work),
            format!("Claude Code Max 5x · {}", &work[..6])
        );
        let nick = format!("analytics.accounts.{}=work", &work[..6]);
        let c2 = Config::in_memory(&["analytics.account_label=plan", &nick]);
        assert_eq!(label(&c2, &b, &work), "work");
        assert_eq!(b.find(&c2, "work").as_deref(), Some(work.as_str()));
        assert_eq!(b.find(&c2, &me[..6]).as_deref(), Some(me.as_str()));
        assert_eq!(b.find(&c2, "ME@HOME.DEV").as_deref(), Some(me.as_str()));
        // the same account on another machine is that one account
        let mut out2 = run(
            vec![RootAccount {
                agent: "claude".into(),
                root: "/h/.claude/projects".into(),
                id: "uuid-me".into(),
                ..Default::default()
            }],
            vec![],
        );
        stamp(&mut b, "gpu2", &mut out2, 6000);
        assert_eq!(b.accounts.len(), 2);
        assert!(b.accounts[&me].hosts.contains("gpu2") && b.accounts[&me].hosts.contains("b31"));
    }

    #[test]
    fn a_switch_in_one_folder_leaves_the_gap_to_a_guess() {
        let mut b = Book::default();
        let n = |id: &str| RootAccount {
            agent: "claude".into(),
            root: "/h/.claude/projects".into(),
            id: id.into(),
            ..Default::default()
        };
        let mut o1 = run(vec![n("a")], vec![]);
        stamp(&mut b, "b31", &mut o1, 1000);
        let mut o2 = run(vec![n("a")], vec![]);
        stamp(&mut b, "b31", &mut o2, 2000);
        // logged in to another account between two reads
        let mut o3 = run(
            vec![n("b")],
            vec![file(
                "claude",
                "/h/.claude/projects/-x/1.jsonl",
                vec![
                    rec("claude", 1500),
                    rec("claude", 2500),
                    rec("claude", 3000),
                ],
            )],
        );
        stamp(&mut b, "b31", &mut o3, 3000);
        let (a, bb) = (parse::hash(&["claude", "a"]), parse::hash(&["claude", "b"]));
        let rs = &o3.files[0].records;
        assert_eq!(rs[0].acct, a, "inside a's sighting");
        assert_eq!(rs[1].acct, "", "between the two: not known");
        assert_eq!(rs[2].acct, bb);
        assert_eq!(
            b.guess(&rs[1]).as_deref(),
            Some(bb.as_str()),
            "the next one seen"
        );
        let st = b.stamp();
        b.sight("b31", &rs[0].root, &a, 900, 4000);
        assert_ne!(st, b.stamp());
    }

    #[test]
    fn codex_names_its_account_in_the_log() {
        let mut b = Book::default();
        let mut r1 = rec("codex", 100);
        r1.acct = parse::acct_id("codex", "b31", "acc-1");
        let mut f = file(
            "codex",
            "/h/.codex/sessions/2026/10/rollout-1.jsonl",
            vec![r1.clone(), rec("codex", 50)],
        );
        f.accts.insert(r1.acct.clone(), "acc-1".into());
        f.limits.push(parse::Limit {
            id: "codex".into(),
            plan: "pro".into(),
            ..Default::default()
        });
        let mut out = run(vec![], vec![f]);
        stamp(&mut b, "b31", &mut out, 200);
        let a = &b.accounts[&r1.acct];
        assert_eq!((a.kind.as_str(), a.plan.as_str()), ("plan", "pro"));
        assert_eq!(out.files[0].limits[0].account, r1.acct);
        // the older request in that log, read before: guessed from the log's own
        assert_eq!(out.files[0].records[1].acct, "");
        assert_eq!(
            b.guess(&out.files[0].records[1]).as_deref(),
            Some(r1.acct.as_str())
        );
        note_live(&mut b, "b31", &r1.acct, "pro", "me@chatgpt.dev", 300);
        assert_eq!(b.accounts[&r1.acct].email, "me@chatgpt.dev");
        // a machine account for an agent that never says
        let mut q = file(
            "qwen",
            "/h/.qwen/tmp/x/chats/1.jsonl",
            vec![rec("qwen", 10)],
        );
        q.records[0].agent = "qwen".into();
        let mut out = run(vec![], vec![q]);
        stamp(&mut b, "b31", &mut out, 20);
        let m = &out.files[0].records[0].acct;
        assert_eq!(b.accounts[m].kind, "machine");
        assert_eq!(
            label(&Config::in_memory(&[]), &b, m),
            format!("Qwen Code b31 · {}", &m[..6])
        );
        let back = Book::from_json(&b.to_json());
        assert_eq!(back, b);
    }

    #[test]
    fn short_ids_grow_on_a_clash() {
        let mut b = Book::default();
        for id in ["a3f2c1aa00000000", "a3f2c1bb00000000", "ffff000000000000"] {
            b.note(Account {
                id: id.into(),
                agent: "claude".into(),
                ..Default::default()
            });
        }
        assert_eq!(b.short("a3f2c1aa00000000"), "a3f2c1aa");
        assert_eq!(b.short("ffff000000000000"), "ffff00");
    }
}
