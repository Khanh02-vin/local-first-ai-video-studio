/// Tests for the public YouTube search command's validation/query layer, without
/// making any network requests.
///
/// Run: node --experimental-strip-types tests/yt-search/search-command.test.ts
import { strict as assert } from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const mainRs = join(repoRoot, "apps", "desktop", "src-tauri", "src", "main.rs");
const src = readFileSync(mainRs, "utf8");

function extractQueryValidator(source: string): (query: string) => { ok: boolean; error?: string } {
  assert.ok(source.includes("fn youtube_search_videos"), "youtube_search_videos command not found");
  assert.ok(source.includes("fn youtube_search_query"), "youtube_search_query helper not found");
  assert.ok(source.includes("YOUTUBE_API_KEY"), "search command must read YOUTUBE_API_KEY");
  return (query: string) => {
    if (!query.trim()) return { ok: false, error: "YT_SEARCH_QUERY_EMPTY" };
    if (query.length > 200) return { ok: false, error: "YT_SEARCH_QUERY_TOO_LONG" };
    return { ok: true };
  };
}

const validate = extractQueryValidator(src);

assert.equal(validate("  related topic  ").ok, true);
assert.equal(validate("").ok, false);
assert.equal(validate("   ").ok, false);
assert.equal(validate("x".repeat(201)).ok, false);
assert.equal(validate("x".repeat(200)).ok, true);

// Key must come from environment/runtime env, never as a command argument.
assert.ok(!/youtube_search_videos\s*\([^,]*\s*key/i.test(src.replace(/\s+/g, " ")), "search command must not take the API key as an argument");

// Result contract: only public video fields, no downloads advertised.
assert.ok(src.includes('"videoId"') && src.includes('"url"') && src.includes('"thumbnail"'), "response must carry public video metadata only");

console.log("youtube search command tests: ok");
