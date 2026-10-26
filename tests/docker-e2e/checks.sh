#!/bin/bash
# Runs inside the client container as `dev` (see run.sh). aleph talks to
# the box's real herdr server through `herdr --machine box` (ssh). Sessions are
# created with herdr itself: `aleph new` needs a terminal.
set -u
export NO_COLOR=1
pass=0 fail=0
check() { # check <name> <command…>
  local n=$1; shift
  if out=$("$@" 2>&1); then pass=$((pass + 1)); printf '  ok    %s\n' "$n"
  else fail=$((fail + 1)); printf '  FAIL  %s\n%s\n' "$n" "$(printf '%s' "$out" | tail -n 12 | sed 's/^/        /')"; fi
}
ch() { aleph "$@" </dev/null; }
hm() { herdr --machine box "$@" </dev/null; }
# first pane id / tab id in a herdr JSON reply
pane_of() { jq -r '[.. | objects | .pane_id? // empty][0] // empty'; }

wait_for() { # wait_for <seconds> <command…>
  local t=$1; shift
  for _ in $(seq "$((t * 2))"); do "$@" >/dev/null 2>&1 && return 0; sleep 0.5; done
  "$@"
}

session_named() { ch ls --json | jq -e --arg n "$1" '[.hosts[].sessions[] | select(.name == $n or .display == ("claude_" + $n))] | length == 1'; }
session_gone() { ch ls --json | jq -e --arg n "$1" '[.hosts[].sessions[] | select(.name == $n or .display == ("claude_" + $n))] | length == 0'; }
recent_has() { hm pane read "$1" --source recent --lines 40 | grep -q -- "$2"; }

echo "aleph docker e2e · $(aleph version </dev/null | head -n 1) · $(herdr --version)"

# --- the box is reachable and saved in herdr ------------------------------------
wait_for 30 ssh -o BatchMode=yes box true || { echo "can't ssh to box" >&2; exit 1; }
if ! herdr machine list --json </dev/null | jq -e '[.. | objects | select(.label? == "box")] | length > 0' >/dev/null 2>&1; then
  herdr machine add box --label box </dev/null || { echo "herdr machine add box failed" >&2; exit 1; }
fi
check "herdr reaches the box"        hm workspace list

# --- two sessions, started the way `aleph new` does (tab + agent start) -------------
hm workspace create --label e2e --cwd /home/dev/project --no-focus >/dev/null 2>&1 || true
start() { # start <name> → prints pane id
  local n=$1 r p
  r=$(hm tab create --label "claude_$n" --cwd /home/dev/project 2>/dev/null) ||
    r=$(hm workspace create --label "claude_$n" --cwd /home/dev/project 2>/dev/null)
  p=$(printf '%s' "$r" | pane_of)
  [ -n "$p" ] || { echo "no pane for $n: $r" >&2; return 1; }
  sleep 1 # let the shell reach its prompt
  hm agent start "$n" --kind claude --pane "$p" --timeout 15000 >/dev/null 2>&1 ||
    hm pane run "$p" claude >/dev/null 2>&1
  echo "$p"
}
P1=$(start alpha) || exit 1
P2=$(start beta) || exit 1
echo "  started alpha=$P1 beta=$P2"

check "doctor --json is valid JSON"  sh -c 'aleph doctor --json </dev/null >/tmp/doctor.json; jq -e ".sections | length > 0" /tmp/doctor.json'
check "doctor: key login works"      jq -e '[.sections[].checks[] | select(.label == "key login" and .status == "ok")] | length == 1' /tmp/doctor.json
check "doctor: herdr on the box"     jq -e '[.sections[] | select(.section | startswith("On ")) | .checks[] | select(.label == "herdr" and .status == "ok")] | length == 1' /tmp/doctor.json
check "doctor: machine saved"        jq -e '[.sections[].checks[] | select(.label == "machine" and .status == "ok")] | length == 1' /tmp/doctor.json

check "ls --json sees both sessions" wait_for 20 sh -c 'aleph ls --json </dev/null | jq -e "[.hosts[].sessions[]] | length >= 2"'
check "ls --json: box is reachable"  sh -c 'aleph ls --json </dev/null | jq -e ".hosts[0].alias == \"box\" and .hosts[0].reachable and .hosts[0].error == null"'
check "ls --json: session fields"    sh -c 'aleph ls --json </dev/null | jq -e "[.hosts[].sessions[] | select(.agent == \"claude\" and .pane_id != \"\" and .tab_id != \"\" and (.display | startswith(\"claude_\")))] | length >= 2"'
check "ls piped is TSV"              sh -c 'aleph ls </dev/null | grep -q "^box	claude_alpha	"'

check "send reaches the agent"       ch send alpha "hello from e2e"
check "send: the stub got it"        wait_for 15 recent_has "$P1" "got: hello from e2e"
check "keys: 1 + enter"              ch keys box:alpha 1 enter
check "keys: the stub got them"      wait_for 15 recent_has "$P1" "got: 1"

check "rename alpha → gamma"         ch rename alpha gamma
check "rename: tab label follows"    wait_for 15 sh -c 'aleph ls --json </dev/null | jq -e "[.hosts[].sessions[] | select(.display == \"claude_gamma\")] | length == 1"'
check "send by the new name"         ch send gamma "still there?"

check "close --yes"                  ch close beta --yes
check "close: beta is gone"          wait_for 15 session_gone beta
check "close: gamma still runs"      session_named gamma
check "unknown session is an error"  sh -c '! aleph send nosuch hi </dev/null'

echo
echo "$pass passed, $fail failed"
if [ "$fail" -gt 0 ]; then
  echo "--- herdr agent list"; hm agent list | head -c 4000; echo
  echo "--- aleph ls --json"; ch ls --json | head -c 4000; echo
fi
[ "$fail" -eq 0 ]
