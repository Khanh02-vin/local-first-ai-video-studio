import type { RequestHandler } from "@sveltejs/kit";
import { mkdirSync, readFileSync, writeFileSync, existsSync, readdirSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";

/**
 * Local YouTube knowledge base, served entirely from the client's machine.
 *
 * Data lives under the local-first state directory (overridable via
 * LOCAL_FIRST_STATE_DIR, matching the desktop app):
 *   <state>/youtube/playlists/<id>.json   — playlist crawl records
 *   <state>/youtube/videos/<id>.json      — per-video transcript segments
 *   <state>/youtube/index.json            — TF-IDF index over all segments
 *   <state>/youtube/embeddings.json       — per-segment vectors (llama-server)
 *
 * Retrieval is hybrid: cosine top-k over stored embeddings when the local
 * llama-server (LOCAL_LLM_BASE_URL) is reachable, TF-IDF otherwise. Answer
 * synthesis likewise prefers the local LLM and falls back to a deterministic
 * excerpt summary, so the service keeps working with no LLM running.
 *
 * SvelteKit registers one route entry per filesystem path: a +server.ts at
 * the literal /api/rag path would only dispatch that exact URL, while the
 * YouTube Studio UI fetches sub-paths (/api/rag/playlists, /ingest, /query,
 * /index, DELETE /api/rag/playlists/<id>). The rest-parameter sibling at
 * src/routes/api/[...rest]/+server.ts claims the whole /api/* subtree and
 * re-exports this module, so under adapter-node those sub-paths reach the
 * handlers instead of the SPA catch-all.
 *
 * Every endpoint validates its inputs strictly and fails with a 400 JSON
 * body rather than a stack trace, so the UI can display the error as-is.
 */

function dataDir(): string {
  const state = process.env.LOCAL_FIRST_STATE_DIR ?? join(process.env.HOME ?? ".", ".cache", "local-first-ai-video-studio");
  const dir = join(state, "youtube");
  mkdirSync(join(dir, "playlists"), { recursive: true });
  mkdirSync(join(dir, "videos"), { recursive: true });
  return dir;
}

type Segment = { start: number; end: number; text: string };
type VideoDoc = {
  videoId: string;
  playlistId: string;
  title: string | null;
  topic: string | null;
  durationSeconds: number;
  segments: Segment[];
  indexedAt: string;
};
type PlaylistDoc = {
  id: string;
  playlistId: string;
  playlistUrl: string;
  title: string | null;
  status: "queued" | "indexing" | "indexed" | "failed";
  createdAt: string;
  updatedAt: string;
};

function loadPlaylist(id: string): PlaylistDoc | null {
  const file = join(dataDir() + "/playlists", `${id}.json`);
  if (!existsSync(file)) return null;
  return JSON.parse(readFileSync(file, "utf8")) as PlaylistDoc;
}
function savePlaylist(doc: PlaylistDoc): void {
  writeFileSync(join(dataDir() + "/playlists", `${doc.id}.json`), JSON.stringify(doc, null, 2));
}
function loadVideo(videoId: string): VideoDoc | null {
  const file = join(dataDir() + "/videos", `${videoId}.json`);
  if (!existsSync(file)) return null;
  return JSON.parse(readFileSync(file, "utf8")) as VideoDoc;
}
function saveVideo(doc: VideoDoc): void {
  writeFileSync(join(dataDir() + "/videos", `${doc.videoId}.json`), JSON.stringify(doc, null, 2));
}

function parseVideoIdFromUrl(url: string): string | null {
  const match = url.match(/(?:youtube\.com\/(?:watch\?v=|embed\/|shorts\/)|youtu\.be\/)([\w-]{6,})/i)
    ?? url.match(/list=([\w-]+)/i) ?? url.match(/videoId=([\w-]{6,})/i);
  return match?.[1] ?? null;
}

function isValidVideoId(id: string): boolean {
  return /^[\w-]{6,}$/.test(id);
}

function tokenize(text: string): string[] {
  return text.toLowerCase().split(/\W+/).filter((t) => t.length > 2);
}

function loadIndex(): Record<string, Record<string, number>> {
  const file = join(dataDir(), "index.json");
  if (!existsSync(file)) return {};
  return JSON.parse(readFileSync(file, "utf8")) as Record<string, Record<string, number>>;
}

function rebuildIndex(): void {
  const index: Record<string, Record<string, number>> = {};
  const dir = dataDir() + "/videos";
  for (const name of readdirSync(dir)) {
    if (!name.endsWith(".json")) continue;
    const doc = loadVideo(name.replace(/\.json$/, ""));
    if (!doc) continue;
    for (const segment of doc.segments) {
      const key = `${doc.videoId}:${segment.start.toFixed(1)}`;
      index[key] ??= {};
      for (const token of tokenize(segment.text)) index[key][token] = (index[key][token] ?? 0) + 1;
    }
  }
  writeFileSync(join(dataDir(), "index.json"), JSON.stringify(index));
}

// --- local LLM client (llama-server) for embeddings + answer synthesis ---
// The desktop app spawns llama-server with --embeddings --pooling mean, so the
// same bundled Qwen model serves /v1/embeddings and /v1/chat/completions. Every
// helper here is best-effort: when the server is unreachable the callers fall
// back to TF-IDF retrieval and the deterministic answer string.

function llmBase(): string {
  return (process.env.LOCAL_LLM_BASE_URL ?? "http://127.0.0.1:8080").replace(/\/+$/, "");
}

type EmbeddingStore = { version: 1; model: string; dim: number; vectors: Record<string, number[]> };

function loadEmbeddings(): EmbeddingStore | null {
  const file = join(dataDir(), "embeddings.json");
  if (!existsSync(file)) return null;
  try {
    const parsed = JSON.parse(readFileSync(file, "utf8")) as EmbeddingStore;
    return parsed?.version === 1 && parsed.vectors && typeof parsed.vectors === "object" ? parsed : null;
  } catch { return null; }
}
function saveEmbeddings(store: EmbeddingStore): void {
  writeFileSync(join(dataDir(), "embeddings.json"), JSON.stringify(store));
}

/** Embeds texts via the local llama-server. Returns null when the server is down
 *  or replies with a shape/dim mismatch (e.g. model swapped → incompatible dim). */
async function embedTexts(texts: string[]): Promise<{ dim: number; vectors: number[][] } | null> {
  if (!texts.length) return { dim: 0, vectors: [] };
  try {
    const response = await fetch(`${llmBase()}/v1/embeddings`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ model: "local", input: texts }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!response.ok) return null;
    const raw = await response.json() as { data?: Array<{ embedding?: number[] }> };
    const vectors = (raw.data ?? []).map((entry) => entry.embedding ?? []);
    if (vectors.length !== texts.length || !vectors[0]?.length) return null;
    const dim = vectors[0].length;
    if (vectors.some((vector) => vector.length !== dim || vector.some((n) => typeof n !== "number" || !Number.isFinite(n)))) return null;
    return { dim, vectors };
  } catch { return null; }
}

