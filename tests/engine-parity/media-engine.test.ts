import assert from "node:assert/strict";
import { cropWindow } from "../../packages/media-engine/render-plan/index.ts";
import { generateAss } from "../../packages/media-engine/subtitles/index.ts";
import { cropCenterAt, ffmpegCropCenterExpression, trackFacePath } from "../../packages/media-engine/reframe/index.ts";
import { validateMediaInput } from "../../packages/media-engine/validation/index.ts";

validateMediaInput({ duration: 60, width: 1920, height: 1080 });
assert.throws(() => validateMediaInput({ duration: 0, width: 1920, height: 1080 }), /Invalid media duration/);
assert.throws(() => validateMediaInput({ duration: Number.NaN, width: 1920, height: 1080 }), /Invalid media duration/);
assert.throws(() => validateMediaInput({ duration: 60, width: 0, height: 1080 }), /Invalid media dimensions/);
assert.throws(() => validateMediaInput({ duration: 60, width: 1920, height: 1080.5 }), /Invalid media dimensions/);

const tracked = trackFacePath([
  { time: 0, detections: [{ time: 0, x: 0.2, y: 0.2, width: 0.2, height: 0.2, confidence: 0.9 }] },
  { time: 1, detections: [{ time: 1, x: 0.25, y: 0.2, width: 0.2, height: 0.2, confidence: 0.8 }] },
  { time: 3, detections: [] },
]);
assert.equal(tracked.points.length, 3);
assert.equal(tracked.fallbackCount, 1);

const cropKeys = [
  { time: 10, centerX: 0.2, centerY: 0.4, confidence: 0.9, confirmed: true },
  { time: 12, centerX: 0.8, centerY: 0.6, confidence: 0.9, confirmed: true },
];
assert.deepEqual(cropCenterAt(cropKeys, 11), { x: 0.5, y: 0.5 });
assert.match(ffmpegCropCenterExpression(cropKeys, 10), /0\.300000\*t/);
assert.match(ffmpegCropCenterExpression(cropKeys, 10, "y"), /0\.100000\*t/);

const portrait = cropWindow(1920, 1080, "9:16", 0.5);
assert.equal(Math.round(portrait.width / portrait.height * 100), 56);
assert.ok(portrait.x >= 0 && portrait.x + portrait.width <= 1920);

const ass = generateAss({ segments: [{ words: [{ word: "Hello", start: 1, end: 2 }, { word: "world", start: 2, end: 3 }] }] }, 0, 4);
assert.match(ass, /Dialogue: 0/);
assert.match(ass, /Hello world/);

console.log("media engine tests: ok");
