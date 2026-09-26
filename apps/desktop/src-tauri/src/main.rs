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
    let command = file.get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty() && whisper_usable(c)).unwrap_or_else(whisper_command);
    let device = std::env::var("WHISPER_DEVICE").ok().or_else(|| file.get("WHISPER_DEVICE").cloned()).unwrap_or_else(|| "cpu".into());
    let fp16 = if device == "cuda" { "True" } else { "False" };
    let highlight_strategy = file.get("HIGHLIGHT_STRATEGY").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "heuristic".into());
    let gemini_api_key = file.get("GEMINI_API_KEY").cloned().filter(|c| !c.is_empty()).unwrap_or_default();
    let local_llm_base_url = file.get("LOCAL_LLM_BASE_URL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "http://127.0.0.1:8080".into());
    let local_llm_model = file.get("LOCAL_LLM_MODEL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct".into());
    let text = format!("WHISPER_COMMAND={command}\nWHISPER_MODEL={model}\nWHISPER_DEVICE={device}\nWHISPER_FP16={fp16}\nHIGHLIGHT_STRATEGY={highlight_strategy}\nGEMINI_API_KEY={gemini_api_key}\nLOCAL_LLM_BASE_URL={local_llm_base_url}\nLOCAL_LLM_MODEL={local_llm_model}\n");
    std::fs::create_dir_all(state_dir()).map_err(|e| format!("STATE_DIR:{e}"))?;
    std::fs::write(state_dir().join("runtime.env"), text).map_err(|e| format!("RUNTIME_ENV:{e}"))?;
    Ok(model)
}

