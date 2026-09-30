# Customer feedback triage — 2026-09-30

Feedback (paraphrased): app crashes when switching between two pages; a console
window opens repeatedly; page switch feels slow (>2 s); the logo disappears
after switching; asks for a custom title bar and for client-side rendering so
pages load instantly.

Environment of the report: **Windows portable zip** (run `36600662949`).
Verification machine: Linux release build of the same commit, driven over
AT-SPI + frame capture (GUI harness in `/tmp/atspi_nav.py`, `/tmp/nav_time.py`).

## Measurements on the same code (Linux release)

| Scenario | Result |
|---|---|
| Warm page switch (Studio ↔ Settings ↔ YouTube Studio), heading appears | **112–400 ms** |
| 20 rapid switches, 100 ms apart | No crash, UI intact, logo intact, settles in 179 ms |
| `analysis-status.ts --active` (one `list_analysis_jobs` poll) | 90–140 ms + a **new node process** |
| RAG sidecar health check (one `curl`) | 50 ms |

So the CSR architecture itself is already fast — the >2 s and the console
windows are Windows-specific side effects of how the shell runs helpers, not
of SvelteKit. Each finding below maps a complaint to code and a fix.

## Finding-by-finding

### 1. "Nó mở console liên tục" — every helper process flashes a console (Windows)

**Root cause.** `main.rs` runs `node`, `curl`, `kill`/`taskkill`, `df` through
`std::process::Command` with default creation flags. Rust std does **not** set
`CREATE_NO_WINDOW`, and the app is built with `windows_subsystem = "windows"`
(no console of its own), so on Windows every spawned console tool gets its own
console window. The spawns are frequent:

- Studio page mount → `list_analysis_jobs` (node) + a 5 s `reconnectJobs`
  timer that spawns node **again every 5 s while the page is open**
- `analysis_status` polling spawns node every 500 ms during analysis
- YouTube Studio mount → `rag_server_url` spawns node + up to 40 `curl`
  health probes; `youtube_ingest_captions` shells out to `curl` per caption

**Fix (P0, one shared helper).** Add `fn cmd(app, bin) -> Command` that applies
`.creation_flags(0x0800_0000)` (`CREATE_NO_WINDOW`) on `#[cfg(windows)]` and
route every `Command::new` in `main.rs` through it. One-line-per-callsite,
kills the symptom class, not the symptom.

### 2. "Tự tắt app / crash khi chuyển trang" — silent death on Windows, no crash report

**Root cause candidates (ranked).**

1. **Silent panic exit.** `windows_subsystem = "windows"` + no
   `panic::set_hook`: any Rust panic aborts the process with zero UI, which
   reads exactly as "nó tự tắt app k à". There is currently no log file to
   tell us which panic (if any) fired.
2. **Windows resource layout is broken** (see Finding 4): on the portable zip
   `node`/`ffmpeg`/`ffprobe` do not resolve, so `list_analysis_jobs`,
   `start_analysis` and the RAG sidecar all fail at spawn on a machine without
   Node/ffmpeg on PATH. A failing invoke is caught in JS, but the churn
   (5 s node-spawn timer + failed invokes) is exactly the "switch two pages →
   app dies" pattern if any of it panics on the error path.
3. WebView2 renderer crash from GPU compositing (analogous to the WebKitGTK
   DMABUF issue already worked around in `main()` for Linux).

**Fix (P0).**
- `std::panic::set_hook`: append panic + backtrace to
  `~/.cache/local-first-ai-video-studio/panic.log` and show a Tauri dialog
  ("The app hit an internal error — send us this log"). Turns silent deaths
  into actionable reports.
- Fix the resource layout bugs (Finding 4) so the portable actually runs its
  helpers.
- Optional (P1): on Windows set
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--disable-gpu-compositing` behind a
  `runtime.env` toggle for machines that reproduce the crash — mirrors the
  existing `WEBKIT_DISABLE_DMABUF_RENDERER` workaround.

### 3. "Chuyển page delay >2 s" — heavy side effects run on every page mount

**Root cause.** The UI is already client-side rendered (SvelteKit +
adapter-static, no SSR in the bundle). The >2 s comes from work that `onMount`
does synchronously per navigation on the customer machine:

- `runtime_status` (called from layout + Studio + Settings) copies the bundled
  Whisper `.pt` model into `~/.cache/whisper` **synchronously on first call**
  (`ensure_bundled_whisper_model`, ~75–147 MB file copy → seconds on an HDD)
- Studio mount spawns a node process for `list_analysis_jobs` and starts the 5
  s node-spawn poll
- YouTube Studio mount spawns the RAG sidecar node + curl-probes it

**Fix (P1).**
- Move the model copy off the `runtime_status` path: run it in a background
  thread on first run (same pattern as `bootstrap_whisper`) and report
  "preparing engine" until done.
- Cache shared status in module-level state (the `studioState` pattern already
  exists): fetch `runtime_status`/`storage_status`/`license_status` once in
  the layout, let pages read the cache.
- Keep the recent-jobs poll only while a job is active (or raise it to 30 s);
  store the last result in memory so page mounts paint instantly.

**Instant-switch add (P2, optional).** Keep visited pages mounted and toggle
`display:none`, so switching never re-runs `onMount` at all — the fastest
possible switch on top of the CSR shell.

### 4. "Chuyển xong mất logo / app broken on Windows" — helper binaries don't resolve in the portable

**Root cause.** `bundled_bin(app, "node/bin/node", "node")` and the
`ffmpeg`/`ffprobe` lookups don't match what `fetch-bundled-assets.sh` produces
on Windows:

- Windows Node zips extract `node.exe` at the archive root → resources contain
  `node/node.exe`, code looks for `node/bin/node` → falls back to PATH `node`
  (absent on most customer machines) → all analysis/jobs/RAG helpers fail
- ffmpeg/ffprobe are copied to extensionless `resources/ffmpeg`; Windows
  `CreateProcess` appends `.exe` when resolving, so the lookup fails →
  `probe_video`/`render_video` error out

On Linux/macOS the layout matches, which is why this never showed up locally —
only the customer's Windows build hits it. The visible result is a Studio page
whose jobs list, runtime dots and preview are all dead after navigation — which
the customer describes as the page "losing" its UI (logo included).

**Fix (P0).**
- `fetch-bundled-assets.sh`: on Windows keep the real names —
  `resources/node/bin/node.exe`, `resources/ffmpeg.exe`,
  `resources/ffprobe.exe` (and update `release.yml`'s portable assembly
  accordingly).
- `bundled_bin` (or a thin `helper_bin` wrapper): on `#[cfg(windows)]`, try
  `rel.exe` in addition to `rel`.
