// Rest-parameter route: claims the entire /api/* subtree so the sub-paths the
// YouTube Studio UI fetches (/api/rag/playlists, /api/rag/ingest,
// /api/rag/query, /api/rag/index, DELETE /api/rag/playlists/<id>) dispatch
// to the RAG handler module instead of falling through to the SPA catch-all.
// SvelteKit registers one route entry per filesystem path — a +server.ts at
// the literal /api/rag path would only ever receive the exact URL /api/rag.
// The handler logic lives in ../rag/+server.ts — the single source of truth.
export * from "../rag/+server.ts";
