# Local LLM (optional offline highlights)

Local LLM is an optional highlight strategy alongside the default `heuristic`
and `semantic-gemini` BYOK modes. It runs Qwen2.5-3B locally with llama.cpp;
no model or llama.cpp runtime is included in the default installer or portable
package. Heuristic mode remains usable without either download.

## Optional setup

In the desktop app, open **Settings → First-run setup → Install Local LLM** and
confirm the download. Setup fetches the llama.cpp runtime for the current
supported platform and the Qwen2.5-3B Q4_K_M GGUF model (about 2.2 GB total),
then verifies SHA-256 before installing. Internet access and sufficient disk
space are required. Files are stored under the app state directory in
`llama/runtime` and `models/llama`.

Only after setup is complete can **Local LLM (offline)** be selected in Settings.
If runtime or model files are missing, the app reports an actionable setup error;
selecting the strategy never starts a large download. The model identifier
`qwen2.5-3b-instruct` resolves to the fixed Q4_K_M file; arbitrary model names
are not accepted.

## How it works

1. Tauri's `install_local_llm` command downloads the platform's pinned llama.cpp
   release archive and verifies its SHA-256 before extracting the server and
   adjacent shared libraries into app-owned state.
2. The same command downloads the fixed GGUF model to a `.part` file, verifies
   the pinned SHA-256, then renames it into place. Existing model files are
   hashed before being accepted.
3. `start_local_llm` starts `llama-server` bound to loopback and serves the model
   to the OpenAI-compatible `LlamaCppHighlightProvider`.
4. Heuristic and Gemini strategies do not require the local runtime or model.

The manual model-only helper remains available for developers:

```bash
LOCAL_FIRST_STATE_DIR=~/.cache/local-first-ai-video-studio scripts/download-llm-model.sh
```

This helper does not install llama.cpp; the Settings installer is the supported
way to set up both components together.

## Platform notes

Pinned archives are configured for Linux x64/arm64, macOS x64/arm64, and Windows
x64 CPU. Native installation and runtime launch still need verification on each
platform's runner/device. GPU acceleration and additional model choices are not
part of this opt-in setup.
