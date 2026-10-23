import assert from "node:assert/strict";
import { enqueueProjectSave } from "../../apps/web/src/lib/project-save-queue.ts";

const events: string[] = [];
let releaseFirst!: () => void;
const firstGate = new Promise<void>((resolve) => { releaseFirst = resolve; });
const first = enqueueProjectSave("p1", async () => { events.push("first-start"); await firstGate; events.push("first-end"); });
const second = enqueueProjectSave("p1", async () => { events.push("second-start"); events.push("second-end"); });
await new Promise((resolve) => setTimeout(resolve, 0));
assert.deepEqual(events, ["first-start"]);
releaseFirst();
await Promise.all([first, second]);
assert.deepEqual(events, ["first-start", "first-end", "second-start", "second-end"]);

const failed = enqueueProjectSave("p2", async () => { throw new Error("save failed"); });
const recovered = enqueueProjectSave("p2", async () => "saved");
await assert.rejects(failed, /save failed/);
assert.equal(await recovered, "saved");
console.log("project save queue tests: ok");
