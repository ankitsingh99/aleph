//! The demo world: five machines and the sessions on them, each running a
//! script — a pure function of (seed, time, actions), so the daemon, the
//! dashboard and every CLI run agree without sharing anything but the
//! action log.

use super::story::{self, Story};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// ------------------------------------------------------------------ machines

/// One machine of the world.
pub struct Machine {
    pub alias: &'static str,
    /// `uname -s`
    pub kernel: &'static str,
    pub distro: &'static str,
    /// `uname -r`
    pub release: &'static str,
    pub arch: &'static str,
    pub cpu: &'static str,
    pub ncpu: usize,
    pub sockets: usize,
    pub mem_gb: f64,
    pub disk_gb: f64,
    /// (name, VRAM GiB, power limit W)
    pub gpus: &'static [(&'static str, f64, f64)],
    pub user: &'static str,
    /// a machine colour name (`theme::MACHINES`): its sessions' moods follow it
    pub color: &'static str,
    /// how busy its CPU is, on average (0–1)
    pub load: f64,
    /// down for `.1` seconds of every `.0` (its link drops)
    pub outage: Option<(u64, u64)>,
    /// its round trip, ms
    pub rtt: u64,
    pub herdr: &'static str,
}

impl Machine {
    pub fn home(&self) -> String {
        if self.kernel == "Darwin" {
            format!("/Users/{}", self.user)
        } else {
            format!("/home/{}", self.user)
        }
    }
    pub fn mac(&self) -> bool {
        self.kernel == "Darwin"
    }
    /// Does it answer at `t`?
    pub fn up(&self, t: u64) -> bool {
        match self.outage {
            Some((every, len)) => (t + 137) % every >= len,
            None => true,
        }
    }
    /// `Linux · Ubuntu 24.04.3 LTS` (what the fleet shows).
    pub fn os_line(&self) -> String {
        format!("{} · {}", self.kernel, self.distro)
    }
    /// What ssh says when it can't reach it.
    pub fn down_text(&self) -> String {
        format!(
            "ssh: connect to host {} port 22: Operation timed out",
            self.alias
        )
    }
}

pub const MACHINES: &[Machine] = &[
    Machine {
        alias: "heart-of-gold",
        kernel: "Darwin",
        distro: "macOS 26.1",
        release: "25.1.0",
        arch: "arm64",
        cpu: "Apple M4 Max",
        ncpu: 16,
        sockets: 1,
        mem_gb: 64.0,
        disk_gb: 2000.0,
        gpus: &[],
        user: "zaphod",
        color: "peach",
        load: 0.22,
        outage: None,
        rtt: 3,
        herdr: "0.9.5",
    },
    Machine {
        alias: "magrathea",
        kernel: "Linux",
        distro: "Ubuntu 24.04.3 LTS",
        release: "6.8.0-48-generic",
        arch: "x86_64",
        cpu: "AMD EPYC 9354 32-Core Processor",
        ncpu: 64,
        sockets: 1,
        mem_gb: 256.0,
        disk_gb: 7680.0,
        gpus: &[
            ("NVIDIA RTX 6000 Ada Generation", 48.0, 300.0),
            ("NVIDIA RTX 6000 Ada Generation", 48.0, 300.0),
        ],
        user: "slartibartfast",
        color: "mauve",
        load: 0.46,
        outage: None,
        rtt: 38,
        herdr: "0.9.5",
    },
    Machine {
        alias: "europa",
        kernel: "Linux",
        distro: "Debian GNU/Linux 12 (bookworm)",
        release: "6.1.0-26-amd64",
        arch: "x86_64",
        cpu: "Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz",
        ncpu: 16,
        sockets: 1,
        mem_gb: 64.0,
        disk_gb: 1920.0,
        gpus: &[],
        user: "ford",
        color: "sky",
        load: 0.31,
        outage: None,
        rtt: 61,
        herdr: "0.9.5",
    },
    Machine {
        alias: "tycho-base",
        kernel: "Linux",
        distro: "Fedora Linux 41 (Workstation Edition)",
        release: "6.11.4-301.fc41.x86_64",
        arch: "x86_64",
        cpu: "AMD Ryzen 9 7950X 16-Core Processor",
        ncpu: 32,
        sockets: 1,
        mem_gb: 128.0,
        disk_gb: 4000.0,
        gpus: &[("NVIDIA GeForce RTX 4090", 24.0, 450.0)],
        user: "naomi",
        color: "teal",
        load: 0.38,
        outage: None,
        rtt: 96,
        herdr: "0.9.5",
    },
    Machine {
        alias: "vogsphere",
        kernel: "Linux",
        distro: "Ubuntu 22.04.5 LTS",
        release: "5.15.0-124-generic",
        arch: "x86_64",
        cpu: "Intel(R) Core(TM) i5-8500T CPU @ 2.10GHz",
        ncpu: 6,
        sockets: 1,
        mem_gb: 16.0,
        disk_gb: 512.0,
        gpus: &[],
        user: "jeltz",
        color: "green",
        load: 0.18,
        outage: Some((600, 60)),
        rtt: 210,
        herdr: "0.9.4",
    },
];

