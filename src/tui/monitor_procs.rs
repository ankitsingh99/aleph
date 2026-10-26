//! The Monitor's process table (htop): a tree by default (htop's F5: each
//! process under its parent, `├─` / `└─` guides, folds that hide whole
//! subtrees), or a flat list. Both sort, fuzzy-filter and scroll; the
//! selection follows its process as rows move, and a details line describes
//! it. Also the small top-N tables the other views show.

use super::*;
use std::collections::{HashMap, HashSet};

/// The process table's state: what the keys set, and what the last draw
/// found (row count, page height, the selection's parent and folds) for the
/// keys to act on.
#[derive(Clone, Debug)]
pub struct ProcUi {
    pub sort: Sort,
    /// the other way round from the sort's natural order
    pub rev: bool,
    /// tree (default) or flat list
    pub tree: bool,
    pub filter: String,
    /// typing the filter (draws a cursor)
    pub typing: bool,
    /// pids whose subtrees are folded away
    pub collapsed: HashSet<String>,
    /// the selected row; `sel_pid` (when set) wins, so the selection stays
    /// on its process when the order changes
    pub sel: usize,
    pub sel_pid: Option<String>,
    pub scroll: usize,
    /// fold every process with children on the next draw
    pub collapse_all: bool,
    // ---- found by the last draw
    pub rows: usize,
    /// rows on screen (page up / down)
    pub page: usize,
    /// the selected process's parent row (tree), for `←`
    pub parent_row: Option<usize>,
    /// the selected process has children / they're shown
    pub has_kids: bool,
    pub open: bool,
}

impl Default for ProcUi {
    fn default() -> Self {
        ProcUi {
            sort: Sort::Cpu,
            rev: false,
            tree: true,
            filter: String::new(),
            typing: false,
            collapsed: HashSet::new(),
            sel: 0,
            sel_pid: None,
            scroll: 0,
            collapse_all: false,
            rows: 0,
            page: 10,
            parent_row: None,
            has_kids: false,
            open: false,
        }
    }
}

impl ProcUi {
    /// Move the selection by `d` rows (it then follows that process).
    pub fn step(&mut self, d: isize) {
        let n = self.rows.max(1);
        let s = (self.sel as isize + d).clamp(0, n as isize - 1);
        self.sel = s as usize;
        self.sel_pid = None;
    }
    /// Select row `row` (the next draw clamps it to the rows there are).
    pub fn jump(&mut self, row: usize) {
        self.sel = row;
        self.sel_pid = None;
    }
    /// Fold or unfold the selected process.
    pub fn toggle(&mut self) {
        if let Some(p) = self.sel_pid.clone().filter(|_| self.has_kids) {
            if !self.collapsed.remove(&p) {
                self.collapsed.insert(p);
            }
        }
    }
    /// `←`: fold the selection, or go to its parent.
    pub fn left(&mut self) {
        match self.sel_pid.clone() {
            Some(p) if self.tree && self.has_kids && self.open => {
                self.collapsed.insert(p);
            }
            _ => {
                if let Some(r) = self.parent_row.filter(|_| self.tree) {
                    self.jump(r);
                }
            }
        }
    }
    /// `→`: unfold the selection.
    pub fn right(&mut self) {
        if let Some(p) = self.sel_pid.clone().filter(|_| self.tree && self.has_kids) {
            self.collapsed.remove(&p);
        }
    }
}

// ------------------------------------------------------------------ rows

/// A table row: the process and its place in the tree.
struct PRow<'a> {
    p: &'a Proc,
    /// tree guides before the command (`│  ├─ `)
    guide: String,
    /// children shown under it / descendants folded away
    kids: usize,
    hidden: usize,
    /// only here as the ancestor of a filter match
    dim: bool,
    parent: Option<usize>,
}

/// Lifetime CPU when known, else (for the order) what it uses now.
fn avg_of(p: &Proc) -> f64 {
    p.avg_cpu.map(fin).unwrap_or(-1.0)
}

