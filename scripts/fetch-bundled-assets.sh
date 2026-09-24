#!/usr/bin/env bash
# Download the assets that are intentionally excluded from git (size limits), so a CI
# build can bundle them into the installer:
#   - Whisper model weights (tiny, base)
#   - llama.cpp binaries for the host OS
#   - Qwen2.5-3B GGUF (local-LLM highlight)
#
# Local devs usually do NOT need this: the app downloads the LLM model on first use and
# the whisper venv is bootstrapped at runtime. CI needs it because the installer must ship
# with the weights already inside.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RES="$ROOT/apps/desktop/src-tauri/resources"
LLAMA_VERSION="${LLAMA_VERSION:-b11160}"
MODEL_NAME="${LLM_MODEL_NAME:-qwen2.5-3b-instruct-q4_k_m}"
WHISPER_MODELS="${WHISPER_MODELS:-tiny base}"

mkdir -p "$RES/models/llama" "$RES/llama"

echo "==> Whisper models: $WHISPER_MODELS"
for model in $WHISPER_MODELS; do
  dest="$RES/models/$model.pt"
  if [[ -f "$dest" ]]; then echo "    $model already present"; continue; fi
  echo "    downloading $model"
  # URLs + SHA-256 come from the openai-whisper registry (whisper/__init__.py _MODELS).
  case "$model" in
    tiny)  url="https://openaipublic.azureedge.net/main/whisper/models/65147644a518d12f04e32d6f3b26facc3f8dd46e5390956a9424a650c0ce22b9/tiny.pt"; sha="65147644a518d12f04e32d6f3b26facc3f8dd46e5390956a9424a650c0ce22b9" ;;
    base)  url="https://openaipublic.azureedge.net/main/whisper/models/ed3a0b6b1c0edf879ad9b11b1af5a0e6ab5db9205f891f668f8b0e6c6326e34e/base.pt"; sha="ed3a0b6b1c0edf879ad9b11b1af5a0e6ab5db9205f891f668f8b0e6c6326e34e" ;;
    small) url="https://openaipublic.azureedge.net/main/whisper/models/9ecf779972d90ba49c06d968637d720dd632c55bbf19d441fb42bf17a411e794/small.pt"; sha="9ecf779972d90ba49c06d968637d720dd632c55bbf19d441fb42bf17a411e794" ;;
    *) echo "    unknown whisper model '$model' (add its URL from whisper/__init__.py)" >&2; exit 1 ;;
  esac
  curl -L --fail --retry 3 -o "$dest" "$url"
  actual="$(sha256sum "$dest" | awk '{print $1}')"
  [[ "$actual" == "$sha" ]] || { echo "    checksum mismatch for $model: $actual" >&2; exit 1; }
done

echo "==> llama.cpp binaries ($LLAMA_VERSION, host OS)"
case "$(uname -s)" in
  Linux)  case "$(uname -m)" in
            x86_64) LLAMA_ASSET="llama-${LLAMA_VERSION}-bin-ubuntu-x64.tar.gz" ;;
            aarch64) LLAMA_ASSET="llama-${LLAMA_VERSION}-bin-ubuntu-arm64.tar.gz" ;;
            *) echo "unsupported linux arch $(uname -m)" >&2; exit 1 ;;
          esac ;;
  Darwin) case "$(uname -m)" in
            arm64) LLAMA_ASSET="llama-${LLAMA_VERSION}-bin-macos-arm64.tar.gz" ;;
            x86_64) LLAMA_ASSET="llama-${LLAMA_VERSION}-bin-macos-x64.tar.gz" ;;
            *) echo "unsupported mac arch $(uname -m)" >&2; exit 1 ;;
          esac ;;
  MINGW*|MSYS*|CYGWIN*) LLAMA_ASSET="llama-${LLAMA_VERSION}-bin-win-cpu-x64.zip" ;;
  *) echo "unsupported OS $(uname -s)" >&2; exit 1 ;;
esac

if [[ ! -x "$RES/llama/llama-server" && ! -f "$RES/llama/llama-server.exe" ]]; then
  tmp="$(mktemp -d)"
  echo "    fetching $LLAMA_ASSET"
  curl -L --fail --retry 3 -o "$tmp/llama.pkg" "https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_VERSION}/${LLAMA_ASSET}"
  case "$LLAMA_ASSET" in
    *.zip) unzip -q -o "$tmp/llama.pkg" -d "$tmp/x" ;;
    *)     mkdir -p "$tmp/x" && tar xzf "$tmp/llama.pkg" -C "$tmp/x" --strip-components=1 ;;
  esac
  find "$tmp/x" -maxdepth 2 -type f \( -name 'llama-server*' -o -name 'lib*.so*' -o -name 'lib*.dylib' -o -name '*.dll' \) -exec cp -f {} "$RES/llama/" \;
  chmod +x "$RES/llama/llama-server" 2>/dev/null || true
  rm -rf "$tmp"
else
  echo "    llama.cpp binaries already present"
fi

echo "==> Local LLM model: $MODEL_NAME (~2.1 GB)"
dest="$RES/models/llama/${MODEL_NAME}.gguf"
if [[ -f "$dest" ]]; then
  echo "    already present"
else
  curl -L --fail --retry 3 -o "$dest" "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/${MODEL_NAME}.gguf"
fi
EXPECTED="626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d"
if [[ "$MODEL_NAME" == "qwen2.5-3b-instruct-q4_k_m" ]]; then
  ACTUAL="$(sha256sum "$dest" | awk '{print $1}')"
  [[ "$ACTUAL" == "$EXPECTED" ]] || { echo "checksum mismatch: $ACTUAL" >&2; exit 1; }
fi

echo "==> Assets ready:"
du -sh "$RES/models" "$RES/llama" 2>/dev/null || true