/// Persists the highlight strategy ("heuristic" | "semantic-gemini" | "semantic-local"),
/// an optional Gemini API key, and the local-LLM endpoint into runtime.env so the
/// analysis worker picks the right strategy.
#[tauri::command]
fn set_highlight_strategy(_app: tauri::AppHandle, strategy: String, gemini_api_key: String, local_llm_base_url: String, local_llm_model: String) -> Result<String, String> {
    if !["heuristic", "semantic-gemini", "semantic-local"].contains(&strategy.as_str()) { return Err("INVALID_STRATEGY: choose heuristic, semantic-gemini or semantic-local".into()); }
    let file = runtime_env();
    let command = file.get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty() && whisper_usable(c)).unwrap_or_else(whisper_command);
    let model = file.get("WHISPER_MODEL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "tiny".into());
    let device = file.get("WHISPER_DEVICE").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "cpu".into());
    let fp16 = file.get("WHISPER_FP16").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "False".into());
    // Keep previously stored values when switching strategies so the user's key/url are not lost.
    let key = if strategy == "semantic-gemini" { gemini_api_key.clone() } else { file.get("GEMINI_API_KEY").cloned().unwrap_or_default() };
    let llama_url = if strategy == "semantic-local" { local_llm_base_url.clone() } else { file.get("LOCAL_LLM_BASE_URL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "http://127.0.0.1:8080".into()) };
    let llama_model = if strategy == "semantic-local" { local_llm_model.clone() } else { file.get("LOCAL_LLM_MODEL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct".into()) };
    if strategy == "semantic-gemini" && key.trim().is_empty() { return Err("SEMANTIC_REQUIRES_GEMINI_API_KEY".into()); }
    if strategy == "semantic-local" && llama_url.trim().is_empty() { return Err("LOCAL_LLM_REQUIRES_BASE_URL".into()); }
    let text = format!("WHISPER_COMMAND={command}\nWHISPER_MODEL={model}\nWHISPER_DEVICE={device}\nWHISPER_FP16={fp16}\nHIGHLIGHT_STRATEGY={strategy}\nGEMINI_API_KEY={key}\nLOCAL_LLM_BASE_URL={llama_url}\nLOCAL_LLM_MODEL={llama_model}\n");
    std::fs::create_dir_all(state_dir()).map_err(|e| format!("STATE_DIR:{e}"))?;
    std::fs::write(state_dir().join("runtime.env"), text).map_err(|e| format!("RUNTIME_ENV:{e}"))?;
    Ok(strategy)
}
/// Reports the currently stored highlight strategy plus key/url presence flags
/// (the key and url themselves are never returned to the UI).
#[tauri::command]
fn highlight_strategy_status(_app: tauri::AppHandle) -> serde_json::Value {
    let file = runtime_env();
    let strategy = file.get("HIGHLIGHT_STRATEGY").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "heuristic".into());
    let has_key = file.get("GEMINI_API_KEY").cloned().map(|k| !k.trim().is_empty()).unwrap_or(false);
    let has_local_url = file.get("LOCAL_LLM_BASE_URL").cloned().map(|k| !k.trim().is_empty()).unwrap_or(false);
    serde_json::json!({ "strategy": strategy, "hasKey": has_key, "hasLocalUrl": has_local_url })
}
fn node_bin(app: &tauri::AppHandle) -> PathBuf { bundled_bin(app, "node/bin/node", "node") }
fn ffmpeg_bin(app: &tauri::AppHandle) -> PathBuf { bundled_bin(app, "ffmpeg", "ffmpeg") }
fn ffprobe_bin(app: &tauri::AppHandle) -> PathBuf { bundled_bin(app, "ffprobe", "ffprobe") }
/// Resolve the whisper CLI the way the local whisper adapter will find it:
/// env/runtime.env first, then a venv next to the app, then plain PATH.
/// True when a whisper CLI string can actually be executed: a path that exists, or a
/// bare name resolvable via PATH. Guards stale runtime.env entries like `whisper` when
/// whisper is not installed system-wide.
fn whisper_usable(cmd: &str) -> bool {
    if cmd.contains('/') { Path::new(cmd).is_file() } else { command_ready(cmd) }
}
fn whisper_command() -> String {
    if let Ok(cmd) = std::env::var("WHISPER_COMMAND") { if !cmd.is_empty() && whisper_usable(&cmd) { return cmd; } }
    // runtime.env is authoritative only while the command it names still resolves.
    if let Some(cmd) = runtime_env().get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty()) {
        if whisper_usable(&cmd) { return cmd; }
    }
    // App-owned venv created by the first-run bootstrap (client machines).
    let app_venv = whisper_venv_bin("whisper");
    if app_venv.is_file() { return app_venv.to_string_lossy().into_owned(); }
    // Project venv: repo root is three levels up from src-tauri; also accept the legacy
    // apps/desktop/.venv location from earlier checkouts.
    for rel in ["../../../.venv/bin/whisper", "../.venv/bin/whisper"] {
        let candidate = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        if candidate.is_file() { return candidate.to_string_lossy().into_owned(); }
    }
    if let Ok(home) = std::env::var("HOME") { if Path::new(&home).join(".local/bin/whisper").is_file() { return format!("{home}/.local/bin/whisper"); } }
    "whisper".into()
}
type LlamaState = Arc<Mutex<Option<u32>>>; // pid of the detached llama-server, if running
fn llama_binary() -> PathBuf {
    if let Ok(path) = std::env::var("LOCAL_LLM_BINARY") { if Path::new(&path).is_file() { return PathBuf::from(path); } }
    if let Ok(state) = std::env::var("LOCAL_FIRST_STATE_DIR") {
        let candidate = Path::new(&state).join("llama/llama-server");
        if candidate.is_file() { return candidate; }
    }
    let project_resource = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/llama/llama-server");
    if project_resource.is_file() { return project_resource; }
    PathBuf::from("llama-server")
}
fn local_llm_model_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>, model: &str) -> Option<PathBuf> {
    let resource_dir = app.path().resource_dir().ok()?;
    let state = state_dir();
    let candidates = [
        state.join("models/llama").join(format!("{model}.gguf")),
        resource_dir.join("models/llama").join(format!("{model}.gguf")),
    ];
    for candidate in &candidates { if candidate.is_file() { return Some(candidate.clone()); } }
    None
}
fn ensure_llama_server<R: tauri::Runtime>(state: &LlamaState, app: &tauri::AppHandle<R>, port: u16) -> Result<(), String> {
    // If a llama-server is already answering on this port, reuse it.
    let health_url = format!("http://127.0.0.1:{port}/health");
    if Command::new("curl").args(["-s", "-f", "-o", "/dev/null", "-w", "%{http_code}", &health_url]).output().ok().map(|out| String::from_utf8_lossy(&out.stdout).trim() == "200").unwrap_or(false) {
        return Ok(());
    }
    let binary = llama_binary();
    let model = runtime_env().get("LOCAL_LLM_MODEL").cloned().filter(|m| !m.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    let model_path = local_llm_model_path(app, &model).ok_or_else(|| format!("LLAMA_MODEL_MISSING:{model}"))?;
    let mut cmd = Command::new(&binary);
    cmd.arg("-m").arg(&model_path)
        .arg("--port").arg(port.to_string())
        .arg("--host").arg("127.0.0.1")
        .arg("--ctx-size").arg("4096")
        .arg("--parallel").arg("2")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let spawned = cmd.spawn().map_err(|e| format!("LLAMA_SPAWN:{e}"))?;
    let pid = spawned.id();
    // Detach: drop our handle but keep the pid so we can stop it later.

    drop(spawned);
    let mut ready = false;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if Command::new("curl").args(["-s", "-f", "-o", "/dev/null", "-w", "%{http_code}", &health_url]).output().ok().map(|out| String::from_utf8_lossy(&out.stdout).trim() == "200").unwrap_or(false) {
            ready = true;
            break;
        }
    }
    if !ready {
        #[cfg(unix)] let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
        return Err("LLAMA_SERVER_TIMEOUT".into());
    }
    if let Ok(mut guard) = state.lock() { let _ = guard.insert(pid); }
    Ok(())
}

#[tauri::command]
fn start_local_llm(app: tauri::AppHandle, state: tauri::State<'_, LlamaState>) -> Result<String, String> {
    let port: u16 = 8080;
    ensure_llama_server(&state, &app, port)?;
    Ok(format!("http://127.0.0.1:{port}"))
}

