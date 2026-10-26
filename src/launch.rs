//! The bridge between our screens and herdr: a full-screen progress card
//! ("✻ Herding Codex onto b31…") while the tab is created, then herdr takes
//! over the *same* alternate screen — the shell never flashes in between.
//!
//! Two endings: `exec` (CLI launches: this process becomes herdr) or `wait`
//! (from the dashboard with `return_to_dashboard`: herdr runs as a child and
//! the dashboard comes back when you detach with ctrl+b q).

use crate::config::{Config, Target};
use crate::herdr::{Herdr, NewOpts, Started};
use crate::history;
use crate::term::Tui;
use crate::ui::{self, c};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[allow(clippy::large_enum_variant)] // one per launch
pub enum Job {
    New(NewOpts),
    Attach { pane: String, label: String },
}

pub enum Outcome {
    /// the user detached from herdr (wait mode); sessions started, if any
    Detached(Vec<Started>),
    Cancelled,
    Failed(String),
    /// showcase mode: herdr's own screen isn't masked, so open this pane
    /// in Aleph's Focus instead (the session is started, if it was new)
    #[cfg_attr(not(feature = "showcase"), allow(dead_code))]
    Focus(String),
}

enum Msg {
    Step(usize, String),
    Done(Vec<Started>),
    Fail(String),
}

#[derive(Clone, Copy, PartialEq)]
enum S {
    Todo,
    Doing,
    Done,
    Failed,
}