function cosine(a: number[], b: number[]): number {
  let dot = 0, na = 0, nb = 0;
  for (let i = 0; i < a.length; i++) { dot += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i]; }
  if (!na || !nb) return 0;
  return dot / (Math.sqrt(na) * Math.sqrt(nb));
}

const EMBED_BATCH = 32;

/** Embeds this video's segments and merges its vectors into embeddings.json
 *  (replacing any previous vectors for the same videoId). Best-effort: returns
 *  the number of embedded segments, 0 when the LLM is unreachable. */
async function embedVideoSegments(videoId: string, segments: Segment[]): Promise<number> {
  const embedded: Record<string, number[]> = {};
  let dim = 0;
  for (let i = 0; i < segments.length; i += EMBED_BATCH) {
    const batch = segments.slice(i, i + EMBED_BATCH);
    const result = await embedTexts(batch.map((s) => s.text));
    if (!result) break; // keep whatever embedded before the failure
    dim = result.dim;
    batch.forEach((segment, j) => { embedded[`${videoId}:${segment.start.toFixed(1)}`] = result.vectors[j].map((n) => Math.round(n * 10000) / 10000); });
  }
  const count = Object.keys(embedded).length;
  if (!count) return 0;
  const store = loadEmbeddings() ?? { version: 1, model: "local", dim, vectors: {} };
  // A dim change means the loaded model changed: stored vectors are incomparable.
  if (store.dim !== dim) { store.dim = dim; store.vectors = {}; }
  for (const key of Object.keys(store.vectors)) if (key.startsWith(`${videoId}:`)) delete store.vectors[key];
  Object.assign(store.vectors, embedded);
  saveEmbeddings(store);
  return count;
}