#[tauri::command]
fn stop_local_llm(state: tauri::State<'_, LlamaState>) -> Result<(), String> {
    if let Ok(mut guard) = state.lock() {
        if let Some(pid) = guard.take() {
            #[cfg(unix)] { let _ = nix_kill(pid); }
            #[cfg(not(unix))] { let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status(); }
        }
    }
    Ok(())
}
#[cfg(unix)]
fn nix_kill(pid: u32) -> std::io::Result<()> {
    std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status().map(|_| ())
}
// Newtype: LlamaState is also Arc<Mutex<Option<u32>>>. Tauri keys managed state
// by the concrete type, so aliasing both would register the same type twice and
// panic ("already being managed"). This wrapper keeps them distinct.
#[derive(Clone, Default)]
struct RagState(Arc<Mutex<Option<u32>>>); // pid of the detached RAG sidecar, if running
const RAG_PORT: u16 = 4733;
/// Where the bundled adapter-node RAG server entry lives (web/build-node/index.js
/// bundled as the `build-node` resource). Falls back to the dev-tree build so a
/// `tauri dev` run without a packaged resource still works.
fn rag_entry(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let candidate = app.path().resource_dir().map_err(|e| format!("RESOURCE_DIR:{e}"))?.join("build-node/index.js");
    if candidate.is_file() { return Ok(candidate); }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/build-node/index.js");
    if dev.is_file() { return Ok(dev); }
    Err("RAG_SERVER_MISSING: web/build-node/index.js not found in resources or dev tree (run: npm run build:node)".into())
}
/// True when the RAG sidecar is already answering on this port (GET playlists is 200).
fn rag_healthy(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/api/rag/playlists");
    Command::new("curl").args(["-s", "-f", "-o", "/dev/null", "-w", "%{http_code}", &url]).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "200").unwrap_or(false)
}
/// Idempotently spawn the RAG sidecar (bundled node + build-node entry) on
/// 127.0.0.1:RAG_PORT and wait until it answers; reuses an instance already
/// healthy. Returns the base URL the static webview should call via plugin-http.
fn ensure_rag_server(state: &RagState, app: &tauri::AppHandle) -> Result<String, String> {
    let base = format!("http://127.0.0.1:{RAG_PORT}");
    if rag_healthy(RAG_PORT) { return Ok(base); }
    let entry = rag_entry(app)?;
    let mut cmd = Command::new(node_bin(app));
    cmd.arg(&entry)
        .env("PORT", RAG_PORT.to_string())
        .env("HOST", "127.0.0.1")
        .env("LOCAL_FIRST_STATE_DIR", state_dir().to_string_lossy().into_owned())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let spawned = cmd.spawn().map_err(|e| format!("RAG_SPAWN:{e}"))?;
    let pid = spawned.id();
    drop(spawned); // detach: keep only the pid so we can stop it later
    let mut ready = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if rag_healthy(RAG_PORT) { ready = true; break; }
    }
    if !ready {
        #[cfg(unix)] let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
        return Err("RAG_SERVER_TIMEOUT".into());
    }
    if let Ok(mut guard) = state.0.lock() { let _ = guard.insert(pid); }
    Ok(base)
}
#[tauri::command]
fn rag_server_url(app: tauri::AppHandle, state: tauri::State<'_, RagState>) -> Result<String, String> {
    ensure_rag_server(state.inner(), &app)
}
fn status_script(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // Bundled resources come first: in a dev build the tauri dev runtime wires
    // the resource dir to src-tauri/resources, which does not contain the
    // analysis scripts. Installed builds ship scripts/ (bundle.resources), so
    // the resource-dir hit only succeeds there. Fall back to the dev-tree
    // scripts/ next to the repo root.
    let candidate = app.path().resource_dir().map_err(|e| format!("RESOURCE_DIR:{e}"))?.join("scripts/analysis-status.ts");
    if candidate.is_file() { return Ok(candidate); }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join("scripts/analysis-status.ts");
    if dev.is_file() { return Ok(dev); }
    Err("STATUS_SCRIPT_MISSING: scripts/analysis-status.ts not found in resources or the dev tree".into())
}
/// Resolves a bundled helper script by name with the same resource-dir-first,
/// dev-tree-fallback strategy as status_script().
fn helper_script(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let candidate = app.path().resource_dir().map_err(|e| format!("RESOURCE_DIR:{e}"))?.join(format!("scripts/{name}"));
    if candidate.is_file() { return Ok(candidate); }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join("scripts").join(name);
    if dev.is_file() { return Ok(dev); }
    Err(format!("SCRIPT_MISSING:scripts/{name} not found in resources or the dev tree"))
}
const NODE_ARGS: &[&str] = &["--experimental-strip-types"];
fn run_status_script(app: &tauri::AppHandle, args: &[String]) -> Result<serde_json::Value, String> { let script = status_script(app)?; let mut cmd = Command::new(node_bin(app)); cmd.args(NODE_ARGS).arg(&script).arg(jobs_path()).args(args); for (key, value) in helper_env(app, &[]) { cmd.env(key, value); } let output = cmd.output().map_err(|e| format!("STATUS_SPAWN:{e}"))?; if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); } serde_json::from_slice(&output.stdout).map_err(|e| format!("STATUS_JSON:{e}")) }
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
/// Copies the bundled Whisper model into ~/.cache/whisper so the openai-whisper CLI
/// (which looks there by default) finds it without downloading. No-op if the
/// model is already present.
fn ensure_bundled_whisper_model<R: tauri::Runtime>(app: &tauri::AppHandle<R>, model: &str) -> Result<(), String> {
    let model_name = model.to_string();
    let dest = match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".cache/whisper").join(format!("{model_name}.pt")),
        Err(_) => return Ok(()),
    };
    if dest.exists() { return Ok(()); }
    let source = match app.path().resource_dir() {
        Ok(dir) => dir.join(format!("models/{model_name}.pt")),
        Err(_) => return Ok(()),
    };
    if !source.is_file() { return Ok(()); }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("WHISPER_MODEL_DIR:{e}"))?;
    }
    std::fs::copy(&source, &dest).map_err(|e| format!("WHISPER_MODEL_COPY:{e}"))?;
    Ok(())
}

