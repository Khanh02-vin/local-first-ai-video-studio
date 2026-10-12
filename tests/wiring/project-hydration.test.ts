/// Tests for the project hydration persistence path.
///
/// Run: node --experimental-strip-types tests/wiring/project-hydration.test.ts
import { strict as assert } from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const studio = readFileSync(join(here, "..", "..", "apps", "web", "src", "lib", "pages", "Studio.svelte"), "utf8");
const editor = readFileSync(join(here, "..", "..", "apps", "web", "src", "lib", "pages", "Editor.svelte"), "utf8");

assert.ok(/studioState\.transcript\s*=\s*project\.transcript/.test(studio), "hydrateProject must restore transcript");
assert.ok(/studioState\.plans\s*=\s*project\.plans/.test(studio), "hydrateProject must restore plan map");
assert.ok(/studioState\.selectedId\s*=\s*selectedId/.test(studio), "hydrateProject must restore selected id");
assert.ok(/studioState\.plan\s*=\s*studioState\.plans\[selectedId\]/.test(studio), "hydrateProject must set active plan from map");

assert.ok(/studioState\.plans\[id\]/.test(editor), "Editor must read per-highlight plan on select");
assert.ok(/project_save/.test(editor) && /selectedId/.test(editor) && /plans/.test(editor), "Editor save must persist selected plan back to project");

assert.ok(/studioState\.plans\s*=\s*\{\}/.test(studio), "loadVideo must clear plans");
assert.ok(/inputPath\s*=\s*path/.test(studio), "loadVideo must assign inputPath");

console.log("project hydration persistence tests: ok");
