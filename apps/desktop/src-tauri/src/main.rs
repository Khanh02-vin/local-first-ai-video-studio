#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisStatus { status: String, phase: String, progress: f64, result: Option<String>, error: Option<String>, chunk_completed: u64, chunk_total: u64, current_chunk: Option<u64> }
type Jobs = Arc<Mutex<HashMap<String, AnalysisStatus>>>;
fn state_dir() -> PathBuf { std::env::var("LOCAL_FIRST_STATE_DIR").map(PathBuf::from).unwrap_or_else(|_| std::env::var("HOME").map(|h| Path::new(&h).join(".cache/local-first-ai-video-studio")).unwrap_or_else(|_| std::env::temp_dir())) }
fn jobs_path() -> PathBuf { state_dir().join("analysis.sqlite") }
/// Prefer a bundled binary in the app resources, then fall back to PATH.
fn bundled_bin(app: &tauri::AppHandle, rel: &str, fallback: &str) -> PathBuf {
    match app.path().resource_dir() { Ok(dir) => { let candidate = dir.join(rel); if candidate.is_file() { candidate } else { PathBuf::from(fallback) } }, Err(_) => PathBuf::from(fallback) }
}
/// Reads KEY=VALUE lines from the runtime env file (written by scripts/setup-machine.sh).
fn runtime_env() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(text) = std::fs::read_to_string(state_dir().join("runtime.env")) {
        for line in text.lines() { let line = line.trim(); if line.is_empty() || line.starts_with('#') { continue; } if let Some((key, value)) = line.split_once('=') { map.insert(key.trim().to_string(), value.trim().to_string()); } }
    }
    map
}
/// Environment for spawned helpers: bundled binaries + runtime.env defaults, real env wins.
fn helper_env(app: &tauri::AppHandle, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let file = runtime_env();
    let mut env: Vec<(String, String)> = file.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    env.push(("LOCAL_FIRST_STATE_DIR".into(), state_dir().to_string_lossy().into_owned()));
    env.push(("LICENSE_TIER".into(), license_tier()));
    env.push(("FFMPEG_COMMAND".into(), ffmpeg_bin(app).to_string_lossy().into_owned()));
    for (key, value) in extra { env.push((key.to_string(), value.to_string())); }
    env
}

/// Persists the chosen Whisper model into runtime.env so init/worker agree on cache keys.
#[tauri::command]
fn set_runtime_model(_app: tauri::AppHandle, model: String) -> Result<String, String> {
    if !["tiny", "base", "small"].contains(&model.as_str()) { return Err("INVALID_MODEL: choose tiny, base or small".into()); }
    if license_tier() != "pro" && model != "tiny" { return Err("PRO_REQUIRED: larger Whisper models need a Pro license".into()); }
    let file = runtime_env();
    let command = file.get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty()).unwrap_or_else(whisper_command);
    let device = std::env::var("WHISPER_DEVICE").ok().or_else(|| file.get("WHISPER_DEVICE").cloned()).unwrap_or_else(|| "cpu".into());
    let fp16 = if device == "cuda" { "True" } else { "False" };
    let highlight_strategy = file.get("HIGHLIGHT_STRATEGY").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "heuristic".into());
    let gemini_api_key = file.get("GEMINI_API_KEY").cloned().filter(|c| !c.is_empty()).unwrap_or_default();
    let text = format!("WHISPER_COMMAND={command}\nWHISPER_MODEL={model}\nWHISPER_DEVICE={device}\nWHISPER_FP16={fp16}\nHIGHLIGHT_STRATEGY={highlight_strategy}\nGEMINI_API_KEY={gemini_api_key}\n");
    std::fs::create_dir_all(state_dir()).map_err(|e| format!("STATE_DIR:{e}"))?;
    std::fs::write(state_dir().join("runtime.env"), text).map_err(|e| format!("RUNTIME_ENV:{e}"))?;
    Ok(model)
}

