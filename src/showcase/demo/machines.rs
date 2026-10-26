//! What the demo world's machines say when they're looked at: ssh's view of
//! them, what's installed (agents, herdr, tools), their LLM servers, and
//! the doctor's checks — one of them a warning with a fix.

use super::world::{self, Machine};
use crate::doctor::{Fix, Line, Part, St};
use crate::probe::{Check, Found, HostProbe, IState, Integ, Tool};
use crate::services::{Scan, Service};
use crate::sshcfg::Resolved;
use serde_json::json;
use std::time::Duration;

/// An agent's version in the world.
fn version(agent: &str) -> &'static str {
    match agent {
        "claude" => "2.1.42",
        "codex" => "0.58.0",
        "opencode" => "1.0.62",
        "gemini" => "0.14.0",
        "pi" => "0.30.2",
        _ => "1.0.0",
    }
}

/// The agents on a machine: its sessions', and Claude Code everywhere.
fn agents_on(m: &Machine) -> Vec<&'static str> {
    let mut v = vec!["claude"];
    for b in world::BASE.iter().filter(|b| b.host == m.alias) {
        if b.agent != crate::agents::SHELL && !v.contains(&b.agent) {
            v.push(b.agent);
        }
    }
    v
}

pub fn resolve(host: &str) -> Option<Resolved> {
    let m = world::machine(host)?;
    Some(Resolved {
        hostname: m.alias.into(),
        user: m.user.into(),
        port: "22".into(),
        identities: vec!["~/.ssh/id_ed25519".into()],
        proxy: None,
    })
}

/// One integration's state on a machine (opencode's is a version behind on
/// europa: the doctor's warning).
fn integ_state(m: &Machine, name: &str) -> IState {
    if m.alias == "europa" && name == "opencode" {
        IState::Outdated {
            have: "v9".into(),
            want: "v11".into(),
        }
    } else {
        IState::Current("v11".into())
    }
}

pub fn probe(host: &str) -> HostProbe {
    let Some(m) = world::machine(host) else {
        return HostProbe {
            host: host.into(),
            error: Some(super::NOT_HERE.into()),
            ..Default::default()
        };
    };
    if !super::link_up(m) {
        return HostProbe {
            host: host.into(),
            error: Some(m.down_text()),
            ..Default::default()
        };
    }
    let home = m.home();
    let bin = format!("{home}/.local/bin");
    let mut integs = Vec::new();
    let agents: Vec<Found> = agents_on(m)
        .into_iter()
        .map(|id| {
            let a = crate::agents::find(id);
            let integration = a.and_then(|a| a.integration).map(|n| {
                let s = integ_state(m, n);
                integs.push(Integ {
                    name: n.into(),
                    state: s.clone(),
                    path: format!("{home}/.{n}/herdr-agent-state.sh"),
                    experimental: false,
                });
                s
            });
            Found {
                id: id.into(),
                path: format!("{bin}/{}", a.map_or(id, |a| a.exe())),
                version: version(id).into(),
                check: Check::Verified,
                on_path: Some(true),
                shadowed_by: None,
                integration,
            }
        })
        .collect();
    let path = vec![
        bin.clone(),
        "/usr/local/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
    ];
    let tool = |name: &str, version: &str| Tool {
        name: name.into(),
        path: format!("/usr/bin/{name}"),
        version: version.into(),
        prefix: if name == "npm" {
            format!("{home}/.npm-global")
        } else {
            String::new()
        },
        writable: (name == "npm").then_some(true),
    };
    HostProbe {
        host: host.into(),
        error: None,
        home: home.clone(),
        os: format!("{} {}", m.kernel, m.arch),
        os_name: m.distro.into(),
        gpus: m.gpus.iter().map(|g| g.0.to_string()).collect(),
        herdr: Some((format!("{bin}/herdr"), m.herdr.into())),
        agents,
        skipped: vec![],
        integs,
        tools: vec![
            tool("node", "v22.11.0"),
            tool("npm", "10.9.0"),
            tool("curl", ""),
            tool("bash", ""),
            tool("uv", "0.5.4"),
        ],
        login_path: path.clone(),
        shell_path: Some(path.clone()),
        searched: path,
        aliases: vec![],
        moshi: vec![],
        mac: None,
        sensors: None,
        took: Duration::from_millis(180 + world::hash(&[host, "took"]) % 400),
        age: None,
    }
}

