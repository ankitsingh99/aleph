#!/usr/bin/env bash
# install.sh — build Aleph and add `al` (or --name) to ~/.zshrc.
#
#   ./install.sh [default-ssh-host] [--no-build] [--no-completions] [--name <fn>] [--wrap-claude] [--wraps <agent>]
#                [--showcase] [--raycast] [--defaults] [--yes] [--no-pull]
#
# First it pulls the newest source (a fast-forward of this checkout's branch;
# --no-pull skips it). That's optional: offline, local changes or no upstream
# branch, it says so and builds what's here.
# What it needs and doesn't find, it offers to install first (a yes each;
# --yes or --defaults say yes): a C linker (Xcode's command line tools on
# macOS), Rust (rustup, as you — no sudo) and herdr (Homebrew, else herdr's
# own script into ~/.local/bin).
#
# --showcase builds with showcase mode (recordings, posters, the demo world;
# releases leave it out). --raycast also builds the Raycast extension
# (extras/raycast-extension: needs Node.js and npm). --defaults skips the
# questions: setup's quick start installs what this computer is missing
# (herdr, an agent and its herdr integration, the daemon, …) and opens Aleph.
# `aleph` is the binary; `al` is the short shell function. --wrap-claude also
# routes `claude` through Aleph: whatever isn't an Aleph command (Claude Code's
# own flags and subcommands, and every call inside a herdr pane) goes on to
# Claude Code untouched. --wraps <agent> does the same for another agent's
# program (`--wraps codex` defines `codex()`).
#
# Re-runnable. It replaces its own block in ~/.zshrc after taking a backup.
# It also replaces the pre-rename `# >>> claude-herdr >>>` block: the old
# function names keep working as aliases for a release, and an old `claude()`
# wrapper stays a wrapper.
set -euo pipefail
cd "$(dirname "$0")"

ARGS=("$@")
HOST="" BUILD=1 COMP=1 NAME=al WRAP="" FEATURES="" RAYCAST=0 DEFAULTS=0 YES=0 PULL=1
while (( $# )); do
  case $1 in
    --no-pull) PULL=0 ;;
    --no-build) BUILD=0 ;;
    --no-completions) COMP=0 ;;
    --name) NAME=${2:?--name needs a function name}; shift ;;
    --wrap-claude) WRAP=claude ;;
    --wraps) WRAP=${2:?--wraps needs an agent program (claude, codex, …)}; shift ;;
    --showcase) FEATURES=showcase ;;
    --raycast) RAYCAST=1 ;;
    --defaults) DEFAULTS=1; YES=1 ;;
    -y|--yes) YES=1 ;;
    -h|--help) sed -n '2,29p' "$0"; exit 0 ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) HOST=$1 ;;
  esac
  shift
done
[[ $NAME =~ ^[A-Za-z_][A-Za-z0-9_-]*$ ]] || { echo "bad function name: $NAME" >&2; exit 2; }

# ---- what it needs: a yes for each install (--yes / --defaults: yes)
say() { printf '\033[1m%s\033[0m %s\n' "ℵ" "$*"; }
ask() { # ask "question" → 0 for yes
  (( YES )) && return 0
  [[ -t 0 ]] || { echo "  (no terminal to ask: re-run with --yes to install it)" >&2; return 1; }
  local a; read -r -p "  $1 [Y/n] " a; [[ -z $a || $a == [Yy]* ]]
}
OS=$(uname -s)