/// Persists the highlight strategy ("heuristic" or "semantic") and an optional Gemini API key
/// into runtime.env so the analysis worker picks the right strategy.
#[tauri::command]
fn set_highlight_strategy(_app: tauri::AppHandle, strategy: String, gemini_api_key: String) -> Result<String, String> {
    if !["heuristic", "semantic"].contains(&strategy.as_str()) { return Err("INVALID_STRATEGY: choose heuristic or semantic".into()); }
    let file = runtime_env();
    let command = file.get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty()).unwrap_or_else(whisper_command);
    let model = file.get("WHISPER_MODEL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "tiny".into());
    let device = file.get("WHISPER_DEVICE").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "cpu".into());
    let fp16 = file.get("WHISPER_FP16").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "False".into());
    // Keep the previously stored key when switching back to heuristic so the user's key is not lost.
    let key = if strategy == "semantic" { gemini_api_key.clone() } else { file.get("GEMINI_API_KEY").cloned().unwrap_or_default() };
    if strategy == "semantic" && key.trim().is_empty() { return Err("SEMANTIC_REQUIRES_GEMINI_API_KEY".into()); }
    let text = format!("WHISPER_COMMAND={command}\nWHISPER_MODEL={model}\nWHISPER_DEVICE={device}\nWHISPER_FP16={fp16}\nHIGHLIGHT_STRATEGY={strategy}\nGEMINI_API_KEY={key}\n");
    std::fs::create_dir_all(state_dir()).map_err(|e| format!("STATE_DIR:{e}"))?;
    std::fs::write(state_dir().join("runtime.env"), text).map_err(|e| format!("RUNTIME_ENV:{e}"))?;
    Ok(strategy)
}
/// Reports the currently stored highlight strategy and whether a Gemini key is present
/// (the key itself is never returned to the UI — only a hasKey flag).
#[tauri::command]
fn highlight_strategy_status(_app: tauri::AppHandle) -> serde_json::Value {
    let file = runtime_env();
    let strategy = file.get("HIGHLIGHT_STRATEGY").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "heuristic".into());
    let has_key = file.get("GEMINI_API_KEY").cloned().map(|k| !k.trim().is_empty()).unwrap_or(false);
    serde_json::json!({ "strategy": strategy, "hasKey": has_key })
}
fn node_bin(app: &tauri::AppHandle) -> PathBuf { bundled_bin(app, "node/bin/node", "node") }
fn ffmpeg_bin(app: &tauri::AppHandle) -> PathBuf { bundled_bin(app, "ffmpeg", "ffmpeg") }
fn ffprobe_bin(app: &tauri::AppHandle) -> PathBuf { bundled_bin(app, "ffprobe", "ffprobe") }
/// Resolve the whisper CLI the way the local whisper adapter will find it:
/// env/runtime.env first, then a venv next to the app, then plain PATH.
fn whisper_command() -> String {
    if let Ok(cmd) = std::env::var("WHISPER_COMMAND") { if !cmd.is_empty() { return cmd; } }
    if let Some(cmd) = runtime_env().get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty()) { return cmd; }
    let project_venv = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.venv/bin/whisper");
    if project_venv.is_file() { return project_venv.to_string_lossy().into_owned(); }
    if let Ok(home) = std::env::var("HOME") { if Path::new(&home).join(".local/bin/whisper").is_file() { return format!("{home}/.local/bin/whisper"); } }
    "whisper".into()
}
fn status_script(app: &tauri::AppHandle) -> Result<PathBuf, String> { app.path().resource_dir().map_err(|e| format!("RESOURCE_DIR:{e}")).map(|dir| dir.join("scripts/analysis-status.ts")) }
const NODE_ARGS: &[&str] = &["--experimental-strip-types"];
fn run_status_script(app: &tauri::AppHandle, args: &[String]) -> Result<serde_json::Value, String> { let script = status_script(app)?; let mut cmd = Command::new(node_bin(app)); cmd.args(NODE_ARGS).arg(script).arg(jobs_path()).args(args); for (key, value) in helper_env(app, &[]) { cmd.env(key, value); } let output = cmd.output().map_err(|e| format!("STATUS_SPAWN:{e}"))?; if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); } serde_json::from_slice(&output.stdout).map_err(|e| format!("STATUS_JSON:{e}")) }
fn status_from_json(job: &serde_json::Value) -> AnalysisStatus { AnalysisStatus { status: job["status"].as_str().unwrap_or("failed").into(), phase: job["phase"].as_str().unwrap_or("unknown").into(), progress: job["progress"].as_f64().unwrap_or(0.0), result: job["result"].as_str().map(String::from), error: job["error"].as_str().map(String::from), chunk_completed: job["chunkCompleted"].as_u64().unwrap_or(0), chunk_total: job["chunkTotal"].as_u64().unwrap_or(0), current_chunk: job["currentChunk"].as_u64() } }
fn read_sqlite_status(app: &tauri::AppHandle, id: &str) -> Result<AnalysisStatus, String> { let value = run_status_script(app, &[id.to_string()])?; if value.is_null() { return Err("JOB_NOT_FOUND".into()); } Ok(status_from_json(&value)) }

