import assert from "node:assert/strict";
import { transcriptFromCaptions } from "../../services/ai-pipeline/youtube-transcript.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

// Realistic auto-caption shape: short lines, occasional overlap, blanks, tail past duration.
const payload = {
  videoId: "dQw4w9WgXcQ",
  duration: 30,
  language: "en",
  segments: [
    { start: 0.5, end: 3.0, text: "Welcome back to the channel" },
    { start: 2.0, end: 6.0, text: "today we talk about speed" },
    { start: 6.0, end: 7.0, text: "   " },
    { start: 4.0, end: 5.0, text: "fully inside previous" },
    { start: 28, end: 35, text: "tail clipped to duration" },
  ],
};

const transcript = transcriptFromCaptions(payload);
validateTranscript(transcript); // must not throw

assert.equal(transcript.provider, "youtube-captions");
assert.equal(transcript.sourceArtifactId, payload.videoId);
assert.equal(transcript.duration, payload.duration);
assert.ok(transcript.words.length > 0, "pseudo-words must be produced");

// Blank and fully-overlapping segments are dropped; overlap is clamped, tail is clipped.
assert.equal(transcript.segments.length, 3);
assert.ok(transcript.segments.every((segment) => segment.text.trim().length > 0));
assert.equal(transcript.segments.at(-1)!.end, payload.duration);

// Ordering: words never start before the previous word ends, all inside [0, duration].
for (const [index, word] of transcript.words.entries()) {
  assert.ok(word.start >= 0 && word.end <= payload.duration && word.end > word.start, `word ${index} out of bounds`);
  if (index > 0) assert.ok(word.start >= transcript.words[index - 1].end, `word ${index} overlaps previous`);
}

// Referential integrity: every segment.wordIds entry resolves, and each segment's
// words span exactly its own bounds (so highlight wordIds stay seek-accurate).
const byId = new Map(transcript.words.map((word) => [word.id, word]));
for (const segment of transcript.segments) {
  assert.ok(segment.wordIds.length > 0, `${segment.id} has no words`);
  for (const id of segment.wordIds) assert.ok(byId.has(id), `${segment.id} references unknown word ${id}`);
  assert.equal(byId.get(segment.wordIds[0])!.start, segment.start);
  assert.equal(byId.get(segment.wordIds.at(-1)!)!.end, segment.end);
}

console.log("youtube transcript tests: ok");
