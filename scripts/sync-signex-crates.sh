#!/usr/bin/env bash
# Re-vendor signex-expr, signex-types, signex-sketch and signex-library from a checkout
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
for crate in signex-expr signex-types signex-sketch signex-library; do
  rm -rf "$ROOT/crates/$crate"
  cp -r "$SRC/crates/$crate" "$ROOT/crates/$crate"
  echo "$COMMIT" > "$ROOT/crates/$crate/UPSTREAM_COMMIT"
done
for crate in signex-expr signex-sketch signex-library; do
  rm -rf "$ROOT/crates/$crate/tests" "$ROOT/crates/$crate/benches" "$ROOT/crates/$crate/examples"
  python3 "$ROOT/scripts/trim_manifest.py" "$ROOT/crates/$crate/Cargo.toml"
done
# The trim removes the only users of some items in these crates, so rustc
# would warn about them under -D warnings. Upstream lints them in full; here
# the two lints are allowed inside the trimmed crates only (a [lints] table
# would be overridden by the workspace one).
for crate in signex-expr signex-sketch signex-library; do
  lib="$ROOT/crates/$crate/src/lib.rs"
  { printf '%s\n' '// Added by scripts/sync-signex-crates.sh: trim_manifest.py drops the optional' \
      '// adapters, which leaves some imports and items without users.' \
      '#![allow(dead_code, unused_imports)]'; cat "$lib"; } > "$lib.new"
  mv "$lib.new" "$lib"
done
# One vendored signex-types test reads a sample schematic from signex-app by
# a relative path; copy just that file so the test runs here unchanged.
ASSET=crates/signex-app/assets/sample-project/linear-regulator.snxsch
mkdir -p "$ROOT/$(dirname "$ASSET")"
cp "$SRC/$ASSET" "$ROOT/$ASSET"
echo "vendored signex-expr, signex-types, signex-sketch, signex-library @ $COMMIT"