/// Where the app-owned whisper venv lives (created on first run for machines that have
/// neither a project venv nor a whisper on PATH).
fn whisper_venv_dir() -> PathBuf { state_dir().join(".venv") }
fn whisper_venv_bin(name: &str) -> PathBuf {
    #[cfg(windows)] { whisper_venv_dir().join("Scripts").join(format!("{name}.exe")) }
    #[cfg(not(windows))] { whisper_venv_dir().join("bin").join(name) }
}

#[derive(Clone)]
struct BootstrapState(Arc<Mutex<Option<BootstrapProgress>>>);
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapProgress { phase: String, message: String, done: bool, error: Option<String> }
/// Picks a Python interpreter that can build a venv, preferring a version torch still
/// supports (3.10–3.13); falls back to python3 on PATH.
fn python_binary() -> String {
    for candidate in ["python3.12", "python3.11", "python3.13", "python3.10", "python3"] {
        if Command::new(candidate).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) { return candidate.to_string(); }
    }
    "python3".into()
}
fn set_bootstrap(state: &BootstrapState, phase: &str, message: &str, done: bool, error: Option<String>) {
    if let Ok(mut guard) = state.0.lock() { let _ = guard.insert(BootstrapProgress { phase: phase.into(), message: message.into(), done, error }); }
}
/// Idempotent first-run setup: create the app venv, install openai-whisper, then write
/// runtime.env so every helper resolves the same whisper. Progress is polled by the UI.
fn run_whisper_bootstrap<R: tauri::Runtime>(app: &tauri::AppHandle<R>, state: &BootstrapState) -> Result<String, String> {
    let venv = whisper_venv_dir();
    let whisper = whisper_venv_bin("whisper");
    if whisper.is_file() {
        set_bootstrap(state, "ready", "Whisper already installed.", true, None);
        return Ok(whisper.to_string_lossy().into_owned());
    }
    // Fast path: a usable whisper already exists on this machine (project venv, install
    // prefix, or PATH). Never re-install a multi-GB PyTorch stack in that case.
    let existing = whisper_command();
    if whisper_usable(&existing) {
        let file = runtime_env();
        let model = file.get("WHISPER_MODEL").cloned().filter(|m| !m.is_empty()).unwrap_or_else(|| "tiny".into());
        let _ = ensure_bundled_whisper_model(app, &model);
        // A failed runtime.env write must surface: reporting success without it would
        // leave the analysis worker resolving the wrong whisper later.
        write_runtime_env(&existing, &model)?;
        set_bootstrap(state, "ready", "Whisper found on this machine.", true, None);
        return Ok(existing);
    }
    let python = python_binary();
    if !venv.join("pyvenv.cfg").is_file() {
        set_bootstrap(state, "venv", &format!("Creating Python environment with {python}…"), false, None);
        std::fs::create_dir_all(state_dir()).map_err(|e| format!("STATE_DIR:{e}"))?;
        let out = Command::new(&python).args(["-m", "venv"]).arg(&venv).output().map_err(|e| format!("PYTHON_SPAWN:{e}"))?;
        if !out.status.success() { return Err(format!("VENV_FAILED:{}", String::from_utf8_lossy(&out.stderr).trim())); }
    }
    let pip = whisper_venv_bin("pip");
    set_bootstrap(state, "pip", "Upgrading pip…", false, None);
    let _ = Command::new(&pip).args(["install", "--upgrade", "pip"]).output();
    set_bootstrap(state, "whisper", "Installing openai-whisper + crawler deps (this downloads PyTorch, a few GB)…", false, None);
    let install = Command::new(&pip).args(["install", "openai-whisper", "yt-dlp", "youtube-transcript-api"]).output().map_err(|e| format!("PIP_SPAWN:{e}"))?;
    if !install.status.success() { return Err(format!("WHISPER_INSTALL_FAILED:{}", String::from_utf8_lossy(&install.stderr).lines().last().unwrap_or("").trim())); }
    if !whisper.is_file() { return Err("WHISPER_BINARY_MISSING".into()); }
    let model = runtime_env().get("WHISPER_MODEL").cloned().filter(|m| !m.is_empty()).unwrap_or_else(|| "tiny".into());
    let _ = ensure_bundled_whisper_model(app, &model);
    set_bootstrap(state, "env", "Writing runtime configuration…", false, None);
    write_runtime_env(&whisper.to_string_lossy(), &model)?;
    set_bootstrap(state, "ready", "Whisper is ready.", true, None);
    Ok(whisper.to_string_lossy().into_owned())
}