pub fn machine(alias: &str) -> Option<&'static Machine> {
    MACHINES.iter().find(|m| m.alias == alias)
}

// ------------------------------------------------------------------ tasks

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K {
    Read,
    Edit,
    Run,
    Find,
}

/// One thing an agent does on the way (a tool call).
#[derive(Debug)]
pub struct Step {
    pub k: K,
    pub arg: &'static str,
    pub out: &'static str,
}

/// The question it stops at: a command it wants to run.
#[derive(Debug)]
pub struct Ask {
    pub cmd: &'static str,
    pub why: &'static str,
    /// what "don't ask again" covers (`cargo bench`)
    pub family: &'static str,
    /// what the command printed once it ran
    pub out: &'static str,
}

#[derive(Debug)]
pub struct Task {
    pub title: &'static str,
    /// the spinner's verb
    pub verb: &'static str,
    pub prompt: &'static str,
    /// the last one comes after the question
    pub steps: &'static [Step],
    pub ask: Ask,
    pub done: &'static str,
}

const fn st(k: K, arg: &'static str, out: &'static str) -> Step {
    Step { k, arg, out }
}

pub const TASKS: &[Task] = &[
    Task {
        title: "Recalibrating the Infinite Improbability Drive",
        verb: "Recalibrating",
        prompt: "the drive keeps landing us inside a whale — recalibrate the finite improbability generator and add a regression test",
        steps: &[
            st(K::Read, "src/generator.rs", "Read 214 lines"),
            st(K::Find, "brownian", "Found 7 matches across 3 files"),
            st(K::Edit, "src/generator.rs", "Updated src/generator.rs with 18 additions and 6 removals"),
            st(K::Run, "cargo test generator", "test result: ok. 41 passed; 0 failed; finished in 2.31s"),
        ],
        ask: Ask {
            cmd: "cargo bench --bench improbability -- --save-baseline tea",
            why: "Benchmark the generator against a hot cup of tea",
            family: "cargo bench",
            out: "improbability/whale  time: [1.204 ms 1.211 ms 1.219 ms]  change: -97.3%",
        },
        done: "Recalibrated: whale landings went from 1 in 2^276,709 to 1 in 2^4. Added tests/whale_regression.rs.",
    },
    Task {
        title: "Rendering the fjords of Earth Mk II",
        verb: "Rendering",
        prompt: "Slartibartfast wants the Norway coastline crinklier — tune the fjord generator and rerender the preview",
        steps: &[
            st(K::Read, "gen/fjords.py", "Read 388 lines"),
            st(K::Run, "python -m gen.fjords --preview norway", "fractal dimension 1.21 · 1,904 fjords"),
            st(K::Edit, "gen/fjords.py", "Updated gen/fjords.py with 31 additions and 9 removals"),
            st(K::Run, "pytest tests/test_coast.py -q", "12 passed in 4.02s"),
        ],
        ask: Ask {
            cmd: "python -m gen.render --gpus 0,1 --planet earth-mk2",
            why: "Render the whole planet on both GPUs",
            family: "python -m gen.render",
            out: "rendered 510,072,000 km² in 6m 41s → out/earth-mk2/",
        },
        done: "Coastline fractal dimension is 1.52 now (was 1.21). Preview in out/earth-mk2/norway.png — award-worthy crinkles.",
    },
    Task {
        title: "Computing the Question to 42",
        verb: "Computing",
        prompt: "we have the Answer (42). work backwards to the Question — start with the 7.5 million year log",
        steps: &[
            st(K::Read, "logs/compute.log", "7,500,000 years · 1 line of output"),
            st(K::Find, "answer", "src/answer.rs:3: pub const ANSWER: u8 = 42;"),
            st(K::Edit, "src/question.rs", "+42 -6"),
            st(K::Run, "cargo test -q", "test result: ok. 6 passed; 1 ignored (mice)"),
        ],
        ask: Ask {
            cmd: "cargo run --release -- --mode question --years 10000000",
            why: "Run the Question search for ten million years",
            family: "cargo run",
            out: "candidate: \"What do you get if you multiply six by nine?\" (base 13)",
        },
        done: "The Question is still under construction: the computer built to find it was demolished five minutes early.",
    },
    Task {
        title: "Indexing pulsar timing arrays",
        verb: "Indexing",
        prompt: "index the timing residuals by pulsar and epoch, then look for the gravitational-wave background",
        steps: &[
            st(K::Read, "pta/index.py", "Read 156 lines"),
            st(K::Run, "python -m pta.index --epochs", "68 pulsars × 1,342 epochs indexed"),
            st(K::Edit, "pta/hellings_downs.py", "Wrote 94 lines to pta/hellings_downs.py"),
            st(K::Run, "pytest -q", "23 passed in 9.81s"),
        ],
        ask: Ask {
            cmd: "python -m pta.search --gwb --chains 8",
            why: "Run the background search with 8 MCMC chains",
            family: "python -m pta.search",
            out: "Hellings–Downs correlation: 3.1σ · A = 2.4e-15",
        },
        done: "Indexed 68 pulsars over 1,342 epochs. Hellings–Downs correlation at 3.1σ — the background hums.",
    },
    Task {
        title: "Teaching the Babel fish Rust",
        verb: "Translating",
        prompt: "port the babel fish's universal translator core from C to Rust, and keep the FFI shim working",
        steps: &[
            st(K::Read, "src/translate.c", "Read 1,204 lines"),
            st(K::Edit, "src/lib.rs", "Wrote 812 lines to src/lib.rs"),
            st(K::Run, "cargo build --release", "Finished `release` profile [optimized] in 18.2s"),
            st(K::Run, "cargo test ffi", "test result: ok. 88 passed; 0 failed"),
        ],
        ask: Ask {
            cmd: "cargo publish --dry-run",
            why: "Check the crate packages cleanly",
            family: "cargo publish",
            out: "Packaged 14 files, 61.3KiB (17.9KiB compressed) · aborting upload due to dry run",
        },
        done: "Ported 1,204 lines of C to Rust; the FFI shim passes all 88 tests. The fish now speaks borrow checker.",
    },
    Task {
        title: "Tuning the subspace relay's error correction",
        verb: "Tuning",
        prompt: "the subspace relay drops packets past warp 6 — add Reed–Solomon error correction to every frame",
        steps: &[
            st(K::Read, "src/relay/frame.rs", "Read 302 lines"),
            st(K::Find, "checksum", "Found 11 matches across 4 files"),
            st(K::Edit, "src/relay/fec.rs", "Updated src/relay/fec.rs with 64 additions and 2 removals"),
            st(K::Run, "cargo test relay", "test result: ok. 37 passed; 0 failed"),
        ],
        ask: Ask {
            cmd: "cargo run --example warp-soak -- --warp 9.2 --minutes 5",
            why: "Soak-test the relay at warp 9.2",
            family: "cargo run --example",
            out: "warp 9.2 · 5m · 2,883,104 frames · loss 0.000%",
        },
        done: "Packet loss at warp 9: 0.0% (was 14%). Reed–Solomon (255,223) guards every frame now.",
    },
    Task {
        title: "Modelling the Epstein drive's fusion plume",
        verb: "Modelling",
        prompt: "model the Epstein drive's plume — the efficiency jump shouldn't melt the ship this time",
        steps: &[
            st(K::Read, "sim/plume.py", "Read 241 lines"),
            st(K::Edit, "sim/magnetic_nozzle.py", "Updated sim/magnetic_nozzle.py with 22 additions and 5 removals"),
            st(K::Run, "python -m sim.plume --thrust 4g", "peak plume 1.9e8 K · nozzle intact"),
            st(K::Run, "pytest sim -q", "31 passed in 12.4s"),
        ],
        ask: Ask {
            cmd: "python -m sim.plume --thrust 12g --crew 1",
            why: "Simulate a 12g burn with one crew member aboard",
            family: "python -m sim.plume",
            out: "12g sustained · crew: unconscious, alive · nozzle at 97% of limit",
        },
        done: "The plume stays inside the nozzle up to 11g. Recommend juice for anyone aboard past 8g.",
    },
    Task {
        title: "Mapping the ring gate's slow zone",
        verb: "Mapping",
        prompt: "map the slow zone's speed limit as a function of mass and velocity",
        steps: &[
            st(K::Run, "python -m ring.sample --probes 400", "400 probe runs · 17 stopped cold"),
            st(K::Read, "ring/limits.py", "Read 120 lines"),
            st(K::Edit, "ring/limits.py", "+58 -12"),
            st(K::Run, "pytest -q tests/test_limits.py", "9 passed in 1.12s"),
        ],
        ask: Ask {
            cmd: "python -m ring.probe --launch --mass 2e6",
            why: "Launch a two-thousand-tonne probe through the gate",
            family: "python -m ring.probe",
            out: "probe 2e6 kg at 540 m/s: through · the station hums",
        },
        done: "Speed limit fits v_max = 600 m/s · (m / 1e6 kg)^-0.31; plotted in out/slow_zone.svg.",
    },
    Task {
        title: "Optimising the Vogon poetry engine",
        verb: "Optimising",
        prompt: "the poetry engine is too slow to cause real suffering — profile it and make it ten times faster",
        steps: &[
            st(K::Run, "cargo flamegraph --bin recite", "flamegraph.svg written · 61% in stanza::rhyme"),
            st(K::Read, "src/stanza.rs", "Read 433 lines"),
            st(K::Edit, "src/stanza.rs", "Updated src/stanza.rs with 40 additions and 71 removals"),
            st(K::Run, "cargo bench", "recite/ode   time: [2.11 ms 2.13 ms 2.16 ms]  change: -91.2%"),
        ],
        ask: Ask {
            cmd: "./target/release/recite --audience arthur --volume 11",
            why: "Recite the new ode to a test audience",
            family: "./target/release/recite",
            out: "audience: gasping · ears: bleeding · verdict: \"interesting rhythmic devices\"",
        },
        done: "Recitation is 11.4× faster. Listeners' insides now try to leap out through their necks right on schedule.",
    },
    Task {
        title: "Calibrating the point-defence cannons",
        verb: "Calibrating",
        prompt: "the PDC fire-control solution lags by 40 ms — profile the tracker and bring it under 5",
        steps: &[
            st(K::Read, "pdc/tracker.ts", "Read 276 lines"),
            st(K::Run, "npm run bench -- tracker", "solve: 41.8 ms p50 · 63.0 ms p99"),
            st(K::Edit, "pdc/kalman.ts", "Updated pdc/kalman.ts with 47 additions and 19 removals"),
            st(K::Run, "npm test -- pdc", "Tests: 52 passed, 52 total"),
        ],
        ask: Ask {
            cmd: "npm run sim -- --torpedoes 24 --pdcs 6",
            why: "Run the full engagement simulation",
            family: "npm run sim",
            out: "24 torpedoes · 24 intercepted · closest 1.9 km",
        },
        done: "Fire-control lag is 3.7 ms p99 now (was 63). Every torpedo in the sim dies before 2 km.",
    },
];

