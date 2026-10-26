#!/bin/sh
# A build without the `showcase` feature must carry none of it: no words of
# its vocabulary, no state file name, no text of its command. Run on a
# release binary (default: target/release/aleph); prints what it finds and
# fails when it finds anything.
set -eu
bin=${1:-target/release/aleph}
[ -x "$bin" ] || { echo "no binary at $bin (cargo build --release)"; exit 2; }
found=0
for w in showcase.json magrathea vogsphere slartibartfast trillian heartofgold mostlyharmless \
         improbability "space references" "the demo world" \
         showcase.flair keycast starfield banner.js banner.css sc-ribbon \
         "the point that contains all points" "never tell me the odds" "sessions and counting" \
         "nothing real to attach" "isn't masked" \
         showcase.palette showcase.sounds "System/Library/Sounds" canberra-gtk-play supernova \
         showcase.cast_dir showcase.watermark "Showcase: poster" asciinema al-screen sc-mark \
         "be right back" "screen hidden for the demo" showcase_safety showcase.panic_key "alien script" \
         showcase.video_frame showcase.cast_keep palettegen libx264 "MP4 + GIF" "only the .cast was saved" \
         --after-stop ALEPH_ACCENT Menlo.ttc "brew install ffmpeg" \
         showcase.story showcase.warp relay-soak "Earth Mk II" clock.json "time warp" \
         showcase.tour "start the tour" "autopilot tour" "the preview follows the selection" \
         showcase.sync presenter.get presenter.set "following the presenter"; do
  if strings -a "$bin" | grep -qi -- "$w"; then
    echo "found in $bin: $w"
    found=1
  fi
done
[ $found -eq 0 ] && echo "ok: no showcase code in $bin"
exit $found
