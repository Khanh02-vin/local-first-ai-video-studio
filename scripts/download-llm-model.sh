#!/usr/bin/env bash
# Download the fixed, checksum-pinned Local LLM model on demand.
set -euo pipefail

if [[ "${1:-}" != "" && "$1" != "qwen2.5-3b-instruct-q4_k_m" ]]; then
  echo "Only qwen2.5-3b-instruct-q4_k_m is supported." >&2
  exit 2
fi

STATE_DIR="${LOCAL_FIRST_STATE_DIR:-$HOME/.cache/local-first-ai-video-studio}"
MODEL_DIR="$STATE_DIR/models/llama"
MODEL_NAME="qwen2.5-3b-instruct-q4_k_m"
TARGET="$MODEL_DIR/$MODEL_NAME.gguf"
PART="$TARGET.part"
URL="https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/$MODEL_NAME.gguf"
CHECKSUM="626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d"

mkdir -p "$MODEL_DIR"
if [[ -f "$TARGET" ]]; then
  ACTUAL=$(sha256sum "$TARGET" | cut -d ' ' -f1)
  if [[ "$ACTUAL" == "$CHECKSUM" ]]; then
    echo "Model already verified: $TARGET"
    exit 0
  fi
  rm -f "$TARGET"
fi
rm -f "$PART"
trap 'rm -f "$PART"' EXIT
curl -L --fail --retry 3 -o "$PART" "$URL"
ACTUAL=$(sha256sum "$PART" | cut -d ' ' -f1)
if [[ "$ACTUAL" != "$CHECKSUM" ]]; then
  echo "Checksum mismatch: expected $CHECKSUM, got $ACTUAL" >&2
  exit 1
fi
mv "$PART" "$TARGET"
trap - EXIT
echo "Model ready: $TARGET"
