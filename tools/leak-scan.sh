#!/usr/bin/env bash
# leak-scan.sh — nothing private leaves this machine: credentials of any
# kind, and the personal terms in a private list kept outside the repository.
#
#   tools/leak-scan.sh history [REF]    every file of every commit of REF (main),
#                                       its messages, authors and file names
#   tools/leak-scan.sh staged           what's staged now (the pre-commit hook)
#   tools/leak-scan.sh range A..B       the commits a push sends (the pre-push hook)
#   tools/leak-scan.sh tree             the files as they are now (tracked ones)
#   tools/leak-scan.sh binary FILE…     a built binary's strings (releases)
#
# The personal terms: $ALEPH_LEAK_TERMS, else ~/.config/aleph/leak-terms.txt
# (one a line; a space, dot, dash or underscore matches any of them or none;
# "w:" for a whole word; # comments). Without it (CI), only the generic
# patterns. Known fakes and published test vectors: tools/leak-allow.txt.
# Exit 1 when anything is found, listing where; 0 when clean.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 2
mode=${1:-history}; shift || true

terms_file=${ALEPH_LEAK_TERMS:-$HOME/.config/aleph/leak-terms.txt}
allow_file=tools/leak-allow.txt

# credentials and machine paths, by their shapes (case-sensitive)
GENERIC=(
  'sk-ant-[A-Za-z0-9_-]{8,}'
  'sk-or-v1-[A-Za-z0-9]{8,}'
  'sk-proj-[A-Za-z0-9_-]{8,}'
  'gh[pousr]_[A-Za-z0-9]{20,}'
  'github_pat_[A-Za-z0-9_]{20,}'
  'xox[abposr]-[A-Za-z0-9-]{10,}'
  'AKIA[0-9A-Z]{16}'
  'AIza[0-9A-Za-z_-]{35}'
  'BEGIN [A-Z ]*PRIVATE KEY'
  'eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}'
  'hooks\.slack\.com/services/T[A-Za-z0-9]+/B[A-Za-z0-9]+/[A-Za-z0-9]{12,}'
  'discord(app)?\.com/api/webhooks/[0-9]+/[A-Za-z0-9_-]{20,}'
  '/Users/[A-Za-z0-9._-]{2,}'
  '/home/[A-Za-z0-9._-]{2,}'
  '[A-Za-z0-9._%+-]+@[A-Za-z][A-Za-z0-9-]*\.[A-Za-z0-9.-]*[A-Za-z]{2,}'
)

