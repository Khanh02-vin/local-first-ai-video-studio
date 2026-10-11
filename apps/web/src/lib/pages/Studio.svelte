<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { Highlight } from "../../../../../packages/contracts/highlight.ts";
  import type { Collection } from "../../../../../packages/contracts/collection.ts";
  import { studioState } from "../studio-state.svelte.ts";
  import { refreshRuntime, runtimeStore } from "../runtime-status.svelte.ts";
  import Icon from "../icons.svelte";

  // Keep-alive mounted by +layout.svelte: `active` tells this page it is the
  // visible one, so background polls only run while the user can see them.
  let { active = true }: { active?: boolean } = $props();

  type Meta = { name: string; duration: number; width: number; height: number };
  type JobRow = { id: string; input?: string; status: string; phase: string; progress: number; rangeStart?: number; rangeEnd?: number; chunkCompleted?: number; chunkTotal?: number; error?: string };

  let inputPath = $state("");
  let outputPath = $state("");
  let analyzeStart = $state(0);
  let analyzeEnd = $state(600);
  let aspectRatio = $state("9:16");
  let message = $state("Drop a video to begin.");
  let busy = $state(false);
  let jobId = $state("");
  let phase = $state("");
  let progress = $state(0);
  let elapsedSeconds = $state(0);
  let timer: ReturnType<typeof setInterval> | undefined;
  let pollRun = 0;
  let watchStarted = false;
  let timerStartedAt = 0;
  let highlights = $state<Highlight[]>([]);
  let collections = $state<Collection[]>([]);
  let selectedCollection = $state("");
  let selectedId = $state("");
  let youtubeUrl = $state("");
  let youtubeVideoId = $state("");
  // Loopback base for the preview iframe: the packaged webview's tauri:// origin
  // carries no Referer, which YouTube rejects (error 153). Empty in the browser,
  // where the plain embed already works.
  let previewBase = $state("");
  let selectedCandidate = $derived(highlights.find((item) => item.id === selectedId) ?? null);
  // Highlight ids belonging to the selected collection, so the candidate list
  // can mark which clips the grouping picked. Empty when nothing is selected.
  let collectionMembers = $derived(new Set(
    collections.find((collection) => collection.id === selectedCollection)?.items
      .map((item) => (item.reference.kind === "highlight" ? item.reference.highlightId : "")) ?? []
  ));
  function pickCollection(collection: Collection) {
    selectedCollection = collection.id;
    const first = collection.items[0]?.reference;
    if (first?.kind === "highlight" && highlights.some((item) => item.id === first.highlightId)) selectedId = first.highlightId;
  }
  let modelBusy = $state(false);
  let meta = $state<Meta | null>(null);
  let dragOver = $state(false);
  let resumableJobs = $state<JobRow[]>([]);
  let currentStep = $state<1 | 2 | 3 | 4>(1);

  async function choose() {
    if (!isTauri()) { message = "File dialogs only work in the desktop app — open the installed app to load a video."; return; }
    const path = await open({ multiple: false, directory: false, filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm"] }] });
    if (typeof path === "string") await loadVideo(path);
  }

  async function loadVideo(path: string) {
    inputPath = path; highlights = []; collections = []; selectedCollection = ""; selectedId = ""; jobId = ""; busy = false; meta = null; youtubeVideoId = "";
    await reconnectJobs(false);
    try {
      const probe = JSON.parse(await invoke<string>("probe_video", { path }));
      const stream = (probe.streams ?? []).find((s: { codec_type?: string }) => s.codec_type === "video");
      meta = { name: path.split(/[\\/]/).pop() ?? path, duration: Number(probe.format?.duration ?? 0), width: Number(stream?.width ?? 0), height: Number(stream?.height ?? 0) };
      studioState.sourcePath = path; studioState.sourceName = path.split(/[\\/]/).pop() ?? path; studioState.sourceDuration = Number(probe.format?.duration ?? 0);
    } catch { meta = { name: path.split(/[\\/]/).pop() ?? path, duration: 0, width: 0, height: 0 }; studioState.sourcePath = path; studioState.sourceName = path.split(/[\\/]/).pop() ?? path; }
    message = "Video loaded. Set the range and start analysis.";
  }

  function onDrop(event: DragEvent) {
    dragOver = false;
    // Native Tauri drop events carry the real filesystem path; this handler
    // exists for browsers, where the File API cannot expose an absolute path.
    message = "Use the Browse button, or drop the file onto the window (desktop app).";
  }

  async function setModel(next: string) {
    if (!isTauri()) { message = "Model changes apply in the desktop app only."; return; }
    if (modelBusy) return;
    modelBusy = true;
    try { const chosen = await invoke<string>("set_runtime_model", { model: next }); await refreshRuntime(true); message = `Whisper model: ${chosen}.`; }
    catch (error) { message = `Model change failed: ${String(error)}`; }
    finally { modelBusy = false; }
  }

  function stopTimer() { if (timer) clearInterval(timer); timer = undefined; }
  function resetTimer() { stopTimer(); timerStartedAt = 0; elapsedSeconds = 0; }
  function elapsedLabel() { return `${Math.floor(elapsedSeconds / 60)}m ${String(elapsedSeconds % 60).padStart(2, "0")}s`; }
  $effect(() => {
    const visible = active;
    const running = busy;
    if (!visible || !running) { stopTimer(); return; }
    if (!timerStartedAt) timerStartedAt = Date.now() - untrack(() => elapsedSeconds) * 1000;
    timer = setInterval(() => elapsedSeconds = Math.floor((Date.now() - timerStartedAt) / 1000), 1000);
    return stopTimer;
  });

  async function clearCache(id: string) {
    try { const bytes = await invoke<number>("clear_analysis_cache", { id }); resumableJobs = resumableJobs.filter((job) => job.id !== id); message = `Cleared ${(bytes / 1024 / 1024).toFixed(1)} MB for this setup.`; }
    catch (error) { message = `Cache clear failed: ${String(error)}`; }
  }
  async function retryJob(id: string) { try { await invoke("retry_analysis", { id }); await watch(id, true); } catch (error) { message = `Retry failed: ${String(error)}`; } }

  async function reconnectJobs(attach = true) {
    try { resumableJobs = await invoke<JobRow[]>("list_analysis_jobs"); } catch { resumableJobs = []; }
    const active = inputPath ? resumableJobs.find((job) => job.input === inputPath && (job.status === "queued" || job.status === "running")) : undefined;
    if (attach && active && !busy && !jobId) void watch(active.id);
  }

  function stepState() {
    const done = [false, false, false, false, false];
    const current = 0;
    if (!jobId) return { done, current: 0, active: false };
    if (phase === "transcribe" || phase.includes("chunk") || phase.includes("queued")) { done[0] = true; return { done, current: 1, active: busy }; }
    if (phase === "rank") { done[0] = true; done[1] = true; return { done, current: 2, active: busy }; }
    if (phase === "completed") { done[0] = true; done[1] = true; done[2] = true; return { done, current: 3, active: false }; }
    return { done, current, active: false };
  }

  async function watch(id: string, resuming = true) {
    if (!active) { jobId = id; busy = true; return; }
    if (watchStarted) return;
    watchStarted = true;
    const run = ++pollRun;
    jobId = id; busy = true; if (!resuming) { phase = "queued"; progress = 0; resetTimer(); timerStartedAt = Date.now(); } currentStep = 3; message = resuming ? "Resuming analysis…" : "Analyzing…";
    try {
      while (busy && jobId === id && pollRun === run && active) {
        const job = await invoke<{ status: string; phase: string; progress: number; chunkCompleted?: number; chunkTotal?: number; currentChunk?: number; result?: string; error?: string }>("analysis_status", { id });
        if (!active || pollRun !== run || jobId !== id) break;
        phase = job.chunkTotal ? `${job.phase} · chunk ${Math.min((job.currentChunk ?? job.chunkCompleted ?? 0) + 1, job.chunkTotal)}/${job.chunkTotal}` : job.phase;
        progress = job.progress;
        if (job.status === "completed") {
          const result = JSON.parse(job.result ?? "{}"); highlights = result.highlights ?? []; selectedId = highlights[0]?.id ?? "";
          collections = result.collections ?? []; selectedCollection = collections[0]?.id ?? "";
          if (result.videoId) youtubeVideoId = result.videoId;
          studioState.highlights = highlights; studioState.selectedId = selectedId;
          message = highlights.length ? `${highlights.length} highlight candidates ready.` : "No highlights found.";
          currentStep = 4; busy = false; resetTimer(); break;
        }
        if (job.status === "failed") { message = `Analyze failed: ${job.error ?? "unknown error"}`; busy = false; resetTimer(); break; }
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
    } catch (error) { if (pollRun === run) { message = `Resume failed: ${String(error)}`; busy = false; resetTimer(); } }
    finally { if (pollRun === run) { watchStarted = false; await reconnectJobs(false); } }
  }
  $effect(() => {
    const visible = active;
    const id = jobId;
    if (visible && busy && id && !watchStarted) void watch(id, true);
  });

  async function analyze() {
    if (!isTauri()) { message = "Analysis runs in the desktop app only — this browser preview cannot transcribe."; return; }
    if (!inputPath || !runtimeStore.ready) return;
    const existing = resumableJobs.find((job) => job.input === inputPath && (job.status === "queued" || job.status === "running") && Math.abs((job.rangeStart ?? 0) - analyzeStart) < 0.01 && Math.abs((job.rangeEnd ?? analyzeEnd) - analyzeEnd) < 0.01);
    if (existing && confirm("Continue existing analysis? Cancel = clear this setup and start fresh.")) { await watch(existing.id, true); return; }
    if (existing) await clearCache(existing.id);
    highlights = []; collections = []; selectedCollection = ""; selectedId = ""; message = "Analyzing…";
    try {
      const id = await invoke<string>("start_analysis", { path: inputPath, start: analyzeStart, end: analyzeEnd });
      await watch(id, false);
    } catch (error) { message = `Analyze failed: ${String(error)}`; busy = false; resetTimer(); }
  }

  /// YouTube: one metadata call → transcript-based analysis (no Whisper, no download).
  async function analyzeYoutube() {
    if (!isTauri()) { message = "YouTube analysis runs in the desktop app only."; return; }
    const url = youtubeUrl.trim();
    if (!url || busy) return;
    highlights = []; collections = []; selectedCollection = ""; selectedId = ""; jobId = ""; youtubeVideoId = ""; meta = null;
    busy = true; message = "Fetching YouTube transcript…"; resetTimer(); timerStartedAt = Date.now();
    try {
      const info = await invoke<{ id: string; duration: number; title: string }>("start_youtube_analysis", { url });
      inputPath = url;
      meta = { name: info.title, duration: info.duration, width: 0, height: 0 };
      studioState.sourcePath = url; studioState.sourceName = info.title; studioState.sourceDuration = info.duration;
      youtubeUrl = "";
      await watch(info.id, false);
    } catch (error) {
      message = `YouTube analyze failed: ${String(error)}`;
      busy = false; resetTimer();
    }
  }

  async function render() {
    if (!isTauri()) { message = "Rendering runs in the desktop app only."; return; }
    const candidate = highlights.find((item) => item.id === selectedId);
    if (!inputPath || !candidate) return;
    outputPath = await save({ defaultPath: "short.mp4", filters: [{ name: "MP4", extensions: ["mp4"] }] }) ?? "";
    if (!outputPath) return;
    busy = true;
    try {
      message = `Rendered: ${await invoke<string>("render_video", { input: inputPath, output: outputPath, start: candidate.start, duration: candidate.end - candidate.start, aspectRatio })}`;
    } catch (error) { message = `Render failed: ${String(error)}`; }
    finally { busy = false; }
  }

  async function abort() {
    if (!jobId) { busy = false; resetTimer(); return; }
    try { await invoke("stop_analysis", { id: jobId }); message = "Analysis stopped."; }
    catch (error) { message = `Abort failed: ${String(error)}`; }
    finally { busy = false; resetTimer(); await reconnectJobs(false); }
  }

  function clock(seconds: number) { return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`; }
  onMount(() => {
    void refreshRuntime(); void reconnectJobs();
    if (isTauri()) invoke<string>("preview_origin").then((base) => previewBase = base).catch(() => {});
    const unlisten = getCurrentWindow().onDragDropEvent((event) => {
      const payload = event.payload;
      dragOver = payload.type === "over" || payload.type === "enter";
      if (payload.type === "drop") {
        dragOver = false;
        const path = payload.paths?.[0];
        if (path) void loadVideo(path);
      }
    });
    // Browser-preview drop blocking lives in +layout.svelte (once per app
    // session — the old per-mount window listeners leaked on every visit).
    return () => { stopTimer(); unlisten.then((fn) => fn()); };
  });
  // Refresh job lists only while visible; keep-alive pages remain mounted.
  $effect(() => {
    if (!active) return;
    const refresh = setInterval(() => { if (!busy) void reconnectJobs(); }, 5000);
    return () => clearInterval(refresh);
  });
</script>

<main class="bench bench-duo studio-workspace">
  <aside class="bench-left">
    <div class="panel"><span class="eyebrow">Local-first</span><h1>Studio</h1><p class="note">Turn long footage into a clean short-form cut without leaving this machine.</p></div>
    <div class="panel" style="margin-top:auto;"><span class="eyebrow">Engine</span><div class="spread" style="align-items:center;"><span style="display:flex;align-items:center;gap:.5rem;font-size:.82rem;color:var(--ink-2);"><span class="rail-dot" class:ok={runtimeStore.ready}></span> {runtimeStore.ready ? "100% on-device" : "Checking runtime"}</span><a class="btn btn-sm btn-quiet" href="/settings">Details</a></div><p class="note">{runtimeStore.runtimeMessage}</p></div>
  </aside>

  <section class="bench-center">
    <div class="stage"><div class="stage-head"><div class="stage-title"><span class="badge volt">source studio</span><span class="stage-name">{meta?.name ?? "Drop a source to begin"}</span></div><span class="badge">{currentStep === 4 ? "candidates ready" : `step ${currentStep} / 4`}</span></div><div class="stage-body">
    <ol class="steps" aria-label="Workflow steps">
      <li class:active={currentStep === 1} class:done={currentStep > 1 || !!inputPath}><button type="button" onclick={() => currentStep = 1}><span class="step-no">1</span>Drop a video</button></li>
      <li class:active={currentStep === 2}><button type="button" onclick={() => currentStep = 2}><span class="step-no">2</span>Config &amp; Ready</button></li>
      <li class:active={currentStep === 3} class:done={currentStep > 3 || busy}><button type="button" onclick={() => currentStep = 3}><span class="step-no">3</span>Processing</button></li>
      <li class:active={currentStep === 4} class:done={highlights.length > 0}><button type="button" onclick={() => currentStep = 4}><span class="step-no">4</span>Shorts Showcase</button></li>
    </ol>

    {#if currentStep === 1}
    <div class="card dropzone" class:over={dragOver} role="region" aria-label="Video upload drop zone" ondragover={(e) => { e.preventDefault(); dragOver = true; }} ondragleave={() => dragOver = false} ondrop={(e) => { e.preventDefault(); onDrop(e); }}>
      {#if !inputPath}
        <div class="drop-inner">
          <div class="drop-icon"><Icon name="film" size={28} /></div>
          <h2>Drop a video to start</h2>
          <p>Drag & drop here, or</p>
          <button class="browse" onclick={choose} disabled={busy}><Icon name="folder" size={15} /> Browse files</button>
          <p class="hint">MP4 · MOV · MKV — processed 100% on your device</p>
          <div style="display:flex;gap:.5rem;margin-top:1rem;flex-wrap:wrap;justify-content:center;">
            <input type="url" placeholder="…or paste a YouTube URL" aria-label="YouTube URL" bind:value={youtubeUrl} disabled={busy} style="flex:1;min-width:220px;" />
            <button class="browse" onclick={analyzeYoutube} disabled={busy || !youtubeUrl.trim()}>{#if busy}<span class="spinner"></span>{:else}Analyze link{/if}</button>
          </div>
          <p class="hint">YouTube: reads the transcript in seconds — no download, no Whisper.</p>
        </div>
      {:else if meta}
        <div class="preview-card">
          <div class="thumb"><Icon name="film" size={22} /></div>
          <div class="preview-info">
            <strong>{meta.name}</strong>
            <span>{clock(meta.duration)} · {meta.width && meta.height ? `${meta.width}×${meta.height}` : ""}</span>
          </div>
          <button class="cta small" onclick={() => currentStep = 2}>Continue to config →</button>
          <button class="ghost" onclick={() => { inputPath = ""; meta = null; youtubeVideoId = ""; message = "Drop a video to begin."; }}>Remove</button>
        </div>
      {/if}
    </div>
    {/if}

    {#if currentStep === 2}
    <div class="card">
      <div class="section-title">AI model</div>
      <div class="model-row">
        {#each ["tiny", "base", "small"] as m}<button class="chip" class:sel={runtimeStore.model === m} disabled={busy || modelBusy} onclick={() => setModel(m)}>{m}</button>{/each}
      </div>
      <p class="hint">tiny = fastest on CPU · base/small need a Pro license · GPU (CUDA) speeds all up</p>
      <div class="spec-line"><span class="dev">{runtimeStore.device === "cuda" ? "GPU · CUDA" : "CPU · 1 thread"}</span><span>{runtimeStore.model} model</span></div>
      <p class="hint">{runtimeStore.storageMessage}</p>
    </div>

    <div class="card">
      <div class="section-title">Range (seconds{meta ? ` · full ${clock(meta.duration)}` : ""})</div>
      <div class="range-row">
        <label>Start <input type="number" min="0" bind:value={analyzeStart} /></label>
        <label>End <input type="number" min="1" bind:value={analyzeEnd} /></label>
      </div>
      <div class="section-title">Output format</div>
      <div class="model-row">
        {#each ["9:16", "1:1", "16:9"] as ratio}<button class="chip" class:sel={aspectRatio === ratio} disabled={busy} onclick={() => aspectRatio = ratio}>{ratio}</button>{/each}
      </div>
      <p class="hint">Aspect ratio for the final export (FFmpeg crop + scale).</p>
      <button class="cta" disabled={!inputPath || busy || !runtimeStore.ready || analyzeEnd <= analyzeStart} onclick={analyze}>
        {#if busy}<span class="spinner"></span>{/if}
        {busy ? "Processing…" : "Start analysis"}
      </button>
      {#if busy}<button class="ghost wide" onclick={abort}>Abort</button>{/if}
      <p class="hint" aria-live="polite">{runtimeStore.licenseMessage}</p>
    </div>
    {/if}

    {#if currentStep === 3}
    <div class="card">
      <div class="section-title">Pipeline</div>
      <ol class="stepper">
        <li class:done={stepState().done[0]} class:active={stepState().current === 0 && busy}><span class="step-no">1</span>Upload</li>
        <li class:done={stepState().done[1]} class:active={stepState().current === 1 && busy}><span class="step-no">2</span>Transcribe</li>
        <li class:done={stepState().done[2]} class:active={stepState().current === 2 && busy}><span class="step-no">3</span>Highlights</li>
        <li class:done={highlights.length > 0}><span class="step-no">4</span>Edit</li>
        <li><span class="step-no">5</span>Export</li>
      </ol>
      {#if busy}<div class="progress-track"><div class="progress-fill" style:width="{Math.round(progress * 100)}%"></div></div><p class="hint">{phase} · {Math.round(progress * 100)}% · elapsed {elapsedLabel()}</p>
      <button class="ghost wide" onclick={abort}>Abort analysis</button>{:else}<p class="hint">Waiting for a job…</p>{/if}
    </div>
    {/if}

    {#if currentStep === 4}
    {#if highlights.length}
      <div class="card">
        <div class="section-title">Shorts Showcase</div>
        {#if collections.length}
          <div class="stack">
            {#each collections as collection}
              <button class="cand" class:on={collection.id === selectedCollection} aria-pressed={collection.id === selectedCollection} onclick={() => pickCollection(collection)}>
                <span class="cand-rank">{collection.items.length}</span>
                <span>
                  <span class="cand-title">{collection.title}</span>
                  {#if collection.topic}<span class="cand-sub">{collection.topic}</span>{/if}
                </span>
                <span class="cand-score">{collection.state}</span>
              </button>
            {/each}
          </div>
        {/if}
        <div class="candidates">
          {#each highlights as candidate}
            <button class:selected={candidate.id === selectedId} aria-pressed={candidate.id === selectedId} onclick={() => selectedId = candidate.id}>
              <strong>{candidate.title}</strong>
              <span>{clock(candidate.start)}–{clock(candidate.end)} · score {candidate.score}{#if collectionMembers.has(candidate.id)} · in collection{/if}</span>
              {#if candidate.hook}<small>{candidate.hook}</small>{/if}
            </button>
          {/each}
        </div>
        <div class="spec-line"><span>Export {aspectRatio}</span><span>{highlights.length} candidates{collections.length ? ` · ${collections.length} collections` : ""}</span></div>
        <button class="cta secondary" disabled={busy || !selectedId} onclick={render}>
          {#if busy}<span class="spinner"></span>{/if}
          {busy ? "Rendering…" : `Render selected candidate (${aspectRatio})`}
        </button>
        {#if outputPath}<p class="hint">Saved: {outputPath}</p>{/if}
      </div>
      {#if inputPath.startsWith("http") && youtubeVideoId && selectedCandidate}
        <div class="card">
          <div class="section-title">Preview (YouTube)</div>
          <iframe
            title="YouTube clip preview"
            style="width:100%;aspect-ratio:16/9;border:0;border-radius:12px;background:#000;"
            src={previewBase
              ? `${previewBase}/preview?id=${youtubeVideoId}&start=${Math.floor(selectedCandidate.start)}&end=${Math.ceil(selectedCandidate.end)}`
              : `https://www.youtube.com/embed/${youtubeVideoId}?start=${Math.floor(selectedCandidate.start)}&end=${Math.ceil(selectedCandidate.end)}&rel=0`}
            allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
            allowfullscreen
          ></iframe>
          <p class="hint">Seek preview streams from YouTube — nothing is rendered until you export.</p>
        </div>
      {/if}
    {:else}
      <div class="card"><p class="hint">No highlights yet. Run an analysis first.</p><button class="ghost wide" onclick={() => currentStep = 2}>← Back to config</button></div>
    {/if}
    {/if}

    <p class="message" aria-live="polite">{message}</p>

    <div class="panel">
      <div class="panel-head"><span class="panel-title"><span class="eyebrow">Recent jobs</span></span><span class="badge">{resumableJobs.length}</span></div>
      {#if resumableJobs.length}{#each resumableJobs as job}<div class="row"><div class="row-body"><span class="row-title">{job.id.slice(-8)}</span><span class="row-sub">{job.status} · {job.phase}</span></div><div class="row-actions">{#if job.status === "queued" || job.status === "running"}<button class="btn btn-sm" disabled={busy} onclick={() => watch(job.id, true)}>Continue</button>{/if}{#if job.status === "failed"}<button class="btn btn-sm" disabled={busy} onclick={() => retryJob(job.id)}>Retry</button>{/if}</div></div>{/each}{:else}<p class="note">No projects yet — analyze a video to see results here.</p>{/if}
    </div>
      </div>
    <div class="stage-foot">
      <span>Device: {runtimeStore.device === "cuda" ? "GPU (CUDA)" : "CPU"} · Model: {runtimeStore.model} · License: {runtimeStore.licenseMessage}</span>
      <span title="Safe cleanup only: inspect caches and old renders before removing anything.">Storage: {runtimeStore.storageMessage}</span>
    </div>
  </section>
</main>