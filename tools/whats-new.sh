#!/bin/sh
# Start a What's new entry for the next deploy: the commits since
# docs/whats-new.toml last changed (TODO-only commits left out), showcase ones
# in docs/whats-new-showcase.toml. Write the notes by hand after.
#   tools/whats-new.sh "Title of this deploy"
set -eu
cd "$(dirname "$0")/.."
title=${1:?usage: tools/whats-new.sh "title"}
since=$(git log -1 --format=%H -- docs/whats-new.toml)
day=$(date +%Y-%m-%d)
# the letter after the day's last one (a, b, …)
id="$day-a"
for l in a b c d e f g h i j k l m n o p q r s t u v w x y; do
  grep -q "^id = \"$day-$l\"" docs/whats-new.toml && id="$day-$(echo "$l" | tr a-y b-z)"
done
q() { sed 's/\\/\\\\/g; s/"/\\"/g; s/^/  "/; s/$/",/'; }
subjects=$(git log --reverse --format=%s "$since"..HEAD | grep -v "^TODO" || true)
# showcase's own commits: "showcase: …", or "… (showcase builds)" — not
# every subject that names it in passing
main=$(printf '%s\n' "$subjects" | grep -Ev "^showcase|\(showcase builds\)" | grep . | q || true)
show=$(printf '%s\n' "$subjects" | grep -E "^showcase|\(showcase builds\)" | q || true)
entry() { printf '[[entry]]\nid = "%s"\n' "$id"; [ -n "$2" ] && printf 'date = "%s"\ntitle = "%s"\n' "$day" "$2"; printf 'notes = [\n  "…",\n]\ncommits = [\n%s\n]\n\n' "$1"; }
insert() { # before the first entry (after the header comments)
  f=$1; body=$2; tmp=$(mktemp)
  n=$(grep -n '^\[\[entry\]\]' "$f" | head -n 1 | cut -d: -f1)
  if [ -n "$n" ]; then
    head -n $((n - 1)) "$f" > "$tmp"; printf '%s\n' "$body" >> "$tmp"; tail -n +"$n" "$f" >> "$tmp"
  else
    cat "$f" > "$tmp"; printf '\n%s\n' "$body" >> "$tmp"
  fi
  mv "$tmp" "$f"
}
insert docs/whats-new.toml "$(entry "$main" "$title")
"
[ -n "$show" ] && insert docs/whats-new-showcase.toml "$(entry "$show" "")
"
echo "started $id: write its notes in docs/whats-new.toml (and docs/whats-new-showcase.toml)"
