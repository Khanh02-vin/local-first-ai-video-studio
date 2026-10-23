/// Frontend wiring tests for related YouTube search.
///
/// Run: node --experimental-strip-types tests/wiring/youtube-search.test.ts
import { strict as assert } from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const mainRs = join(repoRoot, "apps", "desktop", "src-tauri", "src", "main.rs");
// The Rust shell is split into modules; the handler list still lives in main.rs.
const handler = readFileSync(mainRs, "utf8").match(/generate_handler!\[[^\]]*\]/s)?.[0] ?? "";
assert.ok(handler, "generate_handler![...] not found in main.rs");
const registered = new Set([...handler.matchAll(/[a-z_][a-z0-9_]*/g)].map((m) => m[0]));
assert.ok(registered.has("youtube_search_videos"), "youtube_search_videos not registered in Tauri handler");

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.svelte$|\.ts$/.test(entry)) out.push(full);
  }
  return out;
}

const uiRoot = join(repoRoot, "apps", "web", "src");
const uiSource = walk(uiRoot, []).map((file) => readFileSync(file, "utf8")).join("\n");

assert.ok(/youtube_search_videos/.test(uiSource), "Studio UI must call youtube_search_videos");
assert.ok(!/YOUTUBE_API_KEY/.test(uiSource), "frontend must not reference YOUTUBE_API_KEY directly");

console.log("youtube search UI wiring tests: ok");
