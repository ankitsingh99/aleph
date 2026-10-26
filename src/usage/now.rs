//! Usage now: the daemon's one summary of tokens, cost and quota left —
//! today, the week and 30 days; by agent, machine and account — kept in
//! memory and in `now.json` after every sync (each minute), so the menu bar,
//! Raycast and the dashboard all show the same numbers without reading a log.
//!
//! * Tokens and $ are summed over every machine; a request read on two
//!   machines counts once (`store::dedup`). Machines that are down keep their
//!   history in the totals and say how far behind they are.
//! * An account seen on several machines is one account: its plan, its quota
//!   left (a window that reset since its reading is back at 0 %), the machines it's on,
//!   and the tokens and $ made on it (each request's own account, `accounts`;
//!   one from before Aleph knew whose counts on its guess). An account with no
//!   plan windows (an API key, a provider) is listed too, by its use.
//! * $ is always priced like API usage; usage on a plan says which plan (and
//!   its monthly price, `prices::plan_price`), never discounted.

use super::{accounts, prices, store, Range};
use crate::config::Config;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Sums for one bucket.
#[derive(Clone, Debug, Default, PartialEq)]
struct Sum {
    tokens: u64,
    cost: f64,
    estimated: bool,
    requests: u64,
    unpriced: u64,
}

impl Sum {
    fn add(&mut self, r: &store::Row) {
        self.tokens += r.input + r.output + r.cache_read + r.cache_write_5m + r.cache_write_1h;
        self.cost += r.cost_reported + r.cost_estimated;
        self.estimated |= r.cost_estimated > 0.0;
        self.requests += r.requests;
        self.unpriced += r.unpriced;
    }
    fn to_json(&self) -> Value {
        json!({
            "tokens": self.tokens,
            "cost": (self.cost * 100.0).round() / 100.0,
            "estimated": self.estimated,
            "requests": self.requests,
            "unpriced": self.unpriced,
        })
    }
}

/// today · 7 days · 30 days
#[derive(Clone, Debug, Default)]
struct Spans {
    today: Sum,
    week: Sum,
    month: Sum,
}

impl Spans {
    fn add(&mut self, r: &store::Row, today: i64) {
        self.month.add(r);
        if r.day > today - 7 {
            self.week.add(r);
        }
        if r.day == today {
            self.today.add(r);
        }
    }
    fn to_json(&self) -> Value {
        json!({"today": self.today.to_json(), "week": self.week.to_json(), "month": self.month.to_json()})
    }
}

/// A machine's freshness for the summary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostState {
    pub host: String,
    /// the last sync that worked
    pub ok_at: u64,
    /// the last one tried, and what went wrong
    pub tried_at: u64,
    pub error: Option<String>,
    /// the fleet can't reach it now
    pub down: bool,
    /// (hostname, home) the extractor reported: two aliases, one machine
    pub ident: Option<(String, String)>,
}

/// The windows still worth a number: one that reset after the reading (or a
/// reading older than the window itself) is unknown.
fn live_windows(snap: &Value, now: u64) -> Vec<Value> {
    let ts = snap.get("ts").and_then(Value::as_u64).unwrap_or(0);
    snap.get("windows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|w| {
            let m = w.get("minutes").and_then(Value::as_u64).unwrap_or(0);
            // a window that reset after the reading (or a reading older than
            // the window) starts again from nothing: 0 %
            let reset = match w.get("resets_at").and_then(Value::as_u64) {
                Some(r) => r <= now,
                None => m > 0 && ts + m * 60 <= now,
            };
            let used = if reset {
                0.0
            } else {
                w.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0)
            };
            json!({
                "minutes": m,
                "used": used,
                "left": (100.0 - used).max(0.0),
                "resets_at": if reset { &Value::Null } else { w.get("resets_at").unwrap_or(&Value::Null) },
                "reset": reset,
            })
        })
        .collect()
}

