//! What Usage's screens draw beyond the totals (computed in the same pass
//! off the UI thread, `dash::load`): the cost broken down by a split (account,
//! agent, machine, project), a series a day for the charts, this period
//! against the one before, the model mix, prompts' and sessions' spread, the
//! tools the agents used, the last 12 weeks a day, and the insights.

use super::store::Row as SRow;
use super::{group, split, By, Filter, Range, Row, Stats};
use crate::config::Config;
use std::collections::{BTreeMap, HashMap};

/// The most series a chart shows (the rest is "other").
pub const SERIES: usize = 6;

/// Everything beyond the totals.
#[derive(Clone, Debug, Default)]
pub struct More {
    /// the split the breakup and the daily columns use
    pub split: Option<By>,
    /// (key, label, cost, tokens), biggest first
    pub breakup: Vec<(String, String, f64, u64)>,
    /// the daily columns' series: (key, label); the last may be "other"
    pub series: Vec<(String, String)>,
    /// (day number, label, cost per series)
    pub daily: Vec<(i64, String, Vec<f64>)>,
    /// cost so far, a day at a time: this period, and the one before (same length)
    pub cum: Vec<f64>,
    pub cum_prev: Vec<f64>,
    /// tokens, requests, prompts, sessions, active hours, cache hits (0–1) a day
    pub per_day: BTreeMap<&'static str, Vec<f64>>,
    /// the model mix: models (top ones, then "other") and each day's share of tokens
    pub models: Vec<String>,
    pub model_mix: Vec<(String, Vec<f64>)>,
    /// prompts' and sessions' spread, the costliest
    pub stats: Stats,
    /// the breakdown's tree: each root row and its split
    pub tree: Vec<(Row, Vec<Row>)>,
    /// cost a day over the last 12 weeks (the filters kept, the period not)
    pub calendar: Vec<(i64, f64)>,
    /// tools used in the period: (name, calls, failed), most first
    pub tools: Vec<(String, u64, u64)>,
    /// the most sessions at work in one hour
    pub parallel: u64,
    /// (label, value) lines
    pub insights: Vec<(String, String)>,
    /// plans: (account label, plan, $ a month, API value of the last 30 days)
    pub plans: Vec<(String, String, f64, f64)>,
}

fn label_of(by: By, key: &str, names: &HashMap<String, String>) -> String {
    match by {
        By::Account => names
            .get(key)
            .cloned()
            .unwrap_or_else(|| "account unknown".into()),
        By::Agent => crate::agents::label(key),
        By::Project => super::projects::name_of(key),
        _ if key.is_empty() => "(none)".into(),
        _ => key.to_string(),
    }
}

fn key_of(by: By, r: &SRow) -> String {
    match by {
        By::Account => r.acct.clone(),
        By::Agent => r.agent.clone(),
        By::Host => r.host.clone(),
        By::Project => {
            if r.pid.is_empty() {
                r.project.clone()
            } else {
                r.pid.clone()
            }
        }
        By::Model => r.model.clone(),
        _ => r.project.clone(),
    }
}

fn cost(r: &SRow) -> f64 {
    r.cost_reported + r.cost_estimated
}

fn tokens(r: &SRow) -> u64 {
    r.input + r.output + r.cache_read + r.cache_write_5m + r.cache_write_1h
}

/// The days a range covers (all time: from its first day with data).
fn days_of(range: Range, rs: &[SRow]) -> (i64, i64) {
    let from = if range.from <= 0 {
        rs.iter().map(|r| r.day).min().unwrap_or(range.to)
    } else {
        range.from
    };
    (from, range.to.max(from))
}