/// The table order. CPU, memory, average CPU and CPU time sort descending,
/// uptime youngest first, PID and name ascending; ties by PID so rows don't jitter.
pub(super) fn proc_cmp(a: &Proc, b: &Proc, sort: Sort) -> Ordering {
    let pid = || pid_num(a).cmp(&pid_num(b)).then(a.pid.cmp(&b.pid));
    match sort {
        Sort::Cpu => cmp_f(b.cpu, a.cpu).then(cmp_f(b.rss, a.rss)).then_with(pid),
        Sort::Mem => cmp_f(b.rss, a.rss).then(cmp_f(b.cpu, a.cpu)).then_with(pid),
        Sort::Avg => cmp_f(avg_of(b), avg_of(a))
            .then(cmp_f(b.cpu, a.cpu))
            .then_with(pid),
        Sort::Age => a
            .age
            .unwrap_or(u64::MAX)
            .cmp(&b.age.unwrap_or(u64::MAX))
            .then_with(pid),
        Sort::Time => cmp_f(b.time, a.time).then_with(pid),
        Sort::Pid => pid(),
        Sort::Name => {
            let (x, y) = (a.name.trim(), b.name.trim());
            x.bytes()
                .map(|c| c.to_ascii_lowercase())
                .cmp(y.bytes().map(|c| c.to_ascii_lowercase()))
                .then_with(pid)
        }
    }
}

pub(super) fn sort_procs(v: &mut [&Proc], sort: Sort) {
    v.sort_by(|a, b| proc_cmp(a, b, sort));
}

fn order(a: &Proc, b: &Proc, ui: &ProcUi) -> Ordering {
    let o = proc_cmp(a, b, ui.sort);
    if ui.rev {
        o.reverse()
    } else {
        o
    }
}

/// The rows to show: the filter's matches (in the tree, with their
/// ancestors), sorted, folded.
fn build<'a>(list: &'a [Proc], ui: &ProcUi) -> Vec<PRow<'a>> {
    let q = ui.filter.trim();
    let n = list.len();
    let hit: Vec<bool> = if q.is_empty() {
        vec![true; n]
    } else {
        list.iter()
            .map(|p| crate::fuzzy::score(q, &cmd_text(p).0).is_some())
            .collect()
    };
    if !ui.tree {
        let mut idx: Vec<usize> = (0..n).filter(|&i| hit[i]).collect();
        idx.sort_by(|&a, &b| order(&list[a], &list[b], ui));
        return idx
            .into_iter()
            .map(|i| PRow {
                p: &list[i],
                guide: String::new(),
                kids: 0,
                hidden: 0,
                dim: false,
                parent: None,
            })
            .collect();
    }
    let pos: HashMap<&str, usize> = list
        .iter()
        .enumerate()
        .map(|(i, p)| (p.pid.trim(), i))
        .collect();
    let parent: Vec<Option<usize>> = list
        .iter()
        .enumerate()
        .map(|(i, p)| pos.get(p.ppid.trim()).copied().filter(|&j| j != i))
        .collect();
    // matches and every ancestor of one
    let mut keep = hit.clone();
    if !q.is_empty() {
        for i in (0..n).filter(|&i| hit[i]) {
            let mut j = parent[i];
            let mut guard = 0;
            while let Some(k) = j {
                if keep[k] || guard > n {
                    break;
                }
                keep[k] = true;
                j = parent[k];
                guard += 1;
            }
        }
    }
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();
    for i in (0..n).filter(|&i| keep[i]) {
        match parent[i] {
            Some(j) if keep[j] => kids[j].push(i),
            _ => roots.push(i),
        }
    }
    let by = |a: &usize, b: &usize| order(&list[*a], &list[*b], ui);
    roots.sort_by(by);
    for k in kids.iter_mut() {
        k.sort_by(by);
    }
    let folded = |i: usize| ui.collapsed.contains(list[i].pid.trim());
    // descendants of a folded node (iterative; cycles can't recurse forever)
    let count_desc = |i: usize| -> usize {
        let mut c = 0;
        let mut st = vec![i];
        let mut seen = HashSet::new();
        while let Some(x) = st.pop() {
            for &k in &kids[x] {
                if seen.insert(k) {
                    c += 1;
                    st.push(k);
                }
            }
        }
        c
    };
    let mut out: Vec<PRow<'a>> = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    // (node, guide for its own row, guide its children extend, parent row)
    let mut stack: Vec<(usize, String, String, Option<usize>)> = Vec::new();
    // a cycle (ppid loops, pid reuse) has no root: its members become roots
    let mut pending: Vec<usize> = roots.clone();
    pending.reverse();
    let mut extra = (0..n).filter(|&i| keep[i]).collect::<Vec<_>>().into_iter();
    loop {
        if stack.is_empty() {
            let next = pending.pop().or_else(|| extra.find(|&i| !seen[i]));
            match next {
                Some(r) if !seen[r] => {
                    let g = if folded(r) && !kids[r].is_empty() {
                        "▸ ".to_string()
                    } else {
                        String::new()
                    };
                    stack.push((r, g, String::new(), None));
                }
                Some(_) => continue,
                None => break,
            }
        }
        let Some((i, guide, base, par)) = stack.pop() else {
            break;
        };
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let row = out.len();
        let fold = folded(i) && !kids[i].is_empty();
        let shown: Vec<usize> = if fold {
            // the folded subtree is accounted for: never shown as a stray root
            let mut st = kids[i].clone();
            while let Some(x) = st.pop() {
                if !seen[x] {
                    seen[x] = true;
                    st.extend(kids[x].iter().copied());
                }
            }
            vec![]
        } else {
            kids[i].iter().copied().filter(|&k| !seen[k]).collect()
        };
        out.push(PRow {
            p: &list[i],
            guide,
            kids: kids[i].len(),
            hidden: if fold { count_desc(i) } else { 0 },
            dim: !hit[i],
            parent: par,
        });
        let m = shown.len();
        for (j, &k) in shown.iter().enumerate().rev() {
            let last = j + 1 == m;
            let kf = folded(k) && !kids[k].is_empty();
            let g = format!(
                "{base}{}{} ",
                if last { "└" } else { "├" },
                if kf { "▸" } else { "─" }
            );
            let b = format!("{base}{}", if last { "   " } else { "│  " });
            stack.push((k, g, b, Some(row)));
        }
    }
    out
}

