#!/usr/bin/env bash
# Simulates Tauri's run_whisper_bootstrap on a clean machine:
#   1. No .venv, no runtime.env, no whisper on PATH
#   2. Creates app-owned venv, installs openai-whisper
#   3. Writes runtime.env so the app finds the whisper CLI
#
# This is the exact code path a first-time user hits when they install the .deb
# and press "Setup Whisper" in the Settings screen.
set -euo pipefail

STATE="${LOCAL_FIRST_STATE_DIR:-$HOME/.cache/local-first-ai-video-studio}"
VENV="$STATE/.venv"
WHISPER_BIN="$VENV/bin/whisper"
MODEL="${WHISPER_MODEL:-tiny}"

echo "==> Pristine state check"
[[ ! -d "$STATE" ]] && echo "    state dir absent (fresh)"
[[ ! -d "$VENV" ]] && echo "    venv absent (fresh)"
[[ ! -e "$WHISPER_BIN" ]] && echo "    whisper absent (fresh)"
command -v whisper >/dev/null 2>&1 && { echo "    WARNING: whisper on PATH — not a clean machine"; exit 1; }

echo "==> Creating venv at $VENV"
python3 -m venv "$VENV"

echo "==> Upgrading pip"
"$VENV/bin/pip" install --upgrade pip >/dev/null

echo "==> Installing openai-whisper (downloads PyTorch — may take a few minutes)"
"$VENV/bin/pip" install openai-whisper

echo "==> Verifying whisper CLI"
[[ -x "$WHISPER_BIN" ]] || { echo "    whisper binary missing after install"; exit 1; }
"$WHISPER_BIN" --help >/dev/null

echo "==> Writing runtime.env"
mkdir -p "$STATE"
cat > "$STATE/runtime.env" <<EOF
WHISPER_COMMAND=$WHISPER_BIN
WHISPER_MODEL=$MODEL
WHISPER_DEVICE=cpu
WHISPER_FP16=False
HIGHLIGHT_STRATEGY=heuristic
GEMINI_API_KEY=
LOCAL_LLM_BASE_URL=http://127.0.0.1:8080
LOCAL_LLM_MODEL=qwen2.5-3b-instruct-q4_k_m
EOF

echo "==> Bootstrap complete. runtime.env:"
cat "$STATE/runtime.env"
echo "==> DONE"
