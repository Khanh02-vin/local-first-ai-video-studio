import assert from "node:assert/strict";
import { validateTranscript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";
import { validateCollection } from "../../packages/contracts/collection.ts";
import { validateRenderPlan } from "../../packages/contracts/render-plan.ts";
import { assertTransition, validateJob } from "../../packages/contracts/job.ts";
import { validateArtifact } from "../../packages/contracts/artifact.ts";
import { validatePublishTarget } from "../../packages/contracts/publish-target.ts";

const transcript = {
  version: 1 as const, sourceArtifactId: "src", language: "en", duration: 60, provider: "test",
  words: [{ id: "w1", text: "hello", start: 1, end: 2 }],
  segments: [{ id: "s1", text: "hello", start: 1, end: 2, wordIds: ["w1"] }],
};
validateTranscript(transcript);
assert.throws(() => validateTranscript({ ...transcript, words: [{ ...transcript.words[0], end: 0 }] }));

const highlight = { version: 1 as const, id: "h1", sourceArtifactId: "src", start: 1, end: 10, wordIds: ["w1"], title: "Hook", score: 80 };
validateHighlights([highlight], 60);
assert.throws(() => validateHighlights([highlight, { ...highlight, id: "h2", start: 9, end: 12 }], 60));

const collection = {
  version: 1 as const,
  id: "col1",
  projectId: "p1",
  sourceArtifactId: "src",
  title: "Best moments",
  topic: "tips",
  items: [
    { id: "i1", position: 0, reference: { kind: "highlight" as const, highlightId: "h1" } },
    { id: "i2", position: 1, reference: { kind: "renderPlan" as const, renderPlanId: "rp1" } },
  ],
  state: "approved" as const,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
};
validateCollection(collection);
assert.throws(() => validateCollection({ ...collection, version: 0 }));
assert.throws(() => validateCollection({ ...collection, id: "" }));
assert.throws(() => validateCollection({ ...collection, projectId: "" }));
assert.throws(() => validateCollection({ ...collection, title: "" }));
assert.throws(() => validateCollection({ ...collection, state: "unknown" as never }));
assert.throws(() => validateCollection({ ...collection, createdAt: "" }));
assert.throws(() => validateCollection({ ...collection, updatedAt: "" }));
// Empty items is valid (draft collection)
validateCollection({ ...collection, items: [] });
assert.throws(() => validateCollection({ ...collection, items: [
  { ...collection.items[0], id: "" },
  { ...collection.items[1], position: 0 },
] }));
assert.throws(() => validateCollection({ ...collection, items: [
  { ...collection.items[0], position: 2 },
  { ...collection.items[1], position: 0 },
] }));
assert.throws(() => validateCollection({ ...collection, items: [
  { ...collection.items[0], reference: { kind: "highlight" as const, highlightId: "" } },
] }));
assert.throws(() => validateCollection({ ...collection, items: [
  { ...collection.items[0], reference: { kind: "unknown" as never, highlightId: "h1" } },
] }));

const plan = { version: 1, sourceArtifactId: "src", ranges: [{ start: 1, end: 10 }], aspectRatio: "9:16" as const, cropMode: "track" as const, captions: [], outputProfile: { container: "mp4" as const, codec: "h264" as const } };
validateRenderPlan(plan, 60);
assert.throws(() => validateRenderPlan({ ...plan, version: 0 }, 60));
assert.throws(() => validateRenderPlan({ ...plan, aspectRatio: "4:3" as never }, 60));
assert.throws(() => validateRenderPlan({ ...plan, ranges: [] }, 60), /at least one range/);
validateRenderPlan({ ...plan, ranges: [{ start: 10, end: 12 }, { start: 1, end: 3 }, { start: 3, end: 4 }] }, 60);
validateRenderPlan({ ...plan, cropKeyframes: [{ time: 1, centerX: 0.4, centerY: 0.6, speakerId: "Speaker 1", sourceArtifactId: "src", confidence: 0.9, confirmed: true }] }, 60);
assert.throws(() => validateRenderPlan({ ...plan, cropKeyframes: [{ time: 1, centerX: 0.4, centerY: 0.6, sourceArtifactId: "other", confidence: 0.9, confirmed: true }] }, 60), /source mismatch/);
assert.throws(() => validateRenderPlan({ ...plan, cropKeyframes: [{ time: 1, centerX: 0.4, centerY: 1.1, confidence: 0.9, confirmed: true }] }, 60), /crop keyframe/);
validateRenderPlan({ ...plan, sceneLayouts: [{ start: 1, end: 10, layout: "side-by-side", confidence: 0.8, confirmed: true }] }, 60);
validateRenderPlan({ ...plan, sceneLayouts: [{ start: 1, end: 10, layout: "side-by-side", confidence: 0.8, confirmed: true, subjectBoxes: [{ x: 0.1, y: 0.1, width: 0.2, height: 0.3, confidence: 0.8 }, { x: 0.7, y: 0.2, width: 0.2, height: 0.3, confidence: 0.7 }] }] }, 60);
assert.throws(() => validateRenderPlan({ ...plan, sceneLayouts: [{ start: 1, end: 10, layout: "side-by-side", confidence: 0.8, confirmed: true, subjectBoxes: [{ x: 0.9, y: 0.1, width: 0.3, height: 0.3, confidence: 0.8 }, { x: 0.2, y: 0.2, width: 0.1, height: 0.1, confidence: 0.8 }] }] }, 60), /subject boxes/);
assert.throws(() => validateRenderPlan({ ...plan, sceneLayouts: [{ start: 1, end: 10, layout: "picture-in-picture", confidence: 2, confirmed: true }] }, 60), /scene layout/);

assertTransition("created", "queued");
assert.throws(() => assertTransition("completed", "queued"));
validateJob({ version: 1, id: "j1", type: "render", state: "queued", executionTarget: "local", inputArtifactIds: ["src"], outputArtifactIds: [], idempotencyKey: "key", retryCount: 0, createdAt: "now", updatedAt: "now" });

validateArtifact({ version: 1, id: "a1", artifactVersion: 1, mediaType: "video", storageLocator: "file:///out.mp4", checksum: "a".repeat(64), ownerId: "u1", projectId: "p1" });
assert.throws(() => validateArtifact({ version: 1, id: "a1", artifactVersion: 1, mediaType: "video", storageLocator: "file:///out.mp4", checksum: "", ownerId: "u1", projectId: "p1" }));

validatePublishTarget({ version: 1, id: "p1", artifactId: "a1", platform: "youtube-shorts", accountId: "acc", status: "draft" });
assert.throws(() => validatePublishTarget({ version: 1, id: "p1", artifactId: "a1", platform: "unknown" as never, accountId: "acc", status: "draft" }));

console.log("contract tests: ok");
