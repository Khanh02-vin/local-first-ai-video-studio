# Implementation status

## Local-first MVP decision

The production path is intentionally reduced to one desktop/local process:

```text
Svelte/SvelteKit → SQLite → filesystem → Faster-Whisper local → FFmpeg → MP4
```

> **Architecture — dual build, static UI + RAG sidecar.** `svelte.config.js`
> picks the adapter by `WEB_ADAPTER`: default → `adapter-static` → `web/build`
> (Tauri `frontendDist`, embedded into the desktop binary); `WEB_ADAPTER=node`
> → `adapter-node` → `web/build-node` (the RAG server). `beforeBuildCommand`
> runs both (`npm run build && npm run build:node`), and `build-node` ships as
> the `build-node` app resource.
>
> The static webview (origin `tauri://localhost`) cannot `fetch` the sidecar
> directly — webview CSP is `default-src 'self'`. So the YouTube Studio page
> resolves the sidecar base URL from the Tauri command `rag_server_url` (which
> spawns the sidecar on `127.0.0.1:4733` if it is not already healthy) and
> routes every RAG call through `tauri-plugin-http` — its HTTP runs in Rust, so
> webview CSP/CORS do not apply. The sidecar is stopped on app exit.
> Standalone (browser served by `build-node`) keeps same-origin relative fetch.

Deferred until measured demand:

- PostgreSQL, Redis/BullMQ, S3/R2, cloud workers.
- JWT/session/tenant isolation and network rate limiting.
- CE.SDK.
- Managed (non-BYOK) Gemini/OpenAI cloud AI.
- OAuth/social publishing.
- Billing/team workspace.

## Completed

- Phase 0 documentation and scope.
- Phase 1 independent project skeleton.
- Phase 2 canonical contracts and validation tests.
- Phase 3 host-agnostic media engine modules.
- SQLite local job store with retry/recovery states.
- Local FFmpeg adapter with probe/render/cancel.
- Local Faster-Whisper CLI adapter with cleanup, timeout and transcript validation.
- Deterministic heuristic highlight selector using sentence completeness, length and hook keywords.
- Semantic highlight strategy + Gemini BYOK provider wired into the analysis worker (`HIGHLIGHT_STRATEGY`/`GEMINI_API_KEY` in runtime.env, `SemanticProviderAdapter` bridges the LLM provider into the pipeline); deterministic heuristic/content selector is the offline default and the automatic fallback when the provider errors or no key is set.
- `Collection` canonical contract v1 (`packages/contracts/collection.ts`): items reference highlights/renderPlans/artifacts, contiguous positions starting at 0, no duplicate references, state ∈ {proposed, approved, rejected}, exported from barrel.
- Tauri 2 installer bundles: `.deb`, `.rpm`, and `.AppImage` in `apps/desktop/src-tauri/target/release/bundle/` (Node, ffmpeg, ffprobe, and all TS contracts/adapters/services bundled as app resources).
- `scripts/run-local-demo.ts`: one-command local pipeline (probe → extract audio → Whisper → highlights → 9:16 MP4).
- Local LLM highlight strategy (`HIGHLIGHT_STRATEGY=semantic-local`): `llama-server` (bundled linux x64 binary) + Qwen2.5-3B Q4 GGUF downloaded on demand with SHA-256 verification; Tauri spawns/stops the server (`start_local_llm`/`stop_local_llm`), `LlamaCppHighlightProvider` calls its OpenAI-compatible API per 20-minute transcript window (map-reduce). No API key, no network egress. See `docs/local-llm.md`.
- Whisper models `tiny`/`base` bundled in Tauri resources and copied to `~/.cache/whisper` on startup (`ensure_bundled_whisper_model`) — no model download required on a fresh machine.
- Draggable timeline editor (in/out trim handles, range drag, keyboard slider) — replaces the HTML5 placeholder; edits still flow through `editor-core` `updateRange()` so contract validation stays the single source of truth.
- YouTube Studio RAG backend (`apps/web/src/routes/api/rag/+server.ts`, dispatching rest-route `apps/web/src/routes/api/[...rest]/+server.ts`): playlist queue, manual segment ingest, TF-IDF timestamp search with deterministic synthesis — JSON files under `<state>/youtube`, strict validation, no network. YouTube Studio rail entry un-hidden; web app builds `adapter-static` for the desktop UI and `adapter-node` (`build-node`) for the RAG sidecar.
- RAG sidecar inside the desktop app: dual-adapter build (`WEB_ADAPTER=node` → `build-node`, bundled as a resource) + Tauri command `rag_server_url` spawns the sidecar on `127.0.0.1:4733` (bundled node, stopped on exit) + `tauri-plugin-http` so the static webview reaches it despite webview CSP. Fixed a runtime panic where `LlamaState` and `RagState` were the same underlying managed type (`Arc<Mutex<Option<u32>>>`) — `RagState` is now a distinct newtype.
- Fast YouTube clipping (URL → highlight → preview → export): paste a YouTube URL on the Studio home → `start_youtube_analysis` validates the host and reads duration metadata via `yt-dlp --skip-download` (~1s) → `scripts/analyze-youtube.ts` builds the transcript contract from captions (`scripts/yt-crawler.py --video`, no Whisper, no download) → existing heuristic/semantic selector emits highlights → the step-4 iframe seeks the clip (`start`/`end`) → export resolves fresh CDN URLs at render time (`resolve_stream_url`) and range-requests only the selected section before the usual 9:16 re-encode.