// ------------------------------------------------------------------ text

/// `45s`, `12m`, `5h12m`, `3d4h`.
pub(super) fn age(s: u64) -> String {
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        _ if s < 100 * 86400 => format!("{}d{}h", s / 86400, s % 86400 / 3600),
        _ => format!("{}d", (s / 86400).min(99999)),
    }
}

/// The command for a narrow column: the name, then the arguments with long
/// ones shortened (`--flag=a,b,c,…` → `--flag=a,b…`, long paths to their
/// tail), so one Chrome renderer doesn't push everything else off screen.
pub(super) fn short_cmd(p: &Proc) -> (String, usize) {
    let (txt, nl) = cmd_text(p);
    if txt.chars().count() <= 60 {
        return (txt, nl);
    }
    let name: String = txt.chars().take(nl).collect();
    let rest: String = txt.chars().skip(nl).collect();
    let mut out = name;
    for tok in rest.split_whitespace() {
        out.push(' ');
        let n = tok.chars().count();
        if n <= 28 {
            out.push_str(tok);
        } else if let Some((k, v)) = tok.split_once('=') {
            let k = ui::trunc(k, 24);
            let v: String = v.chars().take(10).collect();
            out.push_str(&format!("{k}={v}…"));
        } else if tok.contains('/') {
            out.push_str(&ui::trunc_left(tok, 24));
        } else {
            out.push_str(&ui::trunc(tok, 24));
        }
        if out.chars().count() > 300 {
            break;
        }
    }
    (out, nl)
}

fn state_word(s: &str) -> &'static str {
    match s.chars().next() {
        Some('R') => "running",
        Some('S') => "sleeping",
        Some('D') => "disk wait",
        Some('Z') => "zombie",
        Some('T') | Some('t') => "stopped",
        Some('I') => "idle",
        _ => "",
    }
}

