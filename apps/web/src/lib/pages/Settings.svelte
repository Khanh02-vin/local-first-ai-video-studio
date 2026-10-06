<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { refreshRuntime, runtimeStore } from "../runtime-status.svelte.ts";

let { active = true }: { active?: boolean } = $props();

// Engine/storage/license details come from the shared runtime store (fetched
// once by the layout) — this page reads the cache and only forces a refresh
// on its Refresh button and after changes.
let modelBusy = $state(false);
let message = $state("");
let strategy = $state("heuristic");
let geminiKey = $state("");
let localLlmUrl = $state("http://127.0.0.1:8080");
let localLlmModel = $state("qwen2.5-3b-instruct");
let strategyBusy = $state(false);
type Progress = { phase: string; message: string; done: boolean; error?: string | null };
let whisperProgress = $state<Progress | null>(null);
let whisperBusy = $state(false);
let modelProgress = $state<Progress | null>(null);
let modelPresent = $state(false);
let llmModelBusy = $state(false);
let smallBusy = $state(false);
let smallProgress = $state<Progress | null>(null);
let smallPresent = $state(false);

async function setupWhisper() {
  if (!isTauri()) { message = "Whisper setup runs in the desktop app only."; return; }
  whisperBusy = true;
  try { await invoke("bootstrap_whisper"); await pollWhisper(); }
  catch (error) { message = `Whisper setup failed: ${String(error)}`; whisperBusy = false; }
}
async function pollWhisper() {
  const progress = await invoke<Progress>("whisper_bootstrap_status");
  whisperProgress = progress;
  if (progress?.done) { whisperBusy = false; await refresh(); message = progress.error ? `Whisper setup error: ${progress.error}` : "Whisper is ready."; }
}
async function downloadModel() {
  if (!isTauri()) { message = "Model download runs in the desktop app only."; return; }
  llmModelBusy = true;
  try { await invoke("download_llm_model"); await pollModel(); }
  catch (error) { message = `Model download failed: ${String(error)}`; llmModelBusy = false; }
}
async function pollModel() {
  const status = await invoke<{ progress: Progress | null; modelPresent: boolean }>("model_download_status");
  modelProgress = status.progress; modelPresent = status.modelPresent;
  if (status.progress?.done || modelPresent) { llmModelBusy = false; message = status.progress?.error ? `Model error: ${status.progress.error}` : "Local LLM model ready."; }
}
async function reloadSetupState() {
  try {
    whisperProgress = await invoke<Progress>("whisper_bootstrap_status");
    const status = await invoke<{ progress: Progress | null; modelPresent: boolean }>("model_download_status");
    modelProgress = status.progress; modelPresent = status.modelPresent;
    const small = await invoke<{ progress: Progress | null; modelPresent: boolean }>("whisper_model_download_status");
    smallProgress = small.progress; smallPresent = small.modelPresent;
  } catch { /* browser dev: ignore */ }
}
async function downloadSmall() {
  if (!isTauri()) { message = "Model download runs in the desktop app only."; return; }
  smallBusy = true;
  try { await invoke("download_whisper_model", { model: "small" }); await pollSmall(); }
  catch (error) { message = `Whisper model download failed: ${String(error)}`; smallBusy = false; }
}
async function pollSmall() {
  const status = await invoke<{ progress: Progress | null; modelPresent: boolean }>("whisper_model_download_status");
  smallProgress = status.progress; smallPresent = status.modelPresent;
  if (status.progress?.done || smallPresent) { smallBusy = false; message = status.progress?.error ? `Whisper model error: ${status.progress.error}` : "Whisper small model ready."; }
}

  async function refresh() {
    if (!isTauri()) { message = "Settings control the local engine — open the installed desktop app to view and change them."; return; }
    await refreshRuntime(true);
  }

  async function saveStrategy(next: string, key: string, llamaUrl: string, llamaModel: string) {
    if (!isTauri()) { message = "Strategy changes apply in the desktop app only."; return; }
    strategyBusy = true;
    try {
      strategy = await invoke<string>("set_highlight_strategy", { strategy: next, geminiApiKey: key, localLlmBaseUrl: llamaUrl, localLlmModel: llamaModel });
      if (next === "semantic-gemini") { geminiKey = key; localLlmUrl = ""; localLlmModel = ""; }
      else if (next === "semantic-local") { localLlmUrl = llamaUrl; localLlmModel = llamaModel; geminiKey = ""; }
      else { geminiKey = ""; localLlmUrl = ""; localLlmModel = ""; }
      await refreshRuntime(true);
      const labels: Record<string, string> = {
        "heuristic": "Heuristic strategy saved — offline, deterministic, no cost.",
        "semantic-gemini": "Semantic (Gemini) strategy saved — API key stored locally in runtime.env.",
        "semantic-local": "Local LLM strategy saved — runs offline on this machine (llama.cpp), no API key.",
      };
      message = labels[next] ?? "Strategy saved.";
    } catch (error) { message = `Strategy change failed: ${String(error)}`; }
    finally { strategyBusy = false; }
  }

  async function setModel(next: string) {
    if (!isTauri()) { message = "Model changes apply in the desktop app only."; return; }
    modelBusy = true;
    try { const chosen = await invoke<string>("set_runtime_model", { model: next }); await refreshRuntime(true); message = `Model set to ${chosen}.`; }
    catch (error) { message = `Model change failed: ${String(error)}`; }
    finally { modelBusy = false; }
  }

  async function readStrategy() {
  try {
    const status = await invoke<{ strategy?: string; hasKey?: boolean; hasLocalUrl?: boolean }>("highlight_strategy_status");
    strategy = status.strategy === "semantic-gemini" ? "semantic-gemini" : status.strategy === "semantic-local" ? "semantic-local" : "heuristic";
    geminiKey = status.hasKey ? "••••••••" : "";
    localLlmUrl = status.hasLocalUrl ? localLlmUrl : "http://127.0.0.1:8080";
  } catch { /* not in Tauri (browser dev): keep defaults */ }
}
onMount(refresh);
onMount(readStrategy);
onMount(() => { void reloadSetupState(); });
$effect(() => {
  const setupBusy = whisperBusy || llmModelBusy || smallBusy;
  if (!active || !setupBusy) return;
  const pollTimer = setInterval(() => {
    if (whisperBusy) void pollWhisper();
    if (llmModelBusy) void pollModel();
    if (smallBusy) void pollSmall();
  }, 2000);
  return () => clearInterval(pollTimer);
});
</script>