## Verified

```text
cd local-first-ai-video-studio
npm test
contract tests: ok
media engine tests: ok
local runtime tests: ok
MVP pipeline tests: ok
local MVP heuristic tests: ok
platform boundary tests: ok
semantic highlight tests: ok
idempotency gap tests: ok (all 6 gaps fixed)
long-video map-reduce tests: ok
json repair tests: ok
youtube transcript tests: ok
command wiring tests: ok (29 UI invocations all registered)

cd apps/desktop/src-tauri && cargo check
Finished `dev` profile
cargo test
test result: ok. 5 passed; 0 failed

cd apps/web
npm run check
svelte-check found 0 errors and 1 warning (pre-existing slot deprecation in layout)
npm run build
built successfully

manual e2e (local LLM): llama-server + Qwen2.5-3B GGUF + LlamaCppHighlightProvider

Clean-machine Docker test (tests/clean-machine/Dockerfile, ubuntu:24.04,
no whisper/venv/state dir; ffmpeg from apt, bundled node/llama/whisper
weights):
  docker build -f tests/clean-machine/Dockerfile -t lfai-clean-machine .
  docker run lfai-clean-machine bash tests/clean-machine/run-whisper-bootstrap.sh
    → venv + openai-whisper + torch-2.14.0+cpu, runtime.env written,
      bundled tiny.pt copied to ~/.cache/whisper
  docker run lfai-clean-machine bash tests/clean-machine/run-analyze-pipeline.sh
    → probe → extract → whisper (0 words on synthetic tone, expected) →
      highlight → 9:16 render: /tmp/short-clean.mp4 1080x1920 OK
    (idempotent: second bootstrap run skips pip install, ~2s)

RAG backend (adapter-node server, LOCAL_FIRST_STATE_DIR=/tmp/rag-test-state):
  POST /api/rag/playlists (202 queued) · POST /api/rag/ingest (200,
  2 segments) · POST /api/rag/query (TF-IDF match + synthesis) ·
  DELETE /api/rag/playlists/<id> (record removed) · GET /api/rag/index
  (TF-IDF index persisted) · bad-domain playlist → 400
  PLAYLIST_MUST_BE_YOUTUBE_URL · unrelated /api/foo → 404 UNKNOWN_ENDPOINT.
```

- Desktop GUI e2e (release build, Wayland GNOME + Xwayland, driven through
  AT-SPI/XSendEvent): fresh app → paste `https://www.youtube.com/watch?v=9bZkp7q19f0`
  → step 4 with 3 Korean highlights in seconds (no Whisper phase) → preview
  iframe plays with **no YouTube Error 153** → seek slider resets 100→0 on
  replay and auto-stops at the clip end (`start=9&end=26`) → Render selected →
  GTK save dialog → `Rendered: …/src-tauri/yt-short.mp4` — ffprobe: h264
  1080×1920, 15.77 s, aac 44.1 kHz, 9.9 MB. The preview is served by the
  loopback shim (`preview_origin` → `http://localhost:14872`; `/health`,
  `/preview` with the `preview_query` whitelist).

