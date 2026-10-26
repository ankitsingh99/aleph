#!/bin/sh
# Package Aleph to take to another computer: one zip of a fresh git repository
# — what's committed now as its one commit, by a neutral author (none of this
# repository's history, its authors or its remote), no build output and no
# installed dependencies (cargo and npm fetch those again).
#   tools/package.sh [OUT_DIR]          → OUT_DIR/aleph-<version>.zip (default: ..)
# On the new computer: unzip, cd aleph-<version>, ./install.sh (README › Install).
set -eu
cd "$(dirname "$0")/.."
out=${1:-..}
mkdir -p "$out"
out=$(cd "$out" && pwd)
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "package.sh: commit first — the zip is what's committed" >&2
  exit 1
fi
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)
name="aleph-$version"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# the files as committed (git archive: no worktrees, build output or ignored
# files), then a new repository around them with one commit
mkdir "$tmp/$name"
git archive --format=tar HEAD | tar -x -C "$tmp/$name"
(
  cd "$tmp/$name"
  git init --quiet --initial-branch=main
  # no background gc: it would pack the objects while zip reads them
  git add -A
  GIT_AUTHOR_NAME=Aleph GIT_AUTHOR_EMAIL=aleph@example.com \
    GIT_COMMITTER_NAME=Aleph GIT_COMMITTER_EMAIL=aleph@example.com \
    git -c gc.auto=0 -c maintenance.auto=false \
      commit --quiet --no-gpg-sign -m "Aleph $version"
)
rm -f "$out/$name.zip"
(cd "$tmp" && zip -qr "$out/$name.zip" "$name")
size=$(du -h "$out/$name.zip" | cut -f1)
echo "✓ $out/$name.zip ($size): unzip, cd $name, ./install.sh"
