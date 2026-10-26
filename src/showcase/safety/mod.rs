//! Safety nets for live demos, each one switched and tuned in Settings ›
//! Showcase, and each acting at output only (nothing real ever changes):
//!
//! * **screens** (`showcase.screens`, `showcase.demo_screens`,
//!   `showcase.screens_keep`) — what an agent's screen shows wherever it
//!   shows: masked (the lens, as it is), redacted (its words in alien
//!   script, its frame kept: `redact.rs`) or hidden (a card);
//! * **the pause** (`showcase.panic_key`, `aleph showcase panic`) — every
//!   window blank at once, a starfield and the ℵ (`pause.rs`);
//! * **the clipboard** (`showcase.clipboard`) — what a copy puts there: the
//!   masked text, nothing, or the real text;
//! * **money** (`showcase.money`, `showcase.money_scale`) — every $ amount
//!   real, scaled (totals still add up) or hidden (`$—`).
//!
//! The settings are read again at most once a second (each process: the
//! daemon, a dashboard, a CLI run); tests set their own (`force`).

pub mod pause;
pub mod redact;
#[cfg(test)]
mod tests;

use super::state::{self, Mode};
use crate::config::Config;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What an agent's screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screens {
    /// as it is, through the lens
    Masked,
    /// its words in alien script, its frame kept
    Redact,
    /// a card saying it's hidden
    Hide,
}

impl Screens {
    fn parse(s: &str) -> Screens {
        match s.trim() {
            "redact" => Screens::Redact,
            "hide" => Screens::Hide,
            _ => Screens::Masked,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Screens::Masked => "masked",
            Screens::Redact => "redact",
            Screens::Hide => "hide",
        }
    }
}

/// What a copy puts on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Masked,
    Off,
    Real,
}

impl Clip {
    fn parse(s: &str) -> Clip {
        match s.trim() {
            "off" => Clip::Off,
            "real" => Clip::Real,
            _ => Clip::Masked,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Clip::Masked => "masked",
            Clip::Off => "off",
            Clip::Real => "real",
        }
    }
}

/// How $ amounts show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Money {
    Real,
    /// every amount times this
    Scaled(f64),
    Hidden,
}

impl Money {
    pub fn name(self) -> &'static str {
        match self {
            Money::Real => "real",
            Money::Scaled(_) => "scaled",
            Money::Hidden => "hidden",
        }
    }
}

/// The scale when `showcase.money_scale` isn't a sensible number.
pub const SCALE: f64 = 0.37;

/// Every safety setting, as one read of the config has them.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub screens: Screens,
    pub demo_screens: Screens,
    /// more phrases that mark a screen line as the agent's own (lowercase)
    pub keep: Vec<String>,
    /// the keys that pause and resume (as the key map writes them)
    pub panic_keys: Vec<String>,
    pub clipboard: Clip,
    pub money: Money,
}

impl Default for Settings {
    /// Every setting's documented default.
    fn default() -> Self {
        Settings::from_lookup(&|k| {
            crate::config::known(k)
                .map(|k| k.default.to_string())
                .unwrap_or_default()
        })
    }
}

impl Settings {
    pub fn from_config(cfg: &Config) -> Settings {
        Settings::from_lookup(&|k| cfg.get_or(k))
    }

    fn from_lookup(get: &dyn Fn(&str) -> String) -> Settings {
        let list = |k: &str| -> Vec<String> {
            get(k)
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        };
        let scale = get("showcase.money_scale")
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite() && *x > 0.0)
            .unwrap_or(SCALE);
        Settings {
            screens: Screens::parse(&get("showcase.screens")),
            demo_screens: Screens::parse(&get("showcase.demo_screens")),
            keep: list("showcase.screens_keep")
                .into_iter()
                .map(|x| x.to_lowercase())
                .collect(),
            panic_keys: list("showcase.panic_key")
                .into_iter()
                .filter(|k| !matches!(k.as_str(), "off" | "none"))
                .collect(),
            clipboard: Clip::parse(&get("showcase.clipboard")),
            money: match get("showcase.money").trim() {
                "scaled" => Money::Scaled(scale),
                "hidden" => Money::Hidden,
                _ => Money::Real,
            },
        }
    }
}

thread_local! {
    /// tests: settings of their own (the config files are never read)
    static FORCED: std::cell::RefCell<Option<Arc<Settings>>> = const { std::cell::RefCell::new(None) };
}

/// Tests: run with these settings (None: the defaults).
#[cfg(test)]
pub fn force(s: Option<Settings>) {
    FORCED.with(|f| *f.borrow_mut() = s.map(Arc::new));
}

/// The settings now (the config read again past a second).
pub fn settings() -> Arc<Settings> {
    if let Some(s) = FORCED.with(|f| f.borrow().clone()) {
        return s;
    }
    if cfg!(test) {
        return Arc::new(Settings::default());
    }
    static C: Mutex<Option<(Instant, Arc<Settings>)>> = Mutex::new(None);
    let mut c = C.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, s)) = c.as_ref() {
        if at.elapsed().as_millis() < 1000 {
            return s.clone();
        }
    }
    let s = Arc::new(Settings::from_config(&Config::load()));
    *c = Some((Instant::now(), s.clone()));
    s
}

