#!/bin/sh
# Copy the bundled logo PNGs into this extension's assets, which is all
# Raycast ships. tools/logogen writes them to the repo's assets/logos/png
# (<id>@64.png / <id>@128.png, optionally in os/, gpu/, agents/, services/).
# Without them the extension shows each brand's favicon, then a coloured dot.
# Runs before `npm run build` and `npm run dev`; the copies aren't committed.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
src="$here/../../assets/logos/png"
dst="$here/assets/logos"
if [ ! -d "$src" ]; then
  echo "logos: none bundled yet ($src) — favicons and coloured dots stand in"
  exit 0
fi
mkdir -p "$dst"
cp -R "$src/." "$dst/"
echo "logos: $(find "$dst" -name '*.png' | wc -l | tr -d ' ') PNGs from $src"