- SvelteKit shell with dashboard, project, editor and settings routes.
- Config, upload validation, local auth-scope boundary and in-memory rate-limit boundary retained as future cloud seams.
- Cloud/provider/OAuth interfaces retained outside the local runtime; no fake production credentials.
- Docker development services and CI smoke workflow retained for future expansion.

## Current MVP limitations

- Faster-Whisper still requires the local `whisper` CLI (auto-discovered via the project venv / runtime.env fallback chain); `tiny`/`base` model weights are bundled.
- Highlight strategies (Settings → Highlight strategy): heuristic (offline default), semantic-gemini (BYOK), semantic-local (llama.cpp, offline). Without an explicit choice the deterministic heuristic/content selector stays the offline default and provider errors fall back to it.
- Local LLM: binary committed for linux x64 only; mac/windows need their llama.cpp build added to resources when built on the right runner (release.yml runs fetch-bundled-assets.sh per-OS, which pulls the matching llama.cpp release). The 2.1 GB GGUF is downloaded on first use (never committed to git). The `llama/` resource dir is shipped in the installer via tauri.conf.json (bundle.resources `./resources/llama`).
- YouTube Studio RAG: index/query/ingest/delete are working; the playlist **crawler** (`scripts/yt-crawler.py`, `crawl_playlist`) fills queued playlists with real video segments, and its `--video <url>` mode feeds the single-video analysis pipeline (transcript → highlights, no Whisper).
- YouTube preview embed: the production webview origin `tauri://localhost` carries no HTTP(S) `Referer`, which YouTube rejects with `EMBEDDER_IDENTITY_MISSING_REFERRER` (on-screen "Error 153"). Fixed by `preview_origin`: an in-process loopback server on `127.0.0.1:14872` (same pattern as `YT_OAUTH_PORT=14871`) whose page embeds the clip with a `http://localhost:14872` Referer; the iframe CSP stays tight (`frame-src` = YouTube hosts + the shim only), and `/preview` validates video id (`[A-Za-z0-9_-]{11}`) plus `start < end`. Windows/Android (`http://tauri.localhost`) would work natively; dev mode (`http://localhost:5173`) also works.
- `resources/ffmpeg` (gitignored, populated by `fetch-bundled-assets.sh` or a host copy) wins over PATH via `bundled_bin`. The johnvansickle static 7.0.2 Linux build that was sitting there segfaults (SIGSEGV) on **any** HTTPS input on this machine, so render died with `FFMPEG_EXIT:signal: 11`; replaced locally with the system ffmpeg 8.0.1 (`cp /usr/bin/ffmpeg resources/ffmpeg`) and the GUI export then succeeded. If a render fails with `FFMPEG_EXIT:signal: 11` on a fresh Linux install, check that binary first — Windows CI downloads its own `ffmpeg.exe` from BtbN and is unaffected.
- Tauri desktop shell loads the static UI from `frontendDist`; the RAG sidecar (`build-node`, spawned on demand by `rag_server_url`, stopped on exit) serves `/api/rag/*` on `127.0.0.1:4733` and the webview reaches it through `tauri-plugin-http`. Verified: full `.deb` release build, app boots, sidecar health `200` with the exact spawn env — but the click-through (load YouTube Studio → plugin fetch returns data) has not been exercised end-to-end in a real GUI session.
- macOS/Windows installers: `release.yml` builds the Windows portable zip + MSI on `windows-latest` and the macOS `.app` bundle on `macos-latest`. GitHub-hosted runners for both are currently billing-gated (see "GitHub Actions billing block" below) — the Windows job has failed identically on every dispatch since 2026-09-27, so a fresh portable cannot be produced locally. The Linux AppImage is built on the self-hosted `[self-hosted, self-vostro]` runner (unaffected) and uploaded to its own Drive file; see "Release artifacts" below. macOS additionally needs the `APPLE_*` / `TAURI_SIGNING_*` secrets and a green run before shipping.
- Collection contract is defined and validated but not yet wired into persistence, API, or UI (scope is P2).
- Editor uses native HTML video/timeline controls; add CE.SDK only if this editor fails real-user requirements.
- Tauri 2 installers (deb/rpm/AppImage) are produced but not yet code-signed; shipping to a store requires signing + notarization (see docs/codesigning-runbook.md).
- Cloud, OAuth and publishing remain deliberately disabled.