- Add one runnable check: a Rust unit test that the staged resource dir
  (as packaged) contains each helper under the name `bundled_bin` will probe —
  fails CI instead of shipping a broken portable again.

### 5. "Custom title bar" — requested, not present

Currently the window uses the OS title bar (`tauri.conf.json` has no
`decorations`/`titleBarStyle`; the customer's crop shows the default chrome).

**Fix (P1).** `"decorations": false` + a slim in-app title bar component:
`data-tauri-drag-region` on the bar (double-click-to-maximize works via the
drag region), min/max/close buttons via
`@tauri-apps/api/window` (`minimize`/`toggleMaximize`/`close`), app title in
the middle. Reuse the existing `.rail` visual language; zero new dependencies.

### 6. Small defects found while triaging (fix alongside)

- `stopBrowserDrop()` in `+page.svelte` adds `dragover`/`drop` listeners to
  `window` on every Studio mount and never removes them — leaks listeners with
  every page switch. Move to the layout or remove in the `onMount` cleanup.
- The `watch()` analysis poll loop keeps running after the user leaves the
  Studio page (no unmount cancel flag) — add an `aborted` flag cleared in the
  `onMount` cleanup.
- Portable `README.txt` should state the Windows prerequisites (no Node needed
  after fix 4) once the helpers resolve from bundled resources.

## Proposed order

1. **P0** — CREATE_NO_WINDOW helper (Finding 1); panic hook + log (2);
   Windows resource paths + CI check (4).
2. **P1** — background model copy + cached status + poll policy (3); custom
   title bar (5).
3. **P2** — keep-alive page shells for instant switches (3); WebView2 GPU
   fallback toggle (2); listener/loop cleanup (6).

After P0+P1, rebuild the Windows portable + MSI, re-upload to Drive, and ask
the customer to retry the two-page switch with the new build — the panic log
(if anything still dies) will name the exact failure instead of a silent exit.

## Implementation status (2026-09-30, same day)

All three priorities are implemented, built, and verified on the real desktop
app (Linux release build driven over AT-SPI + screenshots):

- **P0** — `tool()` applies `CREATE_NO_WINDOW` to every helper spawn on
  Windows (kills the console-window flashing); `panic::set_hook` writes
  `panic.log` and the next launch shows it in-app via the new `last_panic`
  command (silent exits become readable reports); Windows resource layout
  fixed (`node/bin/node.exe`, `ffmpeg.exe`, `ffprobe.exe`) with a CI verify
  step in `release.yml` for both the Windows and Linux jobs before packaging.
- **P1** — Whisper model copy moved off `runtime_status` into a background
  thread; engine/storage/license status cached in
  `src/lib/runtime-status.svelte.ts` (fetched once by the layout, reused by
  Studio + Settings, polled a few times while the first-run copy finishes);
  custom title bar (`decorations: false` + in-app bar with
  `data-tauri-drag-region` and min/max/close controls).
- **P2** — keep-alive pages (layout-mounted, `display:none` toggled) so page
  switching never re-runs `onMount` (measured 87–126 ms warm switches, 20
  rapid switches with no crash); browser drop-block listeners moved to the
  layout (the old per-mount listeners leaked on every visit); Studio's jobs
  poll only runs while the page is visible; `WEBVIEW2_GPU_DISABLE=1` in
  `runtime.env` opts into `--disable-gpu-compositing` on Windows machines
  that reproduce WebView2 renderer crashes.

Root cause found while verifying (worth remembering): the first keep-alive
draft wrote `visited = { ...visited, [page]: true }` inside a `$effect` —
the effect read and wrote the same state object, looping forever and starving
Svelte's update queue, which made every later state write (including invoke
results) silently stop rendering. The fix writes only when the flag is missing
(`if (!visited[page]) visited[page] = true`), so the effect settles.
