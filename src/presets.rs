//! Presets: named starting points — one session or several, on one machine
//! or many — declared in aleph.toml (or a file it includes), shareable:
//!
//! ```toml
//! [presets.ml]
//! description = "Train on b31, review locally"
//! layout = "grid"            # tabs | grid | columns | rows | herdr
//! # herdr: exactly this tree (aleph preset save writes it from a real tab)
//! # tree = { right = 0.6, first = "train", second = "review" }
//! focus = "train"            # open this one in Focus afterwards
//! on_missing_host = "ask"    # ask | skip | fallback (the default machine)
//! vars = { repo = "~/work/hunt" }
//! tags = ["gpu"]
//! [[presets.ml.panes]]
//! name = "train"             # empty: a story name
//! host = "b31"               # or "tag:gpu", "local", "${gpu}"
//! agent = "claude"
//! dir = "${repo}"
//! worktree = "auto"          # none | auto | <branch>
//! args = ["--model", "opus"]
//! prompt = "Run the sweep in plan.md"   # sent once the agent is free
//! ```
//!
//! The old one-session form (`host`, `dir`, `agent`, `args`, `prompt`,
//! `worktree` right under `[presets.<name>]`) still reads as one pane.
//! `plan` turns a preset into what to start where — pure, so it's tested —
//! and `aleph preset launch` carries it out.

