//! The install screen: Apply's queue, full screen and live. Every item with
//! its state and its time; the one running as a card — what runs, for how
//! long, its progress when its output says (a percentage), the last lines of
//! its output. One that needs you (a passphrase, an installer's own
//! questions) waits for enter, then borrows the terminal. At the end: all of
//! it done → a celebration, and enter opens Aleph; failures → what failed
//! and why, `r` runs them again, enter goes on anyway.

use super::review::{self, Item, ItemSt, What};
use super::{Exit, Wiz};
use crate::ui::{self, c};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::time::{Duration, Instant};

/// Output lines kept per item.
pub(crate) const KEEP: usize = 400;

/// Is the install screen up (Apply ran, and it wasn't put aside)?
pub(crate) fn shown(w: &Wiz) -> bool {
    (w.rv.preparing || !w.rv.items.is_empty()) && !w.rv.hidden
}

fn failed(w: &Wiz) -> usize {
    w.rv.items
        .iter()
        .filter(|i| matches!(i.st, ItemSt::Failed(_)))
        .count()
}

/// The item the card shows: the one picked with ↑↓, else the one running
/// (or waiting for you), else the last that ran.
fn shown_item(w: &Wiz) -> Option<usize> {
    if let Some(i) = w.rv.look.filter(|i| *i < w.rv.items.len()) {
        return Some(i);
    }
    if let Some(i) = w.rv.waiting {
        return Some(i);
    }
    if let Some(i) = w.rv.items.iter().position(|i| i.st == ItemSt::Running) {
        return Some(i);
    }
    if w.rv.done {
        if let Some(i) =
            w.rv.items
                .iter()
                .position(|i| matches!(i.st, ItemSt::Failed(_)))
        {
            return Some(i);
        }
    }
    w.rv.items
        .iter()
        .rposition(|i| !matches!(i.st, ItemSt::Todo))
}

// ------------------------------------------------------------------ output

/// A line of an item's output: escapes out, kept (bounded), and a
/// percentage in it moves the item's bar.
pub(crate) fn line(w: &mut Wiz, i: usize, l: &str) {
    let Some(it) = w.rv.items.get_mut(i) else {
        return;
    };
    let t = review::strip(l).trim_end().to_string();
    if t.trim().is_empty() {
        return;
    }
    if let Some(p) = percent(&t) {
        it.pct = Some(p);
    }
    // a progress bar or a spinner redrawn (\r): the line updates in place
    if it.log.last().is_some_and(|l| redraw(l, &t)) {
        if let Some(l) = it.log.last_mut() {
            *l = t;
        }
        return;
    }
    it.log.push(t.clone());
    if it.log.len() > KEEP {
        let cut = it.log.len() - KEEP;
        it.log.drain(..cut);
    }
    review::log_line(w, i, &t);
}

/// Is `new` the same line redrawn — a progress bar (`#### 45.2%`) after
/// another, or the same words behind a spinner frame (`⠙ Downloading…`)?
pub(crate) fn redraw(old: &str, new: &str) -> bool {
    let bar = |s: &str| {
        let n = s.chars().count().max(1);
        let marks = s
            .chars()
            .filter(|c| {
                matches!(
                    c,
                    '#' | '='
                        | '-'
                        | '>'
                        | '█'
                        | '▉'
                        | '▊'
                        | '▋'
                        | '▌'
                        | '▍'
                        | '▎'
                        | '▏'
                        | '░'
                        | '▒'
                        | ' '
                        | '.'
                )
            })
            .count();
        percent(s).is_some() && marks * 2 >= n
    };
    if bar(old) && bar(new) {
        return true;
    }
    let words = |s: &str| {
        s.trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim()
            .to_string()
    };
    let (a, b) = (words(old), words(new));
    !a.is_empty() && a == b
}

