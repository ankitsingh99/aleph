//! Machines, managed from Fleet: every ssh alias, whether it's in the fleet,
//! its colour, herdr and login state; include/exclude, recolour, make
//! default, test, set up, and the add / edit form with its connection check.

use super::{forms, Bg, Dash, Handoff, Mode, Pending};
use crate::sshcfg::{self, Fail, NewHost, Probe, Resolved};
use crate::{herdr, theme};
use crossterm::event::{KeyCode, KeyEvent};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) enum Msg {
    Resolved(String, Option<Resolved>),
    Profiles(Vec<(String, String)>, String),
    Test(String, Probe),
    /// the add form's connection test (generation, result)
    Precheck(u64, sshcfg::Precheck),
    /// a machine that just joined, looked at: herdr there? any agent?
    Looked(String, Box<crate::probe::HostProbe>),
    /// its setup tab opened (the terminal's name), or why not
    TabOpened(String, Result<String, String>),
}

#[cfg(test)]
impl HostRow {
    pub fn for_test(alias: &str) -> HostRow {
        HostRow {
            alias: alias.into(),
            in_config: true,
            res: None,
            test: None,
            testing: None,
        }
    }
}

pub(crate) struct HostRow {
    pub alias: String,
    in_config: bool,
    pub res: Option<Resolved>,
    pub test: Option<Probe>,
    pub testing: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct State {
    pub rows: Vec<HostRow>,
    loaded: bool,
    sel: usize,
    profiles: Option<(Vec<(String, String)>, String)>,
    /// machines that join the fleet once a test passes (after key setup)
    pub join_after_test: std::collections::HashSet<String>,
    /// machines whose setup runs in another tab (since when)
    pub setting_up: std::collections::HashMap<String, Instant>,
}

pub(crate) fn reload(d: &mut Dash) {
    let mut aliases = sshcfg::aliases();
    for h in d.cfg.hosts() {
        if !aliases.contains(&h) {
            aliases.insert(0, h);
        }
    }
    if crate::proc::which(&herdr::bin()).is_some() && !aliases.iter().any(|a| a == sshcfg::LOCAL) {
        aliases.insert(0, sshcfg::LOCAL.to_string());
    }
    let mut known = sshcfg::aliases();
    known.push(sshcfg::LOCAL.to_string());
    let old = std::mem::take(&mut d.hv.rows);
    d.hv.rows = aliases
        .into_iter()
        .map(|a| {
            let prev = old.iter().find(|o| o.alias == a);
            HostRow {
                in_config: known.contains(&a),
                res: prev.and_then(|p| p.res.clone()),
                test: prev.and_then(|p| p.test.clone()),
                testing: None,
                alias: a,
            }
        })
        .collect();
    // fleet machines first, in fleet order
    let fleet = d.cfg.fleet();
    d.hv.rows.sort_by_key(|r| {
        fleet
            .iter()
            .position(|h| *h == r.alias)
            .unwrap_or(usize::MAX)
    });
    d.hv.loaded = true;
    d.hv.profiles = None;
    let list: Vec<String> = d.hv.rows.iter().map(|h| h.alias.clone()).collect();
    let tx = d.tx.clone();
    thread::spawn(move || {
        let o = crate::proc::cap(&herdr::bin(), &["machine", "list"], Duration::from_secs(10));
        let _ = tx.send(Bg::Hosts(Msg::Profiles(herdr::profiles(), o.text())));
        for a in list {
            let r = sshcfg::resolve(&a);
            if tx.send(Bg::Hosts(Msg::Resolved(a, r))).is_err() {
                break;
            }
        }
    });
}

pub(crate) fn test(d: &mut Dash, alias: &str) {
    let Some(row) = d.hv.rows.iter_mut().find(|r| r.alias == alias) else {
        return;
    };
    row.testing = Some(Instant::now());
    let (a, tx) = (alias.to_string(), d.tx.clone());
    thread::spawn(move || {
        let p = sshcfg::probe(&a);
        let _ = tx.send(Bg::Hosts(Msg::Test(a, p)));
    });
}

pub(crate) fn on_msg(d: &mut Dash, m: Msg) {
    match m {
        Msg::Resolved(a, r) => {
            if let Some(h) = d.hv.rows.iter_mut().find(|h| h.alias == a) {
                h.res = r;
            }
        }
        Msg::Profiles(p, t) => d.hv.profiles = Some((p, t)),
        Msg::Precheck(gen, r) => on_precheck(d, gen, r),
        Msg::Looked(a, p) => looked(d, &a, &p),
        Msg::TabOpened(a, r) => match r {
            Ok(t) => d.note(format!(
                "{a}'s setup is open in a new {t} tab — it asks before installing; Aleph looks at {a} again when it's done"
            )),
            Err(e) => {
                d.hv.setting_up.remove(&a);
                d.mode = Mode::Confirm {
                    title: "Set it up here?".into(),
                    text: format!("Couldn't open a new tab ({e}). Set {a} up on this screen instead? herdr, then its agents — it asks before installing anything."),
                    on_yes: Pending::Handoff(Box::new(Handoff::HostSetup(a))),
                };
            }
        },
        Msg::Test(a, p) => {
            let (auth, ok) = (p.fail == Some(Fail::Auth), p.ok());
            if let Some(h) = d.hv.rows.iter_mut().find(|h| h.alias == a) {
                h.testing = None;
                h.test = Some(p);
            }
            if ok && d.hv.join_after_test.remove(&a) && !d.cfg.hosts().contains(&a) {
                toggle_fleet(d, &a);
            }
            if auth {
                d.error(format!(
                    "{a}: no key accepted — press l to set up key login"
                ));
            } else if ok {
                d.note(format!("{a}: key login works"));
            }
        }
    }
}

fn in_herdr(d: &Dash, alias: &str) -> Option<bool> {
    if sshcfg::is_local(alias) {
        return Some(true);
    }
    let (p, txt) = d.hv.profiles.as_ref()?;
    Some(
        p.iter().any(|(t, s)| t == alias || s == alias)
            || txt
                .split(|c: char| c.is_whitespace() || c == ',' || c == '"')
                .any(|w| w == alias),
    )
}

/// Take `alias` out of the fleet (its sessions keep running, its ssh entry
/// stays). The default machine hands that role to another one first.
pub(crate) fn leave(d: &mut Dash, alias: &str) {
    if sshcfg::is_local(alias) {
        toggle_fleet(d, alias);
        return;
    }
    if d.cfg.get("ssh_host").as_deref() == Some(alias) {
        let next = d.cfg.hosts().into_iter().find(|h| h != alias);
        match next {
            Some(n) => {
                if let Err(e) = d.cfg.set("ssh_host", &n) {
                    d.error(format!("couldn't save: {e}"));
                    return;
                }
                d.note(format!("{n} is the default machine now"));
            }
            None => {
                d.error("it's the only machine — add another before removing it");
                return;
            }
        }
    }
    toggle_fleet(d, alias);
}

pub(crate) fn toggle_fleet(d: &mut Dash, alias: &str) {
    if sshcfg::is_local(alias) {
        let on = d.cfg.fleet().iter().any(|h| h == alias);
        let _ = d.cfg.set("local", if on { "false" } else { "true" });
        if on {
            d.fleet.remove_host(alias);
            d.note("this computer's herdr is hidden from the fleet");
        } else {
            theme::assign_machine_colors(&mut d.cfg);
            let cfg = d.cfg.clone();
            d.fleet.add_host(&cfg, alias);
            d.fleet.refresh(Some(alias));
            d.note("this computer's herdr joins the fleet as `local`");
        }
        return;
    }
    let mut v = d.cfg.hosts();
    let def = d.cfg.get("ssh_host");
    if v.iter().any(|h| h == alias) {
        if def.as_deref() == Some(alias) {
            d.error("that's the default machine — make another one default first (enter)");
            return;
        }
        v.retain(|h| h != alias && Some(h) != def.as_ref());
        if let Err(e) = d.cfg.set_hosts(&v) {
            d.error(format!("couldn't save: {e}"));
            return;
        }
        d.fleet.remove_host(alias);
        d.note(format!(
            "{alias} left the fleet (its sessions keep running)"
        ));
    } else {
        v.push(alias.to_string());
        if def.is_none() {
            let _ = d.cfg.set("ssh_host", alias);
        }
        if let Err(e) = d.cfg.set_hosts(&v) {
            d.error(format!("couldn't save: {e}"));
            return;
        }
        theme::assign_machine_colors(&mut d.cfg);
        let cfg = d.cfg.clone();
        d.fleet.add_host(&cfg, alias);
        d.fleet.refresh(Some(alias));
        d.note(format!("{alias} joins the fleet — looking at it"));
        look(d, alias);
    }
}

/// A machine just joined: look at it (off the UI thread) for herdr and its
/// agents; [`looked`] decides what comes next.
pub(crate) fn look(d: &mut Dash, alias: &str) {
    let (a, tx) = (alias.to_string(), d.tx.clone());
    thread::spawn(move || {
        let p = crate::probe::run(&a);
        let _ = tx.send(Bg::Hosts(Msg::Looked(a, Box::new(p))));
    });
}

/// What the look found: no herdr or no agent there → its setup opens in a
/// new tab (Aleph stays here); herdr there but the machine not saved in
/// herdr → the quick save, on this screen.
fn looked(d: &mut Dash, alias: &str, p: &crate::probe::HostProbe) {
    // its agents on the card (the look left its answer on disk)
    if p.ok() {
        d.agents.reload(alias);
    }
    let n = crate::hostsetup::needs(p);
    if p.ok() && n.any() {
        open_setup(d, alias, Some(n.text()));
    } else if in_herdr(d, alias) == Some(false) {
        // herdr there but not saved — or it didn't answer: the checklist on
        // this screen explains (and offers key login)
        d.mode = Mode::Confirm {
            title: if p.ok() {
                "Save it in herdr?".into()
            } else {
                "Set it up for herdr?".into()
            },
            text: if p.ok() {
                format!("'{alias}' has herdr, but isn't a saved herdr machine yet. Check it and save it now?")
            } else {
                format!(
                    "'{alias}' didn't answer ({}). Check it now? Key login, herdr and saving it in herdr — you're asked before anything is installed.",
                    p.error.clone().unwrap_or_default()
                )
            },
            on_yes: Pending::Handoff(Box::new(Handoff::MachineAdd(alias.into()))),
        };
    }
}

/// `aleph host setup <alias>` in a new tab of this terminal: herdr, then a
/// checklist of agents. The machine's card says so until the setup reports
/// back (`refresh.rs`); a terminal that can't open tabs gets it on this screen.
pub(crate) fn open_setup(d: &mut Dash, alias: &str, why: Option<String>) {
    if sshcfg::is_local(alias) {
        d.mode = Mode::Confirm {
            title: "Set up this computer?".into(),
            text: "herdr's agents on this computer: a checklist, installed after you say so."
                .into(),
            on_yes: Pending::Handoff(Box::new(Handoff::HostSetup(alias.into()))),
        };
        return;
    }
    d.hv.setting_up.insert(alias.to_string(), Instant::now());
    d.toast(
        super::Level::Info,
        match why {
            Some(w) => format!("{alias} has {w} — opening its setup in a new tab…"),
            None => format!("opening {alias}'s setup in a new tab…"),
        },
    );
    let (cfg, a, tx) = (d.cfg.clone(), alias.to_string(), d.tx.clone());
    thread::spawn(move || {
        let r = crate::hostsetup::open_tab(&cfg, &a).map(|t| t.title().to_string());
        let _ = tx.send(Bg::Hosts(Msg::TabOpened(a, r)));
    });
}

pub(crate) fn cycle_color(d: &mut Dash, alias: &str) {
    let cur = d.cfg.host_get(alias, "color").unwrap_or_default();
    let names: Vec<&str> = theme::MACHINES.iter().map(|m| m.0).collect();
    let i = names
        .iter()
        .position(|n| *n == cur)
        .map(|i| (i + 1) % names.len())
        .unwrap_or(0);
    let _ = d.cfg.set(&format!("host.{alias}.color"), names[i]);
    let col = theme::machine(&d.cfg, alias);
    if let Some(h) = d.fleet.host_mut(alias) {
        h.color = col;
    }
    d.note(format!("{alias} is now {}", names[i]));
}

pub(crate) fn set_default(d: &mut Dash, alias: &str) {
    let machine =
        d.hv.profiles
            .as_ref()
            .and_then(|(p, _)| p.iter().find(|(t, _)| t == alias).map(|(_, s)| s.clone()))
            .unwrap_or_else(|| alias.to_string());
    if let Err(e) = d
        .cfg
        .set("ssh_host", alias)
        .and_then(|_| d.cfg.set("machine", &machine))
    {
        d.error(format!("couldn't save config: {e}"));
        return;
    }
    let mut v = d.cfg.hosts();
    if !v.iter().any(|h| h == alias) {
        v.push(alias.into());
        let _ = d.cfg.set_hosts(&v);
    }
    theme::assign_machine_colors(&mut d.cfg);
    let cfg = d.cfg.clone();
    d.fleet.add_host(&cfg, alias);
    d.note(format!("default machine → {alias}"));
}

/// The add-a-machine form's keys, by stage.
pub(crate) fn form_key(d: &mut Dash, mut hf: forms::HostForm, k: KeyEvent) -> Option<Handoff> {
    use forms::HostStage as S;
    match hf.stage.clone() {
        S::Edit => match hf.key(k) {
            forms::Res::Keep => {
                d.mode = Mode::HostForm(hf);
                None
            }
            forms::Res::Cancel => None,
            forms::Res::Submit => {
                start_check(d, hf);
                None
            }
        },
        S::Testing(_) => {
            if k.code != KeyCode::Esc {
                d.mode = Mode::HostForm(hf);
            }
            None
        }
        S::NewKey { lines, .. } => match k.code {
            KeyCode::Char('y') => {
                if let Err(e) = sshcfg::trust(&lines) {
                    hf.stage = S::Failed(format!("couldn't save the host key: {e}"));
                    d.mode = Mode::HostForm(hf);
                } else {
                    start_check(d, hf);
                }
                None
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                hf.stage = S::Edit;
                hf.err = Some("not trusted — nothing was saved".into());
                d.mode = Mode::HostForm(hf);
                None
            }
            _ => {
                d.mode = Mode::HostForm(hf);
                None
            }
        },
        S::NeedKey => match k.code {
            KeyCode::Char('k') => {
                let alias = save(d, &mut hf, false)?;
                // after ssh-copy-id: test again, and join the fleet if it works
                d.hv.join_after_test.insert(alias.clone());
                Some(Handoff::KeySetup(alias))
            }
            KeyCode::Char('s') => {
                save(d, &mut hf, true);
                None
            }
            KeyCode::Esc => {
                hf.stage = S::Edit;
                d.mode = Mode::HostForm(hf);
                None
            }
            _ => {
                d.mode = Mode::HostForm(hf);
                None
            }
        },
        S::Failed(_) => match k.code {
            KeyCode::Enter | KeyCode::Char('r') => {
                start_check(d, hf);
                None
            }
            KeyCode::Char('s') => {
                save(d, &mut hf, true);
                None
            }
            KeyCode::Esc => None,
            _ => {
                hf.stage = S::Edit;
                if k.code != KeyCode::Char('e') {
                    hf.key(k);
                }
                d.mode = Mode::HostForm(hf);
                None
            }
        },
    }
}

fn parse(hf: &forms::HostForm) -> Result<NewHost, String> {
    let alias = hf.f[0].text.trim().to_string();
    let (user, hostname, port) =
        sshcfg::valid_alias(&alias).and_then(|_| sshcfg::parse_target(&hf.f[1].text))?;
    if hf.editing.is_none() && d_has_alias(&alias) {
        return Err(format!(
            "{alias} is already an ssh alias — pick another name"
        ));
    }
    let id = hf.f[2].text.trim();
    let jump = hf.f[3].text.trim();
    if jump.contains(char::is_whitespace) {
        return Err("jump via takes one host (or host1,host2)".into());
    }
    Ok(NewHost {
        alias,
        user,
        hostname,
        port,
        identity: (!id.is_empty()).then(|| id.to_string()),
        jump: (!jump.is_empty()).then(|| jump.to_string()),
    })
}

fn d_has_alias(a: &str) -> bool {
    sshcfg::aliases().iter().any(|x| x == a)
}

/// Try the machine in the background; the form shows a spinner meanwhile.
fn start_check(d: &mut Dash, mut hf: forms::HostForm) {
    let h = match parse(&hf) {
        Ok(h) => h,
        Err(e) => {
            hf.stage = forms::HostStage::Edit;
            hf.err = Some(e);
            d.mode = Mode::HostForm(hf);
            return;
        }
    };
    hf.gen += 1;
    hf.err = None;
    hf.stage = forms::HostStage::Testing(Instant::now());
    let (gen, tx) = (hf.gen, d.tx.clone());
    d.mode = Mode::HostForm(hf);
    thread::spawn(move || {
        let r = sshcfg::precheck(&h);
        let _ = tx.send(Bg::Hosts(Msg::Precheck(gen, r)));
    });
}

/// Write the Host block, (optionally) join the fleet, close the form.
fn save(d: &mut Dash, hf: &mut forms::HostForm, join: bool) -> Option<String> {
    let h = match parse(hf) {
        Ok(h) => h,
        Err(e) => {
            hf.stage = forms::HostStage::Edit;
            hf.err = Some(e);
            d.mode = Mode::HostForm(std::mem::replace(hf, forms::HostForm::new()));
            return None;
        }
    };
    let alias = h.alias.clone();
    let editing = hf.editing.is_some();
    let r = if editing {
        sshcfg::update_host(&h)
    } else {
        sshcfg::add_host(&h)
    };
    match r {
        Ok(_) => {
            reload(d);
            if editing {
                d.note(format!(
                    "{alias} updated (the old ~/.ssh/config is backed up)"
                ));
                d.fleet.refresh(Some(&alias));
                return Some(alias);
            }
            if let Some(i) = d.hv.rows.iter().position(|r| r.alias == alias) {
                d.hv.sel = i;
            }
            if join && !d.cfg.hosts().contains(&alias) {
                toggle_fleet(d, &alias);
            }
            Some(alias)
        }
        Err(e) => {
            hf.stage = forms::HostStage::Edit;
            hf.err = Some(e);
            d.mode = Mode::HostForm(std::mem::replace(hf, forms::HostForm::new()));
            None
        }
    }
}

/// A connection test of the add form finished.
fn on_precheck(d: &mut Dash, gen: u64, r: sshcfg::Precheck) {
    use forms::HostStage as S;
    let Mode::HostForm(hf) = &mut d.mode else {
        return;
    };
    if hf.gen != gen || !matches!(hf.stage, S::Testing(_)) {
        return;
    }
    match r {
        sshcfg::Precheck::Ok(took) => {
            let mut hf = match std::mem::replace(&mut d.mode, Mode::Normal) {
                Mode::HostForm(hf) => hf,
                _ => return,
            };
            if let Some(a) = save(d, &mut hf, true) {
                d.note(format!(
                    "{a} added — key login works ({} ms) and it's in the fleet",
                    took.as_millis()
                ));
            }
        }
        sshcfg::Precheck::NewKey {
            fingerprints,
            lines,
        } => {
            hf.stage = S::NewKey {
                fingerprints,
                lines,
            }
        }
        sshcfg::Precheck::Failed(Fail::Auth, _) => hf.stage = S::NeedKey,
        sshcfg::Precheck::Failed(Fail::HostKeyChanged, _) => {
            hf.stage = S::Failed(
                "its host key CHANGED since you last connected — a reinstall, or someone in \
                 between. Nothing saved; check with its owner (ssh-keygen -R removes the old key)."
                    .into(),
            )
        }
        sshcfg::Precheck::Failed(f, detail) => {
            let why = match f {
                Fail::Dns => "the name doesn't resolve".to_string(),
                Fail::Unreachable => "no route to it (offline, asleep, VPN?)".to_string(),
                Fail::Refused => "nothing listens for ssh there (wrong port?)".to_string(),
                Fail::Timeout => "it didn't answer in time".to_string(),
                _ => detail.lines().next().unwrap_or("").to_string(),
            };
            hf.stage = S::Failed(why);
        }
    }
}