use crate::config::Config;
use crate::layouts::{self, Tree};
use serde_json::{json, Value as J};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Worktree {
    #[default]
    None,
    /// a new worktree on `<agent>/<name>`
    Auto,
    Branch(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pane {
    pub name: Option<String>,
    pub host: Option<String>,
    pub agent: Option<String>,
    pub dir: Option<String>,
    pub worktree: Worktree,
    pub args: Vec<String>,
    pub prompt: Option<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Layout {
    /// one herdr tab each
    #[default]
    Tabs,
    /// the panes of one machine in one tab, as a grid
    Grid,
    Columns,
    Rows,
    /// exactly the preset's `tree` (herdr's own layout, saved from a tab)
    Herdr,
}

impl Layout {
    pub fn parse(s: &str) -> Option<Layout> {
        Some(match s {
            "" | "tabs" => Layout::Tabs,
            "grid" => Layout::Grid,
            "columns" | "cols" => Layout::Columns,
            "rows" => Layout::Rows,
            "herdr" | "exact" => Layout::Herdr,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Layout::Tabs => "tabs",
            Layout::Grid => "grid",
            Layout::Columns => "columns",
            Layout::Rows => "rows",
            Layout::Herdr => "herdr",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Missing {
    #[default]
    Ask,
    Skip,
    /// the default machine stands in
    Fallback,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub layout: Layout,
    pub focus: Option<String>,
    pub on_missing_host: Missing,
    pub vars: Vec<(String, String)>,
    pub tags: Vec<String>,
    /// the daemon starts it once its machines are up (opt-in)
    pub autostart: bool,
    pub panes: Vec<Pane>,
    /// `layout = "herdr"`: the tab's exact shape, leaves naming panes (by
    /// name, or number from 1)
    pub tree: Option<Tree<String>>,
    /// the file it's declared in
    pub source: String,
}

fn s(t: &Table, k: &str) -> Option<String> {
    t.get(k)
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
}

fn strings(i: Option<&Item>) -> Vec<String> {
    match i {
        Some(Item::Value(Value::Array(a))) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        // `args = "--model opus"`: split like a shell would (quotes kept simple)
        Some(i) => i.as_str().map(shell_words).unwrap_or_default(),
        None => vec![],
    }
}

/// Split a command line into words: spaces separate, '…' and "…" quote.
pub fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() || any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if !cur.is_empty() || any {
        out.push(cur);
    }
    out
}

fn pairs(i: Option<&Item>) -> Vec<(String, String)> {
    let mut v = Vec::new();
    match i {
        Some(Item::Value(Value::InlineTable(t))) => {
            for (k, x) in t.iter() {
                if let Some(x) = x.as_str() {
                    v.push((k.to_string(), x.to_string()));
                }
            }
        }
        Some(Item::Table(t)) => {
            for (k, x) in t.iter() {
                if let Some(x) = x.as_str() {
                    v.push((k.to_string(), x.to_string()));
                }
            }
        }
        _ => {}
    }
    v
}

fn worktree_of(t: &Table) -> Worktree {
    match t.get("worktree") {
        Some(i) if i.as_bool() == Some(true) => Worktree::Auto,
        Some(i) => match i.as_str().map(str::trim) {
            Some("" | "none" | "false" | "0") | None => Worktree::None,
            Some("auto" | "true" | "1") => Worktree::Auto,
            Some(b) => Worktree::Branch(b.to_string()),
        },
        None => Worktree::None,
    }
}

fn pane_of(t: &Table) -> Pane {
    Pane {
        name: s(t, "name"),
        host: s(t, "host"),
        agent: s(t, "agent"),
        dir: s(t, "dir"),
        worktree: worktree_of(t),
        args: strings(t.get("args")),
        prompt: s(t, "prompt"),
        env: pairs(t.get("env")),
    }
}

/// One preset from its table (the old one-session form or panes).
pub fn parse(name: &str, t: &Table, source: &str) -> Result<Preset, String> {
    let tree = match t.get("tree") {
        None => None,
        Some(i) => Some(
            layouts::read(&layouts::toml_json(i), &layouts::name_leaf).ok_or_else(|| {
                format!(
                    "preset {name}: tree — splits are {{ right = 0.5, first = …, second = … }} (or down), panes their names"
                )
            })?,
        ),
    };
    let layout = match s(t, "layout") {
        // a tree on its own says how
        None if tree.is_some() => Layout::Herdr,
        None => Layout::default(),
        Some(l) => Layout::parse(&l).ok_or_else(|| {
            format!("preset {name}: layout \"{l}\" — use tabs, grid, columns, rows or herdr")
        })?,
    };
    let on_missing_host = match s(t, "on_missing_host").as_deref() {
        None | Some("ask") => Missing::Ask,
        Some("skip") => Missing::Skip,
        Some("fallback") => Missing::Fallback,
        Some(x) => {
            return Err(format!(
                "preset {name}: on_missing_host \"{x}\" — use ask, skip or fallback"
            ))
        }
    };
    let mut panes: Vec<Pane> = match t.get("panes") {
        Some(Item::ArrayOfTables(a)) => a.iter().map(pane_of).collect(),
        Some(Item::Value(Value::Array(a))) => a
            .iter()
            .filter_map(|v| v.as_inline_table())
            .map(|it| pane_of(&it.clone().into_table()))
            .collect(),
        _ => vec![],
    };
    if panes.is_empty() {
        // the one-session form
        let p = pane_of(t);
        if p.host.is_some()
            || p.dir.is_some()
            || p.agent.is_some()
            || p.prompt.is_some()
            || !p.args.is_empty()
        {
            panes.push(p);
        }
    }
    if panes.is_empty() {
        return Err(format!(
            "preset {name}: nothing to start (no panes, no host or folder)"
        ));
    }
    Ok(Preset {
        name: name.to_string(),
        description: s(t, "description").unwrap_or_default(),
        layout,
        focus: s(t, "focus"),
        on_missing_host,
        vars: pairs(t.get("vars")),
        tags: strings(t.get("tags")),
        autostart: t.get("autostart").and_then(Item::as_bool).unwrap_or(false),
        panes,
        tree,
        source: source.to_string(),
    })
}

/// Presets declared in a document (problems kept apart).
pub fn from_doc(doc: &DocumentMut, source: &str) -> (Vec<Preset>, Vec<String>) {
    let mut v = Vec::new();
    let mut bad = Vec::new();
    for key in ["presets", "preset"] {
        if let Some(t) = doc.get(key).and_then(Item::as_table) {
            for (name, item) in t.iter() {
                if let Some(pt) = item.as_table() {
                    match parse(name, pt, source) {
                        Ok(p) => v.push(p),
                        Err(e) => bad.push(e),
                    }
                }
            }
        }
    }
    (v, bad)
}

/// Every preset: from each settings file in precedence order (a later file
/// overrides one of the same name), or the old key=value file's.
pub fn all(cfg: &Config) -> Vec<Preset> {
    all_with_problems(cfg).0
}

pub fn all_with_problems(cfg: &Config) -> (Vec<Preset>, Vec<String>) {
    let mut out: Vec<Preset> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    if !cfg.is_toml() {
        for p in cfg.flat_presets() {
            out.push(from_flat(&p));
        }
        return (out, bad);
    }
    for (_, path) in cfg.files() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = text.parse::<DocumentMut>() else {
            continue;
        };
        let (v, b) = from_doc(&doc, &crate::doctor::tilde(&path));
        bad.extend(b);
        for p in v {
            out.retain(|x| x.name != p.name);
            out.push(p);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, bad)
}

pub fn get(cfg: &Config, name: &str) -> Option<Preset> {
    let name = name.trim_start_matches('@');
    all(cfg).into_iter().find(|p| p.name == name)
}

fn from_flat(p: &crate::config::Preset) -> Preset {
    Preset {
        name: p.name.clone(),
        panes: vec![Pane {
            host: p.host.clone(),
            dir: p.dir.clone(),
            agent: p.agent.clone(),
            args: p.args.as_deref().map(shell_words).unwrap_or_default(),
            prompt: p.prompt.clone(),
            worktree: if p.worktree {
                Worktree::Auto
            } else {
                Worktree::None
            },
            ..Default::default()
        }],
        source: "settings".into(),
        ..Default::default()
    }
}

// ------------------------------------------------------------------ the plan

/// One session to start.
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub host: String,
    pub name: Option<String>,
    pub agent: Option<String>,
    pub dir: Option<String>,
    pub worktree: Worktree,
    pub args: Vec<String>,
    pub prompt: Option<String>,
    pub env: Vec<(String, String)>,
    /// starts in the tab launch `j` opens (same machine, not `tabs`)
    pub split_of: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub launches: Vec<Launch>,
    /// panes left out, and why (a missing machine with `skip`)
    pub skipped: Vec<String>,
    /// panes that need an answer first (a missing machine with `ask`)
    pub ask: Vec<String>,
    /// tabs laid out exactly: (the launch that opens it — its first pane —,
    /// its shape over launch numbers)
    pub tabs: Vec<(usize, Tree<usize>)>,
}

impl Plan {
    /// The order to start them in: a tab's first pane before the others in it.
    pub fn order(&self) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for (i, l) in self.launches.iter().enumerate() {
            if let Some(j) = l.split_of {
                if !out.contains(&j) {
                    out.push(j);
                }
            }
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }

    /// The tab launch `i` opens, if it opens one with others in it.
    pub fn tab_of(&self, i: usize) -> Option<&Tree<usize>> {
        self.tabs.iter().find(|(j, _)| *j == i).map(|(_, t)| t)
    }
}

/// A tree leaf → the pane it names: its name, else its number (from 1).
fn pane_named(p: &Preset, leaf: &str) -> Option<usize> {
    p.panes
        .iter()
        .position(|x| x.name.as_deref() == Some(leaf))
        .or_else(|| {
            leaf.parse::<usize>()
                .ok()
                .filter(|n| (1..=p.panes.len()).contains(n))
                .map(|n| n - 1)
        })
}

/// `${name}` → its value (vars, then the overrides); unknown ones stay.
pub fn expand(s: &str, vars: &[(String, String)]) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

/// What `preset` starts where. `hosts` are the fleet's machines with their
/// tags; `default` is the default machine; `vars` override the preset's.
pub fn plan(
    p: &Preset,
    hosts: &[(String, Vec<String>)],
    default: Option<&str>,
    vars: &[(String, String)],
) -> Plan {
    let mut all_vars = p.vars.clone();
    for (k, v) in vars {
        all_vars.retain(|(x, _)| x != k);
        all_vars.push((k.clone(), v.clone()));
    }
    let x = |s: &Option<String>| s.as_ref().map(|v| expand(v, &all_vars));
    let mut out = Plan::default();
    // pane number → its launch (None: skipped or asked about)
    let mut launch_of: Vec<Option<usize>> = Vec::new();
    for (i, pane) in p.panes.iter().enumerate() {
        launch_of.push(None);
        let label = pane
            .name
            .clone()
            .unwrap_or_else(|| format!("pane {}", i + 1));
        let want = x(&pane.host).or_else(|| default.map(String::from));
        let mut targets: Vec<String> = match &want {
            Some(h) if h.starts_with("tag:") => {
                let tag = &h[4..];
                hosts
                    .iter()
                    .filter(|(_, tags)| tags.iter().any(|t| t == tag))
                    .map(|(a, _)| a.clone())
                    .take(1)
                    .collect()
            }
            Some(h) if hosts.iter().any(|(a, _)| a == h) || h == crate::sshcfg::LOCAL => {
                vec![h.clone()]
            }
            _ => vec![],
        };
        if targets.is_empty() {
            let what = want.unwrap_or_else(|| "no machine".into());
            match p.on_missing_host {
                Missing::Skip => {
                    out.skipped
                        .push(format!("{label}: {what} isn't in the fleet"));
                    continue;
                }
                Missing::Ask => {
                    out.ask.push(format!("{label}: {what} isn't in the fleet"));
                    continue;
                }
                Missing::Fallback => match default {
                    Some(d) => targets.push(d.to_string()),
                    None => {
                        out.skipped.push(format!(
                            "{label}: {what} isn't in the fleet and there's no default"
                        ));
                        continue;
                    }
                },
            }
        }
        let host = targets.remove(0);
        launch_of[i] = Some(out.launches.len());
        out.launches.push(Launch {
            host,
            name: x(&pane.name),
            agent: x(&pane.agent),
            dir: x(&pane.dir),
            worktree: match &pane.worktree {
                Worktree::Branch(b) => Worktree::Branch(expand(b, &all_vars)),
                w => w.clone(),
            },
            args: pane.args.iter().map(|a| expand(a, &all_vars)).collect(),
            prompt: x(&pane.prompt),
            env: pane
                .env
                .iter()
                .map(|(k, v)| (k.clone(), expand(v, &all_vars)))
                .collect(),
            split_of: None,
        });
    }
    // which launches share a tab, in what shape
    let mut shapes: Vec<Tree<usize>> = Vec::new();
    match p.layout {
        Layout::Tabs => {}
        Layout::Herdr => {
            // the tree's panes that start, on its first one's machine, once each
            let mut seen: Vec<usize> = Vec::new();
            let t = p.tree.as_ref().and_then(|t| {
                t.map(&mut |leaf| pane_named(p, leaf).and_then(|n| launch_of[n]))
                    .prune(&mut |l| match l {
                        Some(j) if !seen.contains(j) => {
                            seen.push(*j);
                            true
                        }
                        _ => false,
                    })
            });
            if let Some(t) = t.map(|t| t.map(&mut |l| l.unwrap_or_default())) {
                let host = out.launches[*t.first()].host.clone();
                shapes.extend(t.prune(&mut |j| out.launches[*j].host == host));
            }
        }
        Layout::Grid | Layout::Columns | Layout::Rows => {
            // every machine's launches, in order, in one tab of its own
            let mut hosts: Vec<String> = Vec::new();
            for l in &out.launches {
                if !hosts.contains(&l.host) {
                    hosts.push(l.host.clone());
                }
            }
            for h in hosts {
                let on: Vec<usize> = (0..out.launches.len())
                    .filter(|j| out.launches[*j].host == h)
                    .collect();
                let shape = match p.layout {
                    Layout::Columns => layouts::line(on.len(), layouts::Dir::Right),
                    Layout::Rows => layouts::line(on.len(), layouts::Dir::Down),
                    _ => layouts::grid(on.len()),
                };
                shapes.extend(shape.map(|t| t.map(&mut |k| on[*k])));
            }
        }
    }
    for t in shapes.into_iter().filter(|t| t.len() > 1) {
        let root = *t.first();
        for j in t.leaves().into_iter().skip(1) {
            out.launches[*j].split_of = Some(root);
        }
        out.tabs.push((root, t));
    }
    out
}

/// Problems with a preset against the fleet and the agent registry.
pub fn validate(p: &Preset, hosts: &[(String, Vec<String>)]) -> Vec<String> {
    let mut v = Vec::new();
    let known_var = |s: &str| {
        let mut rest = s;
        while let Some(i) = rest.find("${") {
            let Some(j) = rest[i..].find('}') else {
                return Some(rest[i..].to_string());
            };
            let name = &rest[i + 2..i + j];
            if !p.vars.iter().any(|(k, _)| k == name) {
                return Some(name.to_string());
            }
            rest = &rest[i + j + 1..];
        }
        None
    };
    for (i, pane) in p.panes.iter().enumerate() {
        let label = pane
            .name
            .clone()
            .unwrap_or_else(|| format!("pane {}", i + 1));
        if let Some(a) = &pane.agent {
            if !a.contains("${") && crate::agents::resolve(a).is_err() && a != crate::agents::SHELL
            {
                v.push(format!("{label}: unknown agent \"{a}\""));
            }
        }
        if let Some(h) = &pane.host {
            if let Some(tag) = h.strip_prefix("tag:") {
                if !hosts.iter().any(|(_, t)| t.iter().any(|x| x == tag)) {
                    v.push(format!("{label}: no machine is tagged {tag}"));
                }
            } else if !h.contains("${")
                && !hosts.iter().any(|(a, _)| a == h)
                && h != crate::sshcfg::LOCAL
            {
                v.push(format!("{label}: {h} isn't in the fleet"));
            }
        }
        if let Some(d) = &pane.dir {
            if !d.contains("${") && !d.starts_with('/') && !d.starts_with('~') {
                v.push(format!(
                    "{label}: folder {d} — use an absolute path (or ~/…)"
                ));
            }
        }
        for s in [&pane.host, &pane.agent, &pane.dir, &pane.prompt, &pane.name]
            .into_iter()
            .flatten()
        {
            if let Some(name) = known_var(s) {
                v.push(format!("{label}: ${{{name}}} isn't in vars"));
            }
        }
    }
    match (&p.tree, p.layout) {
        (None, Layout::Herdr) => v.push(
            "layout herdr needs a tree (aleph preset save NAME host:a host:b… writes one from a tab)"
                .into(),
        ),
        (Some(_), l) if l != Layout::Herdr => v.push(format!(
            "it has a tree, but layout = \"{}\" (the tree is used with layout = \"herdr\")",
            l.name()
        )),
        (Some(t), _) => {
            let mut seen: Vec<usize> = Vec::new();
            let mut on: Vec<String> = Vec::new();
            for leaf in t.leaves() {
                match pane_named(p, leaf) {
                    None => v.push(format!("tree: no pane \"{leaf}\" (a name, or a number from 1)")),
                    Some(i) if seen.contains(&i) => {
                        v.push(format!("tree: \"{leaf}\" is in it twice"))
                    }
                    Some(i) => {
                        seen.push(i);
                        if let Some(h) = &p.panes[i].host {
                            if !on.contains(h) {
                                on.push(h.clone());
                            }
                        }
                    }
                }
            }
            if on.len() > 1 && !on.iter().any(|h| h.contains("${") || h.starts_with("tag:")) {
                v.push(format!(
                    "tree: its panes are on {} — a herdr tab lives on one machine (the others get tabs of their own)",
                    on.join(", ")
                ));
            }
        }
        _ => {}
    }
    v
}

// ------------------------------------------------------------------ writing

fn to_table(p: &Preset) -> Table {
    let mut t = Table::new();
    if !p.description.is_empty() {
        t.insert("description", value(&p.description));
    }
    if p.layout != Layout::Tabs {
        t.insert("layout", value(p.layout.name()));
    }
    if let Some(f) = &p.focus {
        t.insert("focus", value(f));
    }
    match p.on_missing_host {
        Missing::Ask => {}
        Missing::Skip => {
            t.insert("on_missing_host", value("skip"));
        }
        Missing::Fallback => {
            t.insert("on_missing_host", value("fallback"));
        }
    }
    if !p.vars.is_empty() {
        let mut it = InlineTable::new();
        for (k, v) in &p.vars {
            it.insert(k, v.as_str().into());
        }
        t.insert("vars", value(it));
    }
    if !p.tags.is_empty() {
        let a: Array = p.tags.iter().map(|x| x.as_str()).collect();
        t.insert("tags", value(a));
    }
    if p.autostart {
        t.insert("autostart", value(true));
    }
    if let Some(tree) = &p.tree {
        t.insert("tree", value(layouts::to_toml(tree)));
    }
    let mut panes = ArrayOfTables::new();
    for pane in &p.panes {
        let mut pt = Table::new();
        for (k, v) in [
            ("name", &pane.name),
            ("host", &pane.host),
            ("agent", &pane.agent),
            ("dir", &pane.dir),
        ] {
            if let Some(v) = v {
                pt.insert(k, value(v));
            }
        }
        match &pane.worktree {
            Worktree::None => {}
            Worktree::Auto => {
                pt.insert("worktree", value("auto"));
            }
            Worktree::Branch(b) => {
                pt.insert("worktree", value(b));
            }
        }
        if !pane.args.is_empty() {
            let a: Array = pane.args.iter().map(|x| x.as_str()).collect();
            pt.insert("args", value(a));
        }
        if let Some(pr) = &pane.prompt {
            pt.insert("prompt", value(pr));
        }
        if !pane.env.is_empty() {
            let mut it = InlineTable::new();
            for (k, v) in &pane.env {
                it.insert(k, v.as_str().into());
            }
            pt.insert("env", value(it));
        }
        panes.push(pt);
    }
    t.insert("panes", Item::ArrayOfTables(panes));
    t
}

/// The preset as TOML (`aleph preset export`).
pub fn to_toml(p: &Preset) -> String {
    let mut doc = DocumentMut::new();
    let mut presets = Table::new();
    presets.set_implicit(true);
    presets.insert(&p.name, Item::Table(to_table(p)));
    doc.insert("presets", Item::Table(presets));
    doc.to_string()
}

/// Machine names → `${host1}`, `${host2}` (with a `vars` entry each), so a
/// team can map them to their own machines.
pub fn generic(p: &Preset) -> Preset {
    let mut q = p.clone();
    let mut seen: Vec<String> = Vec::new();
    for pane in &mut q.panes {
        if let Some(h) = &pane.host {
            if h.starts_with("tag:") || h.contains("${") {
                continue;
            }
            let i = match seen.iter().position(|x| x == h) {
                Some(i) => i,
                None => {
                    seen.push(h.clone());
                    seen.len() - 1
                }
            };
            pane.host = Some(format!("${{host{}}}", i + 1));
        }
    }
    for (i, h) in seen.iter().enumerate() {
        q.vars.push((format!("host{}", i + 1), h.clone()));
    }
    q
}

/// Write (or replace) a preset in the main settings file.
pub fn save(cfg: &mut Config, p: &Preset) -> std::io::Result<()> {
    let p = p.clone();
    cfg.edit_doc(move |doc| put(doc, &p))
}

/// Put (or replace) a preset in a settings document (`save`, and setup's
/// preview of what it will write).
pub fn put(doc: &mut DocumentMut, p: &Preset) {
    let t = to_table(p);
    let name = p.name.clone();
    // an old flat [presets.<name>] or [preset.<name>] goes away first
    for key in ["preset", "presets"] {
        if let Some(x) = doc.get_mut(key).and_then(Item::as_table_mut) {
            x.remove(&name);
        }
    }
    if doc.get("presets").and_then(Item::as_table).is_none() {
        let mut presets = Table::new();
        presets.set_implicit(true);
        doc.insert("presets", Item::Table(presets));
    }
    if let Some(x) = doc.get_mut("presets").and_then(Item::as_table_mut) {
        x.insert(&name, Item::Table(t));
    }
}

pub fn remove(cfg: &mut Config, name: &str) -> std::io::Result<()> {
    let name = name.to_string();
    cfg.edit_doc(move |doc| {
        for key in ["preset", "presets"] {
            if let Some(x) = doc.get_mut(key).and_then(Item::as_table_mut) {
                x.remove(&name);
            }
        }
    })
}

pub fn to_json(p: &Preset) -> J {
    json!({
        "name": p.name, "description": p.description, "layout": p.layout.name(),
        "focus": p.focus, "autostart": p.autostart, "tags": p.tags, "source": p.source,
        "tree": p.tree.as_ref().map(|t| layouts::toml_json(&Item::Value(layouts::to_toml(t)))),
        "shape": p.tree.as_ref().map(layouts::describe),
        "vars": p.vars.iter().map(|(k, v)| json!({"name": k, "value": v})).collect::<Vec<_>>(),
        "panes": p.panes.iter().map(|x| json!({
            "name": x.name, "host": x.host, "agent": x.agent, "dir": x.dir,
            "worktree": match &x.worktree { Worktree::None => J::Null, Worktree::Auto => json!("auto"), Worktree::Branch(b) => json!(b) },
            "args": x.args, "prompt": x.prompt,
        })).collect::<Vec<_>>(),
        // the old one-session fields, for scripts that read them
        "host": p.panes.first().and_then(|x| x.host.clone()),
        "dir": p.panes.first().and_then(|x| x.dir.clone()),
        "agent": p.panes.first().and_then(|x| x.agent.clone()),
        "prompt": p.panes.first().and_then(|x| x.prompt.clone()),
        "worktree": p.panes.first().is_some_and(|x| x.worktree != Worktree::None),
        "args": p.panes.first().map(|x| x.args.join(" ")).filter(|a| !a.is_empty()),
    })
}

// ------------------------------------------------------------------ launching

/// The fleet's machines with their tags (`host.<a>.tags = "gpu,lab"`).
pub fn fleet_tags(cfg: &Config) -> Vec<(String, Vec<String>)> {
    cfg.fleet()
        .into_iter()
        .map(|h| {
            let tags = cfg
                .host_get(&h, "tags")
                .map(|t| {
                    t.split([',', ' '])
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            (h, tags)
        })
        .collect()
}

/// `~/x` on a machine → its absolute path (its `$HOME`, asked once).
fn remote_path(host: &str, p: &str, homes: &mut Vec<(String, String)>) -> String {
    let Some(rest) = p
        .strip_prefix("~/")
        .or(if p == "~" { Some("") } else { None })
    else {
        return p.to_string();
    };
    let home = match homes.iter().find(|(h, _)| h == host) {
        Some((_, x)) => x.clone(),
        None => {
            let o = crate::proc::run(
                &mut crate::sshcfg::shell(host, "printf %s \"$HOME\""),
                std::time::Duration::from_secs(15),
                None,
            );
            let x = o
                .ok()
                .filter(|o| o.ok())
                .map(|o| o.text().trim().to_string())
                .unwrap_or_default();
            homes.push((host.to_string(), x.clone()));
            x
        }
    };
    if home.is_empty() {
        return p.to_string();
    }
    if rest.is_empty() {
        home
    } else {
        format!("{}/{rest}", home.trim_end_matches('/'))
    }
}

/// What was started: (host, session name, pane).
pub type Started = (String, String, String);

/// Carry out a plan: tabs first, splits into them, prompts queued for when
/// each agent is free. `reuse`: a pane whose name already runs there is kept.
/// `step` hears about each one as it goes.
pub fn launch(
    cfg: &Config,
    plan: &Plan,
    reuse: bool,
    step: &mut dyn FnMut(String),
) -> (Vec<Started>, Vec<String>) {
    use crate::herdr::Herdr;
    let mut started: Vec<Option<Started>> = vec![None; plan.launches.len()];
    // the panes of each laid-out tab, by the launch that opened it
    let mut tab_panes: Vec<Option<Vec<String>>> = vec![None; plan.launches.len()];
    let mut errors: Vec<String> = Vec::new();
    let mut homes: Vec<(String, String)> = Vec::new();
    let mut live: Vec<String> = Vec::new();
    for i in plan.order() {
        let l = &plan.launches[i];
        let t = cfg.target_for(&l.host, crate::herdr::machine_for);
        let h = Herdr::new(&t);
        let running = h.sessions().unwrap_or_default();
        live.extend(running.iter().map(|s| s.name.clone()));
        if reuse {
            if let Some(s) = l
                .name
                .as_ref()
                .and_then(|n| running.iter().find(|s| &s.name == n))
            {
                step(format!("{} already runs on {} — kept", s.name, l.host));
                started[i] = Some((l.host.clone(), s.name.clone(), s.pane_id.clone()));
                continue;
            }
        }
        let dir = l.dir.as_ref().map(|d| remote_path(&l.host, d, &mut homes));
        let agent = l.agent.clone().unwrap_or_else(|| {
            crate::agents::default_agent_for(cfg, &l.host)
                .id
                .to_string()
        });
        let queue_prompt = |name: &str| {
            if let Some(p) = &l.prompt {
                crate::queue::add(crate::queue::Task {
                    host: l.host.clone(),
                    prompt: p.clone(),
                    to: Some(name.to_string()),
                    ..Default::default()
                });
            }
        };
        let r: Result<Started, String> = match l.split_of {
            // its pane in the tab its first one opened
            Some(j) => {
                // a name that's taken there gets a number (herdr's are unique)
                let name = match &l.name {
                    Some(n) => crate::names::numbered(n, &live),
                    None => crate::names::pick(&live, &[], crate::names::seed() ^ i as u64),
                };
                live.push(name.clone());
                let pos = plan
                    .tab_of(j)
                    .and_then(|t| t.leaves().iter().position(|x| **x == i));
                let made = tab_panes[j]
                    .as_ref()
                    .and_then(|v| pos.and_then(|k| v.get(k)).cloned());
                match (made, started[j].clone()) {
                    (Some(pane), _) => h.start_in_pane(&pane, &name, &agent, &l.args).map(|_| pane),
                    // its first one was already running (--reuse): beside it,
                    // by herdr's CLI — no OpenRouter key there (its command line)
                    (None, Some((_, _, root_pane))) => {
                        if crate::openrouter::wants(cfg, &agent)
                            && crate::openrouter::configured(cfg)
                        {
                            step(format!(
                                "{name}: {}",
                                crate::openrouter::not_passed(&l.host, "can't split a running tab")
                            ));
                        }
                        h.start_in_split(
                            &root_pane,
                            "right",
                            dir.as_deref().map(str::as_bytes),
                            &l.env,
                            &name,
                            &agent,
                            &l.args,
                        )
                    }
                    (None, None) => Err("its tab didn't open".into()),
                }
                .map(|pane| {
                    queue_prompt(&name);
                    (l.host.clone(), name, pane)
                })
            }
            None => {
                let mut o = match crate::cli::parse_new(&["--host".into(), l.host.clone()], cfg) {
                    Ok(x) => x.opts,
                    Err(e) => {
                        errors.push(format!("{}: {e}", l.host));
                        continue;
                    }
                };
                o.agent = agent.clone();
                o.dir = dir.as_deref().map(|d| crate::path::normalize(d.as_bytes()));
                o.name = l.name.clone();
                o.args.extend(l.args.iter().cloned());
                o.prompt = l.prompt.clone();
                o.worktree = match &l.worktree {
                    Worktree::None => None,
                    Worktree::Auto => Some(String::new()),
                    Worktree::Branch(b) => Some(b.clone()),
                };
                o.env = l.env.clone();
                o.no_focus = true;
                o.live = live.clone();
                // a tab shared with others: all its panes at once, in its shape
                o.layout = plan.tab_of(i).map(|tree| {
                    tree.map(&mut |j| {
                        let m = &plan.launches[*j];
                        // each pane's own agent says whether it gets the OpenRouter key
                        let ag = m.agent.clone().unwrap_or_else(|| {
                            crate::agents::default_agent_for(cfg, &m.host)
                                .id
                                .to_string()
                        });
                        layouts::Leaf {
                            cwd: m.dir.as_ref().map(|d| {
                                crate::path::show(&crate::path::normalize(
                                    remote_path(&m.host, d, &mut homes).as_bytes(),
                                ))
                            }),
                            env: m.env.clone(),
                            openrouter: crate::openrouter::wants(cfg, &ag),
                        }
                    })
                });
                crate::cli::fill_names(&mut o);
                h.new_tab(&o, &mut |_, m| step(m)).and_then(|(v, panes)| {
                    crate::launch::after_start(&t, &o, &v);
                    let s = v.first().ok_or("nothing started")?;
                    live.push(s.name.clone());
                    tab_panes[i] = Some(panes);
                    Ok((l.host.clone(), s.name.clone(), s.pane.clone()))
                })
            }
        };
        match r {
            Ok(x) => {
                step(format!("{} on {}", x.1, x.0));
                started[i] = Some(x);
            }
            Err(e) => errors.push(format!("{}: {e}", l.host)),
        }
    }
    (started.into_iter().flatten().collect(), errors)
}

// ------------------------------------------------------------------ the CLI

/// A preset from running sessions (`aleph preset save NAME host:name…`): each
/// one's machine, agent and folder, with the arguments and prompt it was
/// started with when the history knows them.
pub fn from_sessions(name: &str, sessions: &[crate::herdr::Session]) -> Preset {
    let hist = crate::history::load(500);
    let panes = sessions
        .iter()
        .map(|s| {
            let h = hist
                .iter()
                .rev()
                .find(|e| e.host == s.host && e.name == s.name);
            Pane {
                name: Some(s.name.clone()),
                host: Some(s.host.clone()),
                agent: Some(s.agent.clone()),
                dir: Some(
                    h.map(|e| e.dir.clone())
                        .filter(|d| !d.is_empty())
                        .unwrap_or_else(|| s.cwd.clone()),
                ),
                worktree: Worktree::None,
                args: h.map(|e| e.args.clone()).unwrap_or_default(),
                prompt: h.and_then(|e| e.prompt.clone()),
                env: vec![],
            }
        })
        .collect();
    Preset {
        name: name.to_string(),
        panes,
        ..Default::default()
    }
}

/// Two or more sessions, all in one herdr tab.
fn one_tab(v: &[crate::herdr::Session]) -> bool {
    v.len() > 1
        && !v[0].tab_id.is_empty()
        && v.iter()
            .all(|s| s.host == v[0].host && s.tab_id == v[0].tab_id)
}

/// The shape of the tab `v` share, from herdr (`layout.export`), its leaves
/// their names; the tab's other panes left out (their room goes to their
/// neighbours).
pub fn herdr_tree(cfg: &Config, v: &[crate::herdr::Session]) -> Result<Tree<String>, String> {
    let first = v.first().ok_or("no sessions")?;
    if v.iter().any(|s| s.host != first.host) {
        return Err("they're on different machines — a herdr tab lives on one".into());
    }
    let t = cfg.target_for(&first.host, crate::herdr::machine_for);
    let (_, tree) = crate::herdr::Herdr::new(&t).export_layout(&first.pane_id)?;
    let name_of = |g: &layouts::Got| {
        g.pane_id
            .as_ref()
            .and_then(|id| v.iter().find(|s| &s.pane_id == id))
            .map(|s| s.name.clone())
    };
    let named = tree
        .prune(&mut |g| name_of(g).is_some())
        .map(|t| t.map(&mut |g| name_of(g).unwrap_or_default()))
        .filter(|t| t.len() > 1)
        .ok_or("fewer than two of them are in that tab")?;
    Ok(named)
}

fn print_plan(p: &Preset, plan: &Plan) {
    use crate::out::*;
    let shown = |j: usize| {
        plan.launches[j]
            .name
            .clone()
            .unwrap_or_else(|| format!("pane {}", j + 1))
    };
    for l in &plan.launches {
        let mut what =
            vec![crate::agents::label(l.agent.as_deref().unwrap_or("default agent")).to_string()];
        if let Some(d) = &l.dir {
            what.push(format!("in {d}"));
        }
        if let Some(j) = l.split_of {
            what.push(format!("(in {}'s tab)", shown(j)));
        }
        if let Some(pr) = &l.prompt {
            what.push(format!("then “{}”", crate::ui::trunc(pr, 50)));
        }
        say(ratatui::text::Line::raw(format!(
            "  {:<10} {:<14} {}",
            l.host,
            l.name.clone().unwrap_or_else(|| "(a story name)".into()),
            what.join(" ")
        )));
    }
    for (j, t) in &plan.tabs {
        say(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(
                format!("  {:<10} ", plan.launches[*j].host),
                crate::ui::muted(),
            ),
            ratatui::text::Span::raw(format!(
                "one tab: {}",
                layouts::describe(&t.map(&mut |k| shown(*k)))
            )),
        ]));
    }
    for s in &plan.skipped {
        warn(&format!("skipped {s}"));
    }
    for s in &plan.ask {
        warn(&format!(
            "{s} — on_missing_host = ask: pick one with --var, or set skip / fallback"
        ));
    }
    if !p.description.is_empty() {
        hint(&p.description);
    }
}

fn vars_of(rest: &[String]) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--var" {
            if let Some((k, x)) = rest.get(i + 1).and_then(|kv| kv.split_once('=')) {
                v.push((k.to_string(), x.to_string()));
            }
            i += 1;
        }
        i += 1;
    }
    v
}

const USAGE: &str = "aleph preset [ls | show NAME | launch NAME [--var k=v]… [--reuse] [--dry-run] | add NAME [--host h] [--dir /abs] [--agent k] [--worktree] [--prompt text] [-- args…] | save NAME host:session… [--layout tabs|grid|columns|rows|herdr] | dup NAME NEW | rm NAME | export NAME [--generic] | import FILE|URL [--dry-run] [--yes] | validate [NAME] | edit]";

pub fn cmd(mut a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::*;
    use std::process::ExitCode;
    let r = a.rest.to_vec();
    let sub = r.first().map(String::as_str).unwrap_or("ls");
    let name = r.get(1).cloned().unwrap_or_default();
    match sub {
        "ls" | "list" => {
            let (v, bad) = all_with_problems(&a.cfg);
            if a.has("--json") {
                json(&J::Array(v.iter().map(to_json).collect()));
                return ExitCode::SUCCESS;
            }
            if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                for p in &v {
                    let hosts: Vec<String> =
                        p.panes.iter().filter_map(|x| x.host.clone()).collect();
                    println!(
                        "{}\t{}\t{}\t{}",
                        p.name,
                        p.panes.len(),
                        hosts.join(","),
                        p.description
                    );
                }
                return ExitCode::SUCCESS;
            }
            if v.is_empty() {
                ok("no presets yet");
                hint("aleph preset save NAME host:session…  — from what's running; or add one to aleph.toml ([presets.NAME])");
            }
            for p in &v {
                let hosts: Vec<String> = {
                    let mut h: Vec<String> =
                        p.panes.iter().filter_map(|x| x.host.clone()).collect();
                    h.dedup();
                    h
                };
                say(ratatui::text::Line::from(vec![
                    ratatui::text::Span::styled(format!("  @{:<14}", p.name), crate::ui::bold()),
                    ratatui::text::Span::raw(format!(
                        "{} pane{}  ",
                        p.panes.len(),
                        if p.panes.len() == 1 { " " } else { "s" }
                    )),
                    ratatui::text::Span::styled(
                        format!("{:<20}", hosts.join(", ")),
                        crate::ui::muted(),
                    ),
                    ratatui::text::Span::styled(p.description.clone(), crate::ui::faint()),
                ]));
            }
            for b in bad {
                warn(&b);
            }
            ExitCode::SUCCESS
        }
        "show" => {
            let Some(p) = get(&a.cfg, &name) else {
                return fail(&format!("no preset '{name}' (aleph preset ls)"));
            };
            if a.has("--json") {
                json(&to_json(&p));
                return ExitCode::SUCCESS;
            }
            print!("{}", to_toml(&p));
            println!();
            let plan = plan(
                &p,
                &fleet_tags(&a.cfg),
                a.cfg.get("ssh_host").as_deref(),
                &vars_of(&r),
            );
            head("Would start");
            print_plan(&p, &plan);
            ExitCode::SUCCESS
        }
        "launch" | "start" | "run" => {
            let Some(p) = get(&a.cfg, &name) else {
                return fail(&format!("no preset '{name}' (aleph preset ls)"));
            };
            let probs = validate(&p, &fleet_tags(&a.cfg));
            for x in &probs {
                warn(x);
            }
            let plan = plan(
                &p,
                &fleet_tags(&a.cfg),
                a.cfg.get("ssh_host").as_deref(),
                &vars_of(&r),
            );
            if a.has("--dry-run") || plan.launches.is_empty() {
                head(&format!("@{} would start", p.name));
                print_plan(&p, &plan);
                return if plan.launches.is_empty() {
                    fail("nothing to start")
                } else {
                    ExitCode::SUCCESS
                };
            }
            let sp = Spinner::start(format!("Starting @{}", p.name));
            let (started, errors) = launch(&a.cfg, &plan, a.has("--reuse"), &mut |m| sp.set(m));
            sp.stop();
            for (h, n, _) in &started {
                ok(&format!("{n} on {h}"));
            }
            for e in &errors {
                err(e);
            }
            // then, from a terminal: the focus pane in Focus
            let focus = p
                .focus
                .as_ref()
                .and_then(|f| started.iter().find(|(_, n, _)| n == f))
                .or(started.first())
                .map(|(h, _, pane)| format!("{h}:{pane}"));
            if let (Some(q), true) = (focus, crate::term::interactive() && !a.has("--detach")) {
                return crate::tui::run_dashboard(
                    a.cfg,
                    crate::tui::Start {
                        view: crate::tui::View::Fleet,
                        flag: a.flag.clone(),
                        focus: Some(q),
                        ..Default::default()
                    },
                );
            }
            if errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        "add" | "set" => {
            if name.is_empty() || !crate::names::valid(&name) {
                return fail(&format!("usage: {USAGE}"));
            }
            let rest = r[2..].to_vec();
            let mut pane = Pane::default();
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--" => {
                        pane.args = rest[i + 1..].to_vec();
                        break;
                    }
                    "--worktree" => pane.worktree = Worktree::Auto,
                    f @ ("--host" | "--dir" | "--agent" | "--prompt") if i + 1 < rest.len() => {
                        let v = rest[i + 1].clone();
                        if f == "--dir" && !v.starts_with('/') && !v.starts_with('~') {
                            return fail("--dir must be an absolute remote path (or ~/…)");
                        }
                        match f {
                            "--host" => pane.host = Some(v),
                            "--dir" => pane.dir = Some(v),
                            "--agent" => pane.agent = Some(v),
                            _ => pane.prompt = Some(v),
                        }
                        i += 1;
                    }
                    x => return fail(&format!("unexpected '{x}'")),
                }
                i += 1;
            }
            if pane.host.is_none() {
                pane.host = a.cfg.get("ssh_host");
            }
            let p = Preset {
                name: name.clone(),
                panes: vec![pane],
                ..Default::default()
            };
            save_or_flat(&mut a.cfg, &p)
        }
        "save" => {
            if name.is_empty() || !crate::names::valid(&name) || r.len() < 3 {
                return fail("usage: aleph preset save NAME host:session [host:session…] [--layout tabs|grid|columns|rows|herdr]");
            }
            let f = crate::cli::fleet_now(&a.cfg, &a.flag, "Looking for sessions");
            let all: Vec<crate::herdr::Session> = f.sessions().into_iter().cloned().collect();
            let want = r
                .iter()
                .position(|x| x == "--layout")
                .map(|i| r.get(i + 1).cloned().unwrap_or_default());
            let mut picked = Vec::new();
            let mut skip = false;
            for q in &r[2..] {
                if std::mem::take(&mut skip) {
                    continue;
                }
                if q.starts_with("--") {
                    skip = q == "--layout";
                    continue;
                }
                match crate::herdr::pick(&all, q) {
                    Some(s) => picked.push(s.clone()),
                    None => return fail(&format!("no session '{q}'")),
                }
            }
            let mut p = from_sessions(&name, &picked);
            match want.as_deref().map(Layout::parse) {
                Some(None) => {
                    return fail("--layout takes tabs, grid, columns, rows or herdr");
                }
                // the tab they share, exactly as it is
                Some(Some(Layout::Herdr)) => match herdr_tree(&a.cfg, &picked) {
                    Ok(t) => {
                        p.layout = Layout::Herdr;
                        p.tree = Some(t);
                    }
                    Err(e) => return fail(&format!("couldn't read their tab's layout: {e}")),
                },
                Some(Some(l)) => p.layout = l,
                // all in one tab: keep its shape when herdr can say what it is
                None if one_tab(&picked) => {
                    if let Ok(t) = herdr_tree(&a.cfg, &picked) {
                        hint(&format!(
                            "saved their tab's exact layout {} (--layout tabs saves separate tabs)",
                            layouts::describe(&t)
                        ));
                        p.layout = Layout::Herdr;
                        p.tree = Some(t);
                    }
                }
                None => {}
            }
            save_or_flat(&mut a.cfg, &p)
        }
        "dup" | "copy" => {
            let (Some(p), Some(new)) = (get(&a.cfg, &name), r.get(2)) else {
                return fail("usage: aleph preset dup NAME NEW");
            };
            if !crate::names::valid(new) {
                return fail("preset names are lowercase letters, digits, - and _");
            }
            let mut q = p.clone();
            q.name = new.clone();
            save_or_flat(&mut a.cfg, &q)
        }
        "rm" | "remove" | "delete" => {
            if get(&a.cfg, &name).is_none() {
                return fail(&format!("no preset '{name}'"));
            }
            let r = if a.cfg.is_toml() {
                remove(&mut a.cfg, &name)
            } else {
                a.cfg.unset_prefix(&format!("preset.{name}."))
            };
            match r {
                Ok(()) => {
                    ok(&format!("removed @{name}"));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&format!("couldn't save: {e}")),
            }
        }
        "export" => {
            let Some(p) = get(&a.cfg, &name) else {
                return fail(&format!("no preset '{name}'"));
            };
            let p = if a.has("--generic") { generic(&p) } else { p };
            print!("{}", to_toml(&p));
            ExitCode::SUCCESS
        }
        "import" => {
            if name.is_empty() {
                return fail("usage: aleph preset import FILE|URL [--dry-run] [--yes]");
            }
            let text = if name.starts_with("https://") || name.starts_with("http://") {
                let o = crate::proc::cap(
                    "curl",
                    &["-fsSL", "-m", "20", &name],
                    std::time::Duration::from_secs(25),
                );
                if !o.ok() {
                    return fail(&format!("couldn't fetch {name}: {}", o.reason()));
                }
                o.text()
            } else {
                match std::fs::read_to_string(crate::config::expand(&name)) {
                    Ok(t) => t,
                    Err(e) => return fail(&format!("{name}: {e}")),
                }
            };
            let doc: DocumentMut = match text.parse() {
                Ok(d) => d,
                Err(e) => return fail(&format!("{name} isn't TOML: {e}")),
            };
            let (v, bad) = from_doc(&doc, &name);
            for b in &bad {
                err(b);
            }
            if v.is_empty() {
                return fail("no presets in it");
            }
            // everything it would start, before anything is saved
            for p in &v {
                head(&format!(
                    "@{} — {} pane{}",
                    p.name,
                    p.panes.len(),
                    if p.panes.len() == 1 { "" } else { "s" }
                ));
                print_plan(
                    p,
                    &plan(
                        p,
                        &fleet_tags(&a.cfg),
                        a.cfg.get("ssh_host").as_deref(),
                        &[],
                    ),
                );
                for x in validate(p, &fleet_tags(&a.cfg)) {
                    warn(&x);
                }
            }
            if a.has("--dry-run") {
                return ExitCode::SUCCESS;
            }
            hint("nothing starts now: importing only saves them (aleph preset launch NAME starts one)");
            if !a.has("--yes")
                && !yes(&format!(
                    "Save {} preset{}?",
                    v.len(),
                    if v.len() == 1 { "" } else { "s" }
                ))
            {
                return ExitCode::from(1);
            }
            for p in &v {
                if let Err(e) = save(&mut a.cfg, p) {
                    return fail(&format!("couldn't save @{}: {e}", p.name));
                }
                ok(&format!("saved @{}", p.name));
            }
            ExitCode::SUCCESS
        }
        "validate" | "check" => {
            let (v, bad) = all_with_problems(&a.cfg);
            let mut n = bad.len();
            for b in &bad {
                err(b);
            }
            for p in v.iter().filter(|p| name.is_empty() || p.name == name) {
                let probs = validate(p, &fleet_tags(&a.cfg));
                n += probs.len();
                if probs.is_empty() {
                    ok(&format!("@{}", p.name));
                }
                for x in probs {
                    warn(&format!("@{}: {x}", p.name));
                }
            }
            if n == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        "edit" => crate::configcmd::cmd(crate::cli::Args {
            cfg: a.cfg,
            flag: a.flag,
            rest: &["edit".to_string()],
        }),
        _ => fail(&format!("usage: {USAGE}")),
    }
}

