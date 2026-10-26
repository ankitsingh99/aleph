//! What a request cost, estimated from token counts: a built-in price table
//! per model (USD per million tokens), overridable per model with
//! `analytics.prices.<model> = "input,output[,cache write 5m[,cache write 1h[,cache read]]]"`.
//! Nothing is ever fetched; a model the table doesn't know gets no estimate.
//!
//! Sources: Anthropic's rates are from platform.claude.com/docs/en/about-claude/pricing
//! (checked 2026-10-22). OpenAI and Google rates are their published list
//! prices as last known to us (mid 2025 – early 2026): good for an estimate,
//! and every row can be overridden.

use crate::config::Config;

/// USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

impl Rates {
    /// Anthropic's structure: 5-minute writes 1.25×, 1-hour writes 2×, reads 0.1×.
    const fn anthropic(input: f64, output: f64, cache_read: f64) -> Rates {
        Rates {
            input,
            output,
            cache_write_5m: input * 1.25,
            cache_write_1h: input * 2.0,
            cache_read,
        }
    }
    /// OpenAI and Google: no write premium (cached input is just cheaper).
    const fn flat(input: f64, output: f64, cache_read: f64) -> Rates {
        Rates {
            input,
            output,
            cache_write_5m: input,
            cache_write_1h: input,
            cache_read,
        }
    }
    fn scaled(self, k: f64) -> Rates {
        Rates {
            input: self.input * k,
            output: self.output * k,
            cache_write_5m: self.cache_write_5m * k,
            cache_write_1h: self.cache_write_1h * k,
            cache_read: self.cache_read * k,
        }
    }
}

/// A model's prices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub base: Rates,
    /// a request whose prompt (input + cache reads + writes) is longer than
    /// `.0` tokens pays `.1` instead
    pub long: Option<(u64, Rates)>,
    /// fast mode (`speed: "fast"`) multiplies everything by this
    pub fast: Option<f64>,
    /// US-only inference (`inference_geo: "us"`) costs 1.1× on these models
    pub geo: bool,
}

const fn p(base: Rates) -> Price {
    Price {
        base,
        long: None,
        fast: None,
        geo: false,
    }
}

/// Claude 4.6 and later: US-only inference is 1.1×.
const fn p46(base: Rates) -> Price {
    Price {
        base,
        long: None,
        fast: None,
        geo: true,
    }
}

const OPUS_45: Rates = Rates::anthropic(5.0, 25.0, 0.50);
const OPUS_4: Rates = Rates::anthropic(15.0, 75.0, 1.50);
const SONNET_4: Rates = Rates::anthropic(3.0, 15.0, 0.30);
/// Sonnet 4 / 4.5 past 200K tokens (the 1M-context beta): 2× input, 1.5× output.
const SONNET_4_LONG: Rates = Rates {
    input: 6.0,
    output: 22.5,
    cache_write_5m: 7.5,
    cache_write_1h: 12.0,
    cache_read: 0.60,
};
const FABLE: Rates = Rates::anthropic(10.0, 50.0, 1.0);

