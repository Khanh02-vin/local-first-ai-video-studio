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
LOCAL_FIRST_STATE_DIR="$STATE" WHISPER_COMMAND="$WHISPER_COMMAND" WHISPER_MODEL="$WHISPER_MODEL" \
  node --experimental-strip-types scripts/run-local-demo.ts /tmp/talk.mp4 /tmp/short-clean.mp4 0 12

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
