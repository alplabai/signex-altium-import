#!/usr/bin/env bash
# Re-vendor crates/signex-types from a checkout of alplabai/signex.
#
#   scripts/sync-signex-types.sh ../signex
#
# Why vendored rather than a git dependency: cargo fetches a git
# dependency's submodules, and the main repo carries a private submodule
# (docs/internal) that CI runners cannot authenticate to.
set -euo pipefail
SRC="${1:?usage: $0 <path-to-signex-checkout>}"
DEST="$(cd "$(dirname "$0")/.." && pwd)/crates/signex-types"
test -f "$SRC/crates/signex-types/Cargo.toml" || { echo "not a signex checkout: $SRC" >&2; exit 1; }
rm -rf "$DEST"
cp -r "$SRC/crates/signex-types" "$DEST"
git -C "$SRC" rev-parse HEAD > "$DEST/UPSTREAM_COMMIT"
echo "vendored signex-types @ $(cat "$DEST/UPSTREAM_COMMIT")"
