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
  writeFileSync(join(dir, "..", "index.json"), JSON.stringify(index));
}

// --- endpoints ---

export const GET: RequestHandler = async ({ url }) => {
  if (url.pathname.endsWith("/playlists")) {
    const dir = dataDir() + "/playlists";
    const playlists = readdirSync(dir)
      .filter((n) => n.endsWith(".json"))
      .map((n) => loadPlaylist(n.replace(/\.json$/, "")))
      .filter((p): p is PlaylistDoc => p !== null);
    playlists.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    return Response.json({ playlists });
  }
  if (url.pathname.endsWith("/index")) return Response.json(loadIndex());
  return jsonError(404, "UNKNOWN_ENDPOINT");
};

export const POST: RequestHandler = async ({ request }) => {
  const { pathname } = new URL(request.url);

  if (pathname.endsWith("/playlists")) {
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

  if (pathname.endsWith("/ingest")) {
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
    return Response.json({ videoId, segments: segments.length });
  }

  if (pathname.endsWith("/query")) {
    let body: unknown;
    try { body = await request.json(); } catch { return jsonError(400, "INVALID_JSON"); }
    const { question, topK, videoId, playlistId, topic } = body as {
      question?: string; topK?: number; videoId?: string; playlistId?: string; topic?: string;
    };
    if (typeof question !== "string" || !question.trim()) return jsonError(400, "QUESTION_REQUIRED");
    const limit = Number.isFinite(topK) ? Math.min(20, Math.max(1, Math.round(topK as number))) : 5;

    const queryTokens = tokenize(question);
    if (!queryTokens.length) return jsonError(400, "UNTOKENIZABLE_QUESTION");

    const index = loadIndex();
    const scored: { key: string; videoId: string; segment: Segment; score: number }[] = [];
    let df: Record<string, number> = {};
    for (const token of queryTokens) df[token] = 0;
    for (const entry of Object.values(index)) for (const token of queryTokens) if (entry[token]) df[token] = (df[token] ?? 0) + 1;

    for (const [key, entry] of Object.entries(index)) {
      const [docVideoId] = key.split(":");
      const doc = loadVideo(docVideoId);
      if (!doc) continue;
      if (videoId && doc.videoId !== videoId) continue;
      if (playlistId && doc.playlistId !== playlistId) continue;
      if (topic && doc.topic !== topic) continue;
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
    const top = scored.slice(0, limit);

    // Synthesis: deterministic summary from the top results — no external LLM.
    const answer = top.length
      ? `Found ${top.length} matching excerpt${top.length === 1 ? "" : "s"} across ${new Set(top.map((t) => t.videoId)).size} video(s): ${top.map((t) => `"${t.segment.text.slice(0, 60)}${t.segment.text.length > 60 ? "…" : ""}" (${t.videoId} @ ${Math.round(t.segment.start)}s, ${(t.score * 100).toFixed(0)}% match)`).join("; ")}`
      : "No transcript excerpts matched the question — try different wording or ingest more videos.";

    return Response.json({
      answer,
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
  const match = url.pathname.match(/playlists\/(pl-[\w-]+)/);
  if (!match) return jsonError(404, "UNKNOWN_ENDPOINT");
  const doc = loadPlaylist(match[1]);
  if (!doc) return jsonError(404, "PLAYLIST_NOT_FOUND");
  const dir = dataDir() + "/playlists";
  writeFileSync(join(dir, `${doc.id}.json`), JSON.stringify({ ...doc, status: "failed" }));
  return Response.json({ id: doc.id, removed: false });
};

function jsonError(status: number, message: string): Response {
  return Response.json({ message }, { status });
}
