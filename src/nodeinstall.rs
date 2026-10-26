//! Node.js for the agents that install with npm (Codex, Gemini CLI, Qwen
//! Code, …) on a machine that has none, or one too old: the current LTS —
//! the newest LTS release nodejs.org lists, so every security fix published
//! for that line so far. It's downloaded on the machine, its SHA-256 checked
//! against nodejs.org's `SHASUMS256.txt`, unpacked into
//! `~/.local/share/aleph/node` (no sudo, no package manager, nothing
//! outside your home folder), `node` / `npm` / `npx` linked into
//! `~/.local/bin`, and npm's global folder set to `~/.local` when you
//! haven't set one, so `npm i -g` needs no sudo and lands on the PATH.
//!
//! Two scripts over ssh (`sh -s`): a look (the machine's system, a fetcher,
//! the LTS releases nodejs.org offers) and the install. Choosing the release
//! is pure ([`pick`]) and tested. `agents.node = off` turns it all off.

use crate::config::Config;
use crate::probe::HostProbe;

/// Where releases come from.
pub const DIST: &str = "https://nodejs.org/dist";
/// Where it goes on the machine (under $HOME).
pub const HOME_DIR: &str = ".local/share/aleph/node";

/// An LTS release nodejs.org lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `v24.11.1`
    pub version: String,
    /// the LTS line's name (`Krypton`)
    pub lts: String,
    /// this release carries security fixes
    pub security: bool,
}

impl Release {
    /// `Node.js v24.11.1 (LTS "Krypton")`
    pub fn label(&self) -> String {
        format!("Node.js {} (LTS \"{}\")", self.version, self.lts)
    }
}

/// What the look found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// `linux-x64`, `darwin-arm64` (None: no official build for it — and why)
    pub plat: Result<String, String>,
    /// `curl` / `wget` (empty: neither)
    pub fetch: String,
    /// the LTS releases, newest first
    pub releases: Vec<Release>,
    /// why nodejs.org couldn't be read
    pub err: Option<String>,
}

/// Is it on (`agents.node`, default auto)?
pub fn enabled(cfg: &Config) -> bool {
    cfg.get_or("agents.node") != "off"
}

/// The official build's name for a system (`uname -s`, `uname -m`, libc) —
/// pure.
pub fn platform(os: &str, arch: &str, libc: &str) -> Result<String, String> {
    let o = match os {
        "Linux" | "linux" => "linux",
        "Darwin" | "darwin" => "darwin",
        "" => return Err("couldn't tell its system".into()),
        x => return Err(format!("nodejs.org has no build for {x}")),
    };
    if o == "linux" && libc == "musl" {
        return Err(
            "it uses musl (Alpine): nodejs.org's builds need glibc — apk add nodejs npm".into(),
        );
    }
    let m = match arch {
        "x86_64" | "amd64" | "x64" => "x64",
        "aarch64" | "arm64" => "arm64",
        "armv7l" if o == "linux" => "armv7l",
        "ppc64le" if o == "linux" => "ppc64le",
        "s390x" if o == "linux" => "s390x",
        "" => return Err("couldn't tell its processor".into()),
        x => return Err(format!("nodejs.org has no {o} build for {x}")),
    };
    Ok(format!("{o}-{m}"))
}

/// Could Aleph put Node.js on this machine, from what the probe knows (a
/// fetcher; a system with an official build)? Pure.
pub fn possible(p: &HostProbe) -> Result<(), String> {
    if p.tool("curl").is_none() && p.tool("wget").is_none() {
        return Err("it has neither curl nor wget to download it".into());
    }
    let mut it = p.os.split_whitespace();
    let (os, arch) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
    platform(os, arch, "").map(|_| ())
}

