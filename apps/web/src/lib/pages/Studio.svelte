<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { Highlight } from "../../../../../packages/contracts/highlight.ts";
  import type { Collection } from "../../../../../packages/contracts/collection.ts";
  import type { Transcript } from "../../../../../packages/contracts/transcript.ts";
  import { applySpeakerIntervals, transcriptSpeakerIds, speakerExamples } from "../../../../../services/ai-pipeline/diarization.ts";
  import type { EditVariant, RenderPlan } from "../../../../../packages/contracts/render-plan.ts";
  import { mapEditVariantProposals, VARIANT_PROMPT_VERSION } from "../../../../../services/ai-pipeline/variants.ts";
  import { studioState } from "../studio-state.svelte.ts";
  import { createEditorState, createEditorStateFromVariant } from "../../../../../packages/editor-core/index.ts";
  import { refreshRuntime, runtimeStore } from "../runtime-status.svelte.ts";
  import Icon from "../icons.svelte";
  import RelatedYoutubeSearch from "../components/RelatedYoutubeSearch.svelte";

  // Keep-alive mounted by +layout.svelte: `active` tells this page it is the
  // visible one, so background polls only run while the user can see them.
  let { active = true }: { active?: boolean } = $props();

  type Meta = { name: string; duration: number; width: number; height: number };
  type JobRow = { id: string; input?: string; status: string; phase: string; progress: number; rangeStart?: number; rangeEnd?: number; chunkCompleted?: number; chunkTotal?: number; error?: string };
  type Project = { id: string; title: string; sourcePath: string; sourceName: string; sourceDuration: number; analysisJobId: string | null; analysisResultId?: string | null; transcript: unknown | null; highlights: Highlight[]; selectedId: string | null; selectedIds?: string[]; plans: Record<string, RenderPlan>; variants?: EditVariant[]; selectedVariantId?: string | null; speakerFaceMatches?: { version: 1; sourceArtifactId: string; confirmed: boolean; matches: Record<string, string> } | null; renders: Array<{ output: string; status: string; highlightId: string }>; updatedAt: string };

  let projects = $state<Project[]>([]);
  let projectId = $state("");
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
  let selectedIds = $state<string[]>([]);
  let variants = $state<EditVariant[]>([]);
  let selectedVariantId = $state("");
  let variantsBusy = $state(false);
  let diarizationBusy = $state(false);
  let faceSamplesBusy = $state(false);
  let speakerMatches = $state<Record<string, string>>({});
  let faceSamples = $state<Array<{ id: string; speakerId: string; time: number; thumbnail: string }>>([]);
  let diarizedSourceId = $state("");
  let speakerMatchesConfirmed = $state(false);
  let hfToken = $state("");
  let hfTokenPresent = $state(false);
  let diarizationInstalled = $state(false);
  let diarizationInstallBusy = $state(false);
  let youtubeUrl = $state("");
  let youtubeVideoId = $state("");
  let relatedQuery = $state("");
  let relatedSourceText = $state("");
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

  async function refreshDiarizationStatus() {
    if (!isTauri()) return;
    try {
      const status = await invoke<{ installed: boolean; hasToken: boolean }>("diarization_setup_status");
      diarizationInstalled = status.installed; hfTokenPresent = status.hasToken;
    } catch { diarizationInstalled = false; hfTokenPresent = false; }
  }

  async function runDiarization() {
    const transcript = studioState.transcript;
    if (!isTauri() || !transcript || diarizationBusy || !inputPath || inputPath.startsWith("http")) return;
    if (!hfTokenPresent || !diarizationInstalled) { message = "Save your Hugging Face token and install the optional diarization runtime in Settings first."; return; }
    diarizationBusy = true; message = "Running local speaker diarization…";
    try {
      const intervals = await invoke<unknown>("diarize_video", { path: inputPath, duration: Math.min(meta?.duration ?? transcript.duration, 1800) });
      const updated = applySpeakerIntervals(transcript, intervals);
      studioState.transcript = updated;
      diarizedSourceId = updated.sourceArtifactId;
      speakerMatches = {}; speakerMatchesConfirmed = false; faceSamples = [];
      studioState.speakerFaceMatches = {}; studioState.speakerFaceConfirmed = false;
      await persistProject({ transcript: updated, speakerFaceMatches: null });
      message = `Diarization complete: ${transcriptSpeakerIds(updated).length} speakers found. Review the labels and confirm any face matches.`;
    } catch (error) { message = `Diarization failed: ${String(error)}`; }
    finally { diarizationBusy = false; }
  }

  async function sampleFaces() {
    const transcript = studioState.transcript;
    if (!isTauri() || !transcript || faceSamplesBusy || !inputPath || inputPath.startsWith("http")) return;
    const speakers = transcriptSpeakerIds(transcript);
    if (!speakers.length) return;
    const samples = speakers.slice(0, 8).map((speakerId) => {
      const segment = transcript.segments.find((item) => item.speakerId === speakerId);
      const midpoint = segment ? (segment.start + segment.end) / 2 : transcript.words.find((word) => word.speakerId === speakerId)?.start ?? 0;
      return { speakerId, time: Math.max(0, Math.min(midpoint, Math.max(0, (meta?.duration ?? transcript.duration) - 0.05))) };
    });
    faceSamplesBusy = true; message = "Sampling temporary face crops locally…";
    try {
      const detected = await invoke<Array<{ id: string; speakerId: string; time: number; thumbnail: string }>>("sample_speaker_faces", { path: inputPath, speakers: samples });
      faceSamples = detected.slice(0, 32).map((sample) => ({ ...sample, thumbnail: speakerFaceDataUrl(sample.thumbnail) })).filter((sample) => sample.thumbnail);
      message = faceSamples.length ? `${faceSamples.length} temporary face samples ready. Choose matches, then confirm.` : "No faces were detected in the sampled frames.";
    } catch (error) { message = `Face sampling failed: ${String(error)}`; }
    finally { faceSamplesBusy = false; }
  }

  async function cancelSpeakerMatches() {
    const transcript = studioState.transcript;
    if (transcript) {
      const words = transcript.words.map(({ speakerId: _speakerId, ...word }) => word);
      const segments = transcript.segments.map(({ speakerId: _speakerId, ...segment }) => segment);
      studioState.transcript = { ...transcript, words, segments };
      await persistProject({ transcript: studioState.transcript, speakerFaceMatches: null });
    }
    clearSpeakerState();
    message = "Diarization and face-match changes cancelled.";
  }

  function confirmSpeakerMatches() {
    const transcript = studioState.transcript;
    if (!transcript) return;
    const speakers = transcriptSpeakerIds(transcript);
    const sampleIds = new Set(faceSamples.map((sample) => sample.id));
    const clean: Record<string, string> = {};
    const usedSamples = new Set<string>();
    for (const speakerId of speakers) {
      const selected = speakerMatches[speakerId];
      if (selected && sampleIds.has(selected) && !usedSamples.has(selected) && faceSamples.some((sample) => sample.id === selected && sample.speakerId === speakerId)) { clean[speakerId] = selected; usedSamples.add(selected); }
    }
    speakerMatches = clean; diarizedSourceId = transcript.sourceArtifactId; speakerMatchesConfirmed = true;
    studioState.speakerFaceMatches = clean; studioState.speakerFaceConfirmed = true;
    void persistProject({ speakerFaceMatches: { version: 1, sourceArtifactId: diarizedSourceId, confirmed: true, matches: clean } });
    message = "Speaker labels and face matches confirmed and saved with this project.";
  }

  async function saveHfToken() {
    if (!isTauri() || hfToken.trim().length < 10) return;
    try {
      await invoke("save_diarization_token", { token: hfToken });
      await refreshDiarizationStatus(); message = "Hugging Face token saved locally.";
    } catch (error) { message = `Token save failed: ${String(error)}`; }
  }

  async function installDiarizationRuntime() {
    if (!isTauri() || diarizationInstallBusy) return;
    if (!confirm("Install optional pyannote.audio and OpenCV into the app's local Python environment? This downloads packages from the internet and may use several GB of disk space.")) return;
    diarizationInstallBusy = true; message = "Installing optional local diarization packages…";
    try { await invoke("install_diarization_runtime"); await refreshDiarizationStatus(); message = "Local diarization runtime installed."; }
    catch (error) { message = `Diarization setup failed: ${String(error)}`; }
    finally { diarizationInstallBusy = false; }
  }

  function speakerFaceDataUrl(thumbnail: string): string {
    if (thumbnail.startsWith("data:image/jpeg;base64,")) return thumbnail;
    if (!/^(?:[0-9a-f]{2})+$/i.test(thumbnail)) return "";
    const binary = (thumbnail.match(/.{2}/g) ?? []).map((byte) => String.fromCharCode(parseInt(byte, 16))).join("");
    return `data:image/jpeg;base64,${btoa(binary)}`;
  }

  function clearSpeakerState() {
    speakerMatches = {}; faceSamples = []; diarizedSourceId = ""; speakerMatchesConfirmed = false;
    studioState.speakerFaceMatches = {}; studioState.speakerFaceConfirmed = false;
  }

  async function refreshProjects() {
    if (!isTauri()) return;
    try { projects = await invoke<Project[]>("project_list"); } catch { projects = []; }
  }

  async function persistProject(patch: Partial<Project> = {}) {
    if (!isTauri() || !projectId) return;
    const existing = projects.find((item) => item.id === projectId);
    if (!existing) return;
    const next = { ...existing, sourcePath: inputPath || existing.sourcePath, sourceName: meta?.name ?? existing.sourceName, sourceDuration: meta?.duration ?? existing.sourceDuration, highlights, selectedId: selectedId || existing.selectedId, selectedIds, plans: studioState.plans, variants, selectedVariantId: selectedVariantId || null, speakerFaceMatches: speakerMatchesConfirmed && diarizedSourceId === (studioState.transcript?.sourceArtifactId ?? "") ? { version: 1, sourceArtifactId: diarizedSourceId, confirmed: true, matches: speakerMatches } : null, ...patch };

    try {
      const saved = await invoke<Project>("project_save", { project: next });
      projects = [saved, ...projects.filter((item) => item.id !== saved.id)];
    } catch (error) { message = `Project save failed: ${String(error)}`; }
  }

  async function openProject(id: string) {
    try {
      const project = await invoke<Project>("project_get", { id });
      hydrateProject(project);
    } catch (error) { message = `Could not open project: ${String(error)}`; }
  }

  async function hydrateProject(project: Project) {
    projectId = project.id; inputPath = project.sourcePath; meta = { name: project.sourceName, duration: project.sourceDuration, width: 0, height: 0 };
    await refreshDiarizationStatus();
    faceSamples = []; diarizationBusy = false; faceSamplesBusy = false;
    highlights = project.highlights ?? []; selectedId = project.selectedId ?? highlights[0]?.id ?? ""; selectedIds = project.selectedIds ?? (selectedId ? [selectedId] : []);
    studioState.projectId = project.id; studioState.sourcePath = project.sourcePath; studioState.sourceName = project.sourceName; studioState.sourceDuration = project.sourceDuration;
    studioState.highlights = highlights; studioState.transcript = project.transcript as typeof studioState.transcript; studioState.selectedId = selectedId;
    studioState.plans = project.plans ?? {}; studioState.variants = project.variants ?? []; studioState.selectedVariantId = project.selectedVariantId ?? ""; variants = studioState.variants; selectedVariantId = studioState.selectedVariantId;
    const mapping = project.speakerFaceMatches;
    const mappingMatchesTranscript = mapping?.sourceArtifactId === studioState.transcript?.sourceArtifactId;
    speakerMatches = mappingMatchesTranscript ? mapping?.matches ?? {} : {};
    speakerMatchesConfirmed = mappingMatchesTranscript && !!mapping?.confirmed;
    diarizedSourceId = speakerMatchesConfirmed ? mapping?.sourceArtifactId ?? "" : "";
    studioState.speakerFaceMatches = speakerMatches; studioState.speakerFaceConfirmed = speakerMatchesConfirmed;
    studioState.editingVariantId = ""; studioState.plan = studioState.plans[selectedId] ?? null;
    currentStep = highlights.length ? 4 : 2; message = "Project reopened.";
    if (project.sourcePath && !/^https?:\/\//i.test(project.sourcePath)) {
      try { await invoke("probe_video", { path: project.sourcePath }); }
      catch { message = "Source file is missing or unavailable. Re-link the original file to render."; }
    }
  }

  $effect(() => {
    const id = studioState.projectId;
    if (active && id && id !== projectId) void openProject(id);
  });

  async function ensureProject() {
    if (!isTauri() || projectId || !inputPath) return;
    try {
      const project = await invoke<Project>("project_create", { sourcePath: inputPath, sourceName: meta?.name ?? inputPath.split(/[\\\\/]/).pop() ?? inputPath, sourceDuration: meta?.duration ?? 0 });
      projectId = project.id; studioState.projectId = project.id; projects = [project, ...projects];
    } catch (error) { message = `Could not create project: ${String(error)}`; }
  }

  async function loadVideo(path: string) {
    projectId = ""; studioState.projectId = ""; clearSpeakerState(); studioState.plans = {}; studioState.variants = []; variants = []; selectedVariantId = ""; studioState.selectedVariantId = ""; studioState.editingVariantId = ""; studioState.plan = null; studioState.transcript = null;
    inputPath = path; highlights = []; collections = []; selectedCollection = ""; selectedId = ""; selectedIds = []; jobId = ""; busy = false; meta = null; youtubeVideoId = "";
    studioState.highlights = []; studioState.selectedId = "";
    await reconnectJobs(false);
    try {
      const probe = JSON.parse(await invoke<string>("probe_video", { path }));
      const stream = (probe.streams ?? []).find((s: { codec_type?: string }) => s.codec_type === "video");
      meta = { name: path.split(/[\\/]/).pop() ?? path, duration: Number(probe.format?.duration ?? 0), width: Number(stream?.width ?? 0), height: Number(stream?.height ?? 0) };
      studioState.sourcePath = path; studioState.sourceName = path.split(/[\\/]/).pop() ?? path; studioState.sourceDuration = Number(probe.format?.duration ?? 0);
    } catch { meta = { name: path.split(/[\\/]/).pop() ?? path, duration: 0, width: 0, height: 0 }; studioState.sourcePath = path; studioState.sourceName = path.split(/[\\/]/).pop() ?? path; }
    message = "Video loaded. Set the range and start analysis.";
    await ensureProject();
    await refreshProjects();
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
          const result = JSON.parse(job.result ?? "{}"); highlights = result.highlights ?? []; selectedId = highlights[0]?.id ?? ""; selectedIds = selectedId ? [selectedId] : [];
          collections = result.collections ?? []; selectedCollection = collections[0]?.id ?? ""; variants = []; selectedVariantId = "";
          if (result.videoId) youtubeVideoId = result.videoId;
          studioState.highlights = highlights; studioState.selectedId = selectedId; clearSpeakerState(); studioState.transcript = result.transcript ?? null; studioState.variants = []; studioState.selectedVariantId = ""; studioState.editingVariantId = "";
          if (projectId) {
            const segments = (result.transcript as { segments?: { text?: string }[] } | null)?.segments ?? [];
            relatedSourceText = (segments.map((segment) => segment.text ?? "").join(" ") || result.title || highlights[0]?.title || "").slice(0, 4000);
            relatedQuery = [result.title, highlights[0]?.title, result.topic, ...segments.slice(0, 12).map((segment) => segment.text)].filter(Boolean).join(" ").slice(0, 200);
            await persistProject({ analysisJobId: id, transcript: result.transcript ?? null, highlights, selectedId, variants: [], selectedVariantId: null });
          }

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
    highlights = []; collections = []; selectedCollection = ""; selectedId = ""; clearSpeakerState(); variants = []; studioState.variants = []; selectedVariantId = ""; studioState.selectedVariantId = ""; message = "Analyzing…";
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
    highlights = []; collections = []; selectedCollection = ""; selectedId = ""; selectedIds = []; jobId = ""; youtubeVideoId = ""; meta = null;
    busy = true; message = "Fetching YouTube transcript…"; resetTimer(); timerStartedAt = Date.now();
    try {
      projectId = ""; studioState.projectId = ""; clearSpeakerState(); studioState.plans = {}; studioState.variants = []; variants = []; selectedVariantId = ""; studioState.selectedVariantId = ""; studioState.editingVariantId = ""; studioState.plan = null; studioState.transcript = null;
      const info = await invoke<{ id: string; duration: number; title: string }>("start_youtube_analysis", { url });
      inputPath = url;
      meta = { name: info.title, duration: info.duration, width: 0, height: 0 };
      studioState.sourcePath = url; studioState.sourceName = info.title; studioState.sourceDuration = info.duration;
      const project = await invoke<Project>("project_create", { sourcePath: url, sourceName: info.title, sourceDuration: info.duration });
      projectId = project.id; studioState.projectId = project.id; projects = [project, ...projects];
      youtubeUrl = "";
      await watch(info.id, false);
    } catch (error) {
      message = `YouTube analyze failed: ${String(error)}`;
      busy = false; resetTimer();
    }
  }

  async function generateVariants() {
    if (!isTauri()) { message = "Variant generation is available in the desktop app."; return; }
    if (!studioState.transcript || variantsBusy) return;
    variantsBusy = true; message = "AI is proposing transcript edits…";
    try {
      const result = await invoke<{ promptVersion: string; variants: EditVariant[] }>("generate_transcript_variants", { transcript: studioState.transcript });
      if (result.promptVersion !== VARIANT_PROMPT_VERSION || !result.variants.length) throw new Error("No valid transcript variants were returned.");
      variants = result.variants.map((variant) => {
        const plan = studioState.plans[`variant:${variant.id}`] ?? variant.plan;
        return { ...variant, ...(plan ? { plan } : {}) };
      });
      studioState.variants = variants; selectedVariantId = variants[0]?.id ?? ""; studioState.selectedVariantId = selectedVariantId;
      await persistProject({ variants, selectedVariantId });
      message = `${variants.length} transcript variants ready to compare.`;
    } catch (error) { message = `Variant generation failed: ${String(error)}`; }
    finally { variantsBusy = false; }
  }

  function chooseVariant(id: string) {
    selectedVariantId = id; studioState.selectedVariantId = id;
    void persistProject({ selectedVariantId: id });
  }

  function editVariant(id: string) {
    const variant = variants.find((item) => item.id === id);
    if (!variant) return;
    selectedVariantId = id; studioState.selectedVariantId = id; studioState.editingVariantId = id;
    void persistProject({ selectedVariantId: id });
    window.location.href = "/editor/variant";
  }

  async function renderVariant(id: string) {
    if (!isTauri()) { message = "Rendering runs in the desktop app only."; return; }
    const variant = variants.find((item) => item.id === id);
    const transcript = studioState.transcript;
    if (!variant || !transcript || !inputPath) return;
    busy = true;
    try {
      const plan = variant.plan ?? studioState.plans[`variant:${id}`] ?? createEditorStateFromVariant(transcript, variant).plan;
      variant.plan = plan;
      const output = await save({ defaultPath: `${variant.title.replace(/[^\\w-]+/g, "_") || "variant"}.mp4`, filters: [{ name: "MP4", extensions: ["mp4"] }] });
      if (!output) return;
      await invoke<string>("render_video", { input: inputPath, output, start: plan.ranges[0].start, duration: plan.ranges[0].end - plan.ranges[0].start, aspectRatio: plan.aspectRatio, ranges: plan.ranges, plan });
      outputPath = output; message = `Rendered ${variant.title}: ${output}`;
      const renders = [...(projects.find((item) => item.id === projectId)?.renders ?? []), { output, status: "completed", highlightId: `variant:${id}` }];
      await persistProject({ renders });
    } catch (error) { message = `Render failed for ${variant.title}: ${String(error)}`; }
    finally { busy = false; }
  }

  function chooseCandidate(id: string) {
    selectedId = id; studioState.selectedId = id;
    selectedIds = selectedIds.includes(id) ? selectedIds : [...selectedIds, id];
    void persistProject({ selectedId: id, selectedIds });
  }

  function toggleCandidate(id: string) {
    selectedIds = selectedIds.includes(id) ? selectedIds.filter((selected) => selected !== id) : [...selectedIds, id];
    selectedId = id; studioState.selectedId = id;
    void persistProject({ selectedId: id, selectedIds });
  }

  async function renderSelected() {
    if (!isTauri()) { message = "Rendering runs in the desktop app only."; return; }
    const ids = selectedIds.filter((id) => highlights.some((item) => item.id === id));
    if (!inputPath || !ids.length) return;
    busy = true;
    const renders = [...(projects.find((item) => item.id === projectId)?.renders ?? [])];
    try {
      for (const id of ids) {
        const candidate = highlights.find((item) => item.id === id)!;
        const plan = studioState.plans[id] ?? createEditorState(candidate.sourceArtifactId, meta?.duration ?? candidate.end, candidate).plan;
        const range = plan.ranges[0] ?? { start: candidate.start, end: candidate.end };
        const output = await save({ defaultPath: `${candidate.title.replace(/[^\\w-]+/g, "_") || "short"}.mp4`, filters: [{ name: "MP4", extensions: ["mp4"] }] });
        if (!output) break;
        try {
          await invoke<string>("render_video", { input: inputPath, output, start: range.start, duration: range.end - range.start, aspectRatio: plan.aspectRatio, ranges: plan.ranges, plan });
          renders.push({ output, status: "completed", highlightId: id });
          outputPath = output;
          message = `Rendered ${candidate.title}: ${output}`;
        } catch (error) {
          renders.push({ output, status: `failed: ${String(error)}`, highlightId: id });
          message = `Render failed for ${candidate.title}: ${String(error)}`;
        }
        await persistProject({ renders });
      }
    } finally { busy = false; }
  }

  async function render() {
    if (!selectedIds.includes(selectedId)) selectedIds = [...selectedIds, selectedId];
    await renderSelected();
  }

  async function abort() {
    if (!jobId) { busy = false; resetTimer(); return; }
    try { await invoke("stop_analysis", { id: jobId }); message = "Analysis stopped."; }
    catch (error) { message = `Abort failed: ${String(error)}`; }
    finally { busy = false; resetTimer(); await reconnectJobs(false); }
  }

  function clock(seconds: number) { return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`; }
  onMount(() => {
    void refreshRuntime(); void reconnectJobs(); void refreshProjects(); void refreshDiarizationStatus();
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
      <div class="card speaker-face-match">
        <div class="spread"><div><div class="section-title">Experimental · Match speakers to faces</div><p class="hint">Assign a face sample to each anonymous speaker label. Samples stay temporary; no identity recognition is performed.</p></div><span class="badge" class:ok={speakerMatchesConfirmed}>{speakerMatchesConfirmed ? "confirmed" : "not confirmed"}</span></div>
        {#if inputPath.startsWith("http")}
          <p class="hint">Speaker diarization and local face sampling are available for local video sources only.</p>
        {:else if !studioState.transcript?.words.some((word) => word.speakerId)}
          <div class="stack"><div class="field-row"><button class="btn btn-sm btn-primary" disabled={diarizationBusy || !hfTokenPresent || !diarizationInstalled} onclick={runDiarization}>{diarizationBusy ? "Analyzing speakers…" : "Run local diarization"}</button><span class="note">Requires the optional local runtime and a Hugging Face token for the gated pyannote model.</span></div><p class="hint">Setup: <a href="/settings">Settings</a> → Local diarization. The model's access terms must be accepted on Hugging Face.</p></div>
        {:else}
          <div class="field-row"><button class="btn btn-sm" disabled={faceSamplesBusy} onclick={sampleFaces}>{faceSamplesBusy ? "Sampling frames…" : "Sample faces from video"}</button><button class="btn btn-sm btn-quiet" onclick={cancelSpeakerMatches}>Cancel changes</button></div>
          <div class="speaker-grid">
            {#each transcriptSpeakerIds(studioState.transcript) as speakerId}
              {@const example = speakerExamples(studioState.transcript).find((item) => item.speakerId === speakerId)}
              <section class="speaker-row"><strong>{speakerId}</strong><p class="note">{example?.text ?? "Speaker turn"}</p><label class="field"><span class="field-label">Matching face sample</span><select value={speakerMatches[speakerId] ?? ""} onchange={(event) => { speakerMatches = { ...speakerMatches, [speakerId]: event.currentTarget.value }; speakerMatchesConfirmed = false; studioState.speakerFaceConfirmed = false; }}><option value="">Unknown / no match</option>{#each faceSamples.filter((sample) => sample.speakerId === speakerId) as sample}<option value={sample.id}>Sample at {clock(sample.time)}</option>{/each}</select></label></section>
            {/each}
          </div>
          {#if faceSamples.length}<div class="face-samples">{#each faceSamples as sample}<figure><img src={speakerFaceDataUrl(sample.thumbnail)} alt={`Detected face sample near ${clock(sample.time)}`} /><figcaption>{sample.speakerId} · {clock(sample.time)}</figcaption></figure>{/each}</div>{/if}
          <button class="btn btn-primary" disabled={!transcriptSpeakerIds(studioState.transcript).length} onclick={confirmSpeakerMatches}>Confirm speaker matches</button>
        {/if}
        {#if !hfTokenPresent}<p class="hint">Add a Hugging Face read token in Settings and accept access to pyannote/speaker-diarization-3.1 before running diarization.</p>{/if}
      </div>
      {#if studioState.transcript}
        <div class="card">
          <div class="spread"><div><div class="section-title">Experimental · Transcript edit variants</div><p class="hint">AI suggests ordered source ranges from the transcript. Compare a version, edit it, then export that version only.</p></div><button class="cta small" disabled={variantsBusy || busy} onclick={generateVariants}>{#if variantsBusy}<span class="spinner"></span>{/if}{variantsBusy ? "Generating…" : variants.length ? "Generate again" : "Generate variants"}</button></div>
          {#if variants.length}
            <div class="variant-grid">
              {#each variants as variant}
                {@const plan = variant.plan ?? studioState.plans[`variant:${variant.id}`]}
                {@const duration = (plan?.ranges ?? variant.ranges).reduce((sum, range) => sum + range.end - range.start, 0)}
                <article class="variant-card" class:selected={selectedVariantId === variant.id}>
                  <div class="variant-select" aria-current={selectedVariantId === variant.id ? "true" : undefined}><button class="variant-choice" aria-pressed={selectedVariantId === variant.id} onclick={() => chooseVariant(variant.id)}><strong>{variant.title}</strong><span>{variant.reason ?? "Transcript-selected moments"}</span><small>{variant.ranges.length} ranges · {clock(duration)}</small></button>
                    <ol>{#each variant.ranges as range, index}<li><span>{index + 1}. {clock(range.start)}–{clock(range.end)}</span><small>{(studioState.transcript?.segments ?? []).filter((segment) => segment.end > range.start && segment.start < range.end).map((segment) => segment.text).join(" ")}</small></li>{/each}</ol>
                  </div>
                  <div class="field-row"><button class="btn btn-sm" onclick={() => editVariant(variant.id)}>Edit in Editor</button><button class="btn btn-sm btn-primary" disabled={busy} onclick={() => renderVariant(variant.id)}>Export this version</button></div>
                </article>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
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
            <button class:selected={candidate.id === selectedId} aria-pressed={candidate.id === selectedId} onclick={() => chooseCandidate(candidate.id)}>
              <strong>{candidate.title}</strong>
              <span>{clock(candidate.start)}–{clock(candidate.end)} · rank {highlights.findIndex((item) => item.id === candidate.id) + 1} · score {candidate.score}{#if collectionMembers.has(candidate.id)} · in collection{/if}</span>
              {#if candidate.hook}<small>{candidate.hook}</small>{/if}
            </button>
            <label class="chip"><input type="checkbox" checked={selectedIds.includes(candidate.id)} onchange={() => toggleCandidate(candidate.id)} /> Keep clip</label>
          {/each}
        </div>
        <div class="spec-line"><span>Export {aspectRatio}</span><span>{highlights.length} candidates{collections.length ? ` · ${collections.length} collections` : ""}</span></div>
        <button class="cta secondary" disabled={busy || !selectedIds.length} onclick={renderSelected}>
          {#if busy}<span class="spinner"></span>{/if}
          {busy ? "Rendering…" : `Render ${selectedIds.length} selected clip${selectedIds.length === 1 ? "" : "s"} (${aspectRatio})`}
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
      {#if relatedQuery}
        <RelatedYoutubeSearch initialQuery={relatedQuery} sourceText={relatedSourceText} />
      {/if}
    {:else}
      <div class="card"><p class="hint">No highlights yet. Run an analysis first.</p><button class="ghost wide" onclick={() => currentStep = 2}>← Back to config</button></div>
    {/if}
    {/if}

    <p class="message" aria-live="polite">{message}</p>

    <div class="panel">
      <div class="panel-head"><span class="panel-title"><span class="eyebrow">Recent projects</span></span><span class="badge">{projects.length}</span></div>
      {#if projects.length}{#each projects as project}<div class="row"><div class="row-body"><span class="row-title">{project.title}</span><span class="row-sub">{project.highlights.length} clips · {project.sourceName}</span></div><div class="row-actions"><button class="btn btn-sm" onclick={() => openProject(project.id)}>Open</button></div></div>{/each}{:else}<p class="note">No projects yet — load a local video to create one.</p>{/if}
    </div>
    <div class="panel">
      <div class="panel-head"><span class="panel-title"><span class="eyebrow">Analysis jobs</span></span><span class="badge">{resumableJobs.length}</span></div>
      {#if resumableJobs.length}{#each resumableJobs as job}<div class="row"><div class="row-body"><span class="row-title">{job.id.slice(-8)}</span><span class="row-sub">{job.status} · {job.phase}</span></div><div class="row-actions">{#if job.status === "queued" || job.status === "running"}<button class="btn btn-sm" disabled={busy} onclick={() => watch(job.id, true)}>Continue</button>{/if}{#if job.status === "failed"}<button class="btn btn-sm" disabled={busy} onclick={() => retryJob(job.id)}>Retry</button>{/if}</div></div>{/each}{:else}<p class="note">No analysis jobs yet.</p>{/if}
    </div>
      </div>
    <div class="stage-foot">
      <span>Device: {runtimeStore.device === "cuda" ? "GPU (CUDA)" : "CPU"} · Model: {runtimeStore.model} · License: {runtimeStore.licenseMessage}</span>
      <span title="Safe cleanup only: inspect caches and old renders before removing anything.">Storage: {runtimeStore.storageMessage}</span>
    </div>
    </div>
  </section>
  <style>
    .variant-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,240px),1fr)); gap:.75rem; margin-top:.8rem; }
    .variant-card { border:1px solid var(--line-soft); border-radius:14px; padding:.8rem; background:var(--panel); }
    .variant-card.selected { border-color:var(--volt); box-shadow:0 0 0 1px var(--volt); }
    .variant-select { display:flex; flex-direction:column; align-items:flex-start; gap:.45rem; width:100%; text-align:left; color:inherit; }
    .variant-choice { display:flex; flex-direction:column; align-items:flex-start; gap:.45rem; width:100%; text-align:left; color:inherit; background:none; border:0; padding:0; cursor:pointer; }
    .variant-choice > span,.variant-choice > small { color:var(--ink-2); }
    .variant-select ol { padding-left:1.25rem; margin:.25rem 0; width:100%; }
    .variant-select li { margin:.35rem 0; }
    .variant-select li small { display:block; color:var(--ink-2); }
    .variant-card .field-row { margin-top:.6rem; }
  </style>
</main>
