//! Token usage in the demo world: what the agents' logs would hold, made up
//! hour by hour — busier in working hours, quiet at night — as the
//! extractor's own output (`extract::Output`), for the records since the
//! machine's last good read (at most 30 days back). The normal store,
//! rollup, prices and limits code takes it from there.

use super::world::{self, Machine};
use crate::usage::extract::{AgentSeen, FileRun, Output, RootAccount};
use crate::usage::parse::{Limit, Record, Window};
use crate::usage::steps::{Kind, Step};
use std::collections::HashMap;

/// How far back a first read goes.
pub const BACK: u64 = 30 * 86_400;

fn model(agent: &str, h: u64) -> &'static str {
    match agent {
        "claude" if h % 10 < 3 => "claude-opus-4-5",
        "claude" | "opencode" | "pi" => "claude-sonnet-4-5",
        "codex" => "gpt-5.1-codex",
        "gemini" => "gemini-2.5-pro",
        _ => "claude-sonnet-4-5",
    }
}

/// How likely an hour is worked (by the machine's local hour).
fn busy(lh: u64) -> u64 {
    match lh {
        0..=6 => 80,
        7..=8 => 450,
        9..=12 => 850,
        13 => 600,
        14..=18 => 880,
        19..=21 => 500,
        _ => 220,
    }
}

/// The agents' log files on a machine: (agent, folder, session id, file).
fn logs(m: &Machine) -> Vec<(&'static str, &'static str, String, String)> {
    let home = m.home();
    world::BASE
        .iter()
        .filter(|b| b.host == m.alias && b.agent != crate::agents::SHELL)
        .map(|b| {
            // the conversation in its pane
            let id = world::conversation(b.host, &format!("w{}:p{}", b.ws, b.n));
            let path = match b.agent {
                // a second login on some machines: the work account's own config folder
                "claude" => format!(
                    "{}/{}/{id}.jsonl",
                    claude_root(m, b.dir),
                    format!("{home}/{}", b.dir).replace('/', "-")
                ),
                "codex" => format!("{home}/.codex/sessions/2026/10/rollout-{id}.jsonl"),
                "gemini" => format!("{home}/.gemini/tmp/{id}/chats/session-{id}.json"),
                "pi" => format!("{home}/.pi/agent/sessions/{id}.jsonl"),
                _ => format!("{home}/.local/share/opencode/storage/session/{id}.json"),
            };
            (b.agent, b.dir, id, path)
        })
        .collect()
}

/// A Claude session's config folder's logs: a work login (`~/.claude-work`)
/// for some folders on every other machine, else the default one.
fn claude_root(m: &Machine, dir: &str) -> String {
    let work = world::hash(&[m.alias, "two logins"]).is_multiple_of(2)
        && world::hash(&[dir, "login"]).is_multiple_of(3);
    format!(
        "{}/{}/projects",
        m.home(),
        if work { ".claude-work" } else { ".claude" }
    )
}

/// Whose a Claude config folder is: (its id, email, plan tier).
fn claude_account(m: &Machine, root: &str) -> (String, String, &'static str) {
    if root.contains("/.claude-work/") {
        (
            format!("demo-{}-work", m.user),
            format!("{}@work.example", m.user),
            "default_claude_max_20x",
        )
    } else {
        (
            format!("demo-{}", m.user),
            format!("{}@home.example", m.user),
            "default_claude_max_5x",
        )
    }
}

/// The tools an agent calls: (name, class), the usual ones first.
fn tools(agent: &str) -> &'static [(&'static str, &'static str)] {
    match agent {
        "codex" => &[
            ("shell", "tool"),
            ("apply_patch", "tool"),
            ("read_file", "tool"),
            ("shell", "tool"),
            ("web.run", "web"),
        ],
        "gemini" => &[
            ("read_file", "tool"),
            ("replace", "tool"),
            ("run_shell_command", "tool"),
            ("glob", "tool"),
            ("google_web_search", "web"),
        ],
        "opencode" | "pi" => &[
            ("read", "tool"),
            ("edit", "tool"),
            ("bash", "tool"),
            ("grep", "tool"),
            ("webfetch", "web"),
        ],
        _ => &[
            ("Read", "tool"),
            ("Edit", "tool"),
            ("Bash", "tool"),
            ("Grep", "tool"),
            ("WebSearch", "web"),
        ],
    }
}

