//! What's installed on a machine: one `ssh <host> sh -s` (or a local `sh -s`
//! for this machine) that reports herdr's path and version, every agent
//! executable from the registry, `herdr integration status`, the OS, GPUs and
//! the tools an agent install needs (node, npm and its global prefix, curl,
//! bash, uv, brew).
//!
//! Finding agents is more than `command -v`: the probe looks through the
//! login-shell PATH, the PATH a fresh interactive shell of yours gets (rc
//! files — where nvm and most installers add themselves, and what herdr types
//! agents into), and every folder installers use (`~/.local/bin`,
//! `~/.opencode/bin`, `~/.claude/local`, cargo, bun, deno, volta, pnpm, yarn,
//! go, the npm global prefix, every nvm/fnm node version, asdf/mise shims,
//! Homebrew, snap). Each candidate's `--version` runs in parallel, capped at a
//! few seconds, and its output is checked against the registry
//! ([`Agent::sig`], [`Agent::strict`], [`Agent::not`]) so a same-named
//! unrelated program or a broken install (`env: node: No such file…`) isn't
//! reported as the agent.
//!
//! Results are cached per host for [`TTL`] (`aleph agents --refresh`
//! forces a new look). The parser is pure and unit-tested; the probe itself
//! never installs anything.

use crate::agents::{self, Agent, Launch};
use crate::{chan, config, proc, sshcfg};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The pseudo-alias for this machine.
pub const LOCAL: &str = "local";

const T_PROBE: Duration = Duration::from_secs(45);

/// How long a probe's answer stays good.
pub const TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IState {
    /// installed and current (`v11`)
    Current(String),
    Outdated {
        have: String,
        want: String,
    },
    Missing,
    Other(String),
}

impl IState {
    /// Stable name for JSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            IState::Current(_) => "current",
            IState::Outdated { .. } => "outdated",
            IState::Missing => "not_installed",
            IState::Other(_) => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integ {
    pub name: String,
    pub state: IState,
    pub path: String,
    pub experimental: bool,
}

/// How sure we are that a found executable is the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// `--version` printed the agent's own signature (`(Claude Code)`, `codex-cli`)
    Verified,
    /// a plausible version, and nothing that says it's another program
    Plausible,
    /// it didn't answer `--version` in time, or printed no version
    Unverified,
}

impl Check {
    pub fn as_str(self) -> &'static str {
        match self {
            Check::Verified => "verified",
            Check::Plausible => "plausible",
            Check::Unverified => "unverified",
        }
    }
}

/// A registry agent found on a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub path: String,
    /// empty when it didn't say
    pub version: String,
    pub check: Check,
    /// its folder is on the PATH a shell of yours gets there — herdr types
    /// agents into one (None: we couldn't read that PATH)
    pub on_path: Option<bool>,
    /// another executable of the same name that comes first on the PATH but
    /// isn't the agent (so a shell would run that one)
    pub shadowed_by: Option<String>,
    /// this host's herdr integration for it, when herdr has one
    pub integration: Option<IState>,
}

impl Found {
    /// The folder it's in.
    pub fn dir(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |x| x.0)
    }
}

/// How herdr starts an agent in a fresh pane.
pub fn start_of(a: Option<&Agent>) -> String {
    match a.map(Agent::launch) {
        Some(Launch::Kind(k)) => format!("herdr agent start --kind {k}"),
        Some(Launch::Command(c)) => format!("herdr pane run {c}"),
        None => String::new(),
    }
}

/// An executable named like an agent that isn't it, or doesn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub id: String,
    pub path: String,
    /// the line that gave it away
    pub why: String,
    /// true: it's the agent but doesn't run; false: another program
    pub broken: bool,
}

/// A tool an install needs (`node`, `npm`, `curl`, `wget`, `bash`, `uv`, `brew`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tool {
    pub name: String,
    pub path: String,
    /// `node`: its version
    pub version: String,
    /// `npm`: its global prefix (`npm prefix -g`)
    pub prefix: String,
    /// `npm`: can we write to the global prefix without sudo
    pub writable: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct HostProbe {
    pub host: String,
    pub error: Option<String>,
    /// the user's home folder there
    pub home: String,
    /// `uname -sm`
    pub os: String,
    /// `Ubuntu 22.04.4 LTS`, `macOS 26.0`
    pub os_name: String,
    /// GPU names (`nvidia-smi`), one per GPU
    pub gpus: Vec<String>,
    /// (path, version)
    pub herdr: Option<(String, String)>,
    pub agents: Vec<Found>,
    pub skipped: Vec<Skipped>,
    pub integs: Vec<Integ>,
    pub tools: Vec<Tool>,
    /// the login shell's PATH
    pub login_path: Vec<String>,
    /// the PATH a fresh interactive shell gets (None: couldn't read it)
    pub shell_path: Option<Vec<String>>,
    /// every folder that was searched, in order
    pub searched: Vec<String>,
    /// agent executables your shell has an alias for (`alias claude=~/.claude/local/claude`)
    pub aliases: Vec<String>,
    /// agents moshi-hook there already tells the phone about (Aleph's ids)
    pub moshi: Vec<String>,
    /// its network interface and MAC address (Wake-on-LAN)
    pub mac: Option<(String, String)>,
    /// Linux: the CPU's temperature drivers (`CCK`)
    pub sensors: Option<Sensors>,
    pub took: Duration,
    /// Some when the answer came from the cache: how old it is
    pub age: Option<Duration>,
}

/// Can sessions of this agent survive a herdr server restart on that host?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persist {
    /// the integration is installed (herdr restores the agent session)
    Yes,
    /// installed but older than this herdr's version of it
    Outdated,
    /// the integration exists but isn't installed
    Missing,
    /// no herdr integration for this agent (or this herdr doesn't have it)
    None,
}

/// A Linux machine's CPU temperature drivers, as the probe saw them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sensors {
    pub vendor: String,
    /// `systemd-detect-virt`: `none` on bare metal
    pub virt: String,
    /// the hwmon drivers loaded (`nvme`, `coretemp`, …)
    pub drivers: Vec<String>,
    /// where the driver it needs is (empty: this kernel doesn't have it)
    pub module_path: String,
    pub kernel: String,
    /// `dpkg-query`'s status of `linux-modules-extra-<kernel>` (`none`: no dpkg or no such package)
    pub extra: String,
    pub apt: bool,
    /// a BMC is there (`/dev/ipmi0`): it reads the temperatures too, as root
    pub ipmi: bool,
    /// the probe said all of the above (older probes only the first three)
    pub asked: bool,
}

