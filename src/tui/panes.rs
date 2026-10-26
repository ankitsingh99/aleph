//! Every view's panels, laid out one way (Fleet, Monitor, Inbox, Settings):
//! a main area — one panel, or two stacked (Fleet's list over its
//! resources) — and a side panel (the preview, the processes, the detail).
//!
//! The same keys everywhere: `p` shows or hides the side · `z` maximises in
//! turn (main · top · bottom · side · back) · `{` `}` move the divider
//! between the stacked panels · `=` puts it all back · `L` layout mode (↑↓
//! that divider, ←→ the side's width, 1 main · 2 top · 3 bottom · 4 side,
//! `p`, `0` reset, esc or enter done). Each view's shape is remembered in
//! `layout.<view>` (`side 40 · top 35 · hidden · max side`; empty: the
//! view's own).

use super::Dash;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Max {
    #[default]
    None,
    /// the main area alone (the side hidden for now)
    Main,
    /// the top panel alone
    Top,
    /// the bottom panel alone
    Bottom,
    /// the side alone
    Side,
}

impl Max {
    fn word(self) -> &'static str {
        match self {
            Max::None => "",
            Max::Main => "main",
            Max::Top => "top",
            Max::Bottom => "bottom",
            Max::Side => "side",
        }
    }
}

/// How a view's panels share its area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    /// the side's share of the width, % (None: the view's own)
    pub side: Option<u16>,
    /// the top panel's share of the main area's height, % (None: the view's own)
    pub top: Option<u16>,
    /// the side hidden
    pub hidden: bool,
    pub max: Max,
}

/// What a view has: a side panel, two stacked panels in its main area.
#[derive(Clone, Copy, Debug)]
pub struct Parts {
    pub side: bool,
    pub stacked: bool,
}

impl Shape {
    /// From `layout.<view>` (pure — tested).
    pub fn parse(s: &str) -> Shape {
        let mut sh = Shape::default();
        for t in s.split(['·', ',', ';']).map(str::trim) {
            let mut w = t.split_whitespace();
            match (w.next(), w.next()) {
                (Some("side"), Some(n)) => sh.side = n.trim_end_matches('%').parse().ok(),
                (Some("top"), Some(n)) => sh.top = n.trim_end_matches('%').parse().ok(),
                (Some("hidden"), _) => sh.hidden = true,
                (Some("max"), Some(m)) => {
                    sh.max = match m {
                        "main" => Max::Main,
                        "top" => Max::Top,
                        "bottom" => Max::Bottom,
                        "side" => Max::Side,
                        _ => Max::None,
                    }
                }
                _ => {}
            }
        }
        sh.side = sh.side.map(|x| x.clamp(15, 85));
        sh.top = sh.top.map(|x| x.clamp(10, 90));
        sh
    }

    /// As `layout.<view>` keeps it (empty: the view's own).
    pub fn text(&self) -> String {
        let mut v: Vec<String> = Vec::new();
        if let Some(s) = self.side {
            v.push(format!("side {s}"));
        }
        if let Some(t) = self.top {
            v.push(format!("top {t}"));
        }
        if self.hidden {
            v.push("hidden".into());
        }
        if self.max != Max::None {
            v.push(format!("max {}", self.max.word()));
        }
        v.join(" · ")
    }

    /// Main | side across `r` (the side `own` cells wide by the view's own
    /// measure, `gap` between): either can come back empty.
    pub fn columns(&self, r: Rect, own: u16, gap: u16) -> (Rect, Rect) {
        let none = Rect { width: 0, ..r };
        match self.max {
            Max::Side => return (none, r),
            Max::Main | Max::Top | Max::Bottom => return (r, none),
            Max::None => {}
        }
        if self.hidden {
            return (r, none);
        }
        let side = match self.side {
            Some(p) => r.width * p / 100,
            None => own,
        }
        .min(r.width.saturating_sub(gap + 20));
        let main_w = r.width.saturating_sub(side + gap);
        (
            Rect { width: main_w, ..r },
            Rect {
                x: r.x + main_w + gap,
                width: side,
                ..r
            },
        )
    }

