#!/bin/sh
# Render the Homebrew formula from a release's SHA256SUMS.
#
#   packaging/homebrew/render.sh [--version V] [--repo owner/name] [--template F] [-o out.rb] SHA256SUMS
#
# --version defaults to Cargo.toml's version, --repo to the GitHub `origin` remote.
# SHA256SUMS is `<sha256>  aleph-<target>.tar.gz` lines (sha256sum / shasum -a 256),
# and must cover all four release targets. Writes to stdout unless -o is given.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
template=$here/aleph.rb
version= repo= out= sums=

die() { printf 'error: %s\n' "$1" >&2; [ -z "${2-}" ] || printf 'hint: %s\n' "$2" >&2; exit 2; }

while [ $# -gt 0 ]; do
  case $1 in
    --version) [ $# -ge 2 ] || die "--version needs a value"; version=${2#v}; shift ;;
    --repo) [ $# -ge 2 ] || die "--repo needs owner/name"; repo=$2; shift ;;
    --template) [ $# -ge 2 ] || die "--template needs a file"; template=$2; shift ;;
    -o) [ $# -ge 2 ] || die "-o needs a file"; out=$2; shift ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) [ -z "$sums" ] || die "only one SHA256SUMS file"; sums=$1 ;;
  esac
  shift
done

[ -n "$sums" ] || die "no SHA256SUMS given" "render.sh --version 1.0.0 --repo owner/aleph dist/SHA256SUMS"
[ -r "$sums" ] || die "can't read $sums"
[ -r "$template" ] || die "can't read template $template"

if [ -z "$version" ]; then
  version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
fi
case $version in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) die "version '$version' isn't semver" "--version 1.2.3" ;;
esac

if [ -z "$repo" ]; then
  url=$(git -C "$root" remote get-url origin 2>/dev/null || true)
  repo=$(printf '%s\n' "$url" | sed -n 's#^.*github\.com[:/]\([^/]*/[^/]*\)$#\1#p' | sed 's/\.git$//')
fi
printf '%s\n' "$repo" | grep -Eq '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' ||
  die "no GitHub repo (got '$repo')" "--repo owner/aleph"

# sha256 for one target, from the SHA256SUMS lines (`*` binary-mode marker allowed)
sha() {
  awk -v f="aleph-$1.tar.gz" '{ n = $2; sub(/^\*/, "", n) } n == f { print $1; exit }' "$sums"
}

script=
for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
  h=$(sha "$t")
  printf '%s\n' "$h" | grep -Eq '^[0-9a-f]{64}$' || die "no sha256 for aleph-$t.tar.gz in $sums"
  key=SHA256_$(printf '%s' "$t" | tr 'a-z-' 'A-Z_')
  script="$script s|@$key@|$h|g;"
done
script="$script s|@VERSION@|$version|g; s|@REPO@|$repo|g;"

tmp=$(mktemp "${TMPDIR:-/tmp}/aleph-rb.XXXXXX")
trap 'rm -f "$tmp"' EXIT
sed "$script" "$template" > "$tmp"
# anything left unrendered is a template bug
if grep -n '@[A-Z0-9_]*@' "$tmp" >&2; then
  die "unrendered placeholders left in the formula"
fi
if [ -n "$out" ]; then
  cp "$tmp" "$out.tmp.$$" && mv "$out.tmp.$$" "$out"
else
  cat "$tmp"
fi
