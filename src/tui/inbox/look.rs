//! The Inbox's visual pieces: state pills, the KPI tiles, the 24-hour
//! activity chart, and the session cards layout.

use super::*;
use ratatui::widgets::Block;

/// ` ✓ finished ` on its colour, in dark ink (reversed without colour).
pub(super) fn pill(text: &str, col: Color) -> Span<'static> {
    let st = if ui::c::mono() {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new()
            .bg(col)
            .fg(c::BASE())
            .add_modifier(Modifier::BOLD)
    };
    Span::styled(format!(" {text} "), st)
}

/// Events per hour over the last 24 hours (oldest first), for `keep`.
fn hourly(d: &Dash, keep: impl Fn(Kind) -> bool) -> Vec<f64> {
    let now = crate::cache::now();
    let mut v = vec![0.0; 24];
    for e in d
        .inbox
        .rows
        .iter()
        .take_while(|e| now.saturating_sub(e.t) < 86_400)
    {
        if keep(e.kind) {
            let h = (now.saturating_sub(e.t) / 3600) as usize;
            if h < 24 {
                v[23 - h] += 1.0;
            }
        }
    }
    v
}

fn norm(v: &[f64]) -> Vec<f64> {
    let m = v.iter().cloned().fold(0.0, f64::max);
    if m == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / m).collect()
}

/// One KPI: (title, glyph, colour, the number, a word after it, its kinds
/// per hour, the filter chip a click picks).
struct Tile {
    title: &'static str,
    glyph: &'static str,
    col: Color,
    n: usize,
    unit: String,
    spark: Vec<f64>,
    show: Option<Show>,
}

fn tiles_of(d: &Dash) -> Vec<Tile> {
    let now = crate::cache::now();
    let today = |k: Kind| {
        d.inbox
            .rows
            .iter()
            .filter(|e| e.kind == k && ui::local_day(e.t) == ui::local_day(now))
            .count()
    };
    let sessions = || d.fleet.hosts.iter().flat_map(|h| h.sessions.iter());
    let blocked = sessions().filter(|s| s.blocked()).count();
    let working = sessions().filter(|s| s.status == "working").count();
    let limit = sessions().filter(|s| s.limit.is_some()).count();
    let down = d
        .fleet
        .hosts
        .iter()
        .filter(|h| matches!(&h.conn, Conn::Down(x) if !crate::fleet::not_running(x)))
        .count();
    vec![
        Tile {
            title: "Needs you",
            glyph: crate::look::needs(),
            col: c::NEEDS(),
            n: blocked,
            unit: format!("now · {} today", today(Kind::Blocked)),
            spark: hourly(d, |k| k == Kind::Blocked),
            show: Some(Show::Attention),
        },
        Tile {
            title: "Working",
            glyph: "●",
            col: c::WORKING(),
            n: working,
            unit: format!("now · {} started today", today(Kind::New)),
            spark: hourly(d, |k| k == Kind::New),
            show: None,
        },
        Tile {
            title: "Finished",
            glyph: "✓",
            col: c::DONE(),
            n: today(Kind::Done),
            unit: "today".into(),
            spark: hourly(d, |k| k == Kind::Done),
            show: Some(Show::Finished),
        },
        Tile {
            title: "Problems",
            glyph: "✕",
            col: c::RED(),
            n: down,
            unit: format!(
                "machine{} down · {} today",
                if down == 1 { "" } else { "s" },
                today(Kind::Error)
            ),
            spark: hourly(d, |k| k == Kind::Error),
            show: Some(Show::Problems),
        },
        Tile {
            title: "At limit",
            glyph: "◷",
            col: c::LIMIT(),
            n: limit,
            unit: format!("now · {} today", today(Kind::Limit)),
            spark: hourly(d, |k| k == Kind::Limit),
            show: None,
        },
    ]
}

