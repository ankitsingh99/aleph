//! `aleph services [--host H] [--json]` — local LLM servers and AI gateways
//! on every machine, probed once (whatever `services` is set to): an aligned
//! table per machine, TSV when piped, JSON for Raycast and scripts.

use crate::cli::{self, Args};
use crate::config::Config;
use crate::out::{err, fail, hint, json, out, Spinner};
use crate::services::{self, Found, Health, Scan};
use crate::theme;
use crate::ui::{self, c};
use ratatui::prelude::*;
use serde_json::json;
use std::io::IsTerminal;
use std::process::ExitCode;

const USAGE: &str = "usage: aleph services [--host <alias>] [--json]";

pub fn cmd(a: Args) -> ExitCode {
    let mut host = a.flag.clone();
    let mut want_json = false;
    let mut i = 0;
    while i < a.rest.len() {
        let w = a.rest[i].as_str();
        match w {
            "--host" | "-H" => match a.rest.get(i + 1) {
                Some(h) => {
                    host = Some(h.clone());
                    i += 1;
                }
                None => return fail("--host needs an ssh alias (or local)"),
            },
            "--json" => want_json = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                println!(
                    "  local LLM servers and AI gateways on every machine — aleph help services"
                );
                return ExitCode::SUCCESS;
            }
            _ if w.starts_with("--host=") => host = Some(w["--host=".len()..].to_string()),
            _ => return fail(&format!("unknown argument '{w}' ({USAGE})")),
        }
        i += 1;
    }
    let hosts: Vec<String> = match host {
        Some(h) => vec![h],
        None => a.cfg.fleet(),
    };
    if hosts.is_empty() {
        return cli::no_host();
    }
    let on = services::enabled_services(&a.cfg);
    let sp = Spinner::start(format!(
        "Looking for LLM servers on {}",
        if hosts.len() == 1 {
            hosts[0].clone()
        } else {
            format!("{} machines", hosts.len())
        }
    ));
    let scans = services::scan_all(&hosts, &on);
    sp.stop();
    for s in &scans {
        services::save_cache(s);
    }
    let live = services::enabled(&a.cfg);
    let all_failed = scans.iter().all(|s| s.err.is_some());
    let code = if all_failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    };
    if want_json {
        json(&json!({
            "generated_at": crate::cache::now(),
            "enabled": live,
            "interval": services::interval(&a.cfg).as_secs(),
            "services": on.iter().map(|s| s.id).collect::<Vec<_>>(),
            "hosts": scans.iter().map(|s| s.to_json(&theme::machine_hex(&a.cfg, &s.alias))).collect::<Vec<_>>(),
        }));
        return code;
    }
    if !std::io::stdout().is_terminal() {
        tsv(&scans);
    } else {
        for l in table(&a.cfg, &scans, ui::print_width()) {
            out(l);
        }
    }
    for s in scans.iter().filter(|s| s.err.is_some()) {
        if !std::io::stdout().is_terminal() {
            err(&format!(
                "{}: {}",
                s.alias,
                s.err.clone().unwrap_or_default()
            ));
        }
    }
    if !live {
        hint("aleph settings set services on keeps them live in the dashboard (Fleet and Monitor)");
    }
    code
}

