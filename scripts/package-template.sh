#!/bin/zsh
# Package the repo into a clean .zip to hand out as a template.
#
# Uses `git archive`, so the zip contains ONLY committed, tracked files —
# never target/, dist/, .git/, .claude/, .DS_Store, or anything else local.
# Commit your changes first, then run this.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
OUT="sticky-terminal-template-${VERSION}.zip"

rm -f "$OUT"
git archive --format=zip --prefix=sticky-terminal/ -o "$OUT" HEAD

echo
echo "Built: $OUT"
echo "Contains only committed, tracked files — safe to share."
