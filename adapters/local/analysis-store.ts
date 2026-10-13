import { DatabaseSync } from "node:sqlite";
import { dirname } from "node:path";
import { mkdirSync, unlinkSync, statSync, readdirSync } from "node:fs";
import { runMigrations } from "./migrations/runner.ts";

export type AnalysisStatus = "queued" | "running" | "completed" | "failed" | "interrupted";
export type ChunkStatus = "queued" | "running" | "completed" | "failed";
export type AnalysisChunk = { jobId: string; chunkIndex: number; start: number; end: number; cacheKey: string; status: ChunkStatus; attempts: number; error: string | null };
export type AnalysisJob = { id: string; input: string; duration: number; rangeStart: number; rangeEnd: number; setupKey: string | null; status: AnalysisStatus; phase: string; progress: number; error: string | null; resultPath: string | null; workerPid: number | null; chunkCompleted: number; chunkTotal: number; currentChunk: number | null; attempts: number };

/** A job that keeps dying on the same chunk must stop being auto-resumed. */
export const MAX_JOB_ATTEMPTS = 3;

export class AnalysisStore {
  readonly db: DatabaseSync;
  private constructor(db: DatabaseSync) { this.db = db; }
  static open(path: string, options: { recover?: boolean } = {}) { mkdirSync(dirname(path), { recursive: true }); const store = new AnalysisStore(new DatabaseSync(path)); store.db.exec("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;"); runMigrations(store.db); if (options.recover) store.recover(); return store; }
  create(id: string, input: string, duration: number, rangeStart = 0, rangeEnd = duration, setupKey: string | null = null) { const now = new Date().toISOString(); this.db.prepare("INSERT INTO analysis_jobs (id,input,duration,range_start,range_end,setup_key,status,phase,created_at,updated_at) VALUES (?,?,?,?,?,?,'queued','queued',?,?)").run(id,input,duration,rangeStart,rangeEnd,setupKey,now,now); return this.get(id)!; }
  get(id: string): AnalysisJob | undefined { const row = this.db.prepare("SELECT j.id,j.input,j.duration,j.range_start as rangeStart,j.range_end as rangeEnd,j.setup_key as setupKey,j.status,j.phase,j.progress,j.error,j.result_path as resultPath,j.attempts, (SELECT COUNT(*) FROM analysis_chunks c WHERE c.job_id=j.id AND c.status='completed') AS chunkCompleted, (SELECT COUNT(*) FROM analysis_chunks c WHERE c.job_id=j.id) AS chunkTotal, (SELECT chunk_index FROM analysis_chunks c WHERE c.job_id=j.id AND c.status='running' LIMIT 1) AS currentChunk FROM analysis_jobs j WHERE j.id=?").get(id) as AnalysisJob | undefined; return row; }
  bumpAttempts(id: string): number { this.db.prepare("UPDATE analysis_jobs SET attempts=attempts+1,updated_at=? WHERE id=?").run(new Date().toISOString(),id); const row = this.db.prepare("SELECT attempts FROM analysis_jobs WHERE id=?").get(id) as { attempts: number } | undefined; return row?.attempts ?? 0; }
  resetAttempts(id: string) { this.db.prepare("UPDATE analysis_jobs SET attempts=0,updated_at=? WHERE id=?").run(new Date().toISOString(),id); }
  retry(id: string) { const now = new Date().toISOString(); this.db.prepare("UPDATE analysis_jobs SET attempts=0,status='queued',phase='queued',error=NULL,worker_pid=NULL,updated_at=? WHERE id=?").run(now,id); this.db.prepare("UPDATE analysis_chunks SET status='queued',error=NULL,updated_at=? WHERE job_id=? AND status='failed'").run(now,id); }
  listActive(): AnalysisJob[] { const rows = this.db.prepare("SELECT id FROM analysis_jobs WHERE status IN ('queued','running') ORDER BY updated_at DESC").all() as Array<{ id: string }>; return rows.map((row) => this.get(row.id)!); }
  listQueued(): AnalysisJob[] { const rows = this.db.prepare("SELECT id FROM analysis_jobs WHERE status='queued' ORDER BY updated_at").all() as Array<{ id: string }>; return rows.map((row) => this.get(row.id)!); }
  chunksOf(jobId: string): AnalysisChunk[] { return this.db.prepare("SELECT job_id as jobId,chunk_index as chunkIndex,start,end,cache_key as cacheKey,status,attempts,error FROM analysis_chunks WHERE job_id=? ORDER BY chunk_index").all(jobId) as AnalysisChunk[]; }
  allJobs(): AnalysisJob[] { const rows = this.db.prepare("SELECT id FROM analysis_jobs ORDER BY updated_at DESC").all() as Array<{id:string}>; return rows.map(({id}) => this.get(id)!); }
  update(id: string, patch: Partial<Pick<AnalysisJob,"status"|"phase"|"progress"|"error"|"resultPath">>) { const entries = Object.entries(patch); if (!entries.length) return this.get(id); const cols = entries.map(([key]) => `${key === "resultPath" ? "result_path" : key}=?`).join(","); this.db.prepare(`UPDATE analysis_jobs SET ${cols},updated_at=? WHERE id=?`).run(...entries.map(([,v])=>v),new Date().toISOString(),id); return this.get(id); }
  createChunks(jobId: string, chunks: Array<Pick<AnalysisChunk,"chunkIndex"|"start"|"end"|"cacheKey">>) { const now = new Date().toISOString(); const stmt = this.db.prepare("INSERT OR IGNORE INTO analysis_chunks (job_id,chunk_index,start,end,cache_key,status,updated_at) VALUES (?,?,?,?,?,'queued',?)"); for (const chunk of chunks) stmt.run(jobId,chunk.chunkIndex,chunk.start,chunk.end,chunk.cacheKey,now); }
  updateChunk(jobId: string, chunkIndex: number, patch: Partial<Pick<AnalysisChunk,"status"|"attempts"|"error">>) { const entries = Object.entries(patch); if (!entries.length) return; const cols = entries.map(([key]) => `${key}=?`).join(","); this.db.prepare(`UPDATE analysis_chunks SET ${cols},updated_at=? WHERE job_id=? AND chunk_index=?`).run(...entries.map(([,v])=>v),new Date().toISOString(),jobId,chunkIndex); }
  recover() { const now = new Date().toISOString(); this.db.prepare("UPDATE analysis_chunks SET status='queued',error='recovered after restart',updated_at=? WHERE status='running'").run(new Date().toISOString()); this.db.prepare("UPDATE analysis_jobs SET status='queued',phase='queued',error='recovered after restart',worker_pid=NULL,updated_at=? WHERE status='running'").run(now); }
  workerPid(id: string): number | null { const row = this.db.prepare("SELECT worker_pid as pid FROM analysis_jobs WHERE id=?").get(id) as { pid: number | null } | undefined; return row?.pid ?? null; }
  setWorkerPid(id: string, pid: number | null) { this.db.prepare("UPDATE analysis_jobs SET worker_pid=?,updated_at=? WHERE id=?").run(pid,new Date().toISOString(),id); }
  clear(id: string) { const job = this.get(id); if (!job) return 0; let bytes = 0; if (job.resultPath) { try { bytes = statSync(job.resultPath).size; unlinkSync(job.resultPath); } catch {} } this.db.prepare("DELETE FROM analysis_jobs WHERE id=?").run(id); return bytes; }
  close() { this.db.close(); }
}
