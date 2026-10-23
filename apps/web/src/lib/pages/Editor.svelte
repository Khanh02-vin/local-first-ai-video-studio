<script lang="ts">
  import { convertFileSrc, invoke, isTauri } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";
  import { enqueueProjectSave } from "../project-save-queue.ts";
  import { studioState } from "../studio-state.svelte.ts";
  import Icon from "../icons.svelte";
  import { addRange, captionsFromTranscript, createEditorState, createEditorStateFromVariant, loadTranscriptCaptions, moveRange, removeCaption, removeRange, setCropKeyframes, setSceneLayouts, setSpeakerMap, updateAspectRatio, updateHook, updateRange, upsertCaption, type EditorState } from "../../../../../packages/editor-core/index.ts";
  import type { Highlight } from "../../../../../packages/contracts/highlight.ts";
  import type { Caption, CropKeyframe, SceneLayout } from "../../../../../packages/contracts/render-plan.ts";
  import { cropCenterAt } from "../../../../../packages/media-engine/reframe/index.ts";
  import { cropKeyframesFromPath } from "../../../../../packages/media-engine/reframe/crop-keyframes.ts";

  // Keep-alive mounted by +layout.svelte: re-read the Studio state whenever
  // this page becomes visible, so analysis results show up after navigation.
  export let active = false;

  const fallback: Highlight = { version: 1, id: "demo-highlight", sourceArtifactId: "demo-source", start: 1, end: 10, wordIds: [], title: "Demo highlight", hook: "A useful idea", score: 80 };
  let candidates: Highlight[] = [fallback];
  let sourcePath = "";
  let sourceDuration = 60;
  let selectedId = fallback.id;
  let editingVariantId = "";
  let state: EditorState = createEditorState("demo-source", 60, fallback);
  let saved = false;
  let saveError = "";
  let projectRevision = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let powered = false;
  let playing = false;
  let videoError = "";
  let previewPath = "";
  let previewBusy = false;
  let previewFallbackTried = false;
  let currentTime = 1;
  let video: HTMLVideoElement;
  let activeCaptionId = "";
  let draftCaption = "";
  let timeline: HTMLElement;
  let dragging: "range" | "in" | "out" | null = null;
  let dragStartX = 0;
  let dragStartRange = { start: 0, end: 0 };
  let activeRangeIndex = 0;
  let cropBusy = false;
  let cropMessage = "";
  let trackingEnabled = false;
  let sceneBusy = false;
  let sceneMessage = "";

  $: activeCaption = state.plan.captions.find((caption) => caption.id === activeCaptionId) ?? state.plan.captions[0];
  $: range = state.plan.ranges[activeRangeIndex] ?? state.plan.ranges[0];

  function clientXToTime(event: MouseEvent): number {
    const rect = timeline.getBoundingClientRect();
    return Math.max(0, Math.min(sourceDuration, ((event.clientX - rect.left) / rect.width) * sourceDuration));
  }

  function savePlan() {
    studioState.plan = state.plan;
    if (editingVariantId) {
      const variants = studioState.variants.map((variant) => variant.id === editingVariantId ? { ...variant, plan: state.plan } : variant);
      studioState.variants = variants; studioState.plans = { ...studioState.plans, [`variant:${editingVariantId}`]: state.plan };
      if (isTauri() && studioState.projectId) void save();
    } else studioState.plans = { ...studioState.plans, [selectedId]: state.plan };
  }
  function chooseRange(index: number) { activeRangeIndex = index; seek(state.plan.ranges[index].start); }
  function addRangeAfter() {
    const start = Math.min(sourceDuration - 0.2, range.end + 0.2);
    if (start < 0) return;
    const end = Math.min(sourceDuration, start + Math.min(5, sourceDuration - start));
    if (end <= start) return;
    try {
      state = addRange(state, { start, end }, activeRangeIndex + 1);
      activeRangeIndex += 1; savePlan(); seek(start);
    } catch { /* no valid gap at this position */ }
  }
  function deleteActiveRange() {
    if (state.plan.ranges.length <= 1) return;
    state = removeRange(state, activeRangeIndex);
    activeRangeIndex = Math.min(activeRangeIndex, state.plan.ranges.length - 1); savePlan(); seek(range.start);
  }
  function reorderRange(offset: -1 | 1) {
    const target = activeRangeIndex + offset;
    if (target < 0 || target >= state.plan.ranges.length) return;
    state = moveRange(state, activeRangeIndex, offset); activeRangeIndex = target; savePlan();
  }

  function onRangeKeydown(event: KeyboardEvent) {
    const step = event.shiftKey ? 1 : 0.25;
    let handled = true;
    try {
      if (event.key === "ArrowLeft" || event.key === "ArrowDown") { state = updateRange(state, { start: Math.max(0, range.start - step), end: range.end }, activeRangeIndex); }
      else if (event.key === "ArrowRight" || event.key === "ArrowUp") { state = updateRange(state, { start: range.start, end: Math.min(sourceDuration, range.end + step) }, activeRangeIndex); }
      else handled = false;
    } catch { handled = false; }
    if (handled) { savePlan(); event.preventDefault(); }
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
      try { state = updateRange(state, { start: nextStart, end: nextEnd }, activeRangeIndex); savePlan(); } catch { /* keep the last valid range */ }
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
        try { state = updateRange(state, { start: nextStart, end: dragStartRange.end }, activeRangeIndex); savePlan(); seek(nextStart); } catch { /* keep last valid range */ }
      } else {
        const nextEnd = Math.min(sourceDuration, Math.max(t, dragStartRange.start + 0.2));
        try { state = updateRange(state, { start: dragStartRange.start, end: nextEnd }, activeRangeIndex); savePlan(); seek(nextEnd); } catch { /* keep last valid range */ }
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

  function setTracking(enabled: boolean) {
    const usable = (state.plan.cropKeyframes ?? []).some((keyframe) => keyframe.confidence >= 0.5 && keyframe.speakerId && keyframe.sourceFaceId && keyframe.sourceArtifactId === state.plan.sourceArtifactId);
    trackingEnabled = enabled && usable;
    const keys = (state.plan.cropKeyframes ?? []).map((keyframe) => ({ ...keyframe, confirmed: trackingEnabled }));
    state = setCropKeyframes(state, keys); savePlan();
  }
  async function analyzeCropPath() {
    if (!isTauri() || !sourcePath || !studioState.transcript || cropBusy) return;
    const transcript = studioState.transcript;
    const intervals = transcript.segments.filter((segment) => segment.speakerId).map((segment) => ({ start: segment.start, end: segment.end, speakerId: segment.speakerId }));
    cropBusy = true; cropMessage = "Analyzing local frames…";
    try {
      const raw = await invoke<Array<{ time: number; centerX: number; centerY: number; speakerId: string | null; faceIndex: number; confidence: number; sourceArtifactId: string; sceneId: number }>>("analyze_crop_path", { path: sourcePath, duration: Math.min(sourceDuration, 1800), speakerIntervals: intervals, faceMapping: { confirmed: studioState.speakerFaceConfirmed, sourceArtifactId: transcript.sourceArtifactId, matches: studioState.speakerFaceMatches }, sourceArtifactId: state.plan.sourceArtifactId });
      // The Rust command already tracked and smoothed the path; map it onto the
      // contract as-is. Re-tracking here would smooth the path a second time and
      // drift the crop away from the subject.
      const confirmedMapping = studioState.speakerFaceConfirmed && studioState.transcript?.sourceArtifactId === state.plan.sourceArtifactId && Object.keys(studioState.speakerFaceMatches).length > 0;
      const keys: CropKeyframe[] = cropKeyframesFromPath(raw, { speakerFaceMatches: studioState.speakerFaceMatches, mappingConfirmed: confirmedMapping, sourceArtifactId: state.plan.sourceArtifactId });
      state = setSpeakerMap(state, studioState.speakerFaceConfirmed && studioState.transcript?.sourceArtifactId === state.plan.sourceArtifactId ? { ...studioState.speakerFaceMatches } : {});
      state = setCropKeyframes(state, keys); savePlan(); trackingEnabled = false;
      cropMessage = keys.length ? `${keys.length} crop points ready for review. Approve tracking to use them in export.` : "No usable face path; export will use center crop.";
    } catch (error) { cropMessage = `Crop analysis failed: ${String(error)}`; }
    finally { cropBusy = false; }
  }
  function updateManualCropCenter(centerX: number) {
    const keys = [...(state.plan.cropKeyframes ?? [])];
    const current = cropCenterAt(keys, currentTime);
    const item: CropKeyframe = { time: currentTime, centerX, centerY: current.y, sourceArtifactId: state.plan.sourceArtifactId, confidence: 1, confirmed: trackingEnabled };
    const index = keys.findIndex((key) => Math.abs(key.time - currentTime) < 0.25);
    if (index >= 0) keys[index] = { ...keys[index], ...item }; else keys.push(item);
    keys.sort((a, b) => a.time - b.time);
    state = setCropKeyframes(state, keys.slice(0, 600)); savePlan();
  }
  function adjustCropCenter() {
    const current = cropCenterAt(state.plan.cropKeyframes ?? [], currentTime);
    updateManualCropCenter(current.x);
  }
  async function analyzeScenes() {
    if (!isTauri() || !sourcePath || sceneBusy) return;
    sceneBusy = true; sceneMessage = "Analyzing local scene samples…";
    try {
      const scenes = await invoke<SceneLayout[]>("analyze_scene_layouts", { path: sourcePath, duration: Math.min(sourceDuration, 1800), sourceArtifactId: state.plan.sourceArtifactId });
        state = setSceneLayouts(state, scenes.map((scene) => ({ ...scene, sourceArtifactId: state.plan.sourceArtifactId, confirmed: false }))); savePlan();
      sceneMessage = `${scenes.length} scene intervals ready for review; uncertain scenes remain solo.`;
    } catch (error) { sceneMessage = `Scene analysis failed: ${String(error)}`; }
    finally { sceneBusy = false; }
  }
  function chooseLayout(index: number, layout: SceneLayout["layout"]) {
    const scenes = [...(state.plan.sceneLayouts ?? [])]; scenes[index] = { ...scenes[index], layout, confirmed: true };
    state = setSceneLayouts(state, scenes); savePlan();
  }
  function approveScenes() { state = setSceneLayouts(state, (state.plan.sceneLayouts ?? []).map((scene) => ({ ...scene, confirmed: true }))); savePlan(); }
  function disableScenes() { state = setSceneLayouts(state, []); savePlan(); }

  function handleVideoError() {
    if (video?.error?.code === 4 && !isRemoteSource(sourcePath) && !previewFallbackTried && isTauri()) {
      previewFallbackTried = true;
      previewBusy = true;
      videoError = "Preparing a video preview compatible with Linux…";
      void invoke<string>("prepare_video_preview", { path: sourcePath })
        .then((path) => { previewPath = path; videoError = ""; })
        .catch((error) => { videoError = `Could not prepare a compatible preview: ${String(error)}`; })
        .finally(() => { previewBusy = false; });
      return;
    }
    videoError = isRemoteSource(sourcePath)
      ? "This online video cannot be previewed here. Render it from Studio or open a local video file."
      : video?.error?.code === 4
        ? "This video format or codec is not supported by the preview."
        : "Could not load the source video. Check that the original file still exists and is readable.";
  }
  function isRemoteSource(path: string) { return path.startsWith("http://") || path.startsWith("https://"); }
  function loadFromStudio() {
    candidates = studioState.highlights.length ? studioState.highlights : [fallback];
    sourcePath = studioState.sourcePath;
    projectRevision = studioState.projectRevision;
    previewPath = sourcePath;
    previewBusy = false;
    previewFallbackTried = false;
    videoError = "";
    sourceDuration = studioState.sourceDuration || 60;
    powered = sourcePath !== "";
    editingVariantId = studioState.editingVariantId;
    if (editingVariantId) {
      const variant = studioState.variants.find((item) => item.id === editingVariantId);
      if (!variant || !studioState.transcript) { editingVariantId = ""; studioState.editingVariantId = ""; return; }
      selectedId = `variant:${variant.id}`;
      const storedPlan = variant.plan ?? studioState.plans[selectedId];
      state = storedPlan ? { sourceArtifactId: variant.sourceArtifactId, duration: studioState.transcript.duration, plan: storedPlan } : createEditorStateFromVariant(studioState.transcript, variant);
      if (!storedPlan) { variant.plan = state.plan; studioState.variants = [...studioState.variants]; }
      activeRangeIndex = 0; studioState.plan = state.plan; studioState.plans = { ...studioState.plans, [selectedId]: state.plan };
      currentTime = range.start; return;
    }
    const initial = studioState.selectedId || candidates[0]?.id || fallback.id;
    const picked = candidates.find((candidate) => candidate.id === initial) ?? fallback;
    selectedId = picked.id;
    activeRangeIndex = 0;
    const plan = studioState.plans[initial] ?? studioState.plan;
    state = plan ? { ...state, selected: picked, plan } : createEditorState(picked.sourceArtifactId || "demo-source", sourceDuration, picked);
    if (studioState.transcript && !state.plan.captions.length) state = loadTranscriptCaptions(state, studioState.transcript);
    studioState.plan = state.plan; studioState.plans = { ...studioState.plans, [selectedId]: state.plan };
    currentTime = range.start;
  }
  $: if (active) loadFromStudio();
  $: if (!active && video && !video.paused) video.pause();

  function select(id: string) {
    const candidate = candidates.find((item) => item.id === id) ?? fallback;
    selectedId = id;
    editingVariantId = ""; studioState.editingVariantId = "";
    state = studioState.plans[id] ? { ...state, selected: candidate, plan: studioState.plans[id] } : createEditorState(candidate.sourceArtifactId || "demo-source", sourceDuration, candidate);
    studioState.selectedId = id;
    studioState.plan = state.plan; studioState.plans = { ...studioState.plans, [selectedId]: state.plan };
    currentTime = state.plan.ranges[0]?.start ?? candidate.start;
  }
  function clock(seconds: number) { return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`; }
  function pct(seconds: number) { return `${Math.max(0, Math.min(100, (seconds / Math.max(sourceDuration, 1)) * 100))}%`; }
  function syncTime() { if (video) { currentTime = video.currentTime; playing = !video.paused; if (video.currentTime >= range.end) video.pause(); } }
  function togglePlay() { if (!video) return; if (video.paused) void video.play(); else video.pause(); }
  function seek(seconds: number) { const next = Math.max(0, Math.min(sourceDuration, seconds)); currentTime = next; if (video) video.currentTime = next; }
  function seekTimeline(event: MouseEvent) { const rect = (event.currentTarget as HTMLElement).getBoundingClientRect(); seek(((event.clientX - rect.left) / rect.width) * sourceDuration); }
  function setRangeEdge(edge: "start" | "end", value: number) {
    const next = edge === "start" ? Math.min(value, range.end - 0.1) : Math.max(value, range.start + 0.1);
    try { state = updateRange(state, { start: edge === "start" ? Math.max(0, next) : range.start, end: edge === "end" ? Math.min(sourceDuration, next) : range.end }, activeRangeIndex); savePlan(); } catch { /* keep the last valid range */ }
  }
  function setAspect(value: "9:16" | "1:1" | "16:9") { state = updateAspectRatio(state, value); savePlan(); }
  function setCaptionText(text: string) { if (!activeCaption) return; draftCaption = text; try { state = upsertCaption(state, { ...activeCaption, text }); savePlan(); } catch { /* validation follows the contract */ } }
  function addCaption() { const start = Math.max(range.start, Math.min(currentTime, range.end - 0.5)); const caption: Caption = { id: `caption-${Date.now()}`, text: "New caption", start, end: Math.min(range.end, start + 2.5) }; state = upsertCaption(state, caption); activeCaptionId = caption.id; draftCaption = caption.text; savePlan(); }
  function deleteCaption() { if (!activeCaption) return; state = removeCaption(state, activeCaption.id); activeCaptionId = state.plan.captions[0]?.id ?? ""; savePlan(); }
  async function save() {
    if (!isTauri() || !studioState.projectId || !selectedId) return;
    saved = false; saveError = "";
    try {
      await enqueueProjectSave(studioState.projectId, async () => {
        const project = await invoke<Record<string, unknown>>("project_get", { id: studioState.projectId });
        const plans = editingVariantId
          ? { ...studioState.plans, [`variant:${editingVariantId}`]: state.plan }
          : { ...studioState.plans, [selectedId]: state.plan };
        const variants = editingVariantId
          ? studioState.variants.map((variant) => variant.id === editingVariantId ? { ...variant, plan: state.plan } : variant)
          : studioState.variants;
        const savedProject = await invoke<Record<string, unknown>>("project_save", {
          project: {
            ...project,
            ...(editingVariantId ? { variants, selectedVariantId: editingVariantId } : { selectedId }),
            plans,
            revision: Number(project.revision ?? projectRevision),
          },
        });
        projectRevision = Number(savedProject.revision ?? projectRevision);
        studioState.projectRevision = projectRevision;
        studioState.plans = plans;
        if (editingVariantId) studioState.variants = variants;
      });
      studioState.plan = state.plan;
      saved = true;
      if (saveTimer) clearTimeout(saveTimer);
      saveTimer = setTimeout(() => { saved = false; saveTimer = undefined; }, 1600);
    } catch (error) {
      saved = false; saveError = String(error).includes("PROJECT_REVISION_CONFLICT")
        ? "This project changed in another view. Reload the project before saving again."
        : `Could not save render plan: ${String(error)}`;
    }
  }
  onDestroy(() => { if (saveTimer) clearTimeout(saveTimer); });
  function updateHookText(text: string) { if (!state.plan.hook) return; try { state = updateHook(state, { ...state.plan.hook, text }); savePlan(); } catch { /* keep current valid hook */ } }
</script>

<main class="bench editor-workspace">
  <aside class="bench-left">
    <div class="panel">
      <div class="panel-head"><span class="eyebrow">Project</span><span class="badge volt">editing</span></div>
      <h1>{studioState.sourceName || "Untitled cut"}</h1>
      <p class="note">{powered ? "Local source connected" : "Demo source · open Studio to load media"}</p>
    </div>
    <div class="panel">
      <div class="panel-head"><span class="panel-title">{editingVariantId ? "Transcript variants" : "Highlights"}</span><span class="badge">{editingVariantId ? studioState.variants.length : candidates.length}</span></div>
      <div class="stack">
        {#each studioState.variants as variant, index}
          <button class="cand" class:on={variant.id === editingVariantId} aria-pressed={variant.id === editingVariantId} onclick={() => { editingVariantId = variant.id; studioState.editingVariantId = variant.id; loadFromStudio(); }}>
            <span class="cand-rank">V{index + 1}</span><span><span class="cand-title">{variant.title}</span><span class="cand-sub">Transcript variant · {variant.ranges.length} ranges</span></span>
          </button>
        {/each}
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
      <button class="btn btn-primary btn-wide" onclick={save}>Save edits</button>
      <p class="note">Step 1: choose your clip. Step 2: adjust the preview and dates. Step 3: save edits. To create an MP4, return to Studio and choose Render.</p>
      {#if saveError}<div class="message" role="alert">{saveError}</div>{/if}
      {#if saved}<div class="save-toast" role="status" aria-live="polite"><Icon name="check" size={16} />Saved locally</div>{/if}
    </div>
  </aside>

  <section class="bench-center">
    <div class="stage">
      <div class="stage-head"><div class="stage-title"><span class="badge">cut / 01</span><span class="stage-name">{editingVariantId ? (studioState.variants.find((variant) => variant.id === editingVariantId)?.title ?? "Transcript variant") : (state.selected?.title ?? "Demo highlight")}</span></div><span class="badge ok">local timeline</span></div>
      <div class="stage-body">
        <div class="canvas-view">
          {#if powered && isRemoteSource(sourcePath)}<div class="empty-canvas"><span class="eyebrow">Online video</span><strong>Preview in Studio</strong><span class="note">Local Editor preview works with video files saved on this device.</span></div>{:else if powered}<video bind:this={video} aria-label="Video preview" src={convertFileSrc(previewPath)} onerror={handleVideoError} ontimeupdate={syncTime} onplay={() => { playing = true; videoError = ""; }} onpause={() => playing = false} onloadedmetadata={() => { videoError = ""; sourceDuration = video.duration || sourceDuration; seek(range.start); }}><track kind="captions" /></video>{:else}<div class="empty-canvas"><span class="eyebrow">Preview canvas</span><strong>Open Studio and analyze a source</strong><span class="note">The editor workspace is ready with a demo cut.</span></div>{/if}
          {#if previewBusy}<div class="message preview-error" role="status">Preparing a preview-friendly copy of the video. This may take a few minutes.</div>{:else if videoError}<div class="message error preview-error" role="alert">{videoError}</div>{/if}
          <div class="frame-guide {state.plan.aspectRatio === "9:16" ? "r916" : state.plan.aspectRatio === "1:1" ? "r11" : "r169"}"><i></i></div>
        </div>
        <div class="transport"><button class="btn btn-icon" aria-label={playing ? "Pause" : "Play"} onclick={togglePlay}><Icon name={playing ? "pause" : "play"} size={14} /></button><span class="tc">{clock(currentTime)}</span><span class="transport-track"><span style={`width:${pct(currentTime)}`}></span></span><span class="tc muted">{clock(sourceDuration)}</span></div>
        <div class="timeline" aria-label="Editor timeline" bind:this={timeline}>
          <div class="tl-ruler" role="presentation" onclick={seekTimeline}>{#each [0, 0.25, 0.5, 0.75, 1] as tick}<span class="tl-tick" style={`left:${tick * 100}%`}></span><span class="tl-tick-label" style={`left:${tick * 100}%`}>{clock(sourceDuration * tick)}</span>{/each}</div>
        <div class="timeline-label">VIDEO</div>
        <p class="timeline-help">Click the ruler to seek. Drag the highlighted range to move it, or drag its left/right edges to trim. Arrow keys adjust the selected range.</p>
        <div class="tl-lane" role="presentation" onclick={seekTimeline}>
            <div class="tl-clip-base"></div>
            {#each state.plan.ranges as item, index}
              <div class="tl-range" class:dragging={dragging === "range" && index === activeRangeIndex} class:inactive={index !== activeRangeIndex} style={`left:${pct(item.start)};width:${(Math.max(0, item.end - item.start) / Math.max(sourceDuration, 1)) * 100}%`}
                   onclick={(event) => { event.stopPropagation(); if (index !== activeRangeIndex) chooseRange(index); }}
                   onmousedown={(event) => { if (index === activeRangeIndex) onTimelineMouseDown(event); }}
                   onkeydown={index === activeRangeIndex ? onRangeKeydown : (event) => { if (event.key === "Enter" || event.key === " ") chooseRange(index); }}
                   role="slider" tabindex="0" aria-label={`Range ${index + 1}`} aria-orientation="horizontal" aria-valuemin="0" aria-valuemax={sourceDuration} aria-valuenow={Math.round(item.start * 100) / 100} aria-valuetext={`${clock(item.start)} to ${clock(item.end)}`}>
                {#if index === activeRangeIndex}
                  <span class="tl-handle in" role="slider" tabindex="0" aria-orientation="horizontal" aria-valuemin="0" aria-valuemax={Math.round(item.end * 100) / 100} aria-valuenow={Math.round(item.start * 100) / 100} aria-label="Trim in" onmousedown={(event) => beginHandleDrag("in", event)}></span>
                  <span class="tl-handle out" role="slider" tabindex="0" aria-orientation="horizontal" aria-valuemin={Math.round(item.start * 100) / 100} aria-valuemax={sourceDuration} aria-valuenow={Math.round(item.end * 100) / 100} aria-label="Trim out" onmousedown={(event) => beginHandleDrag("out", event)}></span>
                {:else}<span class="tl-range-number">{index + 1}</span>{/if}
              </div>
            {/each}
            <div class="tl-playhead" style={`left:${pct(currentTime)}`}></div>
          </div>
          <div class="timeline-label">CAPTIONS</div><div class="tl-lane captions" role="presentation" onclick={seekTimeline}>{#each state.plan.captions as caption}<button class="tl-clip" class:on={caption.id === activeCaption?.id} style={`left:${pct(caption.start)};width:${(Math.max(0.5, caption.end - caption.start) / Math.max(sourceDuration, 1)) * 100}%`} onclick={(event) => { event.stopPropagation(); activeCaptionId = caption.id; draftCaption = caption.text; seek(caption.start); }}>{caption.text}</button>{/each}</div>
          {#if state.plan.hook}<div class="timeline-label">HOOK</div><div class="tl-lane captions" role="presentation" onclick={seekTimeline}><button class="tl-clip tl-hook" style={`left:${pct(state.plan.hook.start)};width:${(Math.max(0.5, state.plan.hook.end - state.plan.hook.start) / Math.max(sourceDuration, 1)) * 100}%`} onclick={(event) => { event.stopPropagation(); seek(state.plan.hook?.start ?? range.start); }}>{state.plan.hook.text}</button></div>{/if}
        </div>
      </div>
    </div>
  </section>

  <aside class="bench-right">
    <div class="panel">
      <div class="panel-head"><span class="panel-title"><span class="eyebrow">Clip · edit the selected video segment</span></span><span class="badge volt">active</span></div>
      <div class="field-row"><label class="field"><span class="field-label">Start</span><input type="number" min="0" max={range.end - 0.1} step="0.1" value={range.start} onchange={(event) => setRangeEdge("start", Number(event.currentTarget.value))} /></label><label class="field"><span class="field-label">End</span><input type="number" min={range.start + 0.1} max={sourceDuration} step="0.1" value={range.end} onchange={(event) => setRangeEdge("end", Number(event.currentTarget.value))} /></label></div>
      <div class="spread"><span class="note">Range {activeRangeIndex + 1} of {state.plan.ranges.length} · {clock(range.end - range.start)}</span><span class="note">Order is export order</span></div>
      <div class="field-row"><button class="btn btn-sm btn-quiet" onclick={addRangeAfter}>+ Add range</button><button class="btn btn-sm btn-quiet" disabled={activeRangeIndex === 0} onclick={() => reorderRange(-1)}>Move earlier</button><button class="btn btn-sm btn-quiet" disabled={activeRangeIndex === state.plan.ranges.length - 1} onclick={() => reorderRange(1)}>Move later</button><button class="btn btn-danger btn-sm" disabled={state.plan.ranges.length <= 1} onclick={deleteActiveRange}>Remove</button></div>{#each ["9:16", "1:1", "16:9"] as ratio}<button class="seg-item" class:on={state.plan.aspectRatio === ratio} onclick={() => setAspect(ratio as "9:16" | "1:1" | "16:9")}>{ratio}</button>{/each}
    </div>
    <details class="editor-disclosure">
      <summary>Speaker framing · optional</summary>
      <div class="crop-review"><div class="panel-head"><span class="eyebrow">Speaker framing</span><span class="badge" class:ok={trackingEnabled}>{trackingEnabled ? "approved" : "center crop"}</span></div><p class="note">{cropMessage || "Local face path is used only after explicit review."}</p><div class="field-row"><button class="btn btn-sm" disabled={!powered || cropBusy || !studioState.transcript} onclick={analyzeCropPath}>{cropBusy ? "Analyzing…" : "Analyze local crop path"}</button><button class="btn btn-sm btn-primary" disabled={!state.plan.cropKeyframes?.length} onclick={() => setTracking(!trackingEnabled)}>{trackingEnabled ? "Use center crop" : "Approve tracking"}</button><button class="btn btn-sm btn-quiet" disabled={!trackingEnabled} onclick={adjustCropCenter}>Adjust center here</button></div><div class="field-row"><label>Crop X <input type="range" min="0" max="1" step="0.01" value={cropCenterAt(state.plan.cropKeyframes ?? [], currentTime).x} oninput={(event) => updateManualCropCenter(Number(event.currentTarget.value))} /></label></div></div>
      <p class="note">{clock(range.end - range.start)} selected · {trackingEnabled && state.plan.cropKeyframes?.some((key) => key.confirmed && key.confidence >= 0.5) ? "approved speaker tracking" : "center crop fallback"} · {state.plan.sceneLayouts?.some((scene) => scene.confirmed) ? "reviewed scene layout" : "static solo layout"}</p>
    </details>
    <details class="editor-disclosure">
      <summary>Scene layouts · optional</summary>
      <div class="scene-review"><div class="panel-head"><span class="eyebrow">Scene layouts</span><button class="btn btn-sm" disabled={!powered || sceneBusy} onclick={analyzeScenes}>{sceneBusy ? "Analyzing…" : "Analyze scenes"}</button><button class="btn btn-sm btn-primary" disabled={!state.plan.sceneLayouts?.length} onclick={approveScenes}>Approve scenes</button><button class="btn btn-sm btn-quiet" onclick={disableScenes}>Disable</button></div><p class="note">{sceneMessage || "Local shot analysis is bounded; review each proposed layout before export."}</p>{#if state.plan.sceneLayouts?.length}{#each state.plan.sceneLayouts as scene, index}<div class="field-row"><span class="note">{clock(scene.start)}–{clock(scene.end)} · {Math.round(scene.confidence * 100)}%</span><select value={scene.layout} onchange={(event) => chooseLayout(index, event.currentTarget.value as SceneLayout["layout"])}><option value="solo">Solo</option><option value="side-by-side">Side by side</option></select></div>{/each}{:else}<p class="note">No reviewed scenes. Export keeps the static solo layout.</p>{/if}</div>
    </details>
    <details class="panel editor-disclosure">
      <summary>Captions &amp; hook · optional</summary>
      <div class="panel-head"><span class="panel-title"><span class="eyebrow">Text &amp; hook</span></span><button class="btn btn-sm btn-quiet" onclick={addCaption}>+ Caption</button></div>
      {#if activeCaption}
        <textarea aria-label="Caption text" value={draftCaption} oninput={(event) => setCaptionText(event.currentTarget.value)}></textarea>
      <div class="field-row"><label class="field"><span class="field-label">Start</span><input type="number" step="0.1" value={activeCaption.start} onchange={(event) => { const next = { ...activeCaption, start: Number(event.currentTarget.value) }; try { state = upsertCaption(state, next); savePlan(); } catch { /* keep the last valid caption */ } }} /></label><label class="field"><span class="field-label">End</span><input type="number" step="0.1" value={activeCaption.end} onchange={(event) => { const next = { ...activeCaption, end: Number(event.currentTarget.value) }; try { state = upsertCaption(state, next); savePlan(); } catch { /* keep the last valid caption */ } }} /></label></div>
        <button class="btn btn-danger btn-wide btn-sm" onclick={deleteCaption}>Remove caption</button>
      {:else}<p class="note">No caption cue yet. Add one or load transcript cues.</p>{/if}
      <div class="panel" style="border-top:1px solid var(--line-soft);padding-top:.6rem;">
        {#if state.plan.hook}<div class="spread"><span class="eyebrow">Hook · first five seconds</span><span class="note tc" style="margin:0;">{clock(state.plan.hook.start)}–{clock(state.plan.hook.end)}</span></div><textarea aria-label="Hook text" value={state.plan.hook.text} oninput={(event) => updateHookText(event.currentTarget.value)}></textarea>{:else}<p class="note" style="margin:0;">No hook attached to this cut.</p>{/if}
      </div>
    </details>
  </aside>
</main>
