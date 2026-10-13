-- Migration 1: Initial schema for analysis_jobs and analysis_chunks
-- Applied: 2026-10-13

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
CREATE INDEX IF NOT EXISTS analysis_chunks_status ON analysis_chunks(job_id, status, chunk_index);

CREATE TABLE IF NOT EXISTS setup_identity (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  model TEXT NOT NULL,
  language TEXT NOT NULL,
  pipeline_version TEXT NOT NULL,
  updated_at TEXT NOT NULL
);