//! The folder browser in the demo world: each machine's home holds its
//! sessions' projects (git repositories with the usual folders) and a few
//! others, answered the way the remote helper would (`remote.rs`).

use super::world::{self, Machine};
use crate::cache::Dir;
use crate::remote::{Ev, Info, Listing, Msg, Op, Req};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// What's in a project, by what kind it is.
fn project_dirs(dir: &str) -> &'static [&'static str] {
    match dir {
        "planet-factory" | "pulsar-timing" | "epstein-drive" | "ring-gate" => {
            &["data", "notebooks", "scripts", "src", "tests", ".venv"]
        }
        "rocinante-pdc" => &["node_modules", "pdc", "scripts", "test"],
        _ => &["benches", "docs", "examples", "src", "target", "tests"],
    }
}

fn markers(dir: &str) -> Vec<String> {
    let mut v = vec!["CLAUDE.md".to_string(), "README.md".to_string()];
    v.push(
        match project_dirs(dir).first() {
            Some(&"data") => "pyproject.toml",
            Some(&"node_modules") => "package.json",
            _ => "Cargo.toml",
        }
        .to_string(),
    );
    v
}

fn projects(m: &Machine) -> Vec<&'static str> {
    let mut v: Vec<&str> = world::BASE
        .iter()
        .filter(|b| b.host == m.alias)
        .map(|b| b.dir)
        .collect();
    v.dedup();
    v
}

/// The folders under `rel` (path parts below the home), with a git flag;
/// None: no such folder.
fn children(m: &Machine, rel: &[&str]) -> Option<Vec<(String, bool)>> {
    let ps = projects(m);
    let plain = |v: &[&str]| Some(v.iter().map(|x| (x.to_string(), false)).collect());
    match rel {
        [] => {
            let mut v: Vec<(String, bool)> = ps.iter().map(|p| (p.to_string(), true)).collect();
            for x in ["notes", "scratch", ".config", ".local"] {
                v.push((x.into(), false));
            }
            if !m.gpus.is_empty() {
                v.push(("datasets".into(), false));
            }
            if m.mac() {
                v.extend(["Desktop", "Documents", "Downloads"].map(|x| (x.to_string(), false)));
            }
            v.sort();
            Some(v)
        }
        [p] if ps.contains(p) => {
            let mut v: Vec<(String, bool)> = project_dirs(p)
                .iter()
                .map(|x| (x.to_string(), false))
                .collect();
            v.push((".git".into(), false));
            v.sort();
            Some(v)
        }
        [p, "src"] if ps.contains(p) => plain(&["core", "engine", "io"]),
        [p, _] if ps.contains(p) => plain(&[]),
        ["notes"] => plain(&["2026-09", "2026-10", "ideas"]),
        ["datasets"] => plain(&["coastlines", "pulsars", "telemetry"]),
        ["scratch"] | [".config"] | [".local"] | ["Desktop"] | ["Documents"] | ["Downloads"] => {
            plain(&[])
        }
        _ => None,
    }
}

/// `path` as the parts below the machine's home (None: outside it, or not
/// a folder of this world).
fn rel(m: &Machine, path: &[u8]) -> Option<Vec<String>> {
    let p = String::from_utf8_lossy(path).to_string();
    let home = m.home();
    let p = if p == "~" || p.is_empty() {
        home.clone()
    } else if let Some(r) = p.strip_prefix("~/") {
        format!("{home}/{r}")
    } else {
        p
    };
    let r = p.strip_prefix(&home)?;
    if !r.is_empty() && !r.starts_with('/') {
        return None;
    }
    Some(
        r.split('/')
            .filter(|x| !x.is_empty() && *x != ".")
            .map(String::from)
            .collect(),
    )
}

fn real(m: &Machine, rel: &[String]) -> Vec<u8> {
    let mut p = m.home();
    for x in rel {
        p.push('/');
        p.push_str(x);
    }
    p.into_bytes()
}

fn mtime(rel: &[String]) -> i64 {
    let h = world::hash(&[&rel.join("/"), "mtime"]);
    (super::now() - h % 900_000) as i64
}

fn list(m: &Machine, r: &[String]) -> Option<Vec<Dir>> {
    let parts: Vec<&str> = r.iter().map(String::as_str).collect();
    children(m, &parts).map(|v| {
        v.into_iter()
            .map(|(name, git)| Dir {
                name: name.into_bytes(),
                link: false,
                denied: false,
                git,
            })
            .collect()
    })
}

