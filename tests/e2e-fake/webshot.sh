#!/usr/bin/env bash
# Screenshots of the web console (phone/desktop, dark/light) against the fake world.
#   tests/e2e-fake/webshot.sh 'needs=390x844m:' 'fleet=390x844m:#/fleet' 'desk=1280x860l:#/s/b31:temper'
# Each shot is name=WxH[m][l]:hash (m = phone emulation, l = light theme). PNGs land in .run/web/.
# aleph runs in the fake world; Chrome runs with the REAL home, a scratch profile, a mock keychain and
# the basic password store (with the fake HOME, macOS asks for a keychain). Needs Chrome and node ≥ 22.
REALHOME=$HOME; SHOTS=("$@"); set --
cd "$(dirname "$0")" || exit 1
source env.sh >/dev/null 2>&1; ./reset.sh >/dev/null 2>&1
case "$ST" in *e2e-fake/.run*) ;; *) echo "error: not in the fake world" >&2; exit 1;; esac
[ ${#SHOTS[@]} -gt 0 ] || SHOTS=("needs=390x844m:" "fleet=390x844m:#/fleet" "session=390x1300m:#/s/b31:temper" "inbox=390x844m:#/inbox" "device=390x1000m:#/device" "desk-needs=1280x860:" "desk-session=1280x860l:#/s/b31:temper")
CH=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
NODE=$(PATH=/opt/homebrew/bin:/usr/local/bin:$REALHOME/.local/bin:/usr/bin command -v node)  # not the fake node
O=$RUN/web; P=$RUN/web-profile; rm -rf "$O" "$P"; mkdir -p "$O"
aleph settings set api.http on </dev/null >/dev/null; aleph settings set api.listen 127.0.0.1:17879 </dev/null >/dev/null
aleph daemon run </dev/null >"$O/daemon.log" 2>&1 & d=$!
for _ in $(seq 1 60); do /usr/bin/curl -s -o /dev/null http://127.0.0.1:17879/ && break; sleep 0.25; done
sleep 3
TOK=$(aleph daemon token new webshot --scope control --json </dev/null 2>/dev/null | jq -r .token)
env HOME="$REALHOME" "$CH" --headless=new --user-data-dir="$P" --use-mock-keychain --password-store=basic --no-first-run \
  --disable-gpu --hide-scrollbars --remote-debugging-port=9333 about:blank >/dev/null 2>&1 & c=$!
for _ in $(seq 1 40); do /usr/bin/curl -s -o /dev/null http://127.0.0.1:9333/json/version && break; sleep 0.25; done
"$NODE" webshot.mjs 9333 "$O" http://127.0.0.1:17879 "$TOK" "${SHOTS[@]}"
kill $c $d 2>/dev/null; wait $c $d 2>/dev/null
aleph settings unset api.http </dev/null >/dev/null; aleph settings unset api.listen </dev/null >/dev/null
ls "$O"/*.png
