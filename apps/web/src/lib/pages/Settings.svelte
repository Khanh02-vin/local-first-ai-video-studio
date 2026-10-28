<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { refreshRuntime, runtimeStore, downloadWhisperCppModel, refreshWhisperCppModels, setupWhisperCpp, smokeWhisperCpp, startWhisperBootstrap, stopBootstrapPoll } from "../runtime-status.svelte.ts";

let { active = true }: { active?: boolean } = $props();

// Engine/storage/license details come from the shared runtime store (fetched
// once by the layout) — this page reads the cache and only forces a refresh
// on its Refresh button and after changes.
let modelBusy = $state(false);
let message = $state("");
let strategy = $state("heuristic");
  let localLlmUrl = $state("http://127.0.0.1:8080");
  let localLlmModel = $state("qwen2.5-3b-instruct");
  let strategyBusy = $state(false);
  type AiProvider = { id: string; name: string; baseUrl: string; model: string; hasKey: boolean };
  let providers = $state<AiProvider[]>([]);
  let activeProviderId = $state("");
  let providerFormOpen = $state(false);
  let editingProviderId = $state("");
  let providerName = $state("");
  let providerBaseUrl = $state("");
  let providerApiKey = $state("");
  let providerModel = $state("");
  const suggestedModels = ["gemini-2.0-flash", "gemini-2.5-flash", "gpt-4o-mini", "gpt-4.1-mini", "deepseek-chat", "qwen-plus", "kimi-k2"];
type Progress = { phase: string; message: string; done: boolean; error?: string | null };
let whisperProgress = $state<Progress | null>(null);
let whisperBusy = $state(false);
let modelProgress = $state<Progress | null>(null);
let modelPresent = $state(false);
let llamaRuntimePresent = $state(false);
let llmSetupBusy = $state(false);
let smallBusy = $state(false);
let smallProgress = $state<Progress | null>(null);
let smallPresent = $state(false);
let ggmlBusy = $state("");
let hfToken = $state("");
let diarizationInstalled = $state(false);
let hfTokenPresent = $state(false);
let diarizationBusy = $state(false);

// System Doctor state
let doctorBusy = $state(false);
let doctorMessage = $state("");

