# Linux Tauri desktop E2E

This test launches a dedicated E2E build of the real Linux Tauri app and drives it through WebDriver. It clicks **Browse files**, analyzes a local video, keeps one suggested clip, clicks **Render selected clip**, and verifies the real FFmpeg output. Only the native open/save dialogs are bypassed: the E2E build receives explicit test input/output paths, while the UI buttons, `probe_video`, project persistence, analysis, and `render_video` remain real.

## Prerequisites

- Linux desktop dependencies required by Tauri/WebKitGTK (`webkit2gtk`/GTK development/runtime packages).
- Node.js 22+, Rust/Cargo, and dependencies already installed by `npm ci`.
- `tauri-driver` and WebKitWebDriver compatible with the installed Tauri 2 / WebKitGTK version.
- FFmpeg/ffprobe available to the app.
- A short local video with clear spoken dialogue, suitable for yielding at least one highlight. Set `DESKTOP_E2E_VIDEO=/absolute/path/to/video.mp4`.
- A ready local Whisper runtime/model. For a clean machine, `scripts/setup-machine.sh` sets it up (downloads Python packages and model assets). Set `WHISPER_COMMAND` to the local Whisper executable and ensure the isolated runtime can access the selected model. The test fails if analysis does not produce a candidate.
- A graphical session available to launch Tauri. No `xdotool`, X11 window manager, or native-dialog automation is required.

## Build and run

From the repository root, use a dedicated Cargo target directory and frontend output directory so this build does not overwrite normal app binaries or `apps/web/build`:

```sh
export DESKTOP_E2E_VIDEO=/absolute/path/to/short-dialogue.mp4
export DESKTOP_E2E_ARTIFACTS="$PWD/artifacts/desktop-e2e"
export DESKTOP_E2E_BUILD_VIDEO="$DESKTOP_E2E_VIDEO"
export DESKTOP_E2E_BUILD_OUTPUT="$DESKTOP_E2E_ARTIFACTS/e2e-export.mp4"
export WHISPER_COMMAND="/absolute/path/to/whisper"
export WHISPER_MODEL=tiny
export WHISPER_DEVICE=cpu
export WHISPER_FP16=False
export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/local-first-desktop-e2e-target"
export VITE_DESKTOP_E2E=1
export VITE_DESKTOP_E2E_VIDEO="$DESKTOP_E2E_BUILD_VIDEO"
export VITE_DESKTOP_E2E_OUTPUT="$DESKTOP_E2E_BUILD_OUTPUT"
export VITE_BUILD_OUT_DIR="$CARGO_TARGET_DIR/../local-first-desktop-e2e-web"

(cd apps/web && npm run build -- --mode desktop-e2e)
node -e 'const fs=require("node:fs"); const out=process.env.CARGO_TARGET_DIR+"/../local-first-desktop-e2e-tauri.json"; fs.writeFileSync(out, JSON.stringify({build:{frontendDist:process.env.VITE_BUILD_OUT_DIR,beforeBuildCommand:"true"}})); console.log(out)'
cd apps/desktop && npm exec -- tauri build --no-bundle --config "$CARGO_TARGET_DIR/../local-first-desktop-e2e-tauri.json"

export DESKTOP_E2E_APP="$CARGO_TARGET_DIR/release/local-first-ai-video-studio"
npm run test:desktop-e2e
```

Set `DESKTOP_E2E_BUILD_OUTPUT` and the `VITE_DESKTOP_E2E_OUTPUT` build variable to the same absolute path. All build output locations are derived from `CARGO_TARGET_DIR` (or the chosen `/tmp` target), so normal frontend, sidecar, and release build directories are untouched. The sidecar resource uses the existing `apps/web/build-node` bundle. `VITE_DESKTOP_E2E` must not be set for production builds.

Optional settings:

- `DESKTOP_E2E_TIMEOUT_MS`: analysis/render timeout (defaults to 30 minutes).
- `DESKTOP_E2E_WEBDRIVER_PORT`: WebDriver port (defaults to 4444).
- `DESKTOP_E2E_KEEP_OUTPUT=1`: retain the app's isolated temporary state directory. The rendered MP4 remains in artifacts regardless.
- `DESKTOP_E2E_RUNTIME_ENV`: source `runtime.env` copied to the isolated state directory (default: the normal app state file).
- `DESKTOP_E2E_TRANSCRIPT_FIXTURE`: absolute path to a transcript JSON file (matching the `Transcript` contract). When set, the analysis worker loads it instead of running Whisper — this unlocks the `render-success` scenario on machines without Whisper installed. The worker rebinds the transcript's `sourceArtifactId` to the real job id and runs the highlight selectors on it.
- `DESKTOP_E2E_SCENARIO`: `render-success` (default) exports one clip and verifies a valid MP4. `render-progress-cancel` selects every candidate, starts the batch render, waits for real progress percentages and the cancel control, cancels mid-flight, then requires: a cancellation message, no "Rendered " success claim, no leftover export, the render button re-enabled, and a `cancelled` render record in the isolated project database with no `completed` one. `crop-path` runs the local face-tracking crop path (see below). `report.json` records the sampled `renderTimeline` for the render scenarios.
- `LOCAL_FIRST_VENV`: the Python venv the app uses for the transcriber and the face detector. It defaults to `~/.cache/local-first-ai-video-studio/.venv` — the same venv `scripts/setup-machine.sh` creates. Point it at a different venv if the machine keeps one elsewhere.

### The `crop-path` scenario

This scenario exercises the local crop path end to end, through the app's own IPC: the
Haar-cascade face detector finds the speaker's face in real frames, `analyze_crop_path`
tracks and smooths it, the result becomes contract crop keyframes, and `render_video`
crops the export to follow the subject. It requires the source video to show a visible
face — the cascade cannot see a talking head that is off-screen, and the scenario fails
with a clear message when it finds no face.

It starts before the video is loaded into the UI player: the scenario drives the crop
pipeline through `invoke` and does not need the webview preview, and keeping the player
out of the picture matters on memory-tight machines (WebKit holding a decoded 720p
source alongside the detector is what kills the session on a 7 GB desktop). It fails
fast with a named reason when the face runtime is missing instead of timing out.

The speaker labels are supplied by the test rather than by diarization. Diarization
needs pyannote, a Hugging Face token and a model download, none of which a clean machine
or a CI runner has; but the crop path only needs to know *which* speaker is talking when,
and the test can say that. Everything downstream of that is the real detector, the real
tracker and the real renderer.

It needs OpenCV in the app's Python venv. `scripts/setup-machine.sh` installs
`opencv-python-headless<5` for it — OpenCV 5 removed the cascade API these scripts use.
Without it the scenario fails with a message naming the missing runtime instead of
timing out.

The test uses a fresh `LOCAL_FIRST_STATE_DIR`, so it does not touch a developer's real projects or cached state. The selected source video remains untouched. `report.json` and DOM/screenshot artifacts are written to the artifacts directory. Screenshot capture is best effort because WebKitWebDriver versions differ in screenshot support.

## AI UX review

After a run, give an AI reviewer the task in `report.json` and available artifacts. Ask it to identify points where a first-time user might hesitate, misunderstand status, or fail to find the next step. This is an AI heuristic review, not a substitute for usability sessions with real users. The test verifies behavior and output; it does not prove that the interface is intuitive.

This test is separate from the fast existing `npm test` suite and is not wired into CI until the driver/display setup is validated on the target Linux runner.
