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
let strategyBusy = $state(false);

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

  async function saveStrategy(next: string, key: string) {
    strategyBusy = true;
    try {
      strategy = await invoke<string>("set_highlight_strategy", { strategy: next, geminiApiKey: key });
      geminiKey = next === "semantic" ? key : "";
      message = next === "semantic" ? "Semantic (Gemini) strategy saved — API key stored locally in runtime.env." : "Heuristic strategy saved — no cloud calls.";
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
    const status = await invoke<{ strategy?: string; hasKey?: boolean }>("highlight_strategy_status");
    strategy = status.strategy === "semantic" ? "semantic" : "heuristic";
    geminiKey = status.hasKey ? "••••••••" : "";
  } catch { /* not in Tauri (browser dev): keep defaults */ }
}
onMount(refresh);
onMount(readStrategy);
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
  <p class="hint">Heuristic = offline, deterministic, no cost. Semantic = Gemini LLM proposes clips by meaning (uses your API key, local storage only).</p>
  <div class="model-row">
    <button class="chip" class:sel={strategy === "heuristic"} disabled={strategyBusy} onclick={() => saveStrategy("heuristic", "")}>Heuristic (offline)</button>
    <button class="chip" class:sel={strategy === "semantic"} disabled={strategyBusy || geminiKey.length < 10} onclick={() => saveStrategy("semantic", geminiKey)}>Semantic (Gemini BYOK)</button>
  </div>
  {#if strategy === "semantic"}
    <label class="hint">Gemini API key <input type="password" value={geminiKey} oninput={(e) => geminiKey = e.currentTarget.value} placeholder="AIza..." /></label>
    <button class="chip" disabled={strategyBusy || geminiKey.length < 10} onclick={() => saveStrategy("semantic", geminiKey)}>Save key</button>
  {/if}
  <p class="hint">Current: {strategy}{strategy === "semantic" ? (geminiKey ? " · key stored" : " · no key yet") : ""}.</p>
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