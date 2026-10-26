#!/bin/sh
# box:    sshd + a headless herdr server for `dev`, then wait
# client: run the checks as `dev` (aleph → herdr --machine box → ssh)
# other:  run it as dev (e.g. `bash` for poking around)
set -eu
role=${1:-box}
case $role in
  box)
    /usr/sbin/sshd -E /tmp/sshd.log
    su dev -c 'cd ~ && nohup herdr server >/tmp/herdr-server.log 2>&1 &'
    i=0
    until su dev -c 'herdr workspace list' >/dev/null 2>&1; do
      i=$((i + 1))
      [ $i -lt 60 ] || { echo "herdr server didn't come up" >&2; cat /tmp/herdr-server.log >&2; exit 1; }
      sleep 0.5
    done
    echo "box ready: sshd + herdr server"
    exec tail -f /dev/null
    ;;
  client)
    shift
    exec su dev -c "/usr/local/bin/checks $*"
    ;;
  *)
    exec su dev -c "$*"
    ;;
esac
