#!/usr/bin/env bash
# Visual QA with tmux (macOS + Linux): run a full-screen command in a detached
# tmux pane of a fixed size, feed it keys, then capture the screen.
#   tsnap.sh <name> <cols> <rows> "<keys>" <command…>
# keys: one "<delay-seconds> <tmux key names…>" per line, e.g. $'2 j\n0.5 Enter'
#       (tmux send-keys names: Enter Escape Space Tab BTab Up Down C-k q …)
# Writes .run/<name>.txt (plain) and .run/<name>.ansi (with colours; view
# with `cat`). Source env.sh first so the fake world is used.
ROOT=$(cd "$(dirname "$0")" && pwd)
name=$1 cols=$2 rows=$3 keys=$4; shift 4
mkdir -p "$ROOT/.run"
S="snap-$name-$$"
TM=$(PATH="$PATH:/opt/homebrew/bin:/usr/local/bin" command -v tmux) || { echo "tsnap: needs tmux" >&2; exit 1; }
tmux() { "$TM" "$@"; }
# pass only variables that are set: an empty XDG_* makes tools write relative paths
envs() { for v in PATH HOME ST RHOME XDG_CONFIG_HOME XDG_CACHE_HOME XDG_STATE_HOME XDG_DATA_HOME NO_COLOR CC_REFRESH \
    ALEPH_TAB_CMD ALEPH_EDITOR_CMD ALEPH_SECRETS_FILE ALEPH_FAKE_SOCK ALEPH_FAKE_RBIN ALEPH_FAKE_TOOLS; do
  [ -n "${!v}" ] && printf "%s='%s' " "$v" "${!v}"; done; }
tmux -L ccsnap kill-session -t "$S" 2>/dev/null
tmux -L ccsnap -f /dev/null new-session -d -s "$S" -x "$cols" -y "$rows" \
  "env $(envs) COLORTERM=truecolor TERM=xterm-256color $*; sleep 30"
tmux -L ccsnap set -t "$S" status off >/dev/null
tmux -L ccsnap resize-window -t "$S" -x "$cols" -y "$rows" 2>/dev/null
if [ -n "$keys" ]; then
  while read -r d k; do
    sleep "$d"
    # shellcheck disable=SC2086
    [ -n "$k" ] && tmux -L ccsnap send-keys -t "$S" $k
  done <<<"$keys"
fi
sleep "${SNAP_SETTLE:-0.8}"
tmux -L ccsnap capture-pane -t "$S" -p > "$ROOT/.run/$name.txt"
tmux -L ccsnap capture-pane -t "$S" -e -p > "$ROOT/.run/$name.ansi"
tmux -L ccsnap kill-session -t "$S" 2>/dev/null
cat "$ROOT/.run/$name.txt"
