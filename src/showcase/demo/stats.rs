//! Machine stats in the demo world: waves as a function of time — CPU,
//! cores, memory, load, network, disks, GPUs — and their histories, each
//! point the same function `k` sample periods ago (so every process draws
//! the same charts).

use super::world::{self, Machine};
use crate::telemetry::{AgentProc, DiskIo, Gpu, GpuProc, Net, Proc, Stats};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const HIST: usize = 120;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// A smooth wave in 0..1 around `base`: two sines and a little noise.
fn wave(m: &Machine, what: &str, t: f64, base: f64, amp: f64) -> f64 {
    let h = world::hash(&[m.alias, what]);
    let (p1, p2) = ((h % 1000) as f64 / 159.0, ((h >> 10) % 1000) as f64 / 159.0);
    let (f1, f2) = (
        90.0 + (h >> 20) as f64 % 80.0,
        21.0 + (h >> 30) as f64 % 13.0,
    );
    let tick = (t / 2.0).floor() as u64;
    let n = (world::hash(&[m.alias, what, &tick.to_string()]) % 1000) as f64 / 1000.0 - 0.5;
    (base
        + amp * (std::f64::consts::TAU * t / f1 + p1).sin()
        + amp * 0.45 * (std::f64::consts::TAU * t / f2 + p2).sin()
        + amp * 0.3 * n)
        .clamp(0.01, 0.99)
}

fn hist(f: impl Fn(f64) -> f64, t: f64, period: f64) -> Vec<f64> {
    (0..HIST)
        .map(|k| f(t - (HIST - 1 - k) as f64 * period))
        .collect()
}

/// How busy the machine's agents are: more working sessions, more CPU.
fn busy(m: &Machine, w: &world::World) -> f64 {
    let n = w.on(m.alias).filter(|s| s.status() == "working").count();
    (n as f64 * 0.08).min(0.3)
}

fn cpu_at(m: &Machine, t: f64, extra: f64) -> f64 {
    wave(m, "cpu", t, m.load + extra, 0.16)
}

fn gpu_util(m: &Machine, i: usize, t: f64) -> f64 {
    wave(m, &format!("gpu{i}"), t, 0.62 - 0.2 * i as f64, 0.3)
}

