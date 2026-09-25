#!/usr/bin/env bash
# Simulates the full analysis → render pipeline the Tauri app runs:
#   1. Load runtime.env (whisper path, model, strategy)
#   2. Make a synthetic video with a real voiceover (ffmpeg → /tmp/talk.mp4)
#   3. probe → extract audio → whisper transcribe → highlight → render 9:16
#   4. Verify the output MP4 exists and is a valid 1080x1920 file
set -euo pipefail

STATE="${LOCAL_FIRST_STATE_DIR:-$HOME/.cache/local-first-ai-video-studio}"
ENV_FILE="$STATE/runtime.env"

echo "==> Loading runtime.env"
[[ -f "$ENV_FILE" ]] || { echo "    runtime.env missing — bootstrap step not run?"; exit 1; }
# shellcheck disable=SC1090
source "$ENV_FILE"
echo "    whisper=$WHISPER_COMMAND model=$WHISPER_MODEL strategy=$HIGHLIGHT_STRATEGY"

# --- Make a 12-second source video with a sine "voice" tone so whisper
#     has something to work with. The audio is intentionally a steady tone
#     (not silence) so the pipeline's whisper step finds at least one word.
echo "==> Making synthetic source /tmp/talk.mp4 (12s, 1280x720)"
ffmpeg -y -f lavfi -i "testsrc=duration=12:size=1280x720:rate=30" \
       -f lavfi -i "sine=frequency=600:duration=12" \
       -shortest -c:v libx264 -preset ultrafast -c:a aac -b:a 96k \
       /tmp/talk.mp4

echo "==> Running the same pipeline the Tauri app runs"
cd /repo
# Tauri resolves node via the bundled resources (node_bin() in main.rs): a
# clean machine has no system node, only the one shipped in the installer.
# Fall back to PATH so the script also works on dev machines that never ran
# scripts/fetch-bundled-assets.sh.
NODE_BIN="/repo/apps/desktop/src-tauri/resources/node/bin/node"
[[ -x "$NODE_BIN" ]] || NODE_BIN="$(command -v node)" || { echo "    node not found (neither bundled nor on PATH)"; exit 1; }
# FFmpeg/ffprobe: prefer the bundled resources (what the deb ships); on a
# clean machine the apt install in the Dockerfile is what the app falls back
# to when the resource is absent.
FFMPEG_BIN="/repo/apps/desktop/src-tauri/resources/ffmpeg";   [[ -x "$FFMPEG_BIN" ]]  || FFMPEG_BIN="ffmpeg"
FFPROBE_BIN="/repo/apps/desktop/src-tauri/resources/ffprobe"; [[ -x "$FFPROBE_BIN" ]] || FFPROBE_BIN="ffprobe"
echo "    node=$NODE_BIN ffmpeg=$FFMPEG_BIN"
LOCAL_FIRST_STATE_DIR="$STATE" WHISPER_COMMAND="$WHISPER_COMMAND" WHISPER_MODEL="$WHISPER_MODEL" \
FFMPEG_PATH="$FFMPEG_BIN" FFPROBE_PATH="$FFPROBE_BIN" \
  "$NODE_BIN" --experimental-strip-types scripts/run-local-demo.ts /tmp/talk.mp4 /tmp/short-clean.mp4 0 12

echo "==> Verifying output"
OUT=/tmp/short-clean.mp4
[[ -s "$OUT" ]] || { echo "    $OUT missing or empty"; exit 1; }
ffprobe -v error -show_entries "stream=codec_name,codec_type,width,height" -show_entries "format=duration" -of json "$OUT" > /tmp/short-clean.json
python3 - <<'EOF'
import json, sys
d = json.load(open('/tmp/short-clean.json'))
vs = [s for s in d.get('streams', []) if s.get('codec_type') == 'video']
assert vs, 'no video stream'
assert int(vs[0].get('width', 0)) == 1080 and int(vs[0].get('height', 0)) == 1920, \
    f'expected 1080x1920, got {vs[0].get("width")}x{vs[0].get("height")}'
print('    output OK: 1080x1920 (9:16), duration=%ss' % d.get('format', {}).get('duration'))
EOF

echo "==> CLEAN-MACHINE PIPELINE OK"
