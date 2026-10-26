//! `aleph setup`: the onboarding wizard, full screen in the alternate screen.
//!
//! A rail of twelve steps on the left (✓ done · ! needs a look · ○ not yet),
//! the step on the right, keycaps at the bottom. ↑↓ run through every step's
//! rows as one list (past a step's last row, the next step opens — nothing
//! marked); `enter` changes or runs the row it's on, and each step ends in a
//! Continue button that marks it and opens the next. `tab` the next step,
//! `esc` back, `s` skip, `r` retry, `?` why. Choices go into a [`Plan`] —
//! settings, presets and actions — and nothing changes until Review & apply,
//! which shows the TOML diff and the actions (the ones that run on other
//! machines called out) and applies them with progress; one failure never
//! stops the others. A few things happen in the step that needs them, always
//! after asking: trusting a host key (fingerprints shown, a changed key is
//! refused), key login (the password only ever goes into ssh's own prompt),
//! a test notification.
//!
//! Every external call runs off the UI thread with a timeout (`proc::run`)
//! and a spinner; anything that needs the terminal (a password, an
//! installer's own prompts) gets it on the same alternate screen and comes
//! back. Progress is kept in `state/setup.json`, so a run that stops picks up
//! where it was; `--redo` starts over from the current settings.
//!
//! One file per step in `onboard/`; `rows.rs` is the body every step is
//! made of, `plan.rs` what setup will change, `declare.rs` the
//! non-interactive `aleph setup --apply FILE.toml`.

mod access;
mod agent_setup;
mod analytics;
mod automation;
pub(crate) mod declare;
mod herdr_setup;
mod install;
mod integrations;
pub(crate) mod look;
mod machines;
mod notifications;
pub(crate) mod plan;
mod preflight;
mod quick;
mod review;
pub(crate) mod rows;
mod starters;
#[cfg(test)]
mod tests;

use super::forms::Field;
use crate::config::{self, Config};
use crate::probe::HostProbe;
use crate::term::Tui;
use crate::ui::{self, c};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
pub(crate) use plan::{Action, Plan};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use rows::{Kind, Row, St};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ steps

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Step {
    Preflight,
    Look,
    Machines,
    Herdr,
    Agents,
    Notify,
    Analytics,
    Automation,
    Access,
    Integrations,
    Presets,
    Review,
}

impl Step {
    pub const ALL: [Step; 12] = [
        Step::Preflight,
        Step::Look,
        Step::Machines,
        Step::Herdr,
        Step::Agents,
        Step::Notify,
        Step::Analytics,
        Step::Automation,
        Step::Access,
        Step::Integrations,
        Step::Presets,
        Step::Review,
    ];

    /// Its name on the command line and in setup.json.
    pub fn id(self) -> &'static str {
        match self {
            Step::Preflight => "preflight",
            Step::Look => "look",
            Step::Machines => "machines",
            Step::Herdr => "herdr",
            Step::Agents => "agents",
            Step::Notify => "notifications",
            Step::Analytics => "analytics",
            Step::Automation => "automation",
            Step::Access => "daemon",
            Step::Integrations => "integrations",
            Step::Presets => "presets",
            Step::Review => "review",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Preflight => "Preflight",
            Step::Look => "Look & feel",
            Step::Machines => "Machines",
            Step::Herdr => "herdr",
            Step::Agents => "Agents",
            Step::Notify => "Notifications",
            Step::Analytics => "Analytics",
            Step::Automation => "Automation",
            Step::Access => "Daemon & access",
            Step::Integrations => "Integrations",
            Step::Presets => "Presets",
            Step::Review => "Review & apply",
        }
    }

    /// `machines`, `3`, `notify`, `access`, … (None: not a step).
    pub fn parse(s: &str) -> Option<Step> {
        let s = s.trim().to_lowercase();
        if let Ok(n) = s.parse::<usize>() {
            return Step::ALL.get(n.checked_sub(1)?).copied();
        }
        Some(match s.as_str() {
            "preflight" | "check" | "checks" => Step::Preflight,
            "look" | "look-and-feel" | "appearance" | "theme" | "ui" => Step::Look,
            "machines" | "hosts" | "fleet" => Step::Machines,
            "herdr" => Step::Herdr,
            "agents" => Step::Agents,
            "notifications" | "notify" => Step::Notify,
            "analytics" | "usage" => Step::Analytics,
            "automation" | "auto" => Step::Automation,
            "daemon" | "access" | "api" => Step::Access,
            "integrations" | "integration" => Step::Integrations,
            "presets" | "preset" => Step::Presets,
            "review" | "apply" => Step::Review,
            _ => return None,
        })
    }

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Option<Step> {
        Step::ALL.get(self.index() + 1).copied()
    }

    fn prev(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| Step::ALL[i])
    }

    /// The step's first lines.
    fn intro(self) -> &'static str {
        match self {
            Step::Preflight => "What this computer has. Nothing here changes anything; a fix runs only when you pick it.",
            Step::Look => "How Aleph looks. The preview follows every change.",
            Step::Machines => "The ssh machines your agents run on. Pick them, then check each one: its host key (you decide whether to trust it) and key login.",
            Step::Herdr => "herdr on each machine: installed, the same version as here, saved in herdr. Installs and updates wait for Review, and herdr asks once more.",
            Step::Agents => "The agents on each machine — tick the ones to install, from every agent Aleph knows — the default one, and herdr's integrations (sessions survive a herdr restart).",
            Step::Notify => "Who tells you, where it goes, and when to keep quiet.",
            Step::Analytics => "Tokens, time and cost from the agents' own logs on each machine. Read in place; nothing is installed there.",
            Step::Automation => "What Aleph does on its own: answer safe questions, bring herdr back, carry on after a usage limit.",
            Step::Access => "The daemon watches the fleet while no dashboard runs: notifications, the hub, the API.",
            Step::Integrations => "Aleph in the tools around it. Each one is opt-in.",
            Step::Presets => "Starting points: one session or several, on one machine or many.",
            Step::Review => "Everything setup will change. Nothing has been written yet.",
        }
    }
}

/// How a step stands on the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Todo,
    Done,
    Skipped,
    Warn,
}

impl Mark {
    fn word(self) -> &'static str {
        match self {
            Mark::Todo => "todo",
            Mark::Done => "done",
            Mark::Skipped => "skipped",
            Mark::Warn => "warn",
        }
    }
    fn parse(s: &str) -> Mark {
        match s {
            "done" => Mark::Done,
            "skipped" => Mark::Skipped,
            "warn" => Mark::Warn,
            _ => Mark::Todo,
        }
    }
}

/// How the wizard was started.
#[derive(Clone, Debug, Default)]
pub struct Opts {
    /// start over from the current settings (`--redo`)
    pub redo: bool,
    /// open at this step (`aleph setup <step>`); its "next" is Review
    pub only: Option<Step>,
    /// the first run (a bare `aleph` with no machines): the dashboard after
    pub first: bool,
    /// quick start: every default, installed at once (`--defaults`)
    pub defaults: bool,
}

// ------------------------------------------------------------------ events

/// What a row (or a confirm, an input, a hand-off) asks its step to do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ev {
    Button(String),
    Fix(String),
    Pick(String),
    Cell(String, usize),
    Local(String, usize),
    Input(String, String),
    Confirm(String),
    /// a hand-off came back: (its id, how it went)
    After(String, Result<String, String>),
}

/// Results from background threads.
pub(crate) enum Msg {
    Pre(Vec<preflight::Check>),
    Found(Vec<machines::Cand>),
    Login(String, machines::Login),
    Trusted(String, Result<(), String>),
    Probe(String, Box<HostProbe>),
    /// is herdr's server running on that machine (None: couldn't tell)
    Server(String, Option<bool>),
    Herdr {
        local: Option<String>,
        saved: Vec<String>,
    },
    Hooks(String, Result<Vec<(String, Vec<String>)>, String>),
    Sent(String, Result<(), String>),
    Moshi(Option<crate::moshi::Status>),
    Usage(Vec<analytics::Seen>),
    Import(String, Result<Vec<crate::presets::Preset>, String>),
    Access(access::Facts),
    Integ(integrations::Facts),
    Applied(usize, Result<String, String>),
    /// a machine's agents went in (items i..=j)
    Installed(usize, usize, Result<String, String>),
    /// one agent's own run came back (item k)
    AgentDone(usize, Result<String, String>),
    /// a line of an item's output (the job goes on)
    Line(usize, String),
    Note(bool, String),
    /// is an OpenRouter key in the keystore; what storing one said (bad, text)
    OpenRouter(bool, Option<(bool, String)>),
}

pub(crate) struct Job {
    pub what: String,
    pub label: String,
    pub t0: Instant,
}

/// Something that needs the real terminal (a password, an installer's own
/// prompts); the wizard lends it out on the same alternate screen.
pub(crate) struct Hand {
    pub title: String,
    pub note: String,
    pub run: Run,
    /// the step, and the id its `Ev::After` gets
    pub step: Step,
    pub id: String,
}

pub(crate) enum Run {
    /// a program with its arguments (in a folder)
    Cmd {
        prog: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },
    /// key login: ssh-copy-id, the password typed into ssh's own prompt
    KeySetup(String),
}

impl Run {
    pub fn aleph(args: &[&str]) -> Run {
        Run::Cmd {
            prog: self_exe(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: None,
        }
    }
    pub fn cmd(prog: &str, args: &[&str]) -> Run {
        Run::Cmd {
            prog: prog.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: None,
        }
    }
    /// How it reads in a confirm.
    pub fn shown(&self) -> String {
        match self {
            Run::Cmd { prog, args, cwd } => {
                let p = if *prog == self_exe() {
                    crate::brand::BIN.to_string()
                } else {
                    prog.clone()
                };
                let line = std::iter::once(p)
                    .chain(args.iter().map(|a| crate::proc::sh_quote(a)))
                    .collect::<Vec<_>>()
                    .join(" ");
                match cwd {
                    Some(d) => format!("cd {} && {line}", crate::doctor::tilde(d)),
                    None => line,
                }
            }
            Run::KeySetup(h) => format!("ssh-copy-id {h}"),
        }
    }
}

/// This program, for running its own subcommands.
pub(crate) fn self_exe() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| crate::brand::BIN.into())
}