# a term as a regex: separators loose, everything else literal, lowercase
term_re() {
  local t=$1 whole=0
  case $t in w:*) whole=1; t=${t#w:} ;; esac
  t=$(printf '%s' "$t" | tr '[:upper:]' '[:lower:]' | sed -e 's/[][\\^$*+?(){}|/]/\\&/g' -e 's/[ _.-]/[-_. ]?/g')
  if (( whole )); then printf '(^|[^a-z0-9])%s([^a-z0-9]|$)' "$t"; else printf '%s' "$t"; fi
}
TERMS=()
if [[ -f $terms_file ]]; then
  while IFS= read -r l; do
    l=${l%%#*}; l=$(printf '%s' "$l" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
    [[ -n $l ]] && TERMS+=("$(term_re "$l")")
  done < "$terms_file"
else
  echo "leak-scan: no private terms ($terms_file): generic patterns only" >&2
fi

ALLOW=()
if [[ -f $allow_file ]]; then
  while IFS= read -r l; do
    [[ -z $l || $l == \#* ]] && continue
    ALLOW+=("$l")
  done < "$allow_file"
fi

# lines in ("where<TAB>text") → the findings, allowed ones left out
judge() {
  local d rc
  d=$(mktemp -d "${TMPDIR:-/tmp}/leak-scan.XXXXXX") || exit 2
  printf '%s\n' "${GENERIC[@]}" > "$d/gen"
  printf '%s\n' "${TERMS[@]+"${TERMS[@]}"}" > "$d/terms"
  printf '%s\n' "${ALLOW[@]+"${ALLOW[@]}"}" > "$d/allow"
  awk -F '\t' -v gf="$d/gen" -v tf="$d/terms" -v af="$d/allow" '
    BEGIN {
      while ((getline l < gf) > 0) G[++ng] = l
      while ((getline l < tf) > 0) T[++nt] = l
      while ((getline l < af) > 0) A[++na] = l
    }
    {
      where = $1; text = substr($0, length($1) + 2); hit = ""
      # known fakes and published test values are cut out (not the whole
      # line: a real one beside them still shows)
      t = text
      for (i = 1; i <= na; i++) if (A[i] != "") while ((p = index(t, A[i])) > 0) t = substr(t, 1, p - 1) "\001" substr(t, p + length(A[i]))
      low = tolower(t)
      for (i = 1; i <= ng; i++) if (G[i] != "" && match(t, G[i])) { hit = "pattern"; m = substr(t, RSTART, RLENGTH); break }
      if (hit == "") for (i = 1; i <= nt; i++) if (T[i] != "" && match(low, T[i])) { hit = "private term"; m = substr(t, RSTART, RLENGTH); break }
      if (hit == "") next
      if (seen[where SUBSEP m]++) next
      found++
      s = text; gsub(/^[ \t]+/, "", s)
      # what matched, in the open only when it is a home folder (a key-like
      # match stays masked: CI logs are public); then the text around it, not
      # the start of a long line (the strings of a binary run for thousands)
      shown = (hit == "pattern" && m !~ /^\/(Users|home)\//) ? substr(m, 1, 6) "…(" length(m) ")" : m
      p = index(s, m)
      if (p > 0) s = substr(s, 1, p - 1) "«" shown "»" substr(s, p + length(m))
      if (length(s) > 140) {
        st = (p > 60) ? p - 60 : 1
        s = (st > 1 ? "…" : "") substr(s, st, 140) "…"
      }
      printf "%s: [%s] %s\n", where, hit, s
    }
    END { exit found > 0 }'
  rc=$?
  rm -rf "$d"
  return $rc
}

# every line of a commit's text files, as "commit:path:line<TAB>text"
commit_lines() {
  git grep -I -n -h --full-name -e '' "$1" -- . ':!Cargo.lock' ':!**/package-lock.json' ':!src/logo_data.rs' 2>/dev/null |
    awk -v c="${1:0:7}" '{ i = index($0, ":"); rest = substr($0, i + 1); j = index(rest, ":"); k = index(substr(rest, j + 1), ":")
      printf "%s:%s:%s\t%s\n", c, substr(rest, 1, j - 1), substr(rest, j + 1, k - 1), substr(rest, j + k + 1) }'
}

meta_lines() { # messages, authors, committers and file names of commits
  # (a GitHub no-reply identity is meant to be public: by its shape, never by name here)
  git log --format='%h%x09%an <%ae> / %cn <%ce>%n%h%x09%B' "$@" |
    sed -E 's/<[0-9]+\+[A-Za-z0-9-]+@users\.noreply\.github\.com>/<github-noreply>/g' |
    awk -F '\t' 'NF > 1 { print } NF == 1 && length($0) { print h "\t" $0 } NF > 1 { h = $1 }'
  git log --name-only --format='%h' "$@" | awk 'NF { if ($0 ~ /^[0-9a-f]{7,}$/) h = $0; else print h ":path\t" $0 }'
}

case $mode in
  history)
    ref=${1:-main}
    { for c in $(git rev-list "$ref"); do commit_lines "$c"; done; meta_lines "$ref"; } | judge
    ;;
  tree)
    git grep -I -n --full-name -e '' -- . ':!Cargo.lock' ':!**/package-lock.json' ':!src/logo_data.rs' |
      awk '{ i = index($0, ":"); j = index(substr($0, i + 1), ":"); printf "%s\t%s\n", substr($0, 1, i + j - 1), substr($0, i + j + 1) }' | judge
    ;;
  staged)
    { git diff --cached -U0 --no-color | awk '/^\+\+\+ b\// { f = substr($0, 7); next } /^\+[^+]/ { print f ":staged\t" substr($0, 2) }'
      git diff --cached --name-only | awk '{ print "staged:path\t" $0 }'; } | judge
    ;;
  range)
    r=${1:?range A..B}
    { git log -p --no-color --format='%h' "$r" | awk '/^[0-9a-f]{7,}$/ { h = $0; next } /^\+\+\+ b\// { f = substr($0, 7); next } /^\+[^+]/ { print h ":" f "\t" substr($0, 2) }'
      meta_lines "$r"; } | judge
    ;;
  binary)
    (( $# )) || { echo "leak-scan binary FILE…" >&2; exit 2; }
    # (runs of 8 bytes and up: shorter ones are mostly compressed data, where a
    # 3-letter term turns up by chance; real text sits in long runs)
    # A binary keeps its strings end to end, so a bare home-folder prefix in the
    # code runs into whatever string follows it: a home folder here is a path, a
    # short name and a slash after it (a build machine's checkout still is)
    for i in "${!GENERIC[@]}"; do
      case ${GENERIC[$i]} in
        /Users/*) GENERIC[$i]='/Users/[A-Za-z0-9._-]{2,32}/' ;;
        /home/*) GENERIC[$i]='/home/[A-Za-z0-9._-]{2,32}/' ;;
      esac
    done
    for b in "$@"; do strings -a -n 8 "$b" | awk -v b="$(basename "$b")" '{ print b "\t" $0 }'; done | judge
    ;;
  *) sed -n '2,15p' "$0"; exit 2 ;;
esac
status=$?
(( status == 0 )) && echo "leak-scan: clean ($mode)" >&2
exit $status
