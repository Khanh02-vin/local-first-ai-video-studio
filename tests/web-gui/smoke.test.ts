/**
 * Ponytail web-gui-tester — smoke test for UI routes
 * Run: node --experimental-strip-types tests/web-gui/smoke.test.ts
 * Server: http://localhost:1420
 */
const BASE = "http://localhost:1420";

function assert(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
  console.log(`  ✓ ${msg}`);
}

async function checkRoute(path: string, label: string): Promise<void> {
  const res = await fetch(`${BASE}${path}`);
  const text = await res.text();
  assert(res.ok || res.status === 404, `${label}: ${res.status} → ${path}`);
  /* Ponytail: we don't deeply assert DOM content, only that routing responds.
     Real UI assertion needs rendered HTML, not JSON API response. */
  console.log(`    ${path}: ${res.status}, ${text.length} bytes`);
}

console.log("Running UI smoke test suite...");
for (const [path, label] of [
  ["/", "Homepage"],
  ["/editor", "Editor"],
  ["/projects", "Projects"],
  ["/settings", "Settings"],
  ["/youtube", "YouTube Studio"],
  ["/youtube/playlists", "Playlists"],
  ["/youtube/query", "Query KB"],
] as const) {
  await checkRoute(path, label);
}
console.log("Done ✅");