/// A plain shell's commands and what they print.
pub struct ShellCmd {
    pub cmd: &'static str,
    pub out: &'static [&'static str],
}

pub const SHELL_CMDS: &[ShellCmd] = &[
    ShellCmd {
        cmd: "git log --oneline -4",
        out: &[
            "3f1c2a9 generator: brownian motion from a hot cup of tea",
            "9b7e004 tests: whale regression",
            "c41d8e2 release 2.0.42",
            "77aa1f0 docs: on the dangers of improbability",
        ],
    },
    ShellCmd {
        cmd: "cargo build --release",
        out: &[
            "   Compiling improbability v2.0.42",
            "    Finished `release` profile [optimized] target(s) in 38.12s",
        ],
    },
    ShellCmd {
        cmd: "./target/release/improbability --odds",
        out: &["2^276,709 to 1 against … and falling"],
    },
    ShellCmd {
        cmd: "git status -s",
        out: &[" M src/generator.rs", "?? tests/whale_regression.rs"],
    },
];

// ------------------------------------------------------------------ sessions

/// A session the world starts with.
/// The agent's own id for the conversation in a pane (a uuid, as Claude
/// Code's are): the session's `native_id`, and its log's in the usage world.
pub fn conversation(host: &str, pane: &str) -> String {
    let h = hash(&[host, pane, "conversation"]);
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        h >> 32,
        (h >> 16) & 0xffff,
        h & 0xfff,
        (h >> 20) & 0xfff,
        mix(h) & 0xffff_ffff_ffff
    )
}

