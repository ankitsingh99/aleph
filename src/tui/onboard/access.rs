//! Step 9, Daemon & access: where the daemon runs (this computer, as a
//! login service; or another machine near the fleet), its API, reaching it
//! from anywhere (Tailscale, Cloudflare: detect, guide, run with consent),
//! and pairing devices. Commands other work is still adding (`aleph daemon
//! deploy`, `expose`, `pair`) are offered when this build has them and
//! described otherwise.

use super::rows::{Row, St};
use super::{has_sub, Action, Ev, Hand, Msg, Run, Step, Wiz};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Facts {
    /// the login service is installed (launchd / systemd)
    pub service: bool,
    /// a daemon answers on its socket
    pub running: bool,
    pub tailscale: Option<String>,
    pub cloudflared: Option<String>,
}

#[derive(Default)]
pub(crate) struct State {
    pub facts: Option<Facts>,
    /// 0 this computer · 1 another machine · 2 not now
    pub place: usize,
    pub deploy_to: usize,
}

pub(crate) fn enter(w: &mut Wiz) {
    if w.ac.facts.is_none() {
        retry(w);
    }
}

pub(crate) fn retry(w: &mut Wiz) {
    w.spawn(
        "access",
        "Looking at the daemon, Tailscale and cloudflared",
        || {
            let ver = |b: &str, arg: &str| {
                crate::proc::which(b).map(|p| {
                    let o = crate::proc::cap(&p.to_string_lossy(), &[arg], Duration::from_secs(5));
                    format!("{}\n{}", o.text(), o.stderr)
                        .lines()
                        .find_map(crate::probe::find_version)
                        .unwrap_or_default()
                })
            };
            Msg::Access(Facts {
                service: crate::setup::watch_installed(),
                running: crate::daemon::request("daemon.status", serde_json::json!({})).is_ok(),
                tailscale: ver("tailscale", "version"),
                cloudflared: ver("cloudflared", "--version"),
            })
        },
    );
}

pub(crate) fn rows(w: &Wiz) -> Vec<Row> {
    let f = w.ac.facts.clone();
    let mut v = vec![Row::head("Where the daemon runs")];
    v.push(
        Row::local(
            "place",
            "the daemon runs",
            vec![
                ("here".into(), "on this computer".into()),
                ("there".into(), "on another machine".into()),
                ("none".into(), "not now".into()),
            ],
            w.ac.place,
        )
        .why("The daemon watches the fleet while no dashboard is open: desktop and phone notifications, auto-approve, the queue, self-heal, Moshi, the API Raycast and the web use. One per computer; with daemons on several, hub.machine says which one runs the automation."),
    );
    match w.ac.place {
        0 => {
            let svc = f.as_ref().map(|f| f.service);
            v.push(match svc {
                None => Row::check(St::Busy, "service", "at login", "looking".into()),
                Some(true) => Row::check(St::Ok, "service", "at login", format!("installed ({})", if cfg!(target_os = "macos") { "launchd dev.aleph.daemon" } else { "systemd --user aleph.service" })),
                Some(false) => Row::check(St::Info, "service", "at login", "not installed".into()),
            });
            if svc == Some(false) {
                v.push(
                    Row::pick("svc", "run it at login", w.plan.has(&Action::Service))
                        .note(if has_sub("daemon", "install") {
                            "aleph daemon install, in Review"
                        } else {
                            "aleph watch --install, in Review"
                        })
                        .why("Installs the login service (launchd on macOS, systemd --user on Linux) that runs aleph watch, so notifications and automation work with no dashboard open. aleph watch --uninstall removes it."),
                );
            }
            if let Some(f) = &f {
                v.push(Row::check(
                    if f.running { St::Ok } else { St::Info },
                    "running",
                    "right now",
                    if f.running { "running".to_string() } else { "not running (a dashboard does its work while it's open)".to_string() },
                ));
            }
            v.push(
                Row::input("watch_interval", "checks every (s)", w.value("watch_interval"), "seconds", w.pending("watch_interval"))
                    .why("How often the daemon polls each machine when herdr's live events aren't flowing."),
            );
        }
        1 => {
            let hosts = w.machines();
            if has_sub("daemon", "deploy") && !hosts.is_empty() {
                let i = w.ac.deploy_to.min(hosts.len() - 1);
                v.push(Row::local("deploy-to", "deploy to", hosts.iter().map(|h| (h.clone(), h.clone())).collect(), i));
                let a = Action::Daemon {
                    args: vec!["deploy".into(), hosts[i].clone()],
                };
                v.push(
                    Row::pick("deploy", format!("deploy the daemon to {}", hosts[i]), w.plan.has(&a))
                        .note("aleph daemon deploy, in Review (it builds and copies Aleph there)")
                        .why("Puts Aleph on that machine and runs its daemon there, near the fleet. A bare aleph here then connects to it."),
                );
            } else {
                v.push(Row::info("Deploying the daemon to a machine comes with aleph daemon deploy — not in this build yet."));
                v.push(Row::info("Meanwhile, with Aleph installed there: a bare aleph can open its dashboard there instead."));
            }
            let opts: Vec<(String, String)> = std::iter::once(("local".to_string(), "here".to_string()))
                .chain(hosts.iter().map(|h| (format!("ssh://{h}"), format!("{h} (ssh)"))))
                .chain(hosts.iter().map(|h| (format!("mosh://{h}"), format!("{h} (mosh)"))))
                .collect();
            let cur = w.value("daemon");
            let mut opts = opts;
            if !opts.iter().any(|(x, _)| *x == cur) {
                opts.push((cur.clone(), cur.clone()));
            }
            v.push(
                Row::choice("daemon", "a bare aleph opens", opts, cur, w.pending("daemon"))
                    .why("Where `aleph` with no command opens the dashboard: here, or on a machine near the fleet over ssh or mosh (it reconnects when the link drops). aleph --local always stays here."),
            );
        }
        _ => v.push(Row::info("Without a daemon, an open dashboard does its work (notifications, automation) while it runs.")),
    }
    v.push(Row::head("API"));
    if crate::config::known("api.http").is_some() {
        v.push(
            Row::toggle(
                "api.http",
                "HTTP API",
                w.flag("api.http"),
                w.pending("api.http"),
            )
            .why(
                "The daemon's API over HTTP, for the web console and phones (paired with a token).",
            ),
        );
    } else {
        v.push(Row::info("The daemon's API is its local socket (Raycast, aleph api). An HTTP API for the web console is coming."));
    }
    v.push(Row::head("From anywhere"));
    for (id, name, ver, sub, guide) in [
        (
            "tailscale",
            "Tailscale",
            f.as_ref().and_then(|f| f.tailscale.clone()),
            "tailscale",
            "your tailnet reaches this computer: aleph --via, mosh, the API once it's on HTTP",
        ),
        (
            "cloudflare",
            "cloudflared",
            f.as_ref().and_then(|f| f.cloudflared.clone()),
            "cloudflare",
            "a Cloudflare tunnel can publish the daemon's HTTP API behind Access",
        ),
    ] {
        v.push(match &ver {
            Some(x) => Row::check(
                St::Ok,
                id,
                name,
                format!(
                    "{} · {guide}",
                    if x.is_empty() {
                        "installed".to_string()
                    } else {
                        x.clone()
                    }
                ),
            ),
            None if f.is_none() => Row::check(St::Busy, id, name, "looking".into()),
            None => match install_of(id) {
                Some(a) if w.plan.has(&a) => Row::check(
                    St::Todo,
                    id,
                    name,
                    format!("queued: installs when you apply — {guide}"),
                )
                .fix("unqueue it"),
                Some(_) => Row::check(St::Info, id, name, format!("not installed — {guide}"))
                    .fix("queue its install"),
                None => Row::check(St::Info, id, name, format!("not installed — {guide}")),
            },
        });
        if ver.is_some() && has_sub("daemon", "expose") {
            let a = Action::Daemon {
                args: vec!["expose".into(), sub.into()],
            };
            v.push(
                Row::pick(format!("expose:{sub}"), format!("  expose the API through {name}"), w.plan.has(&a))
                    .note(format!("aleph daemon expose {sub}, in Review"))
                    .why("Runs aleph daemon expose in Review: it says what it will publish and asks before it does."),
            );
        }
    }
    v.push(Row::head("Devices"));
    if has_sub("daemon", "pair") {
        v.push(Row::button("pair", "pair a device (QR code)").why("aleph daemon pair shows a QR code: scan it on the phone or tablet to use the web console."));
    } else {
        v.push(Row::info("Pairing a phone or tablet with a QR code comes with the web console (aleph daemon pair)."));
    }
    v
}

