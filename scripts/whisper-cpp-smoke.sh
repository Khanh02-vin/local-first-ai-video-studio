#!/usr/bin/env bash
# Real CPU smoke test for the bundled whisper.cpp binary + ggml model.
# This is intentionally separate from unit tests: it exercises the dynamic
# libraries, CPU instruction set, model header and CLI invocation together.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RES="$ROOT/apps/desktop/src-tauri/resources"
FFMPEG="$RES/ffmpeg"
BIN="$RES/whisper-cpp/whisper-cli"
MODEL="$RES/models/ggml-tiny.bin"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/lfai-whisper-smoke-XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

[ -x "$FFMPEG" ] || { echo "WHISPER_CPP_SMOKE: bundled ffmpeg missing" >&2; exit 1; }
[ -x "$BIN" ] || { echo "WHISPER_CPP_SMOKE: bundled whisper-cli missing" >&2; exit 1; }
[ -s "$MODEL" ] || { echo "WHISPER_CPP_SMOKE: ggml model missing" >&2; exit 1; }

"$FFMPEG" -y -v error -f lavfi -i anullsrc=r=16000:cl=mono -t 0.5 -c:a pcm_s16le "$TMP/silence.wav"
LD_LIBRARY_PATH="$RES/whisper-cpp${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  timeout 30 "$BIN" -m "$MODEL" -f "$TMP/silence.wav" -t 1 -nt -ojf -of "$TMP/result" >/dev/null
[ -s "$TMP/result.json" ] || { echo "WHISPER_CPP_SMOKE: whisper-cli produced no JSON" >&2; exit 1; }

printf 'whisper.cpp CPU smoke: ok (binary, shared libraries, model, 0.5s audio)\n'