/// CPU % of one core: `31.2`, `340`.
fn cpu_txt(v: f64) -> String {
    let p = (fin(v) * 100.0).clamp(0.0, 99999.0);
    if p >= 100.0 {
        format!("{p:.0}")
    } else {
        format!("{p:.1}")
    }
}

// ------------------------------------------------------------------ columns

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PC {
    Pid,
    User,
    State,
    Cpu,
    Avg,
    Mem,
    Rss,
    Thr,
    Up,
    Time,
    Cmd,
}

const PC_ORDER: [PC; 11] = [
    PC::Pid,
    PC::User,
    PC::State,
    PC::Cpu,
    PC::Avg,
    PC::Mem,
    PC::Rss,
    PC::Thr,
    PC::Up,
    PC::Time,
    PC::Cmd,
];
/// Dropped first when the table narrows.
const PC_DROP: [PC; 8] = [
    PC::Thr,
    PC::Time,
    PC::State,
    PC::User,
    PC::Avg,
    PC::Up,
    PC::Mem,
    PC::Rss,
];

fn pc_w(c: PC) -> usize {
    match c {
        PC::Pid => 7,
        PC::User => 9,
        PC::State => 1,
        PC::Cpu => 5,
        PC::Avg => 5,
        PC::Mem => 5,
        PC::Rss => 6,
        PC::Thr => 4,
        PC::Up => 6,
        PC::Time => 9,
        PC::Cmd => 0,
    }
}

fn pc_title(c: PC) -> &'static str {
    match c {
        PC::Pid => "PID",
        PC::User => "USER",
        PC::State => "S",
        PC::Cpu => "CPU%",
        PC::Avg => "AVG%",
        PC::Mem => "MEM%",
        PC::Rss => "RSS",
        PC::Thr => "THR",
        PC::Up => "UPTIME",
        PC::Time => "TIME+",
        PC::Cmd => "COMMAND",
    }
}

fn pc_sort(c: PC) -> Option<Sort> {
    match c {
        PC::Pid => Some(Sort::Pid),
        PC::Cpu => Some(Sort::Cpu),
        PC::Avg => Some(Sort::Avg),
        PC::Rss => Some(Sort::Mem),
        PC::Up => Some(Sort::Age),
        PC::Time => Some(Sort::Time),
        PC::Cmd => Some(Sort::Name),
        _ => None,
    }
}

/// The columns that fit `w` with a command column ≥ 16 wide.
fn pick_cols(w: usize, want: &[PC], drop: &[PC]) -> Vec<PC> {
    let mut v: Vec<PC> = want.to_vec();
    let need = |v: &[PC]| -> usize { v.iter().map(|c| pc_w(*c) + 1).sum::<usize>() + 16 };
    for d in drop {
        if need(&v) <= w {
            break;
        }
        v.retain(|c| c != d);
    }
    v
}

fn head_line(colv: &[PC], sort: Sort, rev: bool) -> Ln {
    let head_st = ui::faint().add_modifier(Modifier::BOLD);
    let on = ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD);
    let mut h = Ln::new();
    for (i, cc) in colv.iter().enumerate() {
        if i > 0 {
            h.s(" ", Style::new());
        }
        let active = pc_sort(*cc) == Some(sort);
        let mut t = pc_title(*cc).to_string();
        if active {
            t.push_str(sort.arrow(rev));
        }
        let st = if active { on } else { head_st };
        let w = pc_w(*cc);
        match cc {
            PC::User | PC::State | PC::Cmd => {
                h.s(if *cc == PC::Cmd { t } else { ui::pad(&t, w) }, st)
            }
            _ => h.s(format!("{t:>w$}"), st),
        }
    }
    h
}

