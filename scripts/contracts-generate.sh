#!/usr/bin/env bash
# Generates the TypeScript contract from the Rust #[typeshare] declarations.
#
#   scripts/contracts-generate.sh                      # rewrites the committed file
#   scripts/contracts-generate.sh --output-file PATH    # writes somewhere else (tests)
#
# The typeshare CLI is pinned and installed into .tools/ (gitignored) so every machine
# and CI run generates from the same version instead of whatever is on PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="1.13.0"
TOOLS_DIR="$PWD/.tools/tsroot"
BIN="$TOOLS_DIR/bin/typeshare"
OUT="packages/contracts/generated/generated.ts"

if [ "${1:-}" = "--output-file" ]; then
  OUT="$2"
fi

# /tmp can be small or quota-limited; keep cargo's scratch in a writable place.
export TMPDIR="${TMPDIR:-/dev/shm}"
if [ ! -d "$TMPDIR" ]; then export TMPDIR="$PWD/.tools/tmp"; fi
mkdir -p "$TMPDIR"

if [ ! -x "$BIN" ]; then
  echo "installing typeshare-cli $VERSION into .tools/ (first run only)" >&2
  cargo install typeshare-cli --version "$VERSION" --locked --root "$TOOLS_DIR" >/dev/null
fi

"$BIN" --lang typescript --output-file "$OUT" apps/desktop/src-tauri/crates/render-engine/src

if [ "$OUT" = "packages/contracts/generated/generated.ts" ]; then
  echo "contracts regenerated: $OUT"
else
  echo "contracts generated to: $OUT" >&2
fi