/// The KPI tiles across `r` (4 rows). A click on one picks its filter.
pub(super) fn tiles(d: &mut Dash, f: &mut Frame, r: Rect) {
    let ts = tiles_of(d);
    let n = ts.len() as u16;
    let gap = 1;
    let w = (r.width.saturating_sub(gap * (n - 1))) / n;
    if w < 12 {
        return;
    }
    for (i, t) in ts.iter().enumerate() {
        let x = r.x + i as u16 * (w + gap);
        let tw = if i as u16 == n - 1 { r.right() - x } else { w };
        let area = Rect::new(x, r.y, tw, r.height.min(4));
        let lit = t.n > 0;
        let block = Block::bordered()
            .border_set(crate::ui::border_set(false))
            .border_style(ui::fg(if lit { t.col } else { c::LINE() }))
            .title(Span::styled(
                format!(" {} {} ", t.glyph, t.title),
                ui::fg(t.col).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let iw = inner.width as usize;
        let num = Line::from(vec![
            Span::styled(
                format!(" {}", t.n),
                if lit {
                    ui::fg(t.col).add_modifier(Modifier::BOLD)
                } else {
                    ui::bold()
                },
            ),
            Span::styled(
                format!(" {}", ui::trunc(&t.unit, iw.saturating_sub(4))),
                ui::muted(),
            ),
        ]);
        // the 24 hours stretched across the tile
        let sw = iw.saturating_sub(2).max(1);
        let n = norm(&t.spark);
        let wide: Vec<f64> = (0..sw).map(|x| n[x * n.len() / sw]).collect();
        let spark = Line::from(vec![Span::raw(" "), ui::spark(&wide, sw, t.col)]);
        f.render_widget(Paragraph::new(vec![num, spark]), inner);
        if let Some(s) = t.show {
            if let Some(ix) = Show::ALL.iter().position(|x| *x == s) {
                d.hits.chips.push((area, super::super::Chip::Inbox(ix)));
            }
        }
    }
}

/// A stacked series: (colour, glyph, label, which kinds it counts).
type Series = (Color, &'static str, &'static str, fn(Kind) -> bool);

/// The last 24 hours as stacked bars (asked, finished, started, problems,
/// the rest), with an axis and a legend, in a panel.
pub(super) fn activity(d: &Dash, f: &mut Frame, r: Rect) {
    let block = ui::panel(
        Line::from(vec![
            Span::raw("Activity "),
            Span::styled("last 24 hours", ui::muted()),
        ]),
        false,
    );
    let inner = block.inner(r);
    f.render_widget(block, r);
    if inner.height < 2 || inner.width < 30 {
        return;
    }
    let series: [Series; 5] = [
        (c::NEEDS(), crate::look::needs(), "asked", |k| {
            k == Kind::Blocked
        }),
        (c::DONE(), "✓", "finished", |k| k == Kind::Done),
        (c::WORKING(), "+", "started", |k| k == Kind::New),
        (c::RED(), "✕", "problems", |k| {
            matches!(k, Kind::Error | Kind::Limit)
        }),
        (c::MUTED(), "·", "the rest", |k| {
            !matches!(
                k,
                Kind::Blocked | Kind::Done | Kind::New | Kind::Error | Kind::Limit
            )
        }),
    ];
    let per: Vec<Vec<f64>> = series.iter().map(|s| hourly(d, s.3)).collect();
    let totals: Vec<f64> = (0..24).map(|h| per.iter().map(|v| v[h]).sum()).collect();
    let max = totals.iter().cloned().fold(0.0, f64::max).max(1.0);
    let rows = (inner.height as usize).saturating_sub(1).max(1);
    let bw = ((inner.width as usize).saturating_sub(1) / 24).max(1);
    let mut lines: Vec<Line> = Vec::new();
    for row in 0..rows {
        // row 0 is the top; each row holds 8 eighths of the scale
        let floor = (rows - 1 - row) as f64 * 8.0;
        let mut l: Vec<Span> = Vec::new();
        for h in 0..24 {
            let scale = rows as f64 * 8.0 / max;
            // the series stacked from the bottom: which one this cell shows
            let mut acc = 0.0;
            let mut cell = (" ", c::LINE());
            for (s, counts) in series.iter().zip(&per) {
                let lo = acc;
                acc += counts[h] * scale;
                if acc > floor && lo < floor + 8.0 {
                    let top = acc.min(floor + 8.0) - floor;
                    let g = if top >= 8.0 {
                        "█"
                    } else {
                        ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"][(top.max(1.0) as usize - 1).min(7)]
                    };
                    cell = (g, s.0);
                }
            }
            let mut t = cell.0.repeat(bw.saturating_sub(1));
            t.push(' ');
            l.push(Span::styled(t, ui::fg(cell.1)));
        }
        lines.push(Line::from(l));
    }
    // the axis and the legend on one line
    let mut legend: Vec<Span> = vec![Span::styled("24h ago", ui::faint())];
    for (col, g, label, _) in &series {
        legend.push(Span::raw("   "));
        legend.push(Span::styled(*g, ui::fg(*col).add_modifier(Modifier::BOLD)));
        legend.push(Span::styled(format!(" {label}"), ui::muted()));
    }
    let used: usize = legend.iter().map(|s| ui::width(&s.content)).sum();
    legend.push(Span::raw(" ".repeat((bw * 24).saturating_sub(used + 3))));
    legend.push(Span::styled("now", ui::faint()));
    lines.push(Line::from(legend));
    f.render_widget(Paragraph::new(lines), inner);
}

/// The cards layout's items: every live session (its newest alert, or what
/// it's doing), then the sessions that ended (one each, newest first).
pub(super) fn card_items(d: &Dash) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let tl = timeline(d);
    for h in &d.fleet.hosts {
        if d.inbox.host.as_deref().is_some_and(|x| x != h.alias) {
            continue;
        }
        for s in &h.sessions {
            if d.inbox.agent.as_deref().is_some_and(|a| a != s.agent) {
                continue;
            }
            let newest = tl
                .iter()
                .find(|it| it.e.host == h.alias && it.e.name == s.name)
                .cloned();
            let mut it = newest.unwrap_or_else(|| Item {
                e: Event {
                    t: s.since.unwrap_or_else(crate::cache::now),
                    host: h.alias.clone(),
                    name: s.name.clone(),
                    kind: Kind::New,
                    title: s.title.clone(),
                    agent: s.agent.clone(),
                    ..Default::default()
                },
                n: 1,
                first: 0,
                now: false,
                launch: None,
                logged: false,
                life: Life::Checking,
                group: String::new(),
            });
            it.life = life(d, &it.e);
            it.group = "live".into();
            if matches_text(
                d,
                &format!("{} {} {} {}", s.name, s.agent, s.title, s.cwd),
                &h.alias,
                &s.name,
            ) {
                out.push(it);
            }
        }
    }
    // what needs you first, then working, …
    out.sort_by_key(|i| (i.life.rank(), std::cmp::Reverse(i.e.t)));
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for it in tl {
        if it.life != Life::Ended || it.e.name.is_empty() {
            continue;
        }
        if seen.insert((it.e.host.clone(), it.e.name.clone())) {
            let mut it = it;
            it.group = "ended".into();
            out.push(it);
        }
    }
    out
}

/// How many cards fit across `w`.
pub(super) fn card_cols(w: u16) -> usize {
    ((w as usize + 1) / 46).max(1)
}

/// The cards: live sessions as cards in a grid, the ended ones as a shelf.
pub(super) fn cards(d: &mut Dash, f: &mut Frame, r: Rect, its: &[Item]) {
    let live: Vec<usize> = (0..its.len()).filter(|i| its[*i].group == "live").collect();
    let ended: Vec<usize> = (0..its.len())
        .filter(|i| its[*i].group == "ended")
        .collect();
    let cols = card_cols(r.width);
    d.inbox.card_cols = cols;
    let ch = 7u16;
    let cw = (r.width + 1) / cols as u16 - 1;
    // keep the selected card in view
    let sel_row = if d.inbox.sel < live.len() {
        d.inbox.sel / cols
    } else {
        0
    };
    let shelf_h = if ended.is_empty() { 0 } else { 4u16 };
    let fit = ((r.height.saturating_sub(shelf_h + 1)) / ch).max(1) as usize;
    if sel_row < d.inbox.scroll {
        d.inbox.scroll = sel_row;
    } else if sel_row >= d.inbox.scroll + fit {
        d.inbox.scroll = sel_row + 1 - fit;
    }
    let mut y = r.y;
    f.render_widget(
        Paragraph::new(rule(
            vec![Span::styled(
                format!("Live now · {}", live.len()),
                ui::fg(c::GREEN()).add_modifier(Modifier::BOLD),
            )],
            vec![],
            r.width as usize,
        )),
        Rect::new(r.x, y, r.width, 1),
    );
    y += 1;
    if live.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  nothing runs right now — n starts a session",
                ui::muted(),
            ))),
            Rect::new(r.x, y, r.width, 1),
        );
        y += 2;
    }
    for (k, &i) in live.iter().enumerate().skip(d.inbox.scroll * cols) {
        let (row, col) = (k / cols - d.inbox.scroll, k % cols);
        let cy = y + row as u16 * ch;
        if cy + ch > r.bottom().saturating_sub(shelf_h) {
            break;
        }
        let area = Rect::new(r.x + col as u16 * (cw + 1), cy, cw, ch - 1);
        card(d, f, area, &its[i], i == d.inbox.sel);
    }
    if shelf_h > 0 {
        let sy = r.bottom() - shelf_h;
        let mut lines = vec![rule(
            vec![Span::styled(
                format!("Ended · {}", ended.len()),
                ui::muted().add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled("A the archive", ui::faint())],
            r.width as usize,
        )];
        let mut cur: Vec<Span> = vec![Span::raw(" ")];
        let mut cwid = 1;
        for &i in &ended {
            let it = &its[i];
            let t = format!(
                "○ {} · {} {} ",
                it.e.name,
                it.e.host,
                when(it.e.t, crate::cache::now(), false)
            );
            let tw = ui::width(&t) + 2;
            if cwid + tw > r.width as usize {
                lines.push(Line::from(std::mem::replace(
                    &mut cur,
                    vec![Span::raw(" ")],
                )));
                cwid = 1;
                if lines.len() >= shelf_h as usize {
                    break;
                }
            }
            cur.push(Span::styled(
                t,
                if i == d.inbox.sel {
                    ui::selected().fg(c::ACCENT()).add_modifier(Modifier::BOLD)
                } else {
                    ui::muted()
                },
            ));
            cur.push(Span::raw("  "));
            cwid += tw;
        }
        lines.push(Line::from(cur));
        lines.truncate(shelf_h as usize);
        f.render_widget(Paragraph::new(lines), Rect::new(r.x, sy, r.width, shelf_h));
    }
}

