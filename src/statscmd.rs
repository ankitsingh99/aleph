//! `aleph stats` (machine stats across the fleet, one sample) and `aleph peek`
//! (a session's recent output) — for scripts, the Raycast extension and humans.

use crate::cli::{self, Args};
use crate::out::{err, fail, hint, json, out, warn, Spinner};
use crate::telemetry::{Stats, Telemetry};
use crate::ui::{self, c};
use crate::{fleet, herdr, theme};
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// One sample per host, all hosts in parallel (CPU needs two readings).
pub fn sample(hosts: &[String], timeout: Duration) -> Vec<(String, Stats)> {
    let tele: Vec<Telemetry> = hosts.iter().map(|h| Telemetry::start(h, 1)).collect();
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        let done = tele.iter().all(|t| {
            let s = t.snapshot();
            s.samples >= 2 || (s.err.is_some() && s.samples == 0)
        });
        if done {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tele.iter()
        .map(|t| (t.alias.clone(), t.snapshot()))
        .collect()
}

fn gpu_json(g: &crate::telemetry::Gpu) -> Value {
    let mut m = serde_json::Map::new();
    let mut put = |k: &str, v: Value| {
        m.insert(k.to_string(), v);
    };
    put("index", json!(g.index));
    put("name", json!(g.name));
    put("vendor", json!(g.vendor));
    put("util", json!(g.util));
    put("mem_used", json!(g.mem_used));
    put("mem_total", json!(g.mem_total));
    put("mem_util", json!(g.mem_util));
    put("temp", json!(g.temp));
    put("temp_mem", json!(g.temp_mem));
    put("power", json!(g.power));
    put("power_limit", json!(g.power_limit));
    put("fan", json!(g.fan));
    put("clock_sm", json!(g.clock_sm));
    put("clock_mem", json!(g.clock_mem));
    put("clock_sm_max", json!(g.clock_sm_max));
    put("pstate", json!(g.pstate));
    put("pcie", json!(g.pcie));
    put("enc", json!(g.enc));
    put("dec", json!(g.dec));
    put("throttle", json!(g.throttle));
    put("ecc", json!(g.ecc));
    // an Apple GPU's own (the I/O Registry's)
    if g.vendor == "apple" || g.renderer.is_some() {
        put("renderer", json!(g.renderer));
        put("tiler", json!(g.tiler));
        put("cores", json!(g.cores));
        put("mem_alloc", json!(g.mem_alloc));
    }
    Value::Object(m)
}

pub fn stats_json(alias: &str, color: &str, s: &Stats) -> Value {
    let reachable = s.samples > 0;
    let mut v = json!({
        "alias": alias,
        "color": color,
        "reachable": reachable,
        "error": if reachable { Value::Null } else { json!(s.err.clone().unwrap_or_else(|| "no answer".into())) },
        "hostname": s.hostname,
        "os": s.os,
        "kernel": s.kernel,
        "cpu_model": s.cpu_model,
        "ncpu": s.ncpu,
        "sockets": s.sockets,
        "uptime_s": s.uptime,
        "load": s.load,
        "cpu": s.cpu,
        "cores": s.cores,
        "mem_used": s.mem_used,
        "mem_total": s.mem_total,
        "swap_used": s.swap_used,
        "swap_total": s.swap_total,
        "net_rx": s.net_rx,
        "net_tx": s.net_tx,
        "disks": s.disks.iter().map(|(m, t, u)| json!({"mount": m, "used": u, "total": t})).collect::<Vec<_>>(),
        "gpus": s.gpus.iter().map(gpu_json).collect::<Vec<_>>(),
        "gpu_procs": s.gpu_procs.iter().map(|p| json!({"gpu": p.gpu, "pid": p.pid, "mem": p.mem, "user": p.user, "name": p.name})).collect::<Vec<_>>(),
        "top": s.top.iter().map(|p| json!({"pid": p.pid, "user": p.user, "cpu": p.cpu, "rss": p.rss, "name": p.name})).collect::<Vec<_>>(),
        "claude_procs": s.claude_procs,
        "claude_rss": s.claude_rss,
        "convos_today": s.convos_today,
    });
    v["agent_procs"] = json!(s
        .agent_procs
        .iter()
        .map(|(a, n, r)| json!({"agent": a, "count": n, "rss": r}))
        .collect::<Vec<_>>());
    v["sessions_today"] = json!(s
        .sessions_today
        .iter()
        .map(|(a, n)| json!({"agent": a, "count": n}))
        .collect::<Vec<_>>());
    v["platform"] = json!(s.platform);
    v["cpu_split"] = json!({"user": s.cpu_user, "system": s.cpu_system, "iowait": s.cpu_iowait, "steal": s.cpu_steal});
    v["cpu_temp"] = json!(s.cpu_temp);
    v["temps"] = json!(s
        .temps
        .iter()
        .map(|(l, t)| json!({"label": l, "c": t}))
        .collect::<Vec<_>>());
    v["mem_buffers"] = json!(s.mem_buffers);
    v["mem_cached"] = json!(s.mem_cached);
    v["mem_shared"] = json!(s.mem_shared);
    v["mem_available"] = json!(s.mem_available);
    v["procs_total"] = json!(s.procs_total);
    v["procs_running"] = json!(s.procs_running);
    v["threads_total"] = json!(s.threads_total);
    v["users_logged"] = json!(s.users_logged);
    v["nets"] = json!(s
        .nets
        .iter()
        .map(|n| json!({"name": n.name, "rx": n.rx, "tx": n.tx}))
        .collect::<Vec<_>>());
    v["disk_io"] = json!(s
        .disk_io
        .iter()
        .map(|x| json!({"name": x.name, "read": x.read, "write": x.write, "busy": x.busy}))
        .collect::<Vec<_>>());
    v["gpu_note"] = json!(s.gpu_note);
    v
}

fn pct(f: f64) -> Span<'static> {
    Span::styled(format!("{:>4.0}%", f * 100.0), ui::fg(ui::heat(f)))
}