/// Why a machine has no CPU temperature, and what would give it one.
#[derive(Clone, Debug, PartialEq)]
pub enum CpuSensor {
    /// the driver is there, just not loaded: `modprobe` it
    Load(&'static str),
    /// the driver ships in a package that isn't installed (Ubuntu's
    /// `linux-modules-extra-<kernel>`): install it, then load it
    Install {
        module: &'static str,
        package: String,
    },
    /// this kernel has no such driver, and no package is known to have it
    Absent(&'static str),
}

impl HostProbe {
    /// A bare-metal Linux machine with no CPU temperature driver loaded: why,
    /// and the fix. None in a VM, on a Mac, or when one is loaded.
    pub fn cpu_sensor(&self) -> Option<CpuSensor> {
        let s = self.sensors.as_ref()?;
        const CPU: [&str; 5] = [
            "coretemp",
            "k10temp",
            "zenpower",
            "cpu_thermal",
            "x86_pkg_temp",
        ];
        if s.virt != "none" || s.drivers.iter().any(|n| CPU.contains(&n.as_str())) {
            return None;
        }
        let m = match s.vendor.as_str() {
            "GenuineIntel" => "coretemp",
            "AuthenticAMD" | "HygonGenuine" => "k10temp",
            _ => return None,
        };
        if !s.asked || !s.module_path.is_empty() {
            return Some(CpuSensor::Load(m));
        }
        if s.apt && !s.kernel.is_empty() && !s.extra.starts_with("install ok installed") {
            return Some(CpuSensor::Install {
                module: m,
                package: format!("linux-modules-extra-{}", s.kernel),
            });
        }
        Some(CpuSensor::Absent(m))
    }

    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
    pub fn agent(&self, id: &str) -> Option<&Found> {
        self.agents.iter().find(|f| f.id == id)
    }
    pub fn integ(&self, name: &str) -> Option<&Integ> {
        self.integs.iter().find(|i| i.name == name)
    }
    pub fn herdr_version(&self) -> Option<&str> {
        self.herdr.as_ref().map(|h| h.1.as_str())
    }
    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.name == name)
    }
    pub fn persist(&self, a: &Agent) -> Persist {
        let Some(i) = a.integration.and_then(|n| self.integ(n)) else {
            return Persist::None;
        };
        match i.state {
            IState::Current(_) => Persist::Yes,
            IState::Outdated { .. } => Persist::Outdated,
            IState::Missing => Persist::Missing,
            IState::Other(_) => Persist::None,
        }
    }
    /// Registry agents found on this host, in registry order.
    pub fn installed(&self) -> Vec<(&'static Agent, &Found)> {
        agents::ALL
            .iter()
            .filter_map(|a| self.agent(a.id).map(|f| (a, f)))
            .collect()
    }
    /// Is `dir` on the PATH a shell of yours gets there?
    pub fn on_path(&self, dir: &str) -> Option<bool> {
        on_path(&self.login_path, self.shell_path.as_deref(), dir)
    }
    /// `/home/n/.local/bin/x` → `~/.local/bin/x`.
    pub fn tilde(&self, path: &str) -> String {
        let h = self.home.trim_end_matches('/');
        if !h.is_empty() {
            if let Some(rest) = path.strip_prefix(h).and_then(|r| r.strip_prefix('/')) {
                return format!("~/{rest}");
            }
            if path == h {
                return "~".into();
            }
            return path.to_string();
        }
        crate::onboard::tilde_remote(path)
    }
    pub fn is_mac(&self) -> bool {
        self.os.starts_with("Darwin")
    }
    /// `2× NVIDIA GeForce RTX 4090`, `Apple GPU`, None without one.
    pub fn gpu_summary(&self) -> Option<String> {
        if let Some(first) = self.gpus.first() {
            let same = self.gpus.iter().all(|g| g == first);
            return Some(match (self.gpus.len(), same) {
                (1, _) => first.clone(),
                (n, true) => format!("{n}× {first}"),
                (n, false) => format!("{n} GPUs ({first}, …)"),
            });
        }
        (self.is_mac() && self.os.contains("arm64")).then(|| "Apple GPU".to_string())
    }
}

fn on_path(login: &[String], shell: Option<&[String]>, dir: &str) -> Option<bool> {
    if login.is_empty() && shell.is_none() {
        return None;
    }
    let d = dir.trim_end_matches('/');
    let has = |v: &[String]| v.iter().any(|p| p.trim_end_matches('/') == d);
    Some(has(login) || shell.is_some_and(has))
}

// ------------------------------------------------------------------ script

/// Folders installers use outside the home folder. `CCH_SYS_DIRS` replaces
/// the list (tests; an empty value searches none).
const SYS_DIRS: &str = "/opt/homebrew/bin:/usr/local/bin:/home/linuxbrew/.linuxbrew/bin:/snap/bin";