/// Writes runtime.env preserving every previously chosen option (strategy, keys, endpoints).
fn write_runtime_env(whisper_path: &str, model: &str) -> Result<(), String> {
    let file = runtime_env();
    let strategy = file.get("HIGHLIGHT_STRATEGY").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "heuristic".into());
    let gemini = file.get("GEMINI_API_KEY").cloned().unwrap_or_default();
    let llama_url = file.get("LOCAL_LLM_BASE_URL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "http://127.0.0.1:8080".into());
    let llama_model = file.get("LOCAL_LLM_MODEL").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    let device = std::env::var("WHISPER_DEVICE").ok().unwrap_or_else(|| "cpu".into());
    let fp16 = if device == "cuda" { "True" } else { "False" };
    let text = format!("WHISPER_COMMAND={whisper_path}\nWHISPER_MODEL={model}\nWHISPER_DEVICE={device}\nWHISPER_FP16={fp16}\nHIGHLIGHT_STRATEGY={strategy}\nGEMINI_API_KEY={gemini}\nLOCAL_LLM_BASE_URL={llama_url}\nLOCAL_LLM_MODEL={llama_model}\n");
    std::fs::create_dir_all(state_dir()).map_err(|e| format!("STATE_DIR:{e}"))?;
    std::fs::write(state_dir().join("runtime.env"), text).map_err(|e| format!("RUNTIME_ENV:{e}"))
}

/// Starts the whisper bootstrap in the background (first run on a clean machine).
#[tauri::command]
fn bootstrap_whisper(app: tauri::AppHandle, state: tauri::State<'_, BootstrapState>) -> Result<(), String> {
    if let Ok(guard) = state.0.lock() { if let Some(progress) = guard.as_ref() { if !progress.done { return Ok(()); } } }
    set_bootstrap(&state, "starting", "Preparing Whisper…", false, None);
    let app_handle = app.clone();
    let shared = state.inner().clone();
    thread::spawn(move || { if let Err(error) = run_whisper_bootstrap(&app_handle, &shared) { set_bootstrap(&shared, "failed", "Whisper setup failed.", true, Some(error)); } });
    Ok(())
}

#[tauri::command]
fn whisper_bootstrap_status(state: tauri::State<'_, BootstrapState>) -> Option<BootstrapProgress> {
    state.0.lock().ok().and_then(|guard| guard.clone())
}