/// The last percentage in a line (`45.2%`, `######## 100.0%`), 0–100.
pub(crate) fn percent(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut best = None;
    for (k, ch) in b.iter().enumerate() {
        if *ch != b'%' {
            continue;
        }
        let mut j = k;
        while j > 0 && (b[j - 1].is_ascii_digit() || b[j - 1] == b'.') {
            j -= 1;
        }
        if j == k {
            continue;
        }
        if let Ok(v) = s[j..k].parse::<f64>() {
            if (0.0..=100.0).contains(&v) {
                best = Some(v / 100.0);
            }
        }
    }
    best
}

// ------------------------------------------------------------------ keys

pub(crate) fn key(w: &mut Wiz, k: KeyEvent) -> Option<Exit> {
    let n = w.rv.items.len();
    if n == 0 {
        // quick start is still looking: esc gives up on it
        if k.code == KeyCode::Esc {
            w.quick = false;
            w.rv.preparing = false;
        }
        return None;
    }
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            let cur = shown_item(w).unwrap_or(0);
            w.rv.look = Some(cur.saturating_sub(1));
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let cur = shown_item(w).unwrap_or(0);
            w.rv.look = Some((cur + 1).min(n.saturating_sub(1)));
        }
        KeyCode::Char('f') => w.rv.look = None,
        KeyCode::Char('l') => w.rv.full_log = !w.rv.full_log,
        KeyCode::Char('r') if w.rv.done && failed(w) > 0 => review::retry_failed(w),
        // just the one shown, again
        KeyCode::Char('i')
            if shown_item(w).is_some_and(|k| {
                matches!(w.rv.items[k].st, ItemSt::Failed(_) | ItemSt::Skipped(_))
            }) =>
        {
            if let Some(k) = shown_item(w) {
                review::retry_one(w, k);
                w.rv.look = Some(k);
            }
        }
        // stop the agent install shown (a slow one you'd rather run later)
        KeyCode::Char('x') => {
            if let Some(k) = shown_item(w) {
                if review::stop_one(w, k) {
                    line(w, k, "■ stopping it, and what it started…");
                }
            }
        }
        KeyCode::Enter if w.rv.done => {
            return Some(Exit::Dashboard {
                tour: review::tour_due(w),
            })
        }
        KeyCode::Enter if w.rv.waiting.is_some() => w.rv.go = true,
        KeyCode::Esc => {
            w.rv.hidden = true;
            w.rv.look = None;
        }
        KeyCode::Char('q') if w.rv.done => return Some(Exit::Quit),
        KeyCode::Char('q') => w.ask_quit(),
        _ => {}
    }
    None
}

// ------------------------------------------------------------------ drawing

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("0:{s:02}")
    } else if s < 3600 {
        format!("{}:{:02}", s / 60, s % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    }
}

fn took(it: &Item) -> String {
    match (it.took, it.t0) {
        (Some(d), _) if d < Duration::from_secs(1) => format!("{}ms", d.as_millis()),
        (Some(d), _) if d < Duration::from_secs(60) => format!("{:.1}s", d.as_secs_f64()),
        (Some(d), _) => clock(d),
        (None, Some(t)) if it.st == ItemSt::Running => clock(t.elapsed()),
        _ => String::new(),
    }
}

fn icon(w: &Wiz, i: usize, it: &Item) -> Span<'static> {
    if w.rv.waiting == Some(i) {
        let on = (w.t0.elapsed().as_millis() / 500).is_multiple_of(2);
        return Span::styled(
            "◆",
            ui::fg(if on { c::ACCENT() } else { c::PEACH() }).add_modifier(Modifier::BOLD),
        );
    }
    match &it.st {
        ItemSt::Todo => Span::styled("○", ui::faint()),
        ItemSt::Running => Span::styled(
            ui::spinner(it.t0.unwrap_or(w.t0)).to_string(),
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ),
        ItemSt::Ok(_) => Span::styled("✓", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD)),
        ItemSt::Failed(_) => Span::styled("✗", ui::fg(c::RED()).add_modifier(Modifier::BOLD)),
        ItemSt::Skipped(_) => Span::styled("↷", ui::faint()),
    }
}