/// Is `aleph <cmd> <sub>` in this build? (Commands other work is adding:
/// `daemon deploy`, `daemon expose`, `daemon pair`.)
pub(crate) fn has_sub(cmd: &str, sub: &str) -> bool {
    crate::COMMANDS
        .iter()
        .find(|c| c.0 == cmd)
        .is_some_and(|c| {
            c.1.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                .any(|w| w == sub)
        })
}

/// A preview or detail beside (or under) a step's rows.
pub(crate) struct Aside {
    pub title: String,
    pub lines: Vec<Line<'static>>,
}

/// What a confirm does on yes.
pub(crate) enum Yes {
    Step(Step, String),
    Hand(Box<Hand>),
    Apply,
    Quit,
}

pub(crate) enum EditFor {
    Set(String),
    Step(Step, String),
}

pub(crate) enum Overlay {
    None,
    Confirm {
        title: String,
        lines: Vec<Line<'static>>,
        yes: Yes,
    },
    Why {
        title: String,
        text: String,
    },
    Edit {
        title: String,
        hint: String,
        field: Field,
        secret: bool,
        err: Option<String>,
        to: EditFor,
    },
}

/// How the event loop ends a turn.
pub(crate) enum Exit {
    Quit,
    Hand(Hand),
    Dashboard { tour: bool },
}

struct Toast {
    text: String,
    bad: bool,
    at: Instant,
}

#[derive(Default)]
pub(crate) struct Hits {
    rail: Vec<(Rect, Step)>,
    /// each line shown → its row's index
    rows: Vec<(Rect, usize)>,
    /// the `↑ N more` / `↓ N more` lines (true: up)
    more: Vec<(Rect, bool)>,
}

/// The Continue row's button: the wizard's own, never a step's.
const NEXT: &str = "wiz:next";

/// Lines the mouse wheel scrolls.
const WHEEL: isize = 3;

// ------------------------------------------------------------------ the wizard

pub struct Wiz {
    pub cfg: Config,
    pub opts: Opts,
    pub cur: Step,
    pub marks: [Mark; 12],
    pub plan: Plan,
    /// each step's cursor (a row index) and the row it's on ([`Row::ident`]):
    /// rows are rebuilt as answers arrive, and the cursor stays on its row
    sel: [usize; 12],
    sel_id: [String; 12],
    /// each step's first line shown (`usize::MAX`: the end)
    top: [usize; 12],
    /// the list as last drawn: (lines, columns) — page up / down, the wheel
    view: (usize, usize),
    /// the view was scrolled by the mouse: it stays put until the cursor
    /// moves with a key
    loose: bool,
    /// the matrix cell under the cursor
    pub cell: usize,
    pub(crate) mode: Overlay,
    /// quick start is waiting for what it reads (`quick::tick`)
    pub(crate) quick: bool,
    /// just the install screen, for actions handed in (`run_actions_in`):
    /// no wizard behind it, esc goes back once it's done
    pub(crate) queue_only: bool,
    tx: Sender<(u64, Msg)>,
    rx: Receiver<(u64, Msg)>,
    pub t0: Instant,
    pub(crate) jobs: HashMap<u64, Job>,
    next_job: u64,
    toasts: VecDeque<Toast>,
    pub net: crate::net::Net,
    pub(crate) hits: Hits,
    /// tests: no threads, nothing live (the palette is the process's);
    /// state still goes to `state_path` (a temp file there)
    pub quiet: bool,
    pub state_path: PathBuf,
    /// Review applied everything
    pub finished: bool,
    pub(crate) exit: Option<Exit>,
    // what we learned (shared by several steps)
    pub probes: HashMap<String, HostProbe>,
    /// this computer's herdr version (None: not looked yet; Some(None): none)
    pub local_herdr: Option<Option<String>>,
    /// machines saved in herdr (labels)
    pub saved: Option<Vec<String>>,
    // each step's own state
    pub(crate) pre: preflight::State,
    pub(crate) look: look::State,
    pub(crate) mach: machines::State,
    pub(crate) hd: herdr_setup::State,
    pub(crate) ag: agent_setup::State,
    pub(crate) nt: notifications::State,
    pub(crate) an: analytics::State,
    pub(crate) au: automation::State,
    pub(crate) ac: access::State,
    pub(crate) ig: integrations::State,
    pub(crate) ps: starters::State,
    pub(crate) rv: review::State,
}

/// `state/setup.json`.
pub fn state_file() -> PathBuf {
    config::state_dir().join("setup.json")
}

/// What setup.json holds.
#[derive(Clone, Debug, Default)]
pub struct Saved {
    pub at: Option<Step>,
    pub marks: Vec<(Step, Mark)>,
    pub plan: Plan,
    pub finished: bool,
}

/// Did setup run to the end (quick start included)? A fleet of only this
/// computer is a fleet then: a bare `aleph` opens the dashboard.
pub fn finished() -> bool {
    load_saved(&state_file()).is_some_and(|s| s.finished)
}

pub fn load_saved(path: &std::path::Path) -> Option<Saved> {
    let t = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    Some(Saved {
        at: v.get("at").and_then(|x| x.as_str()).and_then(Step::parse),
        marks: v
            .get("marks")
            .and_then(|m| m.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, x)| Some((Step::parse(k)?, Mark::parse(x.as_str()?))))
                    .collect()
            })
            .unwrap_or_default(),
        plan: v.get("plan").map(Plan::from_json).unwrap_or_default(),
        finished: v.get("finished").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

impl Wiz {
    /// The wizard on `cfg`, resuming from `state_path` unless `opts.redo`.
    /// `quiet` (tests) starts no threads and changes nothing live.
    pub fn new(cfg: Config, opts: Opts, state_path: PathBuf, quiet: bool) -> Wiz {
        let (tx, rx) = channel();
        let net = if quiet {
            crate::net::Net::quiet()
        } else {
            crate::net::Net::start(crate::net::target(&cfg), Duration::from_secs(5))
        };
        let mut w = Wiz {
            cfg,
            cur: opts.only.unwrap_or(Step::Preflight),
            opts,
            marks: [Mark::Todo; 12],
            plan: Plan::default(),
            sel: [0; 12],
            sel_id: Default::default(),
            top: [0; 12],
            view: (0, 0),
            loose: false,
            cell: 0,
            mode: Overlay::None,
            quick: false,
            queue_only: false,
            tx,
            rx,
            t0: Instant::now(),
            jobs: HashMap::new(),
            next_job: 1,
            toasts: VecDeque::new(),
            net,
            hits: Hits::default(),
            quiet,
            state_path,
            finished: false,
            exit: None,
            probes: HashMap::new(),
            local_herdr: None,
            saved: None,
            pre: Default::default(),
            look: Default::default(),
            mach: Default::default(),
            hd: Default::default(),
            ag: Default::default(),
            nt: Default::default(),
            an: Default::default(),
            au: Default::default(),
            ac: Default::default(),
            ig: Default::default(),
            ps: Default::default(),
            rv: Default::default(),
        };
        if !w.opts.redo {
            if let Some(s) = load_saved(&w.state_path) {
                for (st, m) in &s.marks {
                    w.marks[st.index()] = *m;
                }
                if !s.finished {
                    let n = s.plan.len();
                    w.plan = s.plan;
                    if w.opts.only.is_none() {
                        if let Some(at) = s.at {
                            w.cur = at;
                        }
                    }
                    if n > 0 {
                        w.toast(
                            false,
                            format!(
                                "picked up where you stopped · {n} change{} waiting for Review",
                                if n == 1 { "" } else { "s" }
                            ),
                        );
                    }
                }
            }
        }
        machines::init(&mut w);
        w
    }

    /// Start the current step (its probes).
    pub fn start(&mut self) {
        self.enter(self.cur);
        if self.opts.defaults {
            quick::start(self);
        }
    }

    // ------------------------------------------------------------ values

    /// A setting as setup will leave it: the plan's value, else the file's
    /// (as written: `secret:NAME` stays a reference), else the default.
    pub fn value(&self, key: &str) -> String {
        match self.plan.pending(key) {
            Some(Some(v)) => v.to_string(),
            Some(None) => default_of(key),
            None => self.cfg.get_raw(key).unwrap_or_else(|| default_of(key)),
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(
            self.value(key).to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }

    pub fn pending(&self, key: &str) -> bool {
        self.plan.pending(key).is_some()
    }

    /// Change a setting in the plan (the current step's). Setting it to what
    /// it already is takes it out of the plan; empty means the default.
    pub fn set(&mut self, key: &str, val: &str) {
        let val = val.trim();
        let now = self.cfg.get_raw(key);
        let effective = now.clone().unwrap_or_else(|| default_of(key));
        if (val.is_empty() && now.is_none()) || (!val.is_empty() && val == effective) {
            self.plan.forget(key);
        } else if val.is_empty() {
            self.plan.put(self.cur, key, None);
        } else {
            self.plan.put(self.cur, key, Some(val.to_string()));
        }
        self.save();
    }

    /// The machines setup is about (the fleet as chosen in Machines).
    pub fn machines(&self) -> Vec<String> {
        match self.plan.pending("hosts") {
            Some(Some(v)) => split(v),
            Some(None) => vec![],
            None => self.cfg.hosts(),
        }
    }

    /// The machines plus `local` when this computer's herdr is in the fleet.
    pub fn fleet(&self) -> Vec<String> {
        let mut v = self.machines();
        if self.flag("local") && !v.iter().any(|h| crate::sshcfg::is_local(h)) {
            v.push(crate::sshcfg::LOCAL.to_string());
        }
        v
    }

    pub fn offline(&self) -> bool {
        self.net.state().is_offline()
    }

    /// Offline with no network at all (not even the LAN).
    pub fn no_network(&self) -> bool {
        matches!(
            self.net.state(),
            crate::net::State::Offline { route: false, .. }
        )
    }

    // ------------------------------------------------------------ jobs

    /// Run `f` on its own thread; its message comes back to the step. A job
    /// with the same `what` already running isn't started twice.
    pub(crate) fn spawn(
        &mut self,
        what: &str,
        label: impl Into<String>,
        f: impl FnOnce() -> Msg + Send + 'static,
    ) {
        if self.quiet || self.running(what) {
            return;
        }
        let id = self.next_job;
        self.next_job += 1;
        self.jobs.insert(
            id,
            Job {
                what: what.to_string(),
                label: label.into(),
                t0: Instant::now(),
            },
        );
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let m = f();
            let _ = tx.send((id, m));
        });
    }