#[derive(Clone)]
struct ModelDownloadState(Arc<Mutex<Option<BootstrapProgress>>>);
const LLM_MODEL_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf";
const LLM_MODEL_SHA256: &str = "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d";
fn llm_model_file(model: &str) -> PathBuf { state_dir().join("models/llama").join(format!("{model}.gguf")) }
fn set_download(state: &ModelDownloadState, phase: &str, message: &str, done: bool, error: Option<String>) {
    if let Ok(mut guard) = state.0.lock() { let _ = guard.insert(BootstrapProgress { phase: phase.into(), message: message.into(), done, error }); }
}
/// Downloads the local-LLM weights on demand (2.1 GB) and verifies the SHA-256 before use.
/// Fetch `url` into `target`, verify its SHA-256 against `expected_sha`, then rename the
/// partial file into place. Leftover partials are removed on any failure so a retry
/// starts clean. Kept separate from run_model_download so tests can drive it with
/// file:// URLs instead of a 2.1 GB network download.
fn download_and_verify(state: &ModelDownloadState, url: &str, expected_sha: &str, target: &Path) -> Result<String, String> {
    if target.is_file() {
        set_download(state, "ready", "Model already downloaded.", true, None);
        return Ok(target.to_string_lossy().into_owned());
    }
    if let Some(parent) = target.parent() { std::fs::create_dir_all(parent).map_err(|e| format!("MODEL_DIR:{e}"))?; }
    let part = PathBuf::from(format!("{}.part", target.display()));
    // A leftover .part means a previous download was killed mid-flight; start clean so
    // we never resume into bytes the checksum would then reject.
    if part.is_file() { let _ = std::fs::remove_file(&part); }
    set_download(state, "download", "Downloading Qwen2.5-3B (~2.1 GB)…", false, None);
    let curl = Command::new("curl").args(["-L", "--fail", "--retry", "3", "-o"]).arg(&part).arg(url).output().map_err(|e| format!("CURL_SPAWN:{e}"))?;
    if !curl.status.success() { let _ = std::fs::remove_file(&part); return Err(format!("MODEL_DOWNLOAD_FAILED:{}", String::from_utf8_lossy(&curl.stderr).trim())); }
    set_download(state, "verify", "Verifying checksum…", false, None);
    let digest = Command::new("sha256sum").arg(&part).output().map_err(|e| format!("SHA256_SPAWN:{e}"))?;
    let actual = String::from_utf8_lossy(&digest.stdout).split_whitespace().next().unwrap_or("").to_string();
    if actual != expected_sha { let _ = std::fs::remove_file(&part); return Err(format!("MODEL_CHECKSUM_MISMATCH:{actual}")); }
    std::fs::rename(&part, target).map_err(|e| format!("MODEL_RENAME:{e}"))?;
    set_download(state, "ready", "Model ready.", true, None);
    Ok(target.to_string_lossy().into_owned())
}
fn run_model_download(state: &ModelDownloadState, model: &str) -> Result<String, String> {
    download_and_verify(state, LLM_MODEL_URL, LLM_MODEL_SHA256, &llm_model_file(model))
}
#[tauri::command]
fn download_llm_model(state: tauri::State<'_, ModelDownloadState>) -> Result<(), String> {
    if let Ok(guard) = state.0.lock() { if let Some(progress) = guard.as_ref() { if !progress.done { return Ok(()); } } }
    let model = runtime_env().get("LOCAL_LLM_MODEL").cloned().filter(|m| !m.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    set_download(&state, "starting", "Preparing download…", false, None);
    let shared = state.inner().clone();
    thread::spawn(move || { if let Err(error) = run_model_download(&shared, &model) { set_download(&shared, "failed", "Model download failed.", true, Some(error)); } });
    Ok(())
}
#[tauri::command]
fn model_download_status(state: tauri::State<'_, ModelDownloadState>, app: tauri::AppHandle) -> serde_json::Value {
    let model = runtime_env().get("LOCAL_LLM_MODEL").cloned().filter(|m| !m.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    let progress = state.0.lock().ok().and_then(|guard| guard.clone());
    let present = local_llm_model_path(&app, &model).is_some();
    serde_json::json!({ "progress": progress, "modelPresent": present, "model": model })
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
    // A clean machine has neither a venv nor a PATH whisper; runtime_status
    // reports that truthfully so the UI can point the user at the bootstrap
    // instead of assuming whisper_command() resolved something usable.
    let whisper_env = std::env::var("WHISPER_COMMAND").ok().filter(|c| !c.is_empty());
    let whisper_file = file.get("WHISPER_COMMAND").cloned().filter(|c| !c.is_empty());
    let whisper = match (whisper_env, whisper_file) {
        (Some(e), _) => e,
        (None, Some(f)) => f,
        (None, None) => whisper_venv_bin("whisper").to_string_lossy().into_owned(),
    };
    let model = std::env::var("WHISPER_MODEL").ok().or_else(|| file.get("WHISPER_MODEL").cloned()).unwrap_or_else(|| "tiny".into());
    // Ship the bundled model to the whisper cache so the CLI needs no download.
    let _ = ensure_bundled_whisper_model(&app, &model);
    let model_ready = std::env::var("WHISPER_MODEL_PATH").is_ok_and(|p| !p.is_empty() && Path::new(&p).exists())
        || std::env::var("HOME").map(|h| Path::new(&h).join(".cache/whisper").join(format!("{model}.pt")).exists()).unwrap_or(false);
    let device = std::env::var("WHISPER_DEVICE").ok().or_else(|| file.get("WHISPER_DEVICE").cloned()).unwrap_or_else(|| "cpu".into());
    let ffmpeg = ffmpeg_bin(&app); let ffprobe = ffprobe_bin(&app); let node = node_bin(&app);
    let whisper_present = Path::new(&whisper).exists() || (whisper_venv_bin("whisper").is_file() && whisper == whisper_venv_bin("whisper").to_string_lossy().into_owned()) || command_ready(&whisper);
    serde_json::json!({ "ffmpeg": ffmpeg.as_path().is_file() || command_ready(ffmpeg.to_str().unwrap_or("ffmpeg")), "ffprobe": ffprobe.as_path().is_file() || command_ready(ffprobe.to_str().unwrap_or("ffprobe")), "node": node.as_path().is_file() || command_ready(node.to_str().unwrap_or("node")), "whisper": whisper_present, "model": model, "modelReady": model_ready, "device": device, "bundled": { "node": node.as_path().is_file(), "ffmpeg": ffmpeg.as_path().is_file(), "ffprobe": ffprobe.as_path().is_file() } })
}

#[tauri::command]
fn start_analysis(app: tauri::AppHandle, jobs: tauri::State<'_, Jobs>, path: String, start: Option<f64>, end: Option<f64>) -> Result<String, String> {
    let file = runtime_env();
    let whisper_model = file.get("WHISPER_MODEL").cloned().unwrap_or_else(|| "tiny".into());
    let whisper_device = file.get("WHISPER_DEVICE").cloned().unwrap_or_else(|| "cpu".into());
    guard_pro_features(&whisper_model, &whisper_device)?;
    let input = Path::new(&path).canonicalize().map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() { return Err("INPUT_NOT_FILE".into()); }
    let id = format!("analysis-{}", SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "CLOCK_ERROR")?.as_nanos());
    let duration = probe_duration(&app, &input)?;
    let init_script = helper_script(&app, "analysis-init.ts")?;
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
            let script = helper_script(&app, "analyze-video.ts")?;
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
    let file = runtime_env();
    let whisper_model = file.get("WHISPER_MODEL").cloned().unwrap_or_else(|| "tiny".into());
    let whisper_device = file.get("WHISPER_DEVICE").cloned().unwrap_or_else(|| "cpu".into());
    guard_pro_features(&whisper_model, &whisper_device)?;
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
fn guard_pro_features(whisper_model: &str, whisper_device: &str) -> Result<(), String> {
    if license_tier() == "pro" { return Ok(()); }
    let model = if whisper_model.is_empty() {
        std::env::var("WHISPER_MODEL").ok()
            .or_else(|| {
                let file = runtime_env();
                file.get("WHISPER_MODEL").cloned().filter(|c| !c.is_empty())
            })
            .unwrap_or_else(|| "tiny".into())
    } else {
        whisper_model.to_string()
    };
    let device = if whisper_device.is_empty() {
        std::env::var("WHISPER_DEVICE").ok()
            .or_else(|| {
                let file = runtime_env();
                file.get("WHISPER_DEVICE").cloned().filter(|c| !c.is_empty())
            })
            .unwrap_or_else(|| "cpu".into())
    } else {
        whisper_device.to_string()
    };
    if device == "cuda" || model != "tiny" {
        return Err("PRO_REQUIRED: GPU rendering and larger Whisper models need a Pro license. Install license.lic or use CPU + tiny model.".into());
    }
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

/// Runs the playlist crawler (bundled scripts/yt-crawler.py) on the local
/// whisper venv, then stores the result via the RAG sidecar ingest endpoint.
/// Best-effort: a missing crawler/venv yields an explanatory error instead of a panic.
fn run_yt_crawler(app: &tauri::AppHandle, playlist_url: &str, limit: u32) -> Result<serde_json::Value, String> {
    // Resolve the Python venv created by the whisper bootstrap (same dir as whisper_venv_bin()).
    #[cfg(windows)] let venv_python = whisper_venv_dir().join("Scripts").join("python.exe");
    #[cfg(not(windows))] let venv_python = whisper_venv_dir().join("bin").join("python3");
    if !venv_python.is_file() { return Err("PYTHON_VENV_MISSING: run whisper bootstrap first".into()); }

    let crawler = helper_script(app, "yt-crawler.py")?;
    let mut cmd = Command::new(&venv_python);
    cmd.arg(&crawler).arg(playlist_url);
    if limit > 0 { cmd.arg("--limit").arg(limit.to_string()); }
    for (key, value) in helper_env(app, &[]) { cmd.env(key, value); }
    let output = cmd.output().map_err(|e| format!("CRAWLER_SPAWN:{e}"))?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }

    // Crawler writes JSON to stdout: [{videoId,title,segments}]
    let videos: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("CRAWLER_JSON:{e}"))?;

    // Ingest each result via the running RAG sidecar (bundled curl — no new HTTP dep).
    let base = ensure_rag_server(&RagState::default(), app)?;
    let ingest_url = format!("{base}/api/rag/ingest");
    let mut indexed = 0usize;
    for video in &videos {
        let body = serde_json::json!({
            "videoUrl": format!("https://www.youtube.com/watch?v={}", video["videoId"].as_str().unwrap_or_default()),
            "title": video.get("title").cloned(),
            "segments": video.get("segments").cloned().unwrap_or(serde_json::Value::Array(vec![])),
        });
        let body_str = body.to_string();
        let out = Command::new("curl")
            .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-X", "POST",
                   "-H", "content-type: application/json", "-d", &body_str, &ingest_url])
            .output()
            .map_err(|e| format!("INGEST_SPAWN:{e}"))?;
        if !out.status.success() { continue; }
        if String::from_utf8_lossy(&out.stdout).trim().starts_with('2') { indexed += 1; }
    }
    Ok(serde_json::json!({ "videos": videos.len(), "indexed": indexed }))
}

