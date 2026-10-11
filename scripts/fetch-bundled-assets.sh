#!/usr/bin/env bash
# Fill apps/desktop/src-tauri/resources/* with the binaries/weights that are
# intentionally excluded from git (size limits), so tauri-build and packaging work
# on a fresh checkout:
#   - resources/node/      (Node dist — dir must exist)
#   - resources/ffmpeg     resources/ffprobe  (static builds or host copies)
#   - resources/models/*.pt                (Whisper weights)
#
# Default builds never fetch llama.cpp or GGUF. The app installs both only after
# the user explicitly opts in from Settings.
# LIGHTWEIGHT=1 only creates the paths tauri-build validates (empty placeholders).
# Default mode downloads core binaries and Whisper weights for the host platform.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RES="$ROOT/apps/desktop/src-tauri/resources"
NODE_VERSION="${NODE_VERSION:-v24.21.0}"
WHISPER_MODELS="${WHISPER_MODELS:-tiny base}"
ARCH="$(uname -m)"
OS="$(uname -s)"

mkdir -p "$RES/models"

# --- extract_zip: try unzip, then python3, then python, then tar --------------
extract_zip() {
  local pkg="$1" dest="$2"
  if command -v unzip >/dev/null 2>&1; then
    unzip -q -o "$pkg" -d "$dest" || return 1
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c "
import sys, zipfile
zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])
" "$pkg" "$dest" || return 1
  elif command -v python >/dev/null 2>&1; then
    python -c "
import sys, zipfile
zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])
" "$pkg" "$dest" || return 1
  else
    # Last resort: GNU tar 1.36+ can read some zips
    tar -xf "$pkg" -C "$dest" || { echo "Error: cannot extract zip ($pkg). Install unzip or python3." >&2; return 1; }
  fi
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
if [[ -x "$RES/node/bin/node" || -f "$RES/node/bin/node.exe" ]]; then
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
    # Node zips nest everything under node-vX.Y.Z-win-x64/ — strip that level
    # (the tarball branch does the same via --strip-components=1) or
    # resources/node/node.exe never exists and the bin/ normalization below
    # silently does nothing (CI verify caught this on 2026-10-01).
    inner="$(find "$tmp/x" -mindepth 1 -maxdepth 1 -type d | head -1)"
    if [[ -n "$inner" ]]; then cp -r "$inner"/. "$tmp/x/"; rm -rf "$inner"; fi
  else
    mkdir -p "$tmp/x"
    tar xf "$tmp/node.pkg" -C "$tmp/x" --strip-components=1
  fi
  mkdir -p "$RES/node"
  cp -r "$tmp/x"/. "$RES/node/"
  # Windows zips put node.exe at the archive root; normalize to the Unix bin/
  # layout so bundled_bin("node/bin/node") resolves on every platform.
  if [[ -f "$RES/node/node.exe" && ! -f "$RES/node/bin/node.exe" ]]; then
    mkdir -p "$RES/node/bin"
    mv "$RES/node/node.exe" "$RES/node/bin/node.exe"
  fi
  rm -rf "$tmp"
fi

# --- ffmpeg / ffprobe ---------------------------------------------------------
# Windows CreateProcess resolves `.exe`; the extensionless Unix name is not an
# executable there, so ship platform-native names (Rust bundled_bin probes both).
ff_target() {
  if [[ "$OS" == MINGW* || "$OS" == MSYS* || "$OS" == CYGWIN* ]]; then echo "$RES/$1.exe"; else echo "$RES/$1"; fi
}
if [[ ( -e "$RES/ffmpeg" || -e "$RES/ffmpeg.exe" ) && ( -e "$RES/ffprobe" || -e "$RES/ffprobe.exe" ) ]]; then
  echo "==> ffmpeg/ffprobe already present"
elif command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null; then
  echo "==> ffmpeg/ffprobe: copying host binaries ($(ffmpeg -version | head -1))"
  cp -f "$(command -v ffmpeg)" "$(ff_target ffmpeg)"
  cp -f "$(command -v ffprobe)" "$(ff_target ffprobe)"
else
  case "$OS-$ARCH" in
    Linux-x86_64)
      # johnvansickle intermittently serves an HTML error page instead of the
      # tarball (CI hit it 2026-09-26); the BtbN GPL release carries both
      # binaries in one archive, so it is the primary source.
      urls=("https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
            "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz") ;;
    Linux-aarch64)
      urls=("https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz") ;;
    Darwin-*)
      # evermeet.cx ships ffmpeg and ffprobe as separate archives; x64 builds
      # run under Rosetta on arm64 runners. static-builds.net serves a single
      # combined archive as the fallback mirror.
      urls=("https://evermeet.cx/ffmpeg/ffmpeg-7.0.2.zip" "https://evermeet.cx/ffmpeg/ffprobe-7.0.2.zip"
            "https://www.static-builds.net/files/ffmpeg-7.1-x86_64-macos-release.zip") ;;
    MINGW*|MSYS*|CYGWIN*)
      urls=("https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip") ;;
    *) echo "unsupported platform for ffmpeg: $OS-$ARCH" >&2; exit 1 ;;
  esac
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/x"
  # Download every archive in turn; stop as soon as BOTH binaries are present.
  # (macOS evermeet ships ffmpeg and ffprobe as two separate zips — breaking
  # after the first one would leave ffprobe missing.)
  for url in "${urls[@]}"; do
    found_ff="$(find "$tmp/x" -type f \( -name 'ffmpeg' -o -name 'ffmpeg.exe' \) | head -1)"
    found_fp="$(find "$tmp/x" -type f \( -name 'ffprobe' -o -name 'ffprobe.exe' \) | head -1)"
    [[ -n "$found_ff" && -n "$found_fp" ]] && break
    echo "==> ffmpeg: downloading $url"
    curl -L --fail --retry 3 -o "$tmp/ff.pkg" "$url" || { echo "    download failed, trying next mirror" >&2; continue; }
    # Integrity guard: a truncated or HTML error page fails extraction here
    # and we fall through to the next mirror instead of aborting the fetch.
    if [[ "$url" == *.zip ]]; then
      extract_zip "$tmp/ff.pkg" "$tmp/x" 2>/dev/null || echo "    archive corrupt or blocked, trying next mirror" >&2
    else
      tar xJf "$tmp/ff.pkg" -C "$tmp/x" 2>/dev/null || tar xf "$tmp/ff.pkg" -C "$tmp/x" 2>/dev/null || echo "    archive corrupt or blocked, trying next mirror" >&2
    fi
  done
  found_ff="$(find "$tmp/x" -type f \( -name 'ffmpeg' -o -name 'ffmpeg.exe' \) | head -1)"
  found_fp="$(find "$tmp/x" -type f \( -name 'ffprobe' -o -name 'ffprobe.exe' \) | head -1)"
  [[ -n "$found_ff" && -n "$found_fp" ]] || { echo "ffmpeg/ffprobe not found inside archive" >&2; exit 1; }
  cp -f "$found_ff" "$(ff_target ffmpeg)"
  cp -f "$found_fp" "$(ff_target ffprobe)"
  chmod +x "$RES/ffmpeg" "$RES/ffprobe" "$RES/ffmpeg.exe" "$RES/ffprobe.exe" 2>/dev/null || true
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

# --- no local-LLM assets are staged in default builds -------------------------

echo "==> Assets ready:"
du -sh "$RES/node" "$RES/ffmpeg" "$RES/models" 2>/dev/null || true