const SCRIPT: &str = r#"LC_ALL=C; export LC_ALL
unset IFS
H=$HOME
T=$(printf '\t')
D=$(mktemp -d 2>/dev/null) || D=/tmp/cch-probe.$$
mkdir -p "$D"
# the PATH a fresh shell of yours gets (login + rc files, from the PATH ssh
# gave us): herdr types agents into one. Capped: rc files can be slow.
( PATH=${O:-$PATH}; export PATH; exec "${SHELL:-/bin/sh}" -lic 'printf "\nCCP:%s\n" "$PATH"; echo CCALIASES; alias' ) </dev/null >"$D/ipath" 2>/dev/null & ip=$!
( sleep 4; kill -9 $ip ) >/dev/null 2>&1 & ik=$!
# job <name> <exe> <args…>: run it (its own folder first on the PATH, so an
# npm CLI finds its node), output to $D/v_<name>, exit code to $D/r_<name>,
# killed after 6s ($D/t_<name>)
job() { jn=$1; shift; ( PATH="${1%/*}:$PATH"; export PATH; exec "$@" ) </dev/null >"$D/v_$jn" 2>&1 & jp=$!; ( sleep 6; : >"$D/t_$jn"; kill -9 $jp ) >/dev/null 2>&1 & jk=$!; wait $jp; echo $? >"$D/r_$jn"; kill $jk 2>/dev/null; }
DIRS=; J=; n=0; h=; nn=; SEEN='
'
scan() {
  IFS=:; set -f
  for d in $1; do
    unset IFS
    for spec in @SPECS@; do
      set -- $spec; id=$1; shift
      for e in "$@"; do
        x=$d/$e
        if [ -L "$x" ] && [ ! -e "$x" ]; then
          n=$((n + 1)); printf '%s\t%s\n' "$id" "$x" >"$D/c_$n"
          echo 127 >"$D/r_a$n"; printf 'dangling symlink -> %s\n' "$(readlink "$x" 2>/dev/null)" >"$D/v_a$n"
          continue
        fi
        [ -f "$x" ] && [ -x "$x" ] || continue
        r=$(readlink -f "$x" 2>/dev/null) || r=$x
        [ -n "$r" ] || r=$x
        case $SEEN in *"
$r
"*) continue;; esac
        SEEN="$SEEN$r
"
        eval "c=\${c_$id:-0}"
        [ "$c" -lt 4 ] || continue
        eval "c_$id=$((c + 1))"
        n=$((n + 1)); printf '%s\t%s\n' "$id" "$x" >"$D/c_$n"
        job "a$n" "$x" --version &
        J="$J $!"
      done
    done
    IFS=:
  done
  unset IFS; set +f
}
# more <a:b:c>: search the folders not searched yet
more() {
  NEW=
  IFS=:; set -f
  for d in $1; do
    [ -n "$d" ] && [ -d "$d" ] || continue
    case ":$DIRS:" in *":$d:"*) continue;; esac
    DIRS="$DIRS${DIRS:+:}$d"; NEW="$NEW${NEW:+:}$d"
  done
  unset IFS; set +f
  [ -z "$NEW" ] || scan "$NEW"
}
# fnd <name> [a:b:c]: the first executable <name> in those folders (all searched ones)
fnd() { L=${2:-$DIRS}; IFS=:; set -f; for d in $L; do if [ -f "$d/$1" ] && [ -x "$d/$1" ]; then unset IFS; set +f; printf '%s\n' "$d/$1"; return 0; fi; done; unset IFS; set +f; return 1; }
# wr <path>: could we create or write it (its nearest existing folder is ours)?
wr() { w_=$1; while [ ! -e "$w_" ]; do case $w_ in */*) w_=${w_%/*}; [ -n "$w_" ] || w_=/;; *) w_=/;; esac; done; [ -w "$w_" ]; }
g=${GOPATH:-$H/go}; g=${g%%:*}
# the login PATH and the session's own, then where installers put things
X=${P-}:${O:-$PATH}
for d in "$H/.local/bin" "$H/bin" "$H/.claude/local" "$H/.opencode/bin" "$H/.cargo/bin" "$H/.bun/bin" "$H/.deno/bin" "$H/.volta/bin" "${PNPM_HOME:-$H/.local/share/pnpm}" "$H/.local/share/pnpm" "$H/Library/pnpm" "$H/.npm-global/bin" "$H/.yarn/bin" "$g/bin" "${ASDF_DATA_DIR:-$H/.asdf}/shims" "$H/.local/share/mise/shims" "${NVM_DIR:-$H/.nvm}"/versions/node/*/bin "$H"/.local/share/fnm/node-versions/*/installation/bin "$H/Library/Application Support/fnm/node-versions"/*/installation/bin "$H"/.fnm/node-versions/*/installation/bin; do X="$X:$d"; done
X="$X:${CCH_SYS_DIRS-@SYS@}"
if [ -r "$H/.npmrc" ]; then
  q=$(sed -n 's/^[[:space:]]*prefix[[:space:]]*=[[:space:]]*//p' "$H/.npmrc" | tail -n 1 | tr -d '"\r')
  case $q in "~"/*) q=$H/${q#"~/"};; "\$HOME"/*) q=$H/${q#"\$HOME/"};; "\${HOME}"/*) q=$H/${q#"\${HOME}/"};; esac
  [ -z "$q" ] || X="$X:$q/bin"
fi
[ -z "${NPM_CONFIG_PREFIX:-}" ] || X="$X:$NPM_CONFIG_PREFIX/bin"
more "$X"
wait $ip 2>/dev/null; kill $ik 2>/dev/null
IP=$(sed -n 's/^CCP://p' "$D/ipath" 2>/dev/null | tail -n 1)
# its aliases' names (Claude Code's older in-home install adds `alias claude=…`)
AL=$(sed -n '/^CCALIASES$/,$p' "$D/ipath" 2>/dev/null | sed '1d' | head -n 500 | sed -n 's/^alias //; s/^\([A-Za-z0-9_.-]*\)[= ].*/\1/p' | tr '\n' ' ')
more "$IP"
# tools as your shell finds them (its PATH first): the npm and node you'd get
if m=$(fnd npm "$IP:$DIRS"); then
  job npm "$m" prefix -g
  q=$(tr -d '\r' <"$D/v_npm" 2>/dev/null | grep '^/' | tail -n 1)
  w=ro
  if [ -n "$q" ]; then
    wr "$q/lib/node_modules" && wr "$q/bin" && w=w
    more "$q/bin"
  fi
  printf 'CCT\tnpm\t%s\t%s\t%s\n' "$m" "$q" "$w"
  [ -x "${m%/*}/node" ] && nn=${m%/*}/node
fi
[ -n "$nn" ] || nn=$(fnd node "$IP:$DIRS") || nn=
[ -z "$nn" ] || { job node "$nn" --version & J="$J $!"; }
if h=$(fnd herdr); then
  job hv "$h" --version & J="$J $!"
  job hi "$h" integration status & J="$J $!"
fi
if gs=$(fnd nvidia-smi); then job gpu "$gs" --query-gpu=name --format=csv,noheader & J="$J $!"; fi
for t in curl wget bash uv brew moshi-hook mosh-server tmux; do if x=$(fnd $t "$IP:$DIRS"); then printf 'CCT\t%s\t%s\n' "$t" "$x"; fi; done
# Moshi's own hook there: which agents it already tells the phone about
if mh=$(fnd moshi-hook "$IP:$DIRS"); then job mh "$mh" status --json & J="$J $!"; fi
[ -z "$J" ] || wait $J 2>/dev/null
[ -z "$nn" ] || printf 'CCT\tnode\t%s\t%s\n' "$nn" "$(tr -d '\r' <"$D/v_node" 2>/dev/null | grep . | head -n 1)"
printf 'CCP\tlogin\t%s\n' "${P-}"
[ -z "$IP" ] || printf 'CCP\tshell\t%s\n' "$IP"
[ -z "$AL" ] || printf 'CCL\t%s\n' "$AL"
printf 'CCD\t%s\n' "$DIRS"
i=1
while [ $i -le $n ]; do
  IFS=$T read -r id x <"$D/c_$i"
  t=0; [ -f "$D/t_a$i" ] && t=1
  printf 'CCC\t%s\t%s\t%s\t%s\t%s\t%s\n' "$i" "$id" "$x" "$(cat "$D/r_a$i" 2>/dev/null)" "$t" "$(tr -d '\r\000' <"$D/v_a$i" 2>/dev/null | grep . | head -n 3 | cut -c1-200 | tr '\t\n' ' \037')"
  i=$((i + 1))
done
if [ -n "$h" ]; then
  printf 'CCH\t%s\t%s\n' "$h" "$(tr -d '\r' <"$D/v_hv" 2>/dev/null | grep . | head -n 1)"
  tr -d '\r' <"$D/v_hi" 2>/dev/null | while IFS= read -r l; do printf 'CCI\t%s\n' "$l"; done
fi
[ "$(cat "$D/r_gpu" 2>/dev/null)" = 0 ] && grep . "$D/v_gpu" | head -n 16 | while IFS= read -r l; do printf 'CCG\t%s\n' "$l"; done
[ "$(cat "$D/r_mh" 2>/dev/null)" = 0 ] && printf 'CCM\t%s\n' "$(tr -d '\r\n\t' <"$D/v_mh" | head -c 16000)"
rm -rf "$D"
printf 'CCW\t%s\n' "$H"
# the default route's interface and its MAC (Wake-on-LAN, while it's asleep)
wn=; wm=
if [ -r /proc/net/route ]; then
  wn=$(awk 'NR > 1 && $2 == "00000000" { print $1; exit }' /proc/net/route 2>/dev/null)
  [ -n "$wn" ] && wm=$(cat "/sys/class/net/$wn/address" 2>/dev/null)
else
  wn=$(route -n get default 2>/dev/null | awk '/interface:/ { print $2; exit }')
  [ -n "$wn" ] && wm=$(ifconfig "$wn" 2>/dev/null | awk '/ether / { print $2; exit }')
fi
[ -n "$wm" ] && printf 'CCN\t%s\t%s\n' "$wn" "$wm"
printf 'CCO\t%s\n' "$(uname -sm 2>/dev/null)"
if [ -r /etc/os-release ]; then printf 'CCR\t%s\n' "$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release | head -n 1 | tr -d '"')"
elif command -v sw_vers >/dev/null 2>&1; then printf 'CCR\t%s %s\n' "$(sw_vers -productName 2>/dev/null)" "$(sw_vers -productVersion 2>/dev/null)"; fi
# Linux: the CPU's vendor, bare metal or not, the temperature drivers loaded,
# whether the one it needs is there to load (else the package that has it:
# Ubuntu keeps coretemp in linux-modules-extra), apt, a BMC (/dev/ipmi0)
if [ -d /sys/class/hwmon ] && [ -r /proc/cpuinfo ]; then
  cv=$(awk -F': *' '/^vendor_id/ { print $2; exit }' /proc/cpuinfo 2>/dev/null)
  cm=coretemp; [ "$cv" = AuthenticAMD ] && cm=k10temp
  kr=$(uname -r 2>/dev/null)
  mp=$(modinfo -n "$cm" 2>/dev/null || grep -l "/$cm.ko" "/lib/modules/$kr/modules.builtin" 2>/dev/null)
  xs=; command -v dpkg-query >/dev/null 2>&1 && xs=$(dpkg-query -W -f='${Status}' "linux-modules-extra-$kr" 2>/dev/null)
  printf 'CCK\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$cv" "$(systemd-detect-virt 2>/dev/null)" \
    "$(cat /sys/class/hwmon/*/name 2>/dev/null | tr '\n' ' ')" "$mp" "$kr" "${xs:-none}" \
    "$(command -v apt-get >/dev/null 2>&1 && echo apt)" "$([ -e /dev/ipmi0 ] && echo ipmi)"
fi
"#;

/// The probe, as a POSIX sh script for `sh -s` (multi-line is fine there).
/// Expects `O` (the PATH ssh gave the session) and `P` (the login shell's
/// PATH); [`sh`] sets both.
pub fn script() -> String {
    let mut specs = String::new();
    for a in agents::ALL {
        // ids go into variable names (`c_<id>`)
        debug_assert!(a.id.bytes().all(|b| b.is_ascii_alphanumeric()));
        specs.push_str(&format!(" \"{} {}\"", a.id, a.exes.join(" ")));
    }
    SCRIPT
        .replace("@SPECS@", specs.trim_start())
        .replace("@SYS@", SYS_DIRS)
}

// ------------------------------------------------------------------ parsing

/// The first version-looking word: `herdr 0.9.5` → `0.9.5`, `v1.2` → `1.2`,
/// `2025.09.18-7ae6800` stays whole. None without one.
pub fn find_version(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    for w in clean.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')') {
        let w = w.trim_start_matches(['v', 'V']);
        let mut parts = w.split('.');
        let first_ok = parts
            .next()
            .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let second_ok = parts
            .next()
            .is_some_and(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()));
        if first_ok && second_ok {
            let end = w
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+'))
                .unwrap_or(w.len());
            return Some(w[..end].trim_end_matches('.').to_string());
        }
    }
    None
}

