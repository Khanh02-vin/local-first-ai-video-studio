import assert from "node:assert/strict";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { generateSemanticHighlights, createMockSemanticProvider, NotImplementedSemanticProvider } from "../../services/ai-pipeline/semantic.ts";

const makeTranscript = (duration = 60): Transcript => ({
  version: 1,
  sourceArtifactId: "src",
  language: "en",
  duration,
  provider: "test",
  words: [
    { id: "w1", text: "This", start: 1, end: 1.4 },
    { id: "w2", text: "is", start: 1.4, end: 1.7 },
    { id: "w3", text: "a", start: 1.7, end: 1.9 },
    { id: "w4", text: "useful", start: 1.9, end: 2.4 },
    { id: "w5", text: "idea", start: 2.4, end: 3 },
    { id: "w6", text: "The", start: 10, end: 10.3 },
    { id: "w7", text: "problem", start: 10.3, end: 10.9 },
    { id: "w8", text: "is", start: 10.9, end: 11.1 },
    { id: "w9", text: "simple", start: 11.1, end: 11.7 },
  ],
  segments: [
    { id: "s1", text: "This is a useful idea", start: 1, end: 3, wordIds: ["w1", "w2", "w3", "w4", "w5"] },
    { id: "s2", text: "The problem is simple", start: 10, end: 11.7, wordIds: ["w6", "w7", "w8", "w9"] },
  ],
});

const transcript = makeTranscript();

const validProposals = [
  { start: 1, end: 3, wordIds: ["w1", "w2", "w3", "w4", "w5"], title: "Useful idea", score: 85 },
  { start: 10, end: 11.7, wordIds: ["w6", "w7", "w8", "w9"], title: "Problem is simple", score: 90 },
];

// Provider configured -> returns semantic highlights
const mockProvider = createMockSemanticProvider(validProposals);
const semanticHighlights = await generateSemanticHighlights(transcript, mockProvider, { limit: 5 });
assert.equal(semanticHighlights.length, 2);
assert.equal(semanticHighlights[0].start, 1);
assert.equal(semanticHighlights[1].start, 10);
assert.ok(semanticHighlights[0].id.startsWith("semantic-"));
assert.ok(semanticHighlights.every((h) => h.sourceArtifactId === "src"));

// NotImplemented provider throws
const notImplemented = new NotImplementedSemanticProvider();
await assert.rejects(() => generateSemanticHighlights(transcript, notImplemented), /SEMANTIC_HIGHLIGHT_NOT_IMPLEMENTED/);

// Empty proposals -> empty highlights
const emptyProvider = createMockSemanticProvider([]);
const emptyHighlights = await generateSemanticHighlights(transcript, emptyProvider);
assert.deepEqual(emptyHighlights, []);

// Out-of-range proposals are filtered
const outOfRange = [
  { start: 100, end: 200, wordIds: ["w1"], title: "Too far", score: 50 },
  { start: 1, end: 3, wordIds: ["w1", "w2", "w3", "w4", "w5"], title: "Good", score: 70 },
];
const filteredProvider = createMockSemanticProvider(outOfRange);
const filteredHighlights = await generateSemanticHighlights(transcript, filteredProvider);
assert.equal(filteredHighlights.length, 1);
assert.equal(filteredHighlights[0].title, "Good");

// Overlapping proposals -> deoverlap picks highest score
const overlapping = [
  { start: 1, end: 5, wordIds: ["w1", "w2", "w3", "w4", "w5"], title: "Longer", score: 60 },
  { start: 2, end: 4, wordIds: ["w2", "w3", "w4"], title: "Shorter better", score: 95 },
];
const overlapProvider = createMockSemanticProvider(overlapping);
const overlapHighlights = await generateSemanticHighlights(transcript, overlapProvider);
assert.equal(overlapHighlights.length, 1);
assert.equal(overlapHighlights[0].title, "Shorter better");

// Unknown wordIds in provider highlights pass through (current behavior - validator doesn't check wordIds)
const unknownWords = [
  { start: 1, end: 3, wordIds: ["w999"], title: "Unknown words", score: 80 },
];
const unknownProvider = createMockSemanticProvider(unknownWords);
const unknownHighlights = await generateSemanticHighlights(transcript, unknownProvider);
assert.equal(unknownHighlights.length, 1);
assert.equal(unknownHighlights[0].title, "Unknown words");
assert.deepEqual(unknownHighlights[0].wordIds, ["w999"]);

// Duration filters - test via rawProposals path since normalizeProposals applies filters
const durationFiltered = [
  { start: 1, end: 5, wordIds: ["w1", "w2", "w3", "w4", "w5"], title: "Too short", score: 80 },
  { start: 10, end: 50, wordIds: ["w1", "w2", "w3", "w4", "w5"], title: "Too long", score: 80 },
  { start: 1, end: 15, wordIds: ["w1", "w2", "w3", "w4", "w5"], title: "Just right", score: 80 },
];
const durationProvider = {
  async generate({ transcript }: { transcript: typeof transcript }) {
    const proposals = durationFiltered.filter(
      (p) => Number.isFinite(p.start) && Number.isFinite(p.end) && p.end <= transcript.duration
    );
    return { highlights: [], provider: "mock-duration", rawProposals: proposals };
  },
};
const durationHighlights = await generateSemanticHighlights(transcript, durationProvider, { minDuration: 10, maxDuration: 60 });
assert.equal(durationHighlights.length, 1);
assert.equal(durationHighlights[0].title, "Just right");

// Limit is respected
const many = Array.from({ length: 10 }, (_, i) => ({
  start: 1 + i * 5,
  end: 3 + i * 5,
  wordIds: ["w1", "w2", "w3"],
  title: `Clip ${i}`,
  score: 80 + i,
}));
const limitProvider = createMockSemanticProvider(many);
const limitedHighlights = await generateSemanticHighlights(transcript, limitProvider, { limit: 3 });
assert.equal(limitedHighlights.length, 3);

console.log("semantic highlight tests: ok");
// Adapter forwards window options to the inner HighlightProvider (real windowing wiring).
import { SemanticProviderAdapter as Adapter } from "../../services/ai-pipeline/semantic.ts";
let received: { windowSeconds?: number; overlapSeconds?: number } | null = null;
const spyProvider = {
  async choose(input: { transcript: Transcript; windowSeconds?: number; overlapSeconds?: number }) {
    received = { windowSeconds: input.windowSeconds, overlapSeconds: input.overlapSeconds };
    return [];
  },
};
const adapter = new Adapter(spyProvider, "spy");
await adapter.generate({ transcript, options: { windowSeconds: 120, windowOverlapSeconds: 15 } });
assert.deepEqual(received, { windowSeconds: 120, overlapSeconds: 15 });
// Omitted options are forwarded as undefined; the 180/30 defaults live in the providers.
received = null;
await adapter.generate({ transcript });
assert.deepEqual(received, { windowSeconds: undefined, overlapSeconds: undefined });

// Provider-side defaults: highlightWindows slices a long transcript at 180s/30s.
import { highlightWindows } from "../../services/ai-pipeline/providers.ts";
const defaultWindows = highlightWindows(makeTranscript(600));
assert.ok(defaultWindows.length >= 4, `expected ≥4 windows for 600s at 180s/30s, got ${defaultWindows.length}`);
assert.ok(defaultWindows.every((w) => w.to - w.from <= 180), "windows never exceed 180s");

console.log("semantic tests: ok");