/// The servers on a machine, as the probe would report them.
fn found(m: &Machine) -> Vec<serde_json::Value> {
    const G: f64 = 1024.0 * 1024.0 * 1024.0;
    let ollama = |models: &[(&str, &str, &str, f64, bool)]| {
        json!({
            "id": "ollama", "port": 11434, "ports": [11434], "addr": "127.0.0.1", "bind": "127.0.0.1",
            "pid": 1187, "process": "ollama serve", "version": "0.12.6", "health": "up",
            "found_by": ["process", "http"],
            "models": models.iter().map(|(n, p, q, gb, loaded)| json!({
                "name": n, "params": p, "quant": q, "size": gb * G, "format": "gguf", "kind": "llm",
                "ctx": 32768, "loaded": loaded,
                "vram": if *loaded { json!(gb * 1.15 * G) } else { json!(null) },
                "mem": if *loaded { json!(gb * 1.15 * G) } else { json!(null) },
            })).collect::<Vec<_>>(),
        })
    };
    match m.alias {
        "heart-of-gold" => vec![ollama(&[
            ("gpt-oss:20b", "20.9B", "MXFP4", 13.8, true),
            ("qwen3:8b", "8.2B", "Q4_K_M", 5.2, false),
            ("nomic-embed-text:latest", "137M", "F16", 0.27, false),
        ])],
        "magrathea" => vec![
            json!({
                "id": "vllm", "port": 8000, "ports": [8000], "addr": "127.0.0.1", "bind": "0.0.0.0",
                "pid": 4242, "process": "python3 -m vllm.entrypoints.openai.api_server", "version": "0.11.0",
                "health": "up", "found_by": ["process", "http"],
                "models": [{"name": "Qwen/Qwen3-Coder-30B-A3B-Instruct", "params": "30.5B", "quant": "FP8",
                            "format": "safetensors", "kind": "llm", "ctx": 131072, "loaded": true,
                            "vram": 41.0 * G}],
                "vram": 2.0 * 41.0 * G,
                "requests_running": 3, "requests_waiting": 1, "kv_cache": 0.41,
            }),
            ollama(&[
                ("qwen2.5-coder:32b", "32.8B", "Q4_K_M", 19.9, true),
                ("llama3.1:8b", "8.0B", "Q4_K_M", 4.9, false),
            ]),
        ],
        "europa" => vec![json!({
            "id": "litellm", "port": 4000, "ports": [4000], "addr": "127.0.0.1", "bind": "0.0.0.0",
            "pid": 2210, "process": "litellm --config /etc/litellm/config.yaml", "version": "1.79.1",
            "health": "up", "found_by": ["process", "http"],
            "models": [
                {"name": "claude-sonnet-4-5", "kind": "route"},
                {"name": "gpt-5.1", "kind": "route"},
                {"name": "magrathea/qwen3-coder", "kind": "route"},
                {"name": "heart-of-gold/gpt-oss", "kind": "route"},
            ],
        })],
        "tycho-base" => vec![json!({
            "id": "llamacpp", "port": 8080, "ports": [8080], "addr": "127.0.0.1", "bind": "127.0.0.1",
            "pid": 3301, "process": "llama-server -m models/devstral-small-2507-Q5_K_M.gguf", "version": "b6799",
            "health": "up", "found_by": ["process", "http"],
            "models": [{"name": "devstral-small-2507-Q5_K_M.gguf", "params": "23.6B", "quant": "Q5_K_M",
                        "format": "gguf", "kind": "llm", "ctx": 65536, "loaded": true, "vram": 18.2 * G}],
            "requests_running": 1, "requests_waiting": 0, "kv_cache": 0.18,
        })],
        _ => vec![],
    }
}