/// How far the whole queue is (the running item's own bar counts in).
pub(crate) fn progress(items: &[Item]) -> f64 {
    if items.is_empty() {
        return 0.0;
    }
    let mut done = 0.0;
    for it in items {
        done += match &it.st {
            ItemSt::Todo => 0.0,
            ItemSt::Running => it.pct.unwrap_or(0.0).min(0.95),
            _ => 1.0,
        };
    }
    done / items.len() as f64
}

/// What an item runs, in a line.
fn runs(it: &Item) -> String {
    match &it.what {
        What::Settings(..) => "writes aleph.toml in one go (aleph config undo steps back)".into(),
        What::Act(a) => review::runs_line(a),
    }
}

/// Quick start, before the queue: what it's looking at.
fn preparing(w: &Wiz, f: &mut Frame, area: Rect) {
    let mut lines = vec![
        Line::from(Span::styled(
            format!(" {} aleph setup · quick start", crate::brand::MARK),
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                format!(" {} ", ui::spinner(w.t0)),
                ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Looking at this computer: what's there, what's missing",
                ui::bold(),
            ),
        ]),
        Line::raw(""),
    ];
    let mut jobs: Vec<&super::Job> = w.jobs.values().collect();
    jobs.sort_by_key(|j| j.t0);
    for j in jobs {
        lines.push(Line::from(vec![
            Span::styled("   · ", ui::faint()),
            Span::styled(j.label.clone(), ui::muted()),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

pub(crate) fn draw(w: &mut Wiz, f: &mut Frame, area: Rect) {
    if w.rv.items.is_empty() {
        return preparing(w, f, area);
    }
    let [head, _, bar, _, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(area);
    let n = w.rv.items.len();
    let ok =
        w.rv.items
            .iter()
            .filter(|i| matches!(i.st, ItemSt::Ok(_)))
            .count();
    let bad = failed(w);
    let ended =
        w.rv.ended
            .unwrap_or_else(|| w.rv.t0.map(|t| t.elapsed()).unwrap_or_default());
    // the header
    let (word, col) = if !w.rv.done {
        ("installing", c::ACCENT())
    } else if bad == 0 {
        ("all set", c::GREEN())
    } else {
        ("done, with some to look at", c::YELLOW())
    };
    let left = Line::from(vec![
        Span::styled(
            format!(" {} aleph setup", crate::brand::MARK),
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", ui::faint()),
        Span::styled(word, ui::fg(col).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(left), head);
    let right = format!("{} ", clock(ended));
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(right, ui::muted()))).alignment(Alignment::Right),
        head,
    );
    // the bar
    let frac = if w.rv.done {
        1.0
    } else {
        progress(&w.rv.items)
    };
    let skipped =
        w.rv.items
            .iter()
            .filter(|i| matches!(i.st, ItemSt::Skipped(_)))
            .count();
    let tail = if w.rv.done {
        let mut t = format!("  {ok} done");
        if bad > 0 {
            t.push_str(&format!(" · {bad} failed"));
        }
        if skipped > 0 {
            t.push_str(&format!(" · {skipped} skipped"));
        }
        t
    } else {
        format!(
            "  {}/{} · {:>3}%",
            ok + bad + skipped,
            n,
            (frac * 100.0).round() as u32
        )
    };
    let bw = (bar.width as usize).saturating_sub(ui::width(&tail) + 2);
    let mut spans = vec![Span::raw(" ")];
    spans.extend(ui::bar(
        frac,
        bw,
        if bad > 0 {
            c::YELLOW()
        } else if w.rv.done {
            c::GREEN()
        } else {
            c::ACCENT()
        },
    ));
    spans.push(Span::styled(tail, ui::muted()));
    f.render_widget(Paragraph::new(Line::from(spans)), bar);
    // the body
    if w.rv.done && bad == 0 && !w.rv.full_log && w.rv.look.is_none() {
        celebrate(w, f, body, ok, ended);
    } else if w.rv.full_log {
        log_panel(w, f, body);
    } else if body.width >= 84 {
        let qw = (body.width * 2 / 5).clamp(30, 46);
        let [q, _, card] = Layout::horizontal([
            Constraint::Length(qw),
            Constraint::Length(1),
            Constraint::Min(30),
        ])
        .areas(body);
        queue(w, f, q);
        self::card(w, f, card);
    } else {
        let qh = (n as u16 + 2).min(body.height / 2).max(3);
        let [q, card] = Layout::vertical([Constraint::Length(qh), Constraint::Min(3)]).areas(body);
        queue(w, f, q);
        self::card(w, f, card);
    }
    // the keys
    let keys: Vec<(&str, &str)> = if w.rv.done && bad == 0 {
        vec![
            ("enter", "open aleph"),
            ("↑↓", "what ran"),
            ("esc", "back to setup"),
            ("q", "quit"),
        ]
    } else if w.rv.done {
        vec![
            ("i", "retry this one"),
            ("r", "retry the failed"),
            ("enter", "open aleph anyway"),
            ("↑↓", "items"),
            ("l", "full log"),
            ("esc", "back to setup"),
        ]
    } else if w.rv.waiting.is_some() {
        vec![
            ("enter", "hand over the terminal"),
            ("↑↓", "items"),
            ("l", "full log"),
            ("esc", "setup"),
        ]
    } else {
        let mut v = vec![("↑↓", "items"), ("f", "follow")];
        // an agent install shown, running: it can be stopped
        if shown_item(w).is_some_and(|k| {
            w.rv.items[k].st == ItemSt::Running
                && matches!(
                    w.rv.items[k].what,
                    review::What::Act(super::Action::AgentInstall { .. })
                )
        }) {
            v.push(("x", "stop this one"));
        }
        v.extend([("l", "full log"), ("esc", "back to setup"), ("q", "leave")]);
        v
    };
    f.render_widget(Paragraph::new(ui::hints_fit(&keys, foot.width)), foot);
}

fn queue(w: &Wiz, f: &mut Frame, r: Rect) {
    let block = ui::panel("Queue", false);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let at = shown_item(w);
    let h = inner.height as usize;
    let n = w.rv.items.len();
    // keep the one shown in view
    let top = at
        .map(|a| a.saturating_sub(h.saturating_sub(2)))
        .unwrap_or(0)
        .min(n.saturating_sub(h));
    let tw = inner.width as usize;
    let mut lines = Vec::new();
    for (i, it) in w.rv.items.iter().enumerate().skip(top).take(h) {
        let t = took(it);
        let lw = tw.saturating_sub(ui::width(&t) + 5);
        let sel = at == Some(i);
        let label_st = match &it.st {
            ItemSt::Todo | ItemSt::Skipped(_) => ui::muted(),
            ItemSt::Failed(_) => ui::fg(c::RED()),
            _ => Style::new().fg(c::TEXT()),
        };
        let mut l = vec![
            Span::styled(
                if sel { "┃" } else { " " },
                ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            ),
            icon(w, i, it),
            Span::raw(" "),
            Span::styled(
                ui::pad(&ui::trunc(&queue_label(&it.label), lw), lw),
                if sel {
                    label_st.add_modifier(Modifier::BOLD)
                } else {
                    label_st
                },
            ),
            Span::raw(" "),
        ];
        l.push(Span::styled(t, ui::faint()));
        lines.push(Line::from(l));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn card(w: &Wiz, f: &mut Frame, r: Rect) {
    let Some(i) = shown_item(w) else {
        return;
    };
    let it = &w.rv.items[i];
    let block = ui::panel(it.label.clone(), true);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let tw = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    // what it is now
    let state: Line = if w.rv.waiting == Some(i) {
        Line::from(vec![
            Span::styled(
                "◆ this one needs you",
                ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" — it asks its own questions, in the terminal", ui::muted()),
        ])
    } else {
        match &it.st {
            ItemSt::Todo => Line::from(Span::styled("○ waiting its turn", ui::muted())),
            ItemSt::Running => Line::from(vec![
                Span::styled(
                    format!("{} running", ui::spinner(it.t0.unwrap_or(w.t0))),
                    ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" · {}", took(it)), ui::muted()),
            ]),
            ItemSt::Ok(m) => Line::from(vec![
                Span::styled("✓ done", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" in {} · ", took(it)), ui::muted()),
                Span::styled(m.clone(), Style::new().fg(c::TEXT())),
            ]),
            ItemSt::Failed(_) => Line::from(vec![
                Span::styled("✗ failed", ui::fg(c::RED()).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" after {}", took(it)), ui::muted()),
            ]),
            ItemSt::Skipped(m) => Line::from(vec![
                Span::styled("↷ skipped", ui::muted().add_modifier(Modifier::BOLD)),
                Span::styled(format!(" · {m}"), ui::muted()),
            ]),
        }
    };
    lines.push(state);
    for (k, t) in ui::wrap(&runs(it), tw.saturating_sub(7))
        .into_iter()
        .enumerate()
    {
        lines.push(Line::from(vec![
            Span::styled(if k == 0 { "runs   " } else { "       " }, ui::faint()),
            Span::styled(t, ui::muted()),
        ]));
    }
    if let ItemSt::Failed(e) = &it.st {
        lines.push(Line::raw(""));
        for t in ui::wrap(e, tw) {
            lines.push(Line::from(Span::styled(t, ui::fg(c::RED()))));
        }
        lines.push(Line::from(vec![
            ui::keycap("i"),
            Span::styled(" runs just this one again   ", ui::muted()),
            ui::keycap("r"),
            Span::styled(" every failed one", ui::muted()),
        ]));
    }
    if w.rv.waiting == Some(i) {
        lines.push(Line::raw(""));
        for t in ui::wrap(&review::why_tty(it), tw) {
            lines.push(Line::from(Span::styled(t, Style::new().fg(c::TEXT()))));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            ui::keycap("enter"),
            Span::styled(
                " hand over the terminal — setup comes back when it's done",
                ui::muted(),
            ),
        ]));
    }
    // its progress
    if it.st == ItemSt::Running {
        lines.push(Line::raw(""));
        let bw = tw.saturating_sub(8).max(4);
        let mut s = vec![Span::raw("")];
        match it.pct {
            Some(p) => {
                s.extend(ui::bar(p, bw, c::ACCENT()));
                s.push(Span::styled(
                    format!(" {:>3}%", (p * 100.0).round() as u32),
                    ui::muted(),
                ));
            }
            None => s.extend(sweep(it.t0.unwrap_or(w.t0), bw)),
        }
        lines.push(Line::from(s));
    }
    // its output
    let room = (inner.height as usize).saturating_sub(lines.len() + 2);
    if !it.log.is_empty() && room > 0 {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("output ", ui::faint()),
            Span::styled("─".repeat(tw.saturating_sub(7)), ui::faint()),
        ]));
        let tail: Vec<&String> = it.log.iter().rev().take(room).collect();
        let k = tail.len();
        for (j, l) in tail.into_iter().rev().enumerate() {
            // the newest brightest
            let st = if j + 1 == k && it.st == ItemSt::Running {
                Style::new().fg(c::TEXT())
            } else {
                ui::muted()
            };
            lines.push(Line::from(Span::styled(ui::trunc(l, tw), st)));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// An indeterminate bar: a short gold run sliding along the track.
fn sweep(since: Instant, w: usize) -> Vec<Span<'static>> {
    let run = (w / 6).clamp(3, 12);
    let span = w + run;
    let at = (since.elapsed().as_millis() / 40) as usize % span.max(1);
    let start = at.saturating_sub(run);
    let end = at.min(w);
    let track = if c::mono() { " " } else { "·" };
    vec![
        Span::styled(track.repeat(start), Style::new().fg(c::LINE())),
        Span::styled("━".repeat(end.saturating_sub(start)), ui::fg(c::ACCENT())),
        Span::styled(
            track.repeat(w.saturating_sub(end)),
            Style::new().fg(c::LINE()),
        ),
    ]
}

fn log_panel(w: &Wiz, f: &mut Frame, r: Rect) {
    let Some(i) = shown_item(w) else {
        return;
    };
    let it = &w.rv.items[i];
    let block = ui::panel(format!("{} · output", it.label), true);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let h = inner.height as usize;
    let lines: Vec<Line> = if it.log.is_empty() {
        vec![Line::from(Span::styled("no output (yet)", ui::faint()))]
    } else {
        it.log
            .iter()
            .skip(it.log.len().saturating_sub(h))
            .map(|l| {
                Line::from(Span::styled(
                    ui::trunc(l, inner.width as usize),
                    ui::muted(),
                ))
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), inner);
}

/// Everything went in: the ℵ drawn in light, sparks around it, what's in.
fn celebrate(w: &Wiz, f: &mut Frame, r: Rect, ok: usize, took: Duration) {
    let t =
        w.rv.celebrate
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(10_000);
    let motion = !c::mono()
        && crate::tui::motion::Motion::parse(&w.cfg.get_or("ui.motion"))
            == crate::tui::motion::Motion::On;
    let t = if motion { t } else { 10_000 };
    // the mark: as big as fits above the words
    let words_h = 7u16;
    let mh = r.height.saturating_sub(words_h + 1).min(14);
    let mw = (mh * 2 + 2).min(r.width);
    let buf = f.buffer_mut();
    if mh >= 4 {
        let x0 = r.x + (r.width.saturating_sub(mw)) / 2;
        let y0 = r.y + 1;
        let cells = crate::logo::mark_dots(mw as usize, mh as usize);
        let total = cells.len().max(1) as u64;
        for (k, (cx, cy, dots)) in cells.iter().enumerate() {
            // dots arrive over 900 ms, in a scattered order
            let order = hash(k as u64 * 7919) % total;
            if order * 900 / total > t {
                continue;
            }
            let (x, y) = (x0 + *cx as u16, y0 + *cy as u16);
            if x >= r.right() || y >= r.bottom() {
                continue;
            }
            let ch = char::from_u32(0x2800 + *dots as u32).unwrap_or('⣿');
            // a band of light passes over it once it's whole
            let band = t > 900 && ((t - 900) / 30) as i64 - (*cx as i64 + *cy as i64 * 2) == 0;
            let col = if band { c::TEXT() } else { c::ACCENT() };
            buf[(x, y)].set_char(ch).set_fg(col);
        }
        // sparks: twinkling round the mark, a few, never on it
        let n = if motion { 18 } else { 0 };
        for s in 0..n {
            let h = hash(s * 104_729 + 17);
            let x = r.x + (h % r.width.max(1) as u64) as u16;
            let y = r.y + ((h >> 16) % (mh as u64 + 2)) as u16;
            if x >= x0.saturating_sub(1) && x < x0 + mw + 1 && y >= y0 && y < y0 + mh {
                continue;
            }
            let phase = (t / 90 + (h >> 32) % 13) % 13;
            let ch = match phase {
                0 | 12 => '·',
                1 | 11 => '✦',
                2..=10 => {
                    if phase.is_multiple_of(3) {
                        '✶'
                    } else {
                        '·'
                    }
                }
                _ => ' ',
            };
            if x < r.right() && y < r.bottom() && buf[(x, y)].symbol() == " " {
                let col =
                    [c::ACCENT(), c::PEACH(), c::PINK(), c::SKY(), c::GREEN()][(h % 5) as usize];
                buf[(x, y)].set_char(ch).set_fg(col);
            }
        }
    }
    // the words
    let wy = r.y + if mh >= 4 { mh + 2 } else { 1 };
    let wa = Rect::new(
        r.x,
        wy.min(r.bottom().saturating_sub(1)),
        r.width,
        r.bottom().saturating_sub(wy),
    );
    let names: Vec<String> =
        w.rv.items
            .iter()
            .filter(|i| matches!(i.st, ItemSt::Ok(_)))
            .map(|i| {
                short(&queue_label(&i.label))
                    .trim_start_matches("run ")
                    .to_string()
            })
            .collect();
    // what went in, over two lines at most
    let room = (r.width as usize).saturating_sub(8).max(20);
    let mut rows: Vec<Vec<Span>> = vec![Vec::new()];
    let mut used = 0usize;
    let mut left = names.len();
    for nm in &names {
        let wd = ui::width(nm) + 4;
        if used + wd > room {
            if rows.len() == 2 {
                rows[1].push(Span::styled(format!("+{left} more"), ui::faint()));
                break;
            }
            rows.push(Vec::new());
            used = 0;
        }
        let row = rows.last_mut().expect("a row");
        row.push(Span::styled(format!("✓ {nm}"), ui::fg(c::GREEN())));
        row.push(Span::raw("  "));
        used += wd;
        left -= 1;
    }
    let lines = vec![
        Line::from(Span::styled(
            "You're all set.",
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "{ok} thing{} done in {} — Aleph is ready.",
                if ok == 1 { "" } else { "s" },
                clock(took)
            ),
            Style::new().fg(c::TEXT()),
        )),
        Line::raw(""),
    ];
    let mut lines = lines;
    lines.extend(rows.into_iter().map(Line::from));
    lines.extend([
        Line::raw(""),
        Line::from(vec![
            ui::keycap("enter"),
            Span::styled(" open aleph   ", ui::muted()),
            ui::keycap("q"),
            Span::styled(" quit", ui::muted()),
        ]),
    ]);
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), wa);
}

