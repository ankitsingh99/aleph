//! Installing an agent on a machine (`aleph agents install <agent>`): pick
//! the registry's install method whose prerequisites the machine has (from a
//! [`probe`](crate::probe) of it), explain what's missing in plain words, and
//! build the exact script that runs over ssh.
//!
//! Rules: the command shown is the command run; nothing runs as root — a
//! global npm folder that needs root stops the plan, and the run puts a
//! `sudo`/`doas` stub first on the PATH so an installer that reaches for sudo
//! fails loudly instead of escalating quietly.

use crate::agents::Agent;
use crate::probe::HostProbe;

/// How an agent installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// `npm i -g <package>`
    Npm(String),
    /// the vendor's `curl … | bash` installer: the exact command, the
    /// fetcher it uses and the shell it pipes to
    Script {
        cmd: String,
        fetch: &'static str,
        shell: &'static str,
    },
    /// `uv tool install <package>`
    Uv(String),
    /// `brew install <formula>`
    Brew(String),
    /// no command we can run: where to read how
    Manual(String),
}

impl Method {
    /// The command that runs on the machine.
    pub fn command(&self) -> Option<String> {
        match self {
            Method::Npm(p) => Some(format!("npm i -g {p}")),
            Method::Script { cmd, .. } => Some(cmd.clone()),
            Method::Uv(p) => Some(format!("uv tool install {p}")),
            Method::Brew(f) => Some(format!("brew install {f}")),
            Method::Manual(_) => None,
        }
    }
}

/// A registry install string → a method. Anything we don't recognise is
/// treated as instructions to read, never run.
pub fn parse_method(s: &str) -> Method {
    // `npm i -g command-code (https://commandcode.ai)`: the note isn't part of it
    let s = s.split(" (").next().unwrap_or(s).trim();
    let w: Vec<&str> = s.split_whitespace().collect();
    let pkg_ok = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@/._-:".contains(&b))
    };
    match w.as_slice() {
        ["npm", "i" | "install", "-g", p] if pkg_ok(p) => Method::Npm(p.to_string()),
        ["uv", "tool", "install", p] if pkg_ok(p) => Method::Uv(p.to_string()),
        ["brew", "install", f] if pkg_ok(f) => Method::Brew(f.to_string()),
        [fetch @ ("curl" | "wget"), ..] if !s.contains("sudo") && s.matches('|').count() == 1 => {
            match script_shell(&w) {
                Some(shell) => Method::Script {
                    cmd: s.to_string(),
                    fetch: if *fetch == "curl" { "curl" } else { "wget" },
                    shell,
                },
                None => Method::Manual(s.to_string()),
            }
        }
        _ => Method::Manual(s.to_string()),
    }
}

/// The shell a `curl … | bash` line pipes into: `| bash`, or `| bash -s --`
/// with the installer's own plain `--flags` after it (pure).
fn script_shell(w: &[&str]) -> Option<&'static str> {
    let i = w.iter().position(|x| *x == "|")?;
    let shell = match *w.get(i + 1)? {
        "bash" => "bash",
        "sh" => "sh",
        _ => return None,
    };
    match &w[i + 2..] {
        [] => Some(shell),
        ["-s", "--", flags @ ..]
            if !flags.is_empty()
                && flags.iter().all(|f| {
                    f.starts_with("--")
                        && f.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-=._".contains(&b))
                }) =>
        {
            Some(shell)
        }
        _ => None,
    }
}

/// Every way the registry knows to install `a`, preferred first.
pub fn methods(a: &Agent) -> Vec<Method> {
    [a.install, a.alt_install]
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| parse_method(s))
        .collect()
}

/// One thing the install needs, and whether the machine has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prereq {
    pub what: String,
    pub ok: bool,
    /// what we found (or didn't)
    pub detail: String,
    /// how to get it, when it's missing
    pub fix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub agent: &'static Agent,
    pub method: Method,
    /// exactly what runs (None for a manual install)
    pub command: Option<String>,
    pub prereqs: Vec<Prereq>,
    /// why it can't run here, in a sentence
    pub blocker: Option<String>,
    /// a folder to put first on the PATH (the npm that should run)
    pub path_first: Option<String>,
    /// where the agent's executable will be, when we know
    pub lands_in: Option<String>,
    pub notes: Vec<String>,
}