pub fn services(alias: &str, on: &[&'static Service]) -> Scan {
    let now = super::now();
    let Some(m) = world::machine(alias) else {
        return Scan {
            err: Some(super::NOT_HERE.into()),
            ..Scan::new(alias)
        };
    };
    if !super::link_up(m) {
        return Scan {
            err: Some(m.down_text()),
            took_ms: 8000,
            ..Scan::new(alias)
        };
    }
    let found = found(m)
        .iter()
        .filter_map(crate::services::Found::from_json)
        .filter(|f| on.iter().any(|s| s.id == f.service.id))
        .collect();
    Scan {
        alias: alias.into(),
        found,
        at: now,
        took_ms: 400 + world::hash(&[alias, "svc"]) % 900,
        listeners: "ss".into(),
        http: "curl".into(),
        ..Default::default()
    }
}

// ------------------------------------------------------------------ the doctor

fn line(st: St, label: &str, text: impl Into<String>) -> Line {
    Line {
        st,
        label: label.into(),
        text: text.into(),
        fix: vec![],
        action: None,
    }
}

fn head(t: &str) -> Line {
    line(St::Head, t, "")
}

pub fn doctor(p: &Part) -> Vec<Line> {
    match p {
        Part::Local => vec![
            head("This Mac"),
            line(
                St::Ok,
                "aleph",
                format!("{}  ·  ~/.cargo/bin/aleph", env!("CARGO_PKG_VERSION")),
            ),
            line(St::Ok, "zsh", "al → aleph  ·  ~/.zshrc"),
            line(St::Ok, "herdr", "0.9.5  ·  ~/.local/bin/herdr"),
            line(
                St::Ok,
                "hub",
                "runs here (auto-approve, queue, notifications)",
            ),
            line(St::Ok, "ssh", "OpenSSH_10.0p2, LibreSSL 3.3.6"),
        ],
        Part::Daemon => vec![
            head("Daemon"),
            line(St::Ok, "service", "running (launchd) · since this morning"),
            line(St::Ok, "socket", "answers in 2 ms"),
            line(St::Ok, "api", "http://127.0.0.1:7878 · loopback only"),
        ],
        Part::Host(t) => host(&t.ssh),
        Part::NoHost => vec![],
        Part::Shortcuts => vec![
            head("Shortcuts"),
            line(St::Ok, "Raycast", "Aleph → all machines"),
            line(St::Ok, "sidebar", "herdr colours sessions by glyph"),
        ],
    }
}

fn host(alias: &str) -> Vec<Line> {
    let Some(m) = world::machine(alias) else {
        return vec![
            head(&format!("Host · {alias}")),
            line(St::Bad, "machine", super::NOT_HERE),
        ];
    };
    let mut v = vec![
        head(&format!("Host · {alias}")),
        line(
            St::Ok,
            "default",
            format!("{alias}  ·  from the demo world"),
        ),
        line(St::Ok, "ssh alias", "defined in ~/.ssh/config"),
        line(St::Ok, "connects to", format!("{}@{}", m.user, m.alias)),
    ];
    if !super::link_up(m) {
        v.push(Line {
            fix: vec![
                "check that it's on and on the network (it comes back by itself here)".into(),
            ],
            ..line(
                St::Bad,
                "reachable",
                format!("no answer — {}", m.down_text()),
            )
        });
        return v;
    }
    v.push(line(St::Ok, "auth methods", "publickey"));
    v.push(line(St::Ok, "key login", "works — no password prompts"));
    v.push(head(&format!("On {alias}")));
    v.push(line(
        St::Ok,
        "herdr",
        format!("{}  ·  ~/.local/bin/herdr", m.herdr),
    ));
    for a in agents_on(m) {
        v.push(line(
            St::Ok,
            &crate::agents::label(a),
            format!("{}  ·  ~/.local/bin/{a}", version(a)),
        ));
        if let Some(n) = crate::agents::find(a).and_then(|x| x.integration) {
            if let IState::Outdated { have, want } = integ_state(m, n) {
                v.push(Line {
                    fix: vec![format!("herdr integration install {n}   (on {alias})")],
                    action: Some(Fix::Integration {
                        host: alias.into(),
                        name: n.into(),
                    }),
                    ..line(
                        St::Warn,
                        &format!("{n} integration"),
                        format!("outdated ({have} < {want}) — sessions may not come back after a herdr restart"),
                    )
                });
            }
        }
    }
    v.push(Line {
        st: St::Info,
        ..line(
            St::Info,
            "system",
            format!("{} · {} · {} cores", m.distro, m.cpu, m.ncpu),
        )
    });
    v.push(head(&format!("herdr · {alias}")));
    v.push(line(
        St::Ok,
        "machine",
        format!("'{alias}' is saved in herdr"),
    ));
    let n = super::world().on(alias).count();
    v.push(line(St::Ok, "session", format!("answers · {n} sessions")));
    v
}