fn key(parts: &[&str]) -> String {
    format!("{:016x}", world::hash(parts))
}

/// A step of the demo world's session `id` on `m`.
fn step(m: &Machine, agent: &str, id: &str, kind: Kind, ms: u64, k: String) -> Step {
    Step {
        kind,
        ms,
        host: m.alias.into(),
        agent: agent.into(),
        session: id.into(),
        lane: String::new(),
        key: k,
        name: String::new(),
        class: String::new(),
        n1: 0,
        n2: 0,
    }
}

/// What one request did after it: a few tool calls (now and then a run of
/// reads, a failing command, a web search), starting at `ms`.
fn calls(m: &Machine, agent: &str, id: &str, req: &str, r: u64, ms: u64, lane: &str) -> Vec<Step> {
    let list = tools(agent);
    let mut out = Vec::new();
    let reads = (r >> 40).is_multiple_of(7);
    let n = if reads {
        4 + (r >> 44) % 5
    } else {
        (r >> 47) % 4
    };
    let mut at = ms + 400;
    for i in 0..n {
        let x = world::hash(&[req, "call", &i.to_string()]);
        let (name, class) = if reads {
            list[0]
        } else if x.is_multiple_of(31) {
            list[4]
        } else {
            list[(x >> 5) as usize % 4]
        };
        let took = 150 + (x >> 9) % if class == "web" { 4_000 } else { 2_500 };
        let k = key(&[req, "call", &i.to_string(), "k"]);
        let mut s = step(m, agent, id, Kind::ToolStart, at, k.clone());
        s.lane = lane.into();
        s.name = name.into();
        s.class = class.into();
        out.push(s);
        let mut e = step(m, agent, id, Kind::ToolEnd, at + took, k);
        e.lane = lane.into();
        e.name = if x.is_multiple_of(23) && class == "tool" {
            "err"
        } else {
            "ok"
        }
        .into();
        e.n1 = took;
        out.push(e);
        at += took + 120;
    }
    out
}

