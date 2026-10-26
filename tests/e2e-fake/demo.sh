#!/usr/bin/env bash
# Showcase's demo world, end to end, in the fake world (needs bash + jq and a
# build with the feature):
#   cargo build --features showcase && tests/e2e-fake/demo.sh
# The fleet, stats, usage, agents, services, doctor, approve, new and close
# all answer from the made-up world, and nothing real is reached: the fake
# ssh and herdr record no call at all.
set -u
ROOT=$(cd "$(dirname "$0")" && pwd)
# never against the real machine: only from tests/e2e-fake, with its fake world
[ -f "$ROOT/env.sh" ] && [ "$(basename "$ROOT")" = "e2e-fake" ] || { echo "demo.sh: run it from tests/e2e-fake (no fake world here)" >&2; exit 2; }
source "$ROOT/env.sh" "${1:-debug}" || exit 2
case "${ST:-}:${HOME:-}" in *e2e-fake/.run*:*e2e-fake/.run*) ;; *) echo "demo.sh: the fake world didn't load (ST/HOME aren't isolated)" >&2; exit 2;; esac
aleph showcase status </dev/null >/dev/null 2>&1 || { echo "demo.sh: this build has no showcase mode (cargo build --features showcase)" >&2; exit 2; }
"$ROOT/reset.sh"
pass=0; fail=0
check() { # check <name> <command…>  (passes when the command succeeds)
  local n=$1; shift
  if out=$("$@" 2>&1); then pass=$((pass+1)); printf '  \033[32m✓\033[0m %s\n' "$n"
  else fail=$((fail+1)); printf '  \033[31m✗\033[0m %s\n%s\n' "$n" "$(printf '%s' "$out" | tail -n 8 | sed 's/^/      /')"; fi
}
W="$XDG_STATE_HOME/aleph/showcase"

