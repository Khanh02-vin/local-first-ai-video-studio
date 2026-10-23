/// Guards the media library and the crop path that feeds the renderer.
///
/// The renderer is the Rust render engine (ADR-002, ADR-003). The TypeScript side no
/// longer contains a second copy of the tracking or subtitle algorithms, so this test
/// covers what the TypeScript side is still responsible for:
///   - media input validation,
///   - the editor's view of where the crop currently sits,
///   - the mapping from the Rust tracker's raw output onto contract keyframes.
///
/// The last one carries the regression this file exists for: the Editor used to feed
/// the Rust tracker's output back through a TypeScript tracker, which smoothed the
/// path a second time. The stored crop then drifted away from the subject, and the
/// renderer — which reads the plan — followed the drifted path.
import assert from "node:assert/strict";
import { cropCenterAt } from "../../packages/media-engine/reframe/index.ts";
import { cropKeyframesFromPath, type RawCropPathPoint } from "../../packages/media-engine/reframe/crop-keyframes.ts";
import { validateMediaInput } from "../../packages/media-engine/validation/index.ts";

validateMediaInput({ duration: 60, width: 1920, height: 1080 });
assert.throws(() => validateMediaInput({ duration: 0, width: 1920, height: 1080 }), /Invalid media duration/);
assert.throws(() => validateMediaInput({ duration: Number.NaN, width: 1920, height: 1080 }), /Invalid media duration/);
assert.throws(() => validateMediaInput({ duration: 60, width: 0, height: 1080 }), /Invalid media dimensions/);
assert.throws(() => validateMediaInput({ duration: 60, width: 1920, height: 1080.5 }), /Invalid media dimensions/);

const cropKeys = [
  { time: 10, centerX: 0.2, centerY: 0.4, confidence: 0.9, confirmed: true },
  { time: 12, centerX: 0.8, centerY: 0.6, confidence: 0.9, confirmed: true },
];
assert.deepEqual(cropCenterAt(cropKeys, 11), { x: 0.5, y: 0.5 });
assert.deepEqual(cropCenterAt(cropKeys, 9), { x: 0.2, y: 0.4 });
assert.deepEqual(cropCenterAt(cropKeys, 13), { x: 0.8, y: 0.6 });
assert.deepEqual(cropCenterAt([], 5), { x: 0.5, y: 0.5 });

// The raw path the Rust `analyze_crop_path` command returns.
const raw: RawCropPathPoint[] = [
  { time: 0, centerX: 0.2, centerY: 0.4, speakerId: "A", faceIndex: 0, confidence: 0.9, sourceArtifactId: "src", sceneId: 0 },
  { time: 5, centerX: 0.5, centerY: 0.45, speakerId: "A", faceIndex: 0, confidence: 0.85, sourceArtifactId: "src", sceneId: 0 },
  { time: 10, centerX: 0.8, centerY: 0.5, speakerId: null, faceIndex: 0, confidence: 0.7, sourceArtifactId: "src", sceneId: 1 },
];

// A confirmed mapping keeps the speaker link and the real confidence.
const confirmed = cropKeyframesFromPath(raw, {
  speakerFaceMatches: { A: "face-1" },
  mappingConfirmed: true,
  sourceArtifactId: "src",
});
assert.equal(confirmed.length, 3);
assert.deepEqual(
  confirmed.map((key) => [key.time, key.centerX, key.centerY]),
  [[0, 0.2, 0.4], [5, 0.5, 0.45], [10, 0.8, 0.5]],
  "the stored crop must be exactly what the Rust tracker computed",
);
assert.equal(confirmed[0].speakerId, "A");
assert.equal(confirmed[0].sourceFaceId, "face-1");
assert.equal(confirmed[0].confidence, 0.9);
assert.equal(confirmed[0].confirmed, false, "the user still has to approve tracking");
assert.equal(confirmed[2].speakerId, undefined, "a point without a speaker has no speaker link");

// An unconfirmed mapping must not survive the renderer's confidence gate: the renderer
// ignores keyframes below 0.5, so an unapproved mapping falls back to a centre crop
// instead of following a face nobody confirmed.
const unconfirmed = cropKeyframesFromPath(raw, {
  speakerFaceMatches: { A: "face-1" },
  mappingConfirmed: false,
  sourceArtifactId: "src",
});
assert.equal(unconfirmed[0].speakerId, undefined);
assert.equal(unconfirmed[0].sourceFaceId, undefined);
assert.ok(unconfirmed.every((key) => key.confidence < 0.5), "an unconfirmed mapping must stay below the renderer's confidence gate");

// The regression this file exists for: the stored path must not be smoothed a second
// time. A subject that jumps from x=0.2 to x=0.8 must be stored as the tracker computed
// it, not pulled back towards the previous position.
const jumped: RawCropPathPoint[] = [
  { time: 0, centerX: 0.2, centerY: 0.5, speakerId: null, faceIndex: 0, confidence: 0.9, sourceArtifactId: "src", sceneId: 0 },
  { time: 5, centerX: 0.8, centerY: 0.5, speakerId: null, faceIndex: 0, confidence: 0.9, sourceArtifactId: "src", sceneId: 1 },
];
const stored = cropKeyframesFromPath(jumped, { mappingConfirmed: false, sourceArtifactId: "src" });
assert.equal(stored[1].centerX, 0.8, "the crop must follow the tracker, not a second smoothing pass");
assert.ok(Math.abs(stored[1].centerX - 0.8) < 1e-9);

console.log("media engine tests: ok");
