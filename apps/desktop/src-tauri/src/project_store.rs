use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub title: String,
    pub source_path: String,
    pub source_name: String,
    pub source_duration: f64,
    pub analysis_job_id: Option<String>,
    #[serde(default)]
    pub analysis_result_id: Option<String>,
    pub transcript: Option<Value>,
    pub highlights: Vec<Value>,
    pub selected_id: Option<String>,
    pub plans: Value,
    pub renders: Vec<Value>,
    pub updated_at: String,
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("INVALID_PROJECT_ID".into());
    }
    Ok(())
}

fn json_project(raw: &[u8]) -> Result<Project, String> {
    serde_json::from_slice(raw).map_err(|e| format!("PROJECT_JSON:{e}"))
}

fn validate_project(project: &Project) -> Result<(), String> {
    validate_id(&project.id)?;
    if !project.source_duration.is_finite() || project.source_duration < 0.0 {
        return Err("INVALID_SOURCE_DURATION".into());
    }
    // Keep the project's established plans shape (an object keyed by highlight id).
    let plans = project.plans.as_object().ok_or("INVALID_PROJECT_PLANS")?;
    for plan in plans.values() {
        if let Some(version) = plan.get("version") {
            if !version.as_u64().is_some_and(|version| version > 0) {
                return Err("INVALID_RENDER_PLAN_VERSION".into());
            }
        }
        if let Some(source) = plan.get("sourceArtifactId") {
            if !source.as_str().is_some_and(|source| !source.is_empty()) {
                return Err("INVALID_RENDER_PLAN_SOURCE".into());
            }
        }
    }
    Ok(())
}

fn open(dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(dir).map_err(|e| format!("PROJECT_DIR:{e}"))?;
    let db =
        Connection::open(dir.join("projects.sqlite")).map_err(|e| format!("PROJECT_DB:{e}"))?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("PROJECT_DB:{e}"))?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, record_json TEXT NOT NULL, updated_at TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS projects_updated_at ON projects(updated_at DESC);")
        .map_err(|e| format!("PROJECT_DB:{e}"))?;
    import_json_projects(&db, dir)?;
    Ok(db)
}

/// Import legacy files once per project ID. Never removes or overwrites the JSON source.
fn import_json_projects(db: &Connection, dir: &Path) -> Result<(), String> {
    let folder = dir.join("projects");
    if !folder.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(folder).map_err(|e| format!("PROJECT_LIST:{e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("PROJECT_LIST:{e}"))?;
        if entry.path().extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let raw = fs::read(entry.path()).map_err(|e| format!("PROJECT_READ:{e}"))?;
        let Ok(project) = json_project(&raw) else {
            continue;
        };
        if validate_project(&project).is_err() {
            continue;
        }
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [&project.id],
                |row| row.get(0),
            )
            .map_err(|e| format!("PROJECT_DB:{e}"))?;
        if !exists {
            let record =
                serde_json::to_string(&project).map_err(|e| format!("PROJECT_JSON:{e}"))?;
            db.execute(
                "INSERT INTO projects(id,record_json,updated_at) VALUES(?1,?2,?3)",
                params![project.id, record, project.updated_at],
            )
            .map_err(|e| format!("PROJECT_DB:{e}"))?;
        }
    }
    Ok(())
}

fn read_project(db: &Connection, id: &str) -> Result<Option<Project>, String> {
    let raw: Option<String> = db
        .query_row(
            "SELECT record_json FROM projects WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("PROJECT_DB:{e}"))?;
    raw.map(|raw| json_project(raw.as_bytes())).transpose()
}

pub fn save(dir: &Path, mut project: Project) -> Result<Project, String> {
    validate_project(&project)?;
    let db = open(dir)?;
    project.updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("CLOCK:{e}"))?
        .as_secs()
        .to_string();
    let record = serde_json::to_string(&project).map_err(|e| format!("PROJECT_JSON:{e}"))?;
    db.execute("INSERT INTO projects(id,record_json,updated_at) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET record_json=excluded.record_json,updated_at=excluded.updated_at", params![project.id, record, project.updated_at]).map_err(|e| format!("PROJECT_DB:{e}"))?;
    Ok(project)
}

pub fn get(dir: &Path, id: &str) -> Result<Project, String> {
    validate_id(id)?;
    read_project(&open(dir)?, id)?.ok_or_else(|| "PROJECT_NOT_FOUND".into())
}

pub fn list(dir: &Path) -> Result<Vec<Project>, String> {
    let db = open(dir)?;
    let mut stmt = db
        .prepare("SELECT record_json FROM projects ORDER BY updated_at DESC,id")
        .map_err(|e| format!("PROJECT_DB:{e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("PROJECT_DB:{e}"))?;
    rows.map(|row| {
        let raw = row.map_err(|e| format!("PROJECT_DB:{e}"))?;
        json_project(raw.as_bytes())
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "lf-project-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn sample(id: &str) -> Project {
        Project {
            id: id.into(),
            title: "Video".into(),
            source_path: "/tmp/video.mp4".into(),
            source_name: "video.mp4".into(),
            source_duration: 30.0,
            analysis_job_id: Some("analysis-1".into()),
            analysis_result_id: None,
            transcript: None,
            highlights: vec![],
            selected_id: None,
            plans: serde_json::json!({}),
            renders: vec![],
            updated_at: String::new(),
        }
    }
    #[test]
    fn creates_lists_updates_and_reopens_project() {
        let dir = temp();
        let saved = save(&dir, sample("project-1")).unwrap();
        assert_eq!(list(&dir).unwrap(), vec![saved.clone()]);
        assert_eq!(get(&dir, "project-1").unwrap(), saved);
        let mut updated = sample("project-1");
        updated.title = "Edited".into();
        let saved = save(&dir, updated).unwrap();
        assert_eq!(get(&dir, "project-1").unwrap(), saved);
        drop(open(&dir).unwrap());
        assert_eq!(get(&dir, "project-1").unwrap().title, "Edited");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn imports_json_projects_idempotently_without_removing_source() {
        let dir = temp();
        let folder = dir.join("projects");
        fs::create_dir_all(&folder).unwrap();
        let project = sample("legacy-1");
        let source = folder.join("legacy-1.json");
        let bytes = serde_json::to_vec(&project).unwrap();
        fs::write(&source, &bytes).unwrap();
        assert_eq!(list(&dir).unwrap(), vec![project.clone()]);
        assert_eq!(list(&dir).unwrap(), vec![project]);
        assert_eq!(fs::read(source).unwrap(), bytes);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn preserves_plan_versions_and_analysis_reference_and_rejects_invalid_version() {
        let dir = temp();
        let mut project = sample("project-2");
        project.plans = serde_json::json!({"h1":{"version":2,"sourceArtifactId":"artifact-1"}});
        let saved = save(&dir, project).unwrap();
        assert_eq!(get(&dir, "project-2").unwrap(), saved);
        let mut bad = sample("project-3");
        bad.plans = serde_json::json!({"h1":{"version":0}});
        assert_eq!(save(&dir, bad).unwrap_err(), "INVALID_RENDER_PLAN_VERSION");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_path_traversal_ids() {
        assert!(validate_id("../bad").is_err());
    }
}
