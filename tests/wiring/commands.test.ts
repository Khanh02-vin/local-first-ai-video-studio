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

const handlerBlock = readFileSync(mainRs, "utf8").match(/generate_handler!\[[^\]]*\]/s)?.[0] ?? "";
assert.ok(handlerBlock, "generate_handler![...] not found in main.rs");
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
