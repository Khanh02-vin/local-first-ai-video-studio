import { invoke, isTauri } from "@tauri-apps/api/core";

// Shared engine/storage/license status. Pages used to re-invoke runtime_status
// on every mount — that call also copied the bundled Whisper model file (tens
// of MB) on first run, freezing page switches for seconds. Everything reads
// from this cache instead; the layout fetches once and polls while the
// first-run model copy finishes in the background.
export type RuntimeInfo = {
  ready: boolean;
  runtimeMessage: string;
  storageMessage: string;
  licenseMessage: string;
  device: string;
  model: string;
  components: { ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; model: boolean };
  bundled: { node: boolean; ffmpeg: boolean; ffprobe: boolean };
  loaded: boolean;
};

export const runtimeStore = $state<RuntimeInfo>({
  ready: false,
  runtimeMessage: "Checking local runtime…",
  storageMessage: "Checking storage…",
  licenseMessage: "",
  device: "cpu",
  model: "tiny",
  components: { ffmpeg: false, ffprobe: false, node: false, whisper: false, model: false },
  bundled: { node: false, ffmpeg: false, ffprobe: false },
  loaded: false,
});


let inflight: Promise<void> | null = null;
let inflightAt = 0;

export async function refreshRuntime(force = false): Promise<void> {
  if (!isTauri()) {
    runtimeStore.loaded = true;
    runtimeStore.runtimeMessage = "Local engine controls live in the desktop app — this browser preview cannot run analysis.";
    return;
  }
  // Retry guard: a lost first invoke (sent before the IPC channel settles)
  // left the poll reusing a promise that never resolves. After 8 s, drop it.
  if (inflight && Date.now() - inflightAt < 8000) return inflight;
  if (runtimeStore.loaded && !force) return;
  inflightAt = Date.now();
  inflight = (async () => {
    try {
      const license = await invoke<{ licensed: boolean; tier?: string; licensee?: string | null; error?: string; configured?: boolean }>("license_status");
      const runtime = await invoke<{ ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; modelReady: boolean; device?: string; model?: string; bundled?: { node: boolean; ffmpeg: boolean; ffprobe: boolean } }>("runtime_status");
      const storage = await invoke<{ home: { ready: boolean; freeBytes?: number; requiredBytes: number }; temp: { ready: boolean; freeBytes?: number } }>("storage_status");
      const storageReady = storage.home.ready && storage.temp.ready;
      runtimeStore.ready = runtime.ffmpeg && runtime.ffprobe && runtime.node && runtime.whisper && runtime.modelReady && storageReady;
      runtimeStore.components = { ffmpeg: runtime.ffmpeg, ffprobe: runtime.ffprobe, node: runtime.node, whisper: runtime.whisper, model: runtime.modelReady };
      runtimeStore.bundled = runtime.bundled ?? runtimeStore.bundled;
      runtimeStore.device = runtime.device ?? "cpu";
      runtimeStore.model = runtime.model ?? "tiny";
      runtimeStore.runtimeMessage = runtimeStore.ready ? `Engine ready · ${runtimeStore.device === "cuda" ? "GPU (CUDA)" : "CPU"}` : "Whisper/model, media runtime, or storage is missing.";
      runtimeStore.licenseMessage = license.licensed ? `License ${license.tier}${license.licensee ? ` · ${license.licensee}` : ""}` : (license.error ?? (license.configured === false ? "Unlicensed (free tier)" : "Unlicensed"));
      runtimeStore.storageMessage = storageReady ? `${formatBytes(storage.home.freeBytes ?? 0)} free` : `Need ${formatBytes(storage.home.requiredBytes)}`;
      runtimeStore.loaded = true;
    } catch (error) {
      runtimeStore.runtimeMessage = isTauri() ? `Runtime check failed: ${String(error)}` : "Local engine controls live in the desktop app — this browser preview cannot run analysis.";
      runtimeStore.loaded = true;
    } finally { inflight = null; }
  })();
  return inflight;
}

export function formatBytes(bytes: number) { return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`; }
