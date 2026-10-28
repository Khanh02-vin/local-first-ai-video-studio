import assert from "node:assert/strict";
import { chooseHeuristicHighlights } from "../../services/ai-pipeline/heuristic.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";

function word(id: string, text: string, start: number, end: number) {
  return { id, text, start, end };
}

function segment(id: string, text: string, start: number, end: number, wordIds: string[]) {
  return { id, text, start, end, wordIds };
}

function buildTranscript(
  sourceArtifactId: string,
  duration: number,
  words: Array<{ id: string; text: string; start: number; end: number }>,
  segments: Array<{ id: string; text: string; start: number; end: number; wordIds: string[] }>,
): Transcript {
  return {
    version: 1,
    sourceArtifactId,
    language: "en",
    duration,
    provider: "test",
    words,
    segments,
  };
}

// 15 seconds of speech, one word every 0.5s.
const WORDS = [
  word("w0", "The", 0.0, 0.5),
  word("w1", "secret", 0.5, 1.0),
  word("w2", "to", 1.0, 1.5),
  word("w3", "editing", 1.5, 2.0),
  word("w4", "fast", 2.0, 2.5),
  word("w5", "is", 2.5, 3.0),
  word("w6", "a", 3.0, 3.5),
  word("w7", "strong", 3.5, 4.0),
  word("w8", "hook", 4.0, 4.5),
  word("w9", "within", 4.5, 5.0),
  word("w10", "the", 5.0, 5.5),
  word("w11", "first", 5.5, 6.0),
  word("w12", "few", 6.0, 6.5),
  word("w13", "seconds", 6.5, 7.0),
  word("w14", "before", 7.0, 7.5),
  word("w15", "viewers", 7.5, 8.0),
  word("w16", "decide", 8.0, 8.5),
  word("w17", "to", 8.5, 9.0),
  word("w18", "leave", 9.0, 9.5),
  word("w19", "Why", 9.5, 10.0),
  word("w20", "does", 10.0, 10.5),
  word("w21", "it", 10.5, 11.0),
  word("w22", "matter", 11.0, 11.5),
  word("w23", "Because", 11.5, 12.0),
  word("w24", "the", 12.0, 12.5),
  word("w25", "important", 12.5, 13.0),
  word("w26", "problem", 13.0, 13.5),
  word("w27", "is", 13.5, 14.0),
  word("w28", "knowing", 14.0, 14.5),
  word("w29", "which", 14.5, 15.0),
];

const SEGMENTS_A = [
  segment("sA0", "The secret to editing fast is a strong hook within the first few seconds before viewers decide to leave", 0.0, 9.5, WORDS.slice(0, 19).map((w) => w.id)),
  segment("sA1", "Why does it matter Because the important problem is knowing which", 9.5, 15.0, WORDS.slice(19).map((w) => w.id)),
];

const SEGMENTS_B = [
  segment("sB0", "The secret to editing fast", 0.0, 2.5, WORDS.slice(0, 5).map((w) => w.id)),
  segment("sB1", "is a strong hook within", 2.5, 5.0, WORDS.slice(5, 10).map((w) => w.id)),
  segment("sB2", "the first few seconds before", 5.0, 7.5, WORDS.slice(10, 15).map((w) => w.id)),
  segment("sB3", "viewers decide to", 7.5, 9.5, WORDS.slice(15, 19).map((w) => w.id)),
  segment("sB4", "Why does it matter", 9.5, 11.5, WORDS.slice(19, 23).map((w) => w.id)),
  segment("sB5", "Because the important problem is knowing which", 11.5, 15.0, WORDS.slice(23).map((w) => w.id)),
];

function iou(a: { start: number; end: number }, b: { start: number; end: number }): number {
  const intersection = Math.max(0, Math.min(a.end, b.end) - Math.max(a.start, b.start));
  const union = Math.max(a.end, b.end) - Math.min(a.start, b.start);
  return union > 0 ? intersection / union : 0;
}

const TRANSCRIPT_A = buildTranscript("artifact-1", 15.0, WORDS, SEGMENTS_A);
const TRANSCRIPT_B = buildTranscript("artifact-1", 15.0, WORDS, SEGMENTS_B);

const highlightsA = chooseHeuristicHighlights(TRANSCRIPT_A);
const highlightsB = chooseHeuristicHighlights(TRANSCRIPT_B);

assert.ok(highlightsA.length >= 1, "segmentation A must yield at least one highlight");
assert.ok(highlightsB.length >= 1, "segmentation B must yield at least one highlight");

// Same highlight count (selector robustness).
assert.strictEqual(
  highlightsA.length,
  highlightsB.length,
  `highlight count differs: A=${highlightsA.length} B=${highlightsB.length}`,
);

// Every highlight from A matches one from B with IoU >= 0.7.
for (const ha of highlightsA) {
  const bestIou = Math.max(0, ...highlightsB.map((hb) => iou(ha, hb)));
  assert.ok(bestIou >= 0.7, `highlight A ${ha.id} (${ha.start}-${ha.end}) has no match >= 0.7 in B, best IoU ${bestIou.toFixed(3)}`);
}

// Every highlight from B matches one from A with IoU >= 0.7.
for (const hb of highlightsB) {
  const bestIou = Math.max(0, ...highlightsA.map((ha) => iou(ha, hb)));
  assert.ok(bestIou >= 0.7, `highlight B ${hb.id} (${hb.start}-${hb.end}) has no match >= 0.7 in A, best IoU ${bestIou.toFixed(3)}`);
}

// Both transcripts remain valid contracts.
validateHighlights(highlightsA, TRANSCRIPT_A.duration);
validateHighlights(highlightsB, TRANSCRIPT_B.duration);

console.log(
  `heuristic segmentation-robust: ok (${highlightsA.length} highlights, same across segmentations)`,
);
