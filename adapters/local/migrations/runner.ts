/**
 * Migration runner for SQLite schema versioning.
 * Replaces dynamic ALTER TABLE in AnalysisStore.migrate() with explicit versioned migrations.
 */
import { DatabaseSync } from "node:sqlite";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

export interface Migration {
  version: number;
  name: string;
  up: string;
  down?: string;
}

const MIGRATIONS_DIR = import.meta.dirname;

export function loadMigrations(): Migration[] {
  const files = readdirSync(MIGRATIONS_DIR)
    .filter(f => f.endsWith(".sql"))
    .sort();
  
  return files.map(file => {
    const match = file.match(/^(\d+)_(.+)\.sql$/);
    if (!match) throw new Error(`Invalid migration filename: ${file}`);
    const version = parseInt(match[1], 10);
    const name = match[2].replace(/_/g, " ");
    const sql = readFileSync(join(MIGRATIONS_DIR, file), "utf-8");
    return { version, name, up: sql };
  });
}

export function getAppliedVersion(db: DatabaseSync): number {
  const row = db.prepare(`
    SELECT COALESCE(MAX(version), 0) as version FROM schema_migrations
  `).get() as { version: number } | undefined;
  return row?.version ?? 0;
}

export function applyMigrations(db: DatabaseSync, migrations: Migration[]): number {
  const appliedVersion = getAppliedVersion(db);
  let applied = 0;
  
  for (const migration of migrations) {
    if (migration.version <= appliedVersion) continue;
    
    db.exec("BEGIN");
    try {
      db.exec(migration.up);
      db.prepare(`
        INSERT INTO schema_migrations (version, name, applied_at)
        VALUES (?, ?, ?)
      `).run(migration.version, migration.name, new Date().toISOString());
      db.exec("COMMIT");
      applied++;
    } catch (err) {
      db.exec("ROLLBACK");
      throw new Error(`Migration ${migration.version} failed: ${err}`);
    }
  }
  
  return applied;
}

export function ensureSchemaMigrationsTable(db: DatabaseSync): void {
  db.exec(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      version INTEGER PRIMARY KEY,
      name TEXT NOT NULL,
      applied_at TEXT NOT NULL
    )
  `);
}

export function runMigrations(db: DatabaseSync): number {
  ensureSchemaMigrationsTable(db);
  const migrations = loadMigrations();
  return applyMigrations(db, migrations);
}