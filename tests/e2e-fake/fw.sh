#!/usr/bin/env bash
# Run one command in the fake world (see env.sh), e.g. for a quick look:
#   tests/e2e-fake/fw.sh [--reset] aleph setup --plain
ROOT=$(cd "$(dirname "$0")" && pwd)
source "$ROOT/env.sh" "${FW_BUILD:-debug}"
if [ "${1:-}" = --reset ]; then "$ROOT/reset.sh"; shift; fi
"$@"
