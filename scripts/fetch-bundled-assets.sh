#!/usr/bin/env bash
# Fill apps/desktop/src-tauri/resources/* with the binaries/weights that are
# intentionally excluded from git (size limits), so tauri-build and packaging work
# on a fresh checkout:
#   - resources/node/      (Node dist — dir must exist)
#   - resources/ffmpeg     resources/ffprobe  (static builds or host copies)
#   - resources/models/*.pt                (Whisper weights)
#   - resources/models/llama/*.gggf        (Qwen2.5-3B)
#   - resources/llama/      (llama.cpp — committed; re-fetched if missing)
#
# Modes:
#   LIGHTWEIGHT=1  only create the PATHS that tauri-build validates (empty
#                  placeholders). Enough for `cargo test`, which never executes
#                  the binaries — keeps CI test jobs fast.
#   (default)      download real binaries for the host platform; prefers copying
#                  an ffmpeg already installed on the host.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RES="$ROOT/apps/desktop/src-tauri/resources"
LLAMA_VERSION="${LLAMA_VERSION:-b11160}"
NODE_VERSION="${NODE_VERSION:-v24.21.0}"
MODEL_NAME="${LLM_MODEL_NAME:-qwen2.5-3b-instruct-q4_k_m}"
WHISPER_MODELS="${WHISPER_MODELS:-tiny base}"
ARCH="$(uname -m)"
OS="$(uname -s)"

mkdir -p "$RES/models/llama" "$RES/llama"

# --- extract_zip: try unzip, then python3, then python, then tar --------------
extract_zip() {
  local pkg="$1" dest="$2"
  if command -v unzip >/dev/null 2>&1; then
    unzip -q -o "$pkg" -d "$dest"; return 0
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 -c "
import sys, zipfile, pathlib
z = zipfile.ZipFile(sys.argv[1])
z.extractall(sys.argv[2])
" "$pkg" "$dest"; return 0
  elif command -v python >/dev/null 2>&1; then
    python -c "
import sys, zipfile
zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])
" "$pkg" "$dest"; return 0
  fi
  # Last resort: GNU tar 1.36+ can read some zips
  tar -xf "$pkg" -C "$dest" || {
    echo "Error: cannot extract zip ($pkg). Install unzip or python3." >&2
    exit 1
  }
}

# --- lightweight: only satisfy tauri-build's existence checks ------------------
if [[ "${LIGHTWEIGHT:-0}" == "1" ]]; then
  mkdir -p "$RES/node"
  [[ -e "$RES/ffmpeg" ]]    || : > "$RES/ffmpeg"
  [[ -e "$RES/ffprobe" ]]   || : > "$RES/ffprobe"
  echo "==> LIGHTWEIGHT: placeholder paths created (node dir, ffmpeg, ffprobe)"
  exit 0
fi

# --- node ---------------------------------------------------------------------
if [[ -x "$RES/node/bin/node" ]]; then
  echo "==> node already present"
else
  case "$OS-$ARCH" in
    Linux-x86_64)  NODE_OS=linux NODE_ARCH=x64  NODE_PKG=tar.xz ;;
    Linux-aarch64) NODE_OS=linux NODE_ARCH=arm64 NODE_PKG=tar.xz ;;
    Darwin-arm64)  NODE_OS=darwin NODE_ARCH=arm64 NODE_PKG=tar.gz ;;
    Darwin-x86_64) NODE_OS=darwin NODE_ARCH=x64 NODE_PKG=tar.gz ;;
    MINGW*|MSYS*|CYGWIN*) NODE_OS=win NODE_ARCH=x64 NODE_PKG=zip ;;
    *) echo "unsupported platform for node: $OS-$ARCH" >&2; exit 1 ;;
  esac
  url="https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-${NODE_OS}-${NODE_ARCH}.${NODE_PKG}"
  echo "==> node: downloading $url"
  tmp="$(mktemp -d)"
  curl -L --fail --retry 3 -o "$tmp/node.pkg" "$url"
  if [[ "$NODE_PKG" == "zip" ]]; then
    mkdir -p "$tmp/x"
    extract_zip "$tmp/node.pkg" "$tmp/x"
  else
    mkdir -p "$tmp/x"
    tar xf "$tmp/node.pkg" -C "$tmp/x" --strip-components=1
  fi
  mkdir -p "$RES/node"
  cp -r "$tmp/x"/. "$RES/node/"
  rm -rf "$tmp"
fi

# --- ffmpeg / ffprobe ---------------------------------------------------------
if [[ -e "$RES/ffmpeg" && -e "$RES/ffprobe" ]]; then
  echo "==> ffmpeg/ffprobe already present"
elif command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null; then
  echo "==> ffmpeg/ffprobe: copying host binaries ($(ffmpeg -version | head -1))"
  cp -f "$(command -v ffmpeg)" "$RES/ffmpeg"
  cp -f "$(command -v ffprobe)" "$RES/ffprobe"
