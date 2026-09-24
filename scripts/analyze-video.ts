import { analyzeLocalVideo, analyzeLocalVideoWithSemantic, SemanticProviderAdapter, NotImplementedSemanticProvider, type SemanticHighlightProvider } from "../services/ai-pipeline/index.ts";
import { GeminiHighlightProvider } from "../services/ai-pipeline/providers.ts";
import { AnalysisStore } from "../adapters/local/analysis-store.ts";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

const [mediaPath, sourceArtifactId, durationText, rangeStartText, rangeEndText, jobId, statePath] = process.argv.slice(2);
const duration = Number(durationText);
const rangeStart = Number(rangeStartText ?? 0);
const rangeEnd = Number(rangeEndText ?? duration);
if (!Number.isFinite(rangeStart) || !Number.isFinite(rangeEnd) || rangeStart < 0 || rangeEnd <= rangeStart || rangeEnd > duration) { console.error("ANALYZE_INVALID_RANGE"); process.exit(2); }
if (!mediaPath || !sourceArtifactId || !Number.isFinite(duration) || duration <= 0) {
  console.error("ANALYZE_INVALID_ARGUMENTS");
  process.exit(2);
}

const store = jobId && statePath ? AnalysisStore.open(statePath) : undefined;
const persistResult = async (payload: unknown): Promise<string> => {
  const path = join(dirname(statePath ?? "."), `analysis-result-${jobId}.json`);
  await mkdir(dirname(path), { recursive: true });
  const temp = `${path}.${process.pid}.tmp`;
  await writeFile(temp, JSON.stringify(payload));
  await rename(temp, path);
  return path;
};

let interrupted = false;
const onSignal = () => { interrupted = true; };
process.on("SIGTERM", onSignal);
process.on("SIGINT", onSignal);

try {
  store?.update(jobId!, { status: "running", phase: "transcribe", progress: 0.1 });
  store?.setWorkerPid(jobId!, process.pid);
  const strategy = process.env.HIGHLIGHT_STRATEGY ?? "heuristic";
  const geminiKey = process.env.GEMINI_API_KEY ?? "";
  let result: Awaited<ReturnType<typeof analyzeLocalVideo>>;
  if (strategy === "semantic" && geminiKey) {
    const semanticProvider: SemanticHighlightProvider = new SemanticProviderAdapter(new GeminiHighlightProvider(geminiKey), "gemini");
    result = await analyzeLocalVideoWithSemantic(mediaPath, sourceArtifactId, rangeEnd - rangeStart, {
      semanticProvider,
      semanticLimit: 5,
      command: process.env.WHISPER_COMMAND,
      model: process.env.WHISPER_MODEL,
      language: process.env.WHISPER_LANGUAGE,
      device: process.env.WHISPER_DEVICE,
      fp16: process.env.WHISPER_FP16 === undefined ? undefined : process.env.WHISPER_FP16 === "1" || process.env.WHISPER_FP16 === "true",
      offsetSeconds: rangeStart,
      sourceDuration: duration,
      onChunkStart: (index, total) => store?.updateChunk(jobId!, index, { status: "running", attempts: 1 }),
      onChunkComplete: (index, total, cacheHit) => {
        store?.updateChunk(jobId!, index, { status: "completed", error: cacheHit ? "cache hit" : null });
        store?.update(jobId!, { phase: "transcribe", progress: 0.1 + 0.8 * (index + 1) / total });
        store?.resetAttempts(jobId!);
      },
      onChunkFail: (index, total, error) => store?.updateChunk(jobId!, index, { status: "failed", error }),
      onProgress: (completed, total) => store?.update(jobId!, { phase: "transcribe", progress: 0.1 + 0.8 * completed / total }),
    });
  } else {
    result = await analyzeLocalVideo(mediaPath, sourceArtifactId, rangeEnd - rangeStart, {
      command: process.env.WHISPER_COMMAND,
      model: process.env.WHISPER_MODEL,
      language: process.env.WHISPER_LANGUAGE,
      device: process.env.WHISPER_DEVICE,
      fp16: process.env.WHISPER_FP16 === undefined ? undefined : process.env.WHISPER_FP16 === "1" || process.env.WHISPER_FP16 === "true",
      offsetSeconds: rangeStart,
      sourceDuration: duration,
      onChunkStart: (index, total) => store?.updateChunk(jobId!, index, { status: "running", attempts: 1 }),
      onChunkComplete: (index, total, cacheHit) => {
        store?.updateChunk(jobId!, index, { status: "completed", error: cacheHit ? "cache hit" : null });
        store?.update(jobId!, { phase: "transcribe", progress: 0.1 + 0.8 * (index + 1) / total });
        store?.resetAttempts(jobId!);
      },
      onChunkFail: (index, total, error) => store?.updateChunk(jobId!, index, { status: "failed", error }),
      onProgress: (completed, total) => store?.update(jobId!, { phase: "transcribe", progress: 0.1 + 0.8 * completed / total }),
    });
  }
  if (interrupted) throw new Error("ANALYZE_INTERRUPTED");
  // Free tier: only the top 3 highlights, CPU + tiny already enforced by start_analysis.
  if (process.env.LICENSE_TIER !== "pro") result.highlights = result.highlights.slice(0, 3);
  store?.update(jobId!, { phase: "rank", progress: 0.9 });
  const resultPath = store ? await persistResult(result) : null;
  store?.update(jobId!, { status: "completed", phase: "completed", progress: 1, resultPath, error: null });
  store?.close();
  process.stdout.write(JSON.stringify(result));
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  store?.update(jobId!, { status: interrupted ? "queued" : "failed", phase: interrupted ? "queued" : "failed", error: message });
  store?.close();
  console.error(message);
  process.exit(1);
}