/// Work it all out.
#[allow(clippy::too_many_arguments)]
pub fn more(
    cfg: &Config,
    range: Range,
    filter: &Filter,
    split_by: By,
    root: By,
    tree_split: By,
    rs: &[SRow],
    names: &HashMap<String, String>,
) -> More {
    let mut m = More {
        split: Some(split_by),
        ..Default::default()
    };
    // the breakup, and the series the daily columns use
    let mut by_key: HashMap<String, (f64, u64)> = HashMap::new();
    for r in rs {
        let e = by_key.entry(key_of(split_by, r)).or_default();
        e.0 += cost(r);
        e.1 += tokens(r);
    }
    let mut parts: Vec<(String, f64, u64)> =
        by_key.into_iter().map(|(k, (c, t))| (k, c, t)).collect();
    parts.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
    m.breakup = parts
        .iter()
        .map(|(k, c, t)| (k.clone(), label_of(split_by, k, names), *c, *t))
        .collect();
    m.series = parts
        .iter()
        .take(SERIES)
        .map(|(k, ..)| (k.clone(), label_of(split_by, k, names)))
        .collect();
    if parts.len() > SERIES {
        m.series.push(("\u{0}other".into(), "other".into()));
    }
    let slot = |k: &str| {
        m.series
            .iter()
            .position(|(s, _)| s == k)
            .unwrap_or(m.series.len().saturating_sub(1))
    };
    let (from, to) = days_of(range, rs);
    let n = (to - from + 1).clamp(1, 4000) as usize;
    let mut daily: Vec<(i64, String, Vec<f64>)> = (0..n)
        .map(|i| {
            let d = from + i as i64;
            let (_, mo, dd) = crate::ui::civil(d);
            (d, format!("{mo}/{dd}"), vec![0.0; m.series.len()])
        })
        .collect();
    let mut per_day: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    for k in ["tokens", "requests", "prompts", "active", "cache"] {
        per_day.insert(k, vec![0.0; n]);
    }
    let mut cache_in = vec![(0.0f64, 0.0f64); n];
    let mut sessions_day: Vec<std::collections::HashSet<(String, String, String)>> =
        vec![Default::default(); n];
    let mut hour_sessions: HashMap<(i64, u8), std::collections::HashSet<(String, String)>> =
        HashMap::new();
    let mut models: HashMap<String, u64> = HashMap::new();
    for r in rs {
        let i = (r.day - from).clamp(0, n as i64 - 1) as usize;
        if !m.series.is_empty() {
            daily[i].2[slot(&key_of(split_by, r))] += cost(r);
        }
        per_day.get_mut("tokens").unwrap()[i] += tokens(r) as f64;
        per_day.get_mut("requests").unwrap()[i] += r.requests as f64;
        per_day.get_mut("prompts").unwrap()[i] += r.prompts as f64;
        per_day.get_mut("active").unwrap()[i] += r.active_ms.max(r.turn_ms) as f64 / 3_600_000.0;
        cache_in[i].0 += r.cache_read as f64;
        cache_in[i].1 += (r.input + r.cache_write_5m + r.cache_write_1h) as f64;
        if r.requests > 0 {
            sessions_day[i].insert((r.host.clone(), r.agent.clone(), r.session.clone()));
            hour_sessions
                .entry((r.day, r.hour))
                .or_default()
                .insert((r.host.clone(), r.session.clone()));
        }
        if !r.model.is_empty() {
            *models.entry(r.model.clone()).or_default() += tokens(r);
        }
    }
    per_day.insert(
        "sessions",
        sessions_day.iter().map(|s| s.len() as f64).collect(),
    );
    let ch = per_day.get_mut("cache").unwrap();
    for (i, (hit, fresh)) in cache_in.iter().enumerate() {
        ch[i] = if hit + fresh > 0.0 {
            hit / (hit + fresh)
        } else {
            0.0
        };
    }
    m.per_day = per_day;
    m.parallel = hour_sessions
        .values()
        .map(|s| s.len() as u64)
        .max()
        .unwrap_or(0);
    m.daily = daily;
    // this period's cost so far against the one before
    let day_cost: Vec<f64> = m.daily.iter().map(|d| d.2.iter().sum()).collect();
    let mut acc = 0.0;
    m.cum = day_cost
        .iter()
        .map(|c| {
            acc += c;
            acc
        })
        .collect();
    let prev = super::rows(cfg, Range::between(from - n as i64, from - 1), filter);
    let mut pd = vec![0.0; n];
    for r in &prev {
        let i = r.day - (from - n as i64);
        if (0..n as i64).contains(&i) {
            pd[i as usize] += cost(r);
        }
    }
    let mut acc = 0.0;
    m.cum_prev = pd
        .iter()
        .map(|c| {
            acc += c;
            acc
        })
        .collect();
    // the model mix: each day's tokens by model, as shares
    let mut ms: Vec<(String, u64)> = models.into_iter().collect();
    ms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    m.models = ms
        .iter()
        .take(5)
        .map(|(k, _)| crate::timeline::short_model(k))
        .collect();
    if ms.len() > 5 {
        m.models.push("other".into());
    }
    let top: Vec<String> = ms.iter().take(5).map(|(k, _)| k.clone()).collect();
    let mut mix = vec![vec![0.0; m.models.len()]; n];
    for r in rs {
        let i = (r.day - from).clamp(0, n as i64 - 1) as usize;
        let j = top
            .iter()
            .position(|x| *x == r.model)
            .unwrap_or(m.models.len().saturating_sub(1));
        if let Some(v) = mix[i].get_mut(j) {
            *v += tokens(r) as f64;
        }
    }
    m.model_mix = m
        .daily
        .iter()
        .zip(mix)
        .map(|((_, l, _), v)| {
            let t: f64 = v.iter().sum();
            (
                l.clone(),
                v.iter()
                    .map(|x| if t > 0.0 { x / t } else { 0.0 })
                    .collect(),
            )
        })
        .collect();
    // spread, the breakdown's tree
    m.stats = super::stats(cfg, range, filter, 10);
    m.tree = split(rs, root, tree_split);
    // 12 weeks a day, the filters kept
    let today = crate::ui::local_day(crate::cache::now());
    let cal = super::rows(cfg, Range::between(today - 7 * 12 + 1, today), filter);
    let mut cd: BTreeMap<i64, f64> = BTreeMap::new();
    for r in &cal {
        *cd.entry(r.day).or_default() += cost(r);
    }
    m.calendar = cd.into_iter().collect();
    // the tools the agents used (the timeline's steps)
    m.tools = tools(range, filter);
    m.plans = plans(cfg);
    m.insights = insights(cfg, &m, rs, range, filter);
    m
}