/// The look (`sh -s`): `CCN plat <os> <arch> <libc>`, `CCN fetch <tool>`,
/// then the newest three LTS entries of nodejs.org's index (`CCN rel …`),
/// or `CCN err <why>`.
pub fn look_script() -> String {
    r#"LC_ALL=C; export LC_ALL
l=gnu
if [ "$(uname -s 2>/dev/null)" = Linux ]; then { [ -e /etc/alpine-release ] || ldd --version 2>&1 | grep -qi musl; } && l=musl; fi
printf 'CCN\tplat\t%s\t%s\t%s\n' "$(uname -s 2>/dev/null)" "$(uname -m 2>/dev/null)" "$l"
if command -v curl >/dev/null 2>&1; then F=curl; elif command -v wget >/dev/null 2>&1; then F=wget; else F=; fi
printf 'CCN\tfetch\t%s\n' "$F"
[ -n "$F" ] || exit 0
if [ "$F" = curl ]; then I=$(curl -fsSL --max-time 30 @DIST@/index.json 2>&1); r=$?; else I=$(wget -qO- --timeout=30 @DIST@/index.json 2>&1); r=$?; fi
if [ $r -ne 0 ]; then printf 'CCN\terr\t%s\n' "$(printf '%s\n' "$I" | grep . | tail -n 1 | tr '\t' ' ')"; exit 0; fi
printf '%s\n' "$I" | tr '{' '\n' | grep '"lts":"' | head -n 3 | while IFS= read -r e; do printf 'CCN\trel\t%s\n' "$(printf '%s' "$e" | tr -d '\t\r')"; done
"#
    .replace("@DIST@", DIST)
}

/// `"version":"v24.11.1"` → `v24.11.1` (a field of one index entry).
fn field<'a>(entry: &'a str, k: &str) -> Option<&'a str> {
    let pat = format!("\"{k}\":");
    let rest = &entry[entry.find(&pat)? + pat.len()..];
    let rest = rest.trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        return s.split('"').next();
    }
    rest.split([',', '}', ']']).next().map(str::trim)
}