echo "aleph demo world · $(aleph version 2>/dev/null | head -1)"
check "showcase on --demo writes 30 days of history" sh -c 'aleph showcase on --demo --fresh </dev/null 2>&1 | grep -q "usage records added"'
: > "$ST/log"
check "ls --json: five machines, ten-plus sessions"  sh -c 'aleph ls --json </dev/null | jq -e "(.hosts | length == 5) and ([.hosts[].sessions[]] | length >= 9)"'
check "ls --json: the cast of agents and a shell"     sh -c 'aleph ls --json </dev/null | jq -e "[.hosts[].sessions[].agent] | (index(\"codex\") and index(\"gemini\") and index(\"pi\") and index(\"opencode\") and index(\"shell\")) != null"'
check "ls: TSV of the demo fleet"                     sh -c 'aleph ls </dev/null | cut -f1 | sort -u | grep -qx magrathea'
check "stats --json: waves, cores and GPUs"           sh -c 'aleph stats --json </dev/null | jq -e "[.hosts[] | select(.alias==\"magrathea\")][0] | .reachable and (.gpus | length == 2) and (.cores | length == 64) and .cpu > 0"'
check "usage --json: a month of tokens and dollars"   sh -c 'aleph usage 30d --json --no-sync </dev/null | jq -e ".total.cost > 10 and (.rows | length) > 0"'
check "agents --json: what each machine has"          sh -c 'aleph agents --json </dev/null | jq -e "[.hosts[] | select(.alias==\"tycho-base\")][0] | [.agents[] | select(.installed) | .id] | index(\"pi\") != null"'
check "services --json: Ollama, vLLM, a gateway"      sh -c 'aleph services --json </dev/null | jq -e "[.hosts[].services[].id] | (index(\"ollama\") and index(\"vllm\") and index(\"litellm\")) != null"'
check "doctor: one warning, with a fix"               sh -c 'aleph doctor </dev/null 2>&1 | grep -q "1 warning"'
check "approve answers a question in the world"       sh -c 'B=$(aleph ls </dev/null | awk -F"\t" "\$3==\"blocked\"{print \$1\":\"\$2; exit}"); [ -z "$B" ] && exit 0; aleph approve "$B" </dev/null && grep -q "\"do\":\"approve\"" "'"$W"'/actions.jsonl"'
check "new starts a session in the world"             sh -c 'aleph --host europa new --agent codex --detach -m "polish the errors" </dev/null >/dev/null 2>&1; aleph ls --json </dev/null | jq -e "[.hosts[].sessions[] | select(.host==\"europa\" and .agent==\"codex\")] | length == 1"'
check "close ends it"                                 sh -c 'N=$(aleph ls --json </dev/null | jq -r "[.hosts[].sessions[] | select(.host==\"europa\" and .agent==\"codex\")][0].name"); aleph --host europa close "$N" --yes </dev/null >/dev/null 2>&1; aleph ls --json </dev/null | jq -e "[.hosts[].sessions[] | select(.host==\"europa\" and .agent==\"codex\")] | length == 0"'
check "wake, update, host setup are refused"          sh -c '! aleph wake vogsphere </dev/null 2>/dev/null && ! aleph update --check </dev/null 2>/dev/null && ! aleph host setup europa --yes </dev/null 2>/dev/null'
check "the fleet's machines can't be changed"         sh -c '! aleph settings set hosts b31 </dev/null 2>/dev/null'
check "its files live in the sandbox"                 sh -c '[ -f "'"$W"'/usage/offsets.json" ] && [ ! -e "$XDG_STATE_HOME/aleph/usage/offsets.json" ]'
# capture & share: posters, casts, the watermark (into the fake HOME's Pictures / Movies)
P="$HOME/Pictures/Aleph"
aleph settings set showcase.watermark "made by alex" </dev/null >/dev/null 2>&1
check "snap: the fleet as an SVG card, nothing real"  sh -c 'f=$(aleph showcase snap --view fleet --png off --json </dev/null | jq -r .svg); [ -s "$f" ] && grep -q "<svg" "$f" && grep -q "made by alex" "$f" && grep -q magrathea "$f" && ! grep -Eq ">[^<]*(b31|gpu2)" "$f"'
check "snap --view usage --frame plain --out FILE"    sh -c 'aleph showcase snap --view usage --frame plain --size 120x40 --png off --out "'"$P"'/u.svg" </dev/null >/dev/null && grep -q "Monitor › Usage" "'"$P"'/u.svg" && ! grep -q "al-page" "'"$P"'/u.svg"'
check "snap: an unknown view, a bad size refused"     sh -c '! aleph showcase snap --view nowhere </dev/null 2>/dev/null && ! aleph showcase snap --size 9x9 </dev/null 2>/dev/null'
check "casts: none yet, as JSON"                      sh -c 'aleph showcase casts --json </dev/null | jq -e ".casts | length == 0"'
if PATH="$PATH:/opt/homebrew/bin:/usr/local/bin" command -v tmux >/dev/null 2>&1; then
  check "the dashboard records a cast (ctrl+r, tmux)" sh -c 'SNAP_SETTLE=0.3 "'"$ROOT"'/tsnap.sh" democast 120 32 "$(printf "4 C-r\n2 j\n1 C-r")" aleph >/dev/null; f=$(aleph showcase casts --json </dev/null | jq -r ".casts[0].path"); [ -s "$f" ] && head -1 "$f" | jq -e ".version == 2 and .width == 120" >/dev/null && grep -q "\"o\"" "$f" && grep -q "made by alex" "$f" && ! tail -n +2 "$f" | grep -Eq "b31|gpu2"'
  # (the fake world's PATH has no ffmpeg)
  check "no ffmpeg: the .cast stays, an alert says so"  sh -c 'grep -q "ffmpeg isn.t on this computer" "'"$ROOT"'/.run/democast.txt" && [ -s "$(aleph showcase casts --json </dev/null | jq -r ".casts[0].cast")" ]'
fi
# recordings as video: the render refused without ffmpeg; with this
# computer's ffmpeg (the only thing added to PATH), an MP4 and a GIF
check "render without ffmpeg: refused, the .cast kept" sh -c 'f=$(aleph showcase casts --json </dev/null | jq -r ".casts[0].cast // empty"); [ -z "$f" ] && exit 0; ! aleph showcase render "$f" </dev/null 2>/dev/null && [ -s "$f" ]'
FF=$(PATH="$PATH:/opt/homebrew/bin:/usr/local/bin" command -v ffmpeg)
if [ -n "$FF" ]; then
  mkdir -p "$RUN/ffbin"; ln -sf "$FF" "$RUN/ffbin/ffmpeg"
  check "render: a cast as MP4 + GIF, beside it (the cast stays)"     sh -c 'f=$(aleph showcase casts --json </dev/null | jq -r ".casts[0].cast // empty"); [ -z "$f" ] && exit 0; PATH="$PATH:'"$RUN"'/ffbin" aleph showcase render "$f" --json </dev/null | jq -e ".ok and .width == 1920 and .height == 1080" >/dev/null && aleph showcase casts --json </dev/null | jq -e ".casts[0] | .mp4 != null and .gif != null and .cast != null and .width == 1920" >/dev/null'
  if PATH="$PATH:/opt/homebrew/bin:/usr/local/bin" command -v tmux >/dev/null 2>&1; then
    check "after a stop: MP4 + GIF made after the dashboard's gone" sh -c 'PATH="$PATH:'"$RUN"'/ffbin" SNAP_SETTLE=0.3 "'"$ROOT"'/tsnap.sh" demorender 100 30 "$(printf "4 C-r\n2 j\n1 C-r")" aleph >/dev/null; for i in $(seq 90); do aleph showcase casts --json </dev/null | jq -e ".casts[0] | .mp4 != null and .gif != null and .cast == null" >/dev/null && exit 0; sleep 1; done; exit 1'
  fi
