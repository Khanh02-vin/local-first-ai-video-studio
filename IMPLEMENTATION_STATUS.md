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
- Local LLM highlight strategy (`HIGHLIGHT_STRATEGY=semantic-local`): `llama-server` (bundled linux x64 binary) + Qwen2.5-3B Q4 GGUF downloaded on demand with SHA-256 verification; Tauri spawns/stops the server (`start_local_llm`/`stop_local_llm`), `LlamaCppHighlightProvider` calls its OpenAI-compatible API per transcript window (map-reduce). No API key, no network egress. See `docs/local-llm.md`.
- Real LLM windowing + structured output (AI pipeline hardening): both `GeminiHighlightProvider` and `LlamaCppHighlightProvider` slice transcripts into 180s windows with 30s overlap (configurable via `windowSeconds`/`windowOverlapSeconds`, forwarded from `generateSemanticHighlights` through `SemanticProviderAdapter`) — long videos no longer overflow a single small-model prompt; per-window proposals merge and de-overlap via `mergeWindowHighlights`. Both providers now request schema-enforced JSON (`response_format: {type:"json_object", schema}` on llama-server, `responseMimeType:"application/json"` + `responseSchema` on Gemini), so replies parse without fence-stripping; `repairTruncatedJsonArray` remains as the fallback for llama-server builds without constrained decoding.
- Highlight-selection eval harness (`tests/ai-pipeline/eval-highlights.test.ts` + `fixtures/highlight-eval.json`, wired as `test:eval-highlights` in `npm test`): 6 curated fixtures / 8 human-picked expected highlights, scored by time-IoU + rank@1/3/5 (threshold 0.3). Offline CI rows: heuristic, content, and two oracle semantic mocks (exact + ±2s jitter) that pin the metric code itself. Live rows auto-skip unless `GEMINI_API_KEY` is set or a llama-server answers `/v1/models` — with a real server the same run compares the bundled Qwen against the heuristic. Findings from the first live run (Qwen2.5-0.5B, llama.cpp b11160): heuristic IoU 0.75 vs semantic-local IoU 0.02 — the bundled 0.5B does not beat the heuristic on highlight picking, and it fails for two fixable reasons the harness exposed: (1) the old word-object JSON prompt cost ~4.2k tokens for a 96s clip, overflowing the production llama-server context (`ensure_llama_server` ran 4096 ctx / parallel 2 = 2048 tokens per slot, so every real window silently 400'd and fell back to the heuristic anyway); (2) 0.5B wastes its token budget echoing wordIds and picks single-word spans. Fixes shipped with the harness: prompts are now compact `wordIndex [start-end] text` lines (~3.5x smaller) with wordIds/times re-derived server-side (`highlightPrompt`/`mapWordRangeProposals`, schema no longer contains wordIds), and `ensure_llama_server` runs 8192 ctx / parallel 1. Recommendation recorded by the data: keep the heuristic as the default selector; treat semantic-local as opt-in for machines that download a larger model (1.5B+), and re-run `LOCAL_LLM_BASE_URL=... npm run test:eval-highlights` to compare before changing the bundle.
- Whisper models `tiny`/`base` bundled in Tauri resources and copied to `~/.cache/whisper` on startup (`ensure_bundled_whisper_model`) — no model download required on a fresh machine.
- Draggable timeline editor (in/out trim handles, range drag, keyboard slider) — replaces the HTML5 placeholder; edits still flow through `editor-core` `updateRange()` so contract validation stays the single source of truth.
- YouTube Studio RAG backend (`apps/web/src/routes/api/rag/+server.ts`, dispatching rest-route `apps/web/src/routes/api/[...rest]/+server.ts`): playlist queue, manual segment ingest, TF-IDF timestamp search with deterministic synthesis — JSON files under `<state>/youtube`, strict validation, no network. YouTube Studio rail entry un-hidden; web app builds `adapter-static` for the desktop UI and `adapter-node` (`build-node`) for the RAG sidecar.
- Hybrid RAG retrieval + local-LLM answers (YouTube Studio): ingest embeds each transcript segment via the bundled llama-server `/v1/embeddings` (vectors stored in `<state>/youtube/embeddings.json`); query ranks by cosine top-k when the LLM is reachable and falls back to the existing TF-IDF path otherwise (dims mismatch from a model swap also forces the fallback). Answer synthesis feeds the top excerpts to the local Qwen (`/v1/chat/completions`) and falls back to the deterministic excerpt string on any error/timeout; `synthesize:false` skips the LLM. The sidecar learns the LLM URL from `LOCAL_LLM_BASE_URL` (set by `ensure_rag_server`, default `127.0.0.1:8080`). `ensure_llama_server` now spawns with `--embeddings --pooling mean` — Qwen chat GGUFs default to pooling `none`, which the embeddings endpoint rejects; forcing mean pooling lets one server both embed and chat (verified against llama.cpp b11160: embeddings 896-dim, chat unaffected, related/unrelated cosine margin ~0.11 — no extra embedding model needed).
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
eval highlight tests: ok
rag hybrid tests: ok
youtube transcript tests: ok
command wiring tests: ok (30 UI invocations all registered)

cd apps/desktop/src-tauri && cargo check
Finished `dev` profile
cargo test
test result: ok. 5 passed; 0 failed

cd apps/web
npm run check
svelte-check found 0 errors and 3 warnings (pre-existing: slot deprecation in layout, implicit-close + a11y in Studio/Youtube pages)
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
- `resources/ffmpeg` (gitignored, populated by `fetch-bundled-assets.sh` or a host copy) wins over PATH via `bundled_bin`. The johnvansickle static 7.0.2 Linux build that was sitting there segfaults (SIGSEGV) on **any** HTTPS input on this machine, so render died with `FFMPEG_EXIT:signal: 11`; replaced locally with the system ffmpeg 8.0.1 (`cp /usr/bin/ffmpeg resources/ffmpeg`) and the GUI export then succeeded. The same replacement was applied to `~/.local/bin/ffmpeg` + `ffprobe`, which the self-hosted runner's PATH resolves first when `fetch-bundled-assets.sh` takes its host-copy branch — otherwise every CI AppImage would have shipped the broken binary. If a render fails with `FFMPEG_EXIT:signal: 11` on a fresh Linux install, check that binary first — Windows CI downloads its own `ffmpeg.exe` from BtbN and is unaffected.
- Tauri desktop shell loads the static UI from `frontendDist`; the RAG sidecar (`build-node`, spawned on demand by `rag_server_url`, stopped on exit) serves `/api/rag/*` on `127.0.0.1:4733` and the webview reaches it through `tauri-plugin-http`. Verified: full `.deb` release build, app boots, sidecar health `200` with the exact spawn env — but the click-through (load YouTube Studio → plugin fetch returns data) has not been exercised end-to-end in a real GUI session.
- macOS/Windows installers: `release.yml` builds the Windows portable zip + MSI on `windows-latest`, the macOS `.app` bundle on `macos-latest`, and the Linux AppImage on the self-hosted `[self-hosted, self-vostro]` runner. Run `36600662949` (commit `60b3ead`, 2026-09-30) went fully green on all five jobs (`lint-test`, `windows`, `macos`, `linux`, `publish`) after the repo was made public — which lifted the hosted-runner billing gate that had blocked `windows`/`macos` since 2026-09-27 (see "GitHub Actions billing block"). See "Release artifacts" below for the customer links.
- Customer-reported stability/perf fixes (2026-09-30, triage in `docs/customer-feedback-2026-09-30.md`): all helper processes spawn through `tool()`, which sets `CREATE_NO_WINDOW` on Windows so node/curl/kill no longer flash console windows; `panic::set_hook` logs panics to `panic.log` and the next launch surfaces the report in-app (`last_panic`), so the Windows release build's silent exits become readable; `ensure_bundled_whisper_model` copies the Whisper `.pt` into the CLI cache on a background thread instead of inside `runtime_status` (first-run page loads no longer freeze for the multi-second copy); engine/storage/license status is cached in `apps/web/src/lib/runtime-status.svelte.ts` and polled briefly until the copy finishes.
- Windows helper layout: `fetch-bundled-assets.sh` now ships `node/bin/node.exe`, `ffmpeg.exe`, `ffprobe.exe` (Windows CreateProcess resolves `.exe`; the extensionless Unix names failed there and made the portable fall back to a PATH node that customer machines lack), `bundled_bin` probes the platform-native name first, `release.yml`'s portable assembly and a new "Verify bundled helper names" CI step (windows + linux jobs) guard against regressions before packaging. `tauri.conf.json` bundles ffmpeg/ffprobe via glob (`./resources/ffmpeg*`) so one config serves both naming schemes.
- Desktop shell UX: `decorations: false` + an in-app title bar (`data-tauri-drag-region`, min/max/close via `@tauri-apps/api/window`); pages are keep-alive mounted by `+layout.svelte` (display-toggled) so switching never re-runs `onMount` — measured 87–126 ms warm switches and 20 rapid switches with no crash on the release build. Caution when touching the layout: the visited-pages effect must write `visited[page]` only when missing — reassigning a fresh object there loops forever and starves Svelte's update queue (invoke results silently stop rendering). `WEBVIEW2_GPU_DISABLE=1` in `runtime.env` opts Windows machines into `--disable-gpu-compositing` (mirrors the WebKitGTK DMABUF workaround) for WebView2 renderer crashes.
- Collection contract is defined and validated but not yet wired into persistence, API, or UI (scope is P2).
- Editor uses native HTML video/timeline controls; add CE.SDK only if this editor fails real-user requirements.
- Tauri 2 installers (deb/rpm/AppImage) are produced but not yet code-signed; shipping to a store requires signing + notarization (see docs/codesigning-runbook.md).
- Cloud, OAuth and publishing remain deliberately disabled.

## Release artifacts (customer-facing links)

All installers ship as GitHub Release `v0.1.0` assets and are mirrored to the `gdrive:LFaIVS-Releases/` folder.

- **Linux:** `Local-first AI Video Studio_0.1.0_amd64.AppImage` — 936 151 544 bytes, built by the `linux` job of run `36803798016` (`cc842ac`, 2026-10-01 — ships the customer stability/perf fixes). The host `~/.local/bin/ffmpeg` that `fetch-bundled-assets.sh`'s host-copy branch picks up was replaced with system ffmpeg 8.0.1 (the johnvansickle static 7.0.2 there segfaults on HTTPS), so the image ships a working ffmpeg.
- **macOS:** `Local-first AI Video Studio.zip` — 848 659 888 bytes, `.app` bundle from the same run (unsigned: right-click → Open on first launch).
- **Windows:** `Local-first AI Video Studio_0.1.0_x64_en-US.msi` — 906 649 780 bytes (signed), and `Local-first AI Video Studio Portable.zip` — 914 911 187 bytes (unzip; double-click `local-first-ai-video-studio.exe` to run).

> **Portable format fix (2026-10-01).** The previous "portable zip" files were actually uncompressed TAR archives: `windows-latest`'s GNU tar `-a` only infers *compression* from the suffix, and `.zip` is not a compression format it knows, so it wrote ustar — unopenable by Windows Explorer. `release.yml` now builds it with `7z -tzip` (7-Zip preinstalled on the runner) plus a `7z t` integrity gate, and the deflated zip also shrank the download from ~1.39 GB to ~915 MB. Run `36819798478` (`ac6b917`) built all four jobs green with the fix; the portable's content was patched in place onto its existing Drive id `14EXa9b…` so previously sent links serve the corrected archive. Its `publish` job was cancelled after the builds went green because the Drive/GitHub refresh had already been done manually from the same run's artifacts (the job's artifact re-download alone takes ~40 min on this link). The self-hosted runner's watchdog was also hardened: it no longer restarts `gh-runner` while a job is in flight (`Runner.Worker` alive) and requires two consecutive offline checks — an offline-during-heavy-load restart had twice cancelled the linux AppImage build mid-bundling.