#[tauri::command]
fn crawl_playlist(app: tauri::AppHandle, playlist_url: String, limit: u32) -> Result<serde_json::Value, String> {
    run_yt_crawler(&app, &playlist_url, limit)
}

fn main() {
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let supervisor_jobs = jobs.clone();
    let llama_state: LlamaState = Arc::new(Mutex::new(None));
    let rag_state: RagState = RagState::default();
    let bootstrap_state = BootstrapState(Arc::new(Mutex::new(None)));
    let download_state = ModelDownloadState(Arc::new(Mutex::new(None)));
    let app = tauri::Builder::default()
        .manage(jobs)
        .manage(llama_state.clone())
        .manage(rag_state.clone())
        .manage(bootstrap_state)
        .manage(download_state)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .setup(move |app| { resume_queued_jobs(app.handle().clone(), supervisor_jobs.clone()); Ok(()) })
        .invoke_handler(tauri::generate_handler![app_info, license_status, set_runtime_model, set_highlight_strategy, highlight_strategy_status, start_local_llm, stop_local_llm, rag_server_url, bootstrap_whisper, whisper_bootstrap_status, download_llm_model, model_download_status, runtime_status, storage_status, clear_analysis_cache, preflight_analyze, start_analysis, analysis_status, list_analysis_jobs, retry_analysis, abandon_analysis, stop_analysis, probe_video, render_video, crawl_playlist])
        .build(tauri::generate_context!())
        .expect("error while building desktop application");
    let llama_shutdown = llama_state.clone();
    let rag_shutdown = rag_state.clone();
    app.run(move |_handle, event| {
        if let tauri::RunEvent::Exit = event {
            // App is exiting: stop the detached llama-server and RAG sidecar if running.
            if let Ok(mut guard) = llama_shutdown.lock() {
                if let Some(pid) = guard.take() {
                    #[cfg(unix)] { let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status(); }
                    #[cfg(not(unix))] { let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status(); }
                }
            }
            if let Ok(mut guard) = rag_shutdown.0.lock() {
                if let Some(pid) = guard.take() {
                    #[cfg(unix)] { let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status(); }
                    #[cfg(not(unix))] { let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status(); }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Cargo runs unit tests in parallel threads inside one process; these tests mutate
    /// process-wide env vars, so they take turns on this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard { dir: PathBuf, old_state: Option<String>, old_whisper: Option<String> }
    impl EnvGuard {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("lfai-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let old_state = std::env::var("LOCAL_FIRST_STATE_DIR").ok();
            let old_whisper = std::env::var("WHISPER_COMMAND").ok();
            std::env::set_var("LOCAL_FIRST_STATE_DIR", &dir);
            EnvGuard { dir, old_state, old_whisper }
        }
        fn state_file(&self) -> PathBuf { self.dir.join("runtime.env") }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old_state { Some(v) => std::env::set_var("LOCAL_FIRST_STATE_DIR", v), None => std::env::remove_var("LOCAL_FIRST_STATE_DIR") }
            match &self.old_whisper { Some(v) => std::env::set_var("WHISPER_COMMAND", v), None => std::env::remove_var("WHISPER_COMMAND") }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn write_runtime_env_preserves_strategy_key_and_endpoints() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("preserve");
        std::fs::write(guard.state_file(), "HIGHLIGHT_STRATEGY=semantic-gemini\nGEMINI_API_KEY=sk-test-123\nLOCAL_LLM_BASE_URL=http://127.0.0.1:9999\nLOCAL_LLM_MODEL=qwen-test\n").unwrap();
        write_runtime_env("/opt/custom/whisper", "base").unwrap();
        let text = std::fs::read_to_string(guard.state_file()).unwrap();
        assert!(text.contains("WHISPER_COMMAND=/opt/custom/whisper"), "{text}");
        assert!(text.contains("WHISPER_MODEL=base"), "{text}");
        // Rebooting the app must not lose the choices the user already made.
        assert!(text.contains("HIGHLIGHT_STRATEGY=semantic-gemini"), "{text}");
        assert!(text.contains("GEMINI_API_KEY=sk-test-123"), "{text}");
        assert!(text.contains("LOCAL_LLM_BASE_URL=http://127.0.0.1:9999"), "{text}");
        assert!(text.contains("LOCAL_LLM_MODEL=qwen-test"), "{text}");
    }

    #[test]
    fn bootstrap_fast_path_adopts_existing_whisper_without_installing() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("fastpath");
        // A usable whisper (absolute path to an existing file) stands in for a machine
        // that already has one; the fast path must adopt it and never start pip.
        let fake = guard.dir.join("whisper-bin");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap(); }
        std::env::set_var("WHISPER_COMMAND", &fake);
        // Any pre-existing choices must survive the bootstrap.
        std::fs::write(guard.state_file(), "HIGHLIGHT_STRATEGY=semantic-local\n").unwrap();

        let app = tauri::test::mock_app();
        let state = BootstrapState(Arc::new(Mutex::new(None)));
        let resolved = run_whisper_bootstrap(app.handle(), &state).expect("fast path must succeed");
        assert_eq!(resolved, fake.to_string_lossy(), "must adopt the existing whisper");

        let progress = state.0.lock().unwrap().clone().expect("progress reported");
        assert!(progress.done && progress.error.is_none(), "{progress:?}");
        let text = std::fs::read_to_string(guard.state_file()).unwrap();
        assert!(text.contains("HIGHLIGHT_STRATEGY=semantic-local"), "{text}");
        assert!(text.contains("WHISPER_COMMAND="), "{text}");
    }

    #[test]
    fn download_and_verify_accepts_good_checksum_and_rejects_bad_one() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("download");
        // file:// URL keeps the test offline: same curl/sha path as the real 2.1 GB download.
        let source = guard.dir.join("weights.bin");
        std::fs::write(&source, b"local-first-llm-model-bytes").unwrap();
        let sha_out = Command::new("sha256sum").arg(&source).output().expect("sha256sum");
        let sha = String::from_utf8_lossy(&sha_out.stdout).split_whitespace().next().unwrap().to_string();
        let url = format!("file://{}", source.display());
        let target = guard.dir.join("model.gguf");
        let state = ModelDownloadState(Arc::new(Mutex::new(None)));

        let ok = download_and_verify(&state, &url, &sha, &target);
        assert!(ok.is_ok(), "expected ok, got {ok:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"local-first-llm-model-bytes");
        assert!(!guard.dir.join("model.gguf.part").exists(), "partial file must be gone");
        let progress = state.0.lock().unwrap().clone().unwrap();
        assert!(progress.done && progress.phase == "ready", "{progress:?}");

        // Corrupted download: wrong checksum must fail loudly, leave nothing behind.
        std::fs::remove_file(&target).unwrap();
        let bad = download_and_verify(&state, &url, &"0".repeat(64), &target);
        let err = bad.expect_err("checksum mismatch must fail");
        assert!(err.contains("MODEL_CHECKSUM_MISMATCH"), "{err}");
        assert!(!target.exists(), "failed download must not leave a target file");
        assert!(!guard.dir.join("model.gguf.part").exists(), "failed download must clean the partial");
    }

    #[test]
    fn download_and_verify_removes_stale_partial_before_starting() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("stale-part");
        let source = guard.dir.join("weights.bin");
        std::fs::write(&source, b"local-first-llm-model-bytes").unwrap();
        let sha_out = Command::new("sha256sum").arg(&source).output().expect("sha256sum");
        let sha = String::from_utf8_lossy(&sha_out.stdout).split_whitespace().next().unwrap().to_string();
        let target = guard.dir.join("model.gguf");
        // Simulate a leftover .part from an interrupted download.
        let part = PathBuf::from(format!("{}.part", target.display()));
        std::fs::write(&part, b"stale-bytes").unwrap();
        let state = ModelDownloadState(Arc::new(Mutex::new(None)));
        let ok = download_and_verify(&state, &format!("file://{}", source.display()), &sha, &target);
        assert!(ok.is_ok(), "expected ok, got {ok:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"local-first-llm-model-bytes");
        assert!(!part.exists(), "stale .part must be cleaned up");
    }
}