impl Plan {
    pub fn ready(&self) -> bool {
        self.blocker.is_none() && self.command.is_some()
    }
}

/// The lowest Node.js an npm-installed agent runs on.
fn min_node(a: &Agent) -> u64 {
    match a.id {
        // gemini-cli needs Node.js 20
        "gemini" => 20,
        _ => 18,
    }
}

fn major(v: &str) -> Option<u64> {
    v.trim_start_matches(['v', 'V'])
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn host_name(h: &str) -> String {
    if h == crate::probe::LOCAL {
        "this machine".into()
    } else {
        h.to_string()
    }
}

/// The plan for one method.
fn plan_with(a: &'static Agent, p: &HostProbe, m: Method) -> Plan {
    let on = host_name(&p.host);
    let mut pl = Plan {
        agent: a,
        command: m.command(),
        method: m.clone(),
        prereqs: vec![],
        blocker: None,
        path_first: None,
        lands_in: None,
        notes: vec![],
    };
    let have = |name: &str| p.tool(name).map(|t| p.tilde(&t.path));
    let need = |pl: &mut Plan, what: &str, found: Option<String>, fix: &str| {
        pl.prereqs.push(Prereq {
            what: what.to_string(),
            ok: found.is_some(),
            detail: found.unwrap_or_else(|| format!("not found on {on}")),
            fix: (!fix.is_empty()).then(|| fix.to_string()),
        });
    };
    match &m {
        Method::Npm(_) => {
            let want = min_node(a);
            let node = p.tool("node");
            let nv = node.and_then(|n| major(&n.version));
            pl.prereqs.push(Prereq {
                what: format!("Node.js ≥ {want}"),
                ok: nv.is_some_and(|v| v >= want),
                detail: match (node, nv) {
                    (None, _) => format!("not found on {on}"),
                    (Some(n), Some(v)) if v < want => {
                        format!("v{} is too old · {}", n.version, p.tilde(&n.path))
                    }
                    (Some(n), _) if n.version.is_empty() => {
                        format!("{} (didn't say its version)", p.tilde(&n.path))
                    }
                    (Some(n), _) => format!("v{} · {}", n.version, p.tilde(&n.path)),
                },
                fix: Some(format!(
                    "install Node.js {want} or newer on {on} — e.g. with nvm (https://github.com/nvm-sh/nvm): nvm install --lts"
                )),
            });
            // a node without its version still runs npm; only a known-old one blocks
            if node.is_some() && nv.is_none() {
                if let Some(x) = pl.prereqs.last_mut() {
                    x.ok = true;
                }
            }
            let npm = p.tool("npm");
            need(
                &mut pl,
                "npm",
                have("npm"),
                "npm comes with Node.js (nvm installs both)",
            );
            if let Some(npm) = npm {
                pl.path_first = npm.path.rsplit_once('/').map(|x| x.0.to_string());
                if !npm.prefix.is_empty() {
                    let bin = format!("{}/bin", npm.prefix);
                    let w = npm.writable;
                    pl.prereqs.push(Prereq {
                        what: "npm's global folder, without sudo".into(),
                        ok: w != Some(false),
                        detail: match w {
                            Some(false) => format!("{} needs root", p.tilde(&npm.prefix)),
                            _ => format!("{} (yours)", p.tilde(&npm.prefix)),
                        },
                        fix: Some(
                            "npm config set prefix ~/.npm-global, then add ~/.npm-global/bin to PATH (or use nvm) — aleph never runs sudo"
                                .into(),
                        ),
                    });
                    if w == Some(false) {
                        pl.blocker = Some(format!(
                            "npm's global folder on {on} ({}) needs root, and aleph never runs sudo",
                            p.tilde(&npm.prefix)
                        ));
                    }
                    if p.on_path(&bin) == Some(false) {
                        pl.notes.push(format!(
                            "{} isn't on your shell's PATH on {on}: herdr types `{}` into a shell, so add it (e.g. in ~/.profile)",
                            p.tilde(&bin),
                            a.exe()
                        ));
                    }
                    pl.lands_in = Some(bin);
                }
            }
            if pl.blocker.is_none() {
                let missing: Vec<&str> = pl
                    .prereqs
                    .iter()
                    .filter(|x| !x.ok)
                    .map(|x| {
                        if x.what.starts_with("Node.js") {
                            "Node.js"
                        } else {
                            "npm"
                        }
                    })
                    .collect();
                if !missing.is_empty() {
                    let mut m = missing.clone();
                    m.dedup();
                    pl.blocker = Some(format!(
                        "{} installs with npm, and {on} needs {} first",
                        a.name,
                        m.join(" and ")
                    ));
                }
            }
        }
        Method::Script { fetch, shell, .. } => {
            need(
                &mut pl,
                fetch,
                have(fetch),
                &format!("install {fetch} with your package manager (e.g. apt install {fetch})"),
            );
            if *shell == "bash" {
                need(&mut pl, "bash", have("bash"), "install bash");
            }
            if let Some(x) = pl.prereqs.iter().find(|x| !x.ok) {
                pl.blocker = Some(format!("{}'s installer needs {} on {on}", a.name, x.what));
            }
            pl.notes.push(
                "the official installer, run as you (it installs into your home folder, usually ~/.local/bin)"
                    .to_string(),
            );
        }
        Method::Uv(_) => {
            need(
                &mut pl,
                "uv",
                have("uv"),
                "install uv: curl -LsSf https://astral.sh/uv/install.sh | sh",
            );
            if !pl.prereqs[0].ok {
                pl.blocker = Some(format!(
                    "{} installs with uv, which {on} doesn't have",
                    a.name
                ));
            }
            pl.lands_in = Some("~/.local/bin".into());
        }
        Method::Brew(_) => {
            need(&mut pl, "Homebrew", have("brew"), "https://brew.sh");
            if !pl.prereqs[0].ok {
                pl.blocker = Some(format!(
                    "{} installs with Homebrew, which {on} doesn't have",
                    a.name
                ));
            }
        }
        Method::Manual(how) => {
            pl.blocker = Some(format!(
                "{} has no install command we can run — see {how}",
                a.name
            ));
        }
    }
    pl
}

/// The best plan for `a` on the probed machine: the first method whose
/// prerequisites are there, else the preferred one (with what's missing).
pub fn plan(a: &'static Agent, p: &HostProbe) -> Plan {
    let all: Vec<Plan> = methods(a).into_iter().map(|m| plan_with(a, p, m)).collect();
    let first_ready = all.iter().position(Plan::ready);
    let mut v = all;
    match first_ready {
        Some(i) => v.swap_remove(i),
        None if v.is_empty() => plan_with(a, p, Method::Manual(a.install.to_string())),
        None => v.swap_remove(0),
    }
}

/// The script that runs the plan (`sh -s`): a `sudo`/`doas` stub first on the
/// PATH, the npm the plan picked next, the command with no stdin in a scratch
/// folder of its own (installers that download into the folder they're run
/// from leave nothing behind, even when stopped), and its exit code as
/// `CCX\t<rc>` (a stub hit prints `CCSUDO …`).
pub fn script(pl: &Plan) -> Option<String> {
    let cmd = pl.command.as_ref()?;
    let first = pl
        .path_first
        .as_ref()
        .map(|d| format!("{}:", crate::proc::sh_quote(d)))
        .unwrap_or_default();
    // npm 11 holds back packages' install scripts unless allowed: the
    // agent's own may fetch its binary
    let allow = match &pl.method {
        Method::Npm(p) => format!(
            "npm_config_allow_scripts={}; export npm_config_allow_scripts\n",
            crate::proc::sh_quote(p)
        ),
        _ => String::new(),
    };
    Some(format!(
        r#"LC_ALL=C; export LC_ALL
S=$(mktemp -d 2>/dev/null) || S=/tmp/cch-install.$$
mkdir -p "$S"
printf '#!/bin/sh\necho "CCSUDO $0 $*" >&2\nexit 1\n' >"$S/sudo"
cp "$S/sudo" "$S/doas"; chmod +x "$S/sudo" "$S/doas"
PATH="$S:{first}$PATH"; export PATH
mkdir -p "$S/work" && cd "$S/work"
trap 'cd /; rm -rf "$S"; exit 143' HUP INT TERM
{allow}(set -o pipefail) 2>/dev/null && set -o pipefail
{{ {cmd}; }} </dev/null 2>&1
rc=$?
cd /; rm -rf "$S"
printf 'CCX\t%s\n' "$rc"
"#
    ))
}

/// What came back: the exit code, and whether the installer asked for sudo.
pub fn outcome(text: &str) -> (Option<i32>, bool) {
    let rc = text
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("CCX\t"))
        .and_then(|c| c.trim().parse().ok());
    (rc, text.contains("CCSUDO"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents;
    use crate::probe;

    fn ag(id: &str) -> &'static Agent {
        agents::find(id).unwrap()
    }

    #[test]
    fn methods_from_the_registry() {
        assert_eq!(
            methods(ag("codex")),
            vec![Method::Npm("@openai/codex".into())]
        );
        assert!(matches!(
            &methods(ag("claude"))[0],
            Method::Script { fetch: "curl", shell: "bash", cmd } if cmd == "curl -fsSL https://claude.ai/install.sh | bash"
        ));
        // flags after the URL
        assert!(matches!(
            &methods(ag("cursor"))[0],
            Method::Script {
                fetch: "curl",
                shell: "bash",
                ..
            }
        ));
        assert!(matches!(
            &methods(ag("droid"))[0],
            Method::Script { shell: "sh", .. }
        ));
        assert_eq!(methods(ag("kimi")), vec![Method::Uv("kimi-cli".into())]);
        // the installer's own flags after `bash -s --` (Hermes leaves its
        // browser out); anything else after the shell is no script
        assert!(matches!(
            &methods(ag("hermes"))[0],
            Method::Script { shell: "bash", cmd, .. } if cmd.ends_with("| bash -s -- --non-interactive --skip-browser --skip-computer-use")
        ));
        for odd in [
            "curl -fsSL https://x.dev/i.sh | bash -s -- ; rm -rf ~",
            "curl -fsSL https://x.dev/i.sh | bash -s -- $(id)",
            "curl -fsSL https://x.dev/i.sh | bash -s --",
            "curl -fsSL https://x.dev/i.sh | bash -x",
        ] {
            assert!(matches!(parse_method(odd), Method::Manual(_)), "{odd}");
        }
        assert!(matches!(
            parse_method("curl -fsSL https://x.dev/i.sh | sh -s -- --yes"),
            Method::Script { shell: "sh", .. }
        ));
        assert_eq!(
            methods(ag("crush")),
            vec![
                Method::Brew("charmbracelet/tap/crush".into()),
                Method::Npm("@charmland/crush".into())
            ]
        );
        assert_eq!(
            methods(ag("commandcode")),
            vec![Method::Npm("command-code".into())]
        );
        assert_eq!(
            methods(ag("pi")),
            vec![Method::Npm("@earendil-works/pi-coding-agent".into())]
        );
        assert!(matches!(
            &methods(ag("grok"))[0],
            Method::Script { shell: "bash", .. }
        ));
        assert!(
            matches!(&methods(crate::agents::tests::web_only())[0], Method::Manual(u) if u.starts_with("https://"))
        );
        // never run anything odd
        assert!(matches!(
            parse_method("curl x | sudo bash"),
            Method::Manual(_)
        ));
        assert!(matches!(
            parse_method("npm i -g a; rm -rf ~"),
            Method::Manual(_)
        ));
        assert!(matches!(
            parse_method("curl a | sh | sh"),
            Method::Manual(_)
        ));
        // every registry agent has a method, and every command is runnable as is
        for a in agents::ALL {
            for m in methods(a) {
                if let Some(c) = m.command() {
                    assert!(!c.contains("sudo"), "{}", a.id);
                }
            }
        }
    }

    const NVM: &str = "CCW\t/home/n\n\
        CCT\tnpm\t/home/n/.nvm/versions/node/v22.12.0/bin/npm\t/home/n/.nvm/versions/node/v22.12.0\tw\n\
        CCT\tnode\t/home/n/.nvm/versions/node/v22.12.0/bin/node\tv22.12.0\n\
        CCT\tcurl\t/usr/bin/curl\n\
        CCT\tbash\t/usr/bin/bash\n\
        CCP\tlogin\t/home/n/.local/bin:/usr/bin:/bin\n\
        CCP\tshell\t/home/n/.nvm/versions/node/v22.12.0/bin:/home/n/.local/bin:/usr/bin:/bin\n\
        CCO\tLinux x86_64\n";

    #[test]
    fn npm_install_with_nvm() {
        let p = probe::parse("dex42", NVM);
        let pl = plan(ag("codex"), &p);
        assert!(pl.ready(), "{:?}", pl.blocker);
        assert_eq!(pl.command.as_deref(), Some("npm i -g @openai/codex"));
        assert_eq!(
            pl.path_first.as_deref(),
            Some("/home/n/.nvm/versions/node/v22.12.0/bin")
        );
        assert_eq!(
            pl.lands_in.as_deref(),
            Some("/home/n/.nvm/versions/node/v22.12.0/bin")
        );
        assert!(pl.notes.is_empty(), "on the shell's PATH: {:?}", pl.notes);
        assert!(pl.prereqs.iter().all(|x| x.ok));
        assert!(pl.prereqs[0]
            .detail
            .contains("~/.nvm/versions/node/v22.12.0/bin/node"));
        let s = script(&pl).unwrap();
        assert!(s.contains("{ npm i -g @openai/codex; } </dev/null 2>&1"));
        assert!(s.contains("PATH=\"$S:/home/n/.nvm/versions/node/v22.12.0/bin:$PATH\""));
        assert!(s.contains("CCSUDO") && s.contains("CCX"));
    }

    #[test]
    fn missing_node_and_npm_is_explained() {
        // b31: no node at all
        let p = probe::parse(
            "b31",
            "CCT\tcurl\t/usr/bin/curl\nCCT\tbash\t/usr/bin/bash\nCCO\tLinux x86_64\n",
        );
        let pl = plan(ag("gemini"), &p);
        assert!(!pl.ready());
        let b = pl.blocker.unwrap();
        assert!(b.contains("needs Node.js and npm first"), "{b}");
        assert_eq!(pl.prereqs[0].what, "Node.js ≥ 20");
        assert!(pl.prereqs[0].fix.as_ref().unwrap().contains("nvm"));
        // the script-installed ones only need curl + bash
        let pl = plan(ag("claude"), &p);
        assert!(pl.ready());
        assert_eq!(
            pl.command.as_deref(),
            Some("curl -fsSL https://claude.ai/install.sh | bash")
        );
        let s = script(&pl).unwrap();
        assert!(s.contains("{ curl -fsSL https://claude.ai/install.sh | bash; } </dev/null"));
        // an installer that downloads into the folder it's run from (Crush's
        // archive-XXXXXX) leaves nothing there, nor its scratch folder
        let mut pl = pl;
        pl.command = Some("mkdir archive-t && touch archive-t/x && pwd".into());
        let here = std::env::temp_dir().join(format!("aleph-inst-{}", std::process::id()));
        std::fs::create_dir_all(&here).unwrap();
        let mut c = std::process::Command::new("sh");
        c.arg("-s").current_dir(&here);
        let sc = script(&pl).unwrap();
        let o = crate::proc::run(
            &mut c,
            std::time::Duration::from_secs(20),
            Some(sc.as_bytes()),
        )
        .unwrap()
        .text();
        let ran_in = o.lines().next().unwrap_or("").to_string();
        assert!(o.contains("CCX\t0"), "{o}");
        assert!(std::fs::read_dir(&here).unwrap().next().is_none());
        assert!(
            ran_in.ends_with("/work") && !std::path::Path::new(&ran_in).exists(),
            "{ran_in}"
        );
        let _ = std::fs::remove_dir_all(&here);
        // no curl: say so
        let p = probe::parse("x", "CCT\tbash\t/bin/bash\nCCO\tLinux\n");
        let pl = plan(ag("opencode"), &p);
        assert!(pl.blocker.unwrap().contains("needs curl"));
    }

    #[test]
    fn old_node_and_root_owned_npm() {
        let p = probe::parse(
            "x",
            "CCT\tnpm\t/usr/bin/npm\t/usr\tro\nCCT\tnode\t/usr/bin/node\tv16.20.0\nCCO\tLinux\n",
        );
        let pl = plan(ag("codex"), &p);
        assert!(!pl.ready());
        assert!(pl.blocker.as_ref().unwrap().contains("needs root"));
        assert!(pl.blocker.unwrap().contains("never runs sudo"));
        assert!(pl.prereqs[0].detail.contains("too old"));
        let p = probe::parse(
            "x",
            "CCT\tnpm\t/usr/bin/npm\t/home/u/.npm-global\tw\nCCT\tnode\t/usr/bin/node\tv16.20.0\nCCP\tlogin\t/usr/bin:/bin\nCCO\tLinux\n",
        );
        let pl = plan(ag("codex"), &p);
        assert!(pl.blocker.unwrap().contains("needs Node.js first"));
        // writable prefix off the PATH: a note, not a blocker
        let p = probe::parse(
            "x",
            "CCT\tnpm\t/usr/bin/npm\t/home/u/.npm-global\tw\nCCT\tnode\t/usr/bin/node\tv20.1.0\nCCP\tlogin\t/usr/bin:/bin\nCCO\tLinux\n",
        );
        let pl = plan(ag("codex"), &p);
        assert!(pl.ready());
        assert!(pl.notes[0].contains("isn't on your shell's PATH"));
    }

    #[test]
    fn falls_back_to_the_next_method() {
        // crush: Homebrew first, npm when there's no brew
        let p = probe::parse("dex42", NVM);
        let pl = plan(ag("crush"), &p);
        assert_eq!(pl.method, Method::Npm("@charmland/crush".into()));
        let mac = probe::parse(
            "m",
            "CCT\tbrew\t/opt/homebrew/bin/brew\nCCO\tDarwin arm64\n",
        );
        let pl = plan(ag("crush"), &mac);
        assert_eq!(
            pl.command.as_deref(),
            Some("brew install charmbracelet/tap/crush")
        );
        // neither: the preferred one, with what's missing
        let bare = probe::parse("b", "CCO\tLinux\n");
        let pl = plan(ag("crush"), &bare);
        assert!(pl.blocker.unwrap().contains("Homebrew"));
        // uv
        let pl = plan(ag("kimi"), &bare);
        assert!(pl.blocker.unwrap().contains("uv"));
        assert!(pl.prereqs[0].fix.as_ref().unwrap().contains("astral.sh/uv"));
        // a web page: never run
        let pl = plan(crate::agents::tests::web_only(), &bare);
        assert!(pl.command.is_none() && script(&pl).is_none());
        assert!(pl.blocker.unwrap().contains("no install command"));
    }

    #[test]
    fn outcomes() {
        assert_eq!(outcome("added 1 package\nCCX\t0\n"), (Some(0), false));
        assert_eq!(
            outcome("CCSUDO /tmp/x/sudo mv a b\nCCX\t1\n"),
            (Some(1), true)
        );
        assert_eq!(outcome("ssh died"), (None, false));
    }
}