fn probe_duration(app: &tauri::AppHandle, input: &Path) -> Result<f64, String> {
    let output = Command::new(ffprobe_bin(app)).args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"]).arg(input).output().map_err(|e| format!("FFPROBE_SPAWN:{e}"))?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }
    let duration = String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().map_err(|_| "INVALID_DURATION".to_string())?;
    if !duration.is_finite() || duration <= 0.0 { return Err("INVALID_DURATION".into()); }
    Ok(duration)
}

fn command_ready(command: &str) -> bool {
    if command.contains('/') { return Path::new(command).is_file(); }
    Command::new("which").arg(command).output().map(|o| o.status.success()).unwrap_or(false)
}
fn storage(path: &Path, required: u64) -> serde_json::Value { let output = Command::new("df").args(["-Pk"]).arg(path).output(); let parsed = output.ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|text| text.lines().nth(1).and_then(|line| line.split_whitespace().nth(3).and_then(|kb| kb.parse::<u64>().ok()))); let free = parsed.map(|kb| kb * 1024); serde_json::json!({"path": path, "freeBytes": free, "requiredBytes": required, "ready": free.map(|n| n >= required).unwrap_or(false), "quotaKnown": false}) }
fn io_error(code: &str, message: impl std::fmt::Display) -> String { let text = message.to_string(); if text.contains("ENOSPC") || text.contains("No space left") { format!("STORAGE_FULL:{text}") } else if text.contains("EDQUOT") || text.contains("Disk quota") { format!("QUOTA_EXCEEDED:{text}") } else { format!("{code}:{text}") } }

#[tauri::command]
fn storage_status() -> serde_json::Value { let home = std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir()); serde_json::json!({"home": storage(&home, 2 * 1024 * 1024 * 1024u64), "temp": storage(&std::env::temp_dir(), 512 * 1024 * 1024u64), "modelCache": home.join(".cache/whisper")}) }

#[tauri::command]
fn clear_analysis_cache(app: tauri::AppHandle, id: String) -> Result<u64, String> {
    let status = run_status_script(&app, &[id.clone()])?;
    if let Some(pid) = status["workerPid"].as_i64() {
        if pid > 0 {
            let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
        }
    }
    let result = run_status_script(&app, &[id, "--clear".into()])?;
    Ok(result["bytes"].as_u64().unwrap_or(0))
}
#[tauri::command]
fn preflight_analyze(path: String) -> Result<serde_json::Value, String> { let input = Path::new(&path).canonicalize().map_err(|e| format!("INPUT_PATH:{e}"))?; if !input.is_file() { return Err("INPUT_NOT_FILE".into()); } let result = storage(input.parent().unwrap_or(Path::new("/")), 2 * 1024 * 1024 * 1024u64); if !result["ready"].as_bool().unwrap_or(false) { return Err("STORAGE_FULL:analysis requires at least 2 GiB free".into()); } Ok(result) }

