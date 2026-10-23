/// Fails when the committed TypeScript contract no longer matches the Rust
/// declarations it is generated from.
///
/// This is the guard for the whole class of bug where a hand-written TypeScript
/// shape and a hand-written Rust struct disagree about field names — the probe_video
/// breakage (TypeScript expected camelCase, Rust and ffprobe sent snake_case) was
/// exactly that, and it was invisible to every existing test because nothing
/// compared the two sides.
///
/// It regenerates into a scratch directory and diffs, so the committed file is never
/// rewritten by the test and drift is genuinely detected. The check fails loudly
/// rather than skipping when the typeshare CLI is unavailable: a silently skipped
/// check would reopen the exact blind spot this exists to close.
///
/// Run: node --experimental-strip-types tests/contracts/published.test.ts
import { strict as assert } from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { readdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const published = join(root, "packages/contracts/generated/generated.ts");
// The shared contract structs live in the render engine crate: the desktop app and the
// headless CLI are two front doors to the same engine, so they must speak one contract.
const rustDir = join(root, "apps/desktop/src-tauri/crates/render-engine/src");
const scratch = await mkdtemp(join(tmpdir(), "typeshare-parity-"));

try {
  // The Rust shell is split into modules, so the shared structs can live in any of them.
  const rust = readdirSync(rustDir)
    .filter((name) => name.endsWith(".rs"))
    .map((name) => readFileSync(join(rustDir, name), "utf8"))
    .join("\n");
  // Only an attribute directly above a struct counts: the marker can appear in prose
  // (comments, docs) without declaring anything shared.
  const shared = [...rust.matchAll(/#\[typeshare\](?:\s*\n\s*#\[[^\]]*\])*\s*\n\s*(?:pub(?:\(crate\))?\s+)?struct (\w+)/g)].map((match) => match[1]);
  assert.ok(shared.length > 0, "no #[typeshare] structs found under apps/desktop/src-tauri/crates/render-engine/src — the boundary contract is no longer declared anywhere");

  const contract = await readFile(published, "utf8");
  for (const name of shared) {
    assert.match(contract, new RegExp(`export interface ${name}\\b`), `the published contract is missing ${name}, which Rust marks as shared — run \`npm run contracts:generate\``);
  }

  // Regenerate somewhere throwaway: comparing against the committed file only means
  // something if the generator did not just overwrite it. The npm script owns the
  // pinned-CLI install, so this runs the same code path a developer would.
  execFileSync("npm", ["run", "--silent", "contracts:generate", "--", "--output-file", join(scratch, "generated.ts")], {
    cwd: root,
    stdio: "pipe",
  });

  const fresh = await readFile(join(scratch, "generated.ts"), "utf8");
  assert.equal(
    fresh,
    contract,
    "the committed contract has drifted from the Rust declarations. Run `npm run contracts:generate` and commit the result, updating whatever depended on the old shape.",
  );

  const sharedCount = [...contract.matchAll(/export interface (\w+)/g)].length;
  console.log(`published contract tests: ok (${sharedCount} shared Rust types match the committed TypeScript)`);
} finally {
  await rm(scratch, { recursive: true, force: true });
}