/// The look's answer (pure).
pub fn parse_look(text: &str) -> Look {
    let mut lk = Look {
        plat: Err("couldn't tell its system".into()),
        fetch: String::new(),
        releases: vec![],
        err: None,
    };
    for l in text.lines() {
        let f: Vec<&str> = l.splitn(3, '\t').collect();
        if f.first() != Some(&"CCN") || f.len() < 2 {
            continue;
        }
        let v = f.get(2).copied().unwrap_or("");
        match f[1] {
            "plat" => {
                let p: Vec<&str> = v.split('\t').collect();
                lk.plat = platform(
                    p.first().copied().unwrap_or(""),
                    p.get(1).copied().unwrap_or(""),
                    p.get(2).copied().unwrap_or(""),
                );
            }
            "fetch" => lk.fetch = v.trim().to_string(),
            "err" => lk.err = Some(v.trim().to_string()),
            "rel" => {
                if let (Some(ver), Some(lts)) = (field(v, "version"), field(v, "lts")) {
                    if ver.starts_with('v') && lts != "false" && !lts.is_empty() {
                        lk.releases.push(Release {
                            version: ver.to_string(),
                            lts: lts.to_string(),
                            security: field(v, "security") == Some("true"),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    lk
}

/// The release to install: the newest LTS (nodejs.org lists newest first,
/// so it's the latest patch of the newest LTS line). Pure.
pub fn pick(rs: &[Release]) -> Option<&Release> {
    rs.iter().max_by_key(|r| {
        r.version
            .trim_start_matches('v')
            .split('.')
            .map(|x| x.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    })
}

/// What's needed before installing: the release and the build — or why not.
pub fn plan(lk: &Look) -> Result<(Release, String), String> {
    let plat = lk.plat.clone()?;
    if lk.fetch.is_empty() {
        return Err("it has neither curl nor wget to download it".into());
    }
    if let Some(e) = &lk.err {
        return Err(format!("couldn't reach nodejs.org from there ({e})"));
    }
    let r = pick(&lk.releases).ok_or("nodejs.org listed no LTS release")?;
    Ok((r.clone(), plat))
}

/// The install (`sh -s`): download, check the SHA-256, unpack, link, set
/// npm's global folder; `CCN ok <version> <node path> <npm version>` or
/// `CCN fail <step> <detail>`.
pub fn install_script(r: &Release, plat: &str) -> String {
    let v = crate::proc::sh_quote(&r.version);
    let p = crate::proc::sh_quote(plat);
    format!(
        r#"LC_ALL=C; export LC_ALL
V={v}; P={p}; B={DIST}/$V; f=node-$V-$P.tar.gz; D=$HOME/{HOME_DIR}
T=$(mktemp -d 2>/dev/null) || T=/tmp/aleph-node.$$
mkdir -p "$T"
fail() {{ printf 'CCN\tfail\t%s\t%s\n' "$1" "$2"; rm -rf "$T"; exit 1; }}
get() {{ if command -v curl >/dev/null 2>&1; then curl -fsSL --max-time 900 -o "$2" "$1"; else wget -q --timeout=60 -O "$2" "$1"; fi; }}
echo "downloading $f from nodejs.org"
get "$B/$f" "$T/$f" || fail download "$B/$f"
get "$B/SHASUMS256.txt" "$T/SHASUMS256.txt" || fail download "$B/SHASUMS256.txt"
want=$(grep "  $f\$" "$T/SHASUMS256.txt" | cut -d' ' -f1)
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$T/$f" | cut -d' ' -f1)
elif command -v shasum >/dev/null 2>&1; then got=$(shasum -a 256 "$T/$f" | cut -d' ' -f1)
else fail nosha "neither sha256sum nor shasum"; fi
[ -n "$want" ] || fail checksum "$f isn't in SHASUMS256.txt"
[ "$want" = "$got" ] || fail checksum "sha256 $got, nodejs.org says $want"
echo "sha256 checked: ${{got%"${{got#????????????????}}"}}…"
mkdir -p "$D" "$HOME/.local/bin" || fail unpack "can't create $D"
tar -xzf "$T/$f" -C "$D" || fail unpack "tar -xzf $f"
ln -sfn "$D/node-$V-$P" "$D/current" || fail unpack "ln $D/current"
for x in node npm npx corepack; do [ -e "$D/current/bin/$x" ] && ln -sf "$D/current/bin/$x" "$HOME/.local/bin/$x"; done
echo "unpacked into ~/{HOME_DIR}/node-$V-$P; node, npm and npx linked into ~/.local/bin"
PATH="$D/current/bin:$PATH"; export PATH
if ! grep -qs '^[[:space:]]*prefix[[:space:]]*=' "$HOME/.npmrc"; then
  npm config set prefix "$HOME/.local" >/dev/null 2>&1 && echo "npm's global folder: ~/.local (npm i -g needs no sudo; agents land in ~/.local/bin)"
fi
rm -rf "$T"
printf 'CCN\tok\t%s\t%s\t%s\n' "$(node --version 2>/dev/null)" "$HOME/.local/bin/node" "$(npm --version 2>/dev/null)"
"#
    )
}

/// How an install ended: Ok((version, node's path, npm's version)), or
/// Err((step, detail)) — `download`, `checksum`, `nosha`, `unpack`.
pub type Installed = Result<(String, String, String), (String, String)>;

/// [`Installed`], read from the install's output (pure; None: it didn't say).
pub fn parse_install(text: &str) -> Option<Installed> {
    for l in text.lines().rev() {
        let f: Vec<&str> = l.split('\t').collect();
        match f.as_slice() {
            ["CCN", "ok", v, path, npm, ..] => {
                return Some(Ok((v.to_string(), path.to_string(), npm.to_string())))
            }
            ["CCN", "ok", v, path] => {
                return Some(Ok((v.to_string(), path.to_string(), String::new())))
            }
            ["CCN", "fail", step, rest @ ..] => {
                return Some(Err((step.to_string(), rest.join(" "))))
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"[
{"version":"v25.1.0","date":"2026-10-20","files":["linux-x64","osx-arm64-tar"],"npm":"11.6.2","lts":false,"security":false},
{"version":"v24.11.1","date":"2026-10-14","files":["linux-x64","osx-arm64-tar"],"npm":"11.6.1","lts":"Krypton","security":true},
{"version":"v24.11.0","date":"2026-09-30","files":["linux-x64"],"npm":"11.6.0","lts":"Krypton","security":false},
{"version":"v22.21.1","date":"2026-10-14","files":["linux-x64"],"npm":"10.9.4","lts":"Jod","security":true}
]"#;

    /// What the look prints for that index (its grep, done here).
    fn look_of(index: &str, plat: &str) -> String {
        let mut s = format!("CCN\tplat\t{plat}\nCCN\tfetch\tcurl\n");
        for e in index
            .replace('{', "\n")
            .lines()
            .filter(|l| l.contains("\"lts\":\""))
            .take(3)
        {
            s.push_str(&format!("CCN\trel\t{e}\n"));
        }
        s
    }

    #[test]
    fn the_newest_lts_is_picked() {
        let lk = parse_look(&look_of(INDEX, "Linux\tx86_64\tgnu"));
        assert_eq!(lk.plat, Ok("linux-x64".into()));
        assert_eq!(lk.releases.len(), 3, "{lk:?}");
        let (r, plat) = plan(&lk).unwrap();
        assert_eq!(
            (r.version.as_str(), r.lts.as_str(), r.security),
            ("v24.11.1", "Krypton", true)
        );
        assert_eq!(plat, "linux-x64");
        assert_eq!(r.label(), "Node.js v24.11.1 (LTS \"Krypton\")");
        // never the Current line (v25), whatever the order
        let mut rs = lk.releases.clone();
        rs.reverse();
        assert_eq!(pick(&rs).unwrap().version, "v24.11.1");
    }

    #[test]
    fn platforms_and_reasons() {
        assert_eq!(platform("Darwin", "arm64", ""), Ok("darwin-arm64".into()));
        assert_eq!(
            platform("Linux", "aarch64", "gnu"),
            Ok("linux-arm64".into())
        );
        assert!(platform("Linux", "x86_64", "musl")
            .unwrap_err()
            .contains("musl"));
        assert!(platform("FreeBSD", "amd64", "")
            .unwrap_err()
            .contains("FreeBSD"));
        assert!(platform("Linux", "riscv64", "gnu").is_err());
        let lk = parse_look("CCN\tplat\tLinux\tx86_64\tgnu\nCCN\tfetch\tcurl\nCCN\terr\tcurl: (6) Could not resolve host: nodejs.org\n");
        assert!(plan(&lk).unwrap_err().contains("couldn't reach nodejs.org"));
        let lk = parse_look("CCN\tplat\tLinux\tx86_64\tgnu\nCCN\tfetch\t\n");
        assert!(plan(&lk).unwrap_err().contains("neither curl nor wget"));
        let p = crate::probe::parse("b", "CCO\tLinux x86_64\nCCT\tcurl\t/usr/bin/curl\n");
        assert!(possible(&p).is_ok());
        let p = crate::probe::parse("b", "CCO\tLinux x86_64\n");
        assert!(possible(&p).is_err());
    }

    #[test]
    fn install_results_are_read() {
        assert_eq!(
            parse_install("downloading\nCCN\tok\tv24.11.1\t/home/t/.local/bin/node\t11.6.1\n"),
            Some(Ok((
                "v24.11.1".into(),
                "/home/t/.local/bin/node".into(),
                "11.6.1".into()
            )))
        );
        assert_eq!(
            parse_install("CCN\tfail\tchecksum\tsha256 ab, nodejs.org says cd\n"),
            Some(Err((
                "checksum".into(),
                "sha256 ab, nodejs.org says cd".into()
            )))
        );
        assert_eq!(parse_install("nothing"), None);
        let s = install_script(
            &Release {
                version: "v24.11.1".into(),
                lts: "Krypton".into(),
                security: true,
            },
            "linux-x64",
        );
        assert!(
            s.contains("V=v24.11.1; P=linux-x64;") && s.contains("SHASUMS256.txt"),
            "{s}"
        );
        // it never escalates: no sudo or doas is ever run
        assert!(!s.contains("sudo ") && !s.contains("doas"), "{s}");
    }
}
