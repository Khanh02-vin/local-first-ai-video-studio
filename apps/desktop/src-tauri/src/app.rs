// The app-level commands: project CRUD, the licence tier, and the small probes the
// window asks for on start (app info, last panic, media probe).
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contracts::ProbeMetadata;
use crate::paths::{ffprobe_bin, license_tier, data_dir, tool, LICENSE_PUBLIC_KEY_B64};
use crate::project_store;

#[tauri::command]
pub(crate) fn license_status() -> serde_json::Value {
    if LICENSE_PUBLIC_KEY_B64.is_empty() {
        serde_json::json!({ "licensed": false, "tier": "free", "licensee": null, "configured": false })
    } else if license_tier() == "pro" {
        serde_json::json!({ "licensed": true, "tier": "pro", "licensee": "Creator Test", "expiresAt": null })
    } else {
        serde_json::json!({ "licensed": false, "tier": "free", "licensee": null })
    }
}

#[tauri::command]
pub(crate) fn app_info() -> &'static str {
    "local-first-ai-video-studio"
}

/// Returns the panic log written by the previous run (and clears it to
/// `panic.log.last`) so the UI can show the crash report once after a silent
/// Windows exit.
#[tauri::command]
pub(crate) fn last_panic() -> Option<String> {
    let log = data_dir().join("panic.log");
    let text = std::fs::read_to_string(&log).ok()?;
    let _ = std::fs::rename(&log, data_dir().join("panic.log.last"));
    Some(text)
}

#[tauri::command]
pub(crate) async fn probe_video(app: tauri::AppHandle, path: String) -> Result<ProbeMetadata, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let input = Path::new(&path).canonicalize().map_err(|e| format!("INPUT_PATH:{e}"))?;
        if !input.is_file() {
            return Err("INPUT_NOT_FILE".into());
        }
        let output = tool(ffprobe_bin(&app))
            .args(["-v", "error", "-show_entries", "format=duration,format_name:stream=codec_name,codec_type,width,height", "-of", "json"])
            .arg(&input)
            .output()
            .map_err(|e| format!("FFPROBE_SPAWN:{e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| format!("FFPROBE_JSON:{e}"))
    }).await.map_err(|error| format!("PROBE_WORKER:{error}"))?
}

#[tauri::command]
pub(crate) async fn project_list() -> Result<Vec<project_store::Project>, String> {
    tauri::async_runtime::spawn_blocking(|| project_store::list(&data_dir()))
        .await.map_err(|error| format!("PROJECT_LIST_WORKER:{error}"))?
}

#[tauri::command]
pub(crate) async fn project_get(id: String) -> Result<project_store::Project, String> {
    tauri::async_runtime::spawn_blocking(move || project_store::get(&data_dir(), &id))
        .await.map_err(|error| format!("PROJECT_GET_WORKER:{error}"))?
}

#[tauri::command]
pub(crate) async fn project_save(project: project_store::Project) -> Result<project_store::Project, String> {
    tauri::async_runtime::spawn_blocking(move || project_store::save(&data_dir(), project))
        .await.map_err(|error| format!("PROJECT_SAVE_WORKER:{error}"))?
}

#[tauri::command]
pub(crate) async fn project_delete(id: String) -> Result<project_store::Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let project = project_store::delete(&data_dir(), &id)?;
        // The analysis DB is Node-owned, but deletion is coordinated here so a
        // removed project cannot leave its job/result rows behind.
        if let Some(job_id) = project.analysis_job_id.as_deref() {
            let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/analysis-status.ts");
            let _ = script;
            let db_path = crate::paths::jobs_path();
            if let Ok(connection) = rusqlite::Connection::open(&db_path) {
                let _ = connection.execute("DELETE FROM analysis_chunks WHERE job_id=?1", [job_id]);
                let _ = connection.execute("DELETE FROM analysis_jobs WHERE id=?1", [job_id]);
            }
        }
        Ok(project)
    }).await.map_err(|error| format!("PROJECT_DELETE_WORKER:{error}"))?
}

#[tauri::command]
pub(crate) async fn project_create(
    source_path: String,
    source_name: String,
    source_duration: f64,
) -> Result<project_store::Project, String> {
    if !source_duration.is_finite() || source_duration < 0.0 {
        return Err("INVALID_SOURCE_DURATION".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let id = format!(
            "project-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| format!("CLOCK:{e}"))?.as_nanos()
        );
        project_store::save(&data_dir(), project_store::Project {
            id,
            title: source_name.clone(),
            source_path,
            source_name,
            source_duration,
            analysis_job_id: None,
            analysis_result_id: None,
            transcript: None,
            highlights: Vec::new(),
            selected_id: None,
            selected_ids: Vec::new(),
            plans: serde_json::json!({}),
            variants: Vec::new(),
            selected_variant_id: None,
            speaker_face_matches: None,
            renders: Vec::new(),
            updated_at: String::new(),
            revision: 0,
        })
    }).await.map_err(|error| format!("PROJECT_CREATE_WORKER:{error}"))?
}

