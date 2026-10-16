import assert from "node:assert/strict";
import type { Highlight } from "../../packages/contracts/highlight.ts";
import { validateCollection } from "../../packages/contracts/collection.ts";
import {
  groupHighlightsIntoCollections,
  MAX_CLIPS_PER_COLLECTION,
  type CollectionClusterer,
} from "../../services/ai-pipeline/collections.ts";

const highlight = (id: string, title: string, score: number, extra: Partial<Highlight> = {}): Highlight => ({
  version: 1,
  id,
  sourceArtifactId: "src",
  start: 1,
  end: 10,
  wordIds: [`${id}-w`],
  title,
  score,
  ...extra,
});

const options = { projectId: "p1", now: "2026-01-01T00:00:00Z" };

// 1. LLM clusterer result is used and contract-validated.
const clusterer: CollectionClusterer = () => [
  { title: "Cooking tips", topic: "cooking", highlightIds: ["h1", "h2"] },
  { title: "Knife skills", topic: "knife", highlightIds: ["h3", "h4"] },
];
const fromLlm = await groupHighlightsIntoCollections(
  [highlight("h1", "Boil pasta", 70), highlight("h2", "Salt the water", 65), highlight("h3", "Chop onions", 90), highlight("h4", "Julienne carrots", 85)],
  { ...options, clusterer }
);
assert.equal(fromLlm.length, 2);
assert.equal(fromLlm[0].title, "Cooking tips");
assert.equal(fromLlm[0].state, "proposed");
assert.deepEqual(fromLlm[0].items.map((i) => i.position), [0, 1]);
assert.equal(fromLlm[0].items[0].reference.kind, "highlight");
fromLlm.forEach((c) => validateCollection(c));

// 2. Unknown ids are dropped; the cluster keeps its valid members.
const withUnknown: CollectionClusterer = () => [
  { title: "Valid", highlightIds: ["h1", "h2", "ghost"] },
];
const filtered = await groupHighlightsIntoCollections(
  [highlight("h1", "One", 80), highlight("h2", "Two", 80), highlight("h3", "Three", 80)],
  { ...options, clusterer: withUnknown }
);
assert.equal(filtered.length, 1);
assert.equal(filtered[0].title, "Valid");
assert.equal(filtered[0].items.length, 2);
assert.ok(filtered[0].items.every((i) => i.reference.kind === "highlight" && i.reference.highlightId !== "ghost"));

// 3. A throwing clusterer falls back to shared-token grouping.
const boom: CollectionClusterer = () => { throw new Error("llm down"); };
const shared = await groupHighlightsIntoCollections(
  [highlight("h1", "Roasting coffee beans", 70), highlight("h2", "Grinding coffee beans", 72), highlight("h3", "Steaming milk", 40)],
  { ...options, clusterer: boom }
);
assert.equal(shared.length, 1);
assert.deepEqual(shared[0].items.map((i) => i.reference), [
  { kind: "highlight", highlightId: "h1" },
  { kind: "highlight", highlightId: "h2" },
]);

// 4. No shared tokens -> score tiers; a lone high scorer yields nothing.
const scored = await groupHighlightsIntoCollections(
  [highlight("h1", "Alpha", 90), highlight("h2", "Beta", 85), highlight("h3", "Gamma", 65), highlight("h4", "Delta", 62)],
  options
);
assert.equal(scored.length, 2);
assert.equal(scored[0].topic, "high-score");
assert.equal(scored[1].topic, "medium-score");
const lonely = await groupHighlightsIntoCollections([highlight("h1", "Only one", 99)], options);
assert.deepEqual(lonely, []);

// 5. maxPerCollection caps items.
const many = Array.from({ length: 8 }, (_, i) => highlight(`h${i}`, "Same shared phrase here", 70));
const capped = await groupHighlightsIntoCollections(many, { ...options, maxPerCollection: 3 });
assert.equal(capped[0].items.length, 3);
assert.equal(MAX_CLIPS_PER_COLLECTION, 5);

// 6. Empty input -> no collections.
assert.deepEqual(await groupHighlightsIntoCollections([], options), []);

console.log("collection grouping tests: ok");
