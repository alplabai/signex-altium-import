#!/usr/bin/env bash
# Re-vendor signex-types, signex-sketch and signex-library from a checkout
# of alplabai/signex:
#
#   scripts/sync-signex-crates.sh ../signex
#
# Vendored rather than a git dependency: cargo fetches a git dependency's
# submodules, and the main repo carries a private submodule CI runners
# cannot authenticate to.
#
# signex-library is trimmed on the way in (scripts/trim_manifest.py): its
# optional adapters (search index, git, database, distributors) and their
# dependencies are dropped, and so are its dev-dependencies and tests, which
# the main repo already runs. Only the file formats are needed here.
set -euo pipefail
SRC="${1:?usage: $0 <path-to-signex-checkout>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
test -f "$SRC/crates/signex-types/Cargo.toml" || { echo "not a signex checkout: $SRC" >&2; exit 1; }
COMMIT="$(git -C "$SRC" rev-parse HEAD)"
for crate in signex-types signex-sketch signex-library; do
  rm -rf "$ROOT/crates/$crate"
  cp -r "$SRC/crates/$crate" "$ROOT/crates/$crate"
  echo "$COMMIT" > "$ROOT/crates/$crate/UPSTREAM_COMMIT"
done
for crate in signex-sketch signex-library; do
  rm -rf "$ROOT/crates/$crate/tests" "$ROOT/crates/$crate/benches" "$ROOT/crates/$crate/examples"
  python3 "$ROOT/scripts/trim_manifest.py" "$ROOT/crates/$crate/Cargo.toml"
done
echo "vendored signex-types, signex-sketch, signex-library @ $COMMIT"