/// Runs the job, then attaches. With `wait=false` this process is replaced by
/// herdr on success; otherwise it returns after the user detaches. In
/// showcase mode it never attaches: `Outcome::Focus` says which pane to show.
pub fn run(mut tui: Tui, t: &Target, job: Job, wait: bool, host_color: Color) -> (Tui, Outcome) {
    let focus_instead = matches!(
        crate::showcase::gate(crate::showcase::Outside::Attach),
        crate::showcase::Gate::Focus(_)
    );
    if focus_instead {
        if let Job::Attach { pane, .. } = &job {
            return (tui, Outcome::Focus(pane.clone()));
        }
    }
    let started = Instant::now();
    let verb = ui::verb(crate::names::seed());
    // the agent it starts: what was asked for, else the configured default
    let job = match job {
        Job::New(mut o) => {
            o.agent = o.agent_id();
            Job::New(o)
        }
        j => j,
    };
    let agent_title = match &job {
        Job::New(o) => crate::agents::label(&o.agent),
        Job::Attach { .. } => String::new(),
    };
    let (title, mut steps, folder): (String, Vec<(S, String)>, Option<String>) = match &job {
        Job::New(o) => (
            format!(
                "{verb} {}{agent_title} onto {}…",
                if o.tile > 1 {
                    format!("{}× ", o.tile)
                } else {
                    String::new()
                },
                t.ssh
            ),
            vec![
                (S::Doing, format!("Connecting to herdr on {}", t.ssh)),
                (
                    S::Todo,
                    if o.worktree.is_some() {
                        "Creating a worktree".into()
                    } else {
                        "Opening a new tab".into()
                    },
                ),
                (S::Todo, format!("Starting {agent_title}")),
                (S::Todo, format!("Attaching to {}", t.ssh)),
            ],
            o.dir.as_ref().map(|d| crate::path::show(d)),
        ),
        Job::Attach { label, .. } => (
            format!("Reattaching to {label}…"),
            vec![
                (S::Doing, format!("Focusing {label}")),
                (S::Todo, format!("Attaching to {}", t.ssh)),
            ],
            None,
        ),
    };
    let user = crate::sshcfg::resolve(&t.ssh)
        .map(|r| r.user)
        .unwrap_or_default();
    let folder = folder.map(|f| crate::herdr::short_path(&f, &user));
    let record = match &job {
        Job::New(o) => Some(o.clone()),
        _ => None,
    };

    let (tx, rx) = mpsc::channel();
    let h = Herdr::new(t);
    thread::spawn(move || {
        let r = match job {
            Job::New(o) => {
                let tx2 = tx.clone();
                h.new_session(&o, &mut |i, l| {
                    let _ = tx2.send(Msg::Step(i, l));
                })
            }
            Job::Attach { pane, .. } => h.focus(&pane).map(|_| vec![]),
        };
        let _ = tx.send(match r {
            Ok(v) => Msg::Done(v),
            Err(e) => Msg::Fail(e),
        });
    });

    let mut error: Option<String> = None;
    loop {
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Step(i, label) => {
                    for (j, s) in steps.iter_mut().enumerate() {
                        if j < i {
                            s.0 = S::Done;
                        }
                    }
                    if let Some(s) = steps.get_mut(i) {
                        *s = (S::Doing, label);
                    }
                }
                Msg::Done(v) => {
                    let n = steps.len();
                    for s in steps.iter_mut().take(n - 1) {
                        s.0 = S::Done;
                    }
                    steps[n - 1].0 = S::Doing;
                    let _ = tui.term.draw(|f| {
                        draw(
                            f,
                            &title,
                            &steps,
                            folder.as_deref(),
                            None,
                            started,
                            host_color,
                        );
                        ui::finish(f.buffer_mut());
                    });
                    if let Some(o) = &record {
                        after_start(t, o, &v);
                    }
                    if focus_instead {
                        let pane = v.first().map(|s| s.pane.clone()).unwrap_or_default();
                        return (tui, Outcome::Focus(pane));
                    }
                    if wait {
                        // a remote attach that ends badly after running a while
                        // lost its connection (a detach exits cleanly): the
                        // session keeps running there, so attach again
                        let mut drops = 0;
                        loop {
                            let _ = tui.suspend_keep_screen();
                            let t0 = Instant::now();
                            let r = Herdr::new(t).attach_wait();
                            let _ = tui.resume();
                            match r {
                                Ok(false)
                                    if !crate::sshcfg::is_local(&t.ssh)
                                        && t0.elapsed() > Duration::from_secs(3)
                                        && drops < 5 =>
                                {
                                    drops += 1;
                                    crate::log::warn(
                                        "attach",
                                        &format!("the attach to {} dropped — reattaching", t.ssh),
                                    );
                                    if reattach_card(&mut tui, t, host_color) {
                                        continue;
                                    }
                                    return (tui, Outcome::Detached(v));
                                }
                                Ok(_) => return (tui, Outcome::Detached(v)),
                                Err(e) => {
                                    return (
                                        tui,
                                        Outcome::Failed(format!("couldn't run herdr: {e}")),
                                    )
                                }
                            }
                        }
                    }
                    // hand the very same screen to herdr
                    tui.handoff();
                    let e = Herdr::new(t).attach();
                    let _ = crossterm::execute!(
                        std::io::stderr(),
                        crossterm::terminal::LeaveAlternateScreen
                    );
                    eprintln!("error: couldn't start herdr --remote {}: {e}", t.ssh);
                    std::process::exit(1);
                }
                Msg::Fail(e) => {
                    if let Some(s) = steps.iter_mut().find(|s| s.0 == S::Doing) {
                        s.0 = S::Failed;
                    }
                    error = Some(e);
                }
            }
        }
        let _ = tui.term.draw(|f| {
            draw(
                f,
                &title,
                &steps,
                folder.as_deref(),
                error.as_deref(),
                started,
                host_color,
            );
            ui::finish(f.buffer_mut());
        });
        if event::poll(Duration::from_millis(60)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                // (showcase: the panic key, and every key while paused, is its)
                if k.kind == KeyEventKind::Release
                    || crate::showcase::panic_key(&crate::tui::key_str(&k))
                {
                    continue;
                }
                let ctrl_c =
                    k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c');
                if let Some(e) = error.take() {
                    return (tui, Outcome::Failed(e));
                }
                if ctrl_c || k.code == KeyCode::Esc {
                    return (tui, Outcome::Cancelled);
                }
            }
        }
    }
}

