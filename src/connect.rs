//! `aleph connect <machine> [--mosh] [-- aleph args…]`: Aleph's dashboard on
//! another machine (a VPS near the fleet, say), in this terminal. It's
//! `ssh -t` (or mosh), with the terminal handed straight over, and it
//! reconnects by itself when the link drops: the dashboard there keeps
//! running its hub and notifications the whole time.
//!
//! `aleph --via <machine> [args…]` is the same for one command, and
//! `daemon = ssh://<machine>` (or `mosh://`) makes a bare `aleph` go there
//! (`aleph --local` stays here).

use crate::config::Config;
use crate::out::{err, hint, warn};
use crate::proc;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

/// How the link is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Ssh,
    Mosh,
}

/// `daemon = ssh://vps` / `mosh://vps` → where a bare `aleph` goes.
pub fn remote_of(setting: &str) -> Option<(Via, String)> {
    let s = setting.trim();
    let (via, host) = match s.strip_prefix("ssh://") {
        Some(h) => (Via::Ssh, h),
        None => (Via::Mosh, s.strip_prefix("mosh://")?),
    };
    let host = host.trim_end_matches('/');
    (!host.is_empty()).then(|| (via, host.to_string()))
}

/// The remote command: Aleph through a login `sh` (so its PATH is the
/// user's, plus where Aleph installs), whatever the login shell is (pure —
/// tested). For ssh it's one string the remote shell parses; mosh takes argv.
/// mosh can't carry pictures (and brings iTerm2's `LC_TERMINAL` along), so
/// logos aren't images there.
pub fn remote_argv(via: Via, args: &[String]) -> Vec<String> {
    let script = match via {
        Via::Ssh => {
            r#"PATH="$PATH:$HOME/.local/bin:$HOME/.cargo/bin"; ALEPH_REMOTE=1; export PATH ALEPH_REMOTE; exec aleph "$@""#
        }
        Via::Mosh => {
            r#"PATH="$PATH:$HOME/.local/bin:$HOME/.cargo/bin"; ALEPH_REMOTE=1; ALEPH_IMAGES=0; export PATH ALEPH_REMOTE ALEPH_IMAGES; exec aleph "$@""#
        }
    };
    let mut v = vec![
        "sh".to_string(),
        "-lc".to_string(),
        script.to_string(),
        "sh".to_string(),
    ];
    v.extend(args.iter().cloned());
    v
}

pub fn remote_line(args: &[String]) -> String {
    remote_argv(Via::Ssh, args)
        .iter()
        .map(|a| proc::sh_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `ssh -t …` or `mosh …` for one run (pure — tested).
pub fn argv(via: Via, host: &str, args: &[String]) -> Vec<String> {
    match via {
        Via::Ssh => {
            let mut v = vec![crate::sshcfg::ssh_bin(), "-t".to_string()];
            v.extend(crate::sshcfg::interactive());
            v.push(host.to_string());
            v.push(remote_line(args));
            v
        }
        Via::Mosh => {
            let mut v = vec!["mosh".to_string(), host.to_string(), "--".to_string()];
            v.extend(remote_argv(Via::Mosh, args));
            v
        }
    }
}

/// Run Aleph on `host` until it's quit there, reconnecting after drops.
pub fn run(cfg: &Config, via: Via, host: &str, args: &[String]) -> ExitCode {
    let via = if via == Via::Mosh && proc::which("mosh").is_none() {
        warn("mosh isn't installed here — connecting with ssh");
        Via::Ssh
    } else {
        via
    };
    let _ = cfg;
    let mut drops = 0u32;
    loop {
        let a = argv(via, host, args);
        let t0 = Instant::now();
        let st = Command::new(&a[0]).args(&a[1..]).status();
        let code = match st {
            Ok(s) => s.code(),
            Err(e) => {
                err(&format!("couldn't run {}: {e}", a[0]));
                return ExitCode::from(1);
            }
        };
        let ran = t0.elapsed();
        match code {
            Some(0) => return ExitCode::SUCCESS,
            // ssh's own failure: the connection, not Aleph
            Some(255) | None if via == Via::Ssh => {
                if ran < Duration::from_secs(3) && drops == 0 {
                    err(&format!("couldn't reach {host}"));
                    hint(&format!(
                        "aleph doctor --host {host} — and Aleph must be installed there (aleph daemon deploy {host})"
                    ));
                    return ExitCode::from(1);
                }
                drops += 1;
                if drops > 20 {
                    err(&format!("the link to {host} keeps dropping — giving up"));
                    return ExitCode::from(1);
                }
                let wait = backoff(drops);
                warn(&format!(
                    "the connection to {host} dropped — reconnecting in {}s (ctrl+c stops; Aleph keeps running there)",
                    wait.as_secs()
                ));
                std::thread::sleep(wait);
            }
            Some(127) => {
                err(&format!("Aleph isn't installed on {host}"));
                hint(&format!("aleph daemon deploy {host}"));
                return ExitCode::from(1);
            }
            Some(c) => return ExitCode::from(c.clamp(1, 255) as u8),
            None => return ExitCode::from(1),
        }
    }
}

/// 3 s, then longer, up to 30 s.
fn backoff(n: u32) -> Duration {
    Duration::from_secs((3 * n as u64).min(30))
}

/// `aleph connect <machine> [--mosh | --ssh] [-- args…]`
pub fn cmd(a: crate::cli::Args) -> ExitCode {
    let p: Vec<&String> = a.rest.iter().take_while(|x| *x != "--").collect();
    let rest: Vec<String> = a
        .rest
        .iter()
        .skip_while(|x| *x != "--")
        .skip(1)
        .cloned()
        .collect();
    let host = p
        .iter()
        .find(|x| !x.starts_with('-'))
        .map(|s| s.to_string())
        .or(a.flag.clone());
    let Some(host) = host else {
        return crate::out::fail("usage: aleph connect <machine> [--mosh] [-- aleph args…]");
    };
    let via = if p.iter().any(|x| *x == "--mosh") {
        Via::Mosh
    } else if p.iter().any(|x| *x == "--ssh") {
        Via::Ssh
    } else if a.cfg.get_or("attach_via") == "mosh" {
        Via::Mosh
    } else {
        Via::Ssh
    };
    run(&a.cfg, via, &host, &rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings() {
        assert_eq!(remote_of("ssh://vps"), Some((Via::Ssh, "vps".into())));
        assert_eq!(remote_of("mosh://vps/"), Some((Via::Mosh, "vps".into())));
        assert_eq!(remote_of("local"), None);
        assert_eq!(remote_of("ssh://"), None);
    }

    #[test]
    fn commands() {
        let l = remote_line(&["--host".into(), "b31".into(), "it's".into()]);
        assert!(l.starts_with("sh -lc 'PATH="), "{l}");
        assert!(l.contains("exec aleph \"$@\"'"), "{l}");
        assert!(l.ends_with(" sh --host b31 'it'\\''s'"), "{l}");
        let m = argv(Via::Mosh, "vps", &["focus".into()]);
        assert_eq!(m[..4], ["mosh", "vps", "--", "sh"]);
        assert_eq!(m.last().unwrap(), "focus");
        assert!(m[5].contains("ALEPH_IMAGES=0;"), "{m:?}");
        assert!(!l.contains("ALEPH_IMAGES"), "{l}");
        let s = argv(Via::Ssh, "vps", &[]);
        assert_eq!(s[1], "-t");
        assert!(s.contains(&"vps".to_string()));
        assert_eq!(backoff(1), Duration::from_secs(3));
        assert_eq!(backoff(40), Duration::from_secs(30));
    }
}