pub struct Base {
    pub host: &'static str,
    pub agent: &'static str,
    /// its folder, under the machine's home
    pub dir: &'static str,
    /// into `TASKS` (ignored for a shell)
    pub task: usize,
    pub ws: u32,
    pub n: u32,
    /// stops at its plan's usage limit 4 minutes of every 20
    pub limited: bool,
}

pub const BASE: &[Base] = &[
    Base {
        host: "heart-of-gold",
        agent: "claude",
        dir: "improbability-drive",
        task: 0,
        ws: 1,
        n: 1,
        limited: false,
    },
    Base {
        host: "heart-of-gold",
        agent: "shell",
        dir: "improbability-drive",
        task: 0,
        ws: 1,
        n: 2,
        limited: false,
    },
    Base {
        host: "magrathea",
        agent: "claude",
        dir: "planet-factory",
        task: 1,
        ws: 1,
        n: 1,
        limited: false,
    },
    Base {
        host: "magrathea",
        agent: "codex",
        dir: "deep-thought",
        task: 2,
        ws: 1,
        n: 2,
        limited: true,
    },
    Base {
        host: "magrathea",
        agent: "gemini",
        dir: "pulsar-timing",
        task: 3,
        ws: 2,
        n: 3,
        limited: false,
    },
    Base {
        host: "europa",
        agent: "opencode",
        dir: "babel-fish",
        task: 4,
        ws: 1,
        n: 1,
        limited: false,
    },
    Base {
        host: "europa",
        agent: "claude",
        dir: "subspace-relay",
        task: 5,
        ws: 1,
        n: 2,
        limited: false,
    },
    Base {
        host: "tycho-base",
        agent: "pi",
        dir: "epstein-drive",
        task: 6,
        ws: 1,
        n: 1,
        limited: false,
    },
    Base {
        host: "tycho-base",
        agent: "codex",
        dir: "ring-gate",
        task: 7,
        ws: 1,
        n: 2,
        limited: false,
    },
    Base {
        host: "tycho-base",
        agent: "opencode",
        dir: "rocinante-pdc",
        task: 9,
        ws: 2,
        n: 3,
        limited: false,
    },
    Base {
        host: "vogsphere",
        agent: "claude",
        dir: "poetry-engine",
        task: 8,
        ws: 1,
        n: 1,
        limited: false,
    },
];

