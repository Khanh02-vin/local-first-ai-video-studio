#!/usr/bin/env bash
# One-time setup for a clean machine running Local-first AI Video Studio.
# Installs: python venv + openai-whisper (MIT) + tiny model, writes runtime.env.
# The app bundles Node and FFmpeg itself, so this only covers the Whisper side.
set -euo pipefail

# Durable data (databases, runtime.env) lives in the OS data directory — never in
# ~/.cache, which cleanup tools empty. The app's Rust side resolves the same roots
# (paths::data_dir / paths::cache_dir); LOCAL_FIRST_STATE_DIR overrides both.
case "$(uname -s)" in
  Darwin) DEFAULT_STATE="$HOME/Library/Application Support/local-first-ai-video-studio" ;;
  *)      DEFAULT_STATE="${XDG_DATA_HOME:-$HOME/.local/share}/local-first-ai-video-studio" ;;
esac
STATE="${LOCAL_FIRST_STATE_DIR:-$DEFAULT_STATE}"
VENV="${LOCAL_FIRST_VENV:-${LOCAL_FIRST_CACHE_DIR:-$STATE}/.venv}"
MODEL="${WHISPER_MODEL:-tiny}"
mkdir -p "$STATE"

echo "==> Creating venv at $VENV"
python3 -m venv "$VENV"

echo "==> Installing openai-whisper (CPU)"
"$VENV/bin/pip" install --upgrade pip >/dev/null
"$VENV/bin/pip" install openai-whisper

# The local crop-path analysis (analyze_crop_path / sample_speaker_faces) runs the
# Haar-cascade face detector in scripts/detect-face-boxes.py and scripts/detect-faces.py,
# which need OpenCV. Without it those commands fail with FACE_RUNTIME_MISSING, so the
# Speaker framing panel in the editor cannot produce a crop path.
# opencv-python-headless is pinned below 5: OpenCV 5 moved the cascade API and
# CascadeClassifier no longer exists there.
echo "==> Installing opencv-python-headless (face detection)"
"$VENV/bin/pip" install "opencv-python-headless<5"

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