#[tauri::command]
fn runtime_status(app: tauri::AppHandle) -> serde_json::Value {
    let file = runtime_env();
    let whisper = std::env::var("WHISPER_COMMAND").ok().filter(|c| !c.is_empty()).or_else(|| file.get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty())).unwrap_or_else(whisper_command);
    let model = std::env::var("WHISPER_MODEL").ok().or_else(|| file.get("WHISPER_MODEL").cloned()).unwrap_or("tiny".to_string());
    let model_ready = std::env::var("WHISPER_MODEL_PATH").map(|p| Path::new(&p).exists()).unwrap_or_else(|_| std::env::var("HOME").ok().map(|p| Path::new(&p).join(".cache/whisper").join(format!("{model}.pt")).exists()).unwrap_or(false));
    let device = std::env::var("WHISPER_DEVICE").ok().or_else(|| file.get("WHISPER_DEVICE").cloned()).unwrap_or("cpu".to_string());
    let ffmpeg = ffmpeg_bin(&app); let ffprobe = ffprobe_bin(&app); let node = node_bin(&app);
    serde_json::json!({ "ffmpeg": ffmpeg.as_path().is_file() || command_ready(ffmpeg.to_str().unwrap_or("ffmpeg")), "ffprobe": ffprobe.as_path().is_file() || command_ready(ffprobe.to_str().unwrap_or("ffprobe")), "node": node.as_path().is_file() || command_ready(node.to_str().unwrap_or("node")), "whisper": Path::new(&whisper).exists() || command_ready(&whisper), "model": model, "modelReady": model_ready, "device": device, "bundled": { "node": node.as_path().is_file(), "ffmpeg": ffmpeg.as_path().is_file(), "ffprobe": ffprobe.as_path().is_file() } })
}

#[tauri::command]
fn start_analysis(app: tauri::AppHandle, jobs: tauri::State<'_, Jobs>, path: String, start: Option<f64>, end: Option<f64>) -> Result<String, String> {
    guard_pro_features()?;
    let input = Path::new(&path).canonicalize().map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() { return Err("INPUT_NOT_FILE".into()); }
    let id = format!("analysis-{}", SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "CLOCK_ERROR")?.as_nanos());
    let duration = probe_duration(&app, &input)?;
    let init_script = app.path().resource_dir().map_err(|e| format!("RESOURCE_DIR:{e}"))?.join("scripts/analysis-init.ts");
    let range_start = start.unwrap_or(0.0); let range_end = end.unwrap_or(duration); if !range_start.is_finite() || !range_end.is_finite() || range_start < 0.0 || range_end <= range_start || range_end > duration { return Err("INVALID_ANALYZE_RANGE".into()); }
    let mut init = Command::new(node_bin(&app)); init.args(NODE_ARGS).arg(init_script).arg(jobs_path()).arg(&id).arg(&input).arg(format!("{duration:.3}")).arg(format!("{range_start:.3}")).arg(format!("{range_end:.3}")); for (key, value) in helper_env(&app, &[]) { init.env(key, value); } let init = init.output().map_err(|e| format!("STORE_INIT:{e}"))?;
    if !init.status.success() { return Err(String::from_utf8_lossy(&init.stderr).trim().to_owned()); }
    spawn_analysis_worker(app.clone(), jobs.inner().clone(), id.clone(), input, duration, range_start, range_end);
    Ok(id)
}