/// The machine's stats at `t` (unix seconds, fractions allowed).
pub fn at(m: &Machine, w: &world::World, t: f64, period: f64) -> Stats {
    let extra = busy(m, w);
    let cpu = cpu_at(m, t, extra);
    let mem_total = m.mem_gb * GIB;
    let memf = |t: f64| wave(m, "mem", t, 0.42 + extra * 0.4, 0.06);
    let mem_used = memf(t) * mem_total;
    let cores: Vec<f64> = (0..m.ncpu)
        .map(|i| wave(m, &format!("core{i}"), t, cpu, 0.22))
        .collect();
    let rx = |t: f64| wave(m, "rx", t, 0.25, 0.2) * 12e6;
    let tx = |t: f64| wave(m, "tx", t, 0.18, 0.15) * 6e6;
    let load = |t: f64| cpu_at(m, t, extra) * m.ncpu as f64 * 0.9;
    let temp = |t: f64| 38.0 + 42.0 * cpu_at(m, t, extra);
    let gpus: Vec<Gpu> = m
        .gpus
        .iter()
        .enumerate()
        .map(|(i, (name, vram, watts))| {
            let u = gpu_util(m, i, t);
            let vmem = |t: f64| wave(m, &format!("vram{i}"), t, 0.71, 0.05);
            Gpu {
                index: i.to_string(),
                name: name.to_string(),
                util: u,
                mem_used: vmem(t) * vram * GIB,
                mem_total: vram * GIB,
                temp: Some(34.0 + 44.0 * u),
                power: Some(watts * (0.18 + 0.8 * u)),
                power_limit: Some(*watts),
                fan: Some(30.0 + 55.0 * u),
                bus: format!("00000000:{:02X}:00.0", 0x41 + i * 0x40),
                hist: hist(|t| gpu_util(m, i, t), t, period),
                vendor: "nvidia".into(),
                mem_util: Some(u * 0.7),
                clock_sm: Some(1200.0 + 1300.0 * u),
                clock_mem: Some(10_001.0),
                clock_sm_max: Some(2505.0),
                pstate: if u > 0.1 { "P2" } else { "P8" }.into(),
                pcie: "gen4 x16".into(),
                enc: Some(0.0),
                dec: Some(0.0),
                temp_mem: None,
                throttle: vec![],
                ecc: Some(0),
                mem_hist: hist(vmem, t, period),
                power_hist: hist(|t| 0.18 + 0.8 * gpu_util(m, i, t), t, period),
                temp_hist: hist(|t| 34.0 + 44.0 * gpu_util(m, i, t), t, period),
                ..Default::default()
            }
        })
        .collect();
    // what runs there: its agents, then the usual suspects
    let mut procs: Vec<Proc> = Vec::new();
    let mut agent_list = Vec::new();
    for (k, s) in w.on(m.alias).enumerate() {
        if s.agent == crate::agents::SHELL {
            continue;
        }
        let pid = (2000 + world::hash(&[&s.pane, "pid"]) % 60_000).to_string();
        let working = s.status() == "working";
        procs.push(Proc {
            pid: pid.clone(),
            user: m.user.into(),
            cpu: if working {
                wave(m, &s.pane, t, 0.45, 0.3)
            } else {
                0.02
            },
            rss: (380.0 + 40.0 * k as f64) * 1024.0 * 1024.0,
            name: match s.agent.as_str() {
                "claude" => "claude",
                "codex" => "codex",
                "gemini" | "opencode" | "pi" => "node",
                a => a,
            }
            .to_string(),
            state: if working { "R" } else { "S" }.into(),
            threads: 18,
            cmd: format!("{} --resume", s.agent),
            time: 1200.0 + k as f64 * 300.0,
            ppid: "1".into(),
            age: Some(3600 * (2 + k as u64)),
            avg_cpu: Some(0.12),
        });
        agent_list.push(AgentProc {
            agent: s.agent.clone(),
            pid,
            user: m.user.into(),
            age: Some(3600 * (2 + k as u64)),
            cmd: s.agent.clone(),
            in_herdr: true,
            ..Default::default()
        });
    }
    let extra_procs: &[(&str, f64, f64)] = if m.gpus.is_empty() {
        &[
            ("rust-analyzer", 0.12, 1.4),
            ("cargo", 0.6, 0.9),
            ("sshd", 0.0, 0.01),
        ]
    } else {
        &[
            ("python3", 2.4, 18.0),
            ("vllm", 0.9, 9.5),
            ("ollama", 0.3, 4.1),
            ("sshd", 0.0, 0.01),
        ]
    };
    for (i, (name, c, gb)) in extra_procs.iter().enumerate() {
        procs.push(Proc {
            pid: (900 + i * 211).to_string(),
            user: m.user.into(),
            cpu: wave(m, name, t, (*c / 4.0).min(0.9), 0.2) * 4.0,
            rss: gb * GIB,
            name: name.to_string(),
            state: "S".into(),
            threads: 12,
            cmd: name.to_string(),
            time: 600.0,
            ppid: "1".into(),
            age: Some(86_400),
            avg_cpu: Some(*c),
        });
    }
    procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    let gpu_procs: Vec<GpuProc> = m
        .gpus
        .iter()
        .enumerate()
        .map(|(i, (_, vram, _))| GpuProc {
            gpu: i.to_string(),
            pid: (900 + i * 211).to_string(),
            mem: vram * 0.66 * GIB,
            user: m.user.into(),
            name: if i == 0 { "vllm" } else { "python3" }.into(),
            kind: "C".into(),
            sm: Some(gpu_util(m, i, t)),
            ..Default::default()
        })
        .collect();
    let agent_procs = {
        let mut v: Vec<(String, usize, f64)> = Vec::new();
        for p in &agent_list {
            match v.iter_mut().find(|x| x.0 == p.agent) {
                Some(x) => {
                    x.1 += 1;
                    x.2 += 400.0 * 1024.0 * 1024.0;
                }
                None => v.push((p.agent.clone(), 1, 400.0 * 1024.0 * 1024.0)),
            }
        }
        v
    };
    let disk_total = m.disk_gb * 1e9;
    let iface = if m.mac() { "en0" } else { "eth0" };
    let dev = if m.mac() { "disk0" } else { "nvme0n1" };
    let rd = |t: f64| wave(m, "rd", t, 0.12, 0.1) * 80e6;
    let wr = |t: f64| wave(m, "wr", t, 0.1, 0.1) * 60e6;
    let up = (t as u64).saturating_sub(1_760_000_000) % (86_400 * 23) + 86_400 * 3;
    Stats {
        hostname: m.alias.into(),
        kernel: m.release.into(),
        os: m.distro.into(),
        cpu_model: m.cpu.into(),
        ncpu: m.ncpu,
        sockets: m.sockets,
        cpu,
        cores,
        cpu_hist: hist(|t| cpu_at(m, t, extra), t, period),
        mem_total,
        mem_used,
        swap_total: 8.0 * GIB,
        swap_used: 0.3 * GIB,
        mem_hist: hist(memf, t, period),
        load: [load(t), load(t - 300.0), load(t - 900.0)],
        runnable: format!(
            "{}/{}",
            (cpu * m.ncpu as f64) as usize + 1,
            900 + m.ncpu * 12
        ),
        uptime: up,
        net_rx: rx(t),
        net_tx: tx(t),
        net_hist: hist(|t| rx(t) + tx(t), t, period),
        gpus,
        gpu_procs,
        top: procs.iter().take(8).cloned().collect(),
        disks: vec![("/".into(), disk_total, disk_total * 0.43)],
        claude_procs: agent_list.iter().filter(|a| a.agent == "claude").count(),
        claude_rss: 420.0 * 1024.0 * 1024.0,
        convos_today: Some(3 + (world::hash(&[m.alias, "convos"]) % 9) as usize),
        samples: 2 + t as u64 % 1000,
        updated: Some(Instant::now()),
        err: None,
        has_gpu_tool: !m.gpus.is_empty(),
        platform: if m.mac() { "macos" } else { "linux" }.into(),
        cpu_user: cpu * 0.78,
        cpu_system: cpu * 0.19,
        cpu_iowait: cpu * 0.03,
        cpu_steal: 0.0,
        load_hist: hist(load, t, period),
        cpu_temp: Some(temp(t)),
        temps: vec![
            ("Package".into(), temp(t)),
            ("NVMe".into(), 41.0 + 6.0 * cpu),
        ],
        mem_buffers: mem_total * 0.02,
        mem_cached: mem_total * 0.21,
        mem_shared: mem_total * 0.01,
        mem_available: mem_total - mem_used,
        swap_hist: hist(|_| 0.3 / 8.0, t, period),
        procs_total: 420 + m.ncpu * 4,
        procs_running: 1 + (cpu * 6.0) as usize,
        threads_total: 2400 + m.ncpu * 30,
        users_logged: Some(1),
        nets: vec![Net {
            name: iface.into(),
            rx: rx(t),
            tx: tx(t),
            hist: hist(|t| rx(t) + tx(t), t, period),
            rx_hist: hist(rx, t, period),
            tx_hist: hist(tx, t, period),
        }],
        disk_io: vec![DiskIo {
            name: dev.into(),
            read: rd(t),
            write: wr(t),
            busy: (!m.mac()).then(|| wave(m, "busy", t, 0.08, 0.06)),
            hist: hist(|t| rd(t) + wr(t), t, period),
            read_hist: hist(rd, t, period),
            write_hist: hist(wr, t, period),
        }],
        procs,
        gpu_note: m.gpus.is_empty().then(|| {
            if m.mac() {
                "Apple GPU (no per-process stats)".to_string()
            } else {
                "no nvidia-smi or rocm-smi".to_string()
            }
        }),
        agent_procs,
        agent_list,
        sessions_today: vec![
            (
                "claude".into(),
                4 + (world::hash(&[m.alias, "today"]) % 7) as usize,
            ),
            ("codex".into(), 2),
        ],
        period,
        cpu_user_hist: hist(|t| cpu_at(m, t, extra) * 0.78, t, period),
        cpu_system_hist: hist(|t| cpu_at(m, t, extra) * 0.19, t, period),
        cpu_iowait_hist: hist(|t| cpu_at(m, t, extra) * 0.03, t, period),
        cpu_temp_hist: hist(temp, t, period),
        mem_cache_hist: hist(|_| 0.23, t, period),
        net_rx_hist: hist(rx, t, period),
        net_tx_hist: hist(tx, t, period),
        cpu_mhz: (!m.mac()).then_some(2900.0 + 1400.0 * cpu),
        cpu_mhz_peak: (!m.mac()).then_some(4200.0 + 800.0 * cpu),
        cpu_mhz_max: (!m.mac()).then_some(5000.0),
    }
}

fn now_f() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Feed `st` every `interval` seconds until `stop` (a stopped machine says
/// why, like a dropped ssh would).
pub fn feed(alias: &str, interval: u64, st: &Arc<Mutex<Stats>>, stop: &Arc<Mutex<bool>>) {
    let Some(m) = world::machine(alias) else {
        st.lock().unwrap_or_else(|e| e.into_inner()).err = Some(super::NOT_HERE.into());
        return;
    };
    let every = Duration::from_secs(interval.max(1));
    loop {
        if *stop.lock().unwrap_or_else(|e| e.into_inner()) {
            return;
        }
        let t = now_f();
        {
            let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
            if super::link_up(m) {
                *g = at(m, &super::world(), t, every.as_secs_f64());
            } else {
                g.err = Some(m.down_text());
                g.updated = None;
            }
        }
        let until = Instant::now() + every;
        while Instant::now() < until {
            if *stop.lock().unwrap_or_else(|e| e.into_inner()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