/// `herdr 0.9.5` → `0.9.5`, `codex-cli 0.46.0` → `0.46.0`, `2.1.3 (Claude Code)`
/// → `2.1.3`, `v1.2` → `1.2`. Falls back to the trimmed line.
pub fn version_of(line: &str) -> String {
    find_version(line).unwrap_or_else(|| strip_ansi(line).trim().chars().take(24).collect())
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// One `herdr integration status` line:
/// `claude: outdated (v7 < v11) (/path)` · `letta (experimental): not installed (/p)` ·
/// `claude: current (v11) (/path)`.
pub fn parse_integ(line: &str) -> Option<Integ> {
    let (name, rest) = line.trim().split_once(": ")?;
    let (name, experimental) = match name.strip_suffix(" (experimental)") {
        Some(n) => (n.trim(), true),
        None => (name.trim(), false),
    };
    if name.is_empty() || name.contains(' ') {
        return None;
    }
    let mut rest = rest.trim().to_string();
    let mut path = String::new();
    if rest.ends_with(')') {
        if let Some(i) = rest.rfind(" (") {
            let p = &rest[i + 2..rest.len() - 1];
            if p.starts_with('/') || p.starts_with('~') {
                path = p.to_string();
                rest.truncate(i);
            }
        }
    }
    let low = rest.to_lowercase();
    let detail = rest
        .find('(')
        .and_then(|i| rest[i + 1..].split(')').next())
        .unwrap_or("")
        .to_string();
    let state = if low.starts_with("not installed") {
        IState::Missing
    } else if low.starts_with("outdated") {
        let (h, w) = detail.split_once('<').unwrap_or((detail.as_str(), ""));
        IState::Outdated {
            have: h.trim().to_string(),
            want: w.trim().to_string(),
        }
    } else if low.starts_with("current") || low.starts_with("installed") {
        IState::Current(detail.trim().to_string())
    } else {
        IState::Other(rest.clone())
    };
    // `error: …` and other chatter: not an integration line
    if matches!(state, IState::Other(_)) && path.is_empty() {
        return None;
    }
    Some(Integ {
        name: name.to_string(),
        state,
        path,
        experimental,
    })
}

/// What a candidate's `--version` says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// the agent's own signature (version)
    Verified(String),
    /// a version and nothing against it
    Plausible(String),
    /// no answer in time / no version (what it printed, if anything)
    Unverified(String),
    /// the agent, but it doesn't run (the error)
    Broken(String),
    /// another program (what it printed)
    Other(String),
}

/// Output that means the executable is there but doesn't run.
const BROKEN: &[&str] = &[
    "no such file or directory",
    "command not found",
    ": not found",
    "cannot execute",
    "exec format error",
    "bad interpreter",
    "error while loading shared libraries",
    "permission denied",
    "no version is set",
    "no version set",
    "cannot find module",
    "syntaxerror",
    "requires node",
    "unsupported engine",
    "dangling symlink",
];

/// Judge a candidate by its `--version` (exit code, killed after the cap,
/// first output lines).
pub fn verdict(a: &Agent, rc: Option<i32>, timed_out: bool, lines: &[String]) -> Verdict {
    let lines: Vec<String> = lines
        .iter()
        .map(|l| strip_ansi(l).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let first = lines
        .first()
        .map(|l| crate::ui::trunc(l, 120))
        .unwrap_or_default();
    if timed_out {
        return Verdict::Unverified("didn't answer --version within 6s".into());
    }
    let low = lines.join("\n").to_lowercase();
    let failed = rc.is_some_and(|c| c != 0);
    if matches!(rc, Some(126 | 127)) || (failed && BROKEN.iter().any(|m| low.contains(m))) {
        // the error line, not a banner above it
        let why = lines
            .iter()
            .find(|l| {
                let l = l.to_lowercase();
                BROKEN.iter().any(|m| l.contains(m))
            })
            .map(|l| crate::ui::trunc(l, 120))
            .unwrap_or_else(|| {
                if first.is_empty() {
                    format!("exit status {}", rc.unwrap_or(-1))
                } else {
                    first.clone()
                }
            });
        return Verdict::Broken(why);
    }
    if a.not.iter().any(|m| low.contains(m)) {
        return Verdict::Other(first);
    }
    if let Some(l) = lines.iter().find(|l| {
        let l = l.to_lowercase();
        a.sig.iter().any(|s| l.contains(s))
    }) {
        return Verdict::Verified(find_version(l).unwrap_or_default());
    }
    if a.strict && !lines.is_empty() {
        return Verdict::Other(first);
    }
    match lines.iter().find_map(|l| find_version(l)) {
        Some(v) => Verdict::Plausible(v),
        None => Verdict::Unverified(first),
    }
}

/// One `CCC` line: a candidate executable and its `--version`.
#[derive(Debug, Clone)]
struct Cand {
    n: usize,
    id: String,
    path: String,
    rc: Option<i32>,
    timed_out: bool,
    lines: Vec<String>,
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |x| x.0)
}

/// `…/.nvm/versions/node/v22.1.0/bin` → [22, 1, 0] (newest node first among
/// folders that aren't on the PATH).
fn node_version_of(dir: &str) -> Option<Vec<u64>> {
    let i = dir
        .find("/versions/node/v")
        .map(|i| i + "/versions/node/v".len())
        .or_else(|| {
            dir.find("/node-versions/v")
                .map(|i| i + "/node-versions/v".len())
        })?;
    let v = dir[i..].split('/').next()?;
    Some(v.split('.').map(|p| p.parse().unwrap_or(0)).collect())
}

/// The probe's output → what's on the host.
pub fn parse(host: &str, text: &str) -> HostProbe {
    let mut p = HostProbe {
        host: host.to_string(),
        ..Default::default()
    };
    let mut any = false;
    let mut cands: Vec<Cand> = Vec::new();
    for l in text.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        match f.as_slice() {
            // older probes: the first agent on the PATH
            ["CCA", id, path, rest @ ..] => {
                any = true;
                if agents::find(id).is_some() && !p.agents.iter().any(|x| x.id == *id) {
                    p.agents.push(Found {
                        id: id.to_string(),
                        path: path.to_string(),
                        version: rest.first().map(|v| version_of(v)).unwrap_or_default(),
                        check: Check::Plausible,
                        on_path: None,
                        shadowed_by: None,
                        integration: None,
                    });
                }
            }
            ["CCC", n, id, path, rc, to, rest @ ..] => {
                any = true;
                cands.push(Cand {
                    n: n.parse().unwrap_or(usize::MAX),
                    id: id.to_string(),
                    path: path.to_string(),
                    rc: rc.trim().parse().ok(),
                    timed_out: *to == "1",
                    lines: rest.join(" ").split('\x1f').map(str::to_string).collect(),
                });
            }
            ["CCH", path, ver, ..] => {
                any = true;
                p.herdr = Some((path.to_string(), version_of(ver)));
            }
            ["CCI", line @ ..] => {
                if let Some(i) = parse_integ(&line.join("\t")) {
                    p.integs.push(i);
                }
            }
            ["CCO", os, ..] => {
                any = true;
                p.os = os.to_string();
            }
            ["CCR", name, ..] => p.os_name = name.trim().to_string(),
            ["CCK", vendor, virt, names, rest @ ..] => {
                let g = |i: usize| {
                    rest.get(i)
                        .map(|x| x.trim().to_string())
                        .unwrap_or_default()
                };
                p.sensors = Some(Sensors {
                    vendor: vendor.trim().to_string(),
                    virt: virt.trim().to_string(),
                    drivers: names.split_whitespace().map(str::to_string).collect(),
                    module_path: g(0),
                    kernel: g(1),
                    extra: g(2),
                    apt: g(3) == "apt",
                    ipmi: g(4) == "ipmi",
                    asked: !rest.is_empty(),
                })
            }
            ["CCW", home, ..] => p.home = home.to_string(),
            ["CCG", gpu, ..] if !gpu.trim().is_empty() => p.gpus.push(gpu.trim().to_string()),
            ["CCM", json @ ..] => {
                p.moshi = crate::moshi::parse_status(&json.join("\t"), "").native()
            }
            ["CCN", iface, mac, ..] => {
                if let Some(m) = crate::wol::normalize(mac) {
                    p.mac = Some((iface.to_string(), m));
                }
            }
            ["CCP", "login", path, ..] => p.login_path = split_path(path),
            ["CCP", "shell", path, ..] => p.shell_path = Some(split_path(path)),
            ["CCD", dirs, ..] => p.searched = split_path(dirs),
            ["CCL", names, ..] => {
                p.aliases = names
                    .split_whitespace()
                    .filter(|n| agents::ALL.iter().any(|a| a.exes.contains(n)))
                    .map(str::to_string)
                    .collect()
            }
            ["CCT", name, path, rest @ ..] => {
                let mut t = Tool {
                    name: name.to_string(),
                    path: path.to_string(),
                    ..Default::default()
                };
                if *name == "node" {
                    t.version = rest.first().map(|v| version_of(v)).unwrap_or_default();
                }
                if *name == "npm" {
                    t.prefix = rest.first().unwrap_or(&"").to_string();
                    t.writable = match rest.get(1) {
                        Some(&"w") => Some(true),
                        Some(&"ro") if !t.prefix.is_empty() => Some(false),
                        _ => None,
                    };
                }
                if !p.tools.iter().any(|x| x.name == t.name) {
                    p.tools.push(t);
                }
            }
            _ => {}
        }
    }
    if !any {
        p.error = Some("no answer from the probe".into());
    }
    choose(&mut p, cands);
    for f in &mut p.agents {
        f.integration = agents::find(&f.id)
            .and_then(|a| a.integration)
            .and_then(|n| p.integs.iter().find(|i| i.name == n))
            .map(|i| i.state.clone());
    }
    // registry order
    p.agents.sort_by_key(|f| {
        agents::ALL
            .iter()
            .position(|a| a.id == f.id)
            .unwrap_or(usize::MAX)
    });
    p
}

