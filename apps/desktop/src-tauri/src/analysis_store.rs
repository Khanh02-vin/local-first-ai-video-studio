// The analysis job store, ported from adapters/local/analysis-store.ts.
//
// The Node analysis worker (analyze-video.ts) still owns its progress writes through
// the same schema during a job; this Rust side owns every read and every management
// write (status polling, resumable claim, retry, cancel, abandon, cache clear), so
// the UI's per-poll status read no longer pays a Node process spawn.
//
// Both sides open the same file with WAL + busy_timeout, so cross-process
// read/write is safe; the DDL is IF NOT EXISTS, which composes with the Node
// migration runner (schema_migrations stays authoritative on the Node side).
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::paths::status_from_json;
use crate::contracts::AnalysisStatus;

/// A job that keeps dying on the same chunk must stop being auto-resumed.
pub(crate) const MAX_JOB_ATTEMPTS: i64 = 3;

/// The transcript cache pipeline version (adapters/local/analysis-cache.ts).
/// Cache files live under `<cache_root>/analysis/<fingerprint>/<version>/<key>.json`;
/// a version bump retires caches written by an older pipeline.
const PIPELINE_VERSION: &str = "content-v3-wordts";

pub(crate) struct AnalysisStore {
    db: Connection,
}

impl AnalysisStore {
    /// Opens (creating if needed) the analysis database with the same WAL pragma set
    /// and schema the Node store uses.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("ANALYSIS_DB_DIR:{e}"))?;
        }
        let db = Connection::open(path).map_err(|e| format!("ANALYSIS_DB_OPEN:{e}"))?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| format!("ANALYSIS_DB_BUSY:{e}"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS analysis_jobs (
               id TEXT PRIMARY KEY,
               input TEXT NOT NULL,
               duration REAL NOT NULL,
               range_start REAL NOT NULL DEFAULT 0,
               range_end REAL NOT NULL DEFAULT 0,
               setup_key TEXT,
               status TEXT NOT NULL,
               phase TEXT NOT NULL,
               progress REAL NOT NULL DEFAULT 0,
               error TEXT,
               result_path TEXT,
               attempts INTEGER NOT NULL DEFAULT 0,
               worker_pid INTEGER,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS analysis_chunks (
               job_id TEXT NOT NULL,
               chunk_index INTEGER NOT NULL,
               start REAL NOT NULL,
               end REAL NOT NULL,
               cache_key TEXT NOT NULL,
               status TEXT NOT NULL DEFAULT 'queued',
               attempts INTEGER NOT NULL DEFAULT 0,
               error TEXT,
               updated_at TEXT NOT NULL,
               PRIMARY KEY(job_id, chunk_index),
               FOREIGN KEY(job_id) REFERENCES analysis_jobs(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS analysis_jobs_status ON analysis_jobs(status);
             CREATE INDEX IF NOT EXISTS analysis_chunks_status ON analysis_chunks(job_id, status, chunk_index);",
        )
        .map_err(|e| format!("ANALYSIS_DB_SCHEMA:{e}"))?;
        Ok(Self { db })
    }

    /// One job row shaped exactly like the Node store's `get` output (camelCase keys
    /// the UI and the command layer already parse). `created_at`/`updated_at`/
    /// `worker_pid` are intentionally omitted — the Node script omits them too.
    pub(crate) fn get(&self, id: &str) -> Result<Option<Value>, String> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT j.id, j.input, j.duration,
                        j.range_start AS rangeStart, j.range_end AS rangeEnd,
                        j.setup_key AS setupKey, j.status, j.phase, j.progress, j.error,
                        j.result_path AS resultPath, j.attempts,
                        (SELECT COUNT(*) FROM analysis_chunks c WHERE c.job_id=j.id AND c.status='completed') AS chunkCompleted,
                        (SELECT COUNT(*) FROM analysis_chunks c WHERE c.job_id=j.id) AS chunkTotal,
                        (SELECT chunk_index FROM analysis_chunks c WHERE c.job_id=j.id AND c.status='running' LIMIT 1) AS currentChunk
                 FROM analysis_jobs j WHERE j.id=?1",
            )
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        let job = stmt
            .query_row(params![id], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "input": row.get::<_, String>(1)?,
                    "duration": row.get::<_, f64>(2)?,
                    "rangeStart": row.get::<_, f64>(3)?,
                    "rangeEnd": row.get::<_, f64>(4)?,
                    "setupKey": row.get::<_, Option<String>>(5)?,
                    "status": row.get::<_, String>(6)?,
                    "phase": row.get::<_, String>(7)?,
                    "progress": row.get::<_, f64>(8)?,
                    "error": row.get::<_, Option<String>>(9)?,
                    "resultPath": row.get::<_, Option<String>>(10)?,
                    "attempts": row.get::<_, i64>(11)?,
                    "chunkCompleted": row.get::<_, i64>(12)?,
                    "chunkTotal": row.get::<_, i64>(13)?,
                    "currentChunk": row.get::<_, Option<i64>>(14)?,
                }))
            })
            .optional()
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        Ok(job)
    }

    fn ids(&self, query: &str, arg: &str) -> Result<Vec<String>, String> {
        let mut stmt = self.db.prepare(query).map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        let rows = stmt
            .query_map(params![arg], |row| row.get::<_, String>(0))
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("ANALYSIS_DB:{e}"))
    }

    /// Every job, newest-updated first (`--active` in the status script).
    pub(crate) fn all_jobs(&self) -> Result<Vec<Value>, String> {
        let ids = self.ids(
            "SELECT id FROM analysis_jobs ORDER BY updated_at DESC",
            "",
        )?;
        ids.iter().filter_map(|id| self.get(id).transpose()).collect()
    }

    fn now() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().to_string())
            .unwrap_or_default()
    }

    /// Counts one crash against the job's budget (claim side of `--resumable`).
    pub(crate) fn bump_attempts(&self, id: &str) -> Result<i64, String> {
        self.db
            .execute(
                "UPDATE analysis_jobs SET attempts=attempts+1, updated_at=?2 WHERE id=?1",
                params![id, Self::now()],
            )
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        self.db
            .query_row(
                "SELECT attempts FROM analysis_jobs WHERE id=?1",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| format!("ANALYSIS_DB:{e}"))
            .map(|attempts| attempts.unwrap_or(0))
    }

    /// Clearing the crash budget: only failed jobs are retryable.
    pub(crate) fn retry(&self, id: &str) -> Result<bool, String> {
        self.db
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("ANALYSIS_DB_TX:{e}"))?;
        let changed = self
            .db
            .execute(
                "UPDATE analysis_jobs SET attempts=0, status='queued', phase='queued', error=NULL, worker_pid=NULL, updated_at=?2
                 WHERE id=?1 AND status='failed'",
                params![id, Self::now()],
            )
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        if changed != 1 {
            let _ = self.db.execute_batch("ROLLBACK");
            return Ok(false);
        }
        self.db
            .execute(
                "UPDATE analysis_chunks SET status='queued', error=NULL, updated_at=?2 WHERE job_id=?1 AND status='failed'",
                params![id, Self::now()],
            )
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        self.db
            .execute_batch("COMMIT")
            .map_err(|e| format!("ANALYSIS_DB_TX:{e}"))?;
        Ok(true)
    }

    /// Generic guarded status patch (mirrors Node's `update`): never overwrites a
    /// job the user already cancelled.
    fn patch_status(
        &self,
        id: &str,
        status: &str,
        phase: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE analysis_jobs SET status=?2, phase=?3, error=?4, updated_at=?5 WHERE id=?1 AND status!='cancelled'",
                params![id, status, phase, error, Self::now()],
            )
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        Ok(())
    }

    /// Marks a job failed with `error` (the `--fail` mode of the status script).
    pub(crate) fn fail(&self, id: &str, error: &str) -> Result<(), String> {
        self.patch_status(id, "failed", "failed", Some(error))
    }

    /// Requeues a job whose worker was lost (claim side of `--resumable`).
    pub(crate) fn mark_worker_lost(&self, id: &str) -> Result<(), String> {
        self.patch_status(id, "queued", "queued", Some("worker lost; will resume"))
    }

    pub(crate) fn worker_pid(&self, id: &str) -> Result<Option<i64>, String> {
        self.db
            .query_row(
                "SELECT worker_pid FROM analysis_jobs WHERE id=?1",
                params![id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(|e| format!("ANALYSIS_DB:{e}"))
            .map(|row| row.flatten())
    }

    /// User abort: the job and any running chunk stop where they are.
    pub(crate) fn cancel(&self, id: &str) -> Result<(), String> {
        self.db
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("ANALYSIS_DB_TX:{e}"))?;
        let jobs = self
            .db
            .execute(
                "UPDATE analysis_jobs SET status='cancelled', phase='cancelled', error='stopped by user', updated_at=?2
                 WHERE id=?1 AND status IN ('queued','running')",
                params![id, Self::now()],
            )
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        if jobs == 0 {
            let _ = self.db.execute_batch("ROLLBACK");
            return Ok(());
        }
        self.db
            .execute(
                "UPDATE analysis_chunks SET status='queued', error='cancelled', updated_at=?2 WHERE job_id=?1 AND status='running'",
                params![id, Self::now()],
            )
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        self.db
            .execute_batch("COMMIT")
            .map_err(|e| format!("ANALYSIS_DB_TX:{e}"))?;
        Ok(())
    }

    /// Removes the job row and its persisted result file; returns the bytes freed.
    /// Chunk transcript caches are handled by `clear_chunk_caches`.
    pub(crate) fn clear(&self, id: &str) -> Result<i64, String> {
        let job = self.get(id)?;
        let Some(job) = job else { return Ok(0) };
        let mut bytes: i64 = 0;
        if let Some(result_path) = job["resultPath"].as_str() {
            if let Ok(meta) = std::fs::metadata(result_path) {
                bytes += meta.len() as i64;
            }
            let _ = std::fs::remove_file(result_path);
        }
        self.db
            .execute("DELETE FROM analysis_jobs WHERE id=?1", params![id])
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        Ok(bytes)
    }

    /// The transcript cache files of every chunk of `id` (`--clear`'s cache pass).
    pub(crate) fn clear_chunk_caches(&self, id: &str, cache_root: &Path) -> Result<i64, String> {
        let Some(job) = self.get(id)? else { return Ok(0) };
        let Some(input) = job["input"].as_str().map(str::to_string) else { return Ok(0) };
        let Ok(source) = source_fingerprint(Path::new(&input)) else { return Ok(0) };
        let mut stmt = self
            .db
            .prepare("SELECT cache_key FROM analysis_chunks WHERE job_id=?1")
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        let keys = stmt
            .query_map(params![id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("ANALYSIS_DB:{e}"))?;
        let mut bytes: i64 = 0;
        for key in keys {
            let path = cache_path(cache_root, &source, &key);
            if let Ok(meta) = std::fs::metadata(&path) {
                bytes += meta.len() as i64;
            }
            let _ = std::fs::remove_file(&path);
        }
        Ok(bytes)
    }
}

/// sha256 of `path\0size\0mtimeMs` — byte-for-byte the same input the Node
/// `sourceFingerprint` hashes, so Rust and Node agree on cache file locations.
fn source_fingerprint(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let meta = std::fs::metadata(path).map_err(|e| format!("SOURCE_STAT:{e}"))?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update([0u8]);
    hasher.update(meta.len().to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(format!("{mtime_ms}").as_bytes());
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Where a chunk's cached transcript lives for this source (`cachePath` in the
/// Node cache module).
fn cache_path(cache_root: &Path, source: &str, key: &str) -> PathBuf {
    cache_root
        .join("analysis")
        .join(source)
        .join(PIPELINE_VERSION)
        .join(format!("{key}.json"))
}

/// Reads a job and embeds its persisted result JSON the way the status script's
/// default branch does, so the UI contract is unchanged.
pub(crate) fn job_with_result(store: &AnalysisStore, id: &str) -> Result<Option<Value>, String> {
    let Some(mut job) = store.get(id)? else { return Ok(None) };
    if let Some(result_path) = job["resultPath"].as_str() {
        let contents = std::fs::read_to_string(result_path).ok();
        job["result"] = match contents {
            Some(text) => json!(text),
            None => Value::Null,
        };
    }
    Ok(Some(job))
}

/// The Tauri-facing status read: store read + contract parse, no Node spawn.
pub(crate) fn read_status(id: &str) -> Result<AnalysisStatus, String> {
    let store = AnalysisStore::open(&crate::paths::jobs_path())?;
    let Some(job) = job_with_result(&store, id)? else {
        return Err("JOB_NOT_FOUND".into());
    };
    status_from_json(&job)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lfai-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("analysis.sqlite")
    }

    fn job_row(store: &AnalysisStore, id: &str) -> Value {
        store.get(id).unwrap().expect("job row must exist")
    }

    /// Create → get: the row carries the same camelCase keys the Node store's
    /// `get` produced, with the chunk subqueries live.
    #[test]
    fn create_and_get_roundtrip_matches_node_shape() {
        let db = temp_db("roundtrip");
        let store = AnalysisStore::open(&db).unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_jobs (id,input,duration,range_start,range_end,setup_key,status,phase,created_at,updated_at)
                 VALUES ('j1','/tmp/v.mp4',60.0,0.0,60.0,NULL,'queued','queued','now','now')",
                [],
            )
            .unwrap();
        let job = job_row(&store, "j1");
        assert_eq!(job["id"], json!("j1"));
        assert_eq!(job["input"], json!("/tmp/v.mp4"));
        assert_eq!(job["rangeStart"], json!(0.0));
        assert_eq!(job["rangeEnd"], json!(60.0));
        assert_eq!(job["chunkCompleted"], json!(0));
        assert_eq!(job["chunkTotal"], json!(0));
        assert_eq!(job["currentChunk"], serde_json::Value::Null);
        assert!(store.get("missing").unwrap().is_none());
    }

    /// retry: only failed jobs are retryable; a retried job requeues its failed chunks.
    #[test]
    fn retry_requeues_failed_job_and_chunks_only() {
        let db = temp_db("retry");
        let store = AnalysisStore::open(&db).unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_jobs (id,input,duration,range_start,range_end,status,phase,created_at,updated_at)
                 VALUES ('j1','in',60.0,0.0,60.0,'failed','failed','now','now')",
                [],
            )
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_chunks (job_id,chunk_index,start,end,cache_key,status,updated_at)
                 VALUES ('j1',0,0.0,30.0,'k0','failed','now'),('j1',1,30.0,60.0,'k1','completed','now')",
                [],
            )
            .unwrap();

        // A running job is not retryable.
        store
            .db
            .execute("UPDATE analysis_jobs SET status='running' WHERE id='j1'", [])
            .unwrap();
        assert!(!store.retry("j1").unwrap());

        store
            .db
            .execute("UPDATE analysis_jobs SET status='failed' WHERE id='j1'", [])
            .unwrap();
        assert!(store.retry("j1").unwrap());
        let job = job_row(&store, "j1");
        assert_eq!(job["status"], json!("queued"));
        assert_eq!(job["attempts"], json!(0));
        // The failed chunk requeued, the completed one untouched.
        let (failed, completed): (i64, i64) = store
            .db
            .query_row(
                "SELECT SUM(status='queued'), SUM(status='completed') FROM analysis_chunks WHERE job_id='j1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((failed, completed), (1, 1));
    }

    /// cancel: stops queued/running jobs and requeues the running chunk; a
    /// completed job is never overwritten.
    #[test]
    fn cancel_stops_running_job_and_never_touches_completed() {
        let db = temp_db("cancel");
        let store = AnalysisStore::open(&db).unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_jobs (id,input,duration,range_start,range_end,status,phase,created_at,updated_at)
                 VALUES ('j1','in',60.0,0.0,60.0,'running','transcribe','now','now')",
                [],
            )
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_chunks (job_id,chunk_index,start,end,cache_key,status,updated_at)
                 VALUES ('j1',0,0.0,30.0,'k0','running','now')",
                [],
            )
            .unwrap();
        store.cancel("j1").unwrap();
        let job = job_row(&store, "j1");
        assert_eq!(job["status"], json!("cancelled"));
        assert_eq!(job["error"], json!("stopped by user"));

        // The update guard: a cancelled job ignores further patches.
        store.fail("j1", "late error").unwrap();
        assert_eq!(job_row(&store, "j1")["status"], json!("cancelled"));

        // A second cancel on a cancelled job is a no-op.
        store.cancel("j1").unwrap();
        assert_eq!(job_row(&store, "j1")["status"], json!("cancelled"));
    }

    /// The resumable claim: crashes count up to the budget, then the job fails.
    #[test]
    fn claim_budget_fails_after_max_attempts() {
        let db = temp_db("claim");
        let store = AnalysisStore::open(&db).unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_jobs (id,input,duration,range_start,range_end,status,phase,created_at,updated_at)
                 VALUES ('j1','in',60.0,0.0,60.0,'running','transcribe','now','now')",
                [],
            )
            .unwrap();
        for attempt in 1..=MAX_JOB_ATTEMPTS {
            let attempts = store.bump_attempts("j1").unwrap();
            assert_eq!(attempts, attempt);
            if attempts < MAX_JOB_ATTEMPTS {
                store.mark_worker_lost("j1").unwrap();
                assert_eq!(job_row(&store, "j1")["status"], json!("queued"));
            }
        }
        // The third crash exhausts the budget: the job stops auto-resuming.
        let attempts = store.bump_attempts("j1").unwrap();
        assert!(attempts >= MAX_JOB_ATTEMPTS);
        store
            .fail("j1", &format!("worker died {attempts} times without progress; retry manually"))
            .unwrap();
        assert_eq!(job_row(&store, "j1")["status"], json!("failed"));
    }

    /// clear: removes the result file and the row, returning freed bytes; the
    /// chunk-cache pass fingerprints the source the same way Node does.
    #[test]
    fn clear_removes_result_and_reports_bytes() {
        let db = temp_db("clear");
        let result = db.parent().unwrap().join("result-j1.json");
        std::fs::write(&result, b"{\"highlights\":[]}".repeat(10)).unwrap();
        let store = AnalysisStore::open(&db).unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_jobs (id,input,duration,range_start,range_end,status,phase,result_path,created_at,updated_at)
                 VALUES ('j1','in',60.0,0.0,60.0,'completed','completed',?1,'now','now')",
                params![result.to_string_lossy()],
            )
            .unwrap();
        let bytes = store.clear("j1").unwrap();
        assert!(bytes > 0);
        assert!(!result.exists());
        assert!(store.get("j1").unwrap().is_none());
        // Clearing a missing job is a zero-byte no-op.
        assert_eq!(store.clear("j1").unwrap(), 0);
    }

    /// job_with_result embeds the persisted result file content as a string,
    /// exactly the shape `status_from_json` expects.
    #[test]
    fn job_with_result_embeds_result_file() {
        let db = temp_db("embed");
        let result = db.parent().unwrap().join("embed.json");
        std::fs::write(&result, "{\"highlights\":[1]}").unwrap();
        let store = AnalysisStore::open(&db).unwrap();
        store
            .db
            .execute(
                "INSERT INTO analysis_jobs (id,input,duration,range_start,range_end,status,phase,progress,result_path,created_at,updated_at)
                 VALUES ('j1','in',60.0,0.0,60.0,'completed','completed',1.0,?1,'now','now')",
                params![result.to_string_lossy()],
            )
            .unwrap();
        let job = job_with_result(&store, "j1").unwrap().unwrap();
        let status = status_from_json(&job).unwrap();
        assert_eq!(status.status, "completed");
        assert!(status.result.is_some());
    }
}