/// One live session as a card: its logo, name and agent, its state pill;
/// machine and folder; what it's doing; its last alerts.
fn card(d: &Dash, f: &mut Frame, area: Rect, it: &Item, selected: bool) {
    let e = &it.e;
    let s = live_session(d, e);
    let agent = agent_of(d, e);
    let (g, label, col, _) = it.life.look(d.t0);
    let scol = s.as_ref().map(|s| s.color()).unwrap_or(c::TEXT());
    let mut title = vec![
        Span::raw(" "),
        logo(d, &agent, it.life),
        Span::styled(
            format!(" {} ", e.name),
            ui::fg(scol).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("· {} ", short_agent(&agent)), ui::muted()),
    ];
    if crate::notes::get(&e.host, &e.name).is_some() {
        title.push(Span::styled("✎ ", ui::fg(c::ACCENT())));
    }
    let block = Block::bordered()
        .border_set(crate::ui::border_set(false))
        .border_style(ui::fg(if selected { c::ACCENT() } else { col }))
        .title(Line::from(title))
        .title(
            Line::from(vec![pill(&format!("{g} {label}"), col), Span::raw(" ")]).right_aligned(),
        );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;
    let now = crate::cache::now();
    let mut lines: Vec<Line> = Vec::new();
    let folder = folder_of(d, it);
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(
            e.host.clone(),
            ui::fg(theme::machine(&d.cfg, &e.host)).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            icons::distro(&host_os(d, &e.host))
                .map(|x| format!(" · {}", x.name))
                .unwrap_or_default(),
            ui::muted(),
        ),
        Span::styled(
            format!(
                "  {}",
                ui::trunc_left(&folder, w.saturating_sub(ui::width(&e.host) + 14))
            ),
            ui::fg(c::TEXT()),
        ),
    ]));
    let task = s
        .as_ref()
        .map(|s| s.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| reason(e).0);
    lines.push(Line::from(Span::styled(
        format!(" {}", ui::trunc(&task, w.saturating_sub(2))),
        ui::bold(),
    )));
    let line3 = match &s {
        Some(s) if s.blocked() => Span::styled(
            format!(
                " {} waiting {} — y approves · s replies",
                crate::look::needs(),
                ui::dur(now.saturating_sub(s.since.unwrap_or(e.t)))
            ),
            ui::fg(c::NEEDS()),
        ),
        Some(s) if s.limit.is_some() => Span::styled(
            format!(
                " ◷ {}",
                super::super::fleet::limit_text(s.limit.as_deref().unwrap_or(""))
            ),
            ui::fg(c::LIMIT()),
        ),
        Some(s) => Span::styled(
            format!(
                " {} for {}",
                label,
                ui::dur(now.saturating_sub(s.since.unwrap_or(e.t)))
            ),
            ui::fg(col),
        ),
        None => Span::styled(" ", ui::faint()),
    };
    lines.push(Line::from(line3));
    // its last alerts, oldest to newest
    let mut tl: Vec<&Event> = d
        .inbox
        .rows
        .iter()
        .filter(|x| x.host == e.host && x.name == e.name)
        .take(5)
        .collect();
    tl.reverse();
    let mut l = vec![Span::raw(" ")];
    for (i, x) in tl.iter().enumerate() {
        let (g, kc, _) = look(x.kind);
        if i > 0 {
            l.push(Span::styled(" · ", ui::faint()));
        }
        l.push(Span::styled(ui::clock(x.t), ui::muted()));
        l.push(Span::styled(
            format!(" {g}"),
            ui::fg(kc).add_modifier(Modifier::BOLD),
        ));
    }
    if tl.is_empty() {
        l.push(Span::styled("no alerts yet", ui::faint()));
    }
    lines.push(Line::from(l));
    let mut p = Paragraph::new(lines);
    if selected {
        p = p.style(ui::selected());
    }
    f.render_widget(p, inner);
}
