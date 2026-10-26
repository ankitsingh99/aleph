//! Clone a session to another machine (or move it): the same agent, with the
//! same arguments when Aleph started it (its history), in the same folder
//! there when that folder exists, else the one you name. Nothing is copied
//! between machines; `--move` closes the original once the clone runs.
//!
//! `aleph clone <session> --to <machine> [--dir D] [--move] [--yes]`, and
//! `:clone <machine> [--move]` on the selected session in the dashboard.

use crate::config::{Config, Target};
use crate::herdr::{self, Herdr, Session};
use std::time::Duration;

/// What a clone of `s` starts with (pure — tested): its agent, the arguments
/// and folder of the launch that started it (newest first), and the folder.
pub fn recipe(s: &Session, history: &[crate::history::Entry]) -> (String, Vec<String>, String) {
    let e = history
        .iter()
        .filter(|e| e.host == s.host && e.name == s.name)
        .max_by_key(|e| e.t);
    let agent = if s.agent.is_empty() {
        e.map(|e| e.agent.clone()).unwrap_or_default()
    } else {
        s.agent.clone()
    };
    let args = e.map(|e| e.args.clone()).unwrap_or_default();
    let dir = if s.cwd.starts_with('/') {
        s.cwd.clone()
    } else {
        e.map(|e| e.dir.clone()).unwrap_or_default()
    };
    (agent, args, dir)
}

/// Is `dir` a folder on `host`?
pub fn has_dir(host: &str, dir: &str) -> bool {
    if !dir.starts_with('/') || dir.contains('\'') || dir.contains('\n') {
        return false;
    }
    let mut c = crate::sshcfg::shell(host, &format!("test -d '{dir}' && echo yes"));
    crate::proc::run(&mut c, Duration::from_secs(10), None)
        .map(|o| o.text().contains("yes"))
        .unwrap_or(false)
}

/// Start the clone on `to`; the new session's name.
pub fn clone(cfg: &Config, s: &Session, to: &str, dir: Option<&str>) -> Result<String, String> {
    let (agent, args, src_dir) = recipe(s, &crate::history::load(500));
    let dir = match dir {
        Some(d) => d.to_string(),
        None if has_dir(to, &src_dir) => src_dir.clone(),
        None => {
            return Err(format!(
                "{src_dir} isn't on {to}: name a folder there (--dir, or :clone {to} --dir /path)"
            ))
        }
    };
    let mut o = crate::cli::parse_new(&["--host".into(), to.to_string()], cfg)?.opts;
    o.agent = agent;
    o.args = args;
    o.dir = Some(crate::path::normalize(dir.as_bytes()));
    o.no_focus = true;
    crate::cli::fill_names(&mut o);
    let t: Target = cfg.target_for(to, herdr::machine_for);
    let v = Herdr::new(&t).new_session(&o, &mut |_, _| {})?;
    crate::launch::after_start(&t, &o, &v);
    v.first()
        .map(|x| x.name.clone())
        .ok_or_else(|| "herdr started nothing".to_string())
}

/// Close the original after a move.
pub fn close_original(cfg: &Config, s: &Session) -> Result<(), String> {
    let t = cfg.target_for(&s.host, herdr::machine_for);
    Herdr::new(&t).close_pane(&s.pane_id)?;
    crate::restore::mark_closed(&s.host, &s.name);
    Ok(())
}

/// `aleph clone <session> --to <machine> [--dir D] [--move] [--yes]`
pub fn cmd(a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::{err, fail, hint, ok, yes};
    use std::process::ExitCode;
    let val = |k: &str| {
        a.rest
            .iter()
            .position(|x| x == k)
            .and_then(|i| a.rest.get(i + 1))
            .cloned()
    };
    let q = a.rest.iter().find(|x| {
        !x.starts_with('-') && Some(*x) != val("--to").as_ref() && Some(*x) != val("--dir").as_ref()
    });
    let (Some(q), Some(to)) = (q, val("--to")) else {
        return fail("usage: aleph clone <session> --to <machine> [--dir D] [--move] [--yes]");
    };
    let (_, s) = match crate::cli::find(&a.cfg, &a.flag, q) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let mv = a.has("--move");
    let sp = crate::out::Spinner::start(format!(
        "Starting {} on {to}",
        crate::agents::label(&s.agent)
    ));
    let r = clone(&a.cfg, &s, &to, val("--dir").as_deref());
    sp.stop();
    match r {
        Ok(name) => {
            ok(&format!(
                "{name} runs on {to} ({} from {})",
                crate::agents::label(&s.agent),
                s.display()
            ));
            if mv {
                if !a.has("--yes") && !yes(&format!("Close {} on {} now?", s.display(), s.host)) {
                    hint("kept the original too");
                    return ExitCode::SUCCESS;
                }
                match close_original(&a.cfg, &s) {
                    Ok(()) => ok(&format!("closed {} on {}", s.display(), s.host)),
                    Err(e) => {
                        err(&format!("the original stays: {e}"));
                        return ExitCode::from(1);
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            err(&e);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Entry;

    #[test]
    fn recipes() {
        let s = Session {
            host: "b31".into(),
            name: "ignite".into(),
            agent: "claude".into(),
            cwd: "/home/u/px".into(),
            ..Default::default()
        };
        let h = vec![
            Entry {
                t: 1,
                host: "b31".into(),
                name: "ignite".into(),
                agent: "claude".into(),
                args: vec!["--model".into(), "haiku".into()],
                dir: "/home/u/px".into(),
                ..Default::default()
            },
            Entry {
                t: 2,
                host: "b31".into(),
                name: "ignite".into(),
                agent: "claude".into(),
                args: vec!["--model".into(), "opus".into()],
                dir: "/home/u/px".into(),
                ..Default::default()
            },
            Entry {
                t: 3,
                host: "gpu2".into(),
                name: "ignite".into(),
                args: vec!["x".into()],
                ..Default::default()
            },
        ];
        let (a, args, d) = recipe(&s, &h);
        assert_eq!(a, "claude");
        assert_eq!(args, vec!["--model", "opus"]);
        assert_eq!(d, "/home/u/px");
        // started elsewhere (not by Aleph): just the agent and its folder
        let (_, args, _) = recipe(&s, &[]);
        assert!(args.is_empty());
    }
}
