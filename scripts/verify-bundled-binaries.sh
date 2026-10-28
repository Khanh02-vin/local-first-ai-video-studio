#!/usr/bin/env bash
# Verifies that the bundled ffmpeg/ffprobe binaries in apps/desktop/src-tauri/resources/
# are real executables (not empty placeholders from LIGHTWEIGHT mode).
# Fails fast if a binary is missing or cannot run.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RES="$ROOT/apps/desktop/src-tauri/resources"

fail() { echo "ERROR: $*" >&2; exit 1; }

# Check ffmpeg
if [ ! -s "$RES/ffmpeg" ] && [ ! -s "$RES/ffmpeg.exe" ]; then
  fail "resources/ffmpeg is missing or empty (placeholder). Run scripts/fetch-bundled-assets.sh first."
fi
if [ -f "$RES/ffmpeg" ]; then
  "$RES/ffmpeg" -version >/dev/null 2>&1 || fail "resources/ffmpeg cannot run (not a valid binary)."
  echo "✓ resources/ffmpeg is a working binary"
elif [ -f "$RES/ffmpeg.exe" ]; then
  "$RES/ffmpeg.exe" -version >/dev/null 2>&1 || fail "resources/ffmpeg.exe cannot run (not a valid binary)."
  echo "✓ resources/ffmpeg.exe is a working binary"
fi

# Check ffprobe
if [ ! -s "$RES/ffprobe" ] && [ ! -s "$RES/ffprobe.exe" ]; then
  fail "resources/ffprobe is missing or empty (placeholder). Run scripts/fetch-bundled-assets.sh first."
fi
if [ -f "$RES/ffprobe" ]; then
  "$RES/ffprobe" -version >/dev/null 2>&1 || fail "resources/ffprobe cannot run (not a valid binary)."
  echo "✓ resources/ffprobe is a working binary"
elif [ -f "$RES/ffprobe.exe" ]; then
  "$RES/ffprobe.exe" -version >/dev/null 2>&1 || fail "resources/ffprobe.exe cannot run (not a valid binary)."
  echo "✓ resources/ffprobe.exe is a working binary"
fi

# Check whisper.cpp CLI + ggml model
WHISPER_CLI="$RES/whisper-cpp/whisper-cli"
[ -x "$WHISPER_CLI" ] || [ -x "$RES/whisper-cpp/whisper-cli.exe" ] || fail "resources/whisper-cpp/whisper-cli is missing, empty, or not executable."
if [ -x "$WHISPER_CLI" ]; then
  LD_LIBRARY_PATH="$RES/whisper-cpp${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$WHISPER_CLI" --help >/dev/null 2>&1 || fail "resources/whisper-cpp/whisper-cli cannot run."
  echo "✓ resources/whisper-cpp/whisper-cli is a working binary"
else
  "$RES/whisper-cpp/whisper-cli.exe" --help >/dev/null 2>&1 || fail "resources/whisper-cpp/whisper-cli.exe cannot run."
  echo "✓ resources/whisper-cpp/whisper-cli.exe is a working binary"
fi
[ -s "$RES/models/ggml-tiny.bin" ] || fail "resources/models/ggml-tiny.bin is missing or empty."
echo "✓ resources/models/ggml-tiny.bin is present"

echo "All bundled binaries verified."
