/**
 * Ponytail web-gui-tester — ultra lazy mode
 * Run test: node tests/web-gui/ingest-query.test.ts
 * Requires: node scripts/mock-rag-server.ts (or real dev server)
 */
import { parse, resolve } from "node:path";

const BASE = "http://localhost:1420";

function assert(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
  console.log(`  ✓ ${msg}`);
}

async function api(path: string, opts: { method?: string; body?: unknown } = {}): Promise<unknown> {
  const res = await fetch(`${BASE}${path}`, {
    method: opts.method ?? "GET",
    headers: { "Content-Type": "application/json" },
    body: opts.body ? JSON.stringify(opts.body) : undefined,
  });
  return { status: res.status, data: await res.json() };
}

console.log("Running ingest + query test suite...");

// Step 1: Health check
const health = await api("/api/rag/playlists");
assert((health as { status: number }).status === 200, "Health check passes");

// Step 2: Ingest playlist
const ingest = await api("/api/rag/playlists", {
  method: "POST",
  body: { playlistUrl: "https://www.youtube.com/watch?v=test123", title: "Test Video" },
});
const ingestData = ingest as { id: string; status: string };
assert(ingestData.status === "indexed", `Playlist ingested: ${JSON.stringify(ingestData)}`);

// Step 3: Verify playlist appears
const playlists = await api("/api/rag/playlists") as { playlists: Array<{ id: string }> };
assert(playlists.playlists.some((p) => p.id === ingestData.id), "Playlist appears in list");

// Step 4: Query knowledge base
const query = await api("/api/rag/query", {
  method: "POST",
  body: { question: "What is this video about?" },
}) as {
  answer: string;
  results: Array<{ video_id: string; text: string }>;
};
assert(query.results.length > 0, "Query returns results");
assert(query.results[0].video_id === "test123", "Result matches ingested video");

console.log("All tests passed ✅");
