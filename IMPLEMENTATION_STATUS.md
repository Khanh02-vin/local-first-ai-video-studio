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
- Gemini/OpenAI cloud AI.
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
- Semantic highlight strategy stub (`services/ai-pipeline/semantic.ts`): `SemanticHighlightProvider` interface, `generateSemanticHighlights` orchestrator with normalization/validation/de-overlap, `NotImplementedSemanticProvider` that fails explicitly (so orchestrator can choose deterministic fallback), and deterministic `createMockSemanticProvider` for tests. Heuristic remains the default; semantic only runs when a provider is injected.
- `Collection` canonical contract v1 (`packages/contracts/collection.ts`): items reference highlights/renderPlans/artifacts, contiguous positions starting at 0, no duplicate references, state ∈ {proposed, approved, rejected}, exported from barrel.
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
- Heuristic highlights remain the offline default; semantic highlight is a strategy stub only — production provider (Gemini/OpenAI) integration awaits acceptance data per ADR-002, and no provider is shipped.
- Collection contract is defined and validated but not yet wired into persistence, API, or UI (scope is P2).
- Editor uses native HTML video/timeline controls; add CE.SDK only if this editor fails real-user requirements.
- Tauri 2 Rust shell and dialog capability are scaffolded; native runner command bridge, installer and signing remain release work.
- Cloud, OAuth and publishing remain deliberately disabled.