fn split_path(s: &str) -> Vec<String> {
    s.split(':')
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect()
}

/// Per agent: rank its candidates the way a shell of yours would find them
/// (the interactive PATH, else the login PATH; then the other folders, newest
/// node first), take the first that is the agent, and note the rest.
fn choose(p: &mut HostProbe, cands: Vec<Cand>) {
    let order: Vec<String> = match &p.shell_path {
        Some(s) if !s.is_empty() => s.clone(),
        _ => p.login_path.clone(),
    };
    let pos = |path: &str| {
        let d = dir_of(path).trim_end_matches('/');
        order
            .iter()
            .position(|x| x.trim_end_matches('/') == d)
            .unwrap_or(usize::MAX)
    };
    for a in agents::ALL {
        let mut mine: Vec<&Cand> = cands.iter().filter(|c| c.id == a.id).collect();
        if mine.is_empty() {
            continue;
        }
        mine.sort_by(|x, y| {
            pos(&x.path)
                .cmp(&pos(&y.path))
                .then_with(|| {
                    node_version_of(dir_of(&y.path)).cmp(&node_version_of(dir_of(&x.path)))
                })
                .then(x.n.cmp(&y.n))
        });
        let mut chosen: Option<Found> = None;
        let mut first_bad_on_path: Option<String> = None;
        // an alias wins over the PATH in your shell
        let aliased = p.aliases.iter().any(|x| a.exes.contains(&x.as_str()));
        for c in mine {
            let v = verdict(a, c.rc, c.timed_out, &c.lines);
            let good = match &v {
                Verdict::Verified(ver) => Some((Check::Verified, ver.clone())),
                Verdict::Plausible(ver) => Some((Check::Plausible, ver.clone())),
                Verdict::Unverified(_) => Some((Check::Unverified, String::new())),
                Verdict::Broken(_) | Verdict::Other(_) => None,
            };
            match (good, &chosen) {
                (Some((check, version)), None) => {
                    let on = if aliased {
                        Some(true)
                    } else {
                        on_path(&p.login_path, p.shell_path.as_deref(), dir_of(&c.path))
                    };
                    chosen = Some(Found {
                        id: a.id.to_string(),
                        path: c.path.clone(),
                        version,
                        check,
                        on_path: on,
                        // a shell runs the first one on its PATH: that one
                        shadowed_by: if aliased {
                            None
                        } else {
                            first_bad_on_path.clone()
                        },
                        integration: None,
                    });
                }
                (Some(_), Some(_)) => {}
                (None, _) => {
                    let (why, broken) = match v {
                        Verdict::Broken(w) => (w, true),
                        Verdict::Other(w) => (w, false),
                        _ => unreachable!(),
                    };
                    // (a shell skips a dangling link and keeps looking)
                    if chosen.is_none()
                        && first_bad_on_path.is_none()
                        && pos(&c.path) != usize::MAX
                        && !why.starts_with("dangling symlink")
                    {
                        first_bad_on_path = Some(c.path.clone());
                    }
                    p.skipped.push(Skipped {
                        id: a.id.to_string(),
                        path: c.path.clone(),
                        why,
                        broken,
                    });
                }
            }
        }
        if let Some(f) = chosen {
            if !p.agents.iter().any(|x| x.id == f.id) {
                p.agents.push(f);
            }
        }
    }
}

// ------------------------------------------------------------------ running

/// Run a script with `sh -s` on `host` (ssh, BatchMode; the user's login PATH
/// via `chan::PATH_PRELUDE`) or here for [`LOCAL`] (this process's PATH — the
/// herdr aleph itself runs). `O` holds the PATH the session started
/// with and `P` the login PATH, for scripts that care.
pub fn sh(host: &str, script: &str, timeout: Duration) -> Result<proc::Out, String> {
    sh_cancel(host, script, timeout, None)
}

fn command(host: &str, script: &str) -> (Command, String) {
    let script = if host == LOCAL {
        format!("O=$PATH; P=$PATH\n{script}")
    } else {
        format!("O=$PATH\n{}\n{script}", chan::PATH_PRELUDE)
    };
    let c = if host == LOCAL {
        let mut c = Command::new("sh");
        c.arg("-s");
        c
    } else {
        let mut c = Command::new(sshcfg::ssh_bin());
        c.args(sshcfg::batch()).arg("-T").arg(host).arg("sh -s");
        c
    };
    (c, script)
}

/// [`sh`] that gives up (killing ssh) once `cancel` is set.
pub fn sh_cancel(
    host: &str,
    script: &str,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<proc::Out, String> {
    let (mut c, script) = command(host, script);
    proc::run_cancel(&mut c, timeout, Some(script.as_bytes()), cancel).map_err(|e| e.to_string())
}

/// [`sh`] with every output line handed to `on_line` as it arrives.
pub fn sh_stream(
    host: &str,
    script: &str,
    timeout: Duration,
    on_line: impl FnMut(&str),
) -> Result<proc::Out, String> {
    let (mut c, script) = command(host, script);
    proc::stream(&mut c, timeout, Some(script.as_bytes()), on_line).map_err(|e| e.to_string())
}

/// A fresh look at `host` (saved to the cache when it answered).
pub fn run(host: &str) -> HostProbe {
    run_cancel(host, None)
}

/// [`run`] that gives up once `cancel` is set.
pub fn run_cancel(host: &str, cancel: Option<&AtomicBool>) -> HostProbe {
    look(host, cancel, true)
}

fn look(host: &str, cancel: Option<&AtomicBool>, keep: bool) -> HostProbe {
    // showcase's demo world: what its machines have
    if let Some(p) = crate::showcase::probe(host) {
        return p;
    }
    let t0 = Instant::now();
    let mut p = match sh_cancel(host, &script(), T_PROBE, cancel) {
        Ok(o) => {
            let text = o.text();
            let mut p = parse(host, &text);
            if p.error.is_some() && !o.ok() {
                p.error = Some(o.reason());
            }
            if p.ok() && keep {
                save(&cache_file(host), &text);
            }
            p
        }
        Err(e) => HostProbe {
            host: host.to_string(),
            error: Some(e),
            ..Default::default()
        },
    };
    p.took = t0.elapsed();
    p
}

/// The cached answer when it's younger than [`TTL`], else a fresh look.
pub fn run_cached(host: &str, refresh: bool) -> HostProbe {
    if !refresh {
        if let Some(p) = cached(host, TTL) {
            return p;
        }
    }
    run(host)
}

/// Probe every host in parallel; results in the same order. A host that
/// doesn't answer in time comes back with an error instead of blocking the rest.
pub fn run_all(hosts: &[String]) -> Vec<HostProbe> {
    run_all_with(hosts, false, false)
}

/// [`run_all`], answering from the cache where it's fresh (unless `refresh`).
pub fn run_all_cached(hosts: &[String], refresh: bool) -> Vec<HostProbe> {
    run_all_with(hosts, true, refresh)
}

fn run_all_with(hosts: &[String], use_cache: bool, refresh: bool) -> Vec<HostProbe> {
    let (tx, rx) = mpsc::channel();
    for (i, h) in hosts.iter().enumerate() {
        let (tx, h) = (tx.clone(), h.clone());
        std::thread::spawn(move || {
            let p = if use_cache {
                run_cached(&h, refresh)
            } else {
                run(&h)
            };
            let _ = tx.send((i, p));
        });
    }
    drop(tx);
    let mut out: Vec<Option<HostProbe>> = vec![None; hosts.len()];
    let end = Instant::now() + T_PROBE + Duration::from_secs(5);
    while out.iter().any(Option::is_none) {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((i, p)) => out[i] = Some(p),
            Err(_) => break,
        }
    }
    out.into_iter()
        .enumerate()
        .map(|(i, p)| {
            p.unwrap_or_else(|| HostProbe {
                host: hosts[i].clone(),
                error: Some("timed out".into()),
                ..Default::default()
            })
        })
        .collect()
}

// ------------------------------------------------------------------ cache

