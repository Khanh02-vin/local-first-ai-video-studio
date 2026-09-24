#!/usr/bin/env bash
# One-time setup for a clean machine running Local-first AI Video Studio.
# Installs: python venv + openai-whisper (MIT) + tiny model, writes runtime.env.
# The app bundles Node and FFmpeg itself, so this only covers the Whisper side.
set -euo pipefail

STATE="${LOCAL_FIRST_STATE_DIR:-$HOME/.cache/local-first-ai-video-studio}"
VENV="${LOCAL_FIRST_VENV:-$HOME/.cache/local-first-ai-video-studio/.venv}"
MODEL="${WHISPER_MODEL:-tiny}"
mkdir -p "$STATE"

echo "==> Creating venv at $VENV"
python3 -m venv "$VENV"

echo "==> Installing openai-whisper (CPU)"
"$VENV/bin/pip" install --upgrade pip >/dev/null
"$VENV/bin/pip" install openai-whisper

echo "==> Downloading model '$MODEL'"
"$VENV/bin/whisper" --model "$MODEL" --device cpu --fp16 False /dev/null --output_format json --language en 2>/dev/null || true

MODEL_PATH="$HOME/.cache/whisper/$MODEL.pt"
if [ ! -f "$MODEL_PATH" ]; then
  echo "ERROR: model not found at $MODEL_PATH" >&2
  echo "Try: WHISPER_MODEL=tiny $0" >&2
  exit 1
fi

echo "==> Writing runtime env"
cat > "$STATE/runtime.env" <<EOF
WHISPER_COMMAND=$VENV/bin/whisper
WHISPER_MODEL=$MODEL
WHISPER_FP16=False
WHISPER_DEVICE=cpu
EOF
cat >> "$STATE/runtime.env" <<EOF
LICENSE_PUBLIC_KEY=${LICENSE_PUBLIC_KEY:-}
EOF

echo "==> Done. Launch the app; Whisper will use $VENV/bin/whisper"
echo "    Runtime env: $STATE/runtime.env (edit WHISPER_MODEL/DEVICE for pro GPU users)"