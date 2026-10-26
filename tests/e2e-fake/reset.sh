#!/usr/bin/env bash
# Reset the fake world (see env.sh). Run after sourcing env.sh.
set -e
: "${ST:?source tests/e2e-fake/env.sh first}"
# (a fake socket server still running sees $ST go and stops: fakesock.sh)
rm -rf "$ST" "$HOME" "$RHOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME" ${ALEPH_FAKE_SOCK:+"$ALEPH_FAKE_SOCK"}
# bare machines' own homes ($ST/bare_<host>: see bin/ssh)
rm -rf "$RHOME"_*
mkdir -p "$ST" "$HOME/.ssh/config.d" "$HOME/.config/aleph" "$HOME/.warp/tab_configs" "$HOME/.raycast-scripts" "$RHOME"
cat > "$HOME/.ssh/config" <<'C'
Host *
    ServerAliveInterval 30
Include config.d/*

Host b31
    HostName 10.0.0.49
    User selene
Host gpu2
    HostName gpu2.lan
    User tester
    Port 2222
C
printf 'hostname 10.0.0.49\nuser selene\nport 22\nidentityfile ~/.ssh/id_ed25519\nproxyjump none\n' > "$ST/G_b31"
printf 'hostname gpu2.lan\nuser tester\nport 2222\nidentityfile ~/.ssh/id_ed25519\nproxyjump none\n' > "$ST/G_gpu2"
printf 'PRIVATE KEY\n' > "$HOME/.ssh/id_ed25519"; chmod 600 "$HOME/.ssh/id_ed25519"; echo "ssh-ed25519 AAAA test" > "$HOME/.ssh/id_ed25519.pub"
echo '{"result":{"machines":[{"id":"p1","label":"b31","ssh_target":"b31","enabled":true},{"id":"p2","label":"gpu2","ssh_target":"gpu2","enabled":true}]}}' > "$ST/machines.json"
cat > "$ST/agents_b31.json" <<'J'
{"result":{"agents":[
 {"pane_id":"w1:p3","tab_id":"w1:t3","agent":"claude","agent_status":"working","foreground_cwd":"/home/selene/ox_color_check","name":"ignite","agent_session":"3f2a9c1e-0b7d-4c55-9a61-2e8f0c4d7a10","terminal_title_stripped":"Profile the layout engine","tokens":{"mark":"◆","hue":"amber"}},
 {"pane_id":"w1:p2","tab_id":"w1:t2","agent":"claude","agent_status":"blocked","cwd":"/home/selene","name":"temper","terminal_title_stripped":"Run the cargo benches","tokens":{"mark":"●","hue":"ember"}},
 {"pane_id":"w1:p9","tab_id":"w1:t1","agent":"codex","agent_status":"idle"}]}}
J
echo '{"result":{"tabs":[{"tab_id":"w1:t1","label":"shell"},{"tab_id":"w1:t2","label":"claude_temper"},{"tab_id":"w1:t3","label":"claude_ignite:ox_color_check"}]}}' > "$ST/tabs_b31.json"
cat > "$ST/agents_gpu2.json" <<'J'
{"result":{"agents":[
 {"pane_id":"w1:p4","tab_id":"w1:t4","workspace_id":"w1","agent":"claude","agent_status":"done","cwd":"/srv/train","name":"weave","terminal_title_stripped":"Distil B5 teacher into student","tokens":{"mark":"★","hue":"tide"}}]}}
J
echo '{"result":{"tabs":[{"tab_id":"w1:t4","label":"claude_weave:train"}]}}' > "$ST/tabs_gpu2.json"
touch "$ST/up_b31" "$ST/up_gpu2"
for d in wind_sweep blaze kite-web-ui moss-env-check dev_ox fern_lab zap_plugins zap_probe ox_color_check ox_meadow AuroraMix_3.1 node_modules .cache; do mkdir -p "$RHOME/$d"; done
mkdir -p "$RHOME/ox_color_check/src" "$RHOME/ox_color_check/.git"; echo 'ref: refs/heads/feat/edge-fit' > "$RHOME/ox_color_check/.git/HEAD"
touch "$RHOME/ox_color_check/pyproject.toml" "$RHOME/ox_color_check/CLAUDE.md"
printf 'ssh_host=b31\nmachine=b31\nhosts=b31,gpu2\nlocal=false\nmoshi.session=false\nui.whats_new=off\n' > "$HOME/.config/aleph/config"
printf '# >>> aleph >>>\nclaude() { "%s" "$@"; }\n# <<< aleph <<<\n' "$(command -v aleph)" > "$HOME/.zshrc"
# agents on the fake remotes (both share $RHOME): codex everywhere, older on gpu2
mkdir -p "$RHOME/.local/bin"
printf '#!/bin/sh\n[ "$FAKE_HOST" = gpu2 ] && echo "codex-cli 0.40.1" || echo "codex-cli 0.46.0"\n' > "$RHOME/.local/bin/codex"; chmod +x "$RHOME/.local/bin/codex"
echo 0.8.0 > "$ST/herdrver_gpu2"
# LLM servers on the fake machines: simulated (tool output + recorded HTTP
# answers from tests/fixtures/services), nothing runs
SVC=$(cd "$(dirname "$0")/../fixtures/services" && pwd)
ln -s "$SVC/gpu-box" "$ST/svc_b31"; ln -s "$SVC/ws" "$ST/svc_gpu2"; ln -s "$SVC/mac" "$ST/svc_mac"
printf 'claude outdated\ncodex missing\nopencode missing\n' > "$ST/integ_gpu2"
# the agents' own session logs on the fake remotes (synthetic, from
# tests/fixtures/usage), for aleph usage: Claude Code, Codex, Pi, Gemini,
# OpenCode (its sqlite db when sqlite3 is here)
FX=$(cd "$(dirname "$0")/../fixtures/usage" && pwd)
mkdir -p "$RHOME/.claude" "$RHOME/.codex" "$RHOME/.pi/agent" "$RHOME/.gemini" "$RHOME/.local/share/opencode"
cp -R "$FX/claude/projects" "$RHOME/.claude/"
cp -R "$FX/codex/sessions" "$RHOME/.codex/"
cp -R "$FX/pi/sessions" "$RHOME/.pi/agent/"
cp -R "$FX/gemini/tmp" "$RHOME/.gemini/"
cp -R "$FX/opencode/storage" "$RHOME/.local/share/opencode/"
if command -v sqlite3 >/dev/null 2>&1; then sqlite3 "$RHOME/.local/share/opencode/opencode.db" < "$FX/opencode/opencode.sql"; fi
# whose: Claude Code's account (.claude.json) and a second login in ~/.claude-work
cp "$FX/claude/claude.json" "$RHOME/.claude.json"
mkdir -p "$RHOME/.claude-work"; cp -R "$FX/claude-work/projects" "$RHOME/.claude-work/"
cp "$FX/claude-work/claude.json" "$RHOME/.claude-work/.claude.json"
# the agents' credentials beside them, never read: SECRET must show up nowhere
cp "$FX/canary/claude-credentials.json" "$RHOME/.claude/.credentials.json"
cp "$FX/canary/claude-credentials.json" "$RHOME/.claude-work/.credentials.json"
cp "$FX/canary/codex-auth.json" "$RHOME/.codex/auth.json"
cp "$FX/canary/opencode-auth.json" "$RHOME/.local/share/opencode/auth.json"
cp "$FX/canary/gemini-oauth.json" "$RHOME/.gemini/oauth_creds.json"
cp "$FX/canary/google_accounts.json" "$RHOME/.gemini/google_accounts.json"
# projects: repos on the fake machines (both share this home) — one with a password in its remote (it must
# never come out), its worktree, a clone with no remote — and Claude sessions in each
G="env GIT_CONFIG_GLOBAL=/dev/null GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@x GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@x"
mkdir -p "$RHOME/src"
( cd "$RHOME/src" && $G git init -q app && cd app && $G git commit -q --allow-empty -m one &&
  $G git remote add origin "https://me:SECRET-ghp@github.com/Acme/App.git" && $G git worktree add -q ../app-wt -b wt &&
  cd .. && $G git clone -q app copy && cd copy && $G git remote remove origin ) >/dev/null 2>&1
mkdir -p "$RHOME/.claude/projects/-src-app"
for p in app app-wt copy; do
  printf '{"type":"assistant","message":{"model":"claude-sonnet-5-5","id":"msg_P_%s","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":1000}},"requestId":"req_P_%s","timestamp":"2026-10-22T11:00:00.000Z","sessionId":"5c000000-0000-4000-8000-00000000%s","cwd":"%s/src/%s"}\n' \
    "$p" "$p" "$(printf %s "$p" | cksum | cut -c1-4)" "$RHOME" "$p"
done > "$RHOME/.claude/projects/-src-app/5c000000-0000-4000-8000-000000000001.jsonl"
: > "$ST/log"