/// One row's cells up to (not including) the command.
fn cells(l: &mut Ln, colv: &[PC], p: &Proc, mem_total: f64, dim: bool) {
    for (i, cc) in colv.iter().enumerate() {
        if i > 0 {
            l.s(" ", Style::new());
        }
        let w = pc_w(*cc);
        let r = |s: String| format!("{s:>w$}");
        let none = || format!("{:>w$}", "—");
        let (t, st) = match cc {
            PC::Pid => (r(ui::trunc(&clean(&p.pid), 7)), ui::muted()),
            PC::User => {
                let u = clean(&p.user);
                let st = if u == "root" {
                    ui::fg(c::PEACH())
                } else {
                    ui::muted()
                };
                (ui::pad(&u, w), st)
            }
            PC::State => {
                let s = clean(p.state.trim());
                let s1: String = s.chars().take(1).collect();
                (
                    if s1.is_empty() { "·".to_string() } else { s1 },
                    state_style(&s),
                )
            }
            PC::Cpu => (r(cpu_txt(p.cpu)), cpu_style(p.cpu)),
            PC::Avg => match p.avg_cpu.filter(|v| v.is_finite()) {
                Some(a) => (r(cpu_txt(a)), cpu_style(a).add_modifier(Modifier::DIM)),
                None => (none(), ui::faint()),
            },
            PC::Mem => {
                if mem_total > 0.0 {
                    let m = ratio(p.rss, mem_total);
                    (r(format!("{:.1}", m * 100.0)), mem_style(m))
                } else {
                    (none(), ui::faint())
                }
            }
            PC::Rss => (r(by(p.rss)), ui::muted()),
            PC::Thr => {
                if p.threads > 0 {
                    (r(p.threads.to_string()), ui::faint())
                } else {
                    (none(), ui::faint())
                }
            }
            PC::Up => match p.age {
                Some(a) => (r(age(a)), ui::muted()),
                None => (none(), ui::faint()),
            },
            PC::Time => {
                if fin(p.time) > 0.0 {
                    (r(ptime(p.time)), ui::faint())
                } else {
                    (none(), ui::faint())
                }
            }
            PC::Cmd => (String::new(), Style::new()),
        };
        let st = if dim { ui::faint() } else { st };
        if *cc == PC::Rss {
            // the size bright, its unit dim
            l.add(nu(&t, if dim { st } else { ui::fg(c::TEXT()) }));
        } else {
            l.s(t, st);
        }
    }
}

// ------------------------------------------------------------------ the view

/// htop's header, one line: CPU · memory · swap · GPU · VRAM meters, load and tasks.
fn meter_strip(st: &Stats, w: usize) -> Line<'static> {
    let mut items: Vec<(u8, Vec<Span<'static>>)> = Vec::new();
    let bw = if w >= 180 {
        14
    } else if w >= 140 {
        10
    } else {
        6
    };
    let m = |name: &str, v: f64| -> Vec<Span<'static>> {
        let mut s = vec![Span::styled(
            format!("{name} "),
            ui::fg(c::MUTED()).add_modifier(Modifier::BOLD),
        )];
        s.extend(gbar(v, bw));
        s.push(Span::styled(
            format!(" {:>4}", pct(v)),
            ui::fg(ui::heat(fr(v))).add_modifier(Modifier::BOLD),
        ));
        s
    };
    items.push((0, m("CPU", st.cpu)));
    if st.mem_total > 0.0 {
        items.push((0, m("MEM", st.mem_frac())));
    }
    if st.swap_total > 0.0 {
        items.push((3, m("SWAP", ratio(st.swap_used, st.swap_total))));
    }
    if let Some(g) = st.gpu_util() {
        items.push((1, m("GPU", g)));
        let (u, t) = vram_total(st);
        if t > 0.0 {
            items.push((2, m("VRAM", ratio(u, t))));
        }
    }
    if st.procs_total > 0 {
        let mut t = format!("{} tasks", count(st.procs_total));
        if st.threads_total > 0 {
            t.push_str(&format!(", {} thr", count(st.threads_total)));
        }
        if st.procs_running > 0 {
            t.push_str(&format!(", {} running", st.procs_running));
        }
        items.push((5, vec![Span::styled(t, ui::muted())]));
    }
    Line::from(join_prio(items, w))
}