# ---- the newest source first: optional, never in the way
if (( PULL )) && [[ -d .git ]] && command -v git >/dev/null; then
  if ! git rev-parse --abbrev-ref --symbolic-full-name '@{u}' >/dev/null 2>&1; then
    echo "  (no upstream branch here: building this checkout as it is)"
  else
    before=$(git rev-parse HEAD 2>/dev/null || true)
    self_before=$(git hash-object install.sh 2>/dev/null || true)
    say "Pulling the newest source…"
    # no credential prompts; a fast-forward or nothing
    if out=$(GIT_TERMINAL_PROMPT=0 git pull --ff-only --quiet 2>&1); then
      after=$(git rev-parse HEAD 2>/dev/null || true)
      if [[ $before != "$after" ]]; then
        echo "  up to $(git log -1 --format='%h %s' 2>/dev/null)"
        # a newer install.sh: run that one instead (once)
        if [[ $(git hash-object install.sh 2>/dev/null) != "$self_before" ]]; then
          exec "$0" ${ARGS[@]+"${ARGS[@]}"} --no-pull
        fi
      else
        echo "  already up to date"
      fi
    else
      echo "  couldn't pull (${out%%$'\n'*}) — building this checkout as it is" >&2
    fi
  fi
fi
if (( BUILD )) && ! command -v cargo >/dev/null && [[ -x $HOME/.cargo/bin/cargo ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"   # rustup's, not on this shell's PATH yet
fi
if (( BUILD )); then
  # a C linker: Rust links with the system's
  if [[ $OS == Darwin ]] && ! xcode-select -p >/dev/null 2>&1; then
    say "Rust needs Apple's command line tools (a C linker) to build Aleph."
    if ask "Install them now? (macOS opens its own installer)"; then
      xcode-select --install 2>/dev/null || true
      echo "  When that installer finishes, run ./install.sh again." >&2
    fi
    exit 1
  elif [[ $OS != Darwin ]] && ! command -v cc >/dev/null; then
    say "Rust needs a C compiler and linker (cc) to build Aleph."
    echo "  Install your system's build tools, then run ./install.sh again:" >&2
    echo "    Debian/Ubuntu: sudo apt install build-essential   Fedora: sudo dnf install gcc   Arch: sudo pacman -S base-devel" >&2
    exit 1
  fi
  if ! command -v cargo >/dev/null; then
    say "Rust isn't installed — Aleph is built from source with it."
    echo "  rustup installs it as you, into ~/.rustup and ~/.cargo (no sudo):"
    echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal"
    if ask "Install Rust now?"; then
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path \
        || { echo "error: rustup failed — see https://rustup.rs" >&2; exit 1; }
      export PATH="$HOME/.cargo/bin:$PATH"
      # ~/.cargo/bin on the PATH for new shells (rustup's own env file)
      for rc in "${ZDOTDIR:-$HOME}/.zshrc" "$HOME/.bashrc"; do
        [[ -f $rc || $rc == *zshrc ]] && ! grep -qs '.cargo/env' "$rc" && echo '. "$HOME/.cargo/env"' >> "$rc"
      done
    else
      echo "error: cargo is needed to build Aleph (https://rustup.rs), or use --no-build" >&2; exit 1
    fi
  fi
fi
if ! command -v herdr >/dev/null && [[ ! -x $HOME/.local/bin/herdr ]]; then
  say "herdr isn't installed — the agents run in it (herdr.dev)."
  if command -v brew >/dev/null; then how="brew install herdr"; else how="curl -fsSL https://herdr.dev/install.sh | sh"; fi
  echo "    $how"
  if ask "Install herdr now?"; then
    if [[ $how == brew* ]]; then brew install herdr; else curl -fsSL https://herdr.dev/install.sh | sh; fi \
      || echo "warning: herdr didn't install — aleph setup offers it again" >&2
  else
    echo "  aleph setup offers it again (step 1)."
  fi
fi

if (( BUILD )); then
  command -v cargo >/dev/null || { echo "cargo not found — install Rust (https://rustup.rs) or use --no-build" >&2; exit 1; }
  # cargo's own lines (every crate, where an old copy came from) only when it fails
  echo "  building Aleph (a few minutes the first time)…"
  if [[ -n $FEATURES ]]; then
    ALEPH_SRC="$PWD" cargo install --quiet --locked --path . --features "$FEATURES"
  else
    ALEPH_SRC="$PWD" cargo install --quiet --locked --path .
  fi || { echo "error: the build failed (cargo's output is above)" >&2; exit 1; }
  echo "✓ built $("${CARGO_HOME:-$HOME/.cargo}/bin/aleph" version 2>/dev/null | head -n 1 | sed 's/^[^a-z]*//')"
fi
BIN=$(command -v aleph || true)
[[ -n $BIN ]] || BIN="${CARGO_HOME:-$HOME/.cargo}/bin/aleph"
[[ -x $BIN ]] || { echo "aleph not found (expected $BIN)" >&2; exit 1; }
# the old name keeps working for scripts, Raycast and the old service
OLDBIN="$(dirname "$BIN")/claude-herdr"
if [[ -f $OLDBIN && ! -L $OLDBIN ]]; then ln -sf aleph "$OLDBIN"; fi

# ---- `aleph` on your PATH, so it runs as is. Without sudo when it can be: a
# link in a folder already on your PATH that you can write to works at once;
# root's /usr/local/bin only with a yes (and your password); else a PATH line
# in your shell's rc file, for new shells.
BINDIR=$(dirname "$BIN")
on_path() { case ":$PATH:" in *":$1:"*) return 0 ;; esac; return 1; }
READY="" PATHLINE=0
if on_path "$BINDIR" || [[ $(command -v aleph 2>/dev/null) == "$BIN" ]]; then
  READY=$BINDIR
else
  for d in "$HOME/.local/bin" "$HOME/bin" /opt/homebrew/bin /usr/local/bin; do
    if on_path "$d" && [[ -d $d && -w $d ]] && ln -sf "$BIN" "$d/aleph" 2>/dev/null; then
      READY=$d; echo "✓ linked aleph into $d (on your PATH)"; break
    fi
  done
  if [[ -z $READY ]] && on_path /usr/local/bin && [[ -t 0 ]]; then
    say "$BINDIR isn't on your PATH. /usr/local/bin is, but it belongs to root, so linking aleph there needs sudo:"
    echo "    sudo ln -sf \"$BIN\" /usr/local/bin/aleph"
    # asked even with --yes: it's your password
    read -r -p "  Run that now (sudo asks for your password)? [y/N] " a || a=""
    if [[ $a == [Yy]* ]]; then
      if sudo mkdir -p /usr/local/bin && sudo ln -sf "$BIN" /usr/local/bin/aleph; then
        READY=/usr/local/bin; echo "✓ linked aleph into /usr/local/bin"
      else
        echo "  that didn't work — run the line above yourself" >&2
      fi
    fi
  fi
  [[ -n $READY ]] || PATHLINE=1
fi
qdir=$(printf '%s' "$BINDIR" | sed 's/[\\"$`]/\\&/g')
# a PATH line for a shell other than zsh (zsh's is in its block below)
if (( PATHLINE )); then
  case ${SHELL##*/} in
    zsh) ;;
    bash)
      if [[ $OS == Darwin ]]; then brc=$HOME/.bash_profile; else brc=$HOME/.bashrc; fi
      line="export PATH=\"$qdir:\$PATH\"   # aleph"
      if grep -qsF "$line" "$brc" || { touch "$brc" && echo "$line" >> "$brc"; }; then
        echo "✓ ${brc/#$HOME/~}: $BINDIR on the PATH (new shells)"
      else
        echo "warning: couldn't write $brc — add this line to it yourself:" >&2
        echo "    $line" >&2
      fi
      ;;
    *)
      echo "warning: add $BINDIR to your PATH in your shell's config, e.g.:" >&2
      echo "    export PATH=\"$BINDIR:\$PATH\"" >&2
      ;;
  esac
