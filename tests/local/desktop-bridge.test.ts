import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { LocalStore } from "../../adapters/local/store.ts";

const exec = promisify(execFile);
const root = await mkdtemp(join(tmpdir(), "desktop-local-render-"));
const script = new URL("../../scripts/local-render.ts", import.meta.url).pathname;
const env = { ...process.env, LOCAL_FIRST_STATE_DIR: join(root, "state") };
const run = async (...args: string[]) => JSON.parse((await exec(process.execPath, ["--experimental-strip-types", script, ...args], { env })).stdout);
try {
  const request = { input: join(root, "source.mp4"), output: join(root, "out.mp4"), start: 1, end: 5, aspectRatio: "1:1" };
  const enqueued = await run("enqueue", JSON.stringify(request));
  assert.equal(enqueued.status, "queued");
  assert.equal(enqueued.start, 1);
  assert.equal((await run("enqueue", JSON.stringify(request))).id, enqueued.id);
  assert.equal((await run("get", enqueued.id)).id, enqueued.id);
  assert.equal((await run("list")).length, 1);
  assert.deepEqual((await run("status")).supportedAspectRatios, ["9:16", "1:1", "16:9"]);
  await assert.rejects(exec(process.execPath, ["--experimental-strip-types", script, "enqueue", JSON.stringify({ ...request, input: "https://example.com/video.mp4" })], { env }), /LOCAL_INPUT_REQUIRED/);
  const store = LocalStore.open(join(root, "state", "local-render.sqlite"));
  store.update(enqueued.id, { status: "failed", error: "test" });
  assert.equal(store.retry(enqueued.id)?.status, "queued");
  assert.equal(store.get(enqueued.id)?.attempts, 0);
  assert.equal(store.retry(enqueued.id), undefined);
  store.close();
} finally {
  await rm(root, { recursive: true, force: true });
}
console.log("desktop local render bridge tests: ok");