Run `36803798016` (`cc842ac`, 2026-10-01) built all four from green jobs (`lint-test`, `windows`, `macos`, `linux`); its `publish` job re-downloaded ~4.6 GB of artifacts and its `softprops/action-gh-release` step flaked mid-upload after deleting the previous assets, so the release refresh + Drive upload were completed manually from the same run's artifacts (Drive verified via API: same 4 ids, new content, `anyone:reader` intact; the portable was content-patched in place onto its existing id so previously sent links keep working). The CI `Verify bundled helper names (Windows)` step caught a real regression on the first dispatch (`cc842ac`'s predecessor `652a3f2` shipped a Windows node zip whose top-level dir was never stripped, so `node/bin/node.exe` was missing); fixed in `cc842ac`.

Google Drive direct links (folder `gdrive:LFaIVS-Releases/`, all `anyone-with-link: reader`):

| File | Drive id | Link |
|---|---|---|
| `Local-first AI Video Studio_0.1.0_amd64.AppImage` (Linux) | `1C6caeoH5fV_-CGARWDzStg2KIDygfjDV` | [open](https://drive.google.com/file/d/1C6caeoH5fV_-CGARWDzStg2KIDygfjDV/view) |
| `Local-first AI Video Studio_0.1.0_x64_en-US.msi` (Windows signed) | `1qK9C-fncFJUVYdkrmchzccasb2jI8vr5` | [open](https://drive.google.com/file/d/1qK9C-fncFJUVYdkrmchzccasb2jI8vr5/view) |
| `Local-first AI Video Studio.zip` (macOS) | `139vD_Jyb7bB6pygaM68Q1TpIsTQHAH1k` | [open](https://drive.google.com/file/d/139vD_Jyb7bB6pygaM68Q1TpIsTQHAH1k/view) |
| `Local-first AI Video Studio Portable.zip` (Windows portable) | `14EXa9b2ioGWZKAgk4vkSo4e_RqFolgd2` | [open](https://drive.google.com/file/d/14EXa9b2ioGWZKAgk4vkSo4e_RqFolgd2/view) |

The same three installers (AppImage/MSI/macOS zip) are also attached to GitHub Release `v0.1.0`, refreshed from the same run.

> **Build provenance.** Everything above comes from run `36600662949` at commit `60b3ead`, which includes `da15097` (the YouTube URL → highlight → MP4 feature) and the `release.yml` `shell: bash` fix — the Windows `portable-zip` assembly step previously died on `windows-latest` because the default PowerShell rejects `for exe in *.exe; do …; done`; no code change was needed, only `shell: bash`. Earlier stale artifacts were removed from Drive: the pre-feature portable zip (old id `1hfdZKqCOj2rd49ZCTaqYHBjZ4Jat9nXm`, built at `c5f8dbb`) and the Sep-28 dotted-name MSI/macOS duplicates are gone (in Drive trash); an accidentally created folder masquerading under the portable name was deleted too.

## GitHub Actions billing block (self-inflicted, not a code defect)

The public `ci.yml` test job and the `release.yml` `windows`/`macos` jobs run on GitHub-hosted runners (`ubuntu-latest`, `windows-latest`, `macos-latest`). Since **2026-09-27 15:56**, every one of those jobs fails immediately with zero steps and the annotation:

> The job was not started because recent account payments have failed or your spending limit needs to be increased.

The repo was made **public** on 2026-09-29, which lifted the GitHub-hosted runner billing gate for `ubuntu-latest`, `windows-latest`, and `macos-latest`; run `36600662949` the next day was fully green on all five jobs, producing every artifact listed above. If the repo must ever go private again, the gate returns unless the payment method / Actions spending limit is fixed in *Settings → Billing & plans*.

What keeps working: the self-hosted `[self-hosted, self-vostro]` jobs (`lint-test`, `linux`, `publish`) — they report directly against the linux host and are how the Linux AppImage above was produced. This is also why `release.yml` was split so the `linux` job stages its AppImage to `~/lfavis-dist/linux/` and the `publish` job reads it from that shared dir instead of round-tripping through the GitHub artifact store.

The Windows `fetch-bundled-assets.sh` step downloads a fresh BtbN `ffmpeg.exe` on the runner, so the green `windows` job in run `36600662949` shipped the correct ffmpeg without any local intervention.