/// The built-in table, by normalized model id ([`normalize`]).
const TABLE: &[(&str, Price)] = &[
    // ---- Anthropic (platform.claude.com pricing, 2026-10-22)
    ("claude-fable-5-1", p46(Rates::anthropic(10.0, 50.0, 0.25))),
    ("claude-mythos-5-1", p46(Rates::anthropic(10.0, 50.0, 0.25))),
    ("claude-fable-5", p46(FABLE)),
    ("claude-mythos-5", p46(FABLE)),
    (
        "claude-opus-5-5",
        Price {
            fast: Some(2.0),
            ..p46(Rates::anthropic(4.0, 20.0, 0.20))
        },
    ),
    (
        "claude-opus-5",
        Price {
            fast: Some(2.0),
            ..p46(OPUS_45)
        },
    ),
    (
        "claude-opus-4-8",
        Price {
            fast: Some(2.0),
            ..p46(OPUS_45)
        },
    ),
    ("claude-opus-4-7", p46(OPUS_45)),
    (
        // fast mode on Opus 4.6 was $30 / $150 (6×) while it lasted
        "claude-opus-4-6",
        Price {
            fast: Some(6.0),
            ..p46(OPUS_45)
        },
    ),
    ("claude-opus-4-5", p(OPUS_45)),
    ("claude-opus-4-1", p(OPUS_4)),
    ("claude-opus-4", p(OPUS_4)),
    ("claude-sonnet-5-5", p46(Rates::anthropic(2.0, 10.0, 0.10))),
    ("claude-sonnet-5", p46(Rates::anthropic(2.0, 10.0, 0.20))),
    ("claude-sonnet-4-6", p46(SONNET_4)),
    (
        "claude-sonnet-4-5",
        Price {
            long: Some((200_000, SONNET_4_LONG)),
            ..p(SONNET_4)
        },
    ),
    (
        "claude-sonnet-4",
        Price {
            long: Some((200_000, SONNET_4_LONG)),
            ..p(SONNET_4)
        },
    ),
    (
        "claude-haiku-5-5",
        Price {
            long: Some((100_000, Rates::anthropic(0.50, 2.50, 0.05))),
            ..p46(Rates::anthropic(0.10, 0.50, 0.01))
        },
    ),
    ("claude-haiku-4-5", p(Rates::anthropic(1.0, 5.0, 0.10))),
    ("claude-3-5-haiku", p(Rates::anthropic(0.80, 4.0, 0.08))),
    // retired, still in old logs
    ("claude-3-7-sonnet", p(SONNET_4)),
    ("claude-3-5-sonnet", p(SONNET_4)),
    ("claude-3-opus", p(OPUS_4)),
    ("claude-3-haiku", p(Rates::anthropic(0.25, 1.25, 0.03))),
    // ---- OpenAI (list prices as last known; override when they change)
    ("gpt-5", p(Rates::flat(1.25, 10.0, 0.125))),
    ("gpt-5-codex", p(Rates::flat(1.25, 10.0, 0.125))),
    ("gpt-5-mini", p(Rates::flat(0.25, 2.0, 0.025))),
    ("gpt-5-nano", p(Rates::flat(0.05, 0.40, 0.005))),
    ("gpt-5-pro", p(Rates::flat(15.0, 120.0, 15.0))),
    ("gpt-5.1", p(Rates::flat(1.25, 10.0, 0.125))),
    ("gpt-5.1-codex", p(Rates::flat(1.25, 10.0, 0.125))),
    ("gpt-5.1-codex-max", p(Rates::flat(1.25, 10.0, 0.125))),
    ("gpt-5.1-codex-mini", p(Rates::flat(0.25, 2.0, 0.025))),
    ("gpt-5.2", p(Rates::flat(1.75, 14.0, 0.175))),
    ("gpt-5.2-codex", p(Rates::flat(1.75, 14.0, 0.175))),
    ("codex-mini-latest", p(Rates::flat(1.50, 6.0, 0.375))),
    ("gpt-4.1", p(Rates::flat(2.0, 8.0, 0.50))),
    ("gpt-4.1-mini", p(Rates::flat(0.40, 1.60, 0.10))),
    ("gpt-4.1-nano", p(Rates::flat(0.10, 0.40, 0.025))),
    ("gpt-4o", p(Rates::flat(2.50, 10.0, 1.25))),
    ("gpt-4o-mini", p(Rates::flat(0.15, 0.60, 0.075))),
    ("o3", p(Rates::flat(2.0, 8.0, 0.50))),
    ("o3-pro", p(Rates::flat(20.0, 80.0, 20.0))),
    ("o3-mini", p(Rates::flat(1.10, 4.40, 0.55))),
    ("o4-mini", p(Rates::flat(1.10, 4.40, 0.275))),
    ("o1", p(Rates::flat(15.0, 60.0, 7.50))),
    // ---- Google (list prices as last known)
    (
        "gemini-3-pro",
        Price {
            long: Some((200_000, Rates::flat(4.0, 18.0, 0.40))),
            ..p(Rates::flat(2.0, 12.0, 0.20))
        },
    ),
    (
        "gemini-2.5-pro",
        Price {
            long: Some((200_000, Rates::flat(2.50, 15.0, 0.25))),
            ..p(Rates::flat(1.25, 10.0, 0.125))
        },
    ),
    ("gemini-2.5-flash", p(Rates::flat(0.30, 2.50, 0.03))),
    ("gemini-2.5-flash-lite", p(Rates::flat(0.10, 0.40, 0.01))),
    ("gemini-2.0-flash", p(Rates::flat(0.10, 0.40, 0.025))),
    ("gemini-2.0-flash-lite", p(Rates::flat(0.075, 0.30, 0.075))),
];