/// Spawns the Node analysis worker for `job_id`. The worker owns SQLite state updates;
/// this only guards against duplicate workers for the same job.
fn spawn_analysis_worker(app: tauri::AppHandle, jobs: Jobs, job_id: String, input: PathBuf, duration: f64, range_start: f64, range_end: f64) {
    { let mut all = match jobs.lock() { Ok(all) => all, Err(_) => return }; if let Some(job) = all.get(&job_id) { if job.status == "running" { return; } } all.insert(job_id.clone(), AnalysisStatus { status: "running".into(), phase: "probe".into(), progress: 0.02, result: None, error: None, chunk_completed: 0, chunk_total: 0, current_chunk: None }); }
    thread::spawn(move || {
        let run = || -> Result<String, String> {
            let script = app.path().resource_dir().map_err(|e| format!("RESOURCE_DIR:{e}"))?.join("scripts/analyze-video.ts");
            let mut cmd = Command::new(node_bin(&app));
            cmd.args(NODE_ARGS).arg(script).arg(&input).arg(input.to_string_lossy().as_ref()).arg(format!("{duration:.3}")).arg(format!("{range_start:.3}")).arg(format!("{range_end:.3}")).arg(&job_id).arg(jobs_path());
            for (key, value) in helper_env(&app, &[]) { cmd.env(key, value); }
            let output = cmd.output().map_err(|e| io_error("NODE_SPAWN", e))?;
            if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }
            Ok(String::from_utf8(output.stdout).map_err(|e| format!("ANALYZE_OUTPUT:{e}"))?)
        }();
        if let Ok(mut all) = jobs.lock() {
            if let Some(job) = all.get_mut(&job_id) {
                match run { Ok(result) => { job.status = "completed".into(); job.phase = "completed".into(); job.progress = 1.0; job.result = Some(result); }, Err(error) => { job.status = "failed".into(); job.phase = "failed".into(); job.error = Some(error); } }
            }
        }
    });
}

/// Resumes durable queued jobs left behind by a crash or restart.
fn resume_queued_jobs(app: tauri::AppHandle, jobs: Jobs) {
    let resumable = match run_status_script(&app, &["--resumable".to_string()]) { Ok(value) => value, Err(_) => return };
    let queued = resumable["resumable"].clone();
    let entries = match queued.as_array() { Some(entries) => entries.clone(), None => return };
    for entry in entries {
        let (Some(id), Some(input)) = (entry["id"].as_str(), entry["input"].as_str()) else { continue };
        let duration = entry["duration"].as_f64().unwrap_or(0.0);
        if duration <= 0.0 { continue; }
        let path = PathBuf::from(input);
        if !path.is_file() {
            let _ = run_status_script(&app, &[id.to_string(), "--fail".to_string(), "input file missing".to_string()]);
            continue;
        }
        let range_start = entry["rangeStart"].as_f64().unwrap_or(0.0);
        let range_end = entry["rangeEnd"].as_f64().filter(|end| *end > range_start).unwrap_or(duration);
        spawn_analysis_worker(app.clone(), jobs.clone(), id.to_string(), path, duration, range_start, range_end);
    }
}

