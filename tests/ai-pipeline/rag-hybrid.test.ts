import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { mkdtempSync, existsSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/**
 * Hybrid RAG check: ingest embeds segments via the local llama-server, query
 * does cosine top-k + LLM answer synthesis, and both fall back cleanly (TF-IDF
 * retrieval / deterministic answer) when the LLM is unreachable.
 *
 * A stub llama-server serves deterministic keyword-axis embeddings and a canned
 * answer, so the whole sidecar module runs offline in CI.
 */

// --- stub llama-server -------------------------------------------------------
const AXES: Array<[RegExp, number]> = [
  [/export|render|mp4|output|render/i, 0],
  [/banana|bread|recipe|walnut/i, 1],
  [/video|clip|footage|timeline/i, 2],
];
function stubEmbed(text: string): number[] {
  const vector = [0.05, 0.05, 0.05, 0.05];
  for (const [pattern, axis] of AXES) if (pattern.test(text)) vector[axis] += 1;
  const norm = Math.sqrt(vector.reduce((sum, n) => sum + n * n, 0));
  return vector.map((n) => n / norm);
}

const stub: Server = createServer((request, response) => {
  let body = "";
  request.on("data", (chunk) => { body += chunk; });
  request.on("end", () => {
    if (request.url === "/v1/embeddings" && request.method === "POST") {
      const parsed = JSON.parse(body) as { input: string | string[] };
      const inputs = Array.isArray(parsed.input) ? parsed.input : [parsed.input];
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify({ data: inputs.map((text, index) => ({ object: "embedding", index, embedding: stubEmbed(text) })) }));
      return;
    }
    if (request.url === "/v1/chat/completions" && request.method === "POST") {
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify({ choices: [{ message: { role: "assistant", content: "STUB-ANSWER: export via the render button." } }] }));
      return;
    }
    response.statusCode = 404;
    response.end("{}");
  });
});
await new Promise<void>((resolve) => stub.listen(0, "127.0.0.1", resolve));
const stubPort = (stub.address() as { port: number }).port;

// --- arrange: temp state dir + env before importing the module ---------------
const state = mkdtempSync(join(tmpdir(), "rag-hybrid-"));
process.env.LOCAL_FIRST_STATE_DIR = state;
process.env.LOCAL_LLM_BASE_URL = `http://127.0.0.1:${stubPort}`;

const { POST, GET } = await import("../../apps/web/src/routes/api/rag/+server.ts");

async function call(method: "GET" | "POST", path: string, payload?: unknown): Promise<Response> {
  if (method === "GET") return GET({ url: new URL(`http://127.0.0.1${path}`) } as Parameters<typeof GET>[0]);
  const request = new Request(`http://127.0.0.1${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(payload) });
  return POST({ request } as Parameters<typeof POST>[0]);
}

// --- ingest: two videos, embeddings must be written --------------------------
const ingestA = await call("POST", "/api/rag/ingest", {
  videoUrl: "https://www.youtube.com/watch?v=vidAAAA1",
  title: "Editing basics",
  segments: [
    { start: 0, end: 6, text: "click export to render the mp4 file" },
    { start: 10, end: 16, text: "the timeline holds your video clip" },
  ],
});
assert.equal(ingestA.status, 200);
const ingestAJson = await ingestA.json() as { embedded: number };
assert.equal(ingestAJson.embedded, 2, "ingest must embed both segments while the LLM is up");

const ingestB = await call("POST", "/api/rag/ingest", {
  videoUrl: "https://www.youtube.com/watch?v=vidBBBB2",
  title: "Baking",
  segments: [{ start: 0, end: 8, text: "banana bread recipe with walnuts" }],
});
assert.equal((await ingestB.json() as { embedded: number }).embedded, 1);

const embeddings = JSON.parse(readFileSync(join(state, "youtube", "embeddings.json"), "utf8")) as { dim: number; vectors: Record<string, number[]> };
assert.equal(embeddings.dim, 4);
assert.equal(Object.keys(embeddings.vectors).length, 3);

// --- query with LLM up: cosine retrieval + LLM answer ------------------------
// Paraphrase with no shared tokens with the export segment ("get my clip out")
// — TF-IDF would miss it; the export-axis embedding must rank it first.
const queryUp = await call("POST", "/api/rag/query", { question: "how do I get my clip out as an mp4", synthesize: true });
const queryUpJson = await queryUp.json() as { answer: string; answer_source: string; retrieval: string; results: Array<{ video_id: string; score: number }> };
assert.equal(queryUpJson.retrieval, "embedding", "query must use cosine retrieval when the LLM is up");
assert.equal(queryUpJson.results[0].video_id, "vidAAAA1", "export video must rank first for an export paraphrase");
assert.equal(queryUpJson.answer_source, "llm");
assert.equal(queryUpJson.answer, "STUB-ANSWER: export via the render button.");

// synthesize:false skips the LLM even when it is available.
const noSynth = await call("POST", "/api/rag/query", { question: "how do I export a video", synthesize: false });
const noSynthJson = await noSynth.json() as { answer_source: string; answer: string };
assert.equal(noSynthJson.answer_source, "deterministic");
assert.match(noSynthJson.answer, /^Found \d+ matching excerpt/);

// --- query with LLM down: TF-IDF fallback + deterministic answer -------------
process.env.LOCAL_LLM_BASE_URL = "http://127.0.0.1:1"; // dead port
const queryDown = await call("POST", "/api/rag/query", { question: "banana bread recipe", synthesize: true });
const queryDownJson = await queryDown.json() as { answer: string; answer_source: string; retrieval: string; results: Array<{ video_id: string }> };
assert.equal(queryDownJson.retrieval, "tfidf", "query must fall back to TF-IDF when the LLM is down");
assert.equal(queryDownJson.results[0].video_id, "vidBBBB2");
assert.equal(queryDownJson.answer_source, "deterministic");
assert.match(queryDownJson.answer, /^Found \d+ matching excerpt/);

// Ingest with the LLM down still indexes (embedded:0) — TF-IDF path unaffected.
const ingestDown = await call("POST", "/api/rag/ingest", {
  videoUrl: "https://www.youtube.com/watch?v=vidCCCC3",
  segments: [{ start: 0, end: 5, text: "timeline zoom shortcuts" }],
});
assert.equal((await ingestDown.json() as { embedded: number }).embedded, 0);

// Playlists listing still works (GET path untouched).
const playlists = await call("GET", "/api/rag/playlists");
assert.equal(playlists.status, 200);

stub.close();
rmSync(state, { recursive: true, force: true });
console.log("rag hybrid tests: ok");
