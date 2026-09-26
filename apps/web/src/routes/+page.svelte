<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { Highlight } from "../../../../packages/contracts/highlight.ts";
  import { studioState } from "../lib/studio-state.svelte.ts";

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
  let highlights = $state<Highlight[]>([]);
  let selectedId = $state("");
  let runtimeReady = $state(false);
  let runtimeMessage = $state("Checking local runtime…");
  let storageMessage = $state("Checking storage…");
  let licenseMessage = $state("");
  let device = $state("cpu");
  let model = $state("tiny");
  let modelBusy = $state(false);
  let meta = $state<Meta | null>(null);
  let dragOver = $state(false);
  let resumableJobs = $state<JobRow[]>([]);
  let activeTab: "recent" | "specs" = $state("recent");
  let storageHelp = $state(false);
  let currentStep = $state<1 | 2 | 3 | 4>(1);

  async function choose() {
    if (!isTauri()) { message = "File dialogs only work in the desktop app — open the installed app to load a video."; return; }
    const path = await open({ multiple: false, directory: false, filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm"] }] });
    if (typeof path === "string") await loadVideo(path);
  }

  async function loadVideo(path: string) {
    inputPath = path; highlights = []; selectedId = ""; jobId = ""; busy = false; meta = null;
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

  async function checkRuntime() {
    try {
      const runtime = await invoke<{ ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; modelReady: boolean; device?: string; model?: string }>("runtime_status");
      const storage = await invoke<{ home: { ready: boolean; freeBytes?: number; requiredBytes: number }; temp: { ready: boolean; freeBytes?: number } }>("storage_status");
      const license = await invoke<{ licensed: boolean; tier?: string; licensee?: string | null; error?: string; configured?: boolean }>("license_status");
      const storageReady = storage.home.ready && storage.temp.ready;
      runtimeReady = runtime.ffmpeg && runtime.ffprobe && runtime.node && runtime.whisper && runtime.modelReady && storageReady;
      device = runtime.device ?? "cpu";
      model = runtime.model ?? "tiny";
      runtimeMessage = runtimeReady ? `Engine ready · ${device === "cuda" ? "GPU (CUDA)" : "CPU"}` : "Whisper/model, media runtime, or storage is missing.";
      licenseMessage = license.licensed ? `License ${license.tier}${license.licensee ? ` · ${license.licensee}` : ""}` : (license.error ?? (license.configured === false ? "Unlicensed (free tier)" : "Unlicensed"));
      storageMessage = storageReady ? `${formatBytes(storage.home.freeBytes ?? 0)} free` : `Need ${formatBytes(storage.home.requiredBytes)}`;
    } catch (error) { runtimeMessage = `Runtime check failed: ${String(error)}`; }
  }

  async function setModel(next: string) {
    if (modelBusy) return;
    modelBusy = true;
    try { model = await invoke<string>("set_runtime_model", { model: next }); message = `Whisper model: ${model}.`; }
    catch (error) { message = `Model change failed: ${String(error)}`; }
    finally { modelBusy = false; }
  }

  function stopTimer() { if (timer) clearInterval(timer); timer = undefined; }
  function startTimer() { stopTimer(); const started = Date.now(); elapsedSeconds = 0; timer = setInterval(() => elapsedSeconds = Math.floor((Date.now() - started) / 1000), 250); }
  function elapsedLabel() { return `${Math.floor(elapsedSeconds / 60)}m ${String(elapsedSeconds % 60).padStart(2, "0")}s`; }

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
    jobId = id; busy = true; phase = "queued"; progress = 0; currentStep = 3; message = resuming ? "Resuming analysis…" : "Analyzing…"; startTimer();
    try {
      while (busy && jobId === id) {
        const job = await invoke<{ status: string; phase: string; progress: number; chunkCompleted?: number; chunkTotal?: number; currentChunk?: number; result?: string; error?: string }>("analysis_status", { id });
        phase = job.chunkTotal ? `${job.phase} · chunk ${Math.min((job.currentChunk ?? job.chunkCompleted ?? 0) + 1, job.chunkTotal)}/${job.chunkTotal}` : job.phase;
        progress = job.progress;
        if (job.status === "completed") {
          const result = JSON.parse(job.result ?? "{}"); highlights = result.highlights ?? []; selectedId = highlights[0]?.id ?? "";
          studioState.highlights = highlights; studioState.selectedId = selectedId;
          message = highlights.length ? `${highlights.length} highlight candidates ready.` : "No highlights found.";
          currentStep = 4; busy = false; stopTimer(); break;
        }
        if (job.status === "failed") { message = `Analyze failed: ${job.error ?? "unknown error"}`; busy = false; stopTimer(); break; }
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
    } catch (error) { message = `Resume failed: ${String(error)}`; busy = false; stopTimer(); }
    finally { await reconnectJobs(false); }
  }

  async function analyze() {
    if (!inputPath || !runtimeReady) return;
    const existing = resumableJobs.find((job) => job.input === inputPath && (job.status === "queued" || job.status === "running") && Math.abs((job.rangeStart ?? 0) - analyzeStart) < 0.01 && Math.abs((job.rangeEnd ?? analyzeEnd) - analyzeEnd) < 0.01);
    if (existing && confirm("Continue existing analysis? Cancel = clear this setup and start fresh.")) { await watch(existing.id, true); return; }
    if (existing) await clearCache(existing.id);
    highlights = []; selectedId = ""; message = "Analyzing…";
    try {
      const id = await invoke<string>("start_analysis", { path: inputPath, start: analyzeStart, end: analyzeEnd });
      await watch(id, false);
    } catch (error) { message = `Analyze failed: ${String(error)}`; busy = false; stopTimer(); }
  }

  async function render() {
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
    if (!jobId) { busy = false; stopTimer(); return; }
    try { await invoke("stop_analysis", { id: jobId }); message = "Analysis stopped."; }
    catch (error) { message = `Abort failed: ${String(error)}`; }
    finally { busy = false; stopTimer(); await reconnectJobs(false); }
  }

  function formatBytes(bytes: number) { return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`; }
  function clock(seconds: number) { return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`; }
  function stopBrowserDrop() { window.addEventListener("dragover", (e) => e.preventDefault()); window.addEventListener("drop", (e) => e.preventDefault()); }
  onMount(() => {
    stopBrowserDrop();
    checkRuntime(); void reconnectJobs();
    const unlisten = getCurrentWindow().onDragDropEvent((event) => {
      const payload = event.payload;
      dragOver = payload.type === "over" || payload.type === "enter";
      if (payload.type === "drop") {
        dragOver = false;
        const path = payload.paths?.[0];
        if (path) void loadVideo(path);
      }
    });
    const refresh = setInterval(() => { if (!busy) void reconnectJobs(); }, 5000);
    return () => { stopTimer(); clearInterval(refresh); unlisten.then((fn) => fn()); };
  });
</script>

<main class="bench studio-workspace">
  <aside class="bench-left">
    <div class="panel"><span class="eyebrow">Local-first</span><h1>Studio</h1><p class="note">Turn long footage into a clean short-form cut without leaving this machine.</p></div>
    <div class="panel"><span class="panel-title">Workflow</span><ol class="steps"><li class:done={!!inputPath}><span class="step-num">1</span>Source</li><li class:done={currentStep > 1}><span class="step-num">2</span>Configure</li><li class:done={currentStep > 3}><span class="step-num">3</span>Analyze</li><li class:done={highlights.length > 0}><span class="step-num">4</span>Edit</li></ol></div>
    <div class="panel"><span class="eyebrow">Engine</span><div class="message"><span class="rail-dot" class:ok={runtimeReady}></span> {runtimeReady ? "Ready" : "Checking runtime"}</div><p class="note">{runtimeMessage}</p></div>
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
          <div class="drop-icon">🎬</div>
          <h2>Drop a video to start</h2>
          <p>Drag & drop here, or</p>
          <button class="browse" onclick={choose} disabled={busy}>Browse files</button>
          <p class="hint">MP4 · MOV · MKV — processed 100% on your device</p>
        </div>
      {:else if meta}
        <div class="preview-card">
          <div class="thumb">🎞️</div>
          <div class="preview-info">
            <strong>{meta.name}</strong>
            <span>{clock(meta.duration)} · {meta.width && meta.height ? `${meta.width}×${meta.height}` : ""}</span>
          </div>
          <button class="cta small" onclick={() => currentStep = 2}>Continue to config →</button>
          <button class="ghost" onclick={() => { inputPath = ""; meta = null; message = "Drop a video to begin."; }}>Remove</button>
        </div>
      {/if}
    </div>

    <div class="card">
      <div class="section-title">Engine</div>
      <div class="engine-cards">
        <div class="engine-card selected">
          <span class="engine-icon">🔒</span>
          <div><strong>Local</strong><span class="tag">100% On-device</span></div>
          <p>Processes on your own CPU/GPU. Your footage never leaves this machine.</p>
        </div>
        <div class="engine-card disabled" title="Cloud processing is not part of this build yet.">
          <span class="engine-icon">⚡</span>
          <div><strong>Cloud</strong><span class="tag">Coming soon</span></div>
          <p>Faster renders on remote GPUs — roadmap, not available yet.</p>
        </div>
      </div>
    </div>
    {/if}

    {#if currentStep === 2}
    <div class="card">
      <div class="section-title">AI model</div>
      <div class="model-row">
        {#each ["tiny", "base", "small"] as m}<button class="chip" class:sel={model === m} disabled={busy || modelBusy} onclick={() => setModel(m)}>{m}</button>{/each}
      </div>
      <p class="hint">tiny = fastest on CPU · base/small need a Pro license · GPU (CUDA) speeds all up</p>
      <div class="spec-line"><span class="dev">{device === "cuda" ? "GPU · CUDA" : "CPU · 1 thread"}</span><span>{model} model</span></div>
      <p class="hint">{storageMessage}</p>
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
      <button class="cta" disabled={!inputPath || busy || !runtimeReady || analyzeEnd <= analyzeStart} onclick={analyze}>
        {#if busy}<span class="spinner"></span>{/if}
        {busy ? "Processing…" : "Start analysis"}
      </button>
      {#if busy}<button class="ghost wide" onclick={abort}>Abort</button>{/if}
      <p class="hint" aria-live="polite">{licenseMessage}</p>
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
        <div class="candidates">
          {#each highlights as candidate}
            <button class:selected={candidate.id === selectedId} aria-pressed={candidate.id === selectedId} onclick={() => selectedId = candidate.id}>
              <strong>{candidate.title}</strong>
              <span>{clock(candidate.start)}–{clock(candidate.end)} · score {candidate.score}</span>
              {#if candidate.hook}<small>{candidate.hook}</small>{/if}
            </button>
          {/each}
        </div>
        <div class="spec-line"><span>Export {aspectRatio}</span><span>{highlights.length} candidates</span></div>
        <button class="cta secondary" disabled={busy || !selectedId} onclick={render}>
          {#if busy}<span class="spinner"></span>{/if}
          {busy ? "Rendering…" : `Render selected candidate (${aspectRatio})`}
        </button>
        {#if outputPath}<p class="hint">Saved: {outputPath}</p>{/if}
      </div>
    {:else}
      <div class="card"><p class="hint">No highlights yet. Run an analysis first.</p><button class="ghost wide" onclick={() => currentStep = 2}>← Back to config</button></div>
    {/if}
    {/if}

    <p class="message" aria-live="polite">{message}</p>
      </div></div>
  </section>

  <aside class="bench-right">
    <div class="panel"><div class="panel-head"><span class="panel-title">Session</span><span class="badge">{activeTab}</span></div>
      {#if activeTab === "recent"}
        {#if resumableJobs.length}{#each resumableJobs as job}<div class="row"><div class="row-body"><span class="row-title">{job.id.slice(-8)}</span><span class="row-sub">{job.status} · {job.phase}</span></div><div class="row-actions">{#if job.status === "queued" || job.status === "running"}<button class="btn btn-sm" disabled={busy} onclick={() => watch(job.id, true)}>Continue</button>{/if}{#if job.status === "failed"}<button class="btn btn-sm" disabled={busy} onclick={() => retryJob(job.id)}>Retry</button>{/if}</div></div>{/each}{:else}<p class="note">No projects yet — analyze a video to see results here.</p>{/if}
      {:else}<dl class="kv"><dt>Device</dt><dd>{device === "cuda" ? "GPU (CUDA)" : "CPU"}</dd><dt>Model</dt><dd>{model}</dd><dt>Storage</dt><dd>{storageMessage}</dd><dt>License</dt><dd>{licenseMessage}</dd></dl>{/if}
    </div>
    <div class="panel"><div class="seg"><button class="seg-item" class:on={activeTab === "recent"} onclick={() => activeTab = "recent"}>Recent</button><button class="seg-item" class:on={activeTab === "specs"} onclick={() => activeTab = "specs"}>Device</button></div><button class="btn btn-quiet btn-wide" onclick={() => storageHelp = !storageHelp}>Storage help</button>{#if storageHelp}<p class="note">Safe cleanup only: inspect caches and old renders before removing anything.</p>{/if}</div>
  </aside>
</main>