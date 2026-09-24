import assert from "node:assert/strict";
import { chooseHeuristicHighlights } from "../../services/ai-pipeline/heuristic.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";

const transcript: Transcript = { version: 1, sourceArtifactId: "source", language: "en", duration: 60, provider: "fixture", words: [], segments: [
  { id: "s1", text: "This is a useful solution.", start: 1, end: 8, wordIds: [] },
  { id: "s2", text: "Short.", start: 10, end: 11, wordIds: [] },
  { id: "s3", text: "The key problem is simple and important.", start: 20, end: 28, wordIds: [] },
] };
const highlights = chooseHeuristicHighlights(transcript);
assert.equal(highlights.length, 2);
assert.equal(highlights[0].start, 1);
assert.ok(highlights.some((item) => item.score >= 50));
console.log("local MVP heuristic tests: ok");