/** Asks the local LLM to answer from the top excerpts. Returns null on any
 *  failure/timeout so the caller can fall back to the deterministic string. */
async function synthesizeAnswer(question: string, top: Array<{ videoId: string; segment: Segment }>): Promise<string | null> {
  const excerpts = top.map((t, i) => `[${i + 1}] (${t.videoId} @ ${Math.round(t.segment.start)}s) ${t.segment.text}`).join("\n");
  const prompt = `Answer the question using ONLY the transcript excerpts below. If the excerpts do not contain the answer, say so. Reply in the same language as the question.\n\nQuestion: ${question}\n\nExcerpts:\n${excerpts}`;
  try {
    const response = await fetch(`${llmBase()}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ model: "local", temperature: 0.2, max_tokens: 300, messages: [{ role: "user", content: prompt }] }),
      signal: AbortSignal.timeout(90_000),
    });
    if (!response.ok) return null;
    const raw = await response.json() as { choices?: Array<{ message?: { content?: string } }> };
    const text = raw.choices?.[0]?.message?.content?.trim();
    return text || null;
  } catch { return null; }
}

/** Deterministic excerpt summary — the offline answer when no LLM is available. */
function deterministicAnswer(top: Array<{ videoId: string; segment: Segment; score: number }>): string {
  return top.length
    ? `Found ${top.length} matching excerpt${top.length === 1 ? "" : "s"} across ${new Set(top.map((t) => t.videoId)).size} video(s): ${top.map((t) => `"${t.segment.text.slice(0, 60)}${t.segment.text.length > 60 ? "…" : ""}" (${t.videoId} @ ${Math.round(t.segment.start)}s, ${(t.score * 100).toFixed(0)}% match)`).join("; ")}`
    : "No transcript excerpts matched the question — try different wording or ingest more videos.";
}

// --- endpoints ---

export const GET: RequestHandler = async ({ url }) => {
  // The [...rest] route (src/routes/api/[...rest]) dispatches any /api/* path
  // to this module. Match the exact /api/rag/* sub-paths only, so an unrelated
  // /api/foo request still gets a JSON 404 instead of the SPA HTML fallback.
  if (url.pathname === "/api/rag/playlists") {
    const dir = dataDir() + "/playlists";
    const playlists = readdirSync(dir)
      .filter((n) => n.endsWith(".json"))
      .map((n) => loadPlaylist(n.replace(/\.json$/, "")))
      .filter((p): p is PlaylistDoc => p !== null);
    playlists.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    return Response.json({ playlists });
  }
  if (url.pathname === "/api/rag/index") return Response.json(loadIndex());
  return jsonError(404, "UNKNOWN_ENDPOINT");
};

export const POST: RequestHandler = async ({ request }) => {
  const { pathname } = new URL(request.url);

  if (pathname === "/api/rag/playlists") {
    let body: unknown;
    try { body = await request.json(); } catch { return jsonError(400, "INVALID_JSON"); }
    const playlistUrl = (body as { playlistUrl?: string })?.playlistUrl;
    const title = (body as { title?: string })?.title;
    if (typeof playlistUrl !== "string" || !/^https?:\/\/(www\.)?(youtube\.com|youtu\.be)\/?/.test(playlistUrl)) {
      return jsonError(400, "PLAYLIST_MUST_BE_YOUTUBE_URL");
    }
    const id = `pl-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const now = new Date().toISOString();
    savePlaylist({ id, playlistId: id, playlistUrl, title: typeof title === "string" ? title : null, status: "queued", createdAt: now, updatedAt: now });
    return Response.json({ id, status: "queued" }, { status: 202 });
  }

  if (pathname === "/api/rag/playlists/status") {
    let body: unknown;
    try { body = await request.json(); } catch { return jsonError(400, "INVALID_JSON"); }
    const { id: playlistRecordId, status: newStatus } = body as { id?: string; status?: string };
    if (typeof playlistRecordId !== "string" || typeof newStatus !== "string" || !["indexing", "indexed", "failed"].includes(newStatus)) {
      return jsonError(400, "INVALID_STATUS_UPDATE");
    }
    const doc = loadPlaylist(playlistRecordId);
    if (!doc) return jsonError(404, "PLAYLIST_NOT_FOUND");
    doc.status = newStatus as PlaylistDoc["status"];
    doc.updatedAt = new Date().toISOString();
    savePlaylist(doc);
    return Response.json({ id: doc.id, status: doc.status });
  }

  if (pathname === "/api/rag/ingest") {
    let body: unknown;
    try { body = await request.json(); } catch { return jsonError(400, "INVALID_JSON"); }
    const { videoUrl, title, segments } = body as { videoUrl?: string; title?: string; segments?: Segment[] };
    if (typeof videoUrl !== "string") return jsonError(400, "VIDEO_URL_REQUIRED");
    const videoId = parseVideoIdFromUrl(videoUrl);
    if (!videoId || !isValidVideoId(videoId)) return jsonError(400, "UNPARSEABLE_VIDEO_ID");
    if (!Array.isArray(segments) || !segments.length) return jsonError(400, "NO_SEGMENTS");
    for (const segment of segments) {
      if (typeof segment?.start !== "number" || typeof segment?.end !== "number" || typeof segment?.text !== "string" || segment.end <= segment.start) {
        return jsonError(400, "INVALID_SEGMENT");
      }
    }
    const now = new Date().toISOString();
    saveVideo({
      videoId,
      playlistId: "manual",
      title: typeof title === "string" ? title : null,
      topic: null,
      durationSeconds: Math.max(...segments.map((s) => s.end)),
      segments,
      indexedAt: now,
    });
    rebuildIndex();
    // Best-effort vector indexing: 0 when the local LLM is down (TF-IDF still works).
    const embedded = await embedVideoSegments(videoId, segments);
    return Response.json({ videoId, segments: segments.length, embedded });
  }

  if (pathname === "/api/rag/query") {
    let body: unknown;
    try { body = await request.json(); } catch { return jsonError(400, "INVALID_JSON"); }
    const { question, topK, videoId, playlistId, topic, synthesize } = body as {
      question?: string; topK?: number; videoId?: string; playlistId?: string; topic?: string; synthesize?: boolean;
    };
    if (typeof question !== "string" || !question.trim()) return jsonError(400, "QUESTION_REQUIRED");
    const limit = Number.isFinite(topK) ? Math.min(20, Math.max(1, Math.round(topK as number))) : 5;

    const queryTokens = tokenize(question);
    if (!queryTokens.length) return jsonError(400, "UNTOKENIZABLE_QUESTION");

    type Scored = { key: string; videoId: string; segment: Segment; score: number };
    const passesFilters = (doc: VideoDoc): boolean =>
      (!videoId || doc.videoId === videoId) && (!playlistId || doc.playlistId === playlistId) && (!topic || doc.topic === topic);

    // 1) Semantic retrieval: cosine over stored embeddings. Falls through to
    //    TF-IDF when the LLM is down, vectors are missing, or dims mismatch.
    let top: Scored[] = [];
    let retrieval = "tfidf";
    const store = loadEmbeddings();
    if (store && Object.keys(store.vectors).length) {
      const query = await embedTexts([question]);
      if (query && query.dim === store.dim && query.vectors[0]) {
        const qvec = query.vectors[0];
        for (const [key, vector] of Object.entries(store.vectors)) {
          const [docVideoId, startStr] = key.split(":");
          const doc = loadVideo(docVideoId);
          if (!doc || !passesFilters(doc)) continue;
          const start = Number(startStr);
          const segment = doc.segments.find((s) => Math.abs(s.start - start) < 0.05);
          if (!segment) continue;
          const score = cosine(qvec, vector);
          if (score <= 0) continue;
          top.push({ key, videoId: doc.videoId, segment, score: Math.round(Math.max(0, Math.min(1, score)) * 100) / 100 });
        }
        top.sort((a, b) => b.score - a.score || a.segment.start - b.segment.start);
        top = top.slice(0, limit);
        if (top.length) retrieval = "embedding";
      }
    }

    // 2) Keyword fallback: TF-IDF over the token index.
    if (!top.length) {
      const index = loadIndex();
      const scored: Scored[] = [];
      let df: Record<string, number> = {};
      for (const token of queryTokens) df[token] = 0;
      for (const entry of Object.values(index)) for (const token of queryTokens) if (entry[token]) df[token] = (df[token] ?? 0) + 1;

      for (const [key, entry] of Object.entries(index)) {
        const [docVideoId] = key.split(":");
        const doc = loadVideo(docVideoId);
        if (!doc || !passesFilters(doc)) continue;
        let score = 0;
        const totalTokens = Math.max(1, Object.values(entry).reduce((sum, n) => sum + n, 0));
        for (const token of queryTokens) {
          const tf = (entry[token] ?? 0) / totalTokens;
          if (!tf) continue;
          const idf = Math.log(1 + (Object.keys(index).length / (1 + (df[token] ?? 0))));
          score += tf * idf;
        }
        if (!score) continue;
        const start = Number(key.split(":")[1]);
        const segment = doc.segments.find((s) => Math.abs(s.start - start) < 0.05);
        if (!segment) continue;
        scored.push({ key, videoId: doc.videoId, segment, score: Math.round((score / (queryTokens.length * Math.log(1 + Object.keys(index).length))) * 100) / 100 });
      }
      scored.sort((a, b) => b.score - a.score || a.segment.start - b.segment.start);
      top = scored.slice(0, limit);
    }

    // 3) Answer: local LLM synthesis, deterministic string on any failure.
    let answer: string;
    let answerSource: "llm" | "deterministic";
    if (top.length && synthesize !== false) {
      const generated = await synthesizeAnswer(question, top);
      if (generated) { answer = generated; answerSource = "llm"; }
      else { answer = deterministicAnswer(top); answerSource = "deterministic"; }
    } else if (top.length) {
      answer = deterministicAnswer(top);
      answerSource = "deterministic";
    } else {
      answer = "No transcript excerpts matched the question — try different wording or ingest more videos.";
      answerSource = "deterministic";
    }

    return Response.json({
      answer,
      answer_source: answerSource,
      retrieval,
      results: top.map((t) => ({
        score: t.score,
        video_id: t.videoId,
        video_url: `https://www.youtube.com/watch?v=${t.videoId}`,
        text: t.segment.text,
        start_seconds: t.segment.start,
        end_seconds: t.segment.end,
        timestampUrl: `https://www.youtube.com/watch?v=${t.videoId}&t=${Math.floor(t.segment.start)}s`,
        playlist_id: undefined,
        topic: undefined,
      })),
    });
  }

  return jsonError(404, "UNKNOWN_ENDPOINT");
};

export const DELETE: RequestHandler = async ({ url }) => {
  const match = url.pathname.match(/^\/api\/rag\/playlists\/(pl-[\w-]+)$/);
  if (!match) return jsonError(404, "UNKNOWN_ENDPOINT");
  const doc = loadPlaylist(match[1]);
  if (!doc) return jsonError(404, "PLAYLIST_NOT_FOUND");
  const dir = dataDir() + "/playlists";
  rmSync(join(dir, `${doc.id}.json`), { force: true });
  return Response.json({ id: doc.id, removed: true });
};

function jsonError(status: number, message: string): Response {
  return Response.json({ message }, { status });
}