/// The queue's words for an item: no "install", "on local" or what's in
/// brackets ("herdr's opencode integration", "Claude Code on gpu2").
fn queue_label(label: &str) -> String {
    let s = label.trim_start_matches("install ");
    let s = s.split(" (").next().unwrap_or(s);
    s.strip_suffix(" on local")
        .or_else(|| s.strip_suffix(" on this computer"))
        .unwrap_or(s)
        .to_string()
}

/// A label without its verb and place: "install Claude Code on local" →
/// "Claude Code".
fn short(label: &str) -> String {
    let s = label
        .trim_start_matches("install ")
        .trim_start_matches("write ");
    let s = s.split(" on ").next().unwrap_or(s);
    let s = s.split(" (").next().unwrap_or(s);
    ui::trunc(s, 28)
}

fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_are_read_off_installers_output() {
        assert_eq!(percent("######## 45.2%"), Some(0.452));
        assert_eq!(percent("Downloading 3% then 70%"), Some(0.7));
        assert_eq!(percent("no number %"), None);
        assert_eq!(percent("disk 140% full"), None);
        assert_eq!(percent("plain text"), None);
    }

    #[test]
    fn redrawn_bars_and_spinners_update_in_place() {
        assert!(redraw("########         22.4%", "#########        22.5%"));
        assert!(redraw(
            "⠙ Downloading release package...",
            "⠹ Downloading release package..."
        ));
        assert!(!redraw(
            "✓ Package downloaded",
            "▸ Finalizing installation..."
        ));
        assert!(!redraw("added 5 packages in 7s", "########  40%"));
    }

    #[test]
    fn the_queue_names_what_it_installs() {
        assert_eq!(
            queue_label("install JetBrains Mono Nerd Font"),
            "JetBrains Mono Nerd Font"
        );
        assert_eq!(
            queue_label("herdr's opencode integration on local"),
            "herdr's opencode integration"
        );
        assert_eq!(
            queue_label("install Claude Code on gpu2"),
            "Claude Code on gpu2"
        );
        assert_eq!(
            queue_label("run the daemon at login (aleph watch --install)"),
            "run the daemon at login"
        );
        assert_eq!(queue_label("install herdr on this computer"), "herdr");
    }

    #[test]
    fn labels_shorten_to_what_they_install() {
        assert_eq!(short("install Claude Code on local"), "Claude Code");
        assert_eq!(
            short("write 3 settings and 1 preset"),
            "3 settings and 1 preset"
        );
        assert_eq!(
            short("herdr's codex integration on gpu2"),
            "herdr's codex integration"
        );
    }
}