/// The summary (pure — tested): `rows` of the last 30 days, `raw` limits as
/// saved (`store::raw_limits`), the accounts, each machine's state, now and
/// today's day.
pub fn compute(
    cfg: &Config,
    rows: &[store::Row],
    raw: &Value,
    book: &accounts::Book,
    hosts: &[HostState],
    now: u64,
    today: i64,
) -> Value {
    // rows from before records carried accounts: their machine's agent's newest reading's
    // which account each machine's agent is signed in to (its newest reading)
    let mut account_of: BTreeMap<(String, String), (u64, String)> = BTreeMap::new();
    for (h, agents) in raw.as_object().into_iter().flatten() {
        for (agent, ids) in agents.as_object().into_iter().flatten() {
            for s in ids.as_object().into_iter().flatten().map(|(_, s)| s) {
                let Some(a) = s
                    .get("account")
                    .and_then(Value::as_str)
                    .filter(|a| !a.is_empty())
                else {
                    continue;
                };
                let ts = s.get("ts").and_then(Value::as_u64).unwrap_or(0);
                let k = (h.clone(), agent.clone());
                if account_of.get(&k).is_none_or(|(t, _)| ts > *t) {
                    account_of.insert(k, (ts, a.to_string()));
                }
            }
        }
    }
    let mut total = Spans::default();
    let mut by_agent: BTreeMap<String, Spans> = BTreeMap::new();
    let mut by_host: BTreeMap<String, Spans> = BTreeMap::new();
    let mut by_account: BTreeMap<(String, String), Spans> = BTreeMap::new();
    for r in rows {
        total.add(r, today);
        by_agent.entry(r.agent.clone()).or_default().add(r, today);
        by_host.entry(r.host.clone()).or_default().add(r, today);
        let acct = if r.acct.is_empty() {
            account_of
                .get(&(r.host.clone(), r.agent.clone()))
                .map(|(_, a)| a)
        } else {
            Some(&r.acct)
        };
        if let Some(a) = acct {
            by_account
                .entry((r.agent.clone(), a.clone()))
                .or_default()
                .add(r, today);
        }
    }
    // the accounts: one per (agent, account), the newest reading, its machines
    let merged = store::current(raw.clone());
    let mut accounts: Vec<Value> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (h, agents) in merged.as_object().into_iter().flatten() {
        for (agent, ids) in agents.as_object().into_iter().flatten() {
            for s in ids.as_object().into_iter().flatten().map(|(_, s)| s) {
                let acct = s
                    .get("account")
                    .and_then(Value::as_str)
                    .filter(|a| !a.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{h}/{agent}"));
                if !seen.insert((agent.clone(), acct.clone())) {
                    continue;
                }
                let plan = s
                    .get("plan")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let on: Vec<String> = s
                    .get("hosts")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_else(|| vec![h.clone()]);
                let use_ = by_account
                    .get(&(agent.clone(), acct.clone()))
                    .cloned()
                    .unwrap_or_default();
                let windows = live_windows(s, now);
                let price = prices::plan_price(cfg, agent, &plan);
                accounts.push(json!({
                    "agent": agent,
                    "account": acct,
                    "id": book.short(&acct),
                    "label": accounts::label(cfg, book, &acct),
                    "plan": (!plan.is_empty()).then_some(plan.clone()),
                    "hosts": on,
                    "usage": use_.to_json(),
                    "windows": windows,
                    "lapsed": !windows.is_empty() && windows.iter().all(|w| w["reset"] == true),
                    "as_of": s.get("ts"),
                    "source": s.get("source"),
                    // priced as API usage; on a plan it says which (never discounted)
                    "billed": if plan.is_empty() {
                        json!({"kind": "api"})
                    } else {
                        json!({"kind": "plan", "plan": plan, "monthly": price})
                    },
                }));
            }
        }
    }
    // accounts with no plan windows (an API key, a provider, a machine's): by their use
    for ((agent, acct), use_) in &by_account {
        if seen.contains(&(agent.clone(), acct.clone())) || !book.accounts.contains_key(acct) {
            continue;
        }
        let a = &book.accounts[acct];
        let price = prices::plan_price(cfg, agent, &a.plan);
        accounts.push(json!({
            "agent": agent,
            "account": acct,
            "id": book.short(acct),
            "label": accounts::label(cfg, book, acct),
            "plan": (!a.plan.is_empty()).then_some(a.plan.clone()),
            "hosts": a.hosts,
            "usage": use_.to_json(),
            "windows": [],
            "lapsed": false,
            "as_of": Value::Null,
            "source": Value::Null,
            "billed": if a.kind == "plan" && !a.plan.is_empty() {
                json!({"kind": "plan", "plan": a.plan, "monthly": price})
            } else {
                json!({"kind": "api"})
            },
        }));
    }
    // the fullest first
    let fullest = |a: &Value| {
        a["windows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w["used"].as_f64())
            .fold(-1.0, f64::max)
    };
    accounts.sort_by(|a, b| fullest(b).total_cmp(&fullest(a)));
    // machines: their numbers, how fresh, and aliases that are one machine
    let mut same: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for h in hosts {
        if let Some(id) = &h.ident {
            same.entry(id.clone()).or_default().push(h.host.clone());
        }
    }
    let machines: Vec<Value> = hosts
        .iter()
        .map(|h| {
            let state = if h.down {
                "offline"
            } else if h.error.is_some() && h.tried_at > h.ok_at {
                "error"
            } else if h.ok_at == 0 {
                "never"
            } else if now.saturating_sub(h.ok_at) > 600 {
                "behind"
            } else {
                "ok"
            };
            let also: Vec<&String> = h
                .ident
                .as_ref()
                .and_then(|id| same.get(id))
                .into_iter()
                .flatten()
                .filter(|x| **x != h.host)
                .collect();
            json!({
                "host": h.host,
                "state": state,
                "synced_at": h.ok_at,
                "behind_s": (h.ok_at > 0).then(|| now.saturating_sub(h.ok_at)),
                "error": h.error,
                "same_as": also,
                "usage": by_host.get(&h.host).cloned().unwrap_or_default().to_json(),
            })
        })
        .collect();
    let mut agents: Vec<(String, Spans)> = by_agent.into_iter().collect();
    agents.sort_by(|a, b| {
        b.1.week
            .cost
            .total_cmp(&a.1.week.cost)
            .then(b.1.week.tokens.cmp(&a.1.week.tokens))
    });
    json!({
        "v": 1,
        "generated_at": now,
        "total": total.to_json(),
        "agents": agents.iter().map(|(a, s)| {
            let mut v = s.to_json();
            v["agent"] = json!(a);
            v["accounts"] = json!(accounts.iter().filter(|x| x["agent"] == json!(a)).map(|x| x["account"].clone()).collect::<Vec<_>>());
            v
        }).collect::<Vec<_>>(),
        "accounts": accounts,
        "machines": machines,
    })
}

/// Each machine's state, from the offsets store and who's down now.
pub fn host_states(hosts: &[String], down: &[String]) -> Vec<HostState> {
    let st = store::Offsets::load();
    hosts
        .iter()
        .map(|h| {
            let s = st.hosts.get(h);
            let run = s.and_then(|x| x.run.as_ref());
            HostState {
                host: h.clone(),
                ok_at: s.map(|x| x.ok_at).unwrap_or(0),
                tried_at: run.map(|r| r.at).unwrap_or(0),
                error: run.and_then(|r| r.error.clone()),
                down: down.contains(h),
                ident: run
                    .filter(|r| !r.hostname.is_empty())
                    .map(|r| (r.hostname.clone(), r.home.clone())),
            }
        })
        .collect()
}

fn file() -> std::path::PathBuf {
    store::dir().join("now.json")
}

/// Work the summary out and keep it (`now.json`, written atomically).
pub fn refresh(cfg: &Config, down: &[String]) -> Value {
    let rows = super::rows(cfg, Range::days(30), &super::Filter::default());
    let hosts = host_states(&cfg.fleet(), down);
    let v = compute(
        cfg,
        &rows,
        &store::raw_limits(),
        &accounts::Book::load(),
        &hosts,
        crate::cache::now(),
        Range::today().from,
    );
    let _ = std::fs::create_dir_all(store::dir());
    let _ = crate::config::write_atomic(&file(), v.to_string().as_bytes());
    v
}

/// The kept summary (None: never worked out yet).
pub fn load() -> Option<Value> {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

/// The kept summary, read again at most every 10 s (for what draws every
/// frame: the dashboard's header).
pub fn cached() -> Option<Value> {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    static C: OnceLock<Mutex<(Option<Instant>, Option<Value>)>> = OnceLock::new();
    let m = C.get_or_init(|| Mutex::new((None, None)));
    let mut g = m.lock().unwrap();
    if g.0.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
        *g = (Some(Instant::now()), load());
    }
    g.1.clone()
}

/// `usage.now`: the kept one when it's under two minutes old, else worked
/// out now (and kept).
pub fn get(cfg: &Config) -> Value {
    if let Some(v) = load() {
        let at = v.get("generated_at").and_then(Value::as_u64).unwrap_or(0);
        if crate::cache::now().saturating_sub(at) < 120 {
            return v;
        }
    }
    refresh(cfg, &[])
}

/// What changed between two summaries, apart from when they were made.
pub fn same_numbers(a: &Value, b: &Value) -> bool {
    let strip = |v: &Value| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("generated_at");
            if let Some(m) = o.get_mut("machines").and_then(Value::as_array_mut) {
                for x in m.iter_mut().filter_map(Value::as_object_mut) {
                    x.remove("behind_s");
                }
            }
        }
        v
    };
    strip(a) == strip(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: i64, host: &str, agent: &str, input: u64, cost: f64) -> store::Row {
        store::Row {
            day,
            host: host.into(),
            agent: agent.into(),
            input,
            requests: 1,
            cost_estimated: cost,
            ..Default::default()
        }
    }

    #[test]
    fn two_logins_on_one_machine_each_have_their_own_use() {
        use accounts::{Account, Book};
        let (now, today) = (1_800_000_000, 20_000);
        let (me, work) = ("aaaa000000000000", "bbbb000000000000");
        let mut a = row(today, "b31", "claude", 100, 1.0);
        a.acct = me.into();
        let mut b = row(today, "b31", "claude", 30, 0.3);
        b.acct = work.into();
        let mut c = row(today, "b31", "opencode", 5, 0.05);
        c.acct = "cccc000000000000".into();
        let raw = json!({
            "b31": {"claude": {
                "claude@aaaa0000": {"ts": now - 60, "plan": "Max 20x", "account": me, "source": "cache",
                    "windows": [{"used_percent": 10.0, "minutes": 300, "resets_at": now + 600}]},
                "claude@bbbb0000": {"ts": now - 60, "plan": "Pro", "account": work, "source": "cache",
                    "windows": [{"used_percent": 70.0, "minutes": 300, "resets_at": now + 600}]}}},
        });
        let mut book = Book::default();
        for (id, agent, kind) in [
            (me, "claude", "plan"),
            (work, "claude", "plan"),
            ("cccc000000000000", "opencode", "provider"),
        ] {
            book.note(Account {
                id: id.into(),
                agent: agent.into(),
                kind: kind.into(),
                provider: "anthropic".into(),
                hosts: ["b31".to_string()].into(),
                ..Default::default()
            });
        }
        let v = compute(
            &Config::in_memory(&[]),
            &[a, b, c],
            &raw,
            &book,
            &[],
            now,
            today,
        );
        let accts = v["accounts"].as_array().unwrap();
        let u = |id: &str| {
            accts.iter().find(|x| x["account"] == id).unwrap()["usage"]["today"]["tokens"].clone()
        };
        assert_eq!(
            (u(me), u(work)),
            (json!(100), json!(30)),
            "each login its own, not the newest's"
        );
        let oc = accts.iter().find(|x| x["agent"] == "opencode").unwrap();
        assert_eq!(oc["billed"]["kind"], "api");
        assert_eq!(oc["label"], "OpenCode anthropic · cccc00");
        // the fullest first
        assert_eq!(accts[0]["account"], work);
    }

    #[test]
    fn one_account_on_two_machines_is_one_row_with_both() {
        let now = 1_800_000_000;
        let today = 20_000;
        let rows = vec![
            row(today, "local", "claude", 100, 1.0),
            row(today - 2, "dex42", "claude", 50, 0.5),
            row(today, "b31", "claude", 10, 0.1),
            row(today - 20, "b31", "codex", 7, 0.07),
        ];
        let raw = json!({
            "local": {"claude": {"claude": {"ts": now - 60, "plan": "Max 5x", "account": "m", "source": "cache",
                "windows": [{"used_percent": 44.0, "minutes": 10080, "resets_at": now + 3600}]}}},
            "dex42": {"claude": {"claude": {"ts": now - 120, "plan": "Max 5x", "account": "m", "source": "cache",
                "windows": [{"used_percent": 40.0, "minutes": 10080, "resets_at": now + 3600}]}}},
            "b31": {"claude": {"claude": {"ts": now - 60, "plan": "Pro", "account": "p", "source": "cache",
                "windows": [{"used_percent": 96.0, "minutes": 10080, "resets_at": now - 10}]}}},
        });
        let hosts = vec![
            HostState {
                host: "local".into(),
                ok_at: now - 30,
                ident: Some(("mac".into(), "/Users/u".into())),
                ..Default::default()
            },
            HostState {
                host: "dex42".into(),
                ok_at: now - 3000,
                ..Default::default()
            },
            HostState {
                host: "b31".into(),
                ok_at: now - 30,
                down: true,
                ..Default::default()
            },
            HostState {
                host: "mac2".into(),
                ok_at: now - 30,
                ident: Some(("mac".into(), "/Users/u".into())),
                ..Default::default()
            },
        ];
        let v = compute(
            &Config::in_memory(&[]),
            &rows,
            &raw,
            &accounts::Book::default(),
            &hosts,
            now,
            today,
        );
        assert_eq!(v["total"]["week"]["tokens"], 160);
        assert_eq!(v["total"]["month"]["tokens"], 167);
        assert_eq!(v["total"]["today"]["tokens"], 110);
        let accts = v["accounts"].as_array().unwrap();
        assert_eq!(accts.len(), 2, "{accts:#?}");
        let m = accts.iter().find(|a| a["account"] == "m").unwrap();
        assert_eq!(
            m["hosts"],
            json!(["local", "dex42"]),
            "one account, both machines"
        );
        assert_eq!(m["usage"]["week"]["tokens"], 150, "the account's own usage");
        assert_eq!(m["windows"][0]["left"], 56.0);
        assert_eq!(m["billed"]["kind"], "plan");
        assert_eq!(m["billed"]["monthly"], 100.0);
        let p = accts.iter().find(|a| a["account"] == "p").unwrap();
        assert!(
            p["windows"][0]["used"] == 0.0
                && p["windows"][0]["left"] == 100.0
                && p["lapsed"] == true,
            "reset since: 0 %, not 96 %"
        );
        let ms = v["machines"].as_array().unwrap();
        let st = |h: &str| ms.iter().find(|x| x["host"] == h).unwrap()["state"].clone();
        assert_eq!(
            (st("local"), st("dex42"), st("b31")),
            (json!("ok"), json!("behind"), json!("offline"))
        );
        assert_eq!(
            ms.iter().find(|x| x["host"] == "local").unwrap()["same_as"],
            json!(["mac2"])
        );
        assert_eq!(v["agents"][0]["agent"], "claude");
    }

    #[test]
    fn same_numbers_ignore_the_clock() {
        let a = json!({"generated_at": 1, "total": {"week": {"tokens": 5}}, "machines": [{"host": "x", "behind_s": 3}]});
        let mut b = a.clone();
        b["generated_at"] = json!(2);
        b["machines"][0]["behind_s"] = json!(9);
        assert!(same_numbers(&a, &b));
        b["total"]["week"]["tokens"] = json!(6);
        assert!(!same_numbers(&a, &b));
    }
}