else
  case "$OS-$ARCH" in
    Linux-x86_64)
      urls=("https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz") ;;
    Linux-aarch64)
      urls=("https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz") ;;
    Darwin-*)
      urls=("https://evermeet.cx/ffmpeg/ffmpeg-7.0.2.zip" "https://evermeet.cx/ffmpeg/ffprobe-7.0.2.zip") ;;
    MINGW*|MSYS*|CYGWIN*)
      urls=("https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip") ;;
    *) echo "unsupported platform for ffmpeg: $OS-$ARCH" >&2; exit 1 ;;
  esac
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/x"
  for url in "${urls[@]}"; do
    echo "==> ffmpeg: downloading $url"
    curl -L --fail --retry 3 -o "$tmp/ff.pkg" "$url"
    if [[ "$url" == *.zip ]]; then
      extract_zip "$tmp/ff.pkg" "$tmp/x"
    else
      tar xf "$tmp/ff.pkg" -C "$tmp/x"
    fi
  done
  found_ff="$(find "$tmp/x" -type f -name 'ffmpeg' -o -type f -name 'ffmpeg.exe' | head -1)"
  found_fp="$(find "$tmp/x" -type f -name 'ffprobe' -o -type f -name 'ffprobe.exe' | head -1)"
  [[ -n "$found_ff" && -n "$found_fp" ]] || { echo "ffmpeg/ffprobe not found inside archive" >&2; exit 1; }
  cp -f "$found_ff" "$RES/ffmpeg"
  cp -f "$found_fp" "$RES/ffprobe"
  chmod +x "$RES/ffmpeg" "$RES/ffprobe" 2>/dev/null || true
  rm -rf "$tmp"
fi

# --- whisper models (HuggingFace CDN — Azure blob blocked from CI IPs) ----------
# NOTE: GitHub Actions runners are IP-blocked from openaipublic.azureedge.net.
# HuggingFace CDN works from all CI environments.
# The .bin file is downloaded then renamed to .pt (expected by whisper.cpp).
echo "==> Whisper models: $WHISPER_MODELS"
for model in $WHISPER_MODELS; do
  final="$RES/models/$model.pt"
  if [[ -f "$final" ]]; then echo "    $model already present"; continue; fi
  case "$model" in
    tiny)  url="https://huggingface.co/openai/whisper-tiny/resolve/main/pytorch_model.bin" ;;
    base)  url="https://huggingface.co/openai/whisper-base/resolve/main/pytorch_model.bin" ;;
    small) url="https://huggingface.co/openai/whisper-small/resolve/main/pytorch_model.bin" ;;
    *) echo "    unknown whisper model '$model'" >&2; exit 1 ;;
  esac
  echo "  Fetching: $url -> $final"
  curl -L --fail --retry 3 -o "$final" "$url"
done

# --- llama.cpp binaries (committed; fetched only if missing) -------------------
if [[ -x "$RES/llama/llama-server" || -f "$RES/llama/llama-server.exe" ]]; then
  echo "==> llama.cpp binaries already present"
else
  case "$OS" in
    Linux)  case "$ARCH" in
              x86_64) asset="llama-${LLAMA_VERSION}-bin-ubuntu-x64.tar.gz" ;;
              aarch64) asset="llama-${LLAMA_VERSION}-bin-ubuntu-arm64.tar.gz" ;;
              *) echo "unsupported linux arch $ARCH" >&2; exit 1 ;;
            esac ;;
    Darwin) case "$ARCH" in
              arm64) asset="llama-${LLAMA_VERSION}-bin-macos-arm64.tar.gz" ;;
              *) asset="llama-${LLAMA_VERSION}-bin-macos-x64.tar.gz" ;;
            esac ;;
    MINGW*|MSYS*|CYGWIN*) asset="llama-${LLAMA_VERSION}-bin-win-cpu-x64.zip" ;;
    *) echo "unsupported OS $OS" >&2; exit 1 ;;
  esac
  echo "==> llama.cpp: fetching $asset"
  tmp="$(mktemp -d)"
  curl -L --fail --retry 3 -o "$tmp/llama.pkg" "https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_VERSION}/${asset}"
  if [[ "$asset" == *.zip ]]; then
    mkdir -p "$tmp/x"
    extract_zip "$tmp/llama.pkg" "$tmp/x"
  else
    mkdir -p "$tmp/x"
    tar xzf "$tmp/llama.pkg" -C "$tmp/x" --strip-components=1
  fi
  find "$tmp/x" -maxdepth 3 -type f \( -name 'llama-server*' -o -name 'lib*.so*' -o -name 'lib*.dylib' -o -name '*.dll' \) -exec cp -f {} "$RES/llama/" \;
  chmod +x "$RES/llama/llama-server" 2>/dev/null || true
  rm -rf "$tmp"
fi

# --- local-LLM GGUF (2.1 GB) --------------------------------------------------
dest="$RES/models/llama/${MODEL_NAME}.gguf"
if [[ -f "$dest" ]]; then
  echo "==> Local LLM model already present"
else
  echo "==> Local LLM model: $MODEL_NAME (~2.1 GB)"
  curl -L --fail --retry 3 -o "$dest" "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/${MODEL_NAME}.gguf"
fi
if [[ "$MODEL_NAME" == "qwen2.5-3b-instruct-q4_k_m" ]]; then
  EXPECTED="626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d"
  ACTUAL="$(sha256sum "$dest" | awk '{print $1}')"
  [[ "$ACTUAL" == "$EXPECTED" ]] || { echo "checksum mismatch: $ACTUAL" >&2; exit 1; }
fi

echo "==> Assets ready:"
du -sh "$RES/node" "$RES/ffmpeg" "$RES/llama" "$RES/models" 2>/dev/null || truetest
