import { AnalysisStore, MAX_JOB_ATTEMPTS } from "../adapters/local/analysis-store.ts";
import { readFile, unlink } from "node:fs/promises";
import { sourceFingerprint, cachePath } from "../adapters/local/analysis-cache.ts";

const [dbPath, target, mode, ...rest] = process.argv.slice(2);
if (!dbPath) process.exit(2);
const store = AnalysisStore.open(dbPath);
const alive = (pid: number | null) => { if (!pid) return false; try { process.kill(pid, 0); return true; } catch { return false; } };
// A worker that dies without finishing a chunk counts against the job budget.
// Progress resets the counter, so a long job that keeps advancing is never
// abandoned, while a chunk that always fails stops looping.
const claim = (job: { id: string }) => {
  const attempts = store.bumpAttempts(job.id);
  if (attempts >= MAX_JOB_ATTEMPTS) {
    store.update(job.id, { status: "failed", phase: "failed", error: `worker died ${attempts} times without progress; retry manually` });
    return undefined;
  }
  store.update(job.id, { status: "queued", phase: "queued", error: "worker lost; will resume" });
  return store.get(job.id);
};

if (target === "--recover") {
  store.recover();
  process.stdout.write("null");
} else if (target === "--resumable") {
  for (const job of store.listActive()) if (job.status === "running" && !alive(store.workerPid(job.id))) claim(job);
  const resumable = store.listQueued().filter((job) => !alive(store.workerPid(job.id))).map((job) => claim(job)).filter((job) => job !== undefined);
  const observing = store.listActive().filter((job) => job.status === "running" && alive(store.workerPid(job.id)));
  process.stdout.write(JSON.stringify({ resumable, observing }));
} else if (mode === "--retry") {
  store.retry(target);
  process.stdout.write(JSON.stringify(store.get(target) ?? null));
} else if (mode === "--reset-attempts") {
  store.resetAttempts(target);
  process.stdout.write(JSON.stringify(store.get(target) ?? null));
} else if (mode === "--clear") {
  const job = store.get(target); let bytes = 0;
  if (job) { for (const chunk of store.chunksOf(target)) { try { const path = cachePath(await sourceFingerprint(job.input), chunk.cacheKey); const info = await import("node:fs/promises").then(({ stat }) => stat(path)); bytes += info.size; await unlink(path); } catch {} } }
  bytes += store.clear(target); process.stdout.write(JSON.stringify({ bytes }));
} else if (target === "--claim") {
  process.stdout.write(JSON.stringify(store.listQueued()));
} else if (target === "--active") {
  process.stdout.write(JSON.stringify(store.allJobs()));
} else if (!target) {
  process.stdout.write("null");
} else if (mode === "--fail") {
  store.update(target, { status: "failed", phase: "failed", error: rest.join(" ") || "worker failed" });
  process.stdout.write(JSON.stringify(store.get(target) ?? null));
} else {
  const job = store.get(target);
  const result = job?.resultPath ? await readFile(job.resultPath, "utf8").catch(() => null) : null;
  process.stdout.write(JSON.stringify(job ? { ...job, result, resultPath: job.resultPath } : null));
}

store.close();