fn info(m: &Machine, r: &[String]) -> Option<Info> {
    let parts: Vec<&str> = r.iter().map(String::as_str).collect();
    let kids = children(m, &parts)?;
    let project = matches!(parts.as_slice(), [p] if projects(m).contains(p));
    let h = world::hash(&[m.alias, &r.join("/"), "info"]);
    let task = world::BASE
        .iter()
        .find(|b| b.host == m.alias && parts.first() == Some(&b.dir))
        .map(|b| &world::TASKS[b.task]);
    Some(Info {
        mtime: mtime(r),
        dirs: kids.len(),
        files: (3 + h % 20) as usize,
        markers: if project { markers(parts[0]) } else { vec![] },
        branch: project.then(|| "main".to_string()),
        commit: if project {
            task.map(|t| {
                (
                    format!("{:07x}", h & 0xfff_ffff),
                    format!("{} hours ago", 2 + h % 9),
                    t.steps
                        .first()
                        .map_or("wip".into(), |s| format!("{}: first pass", s.arg)),
                )
            })
        } else {
            None
        },
    })
}

/// Every folder up to `depth` below `r`, as relative paths (dot-folders left out).
fn find(m: &Machine, r: &[String], depth: u64) -> Option<Vec<Dir>> {
    let mut out = Vec::new();
    let mut stack: Vec<(Vec<String>, String, u64)> = vec![(r.to_vec(), String::new(), 0)];
    while let Some((at, prefix, d)) = stack.pop() {
        if d >= depth {
            continue;
        }
        let parts: Vec<&str> = at.iter().map(String::as_str).collect();
        for (name, git) in children(m, &parts)? {
            if name.starts_with('.') {
                continue;
            }
            let relp = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            out.push(Dir {
                name: relp.clone().into_bytes(),
                link: false,
                denied: false,
                git,
            });
            let mut next = at.clone();
            next.push(name);
            stack.push((next, relp, d + 1));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Some(out)
}

fn answer(m: &Machine, r: Req) -> Ev {
    let missing = || "missing".to_string();
    match r.op {
        Op::Stat => Ev::Stat {
            id: r.id,
            res: rel(m, &r.path)
                .filter(|x| list(m, x).is_some())
                .map(|x| (mtime(&x), real(m, &x)))
                .ok_or_else(missing),
        },
        Op::Mkdir => Ev::Stat {
            id: r.id,
            res: Err(super::REFUSED.into()),
        },
        Op::List => Ev::List {
            id: r.id,
            res: rel(m, &r.path)
                .and_then(|x| {
                    list(m, &x).map(|dirs| Listing {
                        mtime: mtime(&x),
                        dirs,
                    })
                })
                .ok_or_else(missing),
        },
        Op::Info => Ev::Info {
            id: r.id,
            res: rel(m, &r.path)
                .and_then(|x| info(m, &x))
                .ok_or_else(missing),
        },
        Op::Find => {
            // `<depth>\t<globs>\t<path>`
            let mut it = r.path.splitn(3, |&b| b == b'\t');
            let depth = it
                .next()
                .and_then(|d| std::str::from_utf8(d).ok()?.parse().ok())
                .unwrap_or(3);
            let _globs = it.next();
            let path = it.next().unwrap_or(b"");
            Ev::Find {
                id: r.id,
                res: rel(m, path)
                    .and_then(|x| {
                        find(m, &x, depth).map(|dirs| {
                            (
                                Listing {
                                    mtime: mtime(&x),
                                    dirs,
                                },
                                false,
                            )
                        })
                    })
                    .ok_or_else(missing),
            }
        }
    }
}

/// Answer the browser's requests for `host` until it shuts down. The machine
/// goes offline while it's down, like a dropped ssh would.
pub fn serve(host: String, rx: Receiver<Msg>, tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let Some(m) = world::machine(&host) else {
            let _ = tx.send(Ev::Offline {
                reason: super::NOT_HERE.into(),
                retry_in: 30,
            });
            return;
        };
        let mut online = None;
        loop {
            let up = super::link_up(m);
            if online != Some(up) {
                online = Some(up);
                let _ = tx.send(if up {
                    Ev::Online {
                        home: m.home().into_bytes(),
                    }
                } else {
                    Ev::Offline {
                        reason: m.down_text(),
                        retry_in: 5,
                    }
                });
            }
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = tx.send(Ev::Closed);
                    return;
                }
                Ok(Msg::Req(r)) if up => {
                    // a link's round trip
                    std::thread::sleep(Duration::from_millis(m.rtt));
                    let _ = tx.send(answer(m, r));
                }
                _ => {}
            }
        }
    });
}