fi
aleph settings unset showcase.watermark </dev/null >/dev/null 2>&1
# safety nets: screens redacted or hidden, money scaled or hidden, the pause
S=$(aleph ls --json </dev/null | jq -r '[.hosts[].sessions[] | select(.agent=="claude")][0] | .host + ":" + .name')
ALIEN='⏃|⏚|☊|⎅|⟒|⎎|☌|⊑|⟟|⟊|☍|⌰|⋔|⋏|⍜|⌿|⍾|⍀|⌇|⏁|⎍|⎐|⍙|⌖|⊬|⋉'
check "the world's screens show as they are"          sh -c 'aleph peek "'"$S"'" </dev/null 2>/dev/null | grep -q "⏺" && ! aleph peek "'"$S"'" </dev/null 2>/dev/null | grep -Eq "'"$ALIEN"'"'
aleph settings set showcase.demo_screens redact </dev/null >/dev/null 2>&1
check "screens = redact: the frame stays, words go"   sh -c 'T=$(aleph peek "'"$S"'" --json </dev/null | jq -r .text); printf "%s" "$T" | grep -q "⏺" && printf "%s" "$T" | grep -Eq "'"$ALIEN"'"'
aleph settings set showcase.demo_screens hide </dev/null >/dev/null 2>&1
check "screens = hide: a card instead"                sh -c 'aleph peek "'"$S"'" --json </dev/null | jq -e ".text == \"◌ screen hidden for the demo\""'
aleph settings unset showcase.demo_screens </dev/null >/dev/null 2>&1
check "money = scaled: every \$ ×0.5, totals add up"  sh -c 'A=$(aleph usage 30d --json --no-sync </dev/null | jq .total.cost); aleph settings set showcase.money scaled >/dev/null && aleph settings set showcase.money_scale 0.5 >/dev/null && aleph usage 30d --json --no-sync </dev/null | jq -e --argjson a "$A" "(.total.cost - \$a / 2 | fabs) < 0.01 and (([.rows[].cost] | add) - .total.cost | fabs) < 0.01"'
aleph settings set showcase.money hidden </dev/null >/dev/null 2>&1
check "money = hidden: 0 in JSON, tokens as ever"     sh -c 'aleph usage 30d --json --no-sync </dev/null | jq -e ".total.cost == 0 and .total.tokens > 0"'
aleph settings unset showcase.money </dev/null >/dev/null 2>&1; aleph settings unset showcase.money_scale </dev/null >/dev/null 2>&1
check "panic on: the state says so, the CLI as ever"  sh -c 'aleph showcase panic on </dev/null 2>&1 | grep -q "paused" && jq -e ".panic == true" "$XDG_STATE_HOME/aleph/showcase.json" >/dev/null && aleph showcase status </dev/null 2>&1 | grep -q "paused" && aleph ls --json </dev/null | jq -e ".hosts | length == 5"'
if PATH="$PATH:/opt/homebrew/bin:/usr/local/bin" command -v tmux >/dev/null 2>&1; then
  check "the paused dashboard: stars, nothing else"   sh -c 'SNAP_SETTLE=0.5 "'"$ROOT"'/tsnap.sh" demopanic 100 30 "3 j" aleph > "'"$ROOT"'/.run/demopanic.out"; grep -q "be right back" "'"$ROOT"'/.run/demopanic.out" && ! grep -Eq "magrathea|europa|Fleet" "'"$ROOT"'/.run/demopanic.out"'
  check "F12 in the dashboard ends the pause"         sh -c 'SNAP_SETTLE=0.5 "'"$ROOT"'/tsnap.sh" demoresume 100 30 "3 F12" aleph >/dev/null; jq -e ".panic == false" "$XDG_STATE_HOME/aleph/showcase.json" >/dev/null'