/// `~/.cache/aleph/agents/<host>.probe`: the raw answer (re-parsed on
/// load, so a newer registry reads an older answer right).
pub fn cache_file(host: &str) -> PathBuf {
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.@".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    config::world_cache()
        .join("agents")
        .join(format!("{safe}.probe"))
}

fn save(f: &std::path::Path, text: &str) {
    let _ = config::write_atomic(f, text.as_bytes());
}

/// The cached answer for `host` if it's younger than `max_age`.
pub fn cached(host: &str, max_age: Duration) -> Option<HostProbe> {
    if let Some(p) = crate::showcase::probe(host) {
        return p.ok().then_some(p);
    }
    load(&cache_file(host), host, max_age)
}

fn load(f: &std::path::Path, host: &str, max_age: Duration) -> Option<HostProbe> {
    let age = std::fs::metadata(f)
        .and_then(|m| m.modified())
        .ok()?
        .elapsed()
        .unwrap_or_default();
    if age > max_age {
        return None;
    }
    let text = std::fs::read_to_string(f).ok()?;
    let mut p = parse(host, &text);
    p.age = Some(age);
    p.ok().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    fn ag(id: &str) -> &'static Agent {
        agents::find(id).unwrap()
    }

    #[test]
    fn cpu_temperature_driver() {
        let p = |vendor: &str, virt: &str, names: &str, rest: &str| {
            parse(
                "q7x",
                &format!("CCO\tLinux x86_64\nCCK\t{vendor}\t{virt}\t{names}{rest}\n"),
            )
        };
        // q7x: bare-metal Xeon, only the nvme sensors, coretemp not in this
        // kernel (Ubuntu keeps it in linux-modules-extra, not installed), a BMC
        let q7x = p(
            "GenuineIntel",
            "none",
            "nvme nvme nvme ",
            "\t\t6.5.0-41-generic\tunknown ok not-installed\tapt\tipmi",
        );
        assert_eq!(
            q7x.cpu_sensor(),
            Some(CpuSensor::Install {
                module: "coretemp",
                package: "linux-modules-extra-6.5.0-41-generic".into()
            })
        );
        assert!(q7x.sensors.as_ref().unwrap().ipmi);
        // the driver is there, only not loaded
        let there = "\t/lib/modules/6.8/kernel/drivers/hwmon/coretemp.ko.zst\t6.8.0\tnone\tapt\t";
        assert_eq!(
            p("GenuineIntel", "none", "nvme ", there).cpu_sensor(),
            Some(CpuSensor::Load("coretemp"))
        );
        // the package is installed, the kernel still has none: nothing to load
        let none = "\t\t6.5.0\tinstall ok installed\tapt\t";
        assert_eq!(
            p("AuthenticAMD", "none", "nvme ", none).cpu_sensor(),
            Some(CpuSensor::Absent("k10temp"))
        );
        assert_eq!(
            p("GenuineIntel", "none", "acpitz coretemp ", "").cpu_sensor(),
            None
        );
        assert_eq!(
            p("GenuineIntel", "kvm", "", "").cpu_sensor(),
            None,
            "a VM has none to load"
        );
        assert_eq!(parse("m", "CCO\tDarwin arm64\n").cpu_sensor(), None);
        // an older probe (three fields): load it
        assert_eq!(
            p("GenuineIntel", "none", "nvme ", "").cpu_sensor(),
            Some(CpuSensor::Load("coretemp"))
        );
    }

    #[test]
    fn moshi_hook_there() {
        let p = parse(
            "b31",
            "CCO\tLinux x86_64\nCCT\tmoshi-hook\t/home/n/.local/bin/moshi-hook\nCCM\t{\"paired\":true,\"hooks\":[{\"target\":\"claude\",\"status\":\"current\"},{\"target\":\"antigravity\",\"status\":\"current\"},{\"target\":\"codex\",\"status\":\"not_found\"}]}\n",
        );
        assert!(p.tool("moshi-hook").is_some());
        assert_eq!(p.moshi, vec!["claude", "agy"]);
        assert!(parse("b31", "CCO\tLinux x86_64\nCCM\tnot json\n")
            .moshi
            .is_empty());
    }

    #[test]
    fn versions() {
        assert_eq!(version_of("herdr 0.9.5"), "0.9.5");
        assert_eq!(version_of("2.1.200 (Claude Code)"), "2.1.200");
        assert_eq!(version_of("codex-cli 0.46.0"), "0.46.0");
        assert_eq!(version_of("v1.2.3"), "1.2.3");
        assert_eq!(version_of("\x1b[1mopencode\x1b[0m 0.15.2"), "0.15.2");
        assert_eq!(version_of("1.0.0-beta.3"), "1.0.0-beta.3");
        assert_eq!(version_of("2025.09.18-7ae6800"), "2025.09.18-7ae6800");
        assert_eq!(version_of("no version here"), "no version here");
        assert_eq!(find_version("nvidia 1."), None);
        assert_eq!(find_version("sh: 1: node: not found"), None);
    }

    #[test]
    fn version_plausibility() {
        // the agent's own signature
        assert_eq!(
            verdict(
                ag("claude"),
                Some(0),
                false,
                &lines(&["2.1.306 (Claude Code)"])
            ),
            Verdict::Verified("2.1.306".into())
        );
        assert_eq!(
            verdict(ag("codex"), Some(0), false, &lines(&["codex-cli 0.149.1"])),
            Verdict::Verified("0.149.1".into())
        );
        // strict: a `claude` that prints something else isn't Claude Code
        assert!(matches!(
            verdict(
                ag("claude"),
                Some(0),
                false,
                &lines(&["claude 1.0 - a clause linter"])
            ),
            Verdict::Other(_)
        ));
        // non-strict agents print a bare version
        assert_eq!(
            verdict(ag("opencode"), Some(0), false, &lines(&["1.18.14"])),
            Verdict::Plausible("1.18.14".into())
        );
        assert_eq!(
            verdict(
                ag("crush"),
                Some(0),
                false,
                &lines(&["crush version v0.10.4"])
            ),
            Verdict::Verified("0.10.4".into())
        );
        // same-named programs
        assert!(matches!(
            verdict(ag("pi"), Some(0), false, &lines(&["pi (cln) 1.3.6"])),
            Verdict::Other(_)
        ));
        assert!(matches!(
            verdict(
                ag("hermes"),
                Some(0),
                false,
                &lines(&[
                    "LLVM (http://llvm.org/):",
                    "  LLVH version 8.0.0svn",
                    "Hermes JavaScript compiler"
                ])
            ),
            Verdict::Other(_)
        ));
        // installed but broken
        assert_eq!(
            verdict(
                ag("gemini"),
                Some(127),
                false,
                &lines(&["/usr/bin/env: 'node': No such file or directory"])
            ),
            Verdict::Broken("/usr/bin/env: 'node': No such file or directory".into())
        );
        assert!(matches!(
            verdict(
                ag("codex"),
                Some(126),
                false,
                &lines(&["No version is set for command codex"])
            ),
            Verdict::Broken(_)
        ));
        assert!(matches!(
            verdict(ag("qwen"), Some(1), false, &lines(&["file:///x.js:3", "SyntaxError: Unexpected token '?'"])),
            Verdict::Broken(w) if w.starts_with("SyntaxError")
        ));
        assert!(matches!(
            verdict(
                ag("claude"),
                Some(127),
                false,
                &lines(&["dangling symlink -> /x/versions/2.0"])
            ),
            Verdict::Broken(_)
        ));
        // slow or silent: still installed, version unknown
        assert!(matches!(
            verdict(ag("claude"), Some(137), true, &[]),
            Verdict::Unverified(_)
        ));
        assert!(matches!(
            verdict(ag("amp"), Some(0), false, &[]),
            Verdict::Unverified(_)
        ));
        // an error exit without a known failure isn't proof it's broken
        assert!(matches!(
            verdict(
                ag("devin"),
                Some(1),
                false,
                &lines(&["error: unexpected argument '--version'"])
            ),
            Verdict::Unverified(_)
        ));
    }

    #[test]
    fn integration_lines() {
        let i =
            parse_integ("claude: outdated (v7 < v11) (/home/u/.claude/hooks/herdr-agent-state.sh)")
                .unwrap();
        assert_eq!(i.name, "claude");
        assert_eq!(
            i.state,
            IState::Outdated {
                have: "v7".into(),
                want: "v11".into()
            }
        );
        assert_eq!(i.path, "/home/u/.claude/hooks/herdr-agent-state.sh");
        let i = parse_integ("claude: current (v11) (/h/.claude/x.sh)").unwrap();
        assert_eq!(i.state, IState::Current("v11".into()));
        let i = parse_integ("letta (experimental): not installed (/h/.letta/x.sh)").unwrap();
        assert!(i.experimental && i.state == IState::Missing && i.name == "letta");
        let i = parse_integ("antigravity-cli: not installed (/h/.gemini/x.sh)").unwrap();
        assert_eq!(i.name, "antigravity-cli");
        assert!(parse_integ("error: something broke badly").is_none());
        assert!(parse_integ("").is_none());
    }

    #[test]
    fn probe_output() {
        let text = "noise from a login shell\n\
            CCA\tclaude\t/home/n/.local/bin/claude\t2.1.200 (Claude Code)\n\
            CCA\tcodex\t/usr/bin/codex\tcodex-cli 0.46.0\n\
            CCA\tbogus\t/usr/bin/bogus\t1.0\n\
            CCH\t/home/n/.local/bin/herdr\therdr 0.9.5\n\
            CCI\tclaude: current (v11) (/home/n/.claude/hooks/herdr-agent-state.sh)\n\
            CCI\tcodex: not installed (/home/n/.codex/herdr-agent-state.sh)\n\
            CCO\tLinux x86_64\n";
        let p = parse("b31", text);
        assert!(p.ok());
        assert_eq!(p.herdr_version(), Some("0.9.5"));
        assert_eq!(p.agents.len(), 2);
        assert_eq!(p.agent("codex").unwrap().version, "0.46.0");
        assert_eq!(p.os, "Linux x86_64");
        let claude = agents::find("claude").unwrap();
        let codex = agents::find("codex").unwrap();
        let amp = agents::find("amp").unwrap();
        let cursor = agents::find("cursor").unwrap();
        assert_eq!(p.persist(claude), Persist::Yes);
        assert_eq!(p.persist(codex), Persist::Missing);
        assert_eq!(p.persist(amp), Persist::None);
        // this herdr doesn't list a cursor integration
        assert_eq!(p.persist(cursor), Persist::None);
        assert_eq!(p.installed().len(), 2);
        assert_eq!(p.agent("codex").unwrap().integration, Some(IState::Missing));
        assert!(parse("x", "ssh: connect refused").error.is_some());
    }

    /// What b31 looks like: claude and codex in ~/.local/bin and opencode in
    /// ~/.opencode/bin, neither on the login PATH, both on the shell's.
    const B31: &str = "CCT\tcurl\t/usr/bin/curl\n\
        CCT\tbash\t/usr/bin/bash\n\
        CCP\tlogin\t/home/n/.cargo/bin:/usr/local/bin:/usr/bin:/bin\n\
        CCP\tshell\t/home/n/.opencode/bin:/home/n/.bun/bin:/home/n/.local/bin:/home/n/.cargo/bin:/usr/local/bin:/usr/bin:/bin\n\
        CCD\t/home/n/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/home/n/.local/bin:/home/n/.opencode/bin\n\
        CCC\t1\tclaude\t/home/n/.local/bin/claude\t0\t0\t2.1.306 (Claude Code)\n\
        CCC\t2\tcodex\t/home/n/.local/bin/codex\t0\t0\tcodex-cli 0.147.0\n\
        CCC\t3\topencode\t/home/n/.opencode/bin/opencode\t0\t0\t1.18.15\n\
        CCH\t/home/n/.local/bin/herdr\therdr 0.9.5\n\
        CCI\tclaude: current (v11) (/home/n/.claude/hooks/x.sh)\n\
        CCI\tcodex: not installed (/home/n/.codex/x.sh)\n\
        CCI\topencode: not installed (/home/n/.config/opencode/x.js)\n\
        CCG\tNVIDIA GeForce RTX 4090\n\
        CCG\tNVIDIA GeForce RTX 4090\n\
        CCO\tLinux x86_64\n\
        CCR\tUbuntu 22.04.4 LTS\n";

    #[test]
    fn candidates_and_paths() {
        let p = parse("b31", B31);
        assert!(p.ok(), "{:?}", p.error);
        let ids: Vec<&str> = p.agents.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["claude", "codex", "opencode"]);
        let oc = p.agent("opencode").unwrap();
        assert_eq!(oc.version, "1.18.15");
        assert_eq!(oc.check, Check::Plausible);
        assert_eq!(oc.on_path, Some(true), "rc files put it on the PATH");
        assert_eq!(
            start_of(agents::find("opencode")),
            "herdr agent start --kind opencode"
        );
        assert_eq!(oc.integration, Some(IState::Missing));
        assert_eq!(p.agent("claude").unwrap().check, Check::Verified);
        assert_eq!(p.os_name, "Ubuntu 22.04.4 LTS");
        assert_eq!(
            p.gpu_summary().as_deref(),
            Some("2× NVIDIA GeForce RTX 4090")
        );
        assert_eq!(p.tool("curl").unwrap().path, "/usr/bin/curl");
        assert!(p.tool("npm").is_none() && p.tool("node").is_none());
        assert_eq!(p.searched.len(), 6);
        // without the shell's PATH, only the login PATH counts
        let no_shell: String = B31
            .lines()
            .filter(|l| !l.starts_with("CCP\tshell"))
            .map(|l| format!("{l}\n"))
            .collect();
        let p = parse("b31", &no_shell);
        assert_eq!(p.agent("opencode").unwrap().on_path, Some(false));
        // neither PATH known: can't tell
        let p = parse(
            "b31",
            "CCC\t1\tcodex\t/x/codex\t0\t0\tcodex-cli 1.0\nCCO\tLinux\n",
        );
        assert_eq!(p.agent("codex").unwrap().on_path, None);
    }

    #[test]
    fn impostors_broken_and_ranking() {
        let text = "CCP\tlogin\t/usr/bin:/bin\n\
            CCP\tshell\t/usr/bin:/bin:/home/n/.nvm/versions/node/v22.12.0/bin\n\
            CCC\t1\tpi\t/usr/bin/pi\t0\t0\tpi (cln) 1.3.6\n\
            CCC\t2\tpi\t/home/n/.nvm/versions/node/v22.12.0/bin/pi\t0\t0\t0.83.0\n\
            CCC\t3\tgemini\t/home/n/.local/bin/gemini\t127\t0\t/usr/bin/env: 'node': No such file or directory\n\
            CCC\t4\tcodex\t/home/n/.nvm/versions/node/v18.20.0/bin/codex\t0\t0\tcodex-cli 0.40.0\n\
            CCC\t5\tcodex\t/home/n/.nvm/versions/node/v20.11.0/bin/codex\t0\t0\tcodex-cli 0.46.0\n\
            CCC\t6\tclaude\t/usr/bin/claude\t0\t1\t\n\
            CCO\tLinux x86_64\n";
        let p = parse("dex42", text);
        // the real pi, though a shell finds the CLN one first
        let pi = p.agent("pi").unwrap();
        assert_eq!(pi.version, "0.83.0");
        assert_eq!(pi.shadowed_by.as_deref(), Some("/usr/bin/pi"));
        assert!(p.skipped.iter().any(|s| s.id == "pi" && !s.broken));
        // broken: not installed for our purposes, but reported
        assert!(p.agent("gemini").is_none());
        let g = p.skipped.iter().find(|s| s.id == "gemini").unwrap();
        assert!(g.broken && g.why.contains("No such file"));
        // neither nvm folder is on the PATH: the newest node wins
        let c = p.agent("codex").unwrap();
        assert_eq!(c.version, "0.46.0");
        assert_eq!(c.on_path, Some(false));
        // timed out: installed, version unknown
        let cl = p.agent("claude").unwrap();
        assert_eq!((cl.check, cl.version.as_str()), (Check::Unverified, ""));
    }

    #[test]
    fn broken_first_on_the_path_shadows_aliases_reach() {
        let text = "CCP\tlogin\t/a:/b\n\
            CCC\t1\tgemini\t/a/gemini\t127\t0\tenv: node: No such file or directory\n\
            CCC\t2\tgemini\t/b/gemini\t0\t0\t0.9.0\n\
            CCC\t3\tqwen\t/a/qwen\t127\t0\tdangling symlink -> /gone\n\
            CCC\t4\tqwen\t/b/qwen\t0\t0\t0.1.0\n\
            CCC\t5\tclaude\t/h/.claude/local/claude\t0\t0\t1.0.80 (Claude Code)\n\
            CCL\tll gs claude la\n\
            CCO\tLinux\n";
        let p = parse("x", text);
        // a shell runs the broken /a/gemini first
        assert_eq!(
            p.agent("gemini").unwrap().shadowed_by.as_deref(),
            Some("/a/gemini")
        );
        // but skips a dangling link
        assert!(p.agent("qwen").unwrap().shadowed_by.is_none());
        // ~/.claude/local isn't on the PATH, but `claude` is an alias for it
        assert_eq!(p.aliases, vec!["claude".to_string()]);
        assert_eq!(p.agent("claude").unwrap().on_path, Some(true));
    }

    #[test]
    fn npm_and_node_tools() {
        let p = parse(
            "x",
            "CCT\tnpm\t/h/.nvm/versions/node/v22.12.0/bin/npm\t/h/.nvm/versions/node/v22.12.0\tw\n\
             CCT\tnode\t/h/.nvm/versions/node/v22.12.0/bin/node\tv22.12.0\n\
             CCO\tLinux\n",
        );
        let npm = p.tool("npm").unwrap();
        assert_eq!(npm.prefix, "/h/.nvm/versions/node/v22.12.0");
        assert_eq!(npm.writable, Some(true));
        assert_eq!(p.tool("node").unwrap().version, "22.12.0");
        let p = parse("x", "CCT\tnpm\t/usr/bin/npm\t/usr\tro\nCCO\tLinux\n");
        assert_eq!(p.tool("npm").unwrap().writable, Some(false));
        let p = parse("x", "CCT\tnpm\t/usr/bin/npm\t\tro\nCCO\tLinux\n");
        assert_eq!(p.tool("npm").unwrap().writable, None, "no prefix: unknown");
        let mac = parse("m", "CCO\tDarwin arm64\nCCR\tmacOS 26.0\n");
        assert_eq!(mac.gpu_summary().as_deref(), Some("Apple GPU"));
    }

    #[test]
    fn script_is_posix_and_complete() {
        let s = script();
        assert!(s.contains("herdr\" integration status") || s.contains("integration status"));
        assert!(s.contains("\"cursor cursor-agent\""));
        assert!(s.contains("LC_ALL=C"));
        assert!(!s.contains("@SPECS@") && !s.contains("@SYS@"));
        for d in [
            ".local/bin",
            ".opencode/bin",
            ".claude/local",
            ".cargo/bin",
            ".bun/bin",
            ".deno/bin",
            ".volta/bin",
            ".local/share/pnpm",
            "Library/pnpm",
            ".npm-global/bin",
            "/versions/node/*/bin",
            "fnm/node-versions/*/installation/bin",
            "Application Support/fnm",
            "/shims",
            "mise/shims",
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/snap/bin",
            "prefix -g",
        ] {
            assert!(s.contains(d), "searches {d}");
        }
        for bashism in ["[[ ", "local ", "pipefail", "$'", "<<<", "function "] {
            assert!(!s.contains(bashism), "bashism in the probe: {bashism}");
        }
    }

    fn write_exe(path: &std::path::Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The real script, run with `sh` against a made-up home: finds agents in
    /// folders that aren't on the PATH, tells impostors and broken installs
    /// apart, reads the npm prefix and the shell's PATH.
    #[test]
    fn discovery_in_a_fake_home() {
        let home = std::env::temp_dir().join(format!("cch-probe-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let h = |p: &str| home.join(p);
        write_exe(
            &h(".nvm/versions/node/v20.11.0/bin/codex"),
            "echo codex-cli 0.46.0",
        );
        write_exe(
            &h(".nvm/versions/node/v18.2.0/bin/codex"),
            "echo codex-cli 0.40.0",
        );
        write_exe(&h(".opencode/bin/opencode"), "echo 1.18.15");
        write_exe(&h(".claude/local/claude"), "echo '2.1.306 (Claude Code)'");
        // an unrelated `pi` on the PATH, the real one in ~/.bun/bin
        write_exe(&h("sysbin/pi"), "echo 'pi (cln) 1.3.6'");
        write_exe(&h(".bun/bin/pi"), "echo 0.83.0");
        // a broken npm install: its interpreter is gone
        std::fs::create_dir_all(h(".local/bin")).unwrap();
        write_exe(&h(".local/bin/gemini"), "exit 0");
        std::fs::write(
            h(".local/bin/gemini"),
            "#!/nonexistent/node\nconsole.log('0.9.0')\n",
        )
        .unwrap();
        // npm with its own prefix
        write_exe(
            &h("npmbin/npm"),
            &format!(
                "[ \"$1 $2\" = 'prefix -g' ] && echo {}",
                h("npmprefix").display()
            ),
        );
        write_exe(&h("npmprefix/bin/amp"), "echo 0.0.1760000000-g1234");
        // the shell's rc puts ~/rcbin on the PATH, and has an alias for claude
        write_exe(&h("rcbin/crush"), "echo crush version v0.10.4");
        write_exe(
            &h("fakesh"),
            &format!(
                "PATH={}:$PATH; export PATH; shift; alias() {{ echo \"alias claude='$HOME/.claude/local/claude'\"; echo \"alias ll='ls -l'\"; }}; eval \"$@\"",
                h("rcbin").display()
            ),
        );
        let path = format!(
            "{}:{}:/usr/bin:/bin",
            h("sysbin").display(),
            h("npmbin").display()
        );
        let out = proc::run(
            Command::new("sh")
                .arg("-s")
                .env_clear()
                .env("HOME", &home)
                .env("PATH", &path)
                .env("SHELL", h("fakesh"))
                .env("CCH_SYS_DIRS", ""),
            Duration::from_secs(30),
            Some(format!("O=$PATH; P=$PATH\n{}", script()).as_bytes()),
        )
        .unwrap();
        let text = out.text();
        let p = parse("t", &text);
        let _ = std::fs::remove_dir_all(&home);
        assert!(p.ok(), "{text}");
        let get = |id: &str| p.agent(id).unwrap_or_else(|| panic!("{id} in {text}"));
        assert_eq!(get("codex").version, "0.46.0", "newest nvm node");
        assert!(get("codex").path.contains("v20.11.0"));
        assert_eq!(get("opencode").version, "1.18.15");
        assert_eq!(get("claude").check, Check::Verified);
        assert!(get("claude").path.ends_with(".claude/local/claude"));
        assert_eq!(get("claude").on_path, Some(true), "via the alias: {text}");
        assert_eq!(get("pi").version, "0.83.0");
        assert!(get("pi").path.ends_with(".bun/bin/pi"));
        // ~/.bun/bin isn't on the PATH, so a shell would run the CLN one
        assert_eq!(get("pi").on_path, Some(false));
        assert!(get("pi")
            .shadowed_by
            .as_deref()
            .is_some_and(|s| s.ends_with("sysbin/pi")));
        assert!(p.skipped.iter().any(|s| s.id == "pi" && !s.broken));
        assert!(p.agent("gemini").is_none());
        assert!(
            p.skipped.iter().any(|s| s.id == "gemini" && s.broken),
            "{text}"
        );
        assert_eq!(
            get("amp").version,
            "0.0.1760000000-g1234",
            "npm prefix searched"
        );
        let npm = p.tool("npm").unwrap();
        assert!(npm.prefix.ends_with("npmprefix") && npm.writable == Some(true));
        assert_eq!(get("crush").on_path, Some(true), "the shell's PATH counts");
        assert_eq!(get("opencode").on_path, Some(false));
        assert!(p
            .shell_path
            .as_ref()
            .unwrap()
            .iter()
            .any(|d| d.ends_with("rcbin")));
    }

    #[test]
    fn local_probe_runs() {
        // runs the real script with sh here; must answer even with nothing installed
        let p = look(LOCAL, None, false);
        assert!(p.ok(), "{:?}", p.error);
        assert!(!p.os.is_empty());
        assert!(!p.login_path.is_empty());
    }

    #[test]
    fn cache_round_trip() {
        let f = std::env::temp_dir()
            .join(format!("cch-probe-cache-{}", std::process::id()))
            .join("b31.probe");
        save(&f, B31);
        let p = load(&f, "b31", TTL).expect("fresh");
        assert!(p.age.is_some() && p.agent("opencode").is_some());
        std::thread::sleep(Duration::from_millis(20));
        assert!(load(&f, "b31", Duration::from_millis(1)).is_none(), "stale");
        save(&f, "ssh: nope");
        assert!(
            load(&f, "b31", TTL).is_none(),
            "a failed answer isn't served"
        );
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
        assert!(cache_file("user@h:22").ends_with("agents/user@h_22.probe"));
    }
}