pub fn stats(a: Args) -> ExitCode {
    let hosts: Vec<String> = match &a.flag {
        Some(h) => vec![h.clone()],
        None => a.cfg.fleet(),
    };
    if hosts.is_empty() {
        return cli::no_host();
    }
    let sp = Spinner::start(format!(
        "Sampling {} machine{}",
        hosts.len(),
        if hosts.len() == 1 { "" } else { "s" }
    ));
    let res = sample(&hosts, Duration::from_secs(12));
    sp.stop();
    if a.has("--json") {
        json(&json!({
            "generated_at": crate::cache::now(),
            "hosts": res.iter().map(|(h, s)| stats_json(h, &theme::machine_hex(&a.cfg, h), s)).collect::<Vec<_>>(),
        }));
        return ExitCode::SUCCESS;
    }
    if !std::io::stdout().is_terminal() {
        println!("host\treachable\tcpu\tgpu\tmem_used\tmem_total\tload1\tagent_procs");
        for (h, s) in &res {
            crate::out::raw(&format!(
                "{h}\t{}\t{:.3}\t{}\t{:.0}\t{:.0}\t{:.2}\t{}",
                s.samples > 0,
                s.cpu,
                s.gpu_util().map(|g| format!("{g:.3}")).unwrap_or_default(),
                s.mem_used,
                s.mem_total,
                s.load[0],
                s.agent_procs.iter().map(|a| a.1).sum::<usize>()
            ));
        }
        return ExitCode::SUCCESS;
    }
    let aw = res
        .iter()
        .map(|(h, _)| ui::width(h))
        .max()
        .unwrap_or(4)
        .max(7);
    out(Line::from(Span::styled(
        format!(
            "   {}  {:>5}  {:>5}  {:>5}  {:>17}  {:>5}  agents",
            ui::pad("machine", aw),
            "cpu",
            "gpu",
            "mem",
            "memory",
            "load"
        ),
        ui::faint(),
    )));
    let mut bad = 0;
    for (h, s) in &res {
        let col = theme::machine(&a.cfg, h);
        let os = format!("{} {}", s.platform, s.os);
        let mut l = vec![
            Span::styled("▌", ui::fg(col)),
            if s.samples > 0 && crate::icons::style() == crate::icons::Style_::Nerd {
                crate::logo::glyph(crate::logo::Kind::Os, os.trim())
            } else {
                Span::raw(" ")
            },
            Span::raw(" "),
            Span::styled(
                ui::pad(h, aw),
                Style::new().fg(col).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
        ];
        if s.samples == 0 {
            bad += 1;
            l.push(Span::styled(
                format!("✕ {}", s.err.clone().unwrap_or_else(|| "no answer".into())),
                ui::fg(c::RED()),
            ));
            out(Line::from(l));
            continue;
        }
        l.push(pct(s.cpu));
        l.push(Span::raw("  "));
        l.push(match s.gpu_util() {
            Some(g) => pct(g),
            None => Span::styled("    –", ui::faint()),
        });
        l.push(Span::raw("  "));
        l.push(pct(s.mem_frac()));
        l.push(Span::styled(
            format!(
                "  {:>17}",
                format!("{}/{}", ui::bytes(s.mem_used), ui::bytes(s.mem_total))
            ),
            ui::muted(),
        ));
        l.push(Span::raw(format!("  {:>5.2}", s.load[0])));
        l.push(Span::styled(
            format!(
                "  {} · {}",
                if s.agent_procs.is_empty() {
                    "none".to_string()
                } else {
                    s.agent_procs
                        .iter()
                        .map(|(a, n, _)| format!("{a} {n}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                },
                ui::bytes(s.agent_procs.iter().map(|a| a.2).sum::<f64>())
            ),
            ui::fg(c::ACCENT()),
        ));
        out(Line::from(l));
        for g in &s.gpus {
            out(Line::from(vec![
                Span::raw(format!("   {}  ", " ".repeat(aw))),
                crate::icons::gpu(&g.vendor),
                Span::styled(format!(" gpu{} ", g.index), ui::faint()),
                pct(g.util),
                Span::styled(
                    format!(
                        "  {}/{}  {}  {}",
                        ui::bytes(g.mem_used),
                        ui::bytes(g.mem_total),
                        g.temp.map(|t| format!("{t:.0}°C")).unwrap_or_default(),
                        g.name
                    ),
                    ui::muted(),
                ),
            ]));
        }
    }
    if bad > 0 {
        hint("aleph doctor explains unreachable machines");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// `aleph approve <name|host:name> [--json]` — answer the agent's question
/// with its "yes" option, whatever its position.
pub fn approve(a: Args) -> ExitCode {
    let p = a.positional();
    let Some(q) = p.first() else {
        return fail("usage: aleph approve <name | host:name> [--json]");
    };
    let (flag, name) = match q.split_once(':') {
        Some((h, n)) if !h.is_empty() => (Some(h.to_string()), n.to_string()),
        _ => (a.flag.clone(), q.to_string()),
    };
    let (t, s) = match cli::find(&a.cfg, &flag, &name) {
        Ok(x) => x,
        Err(c) => return c,
    };
    if !s.blocked() {
        err(&format!(
            "{} isn't asking anything right now ({})",
            s.display(),
            s.status
        ));
        return ExitCode::from(1);
    }
    match herdr::Herdr::new(&t).approve(&s.pane_id) {
        Ok(k) => {
            if a.has("--json") {
                json(&json!({"host": t.ssh, "name": s.name, "keys": k}));
            } else {
                crate::out::ok(&format!(
                    "approved {} on {} ({})",
                    s.display(),
                    t.ssh,
                    k.join(" ")
                ));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            err(&format!("{}: {e}", s.display()));
            ExitCode::from(1)
        }
    }
}

/// `aleph peek <name|host:name> [-n LINES] [--ansi] [--json]` — recent output, no attach.
/// A session's screen as `aleph peek --json` gives it (and the API's
/// `screen.read {ansi}`): the session, its last `lines`, its question and
/// the keys that answer yes, a usage limit seen on screen. `ansi` keeps the
/// colours, and keeps a read that's repeated (Raycast's live preview)
/// passive: a text read past the screen scrolls an idle agent.
pub fn peek_json(
    h: &herdr::Herdr,
    mut s: crate::herdr::Session,
    lines: usize,
    ansi: bool,
) -> Result<(serde_json::Value, crate::herdr::Session, String), String> {
    let text = h.read(&s.pane_id, "recent", lines, ansi)?;
    let question = if s.blocked() {
        h.read(&s.pane_id, "detection", 0, false).ok()
    } else {
        None
    };
    if s.limit.is_none() && s.status != "working" {
        s.limit = if ansi {
            let plain: Vec<String> = crate::ansi::lines(&text)
                .iter()
                .map(crate::ansi::plain)
                .collect();
            crate::limits::detect(&plain.join("\n"))
        } else {
            crate::limits::detect(&text)
        };
    }
    let mut v = fleet::session_to_json(&s);
    // showcase: the screen (and its question) redacted or hidden on the way
    // out (`showcase.screens`); what's worked out above saw the real one
    v["text"] = json!(crate::showcase::screen_text(&text));
    v["approve_keys"] = json!(question.as_deref().and_then(herdr::approve_keys));
    v["question"] = json!(question.as_deref().map(crate::showcase::screen_text));
    Ok((v, s, text))
}

pub fn peek(a: Args) -> ExitCode {
    let p = a.positional();
    let Some(q) = p.first() else {
        return fail("usage: aleph peek <name | host:name> [-n LINES] [--ansi] [--json]");
    };
    let lines: usize = a
        .rest
        .iter()
        .position(|x| x == "-n")
        .and_then(|i| a.rest.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let (flag, name) = match q.split_once(':') {
        Some((h, n)) if !h.is_empty() => (Some(h.to_string()), n.to_string()),
        _ => (a.flag.clone(), q.to_string()),
    };
    let (t, s) = match cli::find(&a.cfg, &flag, &name) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let h = herdr::Herdr::new(&t);
    let ansi = a.has("--ansi");
    let (v, s, text) = match peek_json(&h, s, lines, ansi) {
        Ok(x) => x,
        Err(e) => {
            err(&e);
            return ExitCode::from(1);
        }
    };
    if a.has("--json") {
        json(&v);
        return ExitCode::SUCCESS;
    }
    let question = v["question"].as_str().map(str::to_string);
    print!(
        "{}",
        crate::showcase::text(&crate::showcase::screen_text(&text))
    );
    if !text.ends_with('\n') {
        println!();
    }
    if let Some(r) = &s.limit {
        warn(&format!(
            "{} stopped at its usage limit{}",
            s.display(),
            if r.is_empty() {
                String::new()
            } else {
                format!(" · resets {r}")
            }
        ));
    }
    if let Some(qn) = question {
        err(&format!("{} is asking:", s.display()));
        eprintln!("{}", crate::showcase::text(&qn));
        match herdr::approve_keys(&qn) {
            Some(k) => hint(&format!(
                "aleph approve {}   (sends {}) · or aleph keys {} <keys>",
                s.name,
                k.join(" "),
                s.name
            )),
            None => hint(&format!(
                "aleph keys {} <keys>   e.g. 2 · enter · esc",
                s.name
            )),
        }
    }
    ExitCode::SUCCESS
}
