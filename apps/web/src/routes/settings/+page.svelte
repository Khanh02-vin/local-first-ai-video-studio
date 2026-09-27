<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";

let model = $state("tiny");
let device = $state("cpu");
let runtimeReady = $state(false);
let runtimeMessage = $state("Checking…");
let storageMessage = $state("…");
let licenseMessage = $state("…");
let modelBusy = $state(false);
let message = $state("");
let bundled = $state({ node: false, ffmpeg: false, ffprobe: false });
let components = $state({ ffmpeg: false, ffprobe: false, node: false, whisper: false, model: false });
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
let pollTimer: ReturnType<typeof setInterval> | undefined;

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
  } catch { /* browser dev: ignore */ }
}

  async function refresh() {
    if (!isTauri()) { message = "Settings control the local engine — open the installed desktop app to view and change them."; return; }
    try {
      const runtime = await invoke<{ ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; modelReady: boolean; device?: string; model?: string; bundled?: { node: boolean; ffmpeg: boolean; ffprobe: boolean } }>("runtime_status");
      const storage = await invoke<{ home: { ready: boolean; freeBytes?: number; requiredBytes: number }; temp: { ready: boolean; freeBytes?: number } }>("storage_status");
      const license = await invoke<{ licensed: boolean; tier?: string; licensee?: string | null; error?: string; configured?: boolean }>("license_status");
      runtimeReady = runtime.ffmpeg && runtime.ffprobe && runtime.node && runtime.whisper && runtime.modelReady && storage.home.ready && storage.temp.ready;
      components = { ffmpeg: runtime.ffmpeg, ffprobe: runtime.ffprobe, node: runtime.node, whisper: runtime.whisper, model: runtime.modelReady };
      device = runtime.device ?? "cpu"; model = runtime.model ?? "tiny";
      bundled = runtime.bundled ?? bundled;
      runtimeMessage = runtimeReady ? "All local engine components ready." : "Some component missing — see below.";
      storageMessage = storage.home.ready ? "Enough free space." : "Free space is low.";
      licenseMessage = license.licensed ? `Active · ${license.tier}${license.licensee ? ` · ${license.licensee}` : ""}` : (license.error ?? "Free tier (unlicensed)");
    } catch (error) { message = `Refresh failed: ${String(error)}`; }
  }

  async function saveStrategy(next: string, key: string, llamaUrl: string, llamaModel: string) {
    if (!isTauri()) { message = "Strategy changes apply in the desktop app only."; return; }
    strategyBusy = true;
    try {
      strategy = await invoke<string>("set_highlight_strategy", { strategy: next, geminiApiKey: key, localLlmBaseUrl: llamaUrl, localLlmModel: llamaModel });
      if (next === "semantic-gemini") { geminiKey = key; localLlmUrl = ""; localLlmModel = ""; }
      else if (next === "semantic-local") { localLlmUrl = llamaUrl; localLlmModel = llamaModel; geminiKey = ""; }
      else { geminiKey = ""; localLlmUrl = ""; localLlmModel = ""; }
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
    try { model = await invoke<string>("set_runtime_model", { model: next }); message = `Model set to ${model}.`; }
    catch (error) { message = `Model change failed: ${String(error)}`; }
    finally { modelBusy = false; }
  }

  function formatBytes(bytes: number) { return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`; }
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
onMount(() => {
  void reloadSetupState();
  // Poll while a long-running first-run setup is in flight.
  pollTimer = setInterval(() => {
    if (whisperBusy) void pollWhisper();
    if (llmModelBusy) void pollModel();
  }, 2000);
  return () => { if (pollTimer) clearInterval(pollTimer); };
});
</script>

<h1>Settings</h1>
<main class="settings-page">
  <section class="stage">
    <div class="stage-head">
      <div class="stage-title"><span class="badge volt">settings</span><span class="stage-name">Local engine &amp; preferences</span></div>
      <span class="badge" class:ok={runtimeReady}>{runtimeReady ? "all systems ready" : "engine incomplete"}</span>
    </div>
    <div class="stage-body">
      <div class="settings-wrap stack">

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Runtime</span></div>
          <div class="runtime-row">
            <span class="check"><span class="rail-dot" class:ok={components.ffmpeg}></span> ffmpeg</span>
            <span class="check"><span class="rail-dot" class:ok={components.ffprobe}></span> ffprobe</span>
            <span class="check"><span class="rail-dot" class:ok={components.node}></span> node</span>
            <span class="check"><span class="rail-dot" class:ok={components.whisper}></span> whisper</span>
            <span class="check"><span class="rail-dot" class:ok={components.model}></span> model</span>
          </div>
          <p class="note">{runtimeMessage} · Device: {device === "cuda" ? "GPU (CUDA)" : "CPU"} · License: {licenseMessage}</p>
          <p class="note">Bundled binaries: Node {bundled.node ? "✓" : "—"} · FFmpeg {bundled.ffmpeg ? "✓" : "—"} · ffprobe {bundled.ffprobe ? "✓" : "—"}</p>
          <div class="filters-row">
            <button class="btn btn-sm btn-quiet" onclick={refresh}>Refresh</button>
            <button class="btn btn-sm btn-quiet" onclick={readStrategy}>Reload strategy</button>
          </div>
          {#if message}<p class="hint" aria-live="polite">{message}</p>{/if}
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Transcription model</span></div>
          <div class="seg">
            {#each ["tiny", "base", "small"] as m}<button class="seg-item" class:on={model === m} disabled={modelBusy} onclick={() => setModel(m)}>{m}</button>{/each}
          </div>
          <p class="note">tiny = CPU-friendly (free tier) · base/small need a Pro license · GPU makes all faster. Current: {model} on {device === "cuda" ? "GPU" : "CPU"}.</p>
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
          <p class="note" style="margin:0;">{storageMessage}</p>
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
