import { strict as assert } from "node:assert/strict";
import { rankCandidates } from "../../packages/media-engine/related-videos/index.ts";

const ranked = rankCandidates(
  [1, 0],
  [[0.1, 0.9], [0.99, 0.01]],
  [{ videoId: "one", title: "one", thumbnail: "", url: "" }, { videoId: "two", title: "two", thumbnail: "", url: "" }],
);
assert.deepEqual(ranked.map((video) => video.videoId), ["two", "one"]);
assert.deepEqual(rankCandidates([], [], [{ videoId: "fallback", title: "fallback", thumbnail: "", url: "" }]).map((video) => video.videoId), ["fallback"]);
console.log("related video ranking tests: ok");