/// The id a price is looked up by: lowercase, provider and cloud decorations
/// removed. `anthropic/claude-sonnet-4.5`, `us.anthropic.claude-sonnet-4-5-20250929-v1:0`,
/// `claude-sonnet-4-5@20250929` and `claude-sonnet-4-5-20250929[1m]` are all
/// `claude-sonnet-4-5`; `gpt-4o-2024-08-06` is `gpt-4o`.
pub fn normalize(model: &str) -> String {
    let mut m = model.trim().to_lowercase();
    if let Some(i) = m.rfind('/') {
        m = m[i + 1..].to_string();
    }
    // Bedrock: `us.anthropic.claude-…`, `anthropic.claude-…`
    if let Some(i) = m.find("anthropic.") {
        m = m[i + "anthropic.".len()..].to_string();
    }
    for cut in ['[', '@', ':'] {
        if let Some(i) = m.find(cut) {
            m.truncate(i);
        }
    }
    // Bedrock's `-v1`, then a date: `-20250929` or `-2024-08-06`
    if let Some(i) = m.rfind("-v") {
        if m[i + 2..].chars().all(|c| c.is_ascii_digit()) && m.len() > i + 2 {
            m.truncate(i);
        }
    }
    let b = m.as_bytes();
    let n = b.len();
    if n > 9 && b[n - 9] == b'-' && b[n - 8..].iter().all(u8::is_ascii_digit) {
        m.truncate(n - 9);
    } else if n > 11
        && b[n - 11] == b'-'
        && b[n - 6] == b'-'
        && b[n - 3] == b'-'
        && [&b[n - 10..n - 6], &b[n - 5..n - 3], &b[n - 2..]]
            .iter()
            .all(|s| s.iter().all(u8::is_ascii_digit))
    {
        m.truncate(n - 11);
    }
    // Gemini previews: `gemini-2.5-flash-preview-05-20`, `gemini-3-pro-preview`
    if let Some(i) = m.find("-preview") {
        if m.starts_with("gemini") {
            m.truncate(i);
        }
    }
    if m.starts_with("claude") {
        m = m.replace('.', "-");
        // `claude-opus-4-0` is `claude-opus-4`
        if let Some(s) = m.strip_suffix("-0") {
            m = s.to_string();
        }
    }
    m
}

fn builtin(norm: &str) -> Option<Price> {
    TABLE.iter().find(|(k, _)| *k == norm).map(|(_, p)| *p)
}

/// `analytics.prices.<model>` values: `in,out[,cw5m[,cw1h[,cr]]]`, or named
/// (`in=1.25 out=10 cr=0.125`). Missing cache prices follow Anthropic's
/// structure (writes 1.25× / 2×, reads 0.1× of input).
pub fn parse_override(v: &str) -> Result<Rates, String> {
    let words: Vec<&str> = v
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();
    let num = |s: &str| -> Result<f64, String> {
        let s = s.trim().trim_start_matches('$');
        s.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite() && *x >= 0.0)
            .ok_or_else(|| format!("not a price: {s}"))
    };
    let (mut i, mut o, mut w5, mut w1, mut r) = (None, None, None, None, None);
    if words.iter().any(|w| w.contains('=')) {
        for w in &words {
            let Some((k, x)) = w.split_once('=') else {
                return Err(format!("expected name=price, got {w}"));
            };
            let x = num(x)?;
            match k.trim() {
                "in" | "input" => i = Some(x),
                "out" | "output" => o = Some(x),
                "cw" | "cw5m" | "write" | "cache_write" | "cache_write_5m" => w5 = Some(x),
                "cw1h" | "cache_write_1h" => w1 = Some(x),
                "cr" | "read" | "cache_read" => r = Some(x),
                other => {
                    return Err(format!(
                        "unknown price name {other} (in, out, cw5m, cw1h, cr)"
                    ))
                }
            }
        }
    } else {
        let v: Vec<f64> = words.iter().map(|w| num(w)).collect::<Result<_, _>>()?;
        if v.len() < 2 || v.len() > 5 {
            return Err("give input,output[,cache write 5m[,cache write 1h[,cache read]]] in $ per million tokens".into());
        }
        i = Some(v[0]);
        o = Some(v[1]);
        w5 = v.get(2).copied();
        w1 = v.get(3).copied();
        r = v.get(4).copied();
    }
    let (Some(i), Some(o)) = (i, o) else {
        return Err("an input and an output price are needed".into());
    };
    Ok(Rates {
        input: i,
        output: o,
        cache_write_5m: w5.unwrap_or(i * 1.25),
        cache_write_1h: w1.unwrap_or(i * 2.0),
        cache_read: r.unwrap_or(i * 0.1),
    })
}