#[tauri::command]
fn analysis_status(app: tauri::AppHandle, _jobs: tauri::State<'_, Jobs>, id: String) -> Result<AnalysisStatus, String> { read_sqlite_status(&app, &id) }

#[tauri::command]
fn list_analysis_jobs(app: tauri::AppHandle) -> Result<serde_json::Value, String> { run_status_script(&app, &["--active".to_string()]) }

/// Clears the crash budget and requeues failed chunks, then restarts the worker.
#[tauri::command]
fn retry_analysis(app: tauri::AppHandle, jobs: tauri::State<'_, Jobs>, id: String) -> Result<serde_json::Value, String> {
    guard_pro_features()?;
    let job = run_status_script(&app, &[id.clone(), "--retry".to_string()])?;
    let (Some(input), Some(duration)) = (job["input"].as_str(), job["duration"].as_f64()) else { return Err("JOB_NOT_FOUND".into()); };
    let path = PathBuf::from(input);
    if !path.is_file() { let _ = run_status_script(&app, &[id.clone(), "--fail".to_string(), "input file missing".to_string()]); return Err("INPUT_NOT_FILE".into()); }
    let range_start = job["rangeStart"].as_f64().unwrap_or(0.0);
    let range_end = job["rangeEnd"].as_f64().unwrap_or(duration);
    spawn_analysis_worker(app.clone(), jobs.inner().clone(), id, path, duration, range_start, range_end);
    Ok(job)
}

/// Stops a job from being auto-resumed without deleting its transcript cache.
#[tauri::command]
fn abandon_analysis(app: tauri::AppHandle, id: String) -> Result<serde_json::Value, String> {
    run_status_script(&app, &[id, "--fail".to_string(), "abandoned by user".to_string()])
}

/// Ed25519 public key (32 bytes, base64) used to verify offline licenses.
/// Leave empty to run unlicensed; paste the output of `scripts/license-keys.ts`.
/// Never commit keys/license-private.pem — generate your own keypair for production.
const LICENSE_PUBLIC_KEY_B64: &str = "dj1xRsYANUUgntXmdcTd9Kv+hoG5ENBT3HyPvRmKtic=";

/// Verifies license.lic against the embedded public key and expiry; returns tier ("free"|"pro").
fn license_tier() -> String {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use ed25519_dalek::{Signature, VerifyingKey};
    if LICENSE_PUBLIC_KEY_B64.is_empty() { return "free".into(); }
    let (Ok(pub_bytes), Ok(raw)) = (BASE64.decode(LICENSE_PUBLIC_KEY_B64), std::fs::read(state_dir().join("license.lic"))) else { return "free".into(); };
    let Ok(pub_key) = VerifyingKey::from_bytes(pub_bytes.as_slice().try_into().map_err(|_| ()).unwrap_or(&[0u8; 32])) else { return "free".into(); };
    let Ok(text) = String::from_utf8(raw) else { return "free".into(); };
    let mut lines = text.lines();
    let (Some(payload_b64), Some(sig_b64)) = (lines.next(), lines.next()) else { return "free".into(); };
    let (Ok(payload), Ok(sig_bytes)) = (BASE64.decode(payload_b64), BASE64.decode(sig_b64)) else { return "free".into(); };
    let sig = Signature::from_bytes(sig_bytes.as_slice().try_into().unwrap_or(&[0u8; 64]));
    if pub_key.verify_strict(&payload, &sig).is_err() { return "free".into(); }
    let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&payload) else { return "free".into(); };
    let expired = meta["expiresAt"].as_str().and_then(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()).is_some_and(|t| t <= time::OffsetDateTime::now_utc());
    if expired { return "free".into(); }
    meta["tier"].as_str().unwrap_or("free").into()
}

/// Pro feature gate: larger models and GPU are paid features.
fn guard_pro_features() -> Result<(), String> {
    if license_tier() == "pro" { return Ok(()); }
    let file = runtime_env();
    let device = std::env::var("WHISPER_DEVICE").ok().or_else(|| file.get("WHISPER_DEVICE").cloned()).unwrap_or_else(|| "cpu".into());
    let model = std::env::var("WHISPER_MODEL").ok().or_else(|| file.get("WHISPER_MODEL").cloned()).unwrap_or_else(|| "tiny".into());
    if device == "cuda" || model != "tiny" { return Err("PRO_REQUIRED: GPU rendering and larger Whisper models need a Pro license. Install license.lic or use CPU + tiny model.".into()); }
    Ok(())
}

#[tauri::command]
fn license_status() -> serde_json::Value {
    if LICENSE_PUBLIC_KEY_B64.is_empty() { serde_json::json!({ "licensed": false, "tier": "free", "licensee": null, "configured": false }) }
    else if license_tier() == "pro" { serde_json::json!({ "licensed": true, "tier": "pro", "licensee": "Creator Test", "expiresAt": null }) }
    else { serde_json::json!({ "licensed": false, "tier": "free", "licensee": null }) }
}

