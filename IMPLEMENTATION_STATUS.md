# Implementation status

## Local-first MVP decision

The production path is intentionally reduced to one desktop/local process:

```text
Svelte/SvelteKit → SQLite → filesystem → Faster-Whisper local → FFmpeg → MP4
```

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
- Local LLM: binary currently bundled for **linux x64 only**; mac/windows need their llama.cpp build added to resources when built on the right runner. The 2.1 GB GGUF is downloaded on first use (never committed to git).
- Collection contract is defined and validated but not yet wired into persistence, API, or UI (scope is P2).
- Editor uses native HTML video/timeline controls; add CE.SDK only if this editor fails real-user requirements.
- Tauri 2 installers (deb/rpm/AppImage) are produced but not yet code-signed; shipping to a store requires signing + notarization.
- Cloud, OAuth and publishing remain deliberately disabled.