/// One line per server: host, service, name, version, port, health, models,
/// loaded, vram (bytes), model names.
fn tsv(scans: &[Scan]) {
    println!("host\tservice\tname\tversion\tport\thealth\tmodels\tloaded\tvram\tmodel_names");
    for s in scans {
        for f in &s.found {
            crate::out::raw(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                s.alias,
                f.service.id,
                f.service.name,
                f.version.clone().unwrap_or_default(),
                f.port.map(|p| p.to_string()).unwrap_or_default(),
                f.health.as_str(),
                f.models.len(),
                f.loaded().count(),
                f.vram_total()
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_default(),
                f.models_sorted()
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
    }
}

fn health_text(h: &Health) -> (String, Color) {
    let (g, col) = match h {
        // printed output doesn't animate: a still glyph for "loading"
        Health::Loading => ("◐", c::ACCENT()),
        other => other.dot(std::time::Instant::now()),
    };
    (format!("{g} {}", h.label()), col)
}

fn port_text(f: &Found) -> String {
    match f.port {
        Some(p) if f.ports.len() > 1 => format!(":{p}+{}", f.ports.len() - 1),
        Some(p) => format!(":{p}"),
        None => "–".into(),
    }
}

/// `3 models · 1 loaded: llama3.1:8b, qwen2.5:7b`
fn models_text(f: &Found) -> String {
    let head = f.models_text();
    let names: Vec<&str> = f.models_sorted().iter().map(|m| m.name.as_str()).collect();
    if names.is_empty() {
        head
    } else {
        format!("{head}: {}", names.join(", "))
    }
}

/// The human table: a title, column heads, then each machine and its servers.
fn table(cfg: &Config, scans: &[Scan], width: usize) -> Vec<Line<'static>> {
    let mut v: Vec<Line<'static>> = Vec::new();
    let rows: Vec<&Found> = scans.iter().flat_map(|s| &s.found).collect();
    let nw = rows
        .iter()
        .map(|f| ui::width(f.service.name) + 2)
        .max()
        .unwrap_or(10)
        .max(9);
    let vw = rows
        .iter()
        .map(|f| ui::width(&f.version.clone().unwrap_or_default()).min(14))
        .max()
        .unwrap_or(1)
        .max(7);
    let pw = rows
        .iter()
        .map(|f| ui::width(&port_text(f)))
        .max()
        .unwrap_or(4)
        .max(4);
    let hw = rows
        .iter()
        .map(|f| ui::width(&health_text(&f.health).0))
        .max()
        .unwrap_or(6)
        .max(6);
    let fixed = 2 + nw + 2 + vw + 2 + pw + 2 + hw + 2 + 2 + 6;
    let mw = width.saturating_sub(fixed).max(16);
    let found = rows.len();
    v.push(Line::from(vec![
        Span::styled("⏺ ", ui::fg(c::ACCENT())),
        Span::styled(
            format!(
                "LLM services on {}",
                if scans.len() == 1 {
                    scans[0].alias.clone()
                } else {
                    format!("{} machines", scans.len())
                }
            ),
            ui::bold(),
        ),
        Span::styled(
            format!(
                "  ·  {} found",
                if found == 0 {
                    "none".to_string()
                } else {
                    found.to_string()
                }
            ),
            ui::faint(),
        ),
    ]));
    if found > 0 {
        v.push(Line::from(Span::styled(
            format!(
                "  {}  {}  {}  {}  {}  {:>6}",
                ui::pad("service", nw),
                ui::pad("version", vw),
                ui::pad("port", pw),
                ui::pad("health", hw),
                ui::pad("models", mw),
                "vram"
            ),
            ui::faint(),
        )));
    }
    for s in scans {
        let col = theme::machine(cfg, &s.alias);
        let mut head = vec![
            Span::styled("▌", ui::fg(col)),
            Span::styled(
                s.alias.clone(),
                Style::new().fg(col).add_modifier(Modifier::BOLD),
            ),
        ];
        match &s.err {
            Some(e) => {
                head.push(Span::styled(format!("  ✕ {e}"), ui::fg(c::RED())));
                v.push(Line::from(head));
                continue;
            }
            None => {
                let mut facts = vec![s.listeners.clone(), s.http.clone()];
                facts.retain(|x| !x.is_empty());
                facts.push(format!("{:.1}s", s.took_ms as f64 / 1000.0));
                if s.partial {
                    facts.push("ran out of time: partial".into());
                }
                head.push(Span::styled(
                    format!("  {}", facts.join(" · ")),
                    ui::faint(),
                ));
            }
        }
        v.push(Line::from(head));
        if s.found.is_empty() {
            v.push(Line::from(Span::styled(
                "  no LLM servers or gateways",
                ui::muted(),
            )));
        }
        for f in &s.found {
            let sv = f.service;
            let (ht, hc) = health_text(&f.health);
            let mut l = vec![
                Span::raw("  "),
                services::badge(sv),
                Span::styled(format!(" {}", ui::pad(sv.name, nw - 2)), ui::bold()),
                Span::styled(
                    format!(
                        "  {}",
                        ui::pad(&f.version.clone().unwrap_or_else(|| "–".into()), vw)
                    ),
                    ui::muted(),
                ),
                Span::styled(format!("  {}", ui::pad(&port_text(f), pw)), ui::muted()),
                Span::styled(format!("  {}", ui::pad(&ht, hw)), ui::fg(hc)),
            ];
            let mt = models_text(f);
            l.push(Span::styled(
                format!("  {}", ui::pad(if mt.is_empty() { "–" } else { &mt }, mw)),
                Style::new(),
            ));
            l.push(Span::styled(
                format!("  {:>6}", f.vram_total().map(ui::bytes).unwrap_or_default()),
                ui::fg(c::MAUVE()),
            ));
            v.push(Line::from(l));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simulate(name: &str) -> Scan {
        let on: Vec<_> = services::ALL.iter().collect();
        services::simulate(name, &on)
    }

    #[test]
    fn table_is_aligned_per_machine() {
        let cfg = Config::in_memory(&[]);
        let mut dead = Scan::new("gpu2");
        dead.err = Some("ssh: connect to host gpu2 port 22: Connection refused".into());
        let scans = vec![simulate("gpu-box"), dead, simulate("bare")];
        let lines: Vec<String> = table(&cfg, &scans, 110)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        let text = lines.join("\n");
        assert!(
            lines[0].starts_with("⏺ LLM services on 3 machines") && lines[0].contains("9 found"),
            "{text}"
        );
        assert!(
            lines[1].contains("service")
                && lines[1].contains("version")
                && lines[1].contains("vram")
        );
        assert!(text.contains("▌gpu-box  ss · curl"), "{text}");
        assert!(text.contains("▌gpu2  ✕ ssh: connect to host gpu2 port 22: Connection refused"));
        assert!(text.contains("▌bare  none · none"));
        // every server row has its columns in the same places
        let rows: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains(" ● ") || l.contains(" ? "))
            .collect();
        assert_eq!(rows.len(), 9, "{text}");
        let col = |l: &str, pat: &str| l.find(pat).map(|i| l[..i].chars().count());
        let port_col = col(rows[0], ":11434").unwrap();
        for r in &rows {
            let w = r.chars().count();
            assert!(w <= 110, "too wide ({w}): {r}");
        }
        assert_eq!(col(rows[1], ":8000+1"), Some(port_col), "{text}");
        assert!(
            text.contains("3 models · 1 loaded: llama3.1:8b, nomic-embed"),
            "{text}"
        );
        assert!(text.contains("needs a key"));
        assert!(text.contains("? unknown"));
        // printed output doesn't animate
        let mut s = Scan::new("ws");
        s.found = simulate("ws").found;
        s.at = 1;
        let t: String = table(&cfg, &[s], 100)
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(t.contains("◐ loading"), "{t}");
    }
}