    /// Top over bottom in the main area `r` (the top `own` rows tall by the
    /// view's own measure): either can come back empty.
    pub fn rows(&self, r: Rect, own: u16) -> (Rect, Rect) {
        let none = Rect { height: 0, ..r };
        match self.max {
            Max::Top => return (r, none),
            Max::Bottom => return (none, r),
            _ => {}
        }
        let top = match self.top {
            Some(p) => r.height * p / 100,
            None => own,
        }
        .min(r.height);
        (
            Rect { height: top, ..r },
            Rect {
                y: r.y + top,
                height: r.height - top,
                ..r
            },
        )
    }

    /// The next maximise in turn for these parts.
    fn next_max(&self, parts: Parts) -> Max {
        let order: Vec<Max> = [Max::None, Max::Main, Max::Top, Max::Bottom, Max::Side]
            .into_iter()
            .filter(|m| match m {
                Max::Main | Max::Side => parts.side,
                Max::Top | Max::Bottom => parts.stacked,
                Max::None => true,
            })
            .collect();
        let i = order.iter().position(|m| *m == self.max).unwrap_or(0);
        order[(i + 1) % order.len()]
    }
}

/// The view's shape now (from its setting).
pub fn shape(d: &Dash, view: &str) -> Shape {
    Shape::parse(&d.cfg.get(&format!("layout.{view}")).unwrap_or_default())
}

fn keep(d: &mut Dash, view: &str, sh: Shape) {
    let key = format!("layout.{view}");
    let t = sh.text();
    let _ = if t.is_empty() {
        d.cfg.unset(&key)
    } else {
        d.cfg.set(&key, &t)
    };
}

/// What it says when a key changed it.
fn said(sh: &Shape, parts: Parts) -> String {
    let what = match sh.max {
        Max::Main if parts.stacked => "left side maximised",
        Max::Main => "main panel maximised",
        Max::Top => "top panel maximised",
        Max::Bottom => "bottom panel maximised",
        Max::Side => "side panel maximised",
        Max::None if sh.hidden => "side panel hidden · p brings it back",
        Max::None => "layout",
    };
    match (sh.side, sh.top) {
        (None, None) => what.to_string(),
        (s, t) => {
            let mut v = vec![what.to_string()];
            if let Some(s) = s {
                v.push(format!("side {s}%"));
            }
            if let (Some(t), true) = (t, parts.stacked) {
                v.push(format!("top {t}%"));
            }
            v.join(" · ")
        }
    }
}

/// A layout key on `view` (with these `parts`): true when it was one. The
/// view decides first whether its own keys come before (typing, a filter).
pub fn key(d: &mut Dash, view: &'static str, parts: Parts, k: &KeyEvent) -> bool {
    if d.layout_mode.is_some() {
        mode_key(d, view, parts, k);
        return true;
    }
    let mut sh = shape(d, view);
    let before = sh;
    match k.code {
        KeyCode::Char('p') if parts.side => {
            if sh.max == Max::Side {
                sh.max = Max::None;
            } else {
                sh.hidden = !sh.hidden;
                if sh.max == Max::Main {
                    sh.max = Max::None;
                }
            }
        }
        KeyCode::Char('z') if parts.side || parts.stacked => sh.max = sh.next_max(parts),
        KeyCode::Char('{') if parts.stacked => {
            sh.top = Some(sh.top.unwrap_or(50).saturating_sub(5).max(10))
        }
        KeyCode::Char('}') if parts.stacked => sh.top = Some((sh.top.unwrap_or(50) + 5).min(90)),
        KeyCode::Char('=') if sh != Shape::default() => sh = Shape::default(),
        KeyCode::Char('L') if parts.side || parts.stacked => {
            d.layout_mode = Some(view);
            d.note("layout mode: ↑↓ ←→ resize · 1–4 maximise · p side · 0 reset · esc done");
            return true;
        }
        _ => return false,
    }
    if sh != before {
        keep(d, view, sh);
        d.note(said(&sh, parts));
    }
    true
}

