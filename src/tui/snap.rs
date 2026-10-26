//! Showcase builds only (`aleph showcase snap`): one frame of the dashboard
//! drawn offscreen — a `TestBackend` at the size asked for, once the first
//! polls are in (and Monitor's stats, Focus's screen), through the lens —
//! for a poster. The dashboard is the real one, opened without a terminal:
//! the same views, the same data, nothing typed or pressed.

use super::monitor_draw::Sub;
use super::*;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

/// What a poster can be of: (name, what it shows).
pub(crate) const VIEWS: &[(&str, &str)] = &[
    ("fleet", "Fleet: every agent on every machine"),
    ("monitor", "Monitor: the machines side by side"),
    ("overview", "Monitor › Overview: one machine"),
    ("cpu", "Monitor › CPU"),
    ("memory", "Monitor › Memory"),
    ("gpu", "Monitor › GPU"),
    ("procs", "Monitor › Processes"),
    ("net", "Monitor › Network"),
    ("services", "Monitor › LLM servers and gateways"),
    ("usage", "Monitor › Usage: tokens, time and spend"),
    ("inbox", "Inbox: what needs you, what finished"),
    ("settings", "Settings"),
    ("focus", "Focus: one agent, full screen"),
    ("tile", "Tile: agents side by side"),
    ("help", "the ? overlay over Fleet"),
    ("palette", "the command palette over Fleet"),
];

/// Where a view is: the tab, Monitor's view in it, an overlay.
fn place(view: &str) -> Option<(View, Option<Sub>)> {
    let sub = |s| Some((View::Monitor, Some(s)));
    match view {
        "fleet" | "help" | "palette" | "focus" | "tile" => Some((View::Fleet, None)),
        "monitor" => sub(Sub::Fleet),
        "overview" => sub(Sub::Overview),
        "cpu" => sub(Sub::Cpu),
        "memory" | "mem" => sub(Sub::Memory),
        "gpu" => sub(Sub::Gpu),
        "procs" | "processes" => sub(Sub::Procs),
        "net" | "network" => sub(Sub::Net),
        "services" => sub(Sub::Services),
        "usage" => sub(Sub::Usage),
        "inbox" => Some((View::Inbox, None)),
        "settings" => Some((View::Settings, None)),
        _ => None,
    }
}

/// How long a view settles once the polls are in (Monitor's rates need a
/// second sample, Focus a screen from the pane).
fn settle(view: &str) -> Duration {
    match view {
        "fleet" | "inbox" | "settings" | "help" | "palette" => Duration::from_millis(400),
        "focus" | "tile" => Duration::from_millis(2200),
        _ => Duration::from_millis(2600),
    }
}

/// One frame of `view` at `w`×`h` (at most `wait` for the fleet to answer):
/// the buffer, through the lens, and the view's title.
pub(crate) fn frame(
    cfg: Config,
    view: &str,
    w: u16,
    h: u16,
    wait: Duration,
) -> Result<(Buffer, String), String> {
    let v = view.trim().to_ascii_lowercase();
    let (tab, sub) = place(&v).ok_or_else(|| {
        let all: Vec<&str> = VIEWS.iter().map(|(n, _)| *n).collect();
        format!("no view {view} (one of {})", all.join(", "))
    })?;
    let start = Start {
        view: tab,
        ..Default::default()
    };
    let mut d = Dash::new(cfg, &start);
    let mut t = Terminal::new(TestBackend::new(w, h)).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + wait;
    let mut opened: Option<Instant> = None;
    loop {
        // drawn as the dashboard draws, every turn: what's on screen (a
        // Focus pane, its size) decides what it fetches next
        if opened.is_some() {
            t.draw(|f| d.draw(f)).map_err(|e| e.to_string())?;
        }
        while let Ok(ev) = d.rx.try_recv() {
            d.on_bg(ev);
        }
        d.fleet.tick();
        d.sync_telemetry();
        fleet::pump(&mut d);
        wall::pump(&mut d);
        inbox::pump(&mut d);
        usage::pump(&mut d);
        outside::pump(&mut d);
        let polled = d
            .fleet
            .hosts
            .iter()
            .all(|x| x.conn != crate::fleet::Conn::Unknown);
        let late = Instant::now() >= deadline;
        if opened.is_none() && (polled || late) {
            open(&mut d, &v, tab, sub);
            opened = Some(Instant::now());
        }
        match opened {
            Some(t) if t.elapsed() >= settle(&v) => break,
            Some(_) if late && Instant::now() >= deadline + settle(&v) => break,
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    // the screen alone: no keys just pressed, no caption; in the colours the
    // dashboard has in showcase mode (the cockpit palette)
    crate::showcase::capture::hush();
    crate::showcase::repaint(&d.cfg);
    for _ in 0..2 {
        t.draw(|f| {
            d.draw(f);
            crate::showcase::buffer(f.buffer_mut());
        })
        .map_err(|e| e.to_string())?;
    }
    let buf = t.backend().buffer().clone();
    d.drop_connections();
    d.fleet.stop_listening();
    Ok((buf, title(&v, sub, &d)))
}

/// The view opened, as the keys would: Monitor's view, Focus on the
/// session that needs you most, Tile of the first four, an overlay.
fn open(d: &mut Dash, v: &str, tab: View, sub: Option<Sub>) {
    d.switch(tab);
    if let Some(s) = sub {
        d.mon.sub = s;
    }
    let mut keys: Vec<(u8, String)> = d
        .fleet
        .sessions()
        .iter()
        .map(|s| {
            let rank = match s.status.as_str() {
                "blocked" => 0,
                "working" => 1,
                _ => 2,
            };
            (rank, s.key())
        })
        .collect();
    keys.sort();
    match v {
        "focus" => {
            if let Some((_, k)) = keys.first() {
                wall::focus_on(d, k, View::Fleet);
            }
        }
        "tile" => {
            let pick: Vec<String> = keys.into_iter().take(4).map(|(_, k)| k).collect();
            if !pick.is_empty() {
                wall::tile(d, pick, View::Fleet);
            }
        }
        "help" => d.mode = Mode::Help(help::Help::default()),
        "palette" => d.mode = Mode::Palette(palette::Palette::new(d)),
        _ => {}
    }
}

/// `Fleet`, `Monitor › Usage`, `Focus`.
fn title(v: &str, sub: Option<Sub>, d: &Dash) -> String {
    match (v, sub) {
        ("monitor", _) => "Monitor".into(),
        (_, Some(s)) => format!("Monitor › {}", s.name()),
        ("focus", _) if d.view == View::Wall => "Focus".into(),
        ("tile", _) if d.view == View::Wall => "Tile".into(),
        ("help", _) => "Keys".into(),
        ("palette", _) => "Palette".into(),
        _ => d.view.name().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_view_has_a_place() {
        for (v, _) in VIEWS {
            assert!(place(v).is_some(), "{v}");
        }
        assert!(place("nowhere").is_none());
        assert_eq!(place("usage"), Some((View::Monitor, Some(Sub::Usage))));
    }
}
