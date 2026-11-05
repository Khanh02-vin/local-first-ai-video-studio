import assert from "node:assert/strict";
import { resolveHighlightStrategy } from "../../services/ai-pipeline/index.ts";

// Unset / empty / "heuristic" means auto: resolve by available credentials.
assert.equal(resolveHighlightStrategy({}), "heuristic");
assert.equal(resolveHighlightStrategy({ HIGHLIGHT_STRATEGY: "" }), "heuristic");
assert.equal(resolveHighlightStrategy({ HIGHLIGHT_STRATEGY: "  " }), "heuristic");
assert.equal(resolveHighlightStrategy({ HIGHLIGHT_STRATEGY: "heuristic" }), "heuristic");
// Activated OpenAI-compatible provider (all three fields required) wins first.
assert.equal(
  resolveHighlightStrategy({
    HIGHLIGHT_STRATEGY: "heuristic",
    ACTIVE_PROVIDER_BASE_URL: "https://api.openai.com/v1",
    ACTIVE_PROVIDER_API_KEY: "sk-1",
    ACTIVE_PROVIDER_MODEL: "gpt-4o-mini",
    GEMINI_API_KEY: "gem-1",
  }),
  "semantic-openai",
);
// Partial provider config is not enough — fall through to Gemini.
assert.equal(
  resolveHighlightStrategy({
    HIGHLIGHT_STRATEGY: "heuristic",
    ACTIVE_PROVIDER_BASE_URL: "https://api.openai.com/v1",
    ACTIVE_PROVIDER_API_KEY: "  ",
    ACTIVE_PROVIDER_MODEL: "gpt-4o-mini",
  }),
  "heuristic",
);
assert.equal(resolveHighlightStrategy({ HIGHLIGHT_STRATEGY: "heuristic", GEMINI_API_KEY: "gem-1" }), "semantic-gemini");
// Explicit semantic choices pass through even when their credentials are absent
// (the worker's own fallback handles the failure).
assert.equal(resolveHighlightStrategy({ HIGHLIGHT_STRATEGY: "semantic-gemini" }), "semantic-gemini");
assert.equal(resolveHighlightStrategy({ HIGHLIGHT_STRATEGY: "semantic-local" }), "semantic-local");
assert.equal(resolveHighlightStrategy({ HIGHLIGHT_STRATEGY: "semantic-openai" }), "semantic-openai");

console.log("strategy-resolution: all assertions passed");
