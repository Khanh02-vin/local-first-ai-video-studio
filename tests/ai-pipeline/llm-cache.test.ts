import assert from "node:assert/strict";
import { mkdtempSync, existsSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { CachedSemanticProvider, llmCacheKey } from "../../services/ai-pipeline/llm-cache.ts";
import { HIGHLIGHT_PROMPT_VERSION } from "../../services/ai-pipeline/providers.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import type { SemanticHighlightProvider, SemanticHighlightResult } from "../../services/ai-pipeline/semantic.ts";

/**
 * LLM result cache: same transcript + prompt version replays stored highlights
 * without re-calling the inner provider; model/options/transcript changes miss.
 */

const state = mkdtempSync(join(tmpdir(), "llm-cache-"));
process.env.LOCAL_FIRST_STATE_DIR = state;

const transcript: Transcript = {
  version: 1, sourceArtifactId: "src", language: "en", duration: 30, provider: "test",
  words: [
    { id: "w1", text: "one", start: 0, end: 1 }, { id: "w2", text: "two", start: 1, end: 2 },
    { id: "w3", text: "three", start: 2, end: 3 }, { id: "w4", text: "four", start: 3, end: 4 },
  ],
  segments: [{ id: "s1", text: "one two three four", start: 0, end: 4, wordIds: ["w1", "w2", "w3", "w4"] }],
};
const highlights = [{ version: 1 as const, id: "h1", sourceArtifactId: "src", start: 0, end: 4, wordIds: ["w1", "w2"], title: "ok", score: 80 }];

let calls = 0;
const inner: SemanticHighlightProvider = {
  async generate(): Promise<SemanticHighlightResult> { calls += 1; return { highlights, provider: "fake" }; },
};
const cached = new CachedSemanticProvider(inner, "fake:model-a");

// First call reaches the inner provider and writes the cache file.
const first = await cached.generate({ transcript });
assert.equal(calls, 1);
assert.equal(first.highlights[0].title, "ok");
const key = llmCacheKey("fake:model-a", transcript);
assert.ok(existsSync(join(state, "llm-cache", `${key}.json`)), "cache file must exist after first call");

// Second identical call replays from cache — inner provider NOT invoked.
const second = await cached.generate({ transcript });
assert.equal(calls, 1, "identical transcript+options must hit the cache");
assert.deepEqual(second.highlights, first.highlights);

// Different options → miss.
await cached.generate({ transcript, options: { limit: 3 } });
assert.equal(calls, 2, "different options must miss the cache");

// Different model identity → miss.
const otherModel = new CachedSemanticProvider(inner, "fake:model-b");
await otherModel.generate({ transcript });
assert.equal(calls, 3, "different provider identity must miss the cache");

// Different transcript → miss.
const transcript2: Transcript = { ...transcript, words: [...transcript.words, { id: "w5", text: "five", start: 4, end: 5 }], duration: 31 };
await cached.generate({ transcript: transcript2 });
assert.equal(calls, 4, "different transcript must miss the cache");

// Cache key stability: same inputs → same key; any component change → new key.
assert.equal(llmCacheKey("id", transcript), llmCacheKey("id", transcript));
assert.notEqual(llmCacheKey("id", transcript), llmCacheKey("other", transcript));
assert.notEqual(llmCacheKey("id", transcript), llmCacheKey("id", transcript2));
assert.notEqual(llmCacheKey("id", transcript), llmCacheKey("id", transcript, { limit: 3 }));
assert.equal(typeof HIGHLIGHT_PROMPT_VERSION, "string");

// Corrupt cache file → treated as a miss, inner invoked, file rewritten.
writeFileSync(join(state, "llm-cache", `${key}.json`), "{corrupt");
await cached.generate({ transcript });
assert.equal(calls, 5, "corrupt cache entry must be treated as a miss");

// Contract-invalid cached content (overlapping highlights) → miss.
const badKey = llmCacheKey("fake:model-a", transcript, { limit: 99 });
writeFileSync(join(state, "llm-cache", `${badKey}.json`), JSON.stringify({
  highlights: [
    { version: 1, id: "a", sourceArtifactId: "src", start: 0, end: 4, wordIds: [], title: "x", score: 10 },
    { version: 1, id: "b", sourceArtifactId: "src", start: 2, end: 6, wordIds: [], title: "y", score: 10 },
  ],
  identity: "fake:model-a", promptVersion: HIGHLIGHT_PROMPT_VERSION, cachedAt: "",
}));
await cached.generate({ transcript, options: { limit: 99 } });
assert.equal(calls, 6, "contract-invalid cache entry must be treated as a miss");

// Empty provider result is not cached (retries next time).
let emptyCalls = 0;
const emptyInner: SemanticHighlightProvider = {
  async generate(): Promise<SemanticHighlightResult> { emptyCalls += 1; return { highlights: [], provider: "fake" }; },
};
const emptyCached = new CachedSemanticProvider(emptyInner, "fake:empty");
await emptyCached.generate({ transcript });
await emptyCached.generate({ transcript });
assert.equal(emptyCalls, 2, "empty results must not be cached");

rmSync(state, { recursive: true, force: true });
console.log("llm cache tests: ok");
