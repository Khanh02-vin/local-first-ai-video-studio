import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// Wiring contract: every invoke("...") literal in the Svelte/TS UI must be a registered
// Tauri command. A typo here fails silently for users (button does nothing), so it is
// checked mechanically rather than by clicking.

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const uiRoot = join(repoRoot, "apps", "web", "src");
const mainRs = join(repoRoot, "apps", "desktop", "src-tauri", "src", "main.rs");

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.svelte$|\.ts$/.test(entry)) out.push(full);
  }
  return out;
}

// Bundled binary resolution: ffmpeg/ffprobe must be resolved via bundled_bin()
// so production builds (with resources/ populated) work offline without PATH.
const pathsRs = readFileSync(join(repoRoot, "apps", "desktop", "src-tauri", "src", "paths.rs"), "utf8");
assert.match(pathsRs, /pub\(crate\) fn ffmpeg_bin\(app: &tauri::AppHandle\) -> PathBuf/, "ffmpeg_bin must take AppHandle for resource resolution");
assert.match(pathsRs, /bundled_bin\(app, "ffmpeg"/, "ffmpeg must resolve via bundled_bin(resources/ffmpeg)");
assert.match(pathsRs, /pub\(crate\) fn ffprobe_bin\(app: &tauri::AppHandle\) -> PathBuf/, "ffprobe_bin must take AppHandle for resource resolution");
assert.match(pathsRs, /bundled_bin\(app, "ffprobe"/, "ffprobe must resolve via bundled_bin(resources/ffprobe)");
assert.match(pathsRs, /whisper_cpp_bin/, "whisper.cpp binary must resolve from Tauri resources");
assert.match(pathsRs, /whisper_cpp_model/, "whisper.cpp ggml model must resolve from Tauri resources");
assert.match(pathsRs, /WHISPER_ENGINE.*cpp|cpp_default/, "Rust helper env must default to CPP and protect legacy Python values");
assert.match(readFileSync(join(repoRoot, "apps", "desktop", "src-tauri", "tauri.conf.json"), "utf8"), /whisper-cpp|ggml-tiny/, "Tauri must package whisper-cpp and ggml model resources");
const handlerBlock = readFileSync(mainRs, "utf8").match(/generate_handler!\[[^\]]*\]/s)?.[0] ?? "";
assert.ok(handlerBlock, "generate_handler![...] not found in main.rs");
assert.match(handlerBlock, /setup_whisper_cpp/, "System Doctor setup_whisper_cpp command must be registered");
assert.match(handlerBlock, /whisper_cpp_status/, "whisper_cpp_status command must be registered");
assert.match(handlerBlock, /whisper_cpp_smoke/, "whisper_cpp_smoke command must be registered");
assert.match(handlerBlock, /project_delete/, "project delete cascade command must be registered");

const registered = new Set([...handlerBlock.matchAll(/[a-z_][a-z0-9_]*/g)].map((m) => m[0]));

const invoked = new Map<string, string>();
for (const file of walk(uiRoot)) {
  const src = readFileSync(file, "utf8");
  for (const match of src.matchAll(/invoke(?:<[^>]*>)?\(\s*["']([a-z_][a-z0-9_]*)["']/g)) {
    invoked.set(match[1], file);
  }
}

assert.ok(invoked.size >= 10, `expected many invoke() calls, found ${invoked.size}`);

const missing = [...invoked.entries()].filter(([name]) => !registered.has(name));
assert.equal(
  missing.length,
  0,
  "UI invokes commands that are not registered in the Tauri handler: " +
    missing.map(([name, file]) => `${name} (${file.replace(repoRoot + "/", "")})`).join(", ")
);

console.log(`command wiring tests: ok (${invoked.size} UI invocations all registered)`);