// ------------------------------------------------------------------ actions

/// Something someone did to the world (the action log, `actions.jsonl`).
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub t: u64,
    pub host: String,
    /// the pane (or tab) it's about
    pub pane: String,
    pub act: Act,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// the question was answered
    Approve,
    /// a prompt was sent
    Send(String),
    New {
        tab: String,
        agent: String,
        name: String,
        label: String,
        dir: String,
        prompt: Option<String>,
    },
    /// the pane went (or every pane of the tab `pane` names)
    Close,
    Rename {
        name: Option<String>,
        label: Option<String>,
    },
    /// herdr was restarted on the machine (the hub's self-heal, or `S`)
    Heal,
}

// ------------------------------------------------------------------ the script

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// working, before the question
    Work,
    /// blocked on the question
    Ask,
    /// working again, after it was answered
    After,
    Done,
    Idle,
    /// working on a prompt someone sent
    Prompted,
    /// finished the prompt someone sent
    Replied,
    /// stopped at the plan's usage limit
    Limit,
    /// a plain shell
    Shell,
}

/// A session's timing: how long each part of its cycle lasts (seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cycle {
    pub work: u64,
    pub ask: u64,
    pub after: u64,
    pub done: u64,
    pub len: u64,
}

/// How long a sent prompt keeps it working, then shows as done.
pub const PROMPT_WORK: u64 = 75;
pub const PROMPT_DONE: u64 = 40;
/// An unanswered question answers itself after this long.
pub const ASK: u64 = 120;
/// The limit: 4 minutes of every 20.
pub const LIMIT_EVERY: u64 = 1200;
pub const LIMIT_FOR: u64 = 240;
/// When the world's own sessions began (their cycles count from here).
pub const EPOCH: i64 = 1_760_000_000;

/// Where a session is in its script at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct At {
    pub phase: Phase,
    /// seconds into the phase, and how long it lasts
    pub into: u64,
    pub len: u64,
    /// herdr's `state_change_seq` and `completion_seq`
    pub seq: u64,
    pub done_seq: u64,
    /// cycles so far
    pub round: u64,
}

impl At {
    pub fn status(&self) -> &'static str {
        match self.phase {
            Phase::Work | Phase::After | Phase::Prompted => "working",
            Phase::Ask => "blocked",
            Phase::Done | Phase::Replied => "done",
            Phase::Idle | Phase::Limit | Phase::Shell => "idle",
        }
    }
}