/// The records of one session's log in (from, to], and its steps: a prompt
/// each worked hour, the tool calls after each request, now and then (Claude)
/// a subagent of its own.
fn records(
    m: &Machine,
    agent: &str,
    dir: &str,
    id: &str,
    from: u64,
    to: u64,
) -> (Vec<Record>, Vec<Step>) {
    let mut out = Vec::new();
    let mut steps = Vec::new();
    // its machine's working day: the hours shift a little per machine
    let tz = world::hash(&[m.alias, "tz"]) % 5;
    let (h0, h1) = (from / 3600, to / 3600);
    for hr in h0..=h1 {
        let hs = hr.to_string();
        let h = world::hash(&[m.alias, id, &hs]);
        let lh = (hr + 24 - 2 + tz) % 24;
        if h % 1000 >= busy(lh) {
            continue;
        }
        let n = 1 + (h >> 10) % 5;
        // the hour's prompt: a little before its first request
        let first = (0..n)
            .map(|j| hr * 3600 + world::hash(&[id, &hs, &j.to_string()]) % 3600)
            .min()
            .unwrap_or(hr * 3600);
        let pms = first.saturating_sub(8 + h % 40) * 1000 + h % 1000;
        if pms / 1000 > from && pms / 1000 <= to {
            let mut p = step(
                m,
                agent,
                id,
                Kind::Prompt,
                pms,
                key(&[m.alias, id, &hs, "prompt"]),
            );
            p.name = "u".into();
            steps.push(p);
        }
        // a subagent this hour (Claude, now and then): the last request is its
        let sub = agent == "claude" && n >= 3 && (h >> 30).is_multiple_of(4);
        let lane = if sub {
            key(&[m.alias, id, &hs, "lane"])
        } else {
            String::new()
        };
        let mut last = 0;
        for j in 0..n {
            let r = world::hash(&[id, &hs, &j.to_string()]);
            let ts = hr * 3600 + r % 3600;
            last = last.max(ts);
            if ts <= from || ts > to {
                continue;
            }
            let req = key(&[m.alias, id, &ts.to_string(), &j.to_string()]);
            let theirs = sub && j == n - 1;
            if theirs {
                // the call that starts it, its link, its prompt
                let call = key(&[&lane, "call"]);
                let mut s = step(
                    m,
                    agent,
                    id,
                    Kind::ToolStart,
                    ts * 1000 - 2_000,
                    call.clone(),
                );
                s.name = "Agent".into();
                s.class = "subagent".into();
                steps.push(s);
                let mut e = step(
                    m,
                    agent,
                    id,
                    Kind::ToolEnd,
                    ts * 1000 + 40_000,
                    call.clone(),
                );
                e.name = "ok".into();
                e.n1 = 42_000;
                steps.push(e);
                let mut l = step(
                    m,
                    agent,
                    id,
                    Kind::Lane,
                    ts * 1000 - 1_900,
                    key(&[&lane, "link"]),
                );
                l.lane = lane.clone();
                l.name = "Explore".into();
                l.class = call;
                steps.push(l);
                let mut p = step(
                    m,
                    agent,
                    id,
                    Kind::Prompt,
                    ts * 1000 - 1_800,
                    key(&[&lane, "prompt"]),
                );
                p.lane = lane.clone();
                p.name = "a".into();
                steps.push(p);
            }
            steps.extend(calls(
                m,
                agent,
                id,
                &req,
                r,
                ts * 1000,
                if theirs { &lane } else { "" },
            ));
            let mdl = model(agent, r >> 7);
            let opus = mdl.contains("opus");
            let input = 800 + (r >> 13) % 14_000;
            let output = 200 + (r >> 23) % 3_800;
            out.push(Record {
                ts,
                host: m.alias.into(),
                agent: agent.into(),
                session: id.into(),
                key: format!(
                    "{:016x}",
                    world::hash(&[m.alias, id, &ts.to_string(), &j.to_string()])
                ),
                model: mdl.into(),
                effort: if agent == "codex" {
                    "high".into()
                } else {
                    String::new()
                },
                project: format!("~/{dir}"),
                input,
                output: if opus { output / 2 } else { output },
                cache_read: if agent == "gemini" {
                    0
                } else {
                    10_000 + (r >> 33) % 160_000
                },
                cache_write_5m: if agent == "claude" {
                    500 + (r >> 41) % 9_000
                } else {
                    0
                },
                cache_write_1h: 0,
                reasoning: if agent == "codex" { output / 3 } else { 0 },
                cost: None,
                flags: String::new(),
                web: 0,
                turn_ms: 0,
                lane: if theirs { lane.clone() } else { String::new() },
                ..Default::default()
            });
        }
        // the turn that hour: how long it worked on the prompt
        let tts = last + 30;
        if tts > from && tts <= to {
            out.push(Record {
                ts: tts,
                host: m.alias.into(),
                agent: agent.into(),
                session: id.into(),
                key: format!("{:016x}", world::hash(&[m.alias, id, &hs, "turn"])),
                model: model(agent, h >> 7).into(),
                project: format!("~/{dir}"),
                turn_ms: (6 + (h >> 20) % 20) * 60_000,
                ..Default::default()
            });
        }
    }
    out.sort_by_key(|r| r.ts);
    steps.sort_by_key(|s| s.ms);
    (out, steps)
}

