import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const state = readFileSync(join(root, "apps/web/src/lib/studio-state.svelte.ts"), "utf8");
const projects = readFileSync(join(root, "apps/web/src/lib/pages/Project.svelte"), "utf8");
const studio = readFileSync(join(root, "apps/web/src/lib/pages/Studio.svelte"), "utf8");
assert.match(state, /selectedIds:\s*string\[\]/, "shared Studio state must declare selectedIds");
assert.match(projects, /studioState\.selectedIds\s*=\s*project\.selectedIds\?\.length\s*\?\s*project\.selectedIds\s*:\s*\(studioState\.selectedId\s*\?\s*\[studioState\.selectedId\]\s*:\s*\[\]\)/, "Projects page must restore selectedIds with legacy fallback");
assert.match(studio, /studioState\.selectedIds\s*=\s*selectedIds/, "Studio hydration must copy project selectedIds to shared state");
console.log("shared selectedIds wiring tests: ok");