/// Keys while layout mode is on.
fn mode_key(d: &mut Dash, view: &'static str, parts: Parts, k: &KeyEvent) {
    let mut sh = shape(d, view);
    match k.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('L') | KeyCode::Char('q') => {
            d.layout_mode = None;
            d.note("layout kept");
            return;
        }
        KeyCode::Up | KeyCode::Char('k') if parts.stacked => {
            sh.top = Some(sh.top.unwrap_or(50).saturating_sub(5).max(10))
        }
        KeyCode::Down | KeyCode::Char('j') if parts.stacked => {
            sh.top = Some((sh.top.unwrap_or(50) + 5).min(90))
        }
        // ← widens the side, → narrows it (the divider moves that way)
        KeyCode::Left | KeyCode::Char('h') if parts.side => {
            sh.side = Some((sh.side.unwrap_or(40) + 5).min(85));
            sh.hidden = false;
        }
        KeyCode::Right | KeyCode::Char('l') if parts.side => {
            sh.side = Some(sh.side.unwrap_or(40).saturating_sub(5).max(15));
            sh.hidden = false;
        }
        KeyCode::Char('1') => {
            sh.max = if sh.max == Max::Main {
                Max::None
            } else {
                Max::Main
            }
        }
        KeyCode::Char('2') if parts.stacked => {
            sh.max = if sh.max == Max::Top {
                Max::None
            } else {
                Max::Top
            }
        }
        KeyCode::Char('3') if parts.stacked => {
            sh.max = if sh.max == Max::Bottom {
                Max::None
            } else {
                Max::Bottom
            }
        }
        KeyCode::Char('4') if parts.side => {
            sh.max = if sh.max == Max::Side {
                Max::None
            } else {
                Max::Side
            }
        }
        KeyCode::Char('p') if parts.side => sh.hidden = !sh.hidden,
        KeyCode::Char('0') | KeyCode::Char('=') => sh = Shape::default(),
        _ => return,
    }
    keep(d, view, sh);
    d.note(said(&sh, parts));
}

/// The footer while layout mode is on.
pub fn hints() -> Vec<(&'static str, &'static str)> {
    vec![
        ("↑↓", "divider"),
        ("←→", "side width"),
        ("1–4", "maximise"),
        ("p", "side"),
        ("0", "reset"),
        ("esc", "done"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_reads_and_writes_its_setting() {
        let s = Shape::parse("side 35 · top 60 · hidden · max bottom");
        assert_eq!(
            s,
            Shape {
                side: Some(35),
                top: Some(60),
                hidden: true,
                max: Max::Bottom
            }
        );
        assert_eq!(Shape::parse(&s.text()), s);
        assert_eq!(Shape::parse(""), Shape::default());
        assert_eq!(Shape::default().text(), "");
        // out of range: kept sensible
        assert_eq!(Shape::parse("side 99").side, Some(85));
    }

    #[test]
    fn columns_and_rows_honour_hide_maximise_and_sizes() {
        let r = Rect::new(0, 0, 200, 50);
        let own = Shape::default();
        let (m, s) = own.columns(r, 80, 1);
        assert_eq!((m.width, s.width, s.x), (119, 80, 120));
        let hidden = Shape {
            hidden: true,
            ..own
        };
        assert_eq!(hidden.columns(r, 80, 1).1.width, 0);
        let side = Shape {
            side: Some(50),
            ..own
        };
        assert_eq!(side.columns(r, 80, 1).1.width, 100);
        let max = Shape {
            max: Max::Side,
            ..own
        };
        assert_eq!(max.columns(r, 80, 1), (Rect { width: 0, ..r }, r));
        // rows
        let (t, b) = own.rows(r, 20);
        assert_eq!((t.height, b.height, b.y), (20, 30, 20));
        let top = Shape {
            top: Some(70),
            ..own
        };
        assert_eq!(top.rows(r, 20).0.height, 35);
        let only = Shape {
            max: Max::Bottom,
            ..own
        };
        assert_eq!(only.rows(r, 20).1, r);
    }

    #[test]
    fn maximise_turns_through_what_the_view_has() {
        let both = Parts {
            side: true,
            stacked: true,
        };
        let mut s = Shape::default();
        let mut seen = vec![];
        for _ in 0..5 {
            s.max = s.next_max(both);
            seen.push(s.max);
        }
        assert_eq!(
            seen,
            vec![Max::Main, Max::Top, Max::Bottom, Max::Side, Max::None]
        );
        let side_only = Parts {
            side: true,
            stacked: false,
        };
        let s = Shape::default();
        assert_eq!(s.next_max(side_only), Max::Main);
        let s = Shape {
            max: Max::Main,
            ..s
        };
        assert_eq!(s.next_max(side_only), Max::Side);
    }
}
