<script lang="ts">
  import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";
  import { studioState } from "../studio-state.svelte.ts";
  import Icon from "../icons.svelte";
  import { captionsFromTranscript, createEditorState, loadTranscriptCaptions, removeCaption, updateAspectRatio, updateHook, updateRange, upsertCaption, type EditorState } from "../../../../../packages/editor-core/index.ts";
  import type { Highlight } from "../../../../../packages/contracts/highlight.ts";
  import type { Caption } from "../../../../../packages/contracts/render-plan.ts";

  // Keep-alive mounted by +layout.svelte: re-read the Studio state whenever
  // this page becomes visible, so analysis results show up after navigation.
  export let active = false;

  const fallback: Highlight = { version: 1, id: "demo-highlight", sourceArtifactId: "demo-source", start: 1, end: 10, wordIds: [], title: "Demo highlight", hook: "A useful idea", score: 80 };
  let candidates: Highlight[] = [fallback];
  let sourcePath = "";
  let sourceDuration = 60;
  let selectedId = fallback.id;
  let state: EditorState = createEditorState("demo-source", 60, fallback);
  let saved = false;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let powered = false;
  let playing = false;
  let currentTime = 1;
  let video: HTMLVideoElement;
  let activeCaptionId = "";
  let draftCaption = "";
  let timeline: HTMLElement;
  let dragging: "range" | "in" | "out" | null = null;
  let dragStartX = 0;
  let dragStartRange = { start: 0, end: 0 };

  $: activeCaption = state.plan.captions.find((caption) => caption.id === activeCaptionId) ?? state.plan.captions[0];
  $: range = state.plan.ranges[0];

  function clientXToTime(event: MouseEvent): number {
    const rect = timeline.getBoundingClientRect();
    return Math.max(0, Math.min(sourceDuration, ((event.clientX - rect.left) / rect.width) * sourceDuration));
  }

  function onRangeKeydown(event: KeyboardEvent) {
    const step = event.shiftKey ? 1 : 0.25;
    let handled = true;
    try {
      if (event.key === "ArrowLeft" || event.key === "ArrowDown") { state = updateRange(state, { start: Math.max(0, range.start - step), end: range.end }); }
      else if (event.key === "ArrowRight" || event.key === "ArrowUp") { state = updateRange(state, { start: range.start, end: Math.min(sourceDuration, range.end + step) }); }
      else handled = false;
    } catch { handled = false; }
    if (handled) { studioState.plan = state.plan; event.preventDefault(); }
  }

  function onTimelineMouseDown(event: MouseEvent) {
    seekTimeline(event);
    dragging = "range";
    dragStartX = event.clientX;
    dragStartRange = { start: range.start, end: range.end };
    const onMove = (moveEvent: MouseEvent) => {
      if (!dragging) return;
      const rect = timeline.getBoundingClientRect();
      const movedByClientX = ((moveEvent.clientX - dragStartX) / rect.width) * sourceDuration;
      let nextStart = dragStartRange.start + movedByClientX;
      let nextEnd = dragStartRange.end + movedByClientX;
      if (nextStart < 0) { nextEnd += -nextStart; nextStart = 0; }
      if (nextEnd > sourceDuration) { nextStart -= nextEnd - sourceDuration; nextEnd = sourceDuration; }
      try { state = updateRange(state, { start: nextStart, end: nextEnd }); studioState.plan = state.plan; } catch { /* keep the last valid range */ }
      moveEvent.preventDefault();
    };
    const onUp = () => {
      dragging = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  function beginHandleDrag(kind: "in" | "out", event: MouseEvent) {
    event.stopPropagation();
    event.preventDefault();
    dragging = kind;
    dragStartX = event.clientX;
    dragStartRange = { start: range.start, end: range.end };
    const onMove = (moveEvent: MouseEvent) => {
      if (!dragging) return;
      const rect = timeline.getBoundingClientRect();
      const t = ((moveEvent.clientX - rect.left) / rect.width) * sourceDuration;
      if (dragging === "in") {
        const nextStart = Math.max(0, Math.min(t, dragStartRange.end - 0.2));
        try { state = updateRange(state, { start: nextStart, end: dragStartRange.end }); studioState.plan = state.plan; seek(nextStart); } catch { /* keep last valid range */ }
      } else {
        const nextEnd = Math.min(sourceDuration, Math.max(t, dragStartRange.start + 0.2));
        try { state = updateRange(state, { start: dragStartRange.start, end: nextEnd }); studioState.plan = state.plan; seek(nextEnd); } catch { /* keep last valid range */ }
      }
      moveEvent.preventDefault();
    };
    const onUp = () => {
      dragging = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  function loadFromStudio() {
    candidates = studioState.highlights.length ? studioState.highlights : [fallback];
    sourcePath = studioState.sourcePath;
    sourceDuration = studioState.sourceDuration || 60;
    powered = sourcePath !== "";
    const initial = studioState.selectedId || candidates[0]?.id || fallback.id;
    const picked = candidates.find((candidate) => candidate.id === initial) ?? fallback;
    selectedId = picked.id;
    state = studioState.plan ? { ...state, plan: studioState.plan } : createEditorState(picked.sourceArtifactId || "demo-source", sourceDuration, picked);
    if (studioState.transcript && !state.plan.captions.length) state = loadTranscriptCaptions(state, studioState.transcript);
    studioState.plan = state.plan;
    currentTime = range.start;
  }
  $: if (active) loadFromStudio();
  $: if (!active && video && !video.paused) video.pause();

  function select(id: string) {
    const candidate = candidates.find((item) => item.id === id) ?? fallback;
    selectedId = id;
    state = createEditorState(candidate.sourceArtifactId || "demo-source", sourceDuration, candidate);
    studioState.selectedId = id;
    studioState.plan = state.plan;
    currentTime = candidate.start;
  }
  function clock(seconds: number) { return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`; }
  function pct(seconds: number) { return `${Math.max(0, Math.min(100, (seconds / Math.max(sourceDuration, 1)) * 100))}%`; }
  function syncTime() { if (video) { currentTime = video.currentTime; playing = !video.paused; if (video.currentTime >= range.end) video.pause(); } }
  function togglePlay() { if (!video) return; if (video.paused) void video.play(); else video.pause(); }
  function seek(seconds: number) { const next = Math.max(0, Math.min(sourceDuration, seconds)); currentTime = next; if (video) video.currentTime = next; }
  function seekTimeline(event: MouseEvent) { const rect = (event.currentTarget as HTMLElement).getBoundingClientRect(); seek(((event.clientX - rect.left) / rect.width) * sourceDuration); }
  function setRangeEdge(edge: "start" | "end", value: number) {
    const next = edge === "start" ? Math.min(value, range.end - 0.1) : Math.max(value, range.start + 0.1);
    try { state = updateRange(state, { start: edge === "start" ? Math.max(0, next) : range.start, end: edge === "end" ? Math.min(sourceDuration, next) : range.end }); studioState.plan = state.plan; } catch { /* keep the last valid range */ }
  }
  function setAspect(value: "9:16" | "1:1" | "16:9") { state = updateAspectRatio(state, value); studioState.plan = state.plan; }
  function setCaptionText(text: string) { if (!activeCaption) return; draftCaption = text; try { state = upsertCaption(state, { ...activeCaption, text }); studioState.plan = state.plan; } catch { /* validation follows the contract */ } }
  function addCaption() { const start = Math.max(range.start, Math.min(currentTime, range.end - 0.5)); const caption: Caption = { id: `caption-${Date.now()}`, text: "New caption", start, end: Math.min(range.end, start + 2.5) }; state = upsertCaption(state, caption); activeCaptionId = caption.id; draftCaption = caption.text; studioState.plan = state.plan; }
  function deleteCaption() { if (!activeCaption) return; state = removeCaption(state, activeCaption.id); activeCaptionId = state.plan.captions[0]?.id ?? ""; studioState.plan = state.plan; }
  function save() { if (!isTauri()) return; saved = true; studioState.plan = state.plan; if (saveTimer) clearTimeout(saveTimer); saveTimer = setTimeout(() => { saved = false; saveTimer = undefined; }, 1600); }
  onDestroy(() => { if (saveTimer) clearTimeout(saveTimer); });
  function updateHookText(text: string) { if (!state.plan.hook) return; try { state = updateHook(state, { ...state.plan.hook, text }); studioState.plan = state.plan; } catch { /* keep current valid hook */ } }
</script>

<main class="bench editor-workspace">
  <aside class="bench-left">
    <div class="panel">
      <div class="panel-head"><span class="eyebrow">Project</span><span class="badge volt">editing</span></div>
      <h1>{studioState.sourceName || "Untitled cut"}</h1>
      <p class="note">{powered ? "Local source connected" : "Demo source · open Studio to load media"}</p>
    </div>
    <div class="panel">
      <div class="panel-head"><span class="panel-title">Highlights</span><span class="badge">{candidates.length}</span></div>
      <div class="stack">
        {#each candidates as candidate, index}
          <button class="cand" class:on={candidate.id === selectedId} aria-pressed={candidate.id === selectedId} onclick={() => select(candidate.id)}>
            <span class="cand-rank">{String(index + 1).padStart(2, "0")}</span>
            <span><span class="cand-title">{candidate.title}</span><span class="cand-sub tc">{clock(candidate.start)} — {clock(candidate.end)}</span>{#if candidate.hook}<span class="cand-hook">{candidate.hook}</span>{/if}</span>
            <span class="cand-score">{candidate.score}</span>
          </button>
        {/each}
      </div>
    </div>
    <div class="panel">
      <span class="eyebrow">Source</span>
      <dl class="kv"><dt>Duration</dt><dd class="tc">{clock(sourceDuration)}</dd><dt>Selected</dt><dd class="tc">{clock(range.start)}–{clock(range.end)}</dd><dt>Plan</dt><dd class="tc">v{state.plan.version}</dd></dl>
      <a class="btn btn-quiet btn-wide" href="/">← Back to Studio</a>
      {#if !isTauri()}<p class="note">Save and export are available in the installed desktop app.</p>{/if}
      <button class="btn btn-primary btn-wide" onclick={save}>Save render plan</button>
      {#if saved}<div class="save-toast" role="status" aria-live="polite"><Icon name="check" size={16} />Saved locally</div>{/if}
    </div>
  </aside>

  <section class="bench-center">
    <div class="stage">
      <div class="stage-head"><div class="stage-title"><span class="badge">cut / 01</span><span class="stage-name">{state.selected?.title ?? "Demo highlight"}</span></div><span class="badge ok">local timeline</span></div>
      <div class="stage-body">
        <div class="canvas-view">
          {#if powered}<video bind:this={video} aria-label="Video preview" src={convertFileSrc(sourcePath)} ontimeupdate={syncTime} onplay={() => playing = true} onpause={() => playing = false} onloadedmetadata={() => { sourceDuration = video.duration || sourceDuration; seek(range.start); }}><track kind="captions" /></video>{:else}<div class="empty-canvas"><span class="eyebrow">Preview canvas</span><strong>Open Studio and analyze a source</strong><span class="note">The editor workspace is ready with a demo cut.</span></div>{/if}
          <div class="frame-guide {state.plan.aspectRatio === "9:16" ? "r916" : state.plan.aspectRatio === "1:1" ? "r11" : "r169"}"><i></i></div>
        </div>
        <div class="transport"><button class="btn btn-icon" aria-label={playing ? "Pause" : "Play"} onclick={togglePlay}><Icon name={playing ? "pause" : "play"} size={14} /></button><span class="tc">{clock(currentTime)}</span><span class="transport-track"><span style={`width:${pct(currentTime)}`}></span></span><span class="tc muted">{clock(sourceDuration)}</span></div>
        <div class="timeline" aria-label="Editor timeline" bind:this={timeline}>
          <div class="tl-ruler" role="presentation" onclick={seekTimeline}>{#each [0, 0.25, 0.5, 0.75, 1] as tick}<span class="tl-tick" style={`left:${tick * 100}%`}></span><span class="tl-tick-label" style={`left:${tick * 100}%`}>{clock(sourceDuration * tick)}</span>{/each}</div>
        <div class="timeline-label">VIDEO</div>
        <p class="timeline-help">Click the ruler to seek. Drag the highlighted range to move it, or drag its left/right edges to trim. Arrow keys adjust the selected range.</p>
        <div class="tl-lane" role="presentation" onclick={seekTimeline}>

            <div class="tl-clip-base"></div>
            <div class="tl-range" class:dragging={dragging === "range"} style={`left:${pct(range.start)};width:${(Math.max(0, range.end - range.start) / Math.max(sourceDuration, 1)) * 100}%`}
                 onmousedown={onTimelineMouseDown} onkeydown={onRangeKeydown} role="slider" tabindex="0"
                 aria-label="Selected range" aria-orientation="horizontal" aria-valuemin="0" aria-valuemax={sourceDuration} aria-valuenow={Math.round(range.start * 100) / 100} aria-valuetext={`${clock(range.start)} to ${clock(range.end)}`}>
              <span class="tl-handle in" role="slider" tabindex="0" aria-orientation="horizontal" aria-valuemin="0" aria-valuemax={Math.round(range.end * 100) / 100} aria-valuenow={Math.round(range.start * 100) / 100} aria-label="Trim in" onmousedown={(event) => beginHandleDrag("in", event)}></span>
              <span class="tl-handle out" role="slider" tabindex="0" aria-orientation="horizontal" aria-valuemin={Math.round(range.start * 100) / 100} aria-valuemax={sourceDuration} aria-valuenow={Math.round(range.end * 100) / 100} aria-label="Trim out" onmousedown={(event) => beginHandleDrag("out", event)}></span>
            </div>
            <div class="tl-playhead" style={`left:${pct(currentTime)}`}></div>
          </div>
          <div class="timeline-label">CAPTIONS</div><div class="tl-lane captions" role="presentation" onclick={seekTimeline}>{#each state.plan.captions as caption}<button class="tl-clip" class:on={caption.id === activeCaption?.id} style={`left:${pct(caption.start)};width:${(Math.max(0.5, caption.end - caption.start) / Math.max(sourceDuration, 1)) * 100}%`} onclick={(event) => { event.stopPropagation(); activeCaptionId = caption.id; draftCaption = caption.text; seek(caption.start); }}>{caption.text}</button>{/each}</div>
          {#if state.plan.hook}<div class="timeline-label">HOOK</div><div class="tl-lane captions" role="presentation" onclick={seekTimeline}><button class="tl-clip tl-hook" style={`left:${pct(state.plan.hook.start)};width:${(Math.max(0.5, state.plan.hook.end - state.plan.hook.start) / Math.max(sourceDuration, 1)) * 100}%`} onclick={(event) => { event.stopPropagation(); seek(state.plan.hook?.start ?? range.start); }}>{state.plan.hook.text}</button></div>{/if}
        </div>
      </div>
    </div>
  </section>

  <aside class="bench-right">
    <div class="panel"><div class="panel-head"><span class="panel-title"><span class="eyebrow">Clip</span></span><span class="badge volt">active</span></div>
      <div class="field-row"><label class="field"><span class="field-label">In</span><input type="number" min="0" max={range.end - 0.1} step="0.1" value={range.start} onchange={(event) => setRangeEdge("start", Number(event.currentTarget.value))} /></label><label class="field"><span class="field-label">Out</span><input type="number" min={range.start + 0.1} max={sourceDuration} step="0.1" value={range.end} onchange={(event) => setRangeEdge("end", Number(event.currentTarget.value))} /></label></div>
      <div class="seg">{#each ["9:16", "1:1", "16:9"] as ratio}<button class="seg-item" class:on={state.plan.aspectRatio === ratio} onclick={() => setAspect(ratio as "9:16" | "1:1" | "16:9")}>{ratio}</button>{/each}</div>
      <p class="note">{clock(range.end - range.start)} selected · crop tracked subject · solo layout</p>
    </div>
    <div class="panel"><div class="panel-head"><span class="panel-title"><span class="eyebrow">Text &amp; hook</span></span><button class="btn btn-sm btn-quiet" onclick={addCaption}>+ Caption</button></div>
      {#if activeCaption}
        <textarea aria-label="Caption text" value={draftCaption} oninput={(event) => setCaptionText(event.currentTarget.value)}></textarea>
        <div class="field-row"><label class="field"><span class="field-label">Start</span><input type="number" step="0.1" value={activeCaption.start} onchange={(event) => { const next = { ...activeCaption, start: Number(event.currentTarget.value) }; state = upsertCaption(state, next); studioState.plan = state.plan; }} /></label><label class="field"><span class="field-label">End</span><input type="number" step="0.1" value={activeCaption.end} onchange={(event) => { const next = { ...activeCaption, end: Number(event.currentTarget.value) }; state = upsertCaption(state, next); studioState.plan = state.plan; }} /></label></div>
        <button class="btn btn-danger btn-wide btn-sm" onclick={deleteCaption}>Remove caption</button>
      {:else}<p class="note">No caption cue yet. Add one or load transcript cues.</p>{/if}
      <div class="panel" style="border-top:1px solid var(--line-soft);padding-top:.6rem;">
        {#if state.plan.hook}<div class="spread"><span class="eyebrow">Hook · first five seconds</span><span class="note tc" style="margin:0;">{clock(state.plan.hook.start)}–{clock(state.plan.hook.end)}</span></div><textarea aria-label="Hook text" value={state.plan.hook.text} oninput={(event) => updateHookText(event.currentTarget.value)}></textarea>{:else}<p class="note" style="margin:0;">No hook attached to this cut.</p>{/if}
      </div>
    </div>
  </aside>
</main>
