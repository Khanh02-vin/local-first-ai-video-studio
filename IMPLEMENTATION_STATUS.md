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

cd apps/desktop/src-tauri && cargo check
Finished `dev` profile

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
- SvelteKit shell with dashboard, project, editor and settings routes.
- Config, upload validation, local auth-scope boundary and in-memory rate-limit boundary retained as future cloud seams.
- Cloud/provider/OAuth interfaces retained outside the local runtime; no fake production credentials.
- Docker development services and CI smoke workflow retained for future expansion.

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

cd apps/desktop/src-tauri && cargo check
Finished `dev` profile

cd apps/web
npm run check
svelte-check found 0 errors and 1 warning (pre-existing slot deprecation in layout)
npm run build
built successfully

manual e2e (local LLM): llama-server + Qwen2.5-3B GGUF + LlamaCppHighlightProvider
```

## Current MVP limitations

- Faster-Whisper still requires the local `whisper` CLI (auto-discovered via the project venv / runtime.env fallback chain); `tiny`/`base` model weights are bundled.
- Highlight strategies (Settings → Highlight strategy): heuristic (offline default), semantic-gemini (BYOK), semantic-local (llama.cpp, offline). Without an explicit choice the deterministic heuristic/content selector stays the offline default and provider errors fall back to it.
- Local LLM: binary committed for linux x64 only; mac/windows need their llama.cpp build added to resources when built on the right runner (release.yml runs fetch-bundled-assets.sh per-OS, which pulls the matching llama.cpp release). The 2.1 GB GGUF is downloaded on first use (never committed to git). The `llama/` resource dir is shipped in the installer via tauri.conf.json (bundle.resources `./resources/llama`).
- YouTube Studio RAG: index/query/ingest/delete are working; the playlist **crawler** that would fill `status: queued` playlists with real video segments is not implemented yet — segments are ingested manually (paste `start-end | text` lines in the UI).
- Tauri desktop shell loads the static UI from `frontendDist`; the RAG sidecar (`build-node`, spawned on demand by `rag_server_url`, stopped on exit) serves `/api/rag/*` on `127.0.0.1:4733` and the webview reaches it through `tauri-plugin-http`. Verified: full `.deb` release build, app boots, sidecar health `200` with the exact spawn env — but the click-through (load YouTube Studio → plugin fetch returns data) has not been exercised end-to-end in a real GUI session.
- macOS/Windows installers: `release.yml` is configured (rust toolchain, per-OS llama.cpp fetch, web build, signing secrets), but no build has run on an actual macOS/Windows runner yet — needs the runner + `APPLE_*` / `TAURI_SIGNING_*` secrets.
- Collection contract is defined and validated but not yet wired into persistence, API, or UI (scope is P2).
- Editor uses native HTML video/timeline controls; add CE.SDK only if this editor fails real-user requirements.
- Tauri 2 installers (deb/rpm/AppImage) are produced but not yet code-signed; shipping to a store requires signing + notarization (see docs/codesigning-runbook.md).
- Cloud, OAuth and publishing remain deliberately disabled.