/// Prices in effect: the built-in table plus `analytics.prices.*` overrides.
#[derive(Clone, Debug, Default)]
pub struct Table {
    overrides: Vec<(String, Rates)>,
}

impl Table {
    pub fn from_cfg(cfg: &Config) -> Table {
        let mut overrides = Vec::new();
        for (k, v) in cfg.with_prefix("analytics.prices.") {
            match parse_override(&v) {
                Ok(r) => overrides.push((normalize(&k), r)),
                Err(e) => crate::log::warn("usage", &format!("analytics.prices.{k}: {e}")),
            }
        }
        Table { overrides }
    }

    /// A model's prices (None: unknown, so no estimate).
    pub fn get(&self, model: &str) -> Option<Price> {
        let n = normalize(model);
        if let Some((_, r)) = self.overrides.iter().find(|(k, _)| *k == n) {
            return Some(p(*r));
        }
        builtin(&n)
    }

    /// A fingerprint of everything that changes estimates (stale rollups are rebuilt).
    pub fn stamp(&self) -> String {
        let mut s = env!("CARGO_PKG_VERSION").to_string();
        for (k, r) in &self.overrides {
            s.push_str(&format!(
                "|{k}={},{},{},{},{}",
                r.input, r.output, r.cache_write_5m, r.cache_write_1h, r.cache_read
            ));
        }
        s
    }
}

/// What a subscription costs a month, US$ (`analytics.plans.<plan>`
/// overrides it, e.g. `analytics.plans.max-5x = 90`): the note beside usage
/// that ran on a plan. Usage is always priced as API usage; this never
/// discounts it.
pub fn plan_price(cfg: &Config, agent: &str, plan: &str) -> Option<f64> {
    let key = plan.trim().to_lowercase().replace([' ', '_'], "-");
    if key.is_empty() {
        return None;
    }
    if let Some(v) = cfg.get(&format!("analytics.plans.{key}")) {
        return v.trim().trim_start_matches('$').parse().ok();
    }
    let p = match (agent, key.as_str()) {
        ("claude", "pro") => 20.0,
        ("claude", "max-5x") => 100.0,
        ("claude", "max-20x") => 200.0,
        ("claude", "team") => 30.0,
        ("codex", "plus") => 20.0,
        ("codex", "prolite") => 100.0,
        ("codex", "pro") => 200.0,
        ("codex", "team" | "business") => 30.0,
        ("gemini", "pro") => 20.0,
        ("gemini", "ultra") => 250.0,
        _ => return None,
    };
    Some(p)
}

/// One request's tokens, for [`cost`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub web_searches: u64,
}