/// Tool calls in the period (the machines and agents kept): (name, calls, failed).
fn tools(range: Range, filter: &Filter) -> Vec<(String, u64, u64)> {
    use super::steps::{self, Kind};
    let tz = crate::ui::utc_offset();
    let mut by: HashMap<String, (u64, u64)> = HashMap::new();
    let mut failed: std::collections::HashSet<(String, String)> = Default::default();
    let mut starts: Vec<(String, String, String)> = Vec::new();
    for m in steps::months().into_iter().rev().take(7) {
        for s in steps::read_month(&m) {
            let local = (s.ms / 1000) as i64 + tz;
            let day = local.div_euclid(86_400);
            let hour = (local.rem_euclid(86_400) / 3600) as u8;
            if !range.keeps(day, hour, tz) || !filter.keeps_parts(&s.host, &s.agent, "", "", "") {
                continue;
            }
            match s.kind {
                Kind::ToolStart => starts.push((s.host, s.key, s.name)),
                Kind::ToolEnd if matches!(s.name.as_str(), "err" | "denied") => {
                    failed.insert((s.host, s.key));
                }
                _ => {}
            }
        }
    }
    for (h, k, name) in starts {
        let e = by.entry(name).or_default();
        e.0 += 1;
        if failed.contains(&(h, k)) {
            e.1 += 1;
        }
    }
    let mut v: Vec<(String, u64, u64)> = by.into_iter().map(|(n, (c, f))| (n, c, f)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// Each plan account: what it costs a month, and the API value of its last 30 days.
fn plans(cfg: &Config) -> Vec<(String, String, f64, f64)> {
    let v = super::now::load().unwrap_or_default();
    v.get("accounts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let b = a.get("billed")?;
            let monthly = b.get("monthly").and_then(serde_json::Value::as_f64)?;
            let plan = b
                .get("plan")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            let label = a
                .get("label")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| plan.clone());
            let used = a["usage"]["month"]["cost"].as_f64().unwrap_or(0.0);
            Some((label, plan, monthly, used))
        })
        .filter(|_| cfg.get_or("analytics.cost") != "off")
        .collect()
}

