#!/usr/bin/env bash
# Static cross-platform packaging gates for the bundled Whisper.cpp assets.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONF="$ROOT/apps/desktop/src-tauri/tauri.conf.json"
FETCH="$ROOT/scripts/fetch-bundled-assets.sh"

python3 - "$CONF" "$FETCH" <<'PY'
import json, pathlib, sys
conf = json.loads(pathlib.Path(sys.argv[1]).read_text())
fetch = pathlib.Path(sys.argv[2]).read_text()
resources = conf["bundle"]["resources"]
assert any("whisper-cpp" in key for key in resources), "missing whisper-cpp resource mapping"
assert any("*.bin" in key for key in resources), "missing GGML model resource mapping"
for token in ("Linux-x86_64", "Linux-aarch64", "Darwin-arm64", "Darwin-x86_64", "MINGW", "WHISPER_CLI_BIN"):
    assert token in fetch, f"missing fetch gate: {token}"
assert "WHISPER_CLI_BIN=\"whisper-cli.exe\"" in fetch, "Windows executable name missing"
assert "whisper-cli" in fetch and "ggml-tiny.bin" in fetch, "Whisper.cpp assets missing"
print("static packaging gates: ok (Linux/macOS/Windows branches and resource mappings)")
PY
