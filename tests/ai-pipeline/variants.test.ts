import assert from "node:assert/strict";
import { mapEditVariantProposals, editVariantPrompt, type EditVariant } from "../../services/ai-pipeline/variants.ts";
import { createEditorStateFromVariant } from "../../packages/editor-core/index.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";

const transcript: Transcript = {
  version: 1,
  sourceArtifactId: "source-1",
  language: "en",
  duration: 18,
  provider: "test",
  words: [
    { id: "w0", text: "First", start: 0, end: 1 },
    { id: "w1", text: "idea", start: 1.1, end: 2 },
    { id: "w2", text: "then", start: 5, end: 6 },
    { id: "w3", text: "proof", start: 6, end: 7 },
    { id: "w4", text: "finally", start: 12, end: 13 },
  ],
  segments: [
    { id: "s0", text: "First idea", start: 0, end: 2, wordIds: ["w0", "w1"] },
    { id: "s1", text: "then proof", start: 5, end: 7, wordIds: ["w2", "w3"] },
    { id: "s2", text: "finally", start: 12, end: 13, wordIds: ["w4"] },
  ],
};

const parsed = {
  variants: [
    { title: "Reordered", reason: "Lead with the evidence", ranges: [{ from: 2, to: 3 }, { from: 0, to: 1 }] },
    { title: "Short", ranges: [{ from: 4, to: 4 }] },
    { title: "Invalid indices", ranges: [{ from: -1, to: 2 }] },
    { title: "Overlap", ranges: [{ from: 0, to: 2 }, { from: 2, to: 3 }] },
    { title: "Fraction", ranges: [{ from: 0.5, to: 1 }] },
    { title: "Text index", ranges: [{ from: "0", to: 1 }] },
    { title: "Empty", ranges: [] },
  ],
};

const variants = mapEditVariantProposals(transcript, parsed);
assert.equal(variants.length, 2);
assert.deepEqual(variants[0].ranges, [
  { start: 5, end: 7, wordIds: ["w2", "w3"] },
  { start: 0, end: 2, wordIds: ["w0", "w1"] },
]);
assert.equal(variants[0].sourceArtifactId, transcript.sourceArtifactId);
assert.deepEqual(variants[1].ranges[0].wordIds, ["w4"]);
assert.match(editVariantPrompt(transcript), /inclusive word indices/);
assert.match(editVariantPrompt(transcript), /do not include timestamps or word IDs/);
assert.equal(mapEditVariantProposals(transcript, { variants: [] }).length, 0);
assert.throws(() => mapEditVariantProposals(transcript, { variants: "bad" }), /LLM_VARIANT_SHAPE/);

const state = createEditorStateFromVariant(transcript, variants[0]);
assert.equal(state.plan.sourceArtifactId, transcript.sourceArtifactId);
assert.deepEqual(state.plan.ranges.map(({ start, end }) => ({ start, end })), [{ start: 5, end: 7 }, { start: 0, end: 2 }]);
assert.deepEqual(state.plan.captions.map((caption) => caption.text), ["then proof", "First idea"]);
console.log("transcript variant tests: ok");
