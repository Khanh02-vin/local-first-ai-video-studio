import assert from "node:assert/strict";
import { repairTruncatedJsonArray, extractHighlightProposals } from "../../services/ai-pipeline/providers.ts";

const cases: Array<[string, string]> = [
  ["mid-string", '[{"start":0,"end":10,"wordIds":["w-0","w-1.'],
  ["trailing comma", '[{"start":0,"end":10,"title":"T",'],
  ["dangling key colon", '[{"start":0,"end":10,"title":"T","score":'],
  ["dangling key bare", '[{"start":0,"title":"T","scor'],
  ["completed value kept", '[{"start":1,"end":5,"title":"Useful id'],
  ["complete untouched", '[{"start":1,"end":5,"wordIds":["w-1"],"title":"ok","score":90}]'],
  ["only array", "["],
  // Constrained-decoding envelope shape ({"highlights":[...]}) truncated mid-generation.
  ["envelope mid-item", '{"highlights":[{"start":1,"end":5,"wordIds":["w-1"],'],
  ["envelope mid-string", '{"highlights":[{"start":1,"end":5,"title":"Us'],
  ["envelope only open", '{"highlights":['],
];

for (const [name, input] of cases) {
  const repaired = repairTruncatedJsonArray(input);
  const parsed = JSON.parse(repaired); // must not throw
  assert.ok(Array.isArray(parsed) || typeof parsed === "object", name + " must repair to JSON");
}

// A completed quoted value must survive (never mistaken for a dangling key).
const kept = JSON.parse(repairTruncatedJsonArray('[{"start":1,"end":5,"title":"Useful id'));
assert.equal(kept[0].title, "Useful id");

// A dangling key must be dropped, not left behind.
const dropped = JSON.parse(repairTruncatedJsonArray('[{"start":1,"end":5,"score":'));
assert.ok(!("score" in dropped[0]), "dangling key must be removed");

// extractHighlightProposals accepts both the envelope and a bare array, rejects junk.
const envelope = extractHighlightProposals({ highlights: [{ start: 1, end: 5, title: "T", score: 90 }] });
assert.equal(envelope.length, 1);
const bare = extractHighlightProposals([{ start: 1, end: 5, title: "T", score: 90 }]);
assert.equal(bare.length, 1);
assert.throws(() => extractHighlightProposals({ nope: true }), /LLM_HIGHLIGHT_SHAPE/);

// Envelope repairs still extract after repair (llama.cpp fallback path).
const repairedEnvelope = JSON.parse(repairTruncatedJsonArray('{"highlights":[{"start":1,"end":5,"title":"Us'));
assert.equal(extractHighlightProposals(repairedEnvelope)[0].title, "Us");

console.log("json repair tests: ok");