/// The words of a demo session's timeline: its prompts' first lines and its
/// tools' targets, made up from its steps (as `steps::words` reads them from
/// a real log).
pub fn words(steps: &[Step]) -> HashMap<String, String> {
    const PROMPTS: [&str; 8] = [
        "fix the flaky perf test",
        "add a retry to the upload client",
        "why does the build take 4 minutes now",
        "write tests for the parser edge cases",
        "rename Session::color to hue everywhere",
        "profile the hot loop and make it faster",
        "update the docs for the new flags",
        "review the last commit for bugs",
    ];
    const FILES: [&str; 6] = [
        "src/main.rs",
        "src/client.rs",
        "tests/parse.rs",
        "README.md",
        "src/store/mod.rs",
        "Cargo.toml",
    ];
    const CMDS: [&str; 5] = [
        "cargo test -q",
        "git status --short",
        "cargo build --release",
        "rg -n retry src",
        "npm run lint",
    ];
    let mut w = HashMap::new();
    for s in steps {
        let x = world::hash(&[&s.key, "words"]) as usize;
        let text = match (s.kind, s.name.as_str()) {
            (Kind::Prompt, "a") => "map the modules that touch the store".to_string(),
            (Kind::Prompt, _) => PROMPTS[x % PROMPTS.len()].to_string(),
            (Kind::ToolStart, n) if s.class == "web" => {
                let _ = n;
                "rust tokio retry backoff".to_string()
            }
            (Kind::ToolStart, n) if n.contains("hell") || n.eq_ignore_ascii_case("bash") => {
                CMDS[x % CMDS.len()].to_string()
            }
            (Kind::ToolStart, "Agent") => "Explore: map the store".to_string(),
            (Kind::ToolStart, _) => FILES[x % FILES.len()].to_string(),
            _ => continue,
        };
        w.insert(s.key.clone(), text);
    }
    w
}

/// Claude's plan windows on a machine now (5 hours, a week).
fn claude_limit(m: &Machine, now: u64) -> Limit {
    let five = now - now % 18_000 + 18_000;
    let week = now - now % 604_800 + 604_800;
    let used5 = 100.0 * (1.0 - (five - now) as f64 / 18_000.0) * 0.8;
    let usedw = 100.0 * (1.0 - (week - now) as f64 / 604_800.0) * 0.7 + 5.0;
    Limit {
        ts: now,
        id: "claude".into(),
        plan: "Max 5x".into(),
        windows: vec![
            Window {
                used_percent: used5.round(),
                minutes: 300,
                resets_at: Some(five),
            },
            Window {
                used_percent: usedw.round(),
                minutes: 10_080,
                resets_at: Some(week),
            },
        ],
        source: "cache".into(),
        account: crate::usage::parse::hash(&["claude", &format!("demo-{}", m.user)]),
    }
}

/// One machine's run of the extractor: the records since its last good read.
pub fn run(host: &str) -> Result<Output, String> {
    let m = world::machine(host).ok_or_else(|| super::NOT_HERE.to_string())?;
    let now = super::now();
    if !super::link_up(m) {
        return Err(m.down_text());
    }
    let st = crate::usage::store::Offsets::load();
    let known = st.hosts.get(host);
    let since = known.map_or(0, |h| h.ok_at);
    let from = since.max(now.saturating_sub(BACK));
    let mut out = Output {
        hostname: m.alias.into(),
        home: m.home(),
        now,
        has_zstd: true,
        has_sqlite: true,
        listing_done: true,
        ..Default::default()
    };
    for (agent, dir, id, path) in logs(m) {
        let (mut recs, steps) = records(m, agent, dir, &id, from, now);
        // whose: the log's own account (Codex names it; a Claude folder's is in its settings)
        let mut accts = std::collections::BTreeMap::new();
        let root = match agent {
            "claude" => claude_root(m, dir),
            _ => path.rsplit_once('/').map_or("", |x| x.0).to_string(),
        };
        let raw = match agent {
            "claude" => Some(claude_account(m, &format!("{root}/")).0),
            "codex" => Some("demo-account".to_string()),
            _ => None,
        };
        if let Some(raw) = raw {
            let acct = crate::usage::parse::acct_id(agent, host, &raw);
            for r in recs.iter_mut().filter(|r| !r.is_turn()) {
                r.acct = acct.clone();
            }
            accts.insert(acct, raw);
        }
        if agent == "claude" && !out.accounts.iter().any(|a| a.root == root) {
            let (id, email, tier) = claude_account(m, &format!("{root}/"));
            out.accounts.push(RootAccount {
                agent: "claude".into(),
                root: root.clone(),
                id,
                email,
                billing: "stripe_subscription".into(),
                tier: tier.into(),
                ..Default::default()
            });
        }
        let start = known.and_then(|h| h.files.get(&path)).map_or(0, |f| f.off);
        // the log grows by what it holds since
        let size = start.max(4096) + recs.len() as u64 * 900;
        let limits = if agent == "claude" {
            vec![claude_limit(m, now)]
        } else {
            vec![]
        };
        let seen = out.agents.iter_mut().find(|a| a.agent == agent);
        match seen {
            Some(a) => {
                a.listed += 1;
                a.to_read += 1;
                if !a.roots.contains(&root) {
                    a.roots.push(root.clone());
                }
            }
            None => out.agents.push(AgentSeen {
                agent: agent.into(),
                roots: vec![root.clone()],
                listed: 1,
                unchanged: 0,
                to_read: 1,
                zst_skipped: 0,
            }),
        }
        out.bytes_read += recs.len() as u64 * 900;
        out.files.push(FileRun {
            agent: agent.into(),
            reader: agent.into(),
            path,
            dev: 64_768,
            ino: world::hash(&[&id, "ino"]) % 10_000_000,
            size,
            mtime: now,
            start,
            next: Some(size),
            ctx: String::new(),
            lines: recs.len() as u64,
            records: recs,
            limits,
            accts,
            steps,
        });
    }
    Ok(out)
}