fi
check "panic off: back"                               sh -c 'aleph showcase panic off </dev/null 2>&1 | grep -q "back" && jq -e ".panic == false" "$XDG_STATE_HOME/aleph/showcase.json" >/dev/null'
# demo flow: time warp and story mode (every process on one clock, one script)
check "status --json: the world's clock and its story" sh -c 'aleph showcase status --json </dev/null | jq -e ".world.warp == 1 and (.world.story.beats | length) == 4 and (.world.story.plan | length) == 4 and .world.time > 0"'
check "warp 10: ten times as fast, and no jump"       sh -c 'w() { aleph showcase status --json </dev/null | jq .world.time; }; a=$(w); aleph settings set showcase.warp 10 </dev/null >/dev/null; sleep 2; b=$(w); sleep 1; c=$(w); aleph settings unset showcase.warp </dev/null >/dev/null; d=$(w); echo "$a $b $c $d"; [ $((b - a)) -ge 1 ] && [ $((b - a)) -le 6 ] && [ $((c - b)) -ge 9 ] && [ $((c - b)) -le 25 ] && [ "$d" -ge "$c" ]'
check "the story through the hub: its rule, its heal, a limit, a finish" sh -c 'aleph settings set showcase.warp 5 </dev/null >/dev/null; aleph settings set showcase.story_every 1m </dev/null >/dev/null; aleph watch </dev/null >/dev/null 2>&1 & p=$!; sleep 30; kill $p; aleph settings unset showcase.warp </dev/null >/dev/null; aleph settings unset showcase.story_every </dev/null >/dev/null; aleph inbox -n 300 --json </dev/null | jq -e "any(.[]; .kind == \"auto\" and .host == \"europa\" and (.title | test(\"rule story\"))) and any(.[]; .kind == \"heal\" and .host == \"vogsphere\") and any(.[]; .kind == \"limit\" and .host == \"magrathea\") and any(.[]; .kind == \"done\" and (.title | test(\"Earth Mk II\")))"'
# presenter sync: the dashboard says what it shows, the daemon keeps it and tells its subscribers
check "presenter sync: the daemon keeps it, its topic says so, off is off" sh -c 'aleph settings set showcase.sync on </dev/null >/dev/null; aleph watch </dev/null >/dev/null 2>&1 & p=$!; i=0; until aleph api daemon.ping </dev/null >/dev/null 2>&1 || [ $i -ge 40 ]; do sleep 0.25; i=$((i+1)); done; r=0; ( sleep 1.5; aleph api presenter.set "{\"view\":\"focus\",\"host\":\"europa\",\"pane_id\":\"w1:p2\",\"name\":\"x\"}" </dev/null >/dev/null ) & printf "{\"id\":1,\"method\":\"events.subscribe\",\"params\":{\"state\":false,\"kinds\":[],\"topics\":[\"presenter\"]}}\n" | aleph api --stdio | head -n 2 | jq -se ".[1].changed == \"presenter\"" >/dev/null || r=1; aleph api presenter.get </dev/null | jq -e ".on and .view == \"focus\" and .session == \"europa:x\"" >/dev/null || r=2; aleph settings set showcase.sync off </dev/null >/dev/null; aleph api presenter.get </dev/null | jq -e ".on == false" >/dev/null || r=3; kill $p; aleph settings unset showcase.sync </dev/null >/dev/null; [ $r -eq 0 ] || { echo "step $r"; exit 1; }'
if PATH="$PATH:/opt/homebrew/bin:/usr/local/bin" command -v tmux >/dev/null 2>&1; then
  check "presenter sync: the dashboard tells the daemon what it shows" sh -c 'aleph settings set showcase.sync on </dev/null >/dev/null; aleph watch </dev/null >/dev/null 2>&1 & p=$!; i=0; until aleph api daemon.ping </dev/null >/dev/null 2>&1 || [ $i -ge 40 ]; do sleep 0.25; i=$((i+1)); done; SNAP_SETTLE=0.5 "'"$ROOT"'/tsnap.sh" demosync 120 32 "$(printf "4 j\n1")" aleph >/dev/null; v=$(aleph api presenter.get </dev/null); kill $p; aleph settings unset showcase.sync </dev/null >/dev/null; echo "$v"; printf "%s" "$v" | jq -e ".on and .view == \"fleet\" and (.host | length > 0) and (.name | length > 0) and (.pane_id | test(\"^w\"))" >/dev/null'
fi
check "nothing real was reached (no ssh, no herdr)"   sh -c '! grep -Eq "^(ssh|herdr) " "$ST/log"'
check "sound cues say what they'd play, and don't"    sh -c 'aleph showcase sound needs </dev/null 2>&1 | grep -Eq "Glass .*not played \((the fake world|ALEPH_NO_SOUND is set)\)"'
check "showcase off: the real fleet again"            sh -c 'aleph showcase off </dev/null >/dev/null && aleph ls --json </dev/null | jq -e "[.hosts[].alias] == [\"b31\",\"gpu2\"]"'
echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
