#!/usr/bin/env bash
# The fake herdr socket server (examples/fake_herdr_sock.rs, built by env.sh)
# for the fake world: one socket per machine in $ALEPH_FAKE_SOCK, which the
# fake `herdr status server` then names and the fake `ssh -N -L` forwards to.
# Requests go to $ST/log as `sock <host> <method> <params>`.
#   fakesock.sh start [host…]     (default: b31 gpu2 local)
#   fakesock.sh stop
# Source env.sh first. $ST/sockold_<host> makes one answer like an older herdr.
: "${ST:?source tests/e2e-fake/env.sh first}" "${ALEPH_FAKE_SOCK:?source tests/e2e-fake/env.sh first}"
BIN=${ALEPH_FAKE_TOOLS:-}/fake_herdr_sock
case ${1:-} in
  start)
    shift; [ $# -gt 0 ] || set -- b31 gpu2 local
    [ -x "$BIN" ] || { echo "fakesock: no $BIN (cargo build --example fake_herdr_sock)" >&2; exit 1; }
    mkdir -p "$ALEPH_FAKE_SOCK"; chmod 700 "$ALEPH_FAKE_SOCK"
    # a new one takes over from an old one (it watches $ST/fakesock.pid)
    nohup "$BIN" "$ALEPH_FAKE_SOCK" "$@" >/dev/null 2>"$ST/fakesock.err" </dev/null &
    for _ in $(seq 1 100); do
      ok=1; for h in "$@"; do [ -S "$ALEPH_FAKE_SOCK/$h.sock" ] || ok=0; done
      [ $ok = 1 ] && [ "$(cat "$ST/fakesock.pid" 2>/dev/null)" = "$!" ] && exit 0
      sleep 0.05
    done
    echo "fakesock: the server didn't come up: $(cat "$ST/fakesock.err" 2>/dev/null)" >&2; exit 1;;
  stop)
    p=$(cat "$ST/fakesock.pid" 2>/dev/null); rm -f "$ST/fakesock.pid"
    [ -n "$p" ] && kill "$p" 2>/dev/null
    for _ in $(seq 1 40); do ls "$ALEPH_FAKE_SOCK"/*.sock >/dev/null 2>&1 || exit 0; sleep 0.05; done
    rm -f "$ALEPH_FAKE_SOCK"/*.sock;;
  *) echo "usage: fakesock.sh start [host…] | stop" >&2; exit 2;;
esac