/// History + the local post-start hook.
pub(crate) fn after_start(t: &Target, o: &NewOpts, v: &[Started]) {
    for s in v {
        history::append(&history::Entry {
            t: crate::cache::now(),
            host: t.ssh.clone(),
            dir: s.dir.clone(),
            name: s.name.clone(),
            agent: o.agent_id(),
            args: o.args.clone(),
            prompt: o.prompt.clone(),
            worktree: o.worktree.is_some(),
        });
    }
    let cfg = Config::load();
    if let (Some(hook), Some(first)) = (cfg.get("hook.post_new"), v.first()) {
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(hook)
            .env("CH_HOST", &t.ssh)
            .env("CH_DIR", &first.dir)
            .env("CH_NAME", &first.name)
            .env("CH_PANE", &first.pane)
            .env("CH_LABEL", &first.label)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// "Connection to b31 lost — reattaching in 3s": true to reattach (after
/// the wait, or enter), false to go back (esc).
fn reattach_card(tui: &mut Tui, t: &Target, host_color: Color) -> bool {
    let t0 = Instant::now();
    let wait = Duration::from_secs(3);
    while t0.elapsed() < wait {
        let left = (wait - t0.elapsed()).as_secs() + 1;
        let _ = tui.term.draw(|f| {
            let area = f.area();
            let w = 64u16.min(area.width.saturating_sub(2));
            let r = ui::centered(area, w, 7);
            crate::ui::clear(f, r);
            let lines = vec![
                Line::from(vec![
                    Span::styled(format!("{} ", ui::spinner(t0)), ui::fg(c::ACCENT())),
                    Span::styled("Connection to ", ui::bold()),
                    Span::styled(
                        t.ssh.clone(),
                        ui::fg(host_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" lost", ui::bold()),
                ]),
                Line::raw(""),
                Line::from(Span::styled(
                    format!("the session keeps running there · reattaching in {left}s"),
                    ui::muted(),
                )),
                Line::raw(""),
                ui::hints(&[("enter", "now"), ("esc", "back to Aleph")]),
            ];
            f.render_widget(
                Paragraph::new(lines).block(ui::panel("Reattaching", true)),
                r,
            );
            ui::finish(f.buffer_mut());
        });
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                // (showcase: the panic key, and every key while paused, is its)
                if k.kind != KeyEventKind::Press
                    || crate::showcase::panic_key(&crate::tui::key_str(&k))
                {
                    continue;
                }
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => return false,
                    KeyCode::Enter => return true,
                    _ => {}
                }
            }
        }
    }
    true
}

fn draw(
    f: &mut Frame,
    title: &str,
    steps: &[(S, String)],
    folder: Option<&str>,
    err: Option<&str>,
    started: Instant,
    host_color: Color,
) {
    let area = f.area();
    let w = 64u16.min(area.width.saturating_sub(4)).max(30);
    let mut lines: Vec<Line> = Vec::new();
    let spin = ui::spinner(started);
    let head_style =
        ui::fg(if err.is_some() { c::RED() } else { c::ACCENT() }).add_modifier(Modifier::BOLD);
    lines.push(Line::from(vec![
        Span::styled(if err.is_some() { "✗ " } else { spin }, head_style),
        Span::styled(if err.is_some() { "" } else { " " }, Style::new()),
        Span::styled(
            if err.is_some() {
                "Couldn't start the session"
            } else {
                title
            },
            head_style,
        ),
    ]));
    lines.push(Line::raw(""));
    for (s, label) in steps {
        let (m, st, ls) = match s {
            S::Done => ("✓", ui::fg(c::GREEN()), Style::new()),
            S::Doing => (spin, ui::fg(c::ACCENT()), ui::bold()),
            S::Todo => ("○", ui::faint(), ui::muted()),
            S::Failed => ("✗", ui::fg(c::RED()), ui::fg(c::RED())),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {m}  "), st),
            Span::styled(label.clone(), ls),
        ]));
    }
    if let Some(dir) = folder {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("  ⎿  ", ui::faint()),
            Span::styled(ui::trunc_left(dir, w as usize - 10), ui::muted()),
        ]));
    }
    lines.push(Line::raw(""));
    match err {
        Some(e) => {
            for l in ui::wrap(e, w as usize - 6) {
                lines.push(Line::from(Span::styled(format!("  {l}"), ui::fg(c::RED()))));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("  hint: ", ui::fg(c::BLUE()).add_modifier(Modifier::BOLD)),
                Span::raw("aleph doctor"),
                Span::styled(" explains what's wrong", ui::muted()),
            ]));
            lines.push(Line::from(Span::styled(
                "  press any key to go back",
                ui::faint(),
            )));
        }
        None => lines.push(Line::from(Span::styled(
            format!(
                "  {:.1}s · ctrl+c to cancel",
                started.elapsed().as_secs_f32()
            ),
            ui::faint(),
        ))),
    }
    let h = lines.len() as u16 + 2;
    let r = ui::centered(area, w, h);
    crate::ui::clear(f, r);
    f.render_widget(
        Paragraph::new(lines).block(
            ui::panel(
                Line::from(vec![Span::styled(
                    format!("{} aleph", crate::brand::MARK),
                    ui::fg(c::ACCENT()),
                )]),
                false,
            )
            .border_style(Style::new().fg(host_color)),
        ),
        r,
    );
    crate::showcase::buffer(f.buffer_mut());
}