/// The insights: (what, the line).
fn insights(
    cfg: &Config,
    m: &More,
    rs: &[SRow],
    range: Range,
    filter: &Filter,
) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    let money = |c: f64| crate::timeline::money(c);
    if rs.is_empty() {
        return v;
    }
    // streaks: days in a row with usage, up to today and the longest (12 weeks)
    let today = crate::ui::local_day(crate::cache::now());
    let busy: std::collections::BTreeSet<i64> = m
        .calendar
        .iter()
        .filter(|(_, c)| *c > 0.0)
        .map(|(d, _)| *d)
        .collect();
    let mut cur = 0;
    let mut d = today;
    while busy.contains(&d) {
        cur += 1;
        d -= 1;
    }
    let (mut best, mut run, mut last) = (0, 0, i64::MIN);
    for &d in &busy {
        run = if d == last + 1 { run + 1 } else { 1 };
        best = best.max(run);
        last = d;
    }
    v.push((
        "streak".into(),
        format!("{cur} days now · {best} the longest (12 weeks)"),
    ));
    // the busiest day and hour
    if let Some((day, _, vals)) = m
        .daily
        .iter()
        .max_by(|a, b| a.2.iter().sum::<f64>().total_cmp(&b.2.iter().sum::<f64>()))
    {
        v.push((
            "busiest day".into(),
            format!("{} · {}", super::day_name(*day), money(vals.iter().sum())),
        ));
    }
    let mut hours = [0.0f64; 24];
    let mut wd = [0.0f64; 7];
    for r in rs {
        hours[r.hour as usize % 24] += cost(r);
        wd[(r.day + 3).rem_euclid(7) as usize] += cost(r);
    }
    if let Some((h, _)) = hours.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) {
        v.push((
            "busiest hour".into(),
            format!("{h:02}:00–{:02}:00", (h + 1) % 24),
        ));
    }
    let total: f64 = hours.iter().sum();
    let night: f64 = hours[..6].iter().sum();
    if total > 0.0 {
        v.push((
            "night owl".into(),
            format!(
                "{:.0}% of the cost between midnight and 6",
                100.0 * night / total
            ),
        ));
        let we = wd[5] + wd[6];
        v.push((
            "weekends".into(),
            format!("{:.0}% of it", 100.0 * we / total),
        ));
    }
    // the favourite model, caching's savings
    let by_model = group(rs, By::Model);
    if let Some(top) = by_model.iter().max_by_key(|r| r.tokens()) {
        v.push((
            "favourite model".into(),
            format!(
                "{} · {} tokens",
                crate::timeline::short_model(&top.key),
                crate::usage::dash::tokens(top.tokens())
            ),
        ));
    }
    let t = group(rs, By::Total).pop().unwrap_or_default();
    let fresh = (t.input + t.cache_write()) as f64;
    if t.cache_read > 0 {
        v.push((
            "cache hits".into(),
            format!(
                "{:.0}% of input came from the cache",
                100.0 * t.cache_read as f64 / (t.cache_read as f64 + fresh)
            ),
        ));
    }
    // where it went
    if let Some(p) = group(rs, By::Project)
        .first()
        .filter(|p| !p.key.is_empty() && t.cost() > 0.0)
    {
        v.push((
            "top project".into(),
            format!(
                "{} · {:.0}% of the cost",
                super::projects::name_of(&p.key),
                100.0 * p.cost() / t.cost()
            ),
        ));
    }
    if let Some(p) = m.stats.top_prompts.first() {
        v.push((
            "costliest prompt".into(),
            format!(
                "{} · {} · {} · {}",
                money(p.cost),
                crate::agents::label(&p.agent),
                p.host,
                crate::ui::dur(p.secs)
            ),
        ));
    }
    if let Some(s) = group(rs, By::Session)
        .iter()
        .max_by_key(|r| r.active_secs())
    {
        v.push(("longest session".into(), crate::ui::dur(s.active_secs())));
    }
    // $ a prompt by agent
    let per: Vec<String> = group(rs, By::Agent)
        .iter()
        .filter_map(|r| {
            r.per_prompt()
                .map(|c| format!("{} {}", crate::agents::label(&r.key), money(c)))
        })
        .collect();
    if !per.is_empty() {
        v.push(("$ a prompt".into(), per.join(" · ")));
    }
    if let Some((name, n, f)) = m.tools.first() {
        let all: u64 = m.tools.iter().map(|x| x.1).sum();
        let bad: u64 = m.tools.iter().map(|x| x.2).sum();
        v.push((
            "tools".into(),
            format!(
                "{all} calls · {name} the most ({n}, {f} failed) · {:.0}% failed",
                if all > 0 {
                    100.0 * bad as f64 / all as f64
                } else {
                    0.0
                }
            ),
        ));
    }
    if m.parallel > 1 {
        v.push((
            "in parallel".into(),
            format!("{} sessions at work in one hour, at most", m.parallel),
        ));
    }
    // the period against the one before
    let prev: f64 = m.cum_prev.last().copied().unwrap_or(0.0);
    let now: f64 = m.cum.last().copied().unwrap_or(0.0);
    if prev > 0.0 {
        v.push((
            "vs the period before".into(),
            format!(
                "{:+.0}% ({} then)",
                100.0 * (now - prev) / prev,
                money(prev)
            ),
        ));
    }
    // this month's pace
    let (y, mo, dd) = crate::ui::civil(today);
    let first = crate::usage::parse::days_from_civil(y, mo, 1);
    let next = if mo == 12 {
        crate::usage::parse::days_from_civil(y + 1, 1, 1)
    } else {
        crate::usage::parse::days_from_civil(y, mo + 1, 1)
    };
    let month = super::rows(cfg, Range::between(first, today), filter);
    let so_far: f64 = month.iter().map(cost).sum();
    if so_far > 0.0 && dd > 1 {
        v.push((
            "this month".into(),
            format!(
                "{} so far · about {} by its end",
                money(so_far),
                money(so_far / dd as f64 * (next - first) as f64)
            ),
        ));
    }
    for (label, plan, monthly, used) in &m.plans {
        if *monthly > 0.0 && *used > 0.0 {
            v.push((
                "plan value".into(),
                format!(
                    "{label}: {} of API use on a {} {plan} plan · {:.1}×",
                    money(*used),
                    money(*monthly),
                    used / monthly
                ),
            ));
        }
    }
    let _ = range;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: i64, hour: u8, agent: &str, acct: &str, cost: f64) -> SRow {
        SRow {
            day,
            hour,
            host: "b31".into(),
            agent: agent.into(),
            session: format!("{agent}-{day}"),
            model: "claude-opus-5-5".into(),
            acct: acct.into(),
            input: 100,
            output: 10,
            cache_read: 1000,
            requests: 2,
            prompts: 1,
            cost_estimated: cost,
            ..Default::default()
        }
    }

    #[test]
    fn the_breakup_and_the_days() {
        let today = crate::ui::local_day(crate::cache::now());
        let rs = vec![
            row(today, 9, "claude", "aaaa", 2.0),
            row(today, 10, "claude", "bbbb", 1.0),
            row(today - 1, 9, "codex", "cccc", 0.5),
        ];
        let names: HashMap<String, String> = [("aaaa".to_string(), "work".to_string())].into();
        let m = more(
            &Config::in_memory(&[]),
            Range::days(3),
            &Filter::default(),
            By::Account,
            By::Agent,
            By::Account,
            &rs,
            &names,
        );
        assert_eq!(m.breakup[0].1, "work");
        assert_eq!(m.breakup[0].2, 2.0);
        assert_eq!(m.breakup.len(), 3);
        assert_eq!(m.daily.len(), 3, "three days");
        let last = m.daily.last().unwrap();
        assert_eq!(last.2.iter().sum::<f64>(), 3.0);
        assert_eq!(m.cum.last().copied(), Some(3.5));
        assert_eq!(m.per_day["prompts"].iter().sum::<f64>(), 3.0);
        assert!(m.per_day["cache"].iter().all(|x| (0.0..=1.0).contains(x)));
        assert_eq!(m.tree.len(), 2, "claude and codex, each split by account");
        assert_eq!(m.tree[0].1.len(), 2);
        assert!(m.insights.iter().any(|(k, _)| k == "busiest day"));
        assert_eq!(
            m.model_mix.last().unwrap().1.iter().sum::<f64>().round(),
            1.0
        );
    }
}