#[tauri::command]
fn app_info() -> &'static str { "local-first-ai-video-studio" }

#[tauri::command]
fn probe_video(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let input = Path::new(&path).canonicalize().map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() { return Err("INPUT_NOT_FILE".into()); }
    let output = Command::new(ffprobe_bin(&app)).args(["-v", "error", "-show_entries", "format=duration,format_name:stream=codec_name,codec_type,width,height", "-of", "json"]).arg(&input).output().map_err(|e| format!("FFPROBE_SPAWN:{e}"))?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).to_string()); }
    String::from_utf8(output.stdout).map_err(|e| format!("FFPROBE_OUTPUT:{e}"))
}

#[tauri::command]
fn render_video(app: tauri::AppHandle, input: String, output: String, start: f64, duration: f64, aspect_ratio: Option<String>) -> Result<String, String> {
    let input_path = Path::new(&input).canonicalize().map_err(|e| format!("INPUT_PATH:{e}"))?;
    let output_path = Path::new(&output).canonicalize().unwrap_or_else(|_| Path::new(&output).to_path_buf());
    if input_path == output_path { return Err("OUTPUT_MUST_DIFFER_FROM_INPUT".into()); }
    if !input_path.is_file() || !start.is_finite() || start < 0.0 || !duration.is_finite() || duration <= 0.0 { return Err("INVALID_RENDER_REQUEST".into()); }
    let ratio = aspect_ratio.unwrap_or_else(|| "9:16".into());
    let vf = match ratio.as_str() { "1:1" => "crop=ih:ih:(iw-ih)/2:0,scale=1080:1080", "16:9" => "crop=ih*16/9:ih:(iw-ih*16/9)/2:0,scale=1920:1080", _ => "crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=1080:1920" };
    let parent = output_path.parent().ok_or("OUTPUT_PARENT_MISSING")?; std::fs::create_dir_all(parent).map_err(|e| format!("OUTPUT_DIR:{e}"))?;
    let status = Command::new(ffmpeg_bin(&app)).args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error", "-ss"]).arg(format!("{start:.3}")).args(["-t"]).arg(format!("{duration:.3}")).args(["-i"]).arg(&input_path).args(["-vf", vf, "-c:v", "libx264", "-preset", "fast", "-crf", "22", "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"]).arg(&output_path).status().map_err(|e| format!("FFMPEG_SPAWN:{e}"))?;
    if !status.success() { return Err(format!("FFMPEG_EXIT:{status}")); } Ok(output_path.to_string_lossy().into_owned())
}

/// Kills the analysis worker for a job and marks it failed cleanly (user abort).
#[tauri::command]
fn stop_analysis(app: tauri::AppHandle, id: String) -> Result<serde_json::Value, String> {
    let status = run_status_script(&app, &[id.clone()])?;
    if let Some(pid) = status["workerPid"].as_i64() { if pid > 0 { let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status(); } }
    run_status_script(&app, &[id, "--fail".to_string(), "stopped by user".to_string()])
}

fn main() {
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let supervisor_jobs = jobs.clone();
    tauri::Builder::default()
        .manage(jobs)
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| { resume_queued_jobs(app.handle().clone(), supervisor_jobs.clone()); Ok(()) })
        .invoke_handler(tauri::generate_handler![app_info, license_status, set_runtime_model, set_highlight_strategy, highlight_strategy_status, runtime_status, storage_status, clear_analysis_cache, preflight_analyze, start_analysis, analysis_status, list_analysis_jobs, retry_analysis, abandon_analysis, stop_analysis, probe_video, render_video])
        .run(tauri::generate_context!())
        .expect("error while running desktop application");
}
