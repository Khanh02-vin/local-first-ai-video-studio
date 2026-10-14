import { strict as assert } from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const studio = readFileSync(join(root, "apps/web/src/lib/pages/Studio.svelte"), "utf8");
const bridge = readFileSync(join(root, "apps/desktop/src-tauri/src/main.rs"), "utf8");
const ranking = readFileSync(join(root, "packages/media-engine/related-videos/index.ts"), "utf8");

assert.match(studio, /selectedIds\s*=\s*\$state<string\[]>/, "Studio must support persisted multi-selection");
assert.match(studio, /function toggleCandidate/, "candidate selection must be toggleable");
assert.match(studio, /async function renderSelected/, "selected clips must render as a batch");
assert.match(studio, /plan\s*\}\);/, "render must pass the selected RenderPlan");
assert.match(studio, /selectedIds\s*,\s*plans:/, "multi-selection must be saved on the project");
assert.match(bridge, /plan:\s*Option<serde_json::Value>/, "render command must accept an optional plan");
assert.match(bridge, /drawtext/, "FFmpeg export must overlay hook/captions");
assert.match(ranking, /rankCandidates/, "related results must be reranked from source semantics");

console.log("clip workflow wiring tests: ok");
