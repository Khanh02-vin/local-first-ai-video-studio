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
assert.ok(/studioState\.variants\s*=\s*project\.variants/.test(studio), "hydrateProject must restore transcript variants");
assert.ok(/studioState\.plans\s*=\s*\{\s*\.\.\.studioState\.plans,\s*\[`variant:\$\{editingVariantId\}`\]/.test(editor), "Editor must store variant plans separately from highlight plans");
assert.ok(/project_save/.test(editor) && /variants/.test(editor) && /plans/.test(editor), "Editor save must persist variant plan back to project");

const renderVariant = studio.match(/async function renderVariant\([\s\S]*?\n  \}/)?.[0] ?? "";
const renderSelected = studio.match(/async function renderSelected\([\s\S]*?\n  \}/)?.[0] ?? "";
const hydrateProject = studio.match(/async function hydrateProject\([\s\S]*?\n  \}/)?.[0] ?? "";
const analyze = studio.match(/async function analyze\([\s\S]*?\n  \}/)?.[0] ?? "";
assert.ok(renderVariant.indexOf("persistRender(render)") < renderVariant.indexOf('invoke<string>("render_video"'), "variant render must persist running status before invoking FFmpeg");
assert.match(renderVariant, /persistRender\(\{ \.\.\.render, status: "completed" \}\)/, "variant render must persist completion");
assert.match(renderVariant, /persistRender\(\{ \.\.\.render, status: cancelled \? "cancelled" : "failed", error: String\(error\) \}\)/, "variant render must persist failure or cancellation");
assert.ok(renderSelected.indexOf("persistRender(render)") < renderSelected.indexOf('invoke<string>("render_video"'), "selected render must persist running status before invoking FFmpeg");
assert.match(hydrateProject, /status === "running" \? \{ \.\.\.render, status: "interrupted"(?: as const)? \}/, "project hydration must mark stale render records interrupted");
assert.match(analyze, /jobId = id;[\s\S]*persistProject\(\{ analysisJobId: id \}\)[\s\S]*watch\(id, false\)/, "analysis job ID must be saved before polling");
assert.match(hydrateProject, /analysis_status[\s\S]*job\.status === "completed" && job\.result/, "project hydration must restore an unsaved completed analysis result");
assert.match(studio, /if \(!inputPath \|\| !sourceVerified \|\| !runtimeStore\.ready\)/, "analysis must require a verified source");
assert.match(studio, /if \(!inputPath \|\| !sourceVerified \|\| !ids\.length\)/, "selected render must require a verified source");

assert.ok(/const probe = await invoke<ProbeMetadata>\("probe_video"/.test(studio), "local source load must use typed probe_video metadata");
assert.ok(/item\.codec_type === "video"/.test(studio), "probe stream lookup must read ffprobe's snake_case codec_type");
assert.ok(/if \(!stream \|\| !Number\.isFinite\(duration\) \|\| duration <= 0\)/.test(studio), "local media requires video stream and valid duration");
assert.match(studio, /disabled=\{busy \|\| !sourceVerified\} onclick=\{\(\) => renderVariant\(variant\.id\)\}/, "variant export must be disabled while the source is unverified or work is busy");
assert.match(studio, /disabled=\{busy \|\| !sourceVerified \|\| !selectedIds\.length\}/, "selected export must be disabled while the source is unverified");

console.log("project hydration persistence tests: ok");
