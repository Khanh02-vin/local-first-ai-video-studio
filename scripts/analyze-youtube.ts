import { chooseContentHighlights } from "../services/ai-pipeline/content-highlights.ts";
import { chooseHeuristicHighlights } from "../services/ai-pipeline/heuristic.ts";
import { generateSemanticHighlights, SemanticProviderAdapter, type SemanticHighlightProvider } from "../services/ai-pipeline/semantic.ts";
import { GeminiHighlightProvider, LlamaCppHighlightProvider } from "../services/ai-pipeline/providers.ts";
import { CachedSemanticProvider } from "../services/ai-pipeline/llm-cache.ts";
import { transcriptFromCaptions, type CaptionPayload } from "../services/ai-pipeline/youtube-transcript.ts";
import { validateHighlights, type Highlight } from "../packages/contracts/highlight.ts";
import { AnalysisStore } from "../adapters/local/analysis-store.ts";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";

const [url, durationText, jobId, statePath] = process.argv.slice(2);
const duration = Number(durationText);
if (!url?.startsWith("http") || !Number.isFinite(duration) || duration <= 0 || !jobId || !statePath) {
  console.error("YOUTUBE_INVALID_ARGUMENTS");
  process.exit(2);
}

const store = AnalysisStore.open(statePath);
const persistResult = async (payload: unknown): Promise<string> => {
  const path = join(dirname(statePath), `analysis-result-${jobId}.json`);
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

// One metadata + caption round-trip via the bundled crawler — no Whisper, no download.
function fetchCaptions(videoUrl: string): CaptionPayload {
  const python = process.env.YT_PYTHON;
  const crawler = process.env.YT_CRAWLER;
  if (!python || !crawler) throw new Error("YOUTUBE_CRAWLER_ENV_MISSING");
  const run = spawnSync(python, [crawler, "--video", videoUrl], { encoding: "utf8", timeout: 180_000 });
  if (run.error) throw new Error(`CRAWLER_SPAWN:${run.error.message}`);
  if (run.status !== 0) throw new Error(run.stderr?.trim() || `CRAWLER_EXIT:${run.status}`);
  let payload: CaptionPayload;
  try { payload = JSON.parse(run.stdout) as CaptionPayload; } catch { throw new Error("YOUTUBE_PAYLOAD_JSON"); }
  if (typeof payload.videoId !== "string" || !payload.videoId || !Array.isArray(payload.segments)) throw new Error("YOUTUBE_PAYLOAD_SHAPE");
  return payload;
}

try {
  store.update(jobId, { status: "running", phase: "transcribe", progress: 0.1 });
  store.setWorkerPid(jobId, process.pid);
  const payload = fetchCaptions(url);
  if (interrupted) throw new Error("ANALYZE_INTERRUPTED");
  const transcript = transcriptFromCaptions({ ...payload, duration });
  store.update(jobId, { phase: "transcribe", progress: 0.7 });

  const strategy = process.env.HIGHLIGHT_STRATEGY ?? "heuristic";
  const geminiKey = process.env.GEMINI_API_KEY ?? "";
  const llamaBaseUrl = process.env.LOCAL_LLM_BASE_URL ?? "http://127.0.0.1:8080";
  const llamaModel = process.env.LOCAL_LLM_MODEL ?? "qwen2.5-3b-instruct";

  // CachedSemanticProvider: replays stored highlights for the same transcript +
  // prompt version instead of re-calling the LLM; identity includes the model.
  const semanticProvider: SemanticHighlightProvider | undefined =
    strategy === "semantic-gemini" && geminiKey ? new CachedSemanticProvider(new SemanticProviderAdapter(new GeminiHighlightProvider(geminiKey), "gemini"), "gemini")
    : strategy === "semantic-local" ? new CachedSemanticProvider(new SemanticProviderAdapter(new LlamaCppHighlightProvider(llamaBaseUrl, llamaModel), "llama-cpp"), `llama-cpp:${llamaModel}`)
    : undefined;

  const chooseFallback = (): Highlight[] => duration > 120 ? chooseContentHighlights(transcript) : chooseHeuristicHighlights(transcript);
  let highlights: Highlight[];
  if (semanticProvider) {
    try {
      highlights = await generateSemanticHighlights(transcript, semanticProvider, { limit: 5, minDuration: 10, maxDuration: 90 });
    } catch (error) {
      // Provider failure must not lose the job: fall back to the deterministic selector.
      console.warn(`[ai-pipeline] semantic provider failed (${error instanceof Error ? error.message : error}); fell back to ${duration > 120 ? "content" : "heuristic"} selector`);
      highlights = chooseFallback();
    }
  } else {
    highlights = chooseFallback();
  }
  if (interrupted) throw new Error("ANALYZE_INTERRUPTED");
  validateHighlights(highlights, duration);
  // Free tier: only the top 3 highlights.
  if (process.env.LICENSE_TIER !== "pro") highlights = highlights.slice(0, 3);
  store.update(jobId, { phase: "rank", progress: 0.9 });
  const result = { transcript, highlights, videoId: payload.videoId };
  const resultPath = await persistResult(result);
  store.update(jobId, { status: "completed", phase: "completed", progress: 1, resultPath, error: null });
  store.close();
  process.stdout.write(JSON.stringify(result));
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  store.update(jobId, { status: interrupted ? "queued" : "failed", phase: interrupted ? "queued" : "failed", error: message });
  store.close();
  console.error(message);
  process.exit(1);
}