/// What one request cost at these prices (USD). `fast` / `us` are the
/// request's speed and inference geography.
pub fn cost(pr: &Price, t: &Tokens, fast: bool, us: bool) -> f64 {
    let prompt = t.input + t.cache_read + t.cache_write_5m + t.cache_write_1h;
    let mut r = match pr.long {
        Some((above, long)) if prompt > above => long,
        _ => pr.base,
    };
    if fast {
        if let Some(k) = pr.fast {
            r = r.scaled(k);
        }
    }
    if us && pr.geo {
        r = r.scaled(1.1);
    }
    let m = 1e-6;
    t.input as f64 * r.input * m
        + t.output as f64 * r.output * m
        + t.cache_read as f64 * r.cache_read * m
        + t.cache_write_5m as f64 * r.cache_write_5m * m
        + t.cache_write_1h as f64 * r.cache_write_1h * m
        // server-side web search: $10 per 1,000 (Anthropic)
        + t.web_searches as f64 * 0.01
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_normalize() {
        for (raw, want) in [
            ("claude-opus-5-5", "claude-opus-5-5"),
            ("claude-opus-5-5[1m]", "claude-opus-5-5"),
            ("claude-sonnet-4-5-20250929", "claude-sonnet-4-5"),
            (
                "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
                "claude-sonnet-4-5",
            ),
            (
                "anthropic.claude-haiku-4-5-20251001-v1:0",
                "claude-haiku-4-5",
            ),
            ("claude-opus-4-5@20251101", "claude-opus-4-5"),
            ("anthropic/claude-sonnet-4.5", "claude-sonnet-4-5"),
            ("claude-opus-4-20250514", "claude-opus-4"),
            ("claude-opus-4-0", "claude-opus-4"),
            ("claude-3-5-sonnet-20241022", "claude-3-5-sonnet"),
            ("gpt-4o-2024-08-06", "gpt-4o"),
            ("openai/gpt-5.1-codex", "gpt-5.1-codex"),
            ("GPT-5", "gpt-5"),
            ("models/gemini-2.5-flash-preview-05-20", "gemini-2.5-flash"),
            ("gemini-3-pro-preview", "gemini-3-pro"),
            ("gpt-6-astra", "gpt-6-astra"),
        ] {
            assert_eq!(normalize(raw), want, "{raw}");
        }
    }

    #[test]
    fn known_and_unknown_models() {
        let t = Table::default();
        let o = t.get("claude-opus-5-5").unwrap();
        assert_eq!(o.base.input, 4.0);
        assert_eq!(o.base.output, 20.0);
        assert_eq!(o.base.cache_write_5m, 5.0);
        assert_eq!(o.base.cache_write_1h, 8.0);
        assert_eq!(o.base.cache_read, 0.20);
        assert_eq!(t.get("claude-sonnet-5-5").unwrap().base.cache_read, 0.10);
        assert_eq!(t.get("claude-fable-5-1").unwrap().base.cache_read, 0.25);
        assert_eq!(t.get("claude-opus-4-1-20250805").unwrap().base.input, 15.0);
        assert!(t.get("gpt-6-astra").is_none());
        assert!(t.get("").is_none());
        assert!(t.get("claude-opus-4-9").is_none(), "no guessing by family");
    }

    #[test]
    fn costs_with_tiers_fast_and_geo() {
        let t = Table::default();
        let tok = Tokens {
            input: 1_000_000,
            output: 1_000_000,
            ..Default::default()
        };
        let o = t.get("claude-opus-5-5").unwrap();
        assert!((cost(&o, &tok, false, false) - 24.0).abs() < 1e-9);
        assert!((cost(&o, &tok, true, false) - 48.0).abs() < 1e-9);
        assert!((cost(&o, &tok, false, true) - 26.4).abs() < 1e-9);
        // no geo premium before Claude 4.6
        let s45 = t.get("claude-sonnet-4-5").unwrap();
        let small = Tokens {
            input: 100_000,
            output: 10_000,
            cache_read: 50_000,
            ..Default::default()
        };
        let c = cost(&s45, &small, false, true);
        assert!((c - (0.3 + 0.15 + 0.015)).abs() < 1e-9, "{c}");
        // past 200K: the long-context rates
        let big = Tokens {
            input: 150_000,
            cache_read: 100_000,
            output: 1000,
            ..Default::default()
        };
        let c = cost(&s45, &big, false, false);
        assert!((c - (0.9 + 0.06 + 0.0225)).abs() < 1e-9, "{c}");
        // Haiku 5.5 over 100K
        let h = t.get("claude-haiku-5-5").unwrap();
        let c = cost(
            &h,
            &Tokens {
                input: 120_000,
                ..Default::default()
            },
            false,
            false,
        );
        assert!((c - 0.06).abs() < 1e-9, "{c}");
        // cache writes, both TTLs, and web searches
        let w = Tokens {
            cache_write_5m: 1_000_000,
            cache_write_1h: 1_000_000,
            web_searches: 3,
            ..Default::default()
        };
        assert!((cost(&o, &w, false, false) - 13.03).abs() < 1e-9);
    }

    #[test]
    fn overrides() {
        let r = parse_override("1.25,10").unwrap();
        assert_eq!((r.input, r.output), (1.25, 10.0));
        assert_eq!(
            (r.cache_write_5m, r.cache_write_1h, r.cache_read),
            (1.5625, 2.5, 0.125)
        );
        let r = parse_override("2, 8, 2, 2, 0.5").unwrap();
        assert_eq!(r.cache_read, 0.5);
        let r = parse_override("in=3 out=15 cr=0.3").unwrap();
        assert_eq!((r.input, r.output, r.cache_read), (3.0, 15.0, 0.3));
        assert!(parse_override("3").is_err());
        assert!(parse_override("a,b").is_err());
        assert!(parse_override("in=3 bogus=1").is_err());
        assert!(parse_override("-1,2").is_err());
        let cfg = Config::in_memory(&[
            "analytics.prices.gpt-6-astra=2,16,2,2,0.2",
            "analytics.prices.claude-opus-5-5=0,0",
        ]);
        let t = Table::from_cfg(&cfg);
        assert_eq!(t.get("openai/gpt-6-astra").unwrap().base.output, 16.0);
        assert_eq!(t.get("claude-opus-5-5").unwrap().base.input, 0.0);
        assert_ne!(t.stamp(), Table::default().stamp());
    }
}