/// The Processes sub-view: a meter strip, then the table (tree or flat) with
/// a details line for the selection. Updates `ui` with what it drew.
pub fn procs(f: &mut Frame, r: Rect, st: &Stats, ui: &mut ProcUi) -> usize {
    let r = area(f, r);
    let list = plist(st);
    if ui.collapse_all {
        ui.collapse_all = false;
        let parents: HashSet<&str> = list.iter().map(|p| p.ppid.trim()).collect();
        for p in list {
            if parents.contains(p.pid.trim()) {
                ui.collapsed.insert(p.pid.trim().to_string());
            }
        }
    }
    let rows = build(list, ui);
    let n = rows.len();
    ui.rows = n;
    // the selection: its process if it's still shown, else the same row
    if let Some(pid) = &ui.sel_pid {
        if let Some(i) = rows.iter().position(|x| x.p.pid == *pid) {
            ui.sel = i;
        }
    }
    ui.sel = ui.sel.min(n.saturating_sub(1));
    ui.sel_pid = rows.get(ui.sel).map(|x| x.p.pid.clone());
    let srow = rows.get(ui.sel);
    ui.parent_row = srow.and_then(|x| x.parent);
    ui.has_kids = srow.is_some_and(|x| x.kids > 0);
    ui.open = srow.is_some_and(|x| x.kids > 0 && x.hidden == 0);
    if r.is_empty() {
        return n;
    }
    let mut body = r;
    if r.height >= 8 && st.samples > 0 {
        put(f, band(r, 0, 1), vec![meter_strip(st, r.width as usize)]);
        body = band(r, 1, r.height as usize);
    }
    let q = ui.filter.trim();
    let mut title = vec![Span::raw("Processes")];
    title.push(Span::styled(
        if ui.tree { " · tree" } else { " · list" },
        ui::muted(),
    ));
    if !q.is_empty() || ui.typing {
        title.push(Span::styled("  / ", ui::fg(c::ACCENT())));
        title.push(Span::styled(ui::trunc(&clean(q), 30), ui::bold()));
        if ui.typing {
            title.push(Span::styled("▏", ui::fg(c::ACCENT())));
        }
    }
    let total = if st.procs_total > 0 {
        st.procs_total
    } else {
        list.len()
    };
    let shown = if q.is_empty() {
        format!("{} shown of {}", count(n), count(total))
    } else {
        format!(
            "{} match · {} total",
            count(rows.iter().filter(|x| !x.dim).count()),
            count(total)
        )
    };
    let right = Line::from(vec![
        Span::styled(shown, ui::muted()),
        Span::styled(
            format!(" · by {} {}", ui.sort.name(), ui.sort.arrow(ui.rev)),
            ui::faint(),
        ),
    ]);
    let bottom =
        (n > 0).then(|| Line::from(Span::styled(format!("{}/{n}", ui.sel + 1), ui::faint())));
    let inner = panel_b(f, body, Line::from(title), Some(right), bottom);
    if inner.is_empty() || waiting(f, inner, st, epoch()) {
        return n;
    }
    if n == 0 {
        let msg = if !q.is_empty() {
            format!("nothing matches “{}”", ui::trunc(&clean(q), 40))
        } else {
            "no process data on this machine".to_string()
        };
        centered(f, inner, Line::from(Span::styled(msg, ui::muted())));
        return n;
    }
    let (w, h) = (inner.width as usize, inner.height as usize);
    // details at the bottom: a rule, a facts line and the command line
    let det_h = if h >= 14 {
        4
    } else if h >= 9 {
        2
    } else {
        0
    };
    let rows_h = h.saturating_sub(1 + det_h);
    ui.page = rows_h.max(1);
    let sel = ui.sel;
    let mut off = ui.scroll.min(n.saturating_sub(rows_h.max(1)));
    if rows_h > 0 {
        if sel < off {
            off = sel;
        } else if sel >= off + rows_h {
            off = sel + 1 - rows_h;
        }
    }
    ui.scroll = off;
    // the bar column, then the cells
    let tw = w.saturating_sub(1);
    let colv = pick_cols(tw, &PC_ORDER, &PC_DROP);
    let fixed: usize = colv
        .iter()
        .filter(|c| **c != PC::Cmd)
        .map(|c| pc_w(*c) + 1)
        .sum();
    let cmd_w = tw.saturating_sub(fixed);
    let mut lines = Vec::with_capacity(rows_h + 1);
    let mut hd = Ln::new();
    hd.s(" ", Style::new());
    hd.add(head_line(&colv, ui.sort, ui.rev).spans());
    lines.push(hd.line());
    let mem_total = fin(st.mem_total);
    for (i, x) in rows.iter().enumerate().skip(off).take(rows_h) {
        let mut l = Ln::new();
        l.s(" ", Style::new());
        cells(&mut l, &colv[..colv.len() - 1], x.p, mem_total, x.dim);
        l.s(" ", Style::new());
        // command: guides, the name bold, arguments muted, matches underlined
        let gw = ui::width(&x.guide);
        let cap = cmd_w / 2;
        let guide = if gw > cap {
            ui::trunc_left(&x.guide, cap)
        } else {
            x.guide.clone()
        };
        let gw = ui::width(&guide);
        if gw > 0 {
            let (lines_part, fold) = match guide.strip_suffix("▸ ") {
                Some(g) => (g.to_string(), true),
                None => (guide.clone(), false),
            };
            l.s(lines_part, ui::fg(c::LINE()));
            if fold {
                l.s("▸ ", ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD));
            }
        }
        let (txt, nl) = short_cmd(x.p);
        let hl = if q.is_empty() || x.dim {
            vec![]
        } else {
            crate::fuzzy::score(q, &txt)
                .map(|s| s.1)
                .unwrap_or_default()
        };
        let mut room = cmd_w.saturating_sub(gw);
        let more = if x.hidden > 0 {
            format!(" (+{})", x.hidden)
        } else {
            String::new()
        };
        room = room.saturating_sub(ui::width(&more));
        let name_st = if x.dim { ui::faint() } else { name_style(x.p) };
        let cmd = cmd_spans(&txt, nl, &hl, name_st, room);
        l.add(cmd.spans);
        if !more.is_empty() {
            l.s(more, ui::fg(c::ACCENT()));
        }
        let line = if i == sel {
            Line::from(fit(l.spans(), w)).style(ui::selected())
        } else {
            l.line()
        };
        lines.push(line);
    }
    put(f, band(inner, 0, lines.len()), lines);
    if sel >= off && sel < off + rows_h {
        if let Some(cell) = f
            .buffer_mut()
            .cell_mut((inner.x, inner.y + 1 + clamp16(sel - off)))
        {
            cell.set_symbol("┃").set_fg(c::ACCENT());
        }
    }
    if det_h > 0 {
        if let Some(x) = rows.get(sel) {
            let dr = band(inner, h - det_h, det_h);
            let mut dl = vec![Line::from(Span::styled("─".repeat(w), ui::fg(c::LINE())))];
            dl.extend(details(x.p, st, list, w, det_h - 1));
            put(f, dr, dl);
        }
    }
    n
}