/// Codex's plan windows, as its app server says them: the 5-hour window
/// fills, and runs out 4 minutes of every 20 (the Codex session there
/// stops at its limit then) — or, while the story plays, at its limit beat.
/// The cycle runs on the world's clock; the times it gives are real ones.
pub fn codex(host: &str) -> Option<Limit> {
    let m = world::machine(host)?;
    let now = super::now();
    if !super::link_up(m) || !logs(m).iter().any(|l| l.0 == "codex") {
        return None;
    }
    let t = super::clock();
    let (used, left) = match super::story::current() {
        Some(s) if s.has(super::story::LIMIT) && host == super::story::LIMITED.0 => {
            super::story::window(&s, t)
        }
        _ => {
            let p = t % world::LIMIT_EVERY;
            if p < world::LIMIT_FOR {
                (100.0, world::LIMIT_FOR - p)
            } else {
                (
                    (100.0 * (p - world::LIMIT_FOR) as f64
                        / (world::LIMIT_EVERY - world::LIMIT_FOR) as f64)
                        .min(99.0)
                        .round(),
                    world::LIMIT_EVERY - p + world::LIMIT_FOR,
                )
            }
        }
    };
    Some(Limit {
        ts: now,
        id: "codex".into(),
        plan: "plus".into(),
        windows: vec![
            Window {
                used_percent: used,
                minutes: 300,
                resets_at: Some(super::clock::real_at(t + left)),
            },
            Window {
                used_percent: 38.0,
                minutes: 10_080,
                resets_at: Some(now - now % 604_800 + 604_800),
            },
        ],
        source: "live".into(),
        account: crate::usage::parse::hash(&["codex", "demo-account"]),
    })
}

/// Which repo each folder is in: every folder its own repo, the same
/// folder on two machines one repo (`github.com/demo/<folder>`).
pub fn projects(host: &str, folders: &[String]) -> Vec<(String, crate::usage::projects::Folder)> {
    let now = super::now();
    folders
        .iter()
        .map(|f| {
            let name = f
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("x")
                .to_string();
            (
                f.clone(),
                crate::usage::projects::Folder {
                    top: f.clone(),
                    common: format!("{f}/.git"),
                    remote: format!("git@github.com:demo/{name}.git"),
                    root: format!("{:040x}", world::hash(&[&name, "root"]) as u128),
                    at: now,
                    gone: false,
                },
            )
        })
        .filter(|_| !host.is_empty())
        .collect()
}
