# Clean-machine (first-run bootstrap) test

Mirrors a fresh Ubuntu 24.04 client machine: no `whisper`, no app venv, no
`~/.cache/local-first-ai-video-studio` state dir. The image installs ffmpeg
from apt **only so the test can make a source video** — the real `.deb`
ships its own bundled `resources/ffmpeg`/`ffprobe`, and the pipeline under
test runs exactly like the Tauri app does (bundled node + bundled/PATH
ffmpeg, see `node_bin()`/`ffmpeg_bin()` in `apps/desktop/src-tauri/src/main.rs`).

## Run

```bash
docker build -f tests/clean-machine/Dockerfile -t lfai-clean-machine .

# 1) first-run bootstrap: create the app venv, install openai-whisper
#    (CPU wheels), write runtime.env, copy the bundled whisper weights to
#    ~/.cache/whisper — the exact path a first-time user hits when they
#    install the .deb and press "Setup Whisper"
docker run lfai-clean-machine bash tests/clean-machine/run-whisper-bootstrap.sh

# 2) full pipeline: probe → extract audio → whisper transcribe → highlight →
#    render 1080x1920 MP4, verifying the output dimensions
docker run lfai-clean-machine bash tests/clean-machine/run-analyze-pipeline.sh
```

Steps 1 and 2 must run in **the same container** (step 2 reads the
`runtime.env` that step 1 writes), or re-run step 1 in the same `docker run`
session:

```bash
docker run lfai-clean-machine bash -c \
  'bash tests/clean-machine/run-whisper-bootstrap.sh && bash tests/clean-machine/run-analyze-pipeline.sh'
```

## What each script checks

- `run-whisper-bootstrap.sh` — pristine-state guards (whisper absent from
  PATH, no state dir), then venv + `pip install openai-whisper` with the
  PyTorch **CPU** extra index (~500 MB instead of ~3 GB of CUDA wheels),
  writes `runtime.env`, copies the bundled `$MODEL.pt` into
  `~/.cache/whisper` (mirrors `ensure_bundled_whisper_model` in `main.rs`).
  **Idempotent**: when the venv's whisper already exists it skips
  venv/pip entirely (mirrors the fast path of `run_whisper_bootstrap`).
- `run-analyze-pipeline.sh` — loads `runtime.env`, builds a 12 s synthetic
  source with a steady tone (whisper gets *something* to transcribe), runs
  `scripts/run-local-demo.ts` with the **bundled node**
  (`resources/node/bin/node`) and `FFMPEG_PATH`/`FFPROBE_PATH` pointing at
  the bundled `resources/ffmpeg`/`ffprobe` (falling back to PATH for dev
  machines that never fetched resources), then asserts the output is a
  valid 1080x1920 MP4.

Expected whisper result on the synthetic tone: **0 words** — the tone has
no speech, and the tiny model correctly reports no words; the pipeline must
still produce a valid render.
