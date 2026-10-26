#!/usr/bin/env bash
# Record a full-screen session for visual QA (Linux util-linux `script`).
#   rec.sh <name> <cols> <rows> "<keys>" <command…>
# keys: one "<delay-seconds> <printf-format>" per line, e.g. $'1.5 j\n0.5 \\r'
# Then render: python3 snap.py .run/<name>.ts .run/<name>.tm <cols> <rows> end out.png [--text]
ROOT=$(cd "$(dirname "$0")" && pwd)
name=$1 cols=$2 rows=$3 keys=$4; shift 4
( [ -n "$keys" ] && while read -r d k; do sleep "$d"; printf "$k"; done <<<"$keys"; sleep 1.2 ) |
  script -q --log-timing "$ROOT/.run/$name.tm" --log-out "$ROOT/.run/$name.ts" \
    -c "stty cols $cols rows $rows; env PATH=$PATH HOME=$HOME ST=$ST RHOME=$RHOME XDG_CACHE_HOME=$XDG_CACHE_HOME XDG_STATE_HOME=$XDG_STATE_HOME XDG_DATA_HOME=$XDG_DATA_HOME COLORTERM=truecolor TERM=xterm-256color $*" >/dev/null 2>&1
