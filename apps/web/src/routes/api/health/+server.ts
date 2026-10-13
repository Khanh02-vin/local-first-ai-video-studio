/**
 * Health check endpoint for K8s readiness/liveness probes.
 * GET /api/health
 * Returns 200 with JSON status when all critical deps are healthy.
 */
import type { RequestHandler } from "@sveltejs/kit";
import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";

const DB_PATH = process.env.LOCAL_FIRST_STATE_DIR
  ? join(process.env.LOCAL_FIRST_STATE_DIR, "youtube", "analysis.sqlite")
  : join(process.env.HOME ?? ".", ".cache", "local-first-ai-video-studio", "youtube", "analysis.sqlite");

async function checkSQLite(): Promise<{ healthy: boolean; latencyMs: number }> {
  const start = Date.now();
  try {
    const db = new DatabaseSync(DB_PATH);
    db.prepare("SELECT 1").get();
    db.close();
    return { healthy: true, latencyMs: Date.now() - start };
  } catch {
    return { healthy: false, latencyMs: Date.now() - start };
  }
}

async function checkLLM(): Promise<{ healthy: boolean; latencyMs: number }> {
  const baseUrl = (process.env.LOCAL_LLM_BASE_URL ?? "http://127.0.0.1:8080").replace(/\/+$/, "");
  const start = Date.now();
  try {
    const res = await fetch(`${baseUrl}/v1/models`, { signal: AbortSignal.timeout(5_000) });
    return { healthy: res.ok, latencyMs: Date.now() - start };
  } catch {
    return { healthy: false, latencyMs: Date.now() - start };
  }
}

export const GET: RequestHandler = async () => {
  const [sqlite, llm] = await Promise.all([checkSQLite(), checkLLM()]);
  
  const healthy = sqlite.healthy && llm.healthy;
  const status = healthy ? 200 : 503;
  
  return new Response(JSON.stringify({
    status: healthy ? "healthy" : "degraded",
    timestamp: new Date().toISOString(),
    checks: {
      sqlite: { healthy: sqlite.healthy, latencyMs: sqlite.latencyMs },
      llm: { healthy: llm.healthy, latencyMs: llm.latencyMs },
    },
  }), {
    status,
    headers: { "Content-Type": "application/json" },
  });
};