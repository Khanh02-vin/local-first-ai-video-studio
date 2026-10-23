import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AnalysisStore } from "../../adapters/local/analysis-store.ts";

const root = mkdtempSync(join(tmpdir(), "analysis-store-test-"));
try {
  const store = AnalysisStore.open(join(root, "analysis.sqlite"));
  store.create("cancel-race", "video.mp4", 30);
  store.update("cancel-race", { status: "running", phase: "transcribe" });
  store.createChunks("cancel-race", [{ chunkIndex: 0, start: 0, end: 30, cacheKey: "cache" }]);
  store.updateChunk("cancel-race", 0, { status: "running" });
  store.cancel("cancel-race");
  store.update("cancel-race", { status: "queued", phase: "queued", error: "late interrupt" });
  assert.equal(store.get("cancel-race")?.status, "cancelled");
  assert.equal(store.chunksOf("cancel-race")[0]?.status, "queued");
  store.update("cancel-race", { status: "completed", phase: "completed", progress: 1 });
  assert.equal(store.get("cancel-race")?.status, "cancelled");

  store.create("retry-failed", "video.mp4", 30);
  store.update("retry-failed", { status: "failed", phase: "failed", error: "worker error" });
  store.createChunks("retry-failed", [{ chunkIndex: 0, start: 0, end: 30, cacheKey: "retry-cache" }]);
  store.updateChunk("retry-failed", 0, { status: "failed", error: "chunk error" });
  assert.equal(store.retry("retry-failed"), true);
  assert.equal(store.get("retry-failed")?.status, "queued");
  assert.equal(store.chunksOf("retry-failed")[0]?.status, "queued");
  assert.equal(store.retry("retry-failed"), false);
  assert.equal(store.get("retry-failed")?.status, "queued");
  assert.equal(store.retry("missing-job"), false);

  store.close();
  console.log("analysis cancellation race tests: ok");
} finally {
  rmSync(root, { recursive: true, force: true });
}