fi

RC="${ZDOTDIR:-$HOME}/.zshrc"
BEGIN='# >>> aleph >>>' END='# <<< aleph <<<'
OLDB='# >>> claude-herdr >>>' OLDE='# <<< claude-herdr <<<'
touch "$RC"
cp "$RC" "$RC.bak.$(date +%Y%m%d%H%M%S)"

# function names the previous blocks defined (kept as aliases)
OLDNAMES=$(awk -v b="$OLDB" -v e="$OLDE" -v b2="$BEGIN" -v e2="$END" '
  $0==b || $0==b2 {f=1; next} $0==e || $0==e2 {f=0; next}
  f && /^[A-Za-z_][A-Za-z0-9_-]*\(\)/ { sub(/\(\).*/, ""); print }' "$RC" | sort -u)

tmp=$(mktemp)
awk -v b="$OLDB" -v e="$OLDE" -v b2="$BEGIN" -v e2="$END" '
  $0==b || $0==b2 {skip=1; next} $0==e || $0==e2 {skip=0; next} !skip' "$RC" > "$tmp"
qbin=$(printf '%s' "$BIN" | sed 's/[\\"$`]/\\&/g')
{
  echo "$BEGIN"
  if (( PATHLINE )); then echo "export PATH=\"$qdir:\$PATH\""; fi
  echo "$NAME() { \"$qbin\" \"\$@\"; }"
  for n in $OLDNAMES; do
    [[ $n == "$NAME" || $n == aleph || $n == "$WRAP" ]] && continue
    case $n in
      claude|codex|opencode|gemini|copilot|cursor-agent|droid|amp|kimi|qwen)
        [[ -z $WRAP ]] && WRAP=$n ;;
      *) echo "$n() { ALEPH_VIA=$n \"$qbin\" \"\$@\"; }   # old name — use $NAME (or aleph)" ;;
    esac
  done
  if [[ -n $WRAP ]]; then
    echo "$WRAP() { ALEPH_WRAP=1 \"$qbin\" \"\$@\"; }   # Aleph, and $WRAP's own commands pass through"
  fi
  if (( COMP )); then
    echo "[[ -o interactive && -z \$HERDR_ENV ]] && source <(\"$qbin\" completions zsh --name $NAME </dev/null 2>/dev/null)"
  fi
  echo "$END"
} >> "$tmp"
mv "$tmp" "$RC"
echo "✓ ~/.zshrc: $NAME → $BIN"
if [[ -n $WRAP ]]; then
  "$BIN" settings set wraps "$WRAP" </dev/null >/dev/null && echo "✓ $WRAP goes through Aleph; its own commands pass through to it (settings: wraps)"