## Release artifacts (customer-facing links)

All installers ship as GitHub Release `v0.1.0` assets and are mirrored to the `gdrive:LFaIVS-Releases/` folder.

- **Linux:** `Local-first AI Video Studio_0.1.0_amd64.AppImage` (built at `da15097` on the self-hosted runner; the bundled `ffmpeg` in the AppDir was the static johnvansickle 7.0.2 build that segfaults on HTTPS, so it was overwritten with the host `/usr/bin/ffmpeg` 8.0.1 before packaging with `appimagetool`, then re-uploaded by name to Drive). md5 `7ea2890fb1cda683c51c831e740fd6e4`.
- **macOS:** `Local-first.AI.Video.Studio_0.1.0_amd64.zip` (`.app` bundle, unsigned — right-click → Open on first launch).
- **Windows:** `Local-first.AI.Video.Studio_0.1.0_x64_en-US.msi` (signed) and `Local-first AI Video Studio Portable.zip` (unzip; double-click `local-first-ai-video-studio.exe` to run).

Google Drive direct links:

| File (in `gdrive:LFaIVS-Releases/`) | Drive id |
|---|---|
| `Local-first AI Video Studio_0.1.0_amd64.AppImage` | `1C6caeoH5fV_-CGARWDzStg2KIDygfjDV` |
| `Local-first.AI.Video.Studio_0.1.0_x64_en-US.msi` | `10i00IPnu1EZrtk0ZoxhvtX5I47I9Ct31` |
| `Local-first.AI.Video.Studio.zip` (portable Windows sources / legacy) | `1dFDcfVoJ-nQnhXrLn4DpLP51t4iEVyzN` |
| `Local-first AI Video Studio Portable.zip` (stale) | `1hfdZKqCOj2rd49ZCTaqYHBjZ4Jat9nXm` |

> **Stale-portable notice.** `Local-first AI Video Studio Portable.zip` (Drive id `1hfdZKqCOj2rd49ZCTaqYHBjZ4Jat9nXm`, md5 `1dc16a0c7c1fdc990642b9470b923130`) was produced at `c5f8dbb` — the commit *before* `da15097`. It does **not** contain the YouTube URL → highlight → MP4 feature and is kept here only because the replacement requires a green `windows-latest` job (billing-blocked; see below). The Linux AppImage linked from the same release **does** contain the feature end-to-end.

## GitHub Actions billing block (self-inflicted, not a code defect)

The public `ci.yml` test job and the `release.yml` `windows`/`macos` jobs run on GitHub-hosted runners (`ubuntu-latest`, `windows-latest`, `macos-latest`). Since **2026-09-27 15:56**, every one of those jobs fails immediately with zero steps and the annotation:

> The job was not started because recent account payments have failed or your spending limit needs to be increased.

The repository is private under a free (`plan: null`) account, so the `gh` token cannot reach `/user/settings/billing/actions` (returns `404 Not Found`). Only the account owner can lift this by resolving the failed payment / raising the Actions spending limit.

What keeps working: the self-hosted `[self-hosted, self-vostro]` jobs (`lint-test`, `linux`, `publish`) — they report directly against the linux host and are how the Linux AppImage above was produced. This is also why `release.yml` was split so the `linux` job stages its AppImage to `~/lfavis-dist/linux/` and the `publish` job reads it from that shared dir instead of round-tripping through the GitHub artifact store.

The Windows `fetch-bundled-assets.sh` step downloads a fresh BtbN `ffmpeg.exe` on the runner, so a future green `windows` job will ship the correct ffmpeg without any local intervention.
