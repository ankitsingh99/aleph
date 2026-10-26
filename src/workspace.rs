//! Saved workspaces: a named place in the dashboard. That's a tab (and
//! Monitor's sub-view), its panels' layout, Fleet's filter and the machine
//! in view, all applied at once (`W` in the dashboard, the palette,
//! `aleph ui NAME`, `aleph://open?workspace=NAME`).
//!
//! They're settings, `workspace.<name>.<field>` (`[workspaces.<name>]` in
//! aleph.toml), so a profile or a backup carries them too. Fields: `title`,
//! `view` (fleet, monitor, inbox, settings), `sub` (Monitor's: overview, cpu,
//! memory, gpu, processes, network, services, usage, fleet), `layout` (the
//! view's panels, as `layout.<view>` writes them), `filter` (Fleet's),
//! `machine`. gpu-watch, standup and sessions are starters: shown until you
//! save one of your own by that name.

use crate::config::Config;

pub const FIELDS: &[&str] = &["title", "view", "sub", "layout", "filter", "machine"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspace {
    /// its key: lowercase, digits, - and _
    pub name: String,
    pub title: String,
    pub view: String,
    pub sub: String,
    pub layout: String,
    pub filter: String,
    pub machine: String,
    /// a starter nobody saved yet
    pub starter: bool,
}

/// The starters: (name, title, view, sub, layout).
const STARTERS: &[(&str, &str, &str, &str, &str)] = &[
    ("gpu-watch", "GPU watch", "monitor", "gpu", ""),
    ("standup", "Standup", "inbox", "", ""),
    ("sessions", "Sessions only", "fleet", "", "hidden"),
];

/// `GPU watch` → `gpu-watch`.
pub fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if (c == ' ' || c == '-' || c == '_' || c == '.')
            && !s.ends_with('-')
            && !s.is_empty()
        {
            s.push('-');
        }
    }
    s.trim_end_matches('-').chars().take(32).collect()
}

impl Workspace {
    /// What it shows, in a few words: `Monitor › GPU · side maximised`.
    pub fn what(&self) -> String {
        let mut v = vec![match self.view.as_str() {
            "monitor" if !self.sub.is_empty() => format!("Monitor › {}", sub_title(&self.sub)),
            "" => "Fleet".to_string(),
            x => {
                let mut c = x.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            }
        }];
        if !self.layout.is_empty() {
            v.push(self.layout.clone());
        }
        if !self.filter.is_empty() {
            v.push(format!("“{}”", self.filter));
        }
        if !self.machine.is_empty() {
            v.push(self.machine.clone());
        }
        v.join(" · ")
    }
}

/// `gpu` → `GPU`, `processes` → `Processes` (Monitor's own names).
fn sub_title(s: &str) -> String {
    match s {
        "cpu" | "gpu" => s.to_uppercase(),
        "procs" => "Processes".into(),
        "net" => "Network".into(),
        _ => {
            let mut c = s.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    }
}

/// Every workspace: the saved ones (by name), then the starters not saved.
pub fn all(cfg: &Config) -> Vec<Workspace> {
    let mut names: Vec<String> = Vec::new();
    for (k, _) in cfg.entries() {
        if let Some(rest) = k.strip_prefix("workspace.") {
            if let Some((n, f)) = rest.rsplit_once('.') {
                if FIELDS.contains(&f) && !names.iter().any(|x| x == n) {
                    names.push(n.to_string());
                }
            }
        }
    }
    names.sort();
    let mut v: Vec<Workspace> = names.iter().filter_map(|n| get(cfg, n)).collect();
    for (n, t, view, sub, layout) in STARTERS {
        if !v.iter().any(|w| w.name == *n) {
            v.push(Workspace {
                name: n.to_string(),
                title: t.to_string(),
                view: view.to_string(),
                sub: sub.to_string(),
                layout: layout.to_string(),
                starter: true,
                ..Default::default()
            });
        }
    }
    v
}

/// One saved workspace (or a starter by its name).
pub fn get(cfg: &Config, name: &str) -> Option<Workspace> {
    let f = |field: &str| {
        cfg.get(&format!("workspace.{name}.{field}"))
            .unwrap_or_default()
    };
    let w = Workspace {
        name: name.to_string(),
        title: f("title"),
        view: f("view"),
        sub: f("sub"),
        layout: f("layout"),
        filter: f("filter"),
        machine: f("machine"),
        starter: false,
    };
    if w.view.is_empty() && w.title.is_empty() && w.layout.is_empty() {
        return STARTERS
            .iter()
            .find(|s| s.0 == name)
            .map(|(n, t, view, sub, layout)| Workspace {
                name: n.to_string(),
                title: t.to_string(),
                view: view.to_string(),
                sub: sub.to_string(),
                layout: layout.to_string(),
                starter: true,
                ..Default::default()
            });
    }
    Some(Workspace {
        title: if w.title.is_empty() {
            name.to_string()
        } else {
            w.title.clone()
        },
        ..w
    })
}

/// Find one by name or title (any case).
pub fn find(cfg: &Config, q: &str) -> Option<Workspace> {
    let s = slug(q);
    all(cfg)
        .into_iter()
        .find(|w| w.name == q || w.name == s || w.title.eq_ignore_ascii_case(q))
}

/// Save `w` (as settings), replacing what was there under its name.
pub fn save(cfg: &mut Config, w: &Workspace) -> Result<(), String> {
    if w.name.is_empty() {
        return Err("a workspace needs a name".into());
    }
    remove(cfg, &w.name)?;
    for (f, v) in [
        ("title", &w.title),
        ("view", &w.view),
        ("sub", &w.sub),
        ("layout", &w.layout),
        ("filter", &w.filter),
        ("machine", &w.machine),
    ] {
        if !v.is_empty() {
            cfg.set(&format!("workspace.{}.{f}", w.name), v)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn remove(cfg: &mut Config, name: &str) -> Result<(), String> {
    for f in FIELDS {
        let k = format!("workspace.{name}.{f}");
        if cfg.get_raw(&k).is_some() {
            cfg.unset(&k).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspaces_are_settings_and_starters_fill_in() {
        let cfg = Config::in_memory(&[
            "workspace.night.title=Night shift",
            "workspace.night.view=monitor",
            "workspace.night.sub=gpu",
            "workspace.night.layout=max side",
        ]);
        let all = all(&cfg);
        assert_eq!(all[0].name, "night");
        assert_eq!(all[0].what(), "Monitor › GPU · max side");
        assert!(all.iter().any(|w| w.name == "gpu-watch" && w.starter));
        assert_eq!(find(&cfg, "Night shift").unwrap().name, "night");
        assert_eq!(find(&cfg, "GPU watch").unwrap().sub, "gpu");
        assert!(find(&cfg, "nope").is_none());
        assert_eq!(slug("  GPU  watch!"), "gpu-watch");
        assert_eq!(slug("Stand-up / daily"), "stand-up-daily");
    }
}