// System Doctor: one-click environment check + fix
  async function downloadGgml(model: "base" | "small-q5_1") {
    if (ggmlBusy) return;
    ggmlBusy = model;
    try { await downloadWhisperCppModel(model); doctorMessage = `Whisper.cpp ${model} model ready.`; }
    catch (error) { doctorMessage = `Model download failed: ${String(error)}`; }
    finally { ggmlBusy = ""; }
  }

  async function runSystemDoctor() {
    if (!isTauri() || doctorBusy) return;
    doctorBusy = true;
    doctorMessage = "Running system check…";
    try {
      const status = await invoke<{ ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; modelReady: boolean; device?: string; model?: string }>("runtime_status");
      // Refresh the shared store
      await refreshRuntime(true);
      // Build a summary message
      const components = runtimeStore.components;
      const missing: string[] = [];
      if (!components.ffmpeg) missing.push("ffmpeg");
      if (!components.ffprobe) missing.push("ffprobe");
      if (!components.node) missing.push("node");
      if (!components.whisper) missing.push("whisper");
      if (!components.model) missing.push("model");
      if (missing.length === 0) {
        doctorMessage = "✓ All components ready. Engine is fully operational.";
      } else {
        doctorMessage = `Missing: ${missing.join(", ")}. Use the buttons below to install.`;
      }
    } catch (error) {
      doctorMessage = `System check failed: ${String(error)}`;
    } finally {
      doctorBusy = false;
    }
  }

  async function installWhisperNow() {
    if (!isTauri() || doctorBusy) return;
    doctorBusy = true;
    doctorMessage = "Starting Whisper bootstrap…";
    try {
      await startWhisperBootstrap();
      doctorMessage = "Whisper bootstrap started. Progress will appear below.";
    } catch (error) {
      doctorMessage = `Failed to start Whisper bootstrap: ${String(error)}`;
      doctorBusy = false;
    }
  }

  async function smokeWhisperCppNow() {
    if (!isTauri() || doctorBusy || !runtimeStore.whisperCpp) return;
    doctorBusy = true;
    doctorMessage = "Running real Whisper.cpp CPU smoke test…";
    try { await smokeWhisperCpp(); doctorMessage = "Whisper.cpp CPU/model smoke test passed."; }
    catch (error) { doctorMessage = `Whisper.cpp smoke test failed: ${String(error)}`; }
    finally { doctorBusy = false; }
  }

  async function setupWhisperCppNow() {
    if (!isTauri() || doctorBusy) return;
    doctorBusy = true;
    doctorMessage = "Activating Whisper.cpp — no Python required…";
    try {
      await setupWhisperCpp();
      doctorMessage = "Whisper.cpp is ready. Analysis now runs without Python.";
    } catch (error) {
      doctorMessage = `Whisper.cpp setup failed: ${String(error)}`;
    } finally { doctorBusy = false; }
  }

  async function stopWhisperBootstrap() {
    stopBootstrapPoll();
    doctorBusy = false;
    doctorMessage = "Whisper bootstrap polling stopped.";
  }

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
async function setupLocalLlm() {
  if (!isTauri()) { message = "Local LLM setup runs in the desktop app only."; return; }
  if (!confirm("Download the local LLM runtime and model? This needs an internet connection and about 2.2 GB of disk space. It is optional; Heuristic mode works without it.")) return;
  llmSetupBusy = true;
  try { await invoke("install_local_llm"); await pollLocalLlm(); }
  catch (error) { message = `Local LLM setup failed: ${String(error)}`; llmSetupBusy = false; }
}
async function pollLocalLlm() {
  const status = await invoke<{ progress: Progress | null; runtimePresent: boolean; modelPresent: boolean }>("local_llm_setup_status");
  modelProgress = status.progress; llamaRuntimePresent = status.runtimePresent; modelPresent = status.modelPresent;
  if (status.progress?.done) {
    llmSetupBusy = false;
    message = status.progress.error ? `Local LLM setup error: ${status.progress.error}` : "Local LLM runtime and model are ready.";
  }
}
async function refreshDiarization() {
  if (!isTauri()) return;
  try {
    const status = await invoke<{ installed: boolean; hasToken: boolean }>("diarization_setup_status");
    diarizationInstalled = status.installed; hfTokenPresent = status.hasToken;
  } catch { diarizationInstalled = false; hfTokenPresent = false; }
}
async function saveHfToken() {
  if (!isTauri() || hfToken.trim().length < 10) return;
  try { await invoke("save_diarization_token", { token: hfToken }); hfToken = ""; await refreshDiarization(); message = "Hugging Face token saved locally."; }
  catch (error) { message = `Token save failed: ${String(error)}`; }
}
async function setupDiarization() {
  if (!isTauri() || diarizationBusy) return;
  if (!confirm("Install optional pyannote.audio and OpenCV into the app's local Python environment? This downloads packages from the internet and may use several GB of disk space.")) return;
  diarizationBusy = true;
  try { await invoke("install_diarization_runtime"); await refreshDiarization(); message = "Local diarization runtime installed."; }
  catch (error) { message = `Diarization setup failed: ${String(error)}`; }
  finally { diarizationBusy = false; }
}
async function reloadSetupState() {
  try {
    whisperProgress = await invoke<Progress>("whisper_bootstrap_status");
    const status = await invoke<{ progress: Progress | null; runtimePresent: boolean; modelPresent: boolean }>("local_llm_setup_status");
    modelProgress = status.progress; llamaRuntimePresent = status.runtimePresent; modelPresent = status.modelPresent;
    const small = await invoke<{ progress: Progress | null; modelPresent: boolean }>("whisper_model_download_status");
    smallProgress = small.progress; smallPresent = small.modelPresent;
    await refreshDiarization();
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

  async function loadAiProviders() {
    if (!isTauri()) return;
    try { const result = await invoke<{providers: AiProvider[]; activeProviderId: string | null}>("list_ai_providers"); providers = result.providers; activeProviderId = result.activeProviderId ?? ""; }
    catch (error) { message = `Provider list failed: ${String(error)}`; }
  }
  function openProviderForm(provider?: AiProvider) {
    editingProviderId = provider?.id ?? ""; providerName = provider?.name ?? ""; providerBaseUrl = provider?.baseUrl ?? "https://api.openai.com/v1"; providerApiKey = ""; providerModel = provider?.model ?? ""; providerFormOpen = true;
  }
  async function saveProvider() {
    strategyBusy = true;
    try {
      const id = await invoke<string>("save_ai_provider", { id: editingProviderId || null, name: providerName, baseUrl: providerBaseUrl, apiKey: providerApiKey, model: providerModel });
      await loadAiProviders(); providerFormOpen = false; providerApiKey = ""; message = "Provider saved locally.";
      if (!activeProviderId) await activateProvider(id);
    } catch (error) { message = `Provider save failed: ${String(error)}`; }
    finally { strategyBusy = false; }
  }
  async function activateProvider(id: string) {
    strategyBusy = true;
    try { activeProviderId = await invoke<string>("activate_ai_provider", { id }); strategy = "semantic-openai"; await loadAiProviders(); await refreshRuntime(true); message = "Cloud provider activated."; }
    catch (error) { message = `Provider activation failed: ${String(error)}`; }
    finally { strategyBusy = false; }
  }
  async function removeProvider(provider: AiProvider) {
    if (!confirm(`Delete provider “${provider.name}”?`)) return;
    try { await invoke("delete_ai_provider", { id: provider.id }); if (activeProviderId === provider.id) { activeProviderId = ""; strategy = "heuristic"; } await loadAiProviders(); await refreshRuntime(true); message = "Provider deleted."; }
    catch (error) { message = `Provider delete failed: ${String(error)}`; }
  }
  async function saveStrategy(next: string, _key: string, llamaUrl: string, llamaModel: string) {
    if (!isTauri()) { message = "Strategy changes apply in the desktop app only."; return; }
    if (next === "semantic-local" && !(modelPresent && llamaRuntimePresent)) { message = "Install the Local LLM runtime and model below before selecting this strategy."; return; }
    strategyBusy = true;
    try {
      strategy = await invoke<string>("set_highlight_strategy", { strategy: next, geminiApiKey: "", localLlmBaseUrl: llamaUrl, localLlmModel: llamaModel });
      await refreshRuntime(true);
      message = next === "heuristic" ? "Heuristic strategy saved — offline, deterministic, no cost." : "Local LLM strategy saved — runs offline on this machine (llama.cpp), no API key.";
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
    strategy = status.strategy === "semantic-openai" ? "semantic-openai" : status.strategy === "semantic-local" ? "semantic-local" : "heuristic";
    localLlmUrl = status.hasLocalUrl ? localLlmUrl : "http://127.0.0.1:8080";
  } catch { /* not in Tauri (browser dev): keep defaults */ }
}
onMount(refresh);
onMount(readStrategy);
onMount(loadAiProviders);
onMount(() => { void reloadSetupState(); void refreshWhisperCppModels(); });
$effect(() => {
  const setupBusy = whisperBusy || llmSetupBusy || smallBusy;
  if (!active || !setupBusy) return;
  const pollTimer = setInterval(() => {
    if (whisperBusy) void pollWhisper();
    if (llmSetupBusy) void pollLocalLlm();
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
      <div class="stack">

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
          <div class="panel-title"><span class="eyebrow">Legacy Python fallback</span></div>
          <div class="spread"><span class="note" style="margin:0;">Deprecated: openai-whisper/PyTorch Python venv is retained only for existing installations. New setups should use the bundled Whisper.cpp engine above.</span><button class="btn btn-sm btn-quiet" disabled={whisperBusy} onclick={setupWhisper}>{whisperBusy ? "Setting up…" : "Use legacy Python fallback"}</button></div>
          {#if whisperProgress}<p class="hint" aria-live="polite">{whisperProgress.message}{whisperProgress.error ? ` — ${whisperProgress.error}` : ""}</p>{:else}<p class="note">Status unknown — press Setup to check.</p>{/if}
          <div class="spread" style="border-top:1px solid var(--line-soft);padding-top:.6rem;">
            <span class="note" style="margin:0;">Local LLM — optional llama.cpp runtime and Qwen2.5-3B GGUF model (~2.2 GB total); downloads on request and verifies checksums.</span><button class="btn btn-sm" disabled={llmSetupBusy || (modelPresent && llamaRuntimePresent)} onclick={setupLocalLlm}>{modelPresent && llamaRuntimePresent ? "Ready ✓" : llmSetupBusy ? "Installing…" : "Install Local LLM"}</button>
          </div>
          {#if modelProgress}<p class="hint" aria-live="polite">{modelProgress.message}{modelProgress.error ? ` — ${modelProgress.error}` : ""}</p>{:else}<p class="note">Runtime: {llamaRuntimePresent ? "installed" : "not installed"} · Model: {modelPresent ? "downloaded" : "not downloaded"}. Only needed for the Local LLM highlight strategy.</p>{/if}
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Local diarization &amp; face matching</span><span class="badge" class:ok={diarizationInstalled && hfTokenPresent}>{diarizationInstalled && hfTokenPresent ? "ready" : "optional"}</span></div>
          <p class="note">Speaker diarization runs locally with pyannote.audio. Face samples use local OpenCV detection only; no identity recognition or face embeddings are created.</p>
          <label class="field"><span class="field-label">Hugging Face read token {hfTokenPresent ? "(saved locally)" : ""}</span><input type="password" autocomplete="new-password" bind:value={hfToken} placeholder={hfTokenPresent ? "Token saved — enter a new one to replace" : "hf_…"} /></label>
          <button class="btn btn-sm" disabled={hfToken.trim().length < 10} onclick={saveHfToken}>Save token locally</button>
          <p class="hint">Before use, visit the <a href="https://huggingface.co/pyannote/speaker-diarization-3.1" target="_blank" rel="noreferrer">pyannote/speaker-diarization-3.1 model page</a> and accept its access conditions. The token is kept in this app's local runtime settings and passed only to the local diarization process.</p>
          <div class="spread"><span class="note">Runtime: {diarizationInstalled ? "installed" : "not installed"} · Token: {hfTokenPresent ? "saved" : "not saved"}</span><button class="btn btn-sm" disabled={diarizationBusy || diarizationInstalled} onclick={setupDiarization}>{diarizationInstalled ? "Installed ✓" : diarizationBusy ? "Installing…" : "Install optional runtime"}</button></div>
          {#if !runtimeStore.components.whisper}<p class="hint">Set up the app's Python/Whisper environment first.</p>{/if}
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">System Doctor</span><span class="badge" class:ok={runtimeStore.whisperCpp}>{runtimeStore.whisperEngine === "cpp" ? "Whisper.cpp active" : runtimeStore.whisperCpp ? "Whisper.cpp ready" : "Whisper.cpp missing"}</span></div>
          <p class="note">One-click local engine check. Whisper.cpp uses the bundled binary and ggml model — no Python or PyTorch.</p>
          <p class="note">GGML models: {#each runtimeStore.whisperCppModels ?? [] as model}<span class="badge" class:ok={model.ready}>{model.name} {model.ready ? "ready" : "not downloaded"}</span>{/each}</p>
          <div class="spread"><button class="btn btn-sm" disabled={!!ggmlBusy} onclick={() => downloadGgml("base")}>{ggmlBusy === "base" ? "Downloading…" : "Download GGML base (~142 MB)"}</button><button class="btn btn-sm" disabled={!!ggmlBusy} onclick={() => downloadGgml("small-q5_1")}>{ggmlBusy === "small-q5_1" ? "Downloading…" : "Download GGML small q5 (~190 MB)"}</button></div>
          <div class="spread"><button class="btn btn-sm" disabled={doctorBusy} onclick={runSystemDoctor}>{doctorBusy ? "Checking…" : "Run system check"}</button><button class="btn btn-sm" disabled={doctorBusy || !runtimeStore.whisperCpp} onclick={smokeWhisperCppNow}>CPU smoke test</button><button class="btn btn-sm btn-primary" disabled={doctorBusy || !runtimeStore.whisperCpp} onclick={setupWhisperCppNow}>{runtimeStore.whisperEngine === "cpp" ? "Whisper.cpp active ✓" : "Setup Whisper.cpp (no Python)"}</button></div>
          <div class="spread"><span class="note" style="margin:0;">Python venv is deprecated. Use bundled Whisper.cpp for new setups.</span><button class="btn btn-sm" disabled={doctorBusy || !runtimeStore.whisperCpp} onclick={setupWhisperCppNow}>{runtimeStore.whisperEngine === "cpp" ? "Active ✓" : "Use Whisper.cpp"}</button></div>
          {#if !runtimeStore.whisperCpp}<p class="hint">Rebuild after running <code>scripts/fetch-bundled-assets.sh</code> to include whisper-cli and ggml-tiny.bin.</p>{/if}
          {#if doctorBusy && runtimeStore.whisperBootstrap}<p class="hint" aria-live="polite">{runtimeStore.whisperBootstrap.message} ({Math.round(runtimeStore.whisperBootstrap.progress * 100)}%)</p>{/if}
          {#if doctorMessage}<p class="hint" aria-live="polite">{doctorMessage}</p>{/if}
          <button class="btn btn-sm btn-quiet" disabled={!doctorBusy || !runtimeStore.whisperBootstrap} onclick={stopWhisperBootstrap}>Stop bootstrap polling</button>
        </div>

        <div class="panel">
          <div class="panel-title"><span class="eyebrow">Highlight strategy</span></div>
          <div class="stack">
            <button class="strategy-card" class:on={strategy === "heuristic"} disabled={strategyBusy} onclick={() => saveStrategy("heuristic", "", localLlmUrl, localLlmModel)}>
              <span class="spread"><strong>Heuristic (offline)</strong>{#if strategy === "heuristic"}<span class="badge ok">active</span>{/if}</span>
              <p>Score segments by speech density + keywords. No API key, works offline.</p>
            </button>
            <div class="strategy-card" class:on={strategy === "semantic-openai"}>
              <span class="spread"><strong>Cloud provider (OpenAI-compatible)</strong>{#if strategy === "semantic-openai"}<span class="badge ok">active</span>{/if}</span>
              <p>Use a provider endpoint and model of your choice. API keys are stored locally and never displayed.</p>
              {#each providers as provider (provider.id)}
                <div class="spread" style="border-top:1px solid var(--line-soft);padding:.55rem 0;">
                  <button class="btn btn-sm btn-quiet" disabled={strategyBusy || activeProviderId === provider.id} onclick={() => activateProvider(provider.id)}>{activeProviderId === provider.id ? "Active ✓" : "Activate"}</button>
                  <span class="note" style="margin:0;flex:1;"><strong>{provider.name}</strong> · {provider.model} · {provider.baseUrl} · {provider.hasKey ? "key saved" : "no key"}</span>
                  <button class="btn btn-sm btn-quiet" disabled={strategyBusy} onclick={() => openProviderForm(provider)}>Edit</button>
                  <button class="btn btn-sm btn-quiet" disabled={strategyBusy} onclick={() => removeProvider(provider)}>Delete</button>
                </div>
              {/each}
              <button class="btn btn-sm" onclick={() => openProviderForm()}>+ Add provider</button>
            </div>
            <button class="strategy-card" class:on={strategy === "semantic-local"} disabled={strategyBusy} onclick={() => saveStrategy("semantic-local", "", localLlmUrl, localLlmModel)}>
              <span class="spread"><strong>Local LLM (offline)</strong>{#if strategy === "semantic-local"}<span class="badge ok">active</span>{/if}</span>
              <p>Rank with the locally installed Qwen model via llama.cpp — no key. Runtime and model are optional and must be installed above first.</p>
            </button>
          </div>
          {#if providerFormOpen}
            <div class="stack" style="border-top:1px solid var(--line-soft);padding-top:.8rem;margin-top:.8rem;">
              <label class="field"><span class="field-label">Name</span><input bind:value={providerName} placeholder="My provider" /></label>
              <label class="field"><span class="field-label">Base URL</span><input bind:value={providerBaseUrl} placeholder="https://api.openai.com/v1" /></label>
              <label class="field"><span class="field-label">API Key</span><input type="password" autocomplete="new-password" bind:value={providerApiKey} placeholder={editingProviderId && providers.find(p => p.id === editingProviderId)?.hasKey ? "Key saved — enter a new one to replace" : "API key"} /></label>
              <label class="field"><span class="field-label">Model</span><input bind:value={providerModel} list="provider-model-suggestions" placeholder="model-name" /><datalist id="provider-model-suggestions">{#each suggestedModels as model}<option value={model}></option>{/each}</datalist></label>
              <div class="field-row"><button class="btn btn-sm" disabled={strategyBusy || !providerName.trim() || !providerBaseUrl.trim() || !providerModel.trim() || (!editingProviderId && providerApiKey.trim().length < 10)} onclick={saveProvider}>{strategyBusy ? "Saving…" : "Save provider"}</button><button class="btn btn-sm btn-quiet" onclick={() => { providerFormOpen = false; providerApiKey = ""; }}>Cancel</button></div>
            </div>
          {/if}
          {#if strategy === "semantic-local"}
            <label class="field"><span class="field-label">Local LLM URL</span><input type="text" bind:value={localLlmUrl} placeholder="http://127.0.0.1:8080" /></label>
            <label class="field"><span class="field-label">Model</span><input type="text" bind:value={localLlmModel} placeholder="qwen2.5-3b-instruct" /></label>
            <button class="btn btn-sm" disabled={strategyBusy || localLlmUrl.trim().length < 10} onclick={() => saveStrategy("semantic-local", "", localLlmUrl, localLlmModel)}>Save</button>
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
