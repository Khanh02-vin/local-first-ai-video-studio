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
import { cropCenterAt, resolveFrameByContainment, MIN_PORTRAIT_WIDTH, contains, normalizeSpanBox } from "../../packages/media-engine/reframe/index.ts";
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

// --- Containment solver: P (person) and T (baked-in title/subtitle) decide the frame.
// Rule: whichever box fully contains the other horizontally becomes the frame standard
// ("cái nào chứa trọn cái nào thì lấy cái đó làm chuẩn"); neither contains → union.
const person = { left: 0.30, right: 0.52, top: 0.10, bottom: 0.90 };

// No text: a narrow person stays at the standard full-bleed portrait width.
const noText = resolveFrameByContainment(person);
assert.equal(noText.anchorType, "person");
assert.ok(Math.abs(noText.centerX - 0.41) < 1e-9);
assert.equal(noText.frameWidth, MIN_PORTRAIT_WIDTH);
assert.equal(noText.needsZoomOut, false);

// P contains T in both axes: the person keeps ownership of the frame.
const subtitle = { left: 0.34, right: 0.48, top: 0.30, bottom: 0.50 };
assert.ok(contains(person, subtitle));
const personOwns = resolveFrameByContainment(person, subtitle);
assert.equal(personOwns.anchorType, "person");
assert.ok(Math.abs(personOwns.centerX - 0.41) < 1e-9);
assert.equal(personOwns.frameWidth, MIN_PORTRAIT_WIDTH);

// T contains P in both axes: ownership passes to the text; the frame widens to T.
const wideTitle = { left: 0.02, right: 0.66, top: 0.02, bottom: 0.98 };
assert.ok(contains(wideTitle, person));
const textOwns = resolveFrameByContainment(person, wideTitle);
assert.equal(textOwns.anchorType, "text");
assert.ok(Math.abs(textOwns.centerX - 0.34) < 1e-9);
assert.ok(Math.abs(textOwns.frameWidth - (0.66 - 0.02 + 0.04)) < 1e-9);
assert.equal(textOwns.needsZoomOut, true);

// Neither contains: the union box becomes the frame standard.
const offTitle = { left: 0.70, right: 0.98, top: 0.02, bottom: 0.98 };
const union = resolveFrameByContainment(person, offTitle);
assert.equal(union.anchorType, "union");
assert.ok(Math.abs(union.centerX - (0.30 + 0.98) / 2) < 1e-9);
assert.ok(Math.abs(union.frameWidth - (0.98 - 0.30 + 0.04)) < 1e-9);
assert.ok(union.frameWidth <= 1);

// A wide person on its own also zooms out instead of being cropped away.
const widePerson = resolveFrameByContainment({ left: 0.05, right: 0.60, top: 0.1, bottom: 0.9 });
assert.equal(widePerson.anchorType, "person");
assert.equal(widePerson.needsZoomOut, true);
assert.ok(Math.abs(widePerson.frameWidth - (0.60 - 0.05 + 0.04)) < 1e-9);

// Union wider than the source clamps to 1 rather than overflowing.
const clipped = resolveFrameByContainment(person, { left: 0.0, right: 0.99, top: 0, bottom: 0.2 });
assert.equal(clipped.frameWidth, 1);

// Normalisation: non-finite and inverted coordinates cannot escape 0..1 or flip the box.
assert.deepEqual(normalizeSpanBox({ left: -0.2, right: 1.7, top: 0.3, bottom: 0.3 }), { left: 0, right: 1, top: 0.3, bottom: 0.3 });
assert.deepEqual(normalizeSpanBox({ left: Number.NaN, right: 0.5, top: 0.2, bottom: Number.NaN }), { left: 0, right: 0.5, top: 0.2, bottom: 0.2 });

console.log("media engine tests: ok");