    /// [`spawn`] for a long run whose output the install screen shows:
    /// `f` gets a function that sends each line to item `item`.
    pub(crate) fn spawn_lines(
        &mut self,
        what: &str,
        label: impl Into<String>,
        item: usize,
        f: impl FnOnce(&dyn Fn(&str)) -> Msg + Send + 'static,
    ) {
        if self.quiet || self.running(what) {
            return;
        }
        let id = self.next_job;
        self.next_job += 1;
        self.jobs.insert(
            id,
            Job {
                what: what.to_string(),
                label: label.into(),
                t0: Instant::now(),
            },
        );
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let lines = tx.clone();
            let emit = move |l: &str| {
                let _ = lines.send((id, Msg::Line(item, l.to_string())));
            };
            let m = f(&emit);
            let _ = tx.send((id, m));
        });
    }

    pub fn running(&self, what: &str) -> bool {
        self.jobs.values().any(|j| j.what == what)
    }

    /// Any job whose `what` starts with `prefix`.
    pub fn running_any(&self, prefix: &str) -> bool {
        self.jobs.values().any(|j| j.what.starts_with(prefix))
    }

    fn on_msg(&mut self, id: u64, m: Msg) {
        if let Msg::Line(i, l) = &m {
            install::line(self, *i, l);
            return;
        }
        self.jobs.remove(&id);
        match m {
            Msg::Pre(v) => preflight::msg(self, v),
            Msg::Found(v) => machines::found(self, v),
            Msg::Login(h, l) => machines::login(self, &h, l),
            Msg::Trusted(h, r) => machines::trusted(self, &h, r),
            Msg::Probe(h, p) => {
                self.probes.insert(h.clone(), *p);
                agent_setup::probed(self, &h);
            }
            Msg::Server(h, up) => {
                self.hd.server.insert(h, up);
            }
            Msg::Herdr { local, saved } => {
                // a save queued for a machine herdr has already goes
                self.plan.actions.retain(
                    |(_, a)| !matches!(a, Action::HerdrSave { host, .. } if saved.contains(host)),
                );
                self.local_herdr = Some(local);
                self.saved = Some(saved);
            }
            Msg::Hooks(h, r) => agent_setup::hooks(self, &h, r),
            Msg::Sent(s, r) => notifications::sent(self, &s, r),
            Msg::Moshi(s) => self.nt.moshi = Some(s),
            Msg::Usage(v) => self.an.seen = Some(v),
            Msg::Import(src, r) => starters::imported(self, &src, r),
            Msg::Access(f) => self.ac.facts = Some(f),
            Msg::Integ(f) => self.ig.facts = Some(f),
            Msg::Applied(i, r) => review::applied(self, i, r),
            Msg::Installed(i, j, r) => review::installed(self, i, j, r),
            Msg::AgentDone(k, r) => review::agent_done(self, k, r),
            Msg::Line(..) => {}
            Msg::Note(bad, t) => self.toast(bad, t),
            Msg::OpenRouter(stored, said) => agent_setup::openrouter(self, stored, said),
        }
    }

    // ------------------------------------------------------------ toasts

    pub fn toast(&mut self, bad: bool, text: impl Into<String>) {
        self.toasts.push_back(Toast {
            text: text.into(),
            bad,
            at: Instant::now(),
        });
        while self.toasts.len() > 3 {
            self.toasts.pop_front();
        }
    }

    fn current_toast(&self) -> Option<&Toast> {
        self.toasts
            .back()
            .filter(|t| t.at.elapsed() < Duration::from_secs(6))
    }

    // ------------------------------------------------------------ state

    /// Keep where we are and what's chosen (state/setup.json).
    pub fn save(&self) {
        let marks: serde_json::Map<String, serde_json::Value> = Step::ALL
            .iter()
            .map(|s| (s.id().to_string(), self.marks[s.index()].word().into()))
            .collect();
        let v = serde_json::json!({
            "version": 1,
            "at": self.cur.id(),
            "marks": marks,
            "plan": self.plan.to_json(),
            "finished": self.finished,
            "updated": crate::cache::now(),
        });
        let _ = config::write_private(
            &self.state_path,
            serde_json::to_string_pretty(&v)
                .unwrap_or_default()
                .as_bytes(),
        );
    }

    // ------------------------------------------------------------ moving

    fn enter(&mut self, s: Step) {
        match s {
            Step::Preflight => preflight::enter(self),
            Step::Look => look::enter(self),
            Step::Machines => machines::enter(self),
            Step::Herdr => herdr_setup::enter(self),
            Step::Agents => agent_setup::enter(self),
            Step::Notify => notifications::enter(self),
            Step::Analytics => analytics::enter(self),
            Step::Automation => {}
            Step::Access => access::enter(self),
            Step::Integrations => integrations::enter(self),
            Step::Presets => {}
            Step::Review => review::enter(self),
        }
    }

    /// Open step `s`.
    pub fn go(&mut self, s: Step) {
        if s != self.cur {
            look::leave(self);
        }
        self.cur = s;
        self.cell = 0;
        self.loose = false;
        self.sync_sel();
        self.enter(s);
        self.save();
    }

    /// Where Continue goes from `s`: the next step, or Review when the run
    /// was for one step.
    fn next_of(&self, s: Step) -> Step {
        if self.opts.only.is_some() {
            Step::Review
        } else {
            s.next().unwrap_or(Step::Review)
        }
    }

    /// Done with this step: mark it, then the next one.
    fn next(&mut self) {
        let s = self.cur;
        if s == Step::Review {
            review::enter_key(self);
            return;
        }
        self.marks[s.index()] = self.mark_of(s);
        self.go(self.next_of(s));
    }

    fn mark_of(&self, s: Step) -> Mark {
        let rs = self.rows_of(s);
        let bad = rs.iter().any(|r| {
            matches!(
                r.kind,
                Kind::Check {
                    st: St::Bad | St::Warn,
                    ..
                }
            )
        });
        if bad {
            Mark::Warn
        } else {
            Mark::Done
        }
    }

    fn back(&mut self) {
        match self.cur.prev() {
            Some(p) => self.go(p),
            None => self.ask_quit(),
        }
    }

    fn skip(&mut self) {
        let s = self.cur;
        let n = self.plan.drop_step(s);
        self.marks[s.index()] = Mark::Skipped;
        if n > 0 {
            self.toast(
                false,
                format!(
                    "skipped {} — its {n} change{} dropped",
                    s.title(),
                    if n == 1 { "" } else { "s" }
                ),
            );
        }
        if s == Step::Review {
            self.ask_quit();
            return;
        }
        self.go(s.next().unwrap_or(Step::Review));
    }

    fn ask_quit(&mut self) {
        let n = self.plan.len();
        self.mode = Overlay::Confirm {
            title: "Leave setup?".into(),
            lines: vec![Line::raw(if n == 0 {
                "Nothing was changed. aleph setup opens it again.".to_string()
            } else {
                format!(
                    "Nothing has been written. Your {n} choice{} {} kept — aleph setup picks up here.",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "is" } else { "are" }
                )
            })],
            yes: Yes::Quit,
        };
    }

    // ------------------------------------------------------------ rows

    /// The step's rows, then (before Review, whose own buttons apply and
    /// finish) a Continue button to the next step.
    pub(crate) fn rows_of(&self, s: Step) -> Vec<Row> {
        let mut v = match s {
            Step::Preflight => preflight::rows(self),
            Step::Look => look::rows(self),
            Step::Machines => machines::rows(self),
            Step::Herdr => herdr_setup::rows(self),
            Step::Agents => agent_setup::rows(self),
            Step::Notify => notifications::rows(self),
            Step::Analytics => analytics::rows(self),
            Step::Automation => automation::rows(self),
            Step::Access => access::rows(self),
            Step::Integrations => integrations::rows(self),
            Step::Presets => starters::rows(self),
            Step::Review => review::rows(self),
        };
        if s != Step::Review {
            if v.last().is_some_and(|r| *r != Row::blank()) {
                v.push(Row::blank());
            }
            v.push(
                Row::button(NEXT, format!("Continue → {}", self.next_of(s).title())).why(
                    "Done with this step: it's marked on the rail (✓, or ! when a check wants a look) and the next one opens. ↓ past the last row goes on without marking it; ⇧⇥ (or esc) goes back a step; s skips it and drops its changes.",
                ),
            );
        }
        v
    }

    fn aside_of(&self, s: Step, w: u16) -> Option<Aside> {
        match s {
            Step::Look => look::aside(self, w),
            Step::Agents => agent_setup::aside(self, w),
            Step::Analytics => analytics::aside(self, w),
            Step::Automation => automation::aside(self, w),
            Step::Presets => starters::aside(self, w),
            Step::Review => review::aside(self, w),
            Step::Machines => machines::aside(self, w),
            _ => None,
        }
    }

    fn act(&mut self, ev: Ev) -> Option<Hand> {
        self.act_on(self.cur, ev)
    }

    fn act_on(&mut self, s: Step, ev: Ev) -> Option<Hand> {
        let h = match s {
            Step::Preflight => preflight::act(self, ev),
            Step::Look => look::act(self, ev),
            Step::Machines => machines::act(self, ev),
            Step::Herdr => herdr_setup::act(self, ev),
            Step::Agents => agent_setup::act(self, ev),
            Step::Notify => notifications::act(self, ev),
            Step::Analytics => analytics::act(self, ev),
            Step::Automation => automation::act(self, ev),
            Step::Access => access::act(self, ev),
            Step::Integrations => integrations::act(self, ev),
            Step::Presets => starters::act(self, ev),
            Step::Review => review::act(self, ev),
        };
        self.save();
        h
    }

    fn retry(&mut self) {
        match self.cur {
            Step::Preflight => preflight::retry(self),
            Step::Machines => machines::retry(self),
            Step::Herdr | Step::Agents => agent_setup::retry(self),
            Step::Notify => notifications::retry(self),
            Step::Analytics => analytics::enter(self),
            Step::Access => access::retry(self),
            Step::Integrations => integrations::retry(self),
            Step::Review => review::retry(self),
            _ => self.toast(false, "nothing to check again here"),
        }
    }

    /// The current step's rows, with the cursor back on its row: found by
    /// what the row is (rows come and go as answers arrive), else the
    /// nearest one it can stop at. Until it's been moved (or used) it's on
    /// the first row it can stop at, whichever that is by now.
    fn sync_sel(&mut self) -> Vec<Row> {
        let rs = self.rows_of(self.cur);
        let i = self.cur.index();
        if self.sel_id[i].is_empty() {
            // the first row to act on — never quick start by itself (enter
            // there installs), though Home and ↑ reach it
            self.sel[i] = rs
                .iter()
                .position(|r| r.selectable() && r.ident() != "id:quick")
                .or_else(|| rs.iter().position(Row::selectable))
                .unwrap_or(0);
            return rs;
        }
        let j = place(&rs, self.sel[i], &self.sel_id[i]);
        self.sel[i] = j;
        self.sel_id[i] = rs.get(j).map(Row::ident).unwrap_or_default();
        rs
    }

    /// The cursor stays on the row it's on from now on (a key used it).
    fn pin_sel(&mut self, rs: &[Row]) {
        let i = self.cur.index();
        if let Some(r) = rs.get(self.sel[i]).filter(|r| r.selectable()) {
            self.sel_id[i] = r.ident();
        }
    }

    /// Put the cursor on row `j` (of `rs`, the current rows). The matrix
    /// cell stays when it's another row of the same matrix.
    fn point(&mut self, rs: &[Row], j: usize) {
        let i = self.cur.index();
        let (from, to) = (rs.get(self.sel[i]), rs.get(j));
        let same = match (from.and_then(Row::matrix_of), to.and_then(Row::matrix_of)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        if same {
            if let Some(Kind::Matrix { cols, .. }) = to.map(|r| &r.kind) {
                self.cell = self.cell.min(cols.len().saturating_sub(1));
            }
        } else {
            self.cell = 0;
        }
        self.sel[i] = j;
        self.sel_id[i] = to.map(Row::ident).unwrap_or_default();
    }

    /// The cursor moved with a key: onto row `j`, the view following it. On
    /// the first row the view starts at the top (what's above it shows), on
    /// the last it goes to the end.
    fn put_sel(&mut self, rs: &[Row], j: usize) {
        self.point(rs, j);
        self.loose = false;
        let i = self.cur.index();
        if Some(j) == rs.iter().position(Row::selectable) {
            self.top[i] = 0;
        } else if Some(j) == rs.iter().rposition(Row::selectable) {
            self.top[i] = usize::MAX;
        }
    }

    /// Nothing more to stop at in that direction: if the step goes on out
    /// of sight that way, the view moves half a page (true) — text before
    /// the first row or after the last can always be read.
    fn reveal(&mut self, rs: &[Row], down: bool) -> bool {
        let s = self.sel[self.cur.index()];
        let (h, w) = self.view_size();
        let disp = self.lines(rs, s, w);
        let (a, b, ..) = window(disp.len(), h, self.top_for(rs, &disp, s, h));
        if (down && b < disp.len()) || (!down && a > 0) {
            let d = (h / 2).max(1) as isize;
            self.scroll(if down { d } else { -d });
            return true;
        }
        false
    }

    /// ↑↓: the next row the cursor can stop at. Past the last one (or
    /// before the first), once the rest of the step has been shown, the
    /// next (previous) step opens on its first (last) row — only opened,
    /// not marked; at either end of setup it stays.
    fn move_sel(&mut self, down: bool) {
        let rs = self.sync_sel();
        let s = self.sel[self.cur.index()];
        let found = if down {
            (s + 1..rs.len()).find(|&j| rs[j].selectable())
        } else {
            (0..s.min(rs.len())).rev().find(|&j| rs[j].selectable())
        };
        if let Some(j) = found {
            self.put_sel(&rs, j);
            return;
        }
        if self.reveal(&rs, down) {
            return;
        }
        let Some(to) = (if down {
            self.cur.next()
        } else {
            self.cur.prev()
        }) else {
            return;
        };
        self.go(to);
        let rs = self.rows_of(to);
        let j = if down {
            rs.iter().position(Row::selectable)
        } else {
            rs.iter().rposition(Row::selectable)
        };
        self.put_sel(&rs, j.unwrap_or(0));
    }

    /// Home / End: the first or the last row the cursor can stop at.
    fn ends(&mut self, last: bool) {
        let rs = self.sync_sel();
        let j = if last {
            rs.iter().rposition(Row::selectable)
        } else {
            rs.iter().position(Row::selectable)
        };
        if let Some(j) = j {
            self.put_sel(&rs, j);
        }
    }

    /// The list's size as last drawn (a guess before the first frame).
    fn view_size(&self) -> (usize, usize) {
        let (h, w) = self.view;
        (if h == 0 { 12 } else { h }, if w == 0 { 80 } else { w })
    }

    /// Page up / down: half the lines the list shows, onto the farthest row
    /// the cursor can stop at within that (at least the next one).
    fn page(&mut self, down: bool) {
        let rs = self.sync_sel();
        let s = self.sel[self.cur.index()];
        let (h, w) = self.view_size();
        let first = first_lines(&self.lines(&rs, s, w), rs.len());
        let at = first.get(s).copied().unwrap_or(0);
        let half = (h / 2).max(1);
        let cand: Vec<usize> = if down {
            (s + 1..rs.len()).filter(|&j| rs[j].selectable()).collect()
        } else {
            (0..s.min(rs.len()))
                .rev()
                .filter(|&j| rs[j].selectable())
                .collect()
        };
        let Some(&near) = cand.first() else {
            self.reveal(&rs, down);
            return;
        };
        let j = cand
            .iter()
            .copied()
            .take_while(|&j| {
                if down {
                    first[j] <= at + half
                } else {
                    first[j] + half >= at
                }
            })
            .last()
            .unwrap_or(near);
        self.put_sel(&rs, j);
    }

    /// The wheel (or a click on a `more` line): the view moves by `d` lines
    /// and stays there; the cursor comes along only when it would be out of
    /// sight (onto the nearest row in view).
    fn scroll(&mut self, d: isize) {
        let rs = self.sync_sel();
        if rs.is_empty() {
            return;
        }
        let i = self.cur.index();
        let s = self.sel[i];
        let (h, w) = self.view_size();
        let disp = self.lines(&rs, s, w);
        let n = disp.len();
        let mt = max_top(n, h);
        let now = self.top_for(&rs, &disp, s, h);
        let top = (now as isize + d).clamp(0, mt as isize) as usize;
        self.top[i] = top;
        self.loose = true;
        let (a, b, ..) = window(n, h, top);
        let at = disp.iter().position(|(j, _)| *j == s).unwrap_or(0);
        if (a..b).contains(&at) {
            return;
        }
        let mut seen = disp[a..b]
            .iter()
            .map(|(j, _)| *j)
            .filter(|&j| rs[j].selectable());
        let j = if at < a {
            seen.next()
        } else {
            seen.next_back()
        };
        if let Some(j) = j {
            self.point(&rs, j);
        }
    }

    /// The row under the cursor (None when it's on nothing it can act on).
    pub(crate) fn selected(&self) -> Option<Row> {
        let rs = self.rows_of(self.cur);
        rs.get(self.sel[self.cur.index()])
            .filter(|r| r.selectable())
            .cloned()
    }

    /// Put the cursor on the row whose label starts with `label` (after a
    /// rebuild).
    pub(crate) fn select_label(&mut self, label: &str) {
        let rs = self.rows_of(self.cur);
        if let Some(j) = rs
            .iter()
            .position(|r| r.selectable() && r.label.starts_with(label))
        {
            self.point(&rs, j);
            self.loose = false;
        }
    }

    // ------------------------------------------------------------ keys

    fn key(&mut self, k: KeyEvent) -> Option<Exit> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(k.code, KeyCode::Char('c')) {
            self.save();
            return Some(Exit::Quit);
        }
        match std::mem::replace(&mut self.mode, Overlay::None) {
            Overlay::None => {}
            Overlay::Why { title, text } => {
                if !matches!(
                    k.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')
                ) {
                    self.mode = Overlay::Why { title, text };
                }
                return None;
            }
            Overlay::Confirm { title, lines, yes } => {
                return match k.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => self.accept(yes),
                    KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => None,
                    _ => {
                        self.mode = Overlay::Confirm { title, lines, yes };
                        None
                    }
                };
            }
            Overlay::Edit {
                title,
                hint,
                mut field,
                secret,
                err: _,
                to,
            } => {
                match k.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        let text = field.text.trim().to_string();
                        match &to {
                            EditFor::Set(key) => {
                                if let Err(e) = check_value(key, &text) {
                                    self.mode = Overlay::Edit {
                                        title,
                                        hint,
                                        field,
                                        secret,
                                        err: Some(e),
                                        to,
                                    };
                                    return None;
                                }
                                self.set(key, &text);
                            }
                            EditFor::Step(s, id) => {
                                let (s, id) = (*s, id.clone());
                                if let Some(h) = self.act_on(s, Ev::Input(id, text)) {
                                    return Some(Exit::Hand(h));
                                }
                            }
                        }
                    }
                    _ => {
                        field.key(&k);
                        self.mode = Overlay::Edit {
                            title,
                            hint,
                            field,
                            secret,
                            err: None,
                            to,
                        };
                    }
                }
                return None;
            }
        }
        if let Some(e) = self.exit.take() {
            return Some(e);
        }
        if install::shown(self) {
            return install::key(self, k);
        }
        let rows = self.sync_sel();
        let row = rows
            .get(self.sel[self.cur.index()])
            .filter(|r| r.selectable())
            .cloned();
        if matches!(
            k.code,
            KeyCode::Enter
                | KeyCode::Char(' ')
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char('h')
                | KeyCode::Char('l')
        ) {
            // what it changes may move it (a pick sorts up): the cursor goes along
            self.pin_sel(&rows);
        }
        match k.code {
            KeyCode::Char('q') => {
                self.ask_quit();
                return None;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(true),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(false),
            KeyCode::PageDown => self.page(true),
            KeyCode::PageUp => self.page(false),
            KeyCode::Home | KeyCode::Char('g') => self.ends(false),
            KeyCode::End | KeyCode::Char('G') => self.ends(true),
            KeyCode::Tab | KeyCode::Char(']') => {
                if let Some(n) = self.cur.next() {
                    self.go(n);
                }
            }
            KeyCode::BackTab | KeyCode::Char('[') => {
                if let Some(p) = self.cur.prev() {
                    self.go(p);
                }
            }
            KeyCode::Esc => self.back(),
            KeyCode::Char('s') => self.skip(),
            KeyCode::Char('r') => self.retry(),
            KeyCode::Char('?') => self.why(row.as_ref()),
            KeyCode::Left | KeyCode::Char('h') => return self.change(row.as_ref(), -1),
            KeyCode::Right | KeyCode::Char('l') => return self.change(row.as_ref(), 1),
            KeyCode::Char(' ') => return self.space(row.as_ref()),
            KeyCode::Enter => return self.enter_row(row.as_ref()),
            _ => {}
        }
        None
    }

    fn accept(&mut self, y: Yes) -> Option<Exit> {
        match y {
            Yes::Step(s, id) => self.act_on(s, Ev::Confirm(id)).map(Exit::Hand),
            Yes::Hand(h) => Some(Exit::Hand(*h)),
            Yes::Apply => {
                review::start(self);
                None
            }
            Yes::Quit => {
                self.save();
                Some(Exit::Quit)
            }
        }
    }

    fn why(&mut self, row: Option<&Row>) {
        let (title, text) = match row {
            // Continue: what it does, then what the step's read-only checks say
            Some(r) if matches!(&r.kind, Kind::Button { id, .. } if id == NEXT) => (
                r.label.clone(),
                format!("{}{}", r.why, self.checks_detail()),
            ),
            Some(r) if !r.why.is_empty() => (
                if r.label.is_empty() {
                    self.cur.title().to_string()
                } else {
                    r.label.clone()
                },
                r.why.clone(),
            ),
            Some(r) => match &r.kind {
                Kind::Choice { key, .. } | Kind::Toggle { key, .. } | Kind::Input { key, .. } => (
                    key.clone(),
                    config::known(key)
                        .map(|k| {
                            format!(
                                "{}\n\nsetting: {key} (default {})",
                                k.help,
                                if k.default.is_empty() {
                                    "empty"
                                } else {
                                    k.default
                                }
                            )
                        })
                        .unwrap_or_else(|| format!("setting: {key}")),
                ),
                _ => (self.cur.title().to_string(), self.step_detail()),
            },
            None => (self.cur.title().to_string(), self.step_detail()),
        };
        self.mode = Overlay::Why { title, text };
    }

    /// `?` away from a row of its own: the step's intro, then what each of
    /// its read-only checks says (the cursor passes them by, so this is
    /// where their details are).
    fn step_detail(&self) -> String {
        format!("{}{}", self.cur.intro(), self.checks_detail())
    }

    /// What the step's read-only checks say (`label — why`), each on its own.
    fn checks_detail(&self) -> String {
        let mut t = String::new();
        for r in self.rows_of(self.cur) {
            if matches!(&r.kind, Kind::Check { fix: None, .. }) && !r.why.is_empty() {
                t.push_str(&format!("\n\n{} — {}", r.label, r.why));
            }
        }
        t
    }

    /// ←→ on a row.
    fn change(&mut self, row: Option<&Row>, d: i32) -> Option<Exit> {
        let r = row?;
        match &r.kind {
            Kind::Choice { key, opts, cur, .. } => {
                if opts.is_empty() {
                    return None;
                }
                let i = opts.iter().position(|(v, _)| v == cur).unwrap_or(0) as i32;
                let n = opts.len() as i32;
                let j = ((i + d) % n + n) % n;
                let (key, v) = (key.clone(), opts[j as usize].0.clone());
                self.set(&key, &v);
                look::changed(self, &key);
            }
            Kind::Local { id, opts, cur } => {
                if opts.is_empty() {
                    return None;
                }
                let n = opts.len() as i32;
                let j = ((*cur as i32 + d) % n + n) % n;
                return self.act(Ev::Local(id.clone(), j as usize)).map(Exit::Hand);
            }
            Kind::Toggle { key, on, .. } => {
                let key = key.clone();
                self.set(&key, if *on { "false" } else { "true" });
                look::changed(self, &key);
            }
            Kind::Matrix { cols, .. } => {
                let n = cols.len().max(1) as i32;
                self.cell = (((self.cell as i32 + d) % n + n) % n) as usize;
            }
            _ => {}
        }
        None
    }

    fn space(&mut self, row: Option<&Row>) -> Option<Exit> {
        let r = row?;
        match &r.kind {
            Kind::Pick { id, .. } => self.act(Ev::Pick(id.clone())).map(Exit::Hand),
            Kind::Matrix { id, .. } => {
                let c = self.cell;
                self.act(Ev::Cell(id.clone(), c)).map(Exit::Hand)
            }
            Kind::Choice { .. } | Kind::Toggle { .. } | Kind::Local { .. } => self.change(row, 1),
            _ => self.enter_row(row),
        }
    }

    /// Enter: a value row changes (like space), text opens its editor, a
    /// button runs, a check's fix is carried out; Continue (or nothing to
    /// act on) marks the step and opens the next.
    fn enter_row(&mut self, row: Option<&Row>) -> Option<Exit> {
        let Some(r) = row else {
            self.next();
            return self.exit.take();
        };
        match &r.kind {
            Kind::Choice { .. }
            | Kind::Toggle { .. }
            | Kind::Local { .. }
            | Kind::Pick { .. }
            | Kind::Matrix { .. } => self.space(row),
            Kind::Button { id } if id == NEXT => {
                self.next();
                self.exit.take()
            }
            Kind::Input {
                key, cur, secret, ..
            } => {
                let v = if *secret && cur.starts_with("secret:") {
                    String::new()
                } else {
                    cur.clone()
                };
                self.mode = Overlay::Edit {
                    title: r.label.clone(),
                    hint: config::known(key)
                        .map(|k| k.help.to_string())
                        .unwrap_or_default(),
                    field: Field::new(&v),
                    secret: *secret,
                    err: None,
                    to: EditFor::Set(key.clone()),
                };
                None
            }
            Kind::Button { id } => self.act(Ev::Button(id.clone())).map(Exit::Hand),
            Kind::Check {
                id, fix: Some(_), ..
            } => self.act(Ev::Fix(id.clone())).map(Exit::Hand),
            _ => {
                self.next();
                self.exit.take()
            }
        }
    }

    /// Ask for text (a step's own field).
    pub(crate) fn edit(&mut self, title: &str, hint: &str, value: &str, id: &str) {
        self.mode = Overlay::Edit {
            title: title.to_string(),
            hint: hint.to_string(),
            field: Field::new(value),
            secret: false,
            err: None,
            to: EditFor::Step(self.cur, id.to_string()),
        };
    }

    /// Ask for a secret (a step's own field, shown as dots): it goes to the
    /// step's `Ev::Input` and nowhere else — never the plan.
    pub(crate) fn edit_secret(&mut self, title: &str, hint: &str, id: &str) {
        self.mode = Overlay::Edit {
            title: title.to_string(),
            hint: hint.to_string(),
            field: Field::new(""),
            secret: true,
            err: None,
            to: EditFor::Step(self.cur, id.to_string()),
        };
    }

    /// Ask before doing something (a step's own confirm).
    pub(crate) fn confirm(&mut self, title: &str, lines: Vec<Line<'static>>, id: &str) {
        self.mode = Overlay::Confirm {
            title: title.to_string(),
            lines,
            yes: Yes::Step(self.cur, id.to_string()),
        };
    }

    /// Ask before lending the terminal to `h`.
    pub(crate) fn confirm_hand(&mut self, h: Hand, extra: Vec<Line<'static>>) {
        let mut lines = vec![
            Line::from(Span::raw(h.note.clone())),
            Line::raw(""),
            Line::from(vec![
                Span::styled("$ ", ui::faint()),
                Span::styled(h.run.shown(), ui::bold()),
            ]),
        ];
        lines.extend(extra);
        self.mode = Overlay::Confirm {
            title: h.title.clone(),
            lines,
            yes: Yes::Hand(Box::new(h)),
        };
    }

    fn mouse(&mut self, m: crossterm::event::MouseEvent) -> Option<Exit> {
        if !matches!(self.mode, Overlay::None) || install::shown(self) {
            return None;
        }
        let inside = |r: Rect| {
            m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height
        };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, s)) = self.hits.rail.iter().find(|(r, _)| inside(*r)) {
                    let s = *s;
                    self.go(s);
                    return None;
                }
                // a `more` line scrolls; it never picks a row out of sight
                if let Some((_, up)) = self.hits.more.iter().find(|(r, _)| inside(*r)) {
                    let d = (self.view_size().0 / 2).max(1) as isize;
                    self.scroll(if *up { -d } else { d });
                    return None;
                }
                if let Some((_, i)) = self.hits.rows.iter().find(|(r, _)| inside(*r)) {
                    let i = *i;
                    let rs = self.sync_sel();
                    if rs.get(i).is_some_and(Row::selectable) {
                        let again = self.sel[self.cur.index()] == i;
                        // the view stays where it is, under the pointer
                        self.point(&rs, i);
                        self.loose = true;
                        if again {
                            let r = rs.get(i).cloned();
                            return self.space(r.as_ref());
                        }
                    }
                }
            }
            // the wheel moves the view, not the cursor
            MouseEventKind::ScrollDown => self.scroll(WHEEL),
            MouseEventKind::ScrollUp => self.scroll(-WHEEL),
            _ => {}
        }
        None
    }

    // ------------------------------------------------------------ loop

    /// Run until it ends or needs the terminal.
    pub(crate) fn run(&mut self, tui: &mut Tui) -> std::io::Result<Exit> {
        loop {
            while let Ok((id, m)) = self.rx.try_recv() {
                self.on_msg(id, m);
            }
            quick::tick(self);
            if let Some(h) = review::pump(self) {
                return Ok(Exit::Hand(h));
            }
            if let Some(e) = self.exit.take() {
                return Ok(e);
            }
            tui.term.draw(|f| {
                self.draw(f);
                ui::finish(f.buffer_mut());
                crate::logo::images::frame(f.buffer_mut());
                crate::showcase::buffer(f.buffer_mut());
            })?;
            // (showcase: none while paused)
            let pics = crate::showcase::pictures(crate::logo::images::take());
            if !pics.is_empty() {
                use std::io::Write;
                let b = tui.term.backend_mut();
                let _ = b.write_all(pics.as_bytes());
                let _ = Write::flush(b);
            }
            if event::poll(Duration::from_millis(80))? {
                match event::read()? {
                    // showcase: the panic key (and, while paused, every key) is its
                    Event::Key(k)
                        if k.kind == KeyEventKind::Press
                            && crate::showcase::panic_key(&crate::tui::key_str(&k)) => {}
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        if let Some(e) = self.key(k) {
                            return Ok(e);
                        }
                    }
                    Event::Mouse(_) if crate::showcase::paused() => {}
                    Event::Mouse(m) => {
                        if let Some(e) = self.mouse(m) {
                            return Ok(e);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// A hand-off came back.
    fn after(&mut self, h: &Hand, r: Result<String, String>) {
        if let Err(e) = &r {
            self.toast(true, format!("{}: {e}", h.title));
        }
        self.cfg = Config::load();
        let _ = self.act_on(h.step, Ev::After(h.id.clone(), r));
    }

    // ------------------------------------------------------------ drawing

    pub(crate) fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        self.hits = Hits::default();
        // the cursor back on its row before anything reads it (the asides)
        self.sync_sel();
        if area.width < 30 || area.height < 8 {
            let l = vec![
                Line::from(vec![
                    Span::styled(format!("{} ", crate::brand::MARK), ui::fg(c::ACCENT())),
                    Span::styled("setup", ui::bold()),
                ]),
                Line::from(Span::styled(
                    ui::trunc("make the window bigger", area.width as usize),
                    ui::muted(),
                )),
            ];
            f.render_widget(Paragraph::new(l), area);
            return;
        }
        // Apply's queue takes the whole screen
        if install::shown(self) {
            install::draw(self, f, area);
            self.draw_overlay(f, area);
            return;
        }
        let [head, body, foot] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_header(f, head);
        let rail_w: u16 = match area.width {
            w if w >= 110 => 28,
            w if w >= 72 => 23,
            w if w >= 50 => 7,
            _ => 0,
        };
        let [rail, panel] =
            Layout::horizontal([Constraint::Length(rail_w), Constraint::Min(20)]).areas(body);
        if rail_w > 0 {
            self.draw_rail(f, rail);
        }
        self.draw_step(f, panel);
        self.draw_footer(f, foot);
        self.draw_overlay(f, area);
    }

    fn draw_header(&self, f: &mut Frame, r: Rect) {
        let mut l = vec![
            Span::styled(
                format!(" {} aleph setup", crate::brand::MARK),
                ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "   step {} of 12 · {}",
                    self.cur.index() + 1,
                    self.cur.title()
                ),
                ui::muted(),
            ),
        ];
        if self.opts.redo {
            l.push(Span::styled("  · again, from your settings", ui::faint()));
        }
        let n = self.plan.len();
        let mut right = vec![];
        if n > 0 {
            right.push(Span::styled(
                format!("{n} change{} waiting  ", if n == 1 { "" } else { "s" }),
                ui::fg(c::ACCENT()),
            ));
        }
        let (g, t) = crate::net::label(&self.net.state());
        if !t.is_empty() {
            let col = match self.net.state() {
                crate::net::State::Online { .. } => c::GREEN(),
                crate::net::State::Weak { .. } => c::YELLOW(),
                crate::net::State::Offline { .. } => c::DOWN(),
                _ => c::FAINT(),
            };
            right.push(Span::styled(format!("{g} {t} "), ui::fg(col)));
        }
        let rl = Line::from(right);
        let rw = ui::line_width(&rl) as u16;
        let left = rows::fit(l, r.width.saturating_sub(rw + 1) as usize);
        f.render_widget(Paragraph::new(left), r);
        if rw + 30 < r.width {
            f.render_widget(Paragraph::new(rl).right_aligned(), r);
        }
    }

    fn draw_rail(&mut self, f: &mut Frame, r: Rect) {
        let compact = r.width < 12;
        let block = ui::panel(if compact { "" } else { "Steps" }, false)
            .padding(ratatui::widgets::Padding::horizontal(u16::from(!compact)));
        let inner = block.inner(r);
        f.render_widget(block, r);
        let spin = ui::spinner(self.t0);
        let mut lines = Vec::new();
        let gap = inner.height as usize >= 2 * Step::ALL.len();
        for s in Step::ALL {
            let i = s.index();
            let here = s == self.cur;
            let (m, col) = match self.marks[i] {
                Mark::Done => ("✓", c::GREEN()),
                Mark::Warn => ("!", c::YELLOW()),
                Mark::Skipped => ("–", c::FAINT()),
                Mark::Todo if here => ("●", c::ACCENT()),
                Mark::Todo => ("○", c::FAINT()),
            };
            let busy = here && !self.jobs.is_empty();
            let w = inner.width as usize;
            let mut l = vec![
                Span::styled(if here { "┃" } else { " " }, ui::fg(c::ACCENT())),
                Span::styled(
                    if busy { spin } else { m },
                    ui::fg(col).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:>3} ", i + 1), ui::faint()),
            ];
            if !compact {
                let name_w = w.saturating_sub(8);
                l.push(Span::styled(
                    ui::pad(&ui::trunc(s.title(), name_w), name_w),
                    if here {
                        ui::bold().fg(c::ACCENT())
                    } else if self.marks[i] == Mark::Skipped {
                        ui::faint()
                    } else {
                        Style::new().fg(c::TEXT())
                    },
                ));
                if self.plan.of(s) > 0 {
                    l.push(Span::styled("•", ui::fg(c::ACCENT())));
                }
            }
            let mut line = rows::fit(l, w);
            if here {
                line = line.style(ui::selected());
            }
            let y = inner.y + lines.len() as u16;
            if y < inner.y + inner.height {
                self.hits
                    .rail
                    .push((Rect::new(inner.x, y, inner.width, 1), s));
            }
            lines.push(line);
            if gap && s != Step::Review {
                lines.push(Line::raw(""));
            }
        }
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_step(&mut self, f: &mut Frame, r: Rect) {
        let s = self.cur;
        let n = self.plan.of(s);
        let mut title = vec![Span::raw(format!("{} · {}", s.index() + 1, s.title()))];
        if n > 0 {
            title.push(Span::styled(
                format!("  {n} change{}", if n == 1 { "" } else { "s" }),
                ui::faint(),
            ));
        }
        let block = ui::panel(Line::from(title), true);
        let inner = block.inner(r);
        f.render_widget(block, r);
        if inner.width < 10 || inner.height < 2 {
            return;
        }
        let w = inner.width as usize;
        // the intro, then the rows (with the preview beside or under them),
        // then what's running
        let intro: Vec<Line> = ui::wrap(s.intro(), w)
            .into_iter()
            .take(if inner.height >= 16 { 3 } else { 1 })
            .map(|t| Line::from(Span::styled(t, ui::muted())))
            .collect();
        let ih = intro.len() as u16 + u16::from(inner.height >= 12);
        let jobs = self.job_line(w);
        let jh = u16::from(jobs.is_some());
        let [ia, body, ja] = Layout::vertical([
            Constraint::Length(ih.min(inner.height.saturating_sub(1))),
            Constraint::Min(1),
            Constraint::Length(jh),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(intro), ia);
        if let Some(j) = jobs {
            f.render_widget(Paragraph::new(j), ja);
        }
        let aside = self.aside_of(s, body.width);
        let (list, side) = match aside {
            Some(a) if body.width >= 96 => {
                let aw = (body.width * 42 / 100).clamp(34, 70);
                let [l, sd] =
                    Layout::horizontal([Constraint::Min(30), Constraint::Length(aw)]).areas(body);
                (l, Some((sd, a)))
            }
            Some(a) if body.height >= 14 => {
                // the rows keep what they need (up to half), the rest is the panel's
                let want = a.lines.len() as u16 + 2;
                let need = (self.rows_of(s).len() as u16 + 1).min(body.height / 2);
                let ah = want.min(body.height.saturating_sub(need)).max(5);
                let [l, sd] =
                    Layout::vertical([Constraint::Min(4), Constraint::Length(ah)]).areas(body);
                (l, Some((sd, a)))
            }
            _ => (body, None),
        };
        self.draw_rows(f, list);
        if let Some((sd, a)) = side {
            let b = ui::panel(a.title.as_str(), false)
                .padding(ratatui::widgets::Padding::horizontal(1));
            let ai = b.inner(sd);
            f.render_widget(b, sd);
            let lines: Vec<Line> = a
                .lines
                .into_iter()
                .take(ai.height as usize)
                .map(|l| rows::fit(l.spans, ai.width as usize))
                .collect();
            f.render_widget(Paragraph::new(lines), ai);
        }
    }

    /// Every row as its lines, `w` wide (text wraps; the rest is one line
    /// each), with the row each line belongs to.
    fn lines(&self, rs: &[Row], sel: usize, w: usize) -> Vec<(usize, Line<'static>)> {
        let lw = rows::label_width(rs, w);
        let spin = ui::spinner(self.t0);
        // the cursor shows only on a row it can stop at
        let sel = if rs.get(sel).is_some_and(Row::selectable) {
            sel
        } else {
            usize::MAX
        };
        let mut disp = Vec::new();
        for (j, row) in rs.iter().enumerate() {
            let l = rows::draw(row, j == sel, self.cell, usize::MAX / 4, lw, spin);
            if row.kind == Kind::Info && ui::line_width(&l) > w {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                let st = l
                    .spans
                    .iter()
                    .find(|s| !s.content.trim().is_empty())
                    .map(|s| s.style)
                    .unwrap_or_default();
                for t in ui::wrap(text.trim_start(), w.saturating_sub(2)) {
                    disp.push((j, Line::from(vec![Span::raw("  "), Span::styled(t, st)])));
                }
                continue;
            }
            let mut l = rows::draw(row, j == sel, self.cell, w, lw, spin);
            if j == sel {
                l = l.style(ui::selected());
            }
            disp.push((j, l));
        }
        disp
    }

    /// The first line the list shows of `disp` in `h` rows: where the mouse
    /// left it, else following the cursor's row (with the heading over it).
    fn top_for(&self, rs: &[Row], disp: &[(usize, Line<'static>)], sel: usize, h: usize) -> usize {
        let top = self.top[self.cur.index()];
        if self.loose {
            return top.min(max_top(disp.len(), h));
        }
        let at = disp.iter().position(|(j, _)| *j == sel).unwrap_or(0);
        follow(disp.len(), h, top, at, ctx_start(rs, disp, at, h))
    }

    fn draw_rows(&mut self, f: &mut Frame, r: Rect) {
        let rs = self.rows_of(self.cur);
        let i = self.cur.index();
        let (h, w) = (r.height as usize, r.width as usize);
        self.view = (h, w);
        if rs.is_empty() || h == 0 {
            return;
        }
        let sel = self.sel[i].min(rs.len() - 1);
        let disp = self.lines(&rs, sel, w);
        let n = disp.len();
        let top = self.top_for(&rs, &disp, sel, h);
        self.top[i] = top;
        // the `more` lines take rows of their own, never one a line is on
        let (a, b, up, down) = window(n, h, top);
        let more = |t: String| Line::from(Span::styled(t, ui::faint()));
        let mut lines = Vec::new();
        let mut y = r.y;
        if up {
            self.hits.more.push((Rect::new(r.x, y, r.width, 1), true));
            lines.push(more(format!("  ↑ {a} more")));
            y += 1;
        }
        for (j, l) in &disp[a..b] {
            self.hits.rows.push((Rect::new(r.x, y, r.width, 1), *j));
            lines.push(l.clone());
            y += 1;
        }
        if down {
            self.hits.more.push((Rect::new(r.x, y, r.width, 1), false));
            lines.push(more(format!("  ↓ {} more", n - b)));
        }
        f.render_widget(Paragraph::new(lines), r);
    }

    /// `ℵ Checking key login to gpu2 · 3s` for what's running.
    fn job_line(&self, w: usize) -> Option<Line<'static>> {
        let mut v: Vec<&Job> = self.jobs.values().collect();
        if v.is_empty() {
            return None;
        }
        v.sort_by_key(|j| j.t0);
        let j = v[0];
        let more = v.len() - 1;
        let mut l = vec![
            Span::styled(format!("{} ", ui::spinner(self.t0)), ui::fg(c::ACCENT())),
            Span::styled(format!("{}…", j.label), ui::muted()),
            Span::styled(format!(" {}s", j.t0.elapsed().as_secs()), ui::faint()),
        ];
        if more > 0 {
            l.push(Span::styled(format!("  +{more} more"), ui::faint()));
        }
        Some(rows::fit(l, w))
    }

    fn draw_footer(&self, f: &mut Frame, r: Rect) {
        if let Some(t) = self.current_toast() {
            let col = if t.bad { c::RED() } else { c::GREEN() };
            let l = vec![
                Span::raw(" "),
                Span::styled(
                    if t.bad { " ✗ " } else { " ✓ " },
                    Style::new()
                        .fg(c::BASE())
                        .bg(col)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        " {}",
                        ui::trunc(&t.text, (r.width as usize).saturating_sub(8))
                    ),
                    ui::fg(col),
                ),
            ];
            f.render_widget(Paragraph::new(Line::from(l)), r);
            return;
        }
        let row = self.selected();
        let enter = match row.as_ref() {
            Some(Row {
                kind: Kind::Button { id },
                ..
            }) if id == NEXT => "continue",
            Some(r) => r.enter_word(),
            None if self.cur == Step::Review => review::enter_word(self),
            None => "next",
        };
        let mut items: Vec<(&str, &str)> = vec![("↑↓", "move")];
        match row.as_ref().map(|r| &r.kind) {
            // enter and ←→ both change a choice
            Some(Kind::Choice { .. }) | Some(Kind::Local { .. }) => items.push(("⏎ ←→", enter)),
            Some(Kind::Matrix { .. }) => {
                items.push(("⏎", enter));
                items.push(("←→", "cell"));
            }
            _ => items.push(("⏎", enter)),
        }
        items.extend([
            ("⇥", "next step"),
            ("⇧⇥", "previous step"),
            ("s", "skip"),
            ("r", "retry"),
            ("?", "why"),
            ("q", "quit"),
        ]);
        f.render_widget(Paragraph::new(ui::hints_fit(&items, r.width)), r);
    }

    fn draw_overlay(&self, f: &mut Frame, area: Rect) {
        let w = 66u16.min(area.width.saturating_sub(2));
        if w < 20 {
            return;
        }
        // the panel's borders and padding (it grows with ui_scale)
        let pad = ui::padding();
        let tw = (w - 2 - pad.left - pad.right).max(10) as usize;
        let extra = 2 + pad.top + pad.bottom;
        match &self.mode {
            Overlay::None => {}
            Overlay::Why { title, text } => {
                let mut lines: Vec<Line> = Vec::new();
                for para in text.split('\n') {
                    if para.is_empty() {
                        lines.push(Line::raw(""));
                        continue;
                    }
                    lines.extend(ui::wrap(para, tw).into_iter().map(Line::from));
                }
                lines.push(Line::raw(""));
                lines.push(ui::hints(&[("esc", "close")]));
                let h = (lines.len() as u16 + extra).min(area.height.saturating_sub(2));
                let r = ui::centered(area, w, h);
                crate::ui::clear(f, r);
                f.render_widget(
                    Paragraph::new(lines).block(ui::panel(format!("? {title}"), true)),
                    r,
                );
            }
            Overlay::Confirm { title, lines, .. } => {
                let mut v: Vec<Line> = Vec::new();
                for l in lines {
                    if ui::line_width(l) <= tw {
                        v.push(l.clone());
                    } else {
                        let t: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                        let st = l.spans.first().map(|s| s.style).unwrap_or_default();
                        v.extend(
                            ui::wrap(&t, tw)
                                .into_iter()
                                .map(|x| Line::from(Span::styled(x, st))),
                        );
                    }
                }
                v.push(Line::raw(""));
                v.push(ui::hints(&[("y", "yes"), ("n", "no")]));
                let h = (v.len() as u16 + extra).min(area.height.saturating_sub(2));
                let r = ui::centered(area, w, h);
                crate::ui::clear(f, r);
                f.render_widget(Paragraph::new(v).block(ui::panel(title.as_str(), true)), r);
            }
            Overlay::Edit {
                title,
                hint,
                field,
                secret,
                err,
                ..
            } => {
                let mut v: Vec<Line> = Vec::new();
                for t in ui::wrap(hint, tw).into_iter().take(3) {
                    v.push(Line::from(Span::styled(t, ui::muted())));
                }
                v.push(Line::raw(""));
                let mut spans = vec![Span::styled("› ", ui::fg(c::ACCENT()))];
                if *secret && !field.text.is_empty() {
                    let n = field.text.chars().count();
                    spans.push(Span::raw("•".repeat(n.min(tw.saturating_sub(4)))));
                    spans.push(Span::styled(
                        " ",
                        Style::new().fg(c::BASE()).bg(c::ACCENT()),
                    ));
                } else {
                    spans.extend(field.spans(true, ""));
                }
                v.push(rows::fit(spans, tw));
                if let Some(e) = err {
                    v.push(Line::from(Span::styled(ui::trunc(e, tw), ui::fg(c::RED()))));
                }
                v.push(Line::raw(""));
                v.push(ui::hints(&[
                    ("enter", "save"),
                    ("esc", "cancel"),
                    ("ctrl+u", "clear"),
                ]));
                let h = (v.len() as u16 + extra).min(area.height.saturating_sub(2));
                let r = ui::centered(area, w, h);
                crate::ui::clear(f, r);
                f.render_widget(Paragraph::new(v).block(ui::panel(title.as_str(), true)), r);
            }
        }
    }
}

/// The documented default (empty for keys outside the registry).
pub fn default_of(key: &str) -> String {
    config::known(key)
        .map(|k| k.default.to_string())
        .unwrap_or_default()
}

/// Validate a value before it goes into the plan.
pub fn check_value(key: &str, v: &str) -> Result<(), String> {
    if v.is_empty() {
        return Ok(());
    }
    if key == "notify.quiet" && crate::router::parse_quiet(v).is_none() {
        return Err("quiet hours look like 23:00-08:00".into());
    }
    if key == "accent" && ui::parse_color(v).is_none() {
        return Err("a colour: gold, claude, a palette name or #rrggbb".into());
    }
    if key.starts_with("notify.")
        && (key.ends_with("ntfy") || key.ends_with("slack") || key.ends_with("webhook"))
        && !(v.starts_with("https://") || v.starts_with("http://") || v.starts_with("secret:"))
    {
        return Err("a URL: https://…".into());
    }
    config::check(key, v)
}

pub fn split(v: &str) -> Vec<String> {
    v.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

// ------------------------------------------------------------------ the cursor and the view (pure)

/// Where the cursor goes on (rebuilt) rows: the row that is `id` (the one
/// nearest `old` if there are several), else the stop nearest `old` (after
/// it on a tie). Only rows it can stop at; `old` when there are none.
pub(crate) fn place(rs: &[Row], old: usize, id: &str) -> usize {
    let near = |want: &dyn Fn(&Row) -> bool| {
        rs.iter()
            .enumerate()
            .filter(|(_, r)| r.selectable() && want(r))
            .min_by_key(|(j, _)| (j.abs_diff(old), *j < old))
            .map(|(j, _)| j)
    };
    (!id.is_empty())
        .then(|| near(&|r: &Row| r.ident() == id))
        .flatten()
        .or_else(|| near(&|_| true))
        .unwrap_or_else(|| old.min(rs.len().saturating_sub(1)))
}

/// The first line of each of `n` rows in `disp`.
fn first_lines(disp: &[(usize, Line<'static>)], n: usize) -> Vec<usize> {
    let mut v = vec![usize::MAX; n];
    for (k, (j, _)) in disp.iter().enumerate() {
        if let Some(x) = v.get_mut(*j) {
            *x = (*x).min(k);
        }
    }
    // a row with no line of its own sits where the next one starts
    let mut next = disp.len();
    for x in v.iter_mut().rev() {
        if *x == usize::MAX {
            *x = next;
        }
        next = *x;
    }
    v
}

/// The `more` lines need this much room.
const MARKS_FROM: usize = 4;

/// The last first line worth showing for `n` lines in `h` rows: the end in
/// view (under a `↑ more` line).
pub(crate) fn max_top(n: usize, h: usize) -> usize {
    if n <= h {
        0
    } else if h >= MARKS_FROM {
        n - (h - 1)
    } else {
        n - h
    }
}

/// What `h` rows show of `n` lines from line `top`: (first, end, `↑ more`,
/// `↓ more`). A `more` line takes a row of its own, so it never hides one.
pub(crate) fn window(n: usize, h: usize, top: usize) -> (usize, usize, bool, bool) {
    let top = top.min(max_top(n, h));
    let marks = h >= MARKS_FROM;
    let up = marks && top > 0;
    let room = h - usize::from(up);
    if top + room >= n {
        return (top, n, up, false);
    }
    (top, top + room - usize::from(marks), up, marks)
}

/// The first line of what belongs over line `at`: the text right above it
/// up to (and with) its heading — a few lines at most, so `at` stays in
/// view under the `more` lines.
pub(crate) fn ctx_start(rs: &[Row], disp: &[(usize, Line<'static>)], at: usize, h: usize) -> usize {
    let room = h.saturating_sub(3).min(4);
    let mut k = at.min(disp.len());
    while k > 0 && at - k < room {
        let j = disp[k - 1].0;
        if rs[j].selectable() {
            break;
        }
        k -= 1;
        if rs[j].kind == Kind::Head {
            break;
        }
    }
    k
}

/// The first line to show so that line `at` is in view, moving as little
/// as it can from `top`; the lines from `ctx` (its heading) come along when
/// it moves up to them.
pub(crate) fn follow(n: usize, h: usize, top: usize, at: usize, ctx: usize) -> usize {
    let mt = max_top(n, h);
    let mut t = top.min(mt);
    let (a, b, ..) = window(n, h, t);
    if ctx < a {
        t = ctx;
    } else if at >= b {
        while t < mt && window(n, h, t).1 <= at {
            t += 1;
        }
    }
    t.min(mt)
}

// ------------------------------------------------------------------ running it

/// Lend the terminal to `h` on the same alternate screen, then take it back.
fn run_hand(tui: &mut Tui, h: &Hand) -> Result<String, String> {
    use crate::out::{ask, say};
    let gone = crate::logo::images::clear();
    {
        use std::io::Write;
        let b = tui.term.backend_mut();
        if !gone.is_empty() {
            let _ = b.write_all(gone.as_bytes());
        }
        let _ = b.write_all(b"\x1b[2J\x1b[H");
        let _ = Write::flush(b);
    }
    let _ = tui.suspend_keep_screen();
    say(Line::from(vec![
        Span::styled(
            format!("{} ", crate::brand::MARK),
            ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("aleph setup · {}", h.title), ui::bold()),
    ]));
    if !h.note.is_empty() {
        say(Line::from(Span::styled(
            format!("  {}", h.note),
            ui::muted(),
        )));
    }
    let rule = |t: &str| {
        let w = 64usize.saturating_sub(ui::width(t) + 4);
        say(Line::from(vec![
            Span::styled("── ", ui::faint()),
            Span::styled(t.to_string(), ui::muted()),
            Span::styled(format!(" {}", "─".repeat(w)), ui::faint()),
        ]));
    };
    rule(&h.run.shown());
    let r = match &h.run {
        Run::Cmd { prog, args, cwd } => {
            let mut c = std::process::Command::new(prog);
            c.args(args);
            if let Some(d) = cwd {
                c.current_dir(d);
            }
            match c.status() {
                Ok(s) if s.success() => Ok("done".to_string()),
                Ok(s) => Err(match s.code() {
                    Some(n) => format!("exited with status {n}"),
                    None => "stopped".into(),
                }),
                Err(e) => Err(format!("couldn't run {prog}: {e}")),
            }
        }
        Run::KeySetup(host) => crate::sshcfg::setup_key(host).map(|_| "key login works".into()),
    };
    rule("back to aleph setup");
    match &r {
        Ok(_) => crate::out::ok("done"),
        Err(e) => crate::out::err(e),
    }
    let _ = ask("Press enter to return to setup", "");
    let _ = tui.resume();
    r
}

/// `aleph setup` full screen. After the first run (and when you ask for it
/// at the end) the dashboard opens on the same screen, with the tour.
pub fn run(cfg: Config, opts: Opts) -> ExitCode {
    use crate::out::fail;
    let mut tui = match Tui::enter() {
        Ok(t) => t,
        Err(e) => return fail(&format!("terminal: {e}")),
    };
    crate::term::set_title(&format!("{} — setup", crate::brand::BIN));
    crate::logo::images::set_output(true);
    let first = opts.first;
    let mut w = Wiz::new(cfg, opts, state_file(), false);
    w.start();
    loop {
        match w.run(&mut tui) {
            Ok(Exit::Quit) => {
                w.save();
                let applied = w.finished;
                let fleet = Config::load().hosts();
                // the first run goes on to the dashboard once there's a fleet
                if first && applied && !fleet.is_empty() {
                    return super::run_dashboard_in(tui, Config::load(), tour_start());
                }
                drop(tui);
                if applied {
                    crate::out::ok("setup is done — aleph opens the dashboard");
                } else if !w.plan.is_empty() {
                    crate::out::hint(&format!(
                        "{} choice{} kept for later — aleph setup picks up where you stopped",
                        w.plan.len(),
                        if w.plan.len() == 1 { "" } else { "s" }
                    ));
                }
                return ExitCode::SUCCESS;
            }
            Ok(Exit::Dashboard { tour }) => {
                w.save();
                let mut st = tour_start();
                st.tour = tour;
                return super::run_dashboard_in(tui, Config::load(), st);
            }
            Ok(Exit::Hand(h)) => {
                let r = run_hand(&mut tui, &h);
                w.after(&h, r);
            }
            Err(e) => {
                drop(tui);
                return fail(&format!("terminal: {e}"));
            }
        }
    }
}

/// One step of setup from inside the dashboard (Settings › `S`): on the
/// same screen, with today's values chosen; back to the dashboard when it's
/// applied or left.
pub(crate) fn run_step_in(tui: &mut Tui, cfg: Config, step: Step) -> Result<(), String> {
    let opts = Opts {
        redo: true,
        only: Some(step),
        ..Default::default()
    };
    let mut w = Wiz::new(cfg, opts, state_file(), false);
    w.start();
    loop {
        match w.run(tui) {
            Ok(Exit::Quit) | Ok(Exit::Dashboard { .. }) => {
                w.save();
                return Ok(());
            }
            Ok(Exit::Hand(h)) => {
                let r = run_hand(tui, &h);
                w.after(&h, r);
            }
            Err(e) => return Err(format!("terminal: {e}")),
        }
    }
}

/// Apply `want` on setup's install screen, on the dashboard's own terminal
/// (an agent installed from the new-session form, a doctor fix): live
/// output, retry and the log as in setup, no wizard around it, and none of
/// it saved as setup's answers. Enter (or esc) at the end goes back.
pub(crate) fn run_actions_in(
    tui: &mut Tui,
    cfg: Config,
    want: Vec<(Step, Action)>,
) -> Result<(), String> {
    let opts = Opts {
        redo: true,
        only: Some(Step::Review),
        ..Default::default()
    };
    let mut w = Wiz::new(cfg, opts, state_file(), false);
    w.queue_only = true;
    for (s, a) in want {
        w.plan.want(s, a, true);
    }
    review::start(&mut w);
    loop {
        match w.run(tui) {
            Ok(Exit::Quit) | Ok(Exit::Dashboard { .. }) => return Ok(()),
            Ok(Exit::Hand(h)) => {
                let r = run_hand(tui, &h);
                w.after(&h, r);
            }
            Err(e) => return Err(format!("terminal: {e}")),
        }
    }
}

fn tour_start() -> super::Start {
    super::Start {
        view: super::View::Fleet,
        tour: !Config::load().get("tour").is_some_and(|t| t == "done"),
        ..Default::default()
    }
}