fi

# the first run moves claude-herdr's config, state, service and shortcuts over
"$BIN" version </dev/null >/dev/null 2>&1 || true

CONF="${ALEPH_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/aleph/config}"
if [[ -n $HOST ]]; then
  "$BIN" host set "$HOST" || true
elif [[ ! -e $CONF && ! -e ${CONF%/config}/aleph.toml ]]; then
  echo "next: pick a default host —  aleph host pick   (or: aleph host add <alias> <user@hostname>)"
fi
if (( RAYCAST )); then
  if command -v npm >/dev/null; then
    ( cd extras/raycast-extension && npm ci --no-audit --no-fund --loglevel=error && if [[ -n $FEATURES ]]; then ALEPH_SHOWCASE=1 npm run build; else npm run build; fi ) \
      && echo "✓ Raycast extension built (open Raycast › Import Extension › extras/raycast-extension)"
  else
    echo "warning: --raycast needs Node.js and npm — skipped" >&2
  fi
fi
if [[ -n $READY ]]; then
  echo "✓ ready: aleph runs from any shell now ($NAME: open a new terminal, or source $RC)"
else
  echo "✓ $BINDIR goes on the PATH in new shells — open a new terminal (or: exec \$SHELL -l), then: aleph"
fi
if (( DEFAULTS )) && [[ -t 0 && -t 1 ]]; then
  # quick start: every default, installed on the install screen, then Aleph
  exec "$BIN" setup --defaults
fi
