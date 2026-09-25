#!/usr/bin/env bash
# Builds the Tauri installers for the platform this script runs on,
# with the right llama.cpp binary bundled, and signs them if secrets
# are available.
#
# Run this on the target OS:
#   macOS   → .dmg / .app.tar.gz        (Developer ID + notarize if secrets set)
#   Windows → .msi / .exe (NSIS)        (Tauri signing key if set)
#
# It reuses fetch-bundled-assets.sh, which already pulls the per-platform
# llama.cpp release (llama.cpp b11160) into resources/llama/. This script
# adds the sign + notarize + verify steps on top.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RES="$ROOT/apps/desktop/src-tauri/resources"

echo "==> Fetching platform resources (llama.cpp, node, ffmpeg, whisper weights)"
bash "$ROOT/scripts/fetch-bundled-assets.sh"

case "$(uname -s)" in
  Darwin)
    echo "==> Building macOS bundle"
    cd "$ROOT/apps/desktop"
    npm install --no-fund --no-audit

    # Sign with Developer ID when the cert is available; fall back to
    # an ad-hoc signature so the build still succeeds on unsigned runners.
    if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
      echo "    Signing with identity: $APPLE_SIGNING_IDENTITY"
      npx tauri build --bundles app,dmg
      codesign --verify --deep --strict --verbose=2 "$ROOT/apps/desktop/src-tauri/target/release/bundle/macos/Local-first AI Video Studio.app"
    else
      echo "    No APPLE_SIGNING_IDENTITY; building ad-hoc (unsigned)."
      npx tauri build --bundles app,dmg
    fi

    # Notarize when the full Apple ID flow is configured.
    if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
      APP="$ROOT/apps/desktop/src-tauri/target/release/bundle/macos/Local-first AI Video Studio.app"
      NOTARIZE_ZIP="$ROOT/apps/desktop/src-tauri/target/release/bundle/macos/notarize.zip"
      echo "==> Notarizing $APP"
      ditto -c -k --keepParent "$APP" "$NOTARIZE_ZIP"
      xcrun notarytool submit "$NOTARIZE_ZIP" \
        --team-id "$APPLE_TEAM_ID" \
        --username "$APPLE_ID" \
        --password "$APPLE_APP_SPECIFIC_PASSWORD" \
        --wait
      echo "    Notarization accepted."
    else
      echo "    No APPLE_ID/APPLE_APP_SPECIFIC_PASSWORD; skipping notarization."
    fi

    echo "==> Done: $(ls "$ROOT/apps/desktop/src-tauri/target/release/bundle/macos")"
    ;;

  MINGW*|MSYS*|CYGWIN*|Windows*)
    echo "==> Building Windows installer"
    cd "$ROOT/apps/desktop"
    npm install --no-fund --no-audit
    # Tauri signs the msi/nsis when TAURI_SIGNING_PRIVATE_KEY is set;
    # otherwise the build is unsigned but still valid for local testing.
    npx tauri build --bundles msi,nsis
    echo "==> Done: $(ls "$ROOT/apps/desktop/src-tauri/target/release/bundle/msi" "$ROOT/apps/desktop/src-tauri/target/release/bundle/nsis")"
    ;;

  *)
    echo "Unsupported platform for this script ($(uname -s)); run on macOS or Windows." >&2
    exit 1
    ;;
esac