/// The selected process: who and what it is, then its whole command line.
fn details(p: &Proc, st: &Stats, list: &[Proc], w: usize, h: usize) -> Vec<Line<'static>> {
    let mut items: Vec<(u8, Vec<Span<'static>>)> = Vec::new();
    let name = clean(p.name.trim());
    items.push((
        0,
        vec![
            Span::styled("▸ ", ui::fg(c::ACCENT())),
            Span::styled(
                if name.is_empty() { "?".into() } else { name },
                name_style(p),
            ),
        ],
    ));
    items.push((0, kv("pid", clean(&p.pid), ui::bold())));
    if !p.ppid.trim().is_empty() {
        let pn = list
            .iter()
            .find(|x| x.pid == p.ppid)
            .map(|x| format!(" {}", clean(x.name.trim())))
            .unwrap_or_default();
        items.push((
            4,
            kv("parent", format!("{}{pn}", clean(&p.ppid)), ui::muted()),
        ));
    }
    if !p.user.trim().is_empty() {
        items.push((2, kv("user", clean(&p.user), ui::muted())));
    }
    let sw = state_word(&p.state);
    if !sw.is_empty() {
        items.push((3, vec![Span::styled(sw, state_style(&p.state))]));
    }
    if p.threads > 0 {
        items.push((6, kv("threads", p.threads.to_string(), ui::muted())));
    }
    if let Some(a) = p.age {
        items.push((1, kv("up", age(a), ui::muted())));
    }
    let mut cpu = vec![
        Span::styled("cpu ", ui::faint()),
        Span::styled(
            format!("{}%", cpu_txt(p.cpu)),
            cpu_style(p.cpu).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(a) = p.avg_cpu.filter(|v| v.is_finite()) {
        cpu.push(Span::styled(
            format!(" now · {}% avg", cpu_txt(a)),
            ui::muted(),
        ));
    }
    items.push((0, cpu));
    if fin(p.time) > 0.0 {
        items.push((5, kv("cpu time", ptime(p.time), ui::muted())));
    }
    let mut mem = kv("rss", by(p.rss), ui::bold());
    if st.mem_total > 0.0 {
        mem.push(Span::styled(
            format!(" ({:.1}%)", ratio(p.rss, st.mem_total) * 100.0),
            ui::muted(),
        ));
    }
    items.push((1, mem));
    for g in st.gpu_procs.iter().filter(|g| g.pid == p.pid) {
        let mut s = vec![Span::styled(
            format!("GPU{} ", clean(&g.gpu)),
            ui::fg(c::MAUVE()).add_modifier(Modifier::BOLD),
        )];
        s.push(Span::styled(by(g.mem), ui::fg(c::MAUVE())));
        if let Some(sm) = ok(g.sm) {
            s.push(Span::styled(format!(" · sm {}", pct(sm)), ui::muted()));
        }
        items.push((2, s));
    }
    let mut out = vec![Line::from(join_prio(items, w))];
    if h >= 2 {
        let cmd = clean(p.cmd.trim());
        let cmd = if cmd.is_empty() {
            format!("[{}]", clean(p.name.trim()))
        } else {
            cmd
        };
        let wrapped = ui::wrap(&cmd, w.max(1));
        let k = h - 1;
        for (i, l) in wrapped.iter().take(k).enumerate() {
            let t = if i + 1 == k && wrapped.len() > k {
                ui::trunc(&format!("{l}…"), w)
            } else {
                l.clone()
            };
            out.push(Line::from(Span::styled(t, ui::muted())));
        }
    }
    out
}

/// A small top-N table for the other views: PID [USER] CPU% [AVG%] MEM%
/// [RSS] [UPTIME] COMMAND, sorted by `sort`; `n` lines with the header.
pub(super) fn top_lines(st: &Stats, sort: Sort, w: usize, n: usize) -> Vec<Line<'static>> {
    let list = plist(st);
    if n == 0 || w == 0 {
        return vec![];
    }
    if list.is_empty() {
        return vec![Line::from(Span::styled(
            "no process data on this machine",
            ui::faint(),
        ))];
    }
    let mut v: Vec<&Proc> = list.iter().collect();
    sort_procs(&mut v, sort);
    let want = [
        PC::Pid,
        PC::User,
        PC::Cpu,
        PC::Avg,
        PC::Mem,
        PC::Rss,
        PC::Up,
        PC::Cmd,
    ];
    let colv = pick_cols(w, &want, &[PC::Avg, PC::User, PC::Up, PC::Rss, PC::Mem]);
    let mut lines = vec![head_line(&colv, sort, false).line()];
    let fixed: usize = colv
        .iter()
        .filter(|c| **c != PC::Cmd)
        .map(|c| pc_w(*c) + 1)
        .sum();
    let cmd_w = w.saturating_sub(fixed);
    let mem_total = fin(st.mem_total);
    for p in v.into_iter().take(n.saturating_sub(1)) {
        let mut l = Ln::new();
        cells(&mut l, &colv[..colv.len() - 1], p, mem_total, false);
        l.s(" ", Style::new());
        let (txt, nl) = short_cmd(p);
        l.add(cmd_spans(&txt, nl, &[], name_style(p), cmd_w).spans);
        lines.push(l.line());
    }
    lines
}

#[cfg(test)]
pub(super) fn tree_text(list: &[Proc], ui: &ProcUi) -> Vec<String> {
    build(list, ui)
        .iter()
        .map(|r| format!("{}{}", r.guide, r.p.name))
        .collect()
}
