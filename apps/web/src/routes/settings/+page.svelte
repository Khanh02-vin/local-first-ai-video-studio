<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

let model = $state("tiny");
let device = $state("cpu");
let runtimeReady = $state(false);
let runtimeMessage = $state("Checking…");
let storageMessage = $state("…");
let licenseMessage = $state("…");
let modelBusy = $state(false);
let message = $state("");
let bundled = $state({ node: false, ffmpeg: false, ffprobe: false });
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
    try {
      const runtime = await invoke<{ ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; modelReady: boolean; device?: string; model?: string; bundled?: { node: boolean; ffmpeg: boolean; ffprobe: boolean } }>("runtime_status");
      const storage = await invoke<{ home: { ready: boolean; freeBytes?: number; requiredBytes: number }; temp: { ready: boolean; freeBytes?: number } }>("storage_status");
      const license = await invoke<{ licensed: boolean; tier?: string; licensee?: string | null; error?: string; configured?: boolean }>("license_status");
      runtimeReady = runtime.ffmpeg && runtime.ffprobe && runtime.node && runtime.whisper && runtime.modelReady && storage.home.ready && storage.temp.ready;
      device = runtime.device ?? "cpu"; model = runtime.model ?? "tiny";
      bundled = runtime.bundled ?? bundled;
      runtimeMessage = runtimeReady ? "All local engine components ready." : "Some component missing — see below.";
      storageMessage = storage.home.ready ? "Enough free space." : "Free space is low.";
      licenseMessage = license.licensed ? `Active · ${license.tier}${license.licensee ? ` · ${license.licensee}` : ""}` : (license.error ?? "Free tier (unlicensed)");
    } catch (error) { message = `Refresh failed: ${String(error)}`; }
  }

  async function saveStrategy(next: string, key: string, llamaUrl: string, llamaModel: string) {
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

<section class="card">
  <h2>Engine</h2>
  <dl class="specs">
    <dt>Runtime</dt><dd>{runtimeMessage}</dd>
    <dt>Device</dt><dd>{device === "cuda" ? "GPU (CUDA)" : "CPU"}</dd>
    <dt>License</dt><dd>{licenseMessage}</dd>
  </dl>
  <p class="hint">Bundled binaries: Node {bundled.node ? "✓" : "—"} · FFmpeg {bundled.ffmpeg ? "✓" : "—"} · ffprobe {bundled.ffprobe ? "✓" : "—"}</p>
  <button onclick={refresh}>Refresh</button>
  <button onclick={readStrategy} class="hint-btn">Reload strategy</button>
</section>

<section class="card">
  <h2>Whisper (transcription)</h2>
  <p class="hint">First run on a clean machine: the app creates its own Python environment and installs openai-whisper (downloads PyTorch, a few GB, one time).</p>
  {#if whisperProgress}
    <p aria-live="polite">{whisperProgress.message}{whisperProgress.error ? ` — ${whisperProgress.error}` : ""}</p>
  {:else}
    <p class="hint">Status unknown — press Setup to check.</p>
  {/if}
  <button class="chip" disabled={whisperBusy} onclick={setupWhisper}>{whisperBusy ? "Setting up…" : "Setup Whisper"}</button>
</section>

<section class="card">
  <h2>Local LLM model</h2>
  <p class="hint">Offline highlight strategy needs the Qwen2.5-3B GGUF (~2.1 GB). Downloaded once, checksum-verified, stored on this machine.</p>
  {#if modelPresent}
    <p class="hint">Model present on disk.</p>
  {:else if modelProgress}
    <p aria-live="polite">{modelProgress.message}{modelProgress.error ? ` — ${modelProgress.error}` : ""}</p>
  {:else}
    <p class="hint">Not downloaded yet.</p>
  {/if}
  <button class="chip" disabled={llmModelBusy || modelPresent} onclick={downloadModel}>{modelPresent ? "Downloaded" : llmModelBusy ? "Downloading…" : "Download model (2.1 GB)"}</button>
</section>

<section class="card">
  <h2>AI model</h2>
  <p class="hint">tiny = CPU-friendly (free tier) · base/small need a Pro license · GPU makes all faster.</p>
  <div class="model-row">
    {#each ["tiny", "base", "small"] as m}<button class="chip" class:sel={model === m} disabled={modelBusy} onclick={() => setModel(m)}>{m}</button>{/each}
  </div>
  <p class="hint">Current: {model} on {device === "cuda" ? "GPU" : "CPU"}.</p>
  {#if message}<p aria-live="polite">{message}</p>{/if}
</section>

<section class="card">
  <h2>Highlight strategy</h2>
  <p class="hint">Heuristic = offline, deterministic, no cost. Semantic = LLM proposes clips by meaning (Gemini needs your API key; Local LLM needs llama.cpp running, no key).</p>
  <div class="model-row">
    <button class="chip" class:sel={strategy === "heuristic"} disabled={strategyBusy} onclick={() => saveStrategy("heuristic", "", "", "")}>Heuristic (offline)</button>
    <button class="chip" class:sel={strategy === "semantic-gemini"} disabled={strategyBusy} onclick={() => saveStrategy("semantic-gemini", geminiKey, localLlmUrl, localLlmModel)}>Semantic (Gemini BYOK)</button>
    <button class="chip" class:sel={strategy === "semantic-local"} disabled={strategyBusy} onclick={() => saveStrategy("semantic-local", geminiKey, localLlmUrl, localLlmModel)}>Local LLM (offline)</button>
  </div>
  {#if strategy === "semantic-gemini"}
    <label class="hint">Gemini API key <input type="password" value={geminiKey} oninput={(e) => geminiKey = e.currentTarget.value} placeholder="AIza..." /></label>
    <button class="chip" disabled={strategyBusy || geminiKey.length < 10} onclick={() => saveStrategy("semantic-gemini", geminiKey, localLlmUrl, localLlmModel)}>Save key</button>
  {/if}
  {#if strategy === "semantic-local"}
    <label class="hint">Local LLM URL <input type="text" value={localLlmUrl} oninput={(e) => localLlmUrl = e.currentTarget.value} placeholder="http://127.0.0.1:8080" /></label>
    <label class="hint">Model <input type="text" value={localLlmModel} oninput={(e) => localLlmModel = e.currentTarget.value} placeholder="qwen2.5-3b-instruct" /></label>
    <button class="chip" disabled={strategyBusy || localLlmUrl.trim().length < 10} onclick={() => saveStrategy("semantic-local", geminiKey, localLlmUrl, localLlmModel)}>Save</button>
  {/if}
  <p class="hint">Current: {strategy}{strategy === "semantic-gemini" ? (geminiKey ? " · key stored" : " · no key yet") : strategy === "semantic-local" ? " · local LLM" : ""}.</p>
</section>

<section class="card">
  <h2>Storage</h2>
  <p>{storageMessage}</p>
  <details><summary>Safe cleanup guidance</summary><p class="hint">Không tự động xóa dữ liệu. Kiểm tra:</p><pre>df -h $HOME /tmp
df -i $HOME /tmp
du -h --max-depth=1 ~/.cache 2>/dev/null | sort -h
cargo clean</pre><p class="hint">Chỉ xóa pip cache, Rust build artifacts hoặc output cũ khi bạn xác nhận.</p></details>
</section>

<section class="card">
  <h2>About</h2>
  <p class="hint">Local-first AI Video Studio — Whisper transcription (MIT), FFmpeg encoding, Tauri shell.</p>
  <a href="/">← Back to Studio</a>
</section>