// ------------------------------------------------------------------ screens

/// What screens show now: the demo world's made-up ones follow their own
/// setting (as they are, by default: nothing on them is anyone's).
pub fn screens_now() -> Screens {
    let s = settings();
    match state::mode() {
        Mode::Off => Screens::Masked,
        Mode::Demo => s.demo_screens,
        Mode::Masked => s.screens,
    }
}

/// An agent's screen as text (plain or ANSI): redacted or hidden, or None
/// to leave it to the lens.
pub fn screen_text(s: &str) -> Option<String> {
    match screens_now() {
        Screens::Masked => None,
        Screens::Redact => Some(redact::text(s, &settings().keep)),
        Screens::Hide if s.trim().is_empty() => None,
        Screens::Hide => Some(redact::HIDDEN.to_string()),
    }
}

/// An agent's screen just drawn into `area`: redacted or hidden there.
/// True when it's hidden (no cursor on the card).
pub fn screen_area(buf: &mut Buffer, area: Rect) -> bool {
    match screens_now() {
        Screens::Masked => false,
        Screens::Redact => {
            redact::cells(buf, area, &settings().keep);
            false
        }
        Screens::Hide => {
            redact::card(buf, area);
            true
        }
    }
}

// ------------------------------------------------------------------ the clipboard

/// What a copy of `text` puts on the clipboard (`screen`: an agent's
/// screen, which goes as the screen shows it): None for nothing.
pub fn clip(text: &str, screen: bool) -> Option<Cow<'_, str>> {
    match settings().clipboard {
        Clip::Real => Some(Cow::Borrowed(text)),
        Clip::Off => None,
        Clip::Masked => {
            let t = if screen {
                screen_text(text).map_or(Cow::Borrowed(text), Cow::Owned)
            } else {
                Cow::Borrowed(text)
            };
            Some(Cow::Owned(super::scan::scrub(&t)))
        }
    }
}

/// Why nothing was copied, while copying is off.
pub const CLIP_OFF: &str = "showcase: copying is off (showcase.clipboard)";

// ------------------------------------------------------------------ money

/// A $ amount as it shows: scaled, or None when hidden.
pub fn money(v: f64) -> Option<f64> {
    match settings().money {
        Money::Real => Some(v),
        Money::Scaled(k) => Some(v * k),
        Money::Hidden => None,
    }
}

/// A JSON field that holds dollars.
pub fn money_key(k: &str) -> bool {
    matches!(
        k,
        "cost" | "cost_reported" | "cost_estimated" | "monthly" | "usd" | "price" | "spend"
    ) || k.ends_with("_usd")
        || k.ends_with("_cost")
}

/// A dollar field on its way out: scaled (to the micro-dollar), or 0 when
/// hidden (every client reads 0 as "no amount": `–`).
pub fn money_json(n: &serde_json::Number) -> Value {
    let Some(v) = n.as_f64() else {
        return Value::Number(n.clone());
    };
    match money(v) {
        Some(x) if x == v => Value::Number(n.clone()),
        Some(x) => json!((x * 1e6).round() / 1e6),
        None => json!(0),
    }
}

// ------------------------------------------------------------------ what clients follow

/// The safety nets as clients follow them (the web console's banner, the
/// marker on snapshots and `/api/v1/me`): the pause, money, copies, screens.
pub fn marker() -> Value {
    let s = settings();
    json!({
        "paused": state::panic(),
        "money": s.money.name(),
        "clipboard": s.clipboard.name(),
        "screens": screens_now().name(),
    })
}

/// `aleph showcase status`'s lines about the safety nets, each fine or a
/// warning (`on`: showcase mode is on; off, only the pause has a say).
pub fn status_lines(on: bool) -> Vec<(bool, String)> {
    let s = settings();
    let mut v = Vec::new();
    if state::panic() {
        v.push((
            false,
            match s.panic_keys.first() {
                Some(k) => format!(
                    "paused: every window shows the stars ({} or aleph showcase panic off brings them back)",
                    pause::pretty(k)
                ),
                None => "paused: every window shows the stars (aleph showcase panic off brings them back)".to_string(),
            },
        ));
    } else if on {
        v.push((
            true,
            if s.panic_keys.is_empty() {
                "no panic key (showcase.panic_key): aleph showcase panic pauses every window"
                    .to_string()
            } else {
                let keys: Vec<String> = s.panic_keys.iter().map(|k| pause::pretty(k)).collect();
                format!("{} pauses every window (the panic key)", keys.join(" or "))
            },
        ));
    }
    if !on {
        return v;
    }
    v.push((
        true,
        format!(
            "screens: {} (the demo world's: {})",
            s.screens.name(),
            s.demo_screens.name()
        ),
    ));
    v.push((
        s.clipboard != Clip::Real,
        match s.clipboard {
            Clip::Masked => "what you copy is the masked text".to_string(),
            Clip::Off => "copying is off".to_string(),
            Clip::Real => "what you copy (the clipboard) is the real text".to_string(),
        },
    ));
    v.push((
        true,
        match s.money {
            Money::Real => "$ amounts are real".to_string(),
            Money::Scaled(k) => format!("$ amounts are scaled ×{k}"),
            Money::Hidden => "$ amounts are hidden ($—)".to_string(),
        },
    ));
    v
}
