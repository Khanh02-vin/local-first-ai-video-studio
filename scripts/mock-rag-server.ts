/**
 * Ponytail mock server — ultra lazy fallback (Node native, no deps)
 * Run: node scripts/mock-rag-server.ts
 */
import { createServer } from "node:http";
import { parse } from "node:url";

const PORT = 1420;
const state: { playlists: unknown[]; videos: unknown[] } = { playlists: [], videos: [] };

createServer((req, res) => {
  const { pathname } = parse(req.url ?? "/", true);

  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Headers", "Content-Type");
  res.setHeader("Content-Type", "application/json");

  if (req.method === "OPTIONS") return res.writeHead(204).end();

  if (pathname === "/api/rag/playlists") {
    if (req.method === "GET") return res.end(JSON.stringify({ playlists: state.playlists }));
    if (req.method === "POST") {
      let body = "";
      req.on("data", (c) => (body += c));
      req.on("end", () => {
        const playlist = { id: `pl-${Date.now()}`, ...JSON.parse(body), status: "indexed" };
        state.playlists.push(playlist);
        res.writeHead(201);
        res.end(JSON.stringify({ id: playlist.id, status: "indexed" }));
      });
      return;
    }
  }

  if (pathname === "/api/rag/query" && req.method === "POST") {
    return res.end(
      JSON.stringify({
        answer: "Based on transcript, video discusses key points.",
        retrieval: "embedding",
        results: [
          { score: 0.9, video_id: "test123", text: "Important point about AI models", start_seconds: 5, end_seconds: 10 },
        ],
      })
    );
  }

  res.writeHead(404);
  res.end(JSON.stringify({ message: "UNKNOWN_ENDPOINT" }));
}).listen(PORT, () => console.log(`Ponytail mock RAG server: http://localhost:${PORT}`));
