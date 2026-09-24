import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { LocalRunner } from "../../adapters/local/runner.ts";
import { LocalStore } from "../../adapters/local/store.ts";
import { AnalysisStore } from "../../adapters/local/analysis-store.ts";
import { chunkCacheKey, sourceFingerprint } from "../../adapters/local/analysis-cache.ts";
import { ApiService } from "../../services/api/index.ts";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const root = await mkdtemp(join(tmpdir(), "idempotency-gaps-"));
const ffmpeg = process.env.FFMPEG_PATH ?? "/home/khanh/.local/bin/ffmpeg";
const ffprobe = process.env.FFPROBE_PATH ?? "/home/khanh/.local/bin/ffprobe";

function assertFixed(name: string, fact: boolean, description: string): void {
  assert.ok(fact, `${name}: ${description}`);
  console.log(`${name} fixed: ${description}`);
}

async function makeMp4(path: string, seconds = 2): Promise<void> {
  await execFileAsync(ffmpeg, [
    "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10",
    "-f", "lavfi", "-i", "sine=frequency=1000", "-t", String(seconds),
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", path,
  ]);
}

try {
  // GAP-1: two user clicks on the same render request must dedupe to one job.
  const input = join(root, "input.mp4");
  const output = join(root, "output.mp4");
  await makeMp4(input, 2);
  const runner = new LocalRunner({ ffmpegPath: ffmpeg, ffprobePath: ffprobe, maxDurationSeconds: 10, statePath: ":memory:" });
  const click1 = runner.enqueue({ input, output, aspectRatio: "9:16" });
  const click2 = runner.enqueue({ input, output, aspectRatio: "9:16" });
  assertFixed("GAP-1", click1.id === click2.id && runner.store.list().length === 1,
    "two identical enqueues now return the same job instead of stacking a second one");
  const otherAspect = runner.enqueue({ input, output: join(root, "o2.mp4"), aspectRatio: "16:9" });
  assertFixed("GAP-1b", otherAspect.id !== click1.id && runner.store.list().length === 2,
    "a different aspect ratio is a distinct render and still enqueues its own job");
  runner.close();

  // GAP-2: a render bound to an analysis job is deferred until that analysis completes.
  const analysisDb = join(root, "analysis.sqlite");
  const jobsDb = join(root, "jobs.sqlite");
  const analysis = AnalysisStore.open(analysisDb);
  analysis.create("an-1", "vid.mp4", 60);
  analysis.update("an-1", { status: "running", phase: "transcribe", progress: 0.2 });
  const store = LocalStore.open(jobsDb, analysisDb);
  store.create({ id: "r-1", input: "vid.mp4", output: "out.mp4", start: null, end: null, aspectRatio: "9:16", analysisJobId: "an-1" });
  const whileRunning = store.claim();
  assertFixed("GAP-2", whileRunning === undefined,
    "claim() refuses to start a render whose analysis job has not completed");
  analysis.update("an-1", { status: "completed" });
  const afterCompletion = store.claim();
  assertFixed("GAP-2b", afterCompletion?.id === "r-1" && afterCompletion.status === "running",
    "the render is claimed only after the analysis it depends on is completed");
  store.close();
  analysis.close();

  // GAP-3: the chunk cache key is stable for an untouched source but invalidates when the
  // source file is replaced at the same path; a stale initial fingerprint can never hit a live entry.
  const mediaDir = join(root, "media");
  await mkdir(mediaDir, { recursive: true });
  const mediaPath = join(mediaDir, "video.mp4");
  await makeMp4(mediaPath, 2);
  const fpBefore = await sourceFingerprint(mediaPath);
  const stableKey = await chunkCacheKey(mediaPath, 0, 0, 5, "tiny", "auto", "cpu", false, fpBefore);
  const stableKeyAgain = await chunkCacheKey(mediaPath, 0, 0, 5, "tiny", "auto", "cpu", false, await sourceFingerprint(mediaPath));
  assertFixed("GAP-3", stableKey === stableKeyAgain,
    "the chunk cache key stays stable across re-runs for an unchanged source (no cache churn)");
  await writeFile(mediaPath, "replaced-content", "utf8");
  const replacedKey = await chunkCacheKey(mediaPath, 0, 0, 5, "tiny", "auto", "cpu", false, await sourceFingerprint(mediaPath));
  assertFixed("GAP-3b", stableKey !== replacedKey,
    "a replaced source file at the same path produces a different cache key, so stale transcripts cannot be returned");

  // GAP-4 (parse defect): ApiService and its idempotency-key generation are loadable in the
  // strip-types runtime and accept a client-supplied idempotency key.
  const api = new ApiService();
  const jobA = await api.createRenderJob(["src-1"], "cloud", "client-key-1");
  const jobB = await api.createRenderJob(["src-1"], "cloud", "client-key-1");
  assertFixed("GAP-4", typeof api === "object" && jobA.idempotencyKey === "client-key-1" && jobA.idempotencyKey === jobB.idempotencyKey,
    "ApiService parses in strip-only mode and preserves a caller-supplied idempotency key (repeatable requests stay identifiable)");

  // GAP-5: terminal jobs are re-queued so clicking "Render" again restarts them.
  const requeueStore = LocalStore.open(join(root, "requeue-jobs.sqlite"));
  const jobCancelled = requeueStore.create({ id: "rc", input: "vid-a.mp4", output: "o-a.mp4", start: 0, end: 5, aspectRatio: "9:16" });
  requeueStore.update(jobCancelled.id, { status: "cancelled" });
  const retryCancelled = requeueStore.create({ id: "rc2", input: "vid-a.mp4", output: "o-a.mp4", start: 0, end: 5, aspectRatio: "9:16" });
  assertFixed("GAP-5", retryCancelled.id === jobCancelled.id && retryCancelled.status === "queued",
    "create() on a cancelled job re-queues the same record instead of returning a dead one");
  const jobFailed = requeueStore.create({ id: "rf", input: "vid-b.mp4", output: "o-b.mp4", start: 0, end: 5, aspectRatio: "9:16" });
  requeueStore.update(jobFailed.id, { status: "failed", error: "attempt budget exhausted" });
  const retryFailed = requeueStore.create({ id: "rf2", input: "vid-b.mp4", output: "o-b.mp4", start: 0, end: 5, aspectRatio: "9:16" });
  assertFixed("GAP-5b", retryFailed.id === jobFailed.id && retryFailed.status === "queued" && retryFailed.attempts === 0,
    "create() on a failed job (retry exhausted) resets the crash budget and re-queues the same record");

  // GAP-6: a completed SQLite row is reusable only while its local artifact is intact.
  const missingOutput = join(root, "missing-output.mp4");
  await writeFile(missingOutput, Buffer.alloc(11));
  const completedMissing = requeueStore.create({ id: "gm", input: "vid-missing.mp4", output: missingOutput, start: 0, end: 5, aspectRatio: "9:16" });
  requeueStore.update(completedMissing.id, { status: "completed", progress: 1, outputBytes: 11 });
  await rm(missingOutput, { force: true });
  const rerunMissing = requeueStore.create({ id: "gm-retry", input: "vid-missing.mp4", output: missingOutput, start: 0, end: 5, aspectRatio: "9:16" });
  assertFixed("GAP-6", rerunMissing.id === completedMissing.id && rerunMissing.status === "queued" && rerunMissing.attempts === 0 && rerunMissing.outputBytes === null,
    "create() invalidates a completed job when its output file is missing");

  const changedOutput = join(root, "changed-output.mp4");
  await writeFile(changedOutput, Buffer.alloc(7));
  const completedChanged = requeueStore.create({ id: "gs", input: "vid-changed.mp4", output: changedOutput, start: 0, end: 5, aspectRatio: "9:16" });
  requeueStore.update(completedChanged.id, { status: "completed", progress: 1, outputBytes: 11 });
  const rerunChanged = requeueStore.create({ id: "gs-retry", input: "vid-changed.mp4", output: changedOutput, start: 0, end: 5, aspectRatio: "9:16" });
  assertFixed("GAP-6b", rerunChanged.id === completedChanged.id && rerunChanged.status === "queued" && rerunChanged.attempts === 0 && rerunChanged.outputBytes === null,
    "create() invalidates a completed job when its output size no longer matches SQLite metadata");
  requeueStore.close();
} finally {
  await rm(root, { recursive: true, force: true });
}
console.log("idempotency gap tests: ok (all 6 gaps fixed)");
