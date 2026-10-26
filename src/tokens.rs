//! Machine stats as herdr pane tokens (`herdr_tokens = on`; off by default):
//! every agent pane on a machine carries `$cpu`, `$mem` and, with a GPU,
//! `$gpu` / `$gput`, so herdr's own sidebar can show them where its config
//! asks (`[ui.sidebar.agents] rows_by_agent … "$gpu"`). herdr has tokens per
//! pane, not per workspace, so each pane gets its machine's numbers.
//!
//! They go under a source of their own (`aleph-stats`), apart from the
//! session's name and hue tokens, at most every 30 s per machine, as one
//! shell line for all of its panes — and only while the dashboard has fresh
//! stats for that machine.

use crate::telemetry::Stats;
use std::time::Duration;

/// herdr's metadata source for these tokens.
pub const SOURCE: &str = "aleph-stats";
/// How often a machine's panes are told again.
pub const EVERY: Duration = Duration::from_secs(30);

/// (token, value) for a machine's stats (pure).
pub fn values(st: &Stats) -> Vec<(&'static str, String)> {
    let pct = |x: f64| format!("{:.0}%", (x * 100.0).clamp(0.0, 100.0));
    let mut v = vec![("cpu", pct(st.cpu))];
    if st.mem_total > 0.0 {
        v.push(("mem", pct(st.mem_used / st.mem_total)));
    }
    if !st.gpus.is_empty() {
        let n = st.gpus.len() as f64;
        v.push(("gpu", pct(st.gpus.iter().map(|g| g.util).sum::<f64>() / n)));
        if let Some(t) = st.gpus.iter().filter_map(|g| g.temp).reduce(f64::max) {
            v.push(("gput", format!("{t:.0}°")));
        }
    }
    v
}

/// The one shell line that sets `vals` on these panes (pure).
pub fn script(herdr: &str, panes: &[String], vals: &[(&str, String)]) -> String {
    let q = crate::proc::sh_quote;
    let toks: String = vals
        .iter()
        .map(|(k, v)| format!(" --token {}", q(&format!("{k}={v}"))))
        .collect();
    panes
        .iter()
        .map(|p| {
            format!(
                "{herdr} pane report-metadata {} --source {SOURCE}{toks} >/dev/null 2>&1",
                q(p)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::Gpu;

    #[test]
    fn a_machine_in_four_tokens() {
        let mut st = Stats {
            cpu: 0.226,
            mem_total: 64.0,
            mem_used: 16.0,
            ..Default::default()
        };
        assert_eq!(
            values(&st),
            vec![("cpu", "23%".to_string()), ("mem", "25%".into())]
        );
        st.gpus = vec![
            Gpu {
                util: 0.9,
                temp: Some(71.0),
                ..Default::default()
            },
            Gpu {
                util: 0.5,
                temp: Some(80.4),
                ..Default::default()
            },
        ];
        let v = values(&st);
        assert!(v.contains(&("gpu", "70%".into())) && v.contains(&("gput", "80°".into())));
        let s = script("herdr", &["w1:p2".into(), "w1:p3".into()], &v[..1]);
        assert_eq!(
            s,
            "herdr pane report-metadata w1:p2 --source aleph-stats --token cpu=23% >/dev/null 2>&1; herdr pane report-metadata w1:p3 --source aleph-stats --token cpu=23% >/dev/null 2>&1"
        );
    }
}