/// Save to aleph.toml; an old key=value file gets the one-session form.
fn save_or_flat(cfg: &mut Config, p: &Preset) -> std::process::ExitCode {
    use crate::out::*;
    let r = if cfg.is_toml() {
        save(cfg, p)
    } else if p.panes.len() == 1 {
        let x = &p.panes[0];
        let _ = cfg.unset_prefix(&format!("preset.{}.", p.name));
        let mut res = Ok(());
        for (k, v) in [
            ("host", x.host.clone()),
            ("dir", x.dir.clone()),
            ("agent", x.agent.clone()),
            ("prompt", x.prompt.clone()),
            (
                "args",
                (!x.args.is_empty()).then(|| {
                    x.args
                        .iter()
                        .map(|a| crate::proc::sh_quote(a))
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
            ),
            (
                "worktree",
                (x.worktree != Worktree::None).then(|| "1".to_string()),
            ),
        ] {
            if let Some(v) = v {
                if let Err(e) = cfg.set(&format!("preset.{}.{k}", p.name), &v) {
                    res = Err(e);
                }
            }
        }
        res
    } else {
        Err(std::io::Error::other(
            "a preset with several panes needs aleph.toml",
        ))
    };
    match r {
        Ok(()) => {
            ok(&format!(
                "saved @{} — aleph preset launch {} (or aleph new @{} for its first session)",
                p.name, p.name, p.name
            ));
            std::process::ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("couldn't save: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ML: &str = r#"
[presets.ml]
description = "Train on b31, review locally"
layout = "grid"
focus = "train"
vars = { repo = "~/work/hunt" }
[[presets.ml.panes]]
name = "train"
host = "b31"
agent = "claude"
dir = "${repo}"
args = ["--model", "opus"]
prompt = "Run the sweep"
[[presets.ml.panes]]
name = "review"
host = "b31"
agent = "codex"
dir = "${repo}"
[[presets.ml.panes]]
name = "eval"
host = "tag:gpu"

[presets.old]
host = "gpu2"
dir = "/srv/train"
args = "--model 'opus 4' -v"
worktree = "1"
"#;

    fn doc() -> DocumentMut {
        ML.parse().unwrap()
    }

    #[test]
    fn parses_both_forms() {
        let (v, bad) = from_doc(&doc(), "test");
        assert!(bad.is_empty(), "{bad:?}");
        let ml = v.iter().find(|p| p.name == "ml").unwrap();
        assert_eq!(ml.panes.len(), 3);
        assert_eq!(ml.layout, Layout::Grid);
        assert_eq!(ml.panes[0].args, vec!["--model", "opus"]);
        let old = v.iter().find(|p| p.name == "old").unwrap();
        assert_eq!(old.panes.len(), 1);
        assert_eq!(old.panes[0].args, vec!["--model", "opus 4", "-v"]);
        assert_eq!(old.panes[0].worktree, Worktree::Auto);
    }

    #[test]
    fn plans_where_things_start() {
        let (v, _) = from_doc(&doc(), "test");
        let ml = v.iter().find(|p| p.name == "ml").unwrap();
        let hosts = vec![
            ("b31".to_string(), vec!["gpu".to_string()]),
            ("gpu2".to_string(), vec![]),
        ];
        let p = plan(ml, &hosts, Some("b31"), &[]);
        assert_eq!(p.launches.len(), 3);
        // vars expand; the second b31 pane splits into the first one's tab
        assert_eq!(p.launches[0].dir.as_deref(), Some("~/work/hunt"));
        assert_eq!(p.launches[1].split_of, Some(0));
        // tag:gpu → b31
        assert_eq!(p.launches[2].host, "b31");
        // an override of a var
        let p = plan(ml, &hosts, None, &[("repo".into(), "/x".into())]);
        assert_eq!(p.launches[0].dir.as_deref(), Some("/x"));
        // a machine that's gone: asked about (the default), skipped, or the default stands in
        let mut ml2 = ml.clone();
        ml2.panes[0].host = Some("gone".into());
        assert_eq!(plan(&ml2, &hosts, Some("b31"), &[]).ask.len(), 1);
        ml2.on_missing_host = Missing::Skip;
        assert_eq!(plan(&ml2, &hosts, Some("b31"), &[]).skipped.len(), 1);
        ml2.on_missing_host = Missing::Fallback;
        assert_eq!(
            plan(&ml2, &hosts, Some("gpu2"), &[]).launches[0].host,
            "gpu2"
        );
    }

    #[test]
    fn validates_and_round_trips() {
        let (v, _) = from_doc(&doc(), "test");
        let ml = v.iter().find(|p| p.name == "ml").unwrap();
        let hosts = vec![("b31".to_string(), vec!["gpu".to_string()])];
        assert!(
            validate(ml, &hosts).is_empty(),
            "{:?}",
            validate(ml, &hosts)
        );
        let mut bad = ml.clone();
        bad.panes[0].agent = Some("nope".into());
        bad.panes[1].dir = Some("${missing}".into());
        let probs = validate(&bad, &hosts);
        assert!(
            probs.iter().any(|p| p.contains("unknown agent")),
            "{probs:?}"
        );
        assert!(probs.iter().any(|p| p.contains("${missing}")), "{probs:?}");
        // TOML out and back in is the same preset
        let text = to_toml(ml);
        let (back, _) = from_doc(&text.parse().unwrap(), "test");
        let mut b = back[0].clone();
        b.source = ml.source.clone();
        assert_eq!(&b, ml);
        // generic: machine names become vars
        let g = generic(ml);
        assert_eq!(g.panes[0].host.as_deref(), Some("${host1}"));
        assert!(g.vars.contains(&("host1".into(), "b31".into())));
    }

    #[test]
    fn herdr_layouts_plan_one_exact_tab() {
        let text = r#"
[presets.pair]
tree = { right = 0.7, first = "review", second = { down = 0.4, first = "train", second = 3 } }
[[presets.pair.panes]]
name = "train"
host = "b31"
[[presets.pair.panes]]
name = "review"
host = "b31"
[[presets.pair.panes]]
name = "eval"
host = "b31"
[[presets.pair.panes]]
name = "far"
host = "gpu2"
"#;
        let (v, bad) = from_doc(&text.parse().unwrap(), "t");
        assert!(bad.is_empty(), "{bad:?}");
        let p = &v[0];
        assert_eq!(p.layout, Layout::Herdr);
        let hosts = vec![("b31".to_string(), vec![]), ("gpu2".to_string(), vec![])];
        assert!(validate(p, &hosts).is_empty(), "{:?}", validate(p, &hosts));
        let pl = plan(p, &hosts, None, &[]);
        // review (the tree's first pane) opens the tab; train and eval join
        // it; far, on another machine, gets a tab of its own
        assert_eq!(pl.tabs.len(), 1);
        let (root, t) = &pl.tabs[0];
        assert_eq!(*root, 1);
        assert_eq!(
            t.leaves().into_iter().copied().collect::<Vec<_>>(),
            vec![1, 0, 2]
        );
        assert!(matches!(t, Tree::Split { ratio, .. } if (ratio - 0.7).abs() < 1e-6));
        assert_eq!(pl.launches[0].split_of, Some(1));
        assert_eq!(pl.launches[3].split_of, None);
        assert_eq!(pl.order(), vec![1, 0, 2, 3]);
        // TOML out and back keeps the tree
        let back = from_doc(&to_toml(p).parse().unwrap(), "t").0;
        assert_eq!(back[0].tree, p.tree);
        assert_eq!(back[0].layout, Layout::Herdr);
        // a typo, a pane twice, panes on two machines: said
        let mut q = p.clone();
        q.tree = Some(Tree::split(
            layouts::Dir::Right,
            0.5,
            Tree::Pane("trian".into()),
            Tree::split(
                layouts::Dir::Down,
                0.5,
                Tree::Pane("far".into()),
                Tree::Pane("4".into()),
            ),
        ));
        let probs = validate(&q, &hosts);
        assert!(
            probs.iter().any(|x| x.contains("no pane \"trian\"")),
            "{probs:?}"
        );
        assert!(probs.iter().any(|x| x.contains("twice")), "{probs:?}");
        q.tree = Some(Tree::split(
            layouts::Dir::Right,
            0.5,
            Tree::Pane("review".into()),
            Tree::Pane("far".into()),
        ));
        assert!(validate(&q, &hosts)
            .iter()
            .any(|x| x.contains("lives on one machine")));
        // …and planned anyway: far, on the other machine, leaves the tree
        let pl = plan(&q, &hosts, None, &[]);
        assert!(pl.tabs.is_empty());
        // columns: each machine's panes in a row of their own
        let mut c = p.clone();
        c.layout = Layout::Columns;
        c.tree = None;
        let pl = plan(&c, &hosts, None, &[]);
        assert_eq!(pl.tabs.len(), 1);
        assert_eq!(pl.tabs[0].1, layouts::line(3, layouts::Dir::Right).unwrap());
        assert_eq!(pl.order(), vec![0, 1, 2, 3]);
        // herdr without a tree: one tab each, and said
        c.layout = Layout::Herdr;
        assert!(plan(&c, &hosts, None, &[]).tabs.is_empty());
        assert!(validate(&c, &hosts)
            .iter()
            .any(|x| x.contains("needs a tree")));
    }

    #[test]
    fn words() {
        assert_eq!(shell_words("a 'b c' \"d\" ''"), vec!["a", "b c", "d", ""]);
    }
}