/// A session of the world, at a moment.
#[derive(Clone, Debug)]
pub struct Sess {
    pub host: String,
    pub pane: String,
    pub tab: String,
    pub ws: String,
    pub agent: String,
    pub name: String,
    pub label: String,
    pub cwd: String,
    pub task: &'static Task,
    /// what it was last asked (a sent prompt, else its task's)
    pub prompt: String,
    pub hue: String,
    pub mark: String,
    pub at: At,
    /// seconds the limit has left (Limit only)
    pub limit_left: u64,
}

impl Sess {
    pub fn status(&self) -> &'static str {
        self.at.status()
    }
    pub fn title(&self) -> String {
        match self.at.phase {
            Phase::Shell => String::new(),
            Phase::Prompted | Phase::Replied => crate::ui::trunc(&self.prompt, 60),
            _ => self.task.title.to_string(),
        }
    }
}

/// 64-bit FNV-1a, then mixed (splitmix64).
pub fn hash(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain([0xff]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    mix(h)
}

pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn cycle_of(seed: u64, id: &str) -> Cycle {
    let h = hash(&[&seed.to_string(), id, "cycle"]);
    let work = 150 + h % 120;
    let after = 90 + (h >> 12) % 90;
    let done = 45 + (h >> 24) % 30;
    Cycle {
        work,
        ask: ASK,
        after,
        done,
        len: 660 + (h >> 36) % 300,
    }
}

/// The script at `t` for a session whose cycle 0 began at `origin`,
/// answered at `approvals` and sent prompts at `prompts` (both sorted).
pub fn script(c: &Cycle, origin: i64, t: u64, approvals: &[u64], prompts: &[u64]) -> At {
    let r = (t as i64 - origin).max(0) as u64;
    let (k, p) = (r / c.len, r % c.len);
    let start = t - p;
    let (ask_at, after_at, done_at, idle_at) = (
        c.work,
        c.work + c.ask,
        c.work + c.ask + c.after,
        c.work + c.ask + c.after + c.done,
    );
    let at = |phase, into, len, i: u64| At {
        phase,
        into,
        len,
        seq: 5 * k + i,
        done_seq: k + u64::from(p >= done_at),
        round: k,
    };
    let base = if p < ask_at {
        at(Phase::Work, p, c.work, 1)
    } else if p < after_at {
        // answered (by someone, or by the hub) since the question came up
        match approvals
            .iter()
            .rev()
            .find(|&&a| a >= start + ask_at && a <= t)
        {
            Some(&a) => at(Phase::After, t - a, start + done_at - a, 3),
            None => at(Phase::Ask, p - ask_at, c.ask, 2),
        }
    } else if p < done_at {
        at(Phase::After, p - after_at, c.after, 3)
    } else if p < idle_at {
        at(Phase::Done, p - done_at, c.done, 4)
    } else {
        at(Phase::Idle, p - idle_at, c.len - idle_at, 5)
    };
    // a prompt sent while it sat idle (or done) starts a turn of its own
    if let Some(&s) = prompts.iter().rev().find(|&&s| s <= t) {
        let el = t - s;
        if el < PROMPT_WORK + PROMPT_DONE {
            let then = script(c, origin, s, approvals, &[]);
            if matches!(then.phase, Phase::Idle | Phase::Done) {
                let n = prompts.iter().filter(|&&x| x <= t).count() as u64;
                return if el < PROMPT_WORK {
                    At {
                        phase: Phase::Prompted,
                        into: el,
                        len: PROMPT_WORK,
                        seq: base.seq + 1000 * n,
                        ..base
                    }
                } else {
                    At {
                        phase: Phase::Replied,
                        into: el - PROMPT_WORK,
                        len: PROMPT_DONE,
                        seq: base.seq + 1000 * n + 1,
                        done_seq: base.done_seq + n,
                        ..base
                    }
                };
            }
        }
    }
    base
}

/// Story names for the world's own sessions (by seed; worked out once).
fn base_names(seed: u64) -> Vec<String> {
    static C: OnceLock<Mutex<HashMap<u64, Vec<String>>>> = OnceLock::new();
    let c = C.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = c.lock().unwrap_or_else(|e| e.into_inner()).get(&seed) {
        return v.clone();
    }
    let mut live: Vec<String> = Vec::new();
    for b in BASE {
        let land = crate::names::land_words(machine(b.host).map(|m| m.color), b.host);
        let s = hash(&[&seed.to_string(), b.host, b.dir, b.agent, "name"]);
        let n = crate::names::story(b.agent, land, &live, &[], s);
        live.push(n);
    }
    c.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(seed, live.clone());
    live
}

fn folder(dir: &str) -> &str {
    dir.rsplit('/').find(|x| !x.is_empty()).unwrap_or(dir)
}

/// The world at `now`.
pub struct World {
    pub sessions: Vec<Sess>,
    /// machines whose link is down (no ssh)
    pub down: Vec<&'static str>,
    /// machines whose herdr has stopped (the story's drop): ssh answers, herdr doesn't
    pub stopped: Vec<&'static str>,
}

impl World {
    /// Pure: the same seed, time and actions give the same world (no story).
    #[cfg(test)]
    pub fn at(seed: u64, now: u64, actions: &[Action]) -> World {
        World::script(seed, now, actions, None)
    }

    /// [`World::at`], playing the story's beats (`story.rs`) when there is
    /// one: still pure — the same seed, time, actions and story give the
    /// same world.
    pub fn script(seed: u64, now: u64, actions: &[Action], story: Option<Story>) -> World {
        let past: Vec<&Action> = actions.iter().filter(|a| a.t <= now).collect();
        let closed = |host: &str, pane: &str, tab: &str| {
            past.iter()
                .any(|a| a.act == Act::Close && a.host == host && (a.pane == pane || a.pane == tab))
        };
        let of = |host: &str, pane: &str, want: fn(&Act) -> bool| -> Vec<u64> {
            past.iter()
                .filter(|a| a.host == host && a.pane == pane && want(&a.act))
                .map(|a| a.t)
                .collect()
        };
        let names = base_names(seed);
        let mut out: Vec<Sess> = Vec::new();
        let mut hues: HashMap<&str, Vec<String>> = HashMap::new();
        for (i, b) in BASE.iter().enumerate() {
            let Some(m) = machine(b.host) else { continue };
            let pane = format!("w{}:p{}", b.ws, b.n);
            let tab = format!("w{}:t{}", b.ws, b.n);
            if closed(b.host, &pane, &tab) {
                continue;
            }
            let id = format!("{}|{pane}", b.host);
            let c = cycle_of(seed, &id);
            let offset = (hash(&[&seed.to_string(), &id, "offset"]) % c.len) as i64;
            let approvals = of(b.host, &pane, |a| *a == Act::Approve);
            let prompts = of(b.host, &pane, |a| matches!(a, Act::Send(_)));
            let shell = b.agent == crate::agents::SHELL;
            let me = (b.host, pane.as_str());
            // the story's players run on its beats instead of their own cycle
            let asker = story.filter(|s| s.has(story::ASK) && me == story::ASKER);
            let finisher = story.filter(|s| s.has(story::FINISH) && me == story::FINISHER);
            let mut task = &TASKS[b.task.min(TASKS.len() - 1)];
            let mut at = if shell {
                At {
                    phase: Phase::Shell,
                    into: now,
                    len: 0,
                    seq: 1,
                    done_seq: 0,
                    round: 0,
                }
            } else if let Some(s) = asker {
                task = story::ask_task(s.place(now).0);
                story::ask(&s, now, &approvals)
            } else if let Some(s) = finisher {
                task = &story::BIG;
                story::finish(&s, now)
            } else {
                script(&c, EPOCH - offset, now, &approvals, &prompts)
            };
            // at the plan's limit: the story's beat (its player only), else
            // 4 minutes in 20 — (seconds into it, how long it lasts)
            let hit = match story {
                Some(s) if s.has(story::LIMIT) => (me == story::LIMITED)
                    .then(|| story::limit_at(&s, now))
                    .flatten(),
                _ => (b.limited && now % LIMIT_EVERY < LIMIT_FOR)
                    .then_some((now % LIMIT_EVERY, LIMIT_FOR)),
            };
            let mut limit_left = 0;
            if let Some((into, len)) = hit {
                limit_left = len - into;
                at = At {
                    phase: Phase::Limit,
                    into,
                    len,
                    seq: at.seq + 500,
                    ..at
                };
            }
            let name = names[i].clone();
            let used = hues.entry(b.host).or_default();
            let (hue, mark) = crate::theme::pick_hue(used, hash(&[&id, "hue"]));
            used.push(hue.to_string());
            let mut s = Sess {
                host: b.host.to_string(),
                pane,
                tab,
                ws: format!("w{}", b.ws),
                agent: b.agent.to_string(),
                label: format!("{}_{name}:{}", b.agent, folder(b.dir)),
                name,
                cwd: format!("{}/{}", m.home(), b.dir),
                task,
                prompt: String::new(),
                hue: hue.to_string(),
                mark: mark.to_string(),
                at,
                limit_left,
            };
            s.prompt = last_prompt(&past, &s).unwrap_or_else(|| s.task.prompt.to_string());
            out.push(s);
        }
        // sessions started since
        for a in &past {
            let Act::New {
                tab,
                agent,
                name,
                label,
                dir,
                prompt,
            } = &a.act
            else {
                continue;
            };
            if closed(&a.host, &a.pane, tab)
                || out.iter().any(|s| s.host == a.host && s.pane == a.pane)
            {
                continue;
            }
            let id = format!("{}|{}", a.host, a.pane);
            let c = cycle_of(seed, &id);
            let approvals = of(&a.host, &a.pane, |x| *x == Act::Approve);
            let prompts = of(&a.host, &a.pane, |x| matches!(x, Act::Send(_)));
            let at = if agent == crate::agents::SHELL {
                At {
                    phase: Phase::Shell,
                    into: now,
                    len: 0,
                    seq: 1,
                    done_seq: 0,
                    round: 0,
                }
            } else {
                script(&c, a.t as i64, now, &approvals, &prompts)
            };
            let (hue, mark) = crate::theme::pick_hue(&[], hash(&[&id, "hue"]));
            let task = &TASKS[(hash(&[&id, "task"]) % TASKS.len() as u64) as usize];
            let mut s = Sess {
                host: a.host.clone(),
                pane: a.pane.clone(),
                tab: tab.clone(),
                ws: a.pane.split(':').next().unwrap_or("w1").to_string(),
                agent: agent.clone(),
                name: name.clone(),
                label: label.clone(),
                cwd: dir.clone(),
                task,
                prompt: prompt.clone().unwrap_or_else(|| task.prompt.to_string()),
                hue: hue.to_string(),
                mark: mark.to_string(),
                at,
                limit_left: 0,
            };
            if let Some(p) = last_prompt(&past, &s) {
                s.prompt = p;
            }
            out.push(s);
        }
        // renames, in order
        for a in &past {
            if let Act::Rename { name, label } = &a.act {
                for s in out
                    .iter_mut()
                    .filter(|s| s.host == a.host && (s.pane == a.pane || s.tab == a.pane))
                {
                    if let Some(n) = name {
                        s.name = n.clone();
                    }
                    if let Some(l) = label {
                        s.label = l.clone();
                    }
                }
            }
        }
        // a machine's link drops a minute in ten (not while the story's drop
        // plays: that one is herdr stopping, and the hub bringing it back)
        let drop = story.is_some_and(|s| s.has(story::DROP));
        let down = MACHINES
            .iter()
            .filter(|m| !drop && !m.up(now))
            .map(|m| m.alias)
            .collect();
        let stopped = story
            .map(|s| {
                MACHINES
                    .iter()
                    .filter(|m| story::stopped(&s, m.alias, now, &past))
                    .map(|m| m.alias)
                    .collect()
            })
            .unwrap_or_default();
        World {
            sessions: out,
            down,
            stopped,
        }
    }

    /// Does `host` answer ssh now?
    pub fn link_up(&self, host: &str) -> bool {
        !self.down.contains(&host)
    }

    /// Is herdr running on `host` now (and its link up)?
    pub fn herdr_up(&self, host: &str) -> bool {
        self.link_up(host) && !self.stopped.contains(&host)
    }

    /// The sessions on `host`.
    pub fn on<'a>(&'a self, host: &str) -> impl Iterator<Item = &'a Sess> + 'a {
        let host = host.to_string();
        self.sessions.iter().filter(move |s| s.host == host)
    }

    /// A session on `host` by pane, tab or name.
    pub fn find(&self, host: &str, target: &str) -> Option<&Sess> {
        self.on(host)
            .find(|s| s.pane == target)
            .or_else(|| self.on(host).find(|s| s.tab == target))
            .or_else(|| self.on(host).find(|s| s.name == target))
    }
}

/// The newest prompt sent to `s` (by the time the world is at).
fn last_prompt(past: &[&Action], s: &Sess) -> Option<String> {
    past.iter().rev().find_map(|a| match &a.act {
        Act::Send(p) if a.host == s.host && a.pane == s.pane => Some(p.clone()),
        _ => None,
    })
}