<h1>Settings</h1>
<main class="settings-page">
  <section class="stage">
    <div class="stage-head">
      <div class="stage-title"><span class="badge volt">settings</span><span class="stage-name">Local engine &amp; preferences</span></div>
      <span class="badge" class:ok={runtimeStore.ready}>{runtimeStore.ready ? "all systems ready" : "engine incomplete"}</span>
    </div>
    <div class="stage-body">
      <div class="settings-wrap stack">

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Runtime</span></div>
          <div class="runtime-row">
            <span class="check"><span class="rail-dot" class:ok={runtimeStore.components.ffmpeg}></span> ffmpeg</span>
            <span class="check"><span class="rail-dot" class:ok={runtimeStore.components.ffprobe}></span> ffprobe</span>
            <span class="check"><span class="rail-dot" class:ok={runtimeStore.components.node}></span> node</span>
            <span class="check"><span class="rail-dot" class:ok={runtimeStore.components.whisper}></span> whisper</span>
            <span class="check"><span class="rail-dot" class:ok={runtimeStore.components.model}></span> model</span>
          </div>
          <p class="note">{runtimeStore.runtimeMessage} · Device: {runtimeStore.device === "cuda" ? "GPU (CUDA)" : "CPU"} · License: {runtimeStore.licenseMessage}</p>
          <p class="note">Bundled binaries: Node {runtimeStore.bundled.node ? "✓" : "—"} · FFmpeg {runtimeStore.bundled.ffmpeg ? "✓" : "—"} · ffprobe {runtimeStore.bundled.ffprobe ? "✓" : "—"}</p>
          <div class="filters-row">
            <button class="btn btn-sm btn-quiet" onclick={refresh}>Refresh</button>
            <button class="btn btn-sm btn-quiet" onclick={readStrategy}>Reload strategy</button>
          </div>
          {#if message}<p class="hint" aria-live="polite">{message}</p>{/if}
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Transcription model</span></div>
          <div class="seg">
            {#each ["tiny", "base", "small"] as m}<button class="seg-item" class:on={runtimeStore.model === m} disabled={modelBusy} onclick={() => setModel(m)}>{m}</button>{/each}
          </div>
          <p class="note">tiny = CPU-friendly (free tier) · base/small need a Pro license · GPU makes all faster. Current: {runtimeStore.model} on {runtimeStore.device === "cuda" ? "GPU" : "CPU"}.</p>
        {#if runtimeStore.model === "small"}
          <div class="spread" style="border-top:1px solid var(--line-soft);padding-top:.6rem;">
            <span class="note" style="margin:0;">Whisper small — ~460 MB, tải 1 lần rồi dùng offline (không bundle trong installer).</span>
            <button class="btn btn-sm" disabled={smallBusy || smallPresent} onclick={downloadSmall}>{smallPresent ? "Downloaded ✓" : smallBusy ? "Downloading…" : "Download small (~460 MB)"}</button>
          </div>
          {#if smallProgress}<p class="hint" aria-live="polite">{smallProgress.message}{smallProgress.error ? ` — ${smallProgress.error}` : ""}</p>{/if}
        {/if}
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">First-run setup</span></div>
          <div class="spread"><span class="note" style="margin:0;">Whisper — installs openai-whisper + PyTorch into the app's own environment (one time, a few GB).</span><button class="btn btn-sm" disabled={whisperBusy} onclick={setupWhisper}>{whisperBusy ? "Setting up…" : "Setup Whisper"}</button></div>
          {#if whisperProgress}<p class="hint" aria-live="polite">{whisperProgress.message}{whisperProgress.error ? ` — ${whisperProgress.error}` : ""}</p>{:else}<p class="note">Status unknown — press Setup to check.</p>{/if}
          <div class="spread" style="border-top:1px solid var(--line-soft);padding-top:.6rem;">
            <span class="note" style="margin:0;">Local LLM — Qwen2.5-3B GGUF (~2.1 GB), checksum-verified; only needed for the "Local LLM" strategy.</span><button class="btn btn-sm" disabled={llmModelBusy || modelPresent} onclick={downloadModel}>{modelPresent ? "Downloaded ✓" : llmModelBusy ? "Downloading…" : "Download model"}</button>
          </div>
          {#if modelPresent}<p class="note">Model present on disk.</p>{:else if modelProgress}<p class="hint" aria-live="polite">{modelProgress.message}{modelProgress.error ? ` — ${modelProgress.error}` : ""}</p>{/if}
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Highlight strategy</span></div>
          <div class="stack">
            <button class="strategy-card" class:on={strategy === "heuristic"} disabled={strategyBusy} onclick={() => saveStrategy("heuristic", "", "", "")}>
              <span class="spread"><strong>Heuristic (offline)</strong>{#if strategy === "heuristic"}<span class="badge ok">active</span>{/if}</span>
              <p>Score segments by speech density + keywords. No API key, works offline.</p>
            </button>
            <button class="strategy-card" class:on={strategy === "semantic-gemini"} disabled={strategyBusy} onclick={() => saveStrategy("semantic-gemini", geminiKey, localLlmUrl, localLlmModel)}>
              <span class="spread"><strong>Semantic (Gemini BYOK)</strong>{#if strategy === "semantic-gemini"}<span class="badge ok">active</span>{/if}</span>
              <p>Use your Gemini API key to rank segments semantically.</p>
            </button>
            <button class="strategy-card" class:on={strategy === "semantic-local"} disabled={strategyBusy} onclick={() => saveStrategy("semantic-local", geminiKey, localLlmUrl, localLlmModel)}>
              <span class="spread"><strong>Local LLM (offline)</strong>{#if strategy === "semantic-local"}<span class="badge ok">active</span>{/if}</span>
              <p>Rank with the bundled Qwen model via llama.cpp — no key.</p>
            </button>
          </div>
          {#if strategy === "semantic-gemini"}
            <label class="field"><span class="field-label">Gemini API key</span><input type="password" value={geminiKey} oninput={(e) => geminiKey = e.currentTarget.value} placeholder="AIza..." /></label>
            <button class="btn btn-sm" disabled={strategyBusy || geminiKey.length < 10} onclick={() => saveStrategy("semantic-gemini", geminiKey, localLlmUrl, localLlmModel)}>Save key</button>
          {/if}
          {#if strategy === "semantic-local"}
            <label class="field"><span class="field-label">Local LLM URL</span><input type="text" value={localLlmUrl} oninput={(e) => localLlmUrl = e.currentTarget.value} placeholder="http://127.0.0.1:8080" /></label>
            <label class="field"><span class="field-label">Model</span><input type="text" value={localLlmModel} oninput={(e) => localLlmModel = e.currentTarget.value} placeholder="qwen2.5-3b-instruct" /></label>
            <button class="btn btn-sm" disabled={strategyBusy || localLlmUrl.trim().length < 10} onclick={() => saveStrategy("semantic-local", geminiKey, localLlmUrl, localLlmModel)}>Save</button>
          {/if}
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Storage</span></div>
          <p class="note" style="margin:0;">{runtimeStore.storageMessage}</p>
          <details><summary>Safe cleanup guidance</summary><p class="hint">Không tự động xóa dữ liệu. Kiểm tra:</p><pre>df -h $HOME /tmp
df -i $HOME /tmp
du -h --max-depth=1 ~/.cache 2>/dev/null | sort -h
cargo clean</pre><p class="hint">Chỉ xóa pip cache, Rust build artifacts hoặc output cũ khi bạn xác nhận.</p></details>
        </div>

      </div>
    </div>
    <div class="stage-foot">
      <span>Local-first AI Video Studio — Whisper transcription (MIT), FFmpeg encoding, Tauri shell.</span>
      <a href="/">← Back to Studio</a>
    </div>
  </section>
</main>
