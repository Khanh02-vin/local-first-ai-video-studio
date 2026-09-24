#!/usr/bin/env bash
# Download the local-LLM GGUF model on demand (first use). The app shell bundles only a
# placeholder; this script fetches the real weights and verifies them by SHA-256.
#
#   WHISPER_STATE_DIR defaults to ~/.cache/local-first-ai-video-studio
set -euo pipefail

STATE_DIR="${LOCAL_FIRST_STATE_DIR:-$HOME/.cache/local-first-ai-video-studio}"
MODELS_DIR="$STATE_DIR/models/llama"
MODEL_NAME="${1:-qwen2.5-3b-instruct-q4_k_m}"
URL="https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/${MODEL_NAME}.gguf"

mkdir -p "$MODELS_DIR"
TARGET="$MODELS_DIR/${MODEL_NAME}.gguf"

# Known checksums (SHA-256) for the bundled quantizations.
case "$MODEL_NAME" in
  qwen2.5-3b-instruct-q4_k_m) CHECKSUM="626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d";;
  qwen2.5-3b-instruct-q5_k_m) CHECKSUM="";;
  *) echo "Unknown model '$MODEL_NAME' (expected qwen2.5-3b-instruct-q4_k_m or q5_k_m)." >&2; exit 2;;
esac

if [[ -f "$TARGET" ]]; then
  echo "Model already present: $TARGET"
  exit 0
fi

echo "Downloading $MODEL_NAME ($URL)"
curl -L --fail --retry 3 -o "$TARGET.part" "$URL"
mv "$TARGET.part" "$TARGET"

if [[ -n "$CHECKSUM" ]]; then
  ACTUAL=$(sha256sum "$TARGET" | awk '{print $1}')
  if [[ "$ACTUAL" != "$CHECKSUM" ]]; then
    echo "Checksum mismatch: expected $CHECKSUM, got $ACTUAL" >&2
    rm -f "$TARGET"
    exit 1
  fi
  echo "Checksum verified: $ACTUAL"
fi

echo "Model ready: $TARGET"
