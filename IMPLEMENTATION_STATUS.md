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
- SvelteKit shell with dashboard, project, editor and settings routes.
- HTML video/editor placeholder and versioned RenderPlan state.
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

cd apps/web
npm run check
svelte-check found 0 errors and 0 warnings
npm run build
built successfully
```

## Current MVP limitations

- Faster-Whisper requires the local `whisper` command and a downloaded/licensed model; no model is bundled.
- Semantic highlight ships with the Gemini BYOK provider (opt-in via Settings → Highlight strategy); without an API key the deterministic heuristic/content selector remains the offline default, and provider errors automatically fall back to it.
- Collection contract is defined and validated but not yet wired into persistence, API, or UI (scope is P2).
- Editor uses native HTML video/timeline controls; add CE.SDK only if this editor fails real-user requirements.
- Tauri 2 installers (deb/rpm/AppImage) are produced but not yet code-signed; shipping to a store requires signing + notarization.
- Cloud, OAuth and publishing remain deliberately disabled.