/// How Tailscale or cloudflared gets onto this computer, when Aleph can do
/// it: Homebrew; Tailscale's own script on Linux.
fn install_of(id: &str) -> Option<Action> {
    match id {
        "tailscale" => Action::tool_here("tailscale"),
        "cloudflare" => Action::tool_here("cloudflared"),
        _ => None,
    }
}

pub(crate) fn act(w: &mut Wiz, ev: Ev) -> Option<Hand> {
    match ev {
        Ev::Local(id, i) if id == "place" => {
            w.ac.place = i;
            if i != 0 {
                w.plan.want(Step::Access, Action::Service, false);
            }
        }
        Ev::Local(id, i) if id == "deploy-to" => {
            w.plan.actions.retain(|(_, a)| !matches!(a, Action::Daemon { args } if args.first().is_some_and(|x| x == "deploy")));
            w.ac.deploy_to = i;
        }
        Ev::Pick(id) if id == "svc" => {
            let on = !w.plan.has(&Action::Service);
            w.plan.want(Step::Access, Action::Service, on);
        }
        Ev::Pick(id) if id == "deploy" => {
            let hosts = w.machines();
            let h = hosts
                .get(w.ac.deploy_to.min(hosts.len().saturating_sub(1)))?
                .clone();
            let a = Action::Daemon {
                args: vec!["deploy".into(), h],
            };
            let on = !w.plan.has(&a);
            w.plan.want(Step::Access, a, on);
        }
        Ev::Pick(id) if id.starts_with("expose:") => {
            let a = Action::Daemon {
                args: vec![
                    "expose".into(),
                    id.trim_start_matches("expose:").to_string(),
                ],
            };
            let on = !w.plan.has(&a);
            w.plan.want(Step::Access, a, on);
        }
        Ev::Fix(id) if id == "tailscale" || id == "cloudflare" => {
            if let Some(a) = install_of(&id) {
                let on = !w.plan.has(&a);
                w.toast(
                    false,
                    if on {
                        format!("queued: {} — installs when you apply", a.label())
                    } else {
                        format!("out of the queue: {}", a.label())
                    },
                );
                w.plan.want(Step::Access, a, on);
            }
        }
        Ev::Button(id) if id == "pair" => {
            let h = Hand {
                title: "Pair a device".into(),
                note: "aleph daemon pair shows a QR code; scan it on the device.".into(),
                run: Run::aleph(&["daemon", "pair"]),
                step: Step::Access,
                id: "pair".into(),
            };
            w.confirm_hand(h, vec![]);
        }
        _ => {}
    }
    None
}
