import { createHash } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { dirname } from "node:path";
import { mkdirSync } from "node:fs";
import { AnalysisStore } from "./analysis-store.ts";

export type LocalJobStatus = "queued" | "running" | "completed" | "failed" | "cancelled" | "interrupted";
export type LocalJob = {
  id: string;
  input: string;
  output: string;
  start: number | null;
  end: number | null;
  aspectRatio: string;
  status: LocalJobStatus;
  progress: number;
  attempts: number;
  maxAttempts: number;
  error: string | null;
  outputBytes: number | null;
  idempotencyKey: string | null;
  analysisJobId: string | null;
};

type Raw = {
  id: string; input: string; output: string; start: number | null; end: number | null; aspect_ratio: string;
  status: string; progress: number; attempts: number; max_attempts: number; error: string | null;
  output_bytes: number | null; idempotency_key: string | null; analysis_job_id: string | null;
};

export type JobCreateInput = Omit<LocalJob, "status" | "progress" | "attempts" | "error" | "outputBytes" | "idempotencyKey" | "analysisJobId"> & {
  maxAttempts?: number;
  idempotencyKey?: string;
  analysisJobId?: string;
};

/** Stable content idempotency key: the same input+range+aspect is one logical render. */
export function jobContentKey(input: string, start: number | null, end: number | null, aspectRatio: string): string {
  return createHash("sha256").update([input, start ?? 0, end ?? 0, aspectRatio].join("\u0000")).digest("hex");
}

function migrate(db: DatabaseSync): void {
  const columns = db.prepare("PRAGMA table_info(jobs)").all() as Array<{ name: string }>;
  for (const [name, definition] of [["idempotency_key", "TEXT"], ["analysis_job_id", "TEXT"]] as const) {
    if (!columns.some((column) => column.name === name)) db.exec(`ALTER TABLE jobs ADD COLUMN ${name} ${definition}`);
  }
  db.exec("CREATE UNIQUE INDEX IF NOT EXISTS jobs_idempotency ON jobs(idempotency_key)");
}

export class LocalStore {
  readonly db: DatabaseSync;
  private readonly analysis?: AnalysisStore;
  private constructor(db: DatabaseSync, analysis?: AnalysisStore) { this.db = db; this.analysis = analysis; }
  static open(path: string, analysisDbPath?: string): LocalStore {
    if (path !== ":memory:") mkdirSync(dirname(path), { recursive: true });
    const analysis = analysisDbPath ? AnalysisStore.open(analysisDbPath) : undefined;
    const store = new LocalStore(new DatabaseSync(path), analysis);
    store.db.exec("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, input TEXT NOT NULL, output TEXT NOT NULL, start REAL, end REAL, aspect_ratio TEXT NOT NULL, status TEXT NOT NULL, progress REAL NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0, max_attempts INTEGER NOT NULL DEFAULT 2, error TEXT, output_bytes INTEGER); CREATE INDEX IF NOT EXISTS jobs_status ON jobs(status, id);");
    migrate(store.db);
    return store;
  }
  close(): void { this.db.close(); }
  /**
   * Idempotent create: if a job with the same idempotency key already exists it is returned,
   * otherwise a new job is inserted. The content key is derived from the render request so
   * repeated identical enqueues dedupe instead of stacking.
   */
  create(job: JobCreateInput): LocalJob {
    const key = job.idempotencyKey ?? jobContentKey(job.input, job.start, job.end, job.aspectRatio);
    const existing = this.db.prepare("SELECT * FROM jobs WHERE idempotency_key=?").get(key) as Raw | undefined;
    if (existing) {
      // "Re-render" after a terminal state: the user clicked again, so restart the same
      // record from scratch instead of handing back a dead one.
      if (existing.status === "cancelled" || existing.status === "failed") {
        this.db.prepare("UPDATE jobs SET status='queued', progress=0, attempts=0, error=null WHERE id=?").run(existing.id);
        return this.get(existing.id)!;
      }
      return map(existing);
    }
    this.db.prepare("INSERT INTO jobs (id,input,output,start,end,aspect_ratio,status,max_attempts,idempotency_key,analysis_job_id) VALUES (?,?,?,?,?,?, 'queued', ?,?,?)")
      .run(job.id, job.input, job.output, job.start, job.end, job.aspectRatio, job.maxAttempts ?? 2, key, job.analysisJobId ?? null);
    return this.get(job.id)!;
  }
  get(id: string): LocalJob | undefined { const row = this.db.prepare("SELECT * FROM jobs WHERE id=?").get(id) as Raw | undefined; return row && map(row); }
  list(): LocalJob[] { return (this.db.prepare("SELECT * FROM jobs ORDER BY rowid").all() as Raw[]).map(map); }
  /**
   * Claim the next runnable queued job. A render job that still depends on an analysis
   * job which has not completed is deferred (skipped) so the pipeline never renders ahead
   * of its own analysis.
   */
  claim(): LocalJob | undefined {
    this.db.exec("BEGIN IMMEDIATE");
    try {
      const rows = this.db.prepare("SELECT id FROM jobs WHERE status='queued' ORDER BY rowid").all() as Array<{ id: string }>;
      for (const row of rows) {
        const job = this.get(row.id);
        if (!job) continue;
        if (this.analysisBlocked(job)) continue;
        this.db.prepare("UPDATE jobs SET status='running', progress=0, attempts=attempts+1 WHERE id=? AND status='queued'").run(job.id);
        this.db.exec("COMMIT");
        return this.get(job.id);
      }
      this.db.exec("COMMIT");
      return undefined;
    } catch (error) { this.db.exec("ROLLBACK"); throw error; }
  }
  private analysisBlocked(job: LocalJob): boolean {
    if (!job.analysisJobId || !this.analysis) return false;
    const status = this.analysis.get(job.analysisJobId)?.status;
    return status !== undefined && status !== "completed";
  }
  update(id: string, patch: Partial<Pick<LocalJob, "status" | "progress" | "error" | "outputBytes">>): LocalJob | undefined { const entries = Object.entries(patch); if (!entries.length) return this.get(id); const columns = entries.map(([key]) => `${key === "outputBytes" ? "output_bytes" : key}=?`).join(","); this.db.prepare(`UPDATE jobs SET ${columns} WHERE id=?`).run(...entries.map(([, value]) => value), id); return this.get(id); }
  recover(): void { this.db.prepare("UPDATE jobs SET status=CASE WHEN attempts < max_attempts THEN 'queued' ELSE 'interrupted' END, error=CASE WHEN attempts < max_attempts THEN 'recovered after restart' ELSE 'attempt budget exhausted' END WHERE status='running'").run(); }
}
function map(row: Raw): LocalJob { return { id: row.id, input: row.input, output: row.output, start: row.start, end: row.end, aspectRatio: row.aspect_ratio, status: row.status as LocalJobStatus, progress: row.progress, attempts: row.attempts, maxAttempts: row.max_attempts, error: row.error, outputBytes: row.output_bytes, idempotencyKey: row.idempotency_key, analysisJobId: row.analysis_job_id }; }
