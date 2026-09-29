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
/// True for remote inputs (YouTube URLs) that must skip local file checks.
fn is_remote_input(input: &str) -> bool { input.starts_with("http://") || input.starts_with("https://") }

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
/// Python interpreter inside the app venv (yt-dlp + youtube-transcript-api live there).
fn venv_python() -> PathBuf {
    #[cfg(windows)] { whisper_venv_dir().join("Scripts").join("python.exe") }
    #[cfg(not(windows))] { whisper_venv_dir().join("bin").join("python3") }
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

type ProgressCell = Arc<Mutex<Option<BootstrapProgress>>>;
#[derive(Clone)]
struct ModelDownloadState(ProgressCell);
/// Separate managed state so Whisper-model downloads never collide with the
/// local-LLM download progress (Tauri manages state by type).
#[derive(Clone)]
struct WhisperDlState(ProgressCell);
const LLM_MODEL_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf";
const LLM_MODEL_SHA256: &str = "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d";
fn llm_model_file(model: &str) -> PathBuf { state_dir().join("models/llama").join(format!("{model}.gguf")) }
fn set_download(cell: &ProgressCell, phase: &str, message: &str, done: bool, error: Option<String>) {
    if let Ok(mut guard) = cell.lock() { let _ = guard.insert(BootstrapProgress { phase: phase.into(), message: message.into(), done, error }); }
}
/// Downloads the local-LLM weights on demand (2.1 GB) and verifies the SHA-256 before use.
/// Fetch `url` into `target`, verify its SHA-256 against `expected_sha`, then rename the
/// partial file into place. Leftover partials are removed on any failure so a retry
/// starts clean. Kept separate from run_model_download so tests can drive it with
/// file:// URLs instead of a 2.1 GB network download.
fn download_and_verify(cell: &ProgressCell, url: &str, expected_sha: &str, target: &Path) -> Result<String, String> {
    if target.is_file() {
        set_download(cell, "ready", "Model already downloaded.", true, None);
        return Ok(target.to_string_lossy().into_owned());
    }
    if let Some(parent) = target.parent() { std::fs::create_dir_all(parent).map_err(|e| format!("MODEL_DIR:{e}"))?; }
    let part = PathBuf::from(format!("{}.part", target.display()));
    // A leftover .part means a previous download was killed mid-flight; start clean so
    // we never resume into bytes the checksum would then reject.
    if part.is_file() { let _ = std::fs::remove_file(&part); }
    set_download(cell, "download", "Downloading model…", false, None);
    let curl = Command::new("curl").args(["-L", "--fail", "--retry", "3", "-o"]).arg(&part).arg(url).output().map_err(|e| format!("CURL_SPAWN:{e}"))?;
    if !curl.status.success() { let _ = std::fs::remove_file(&part); return Err(format!("MODEL_DOWNLOAD_FAILED:{}", String::from_utf8_lossy(&curl.stderr).trim())); }
    set_download(cell, "verify", "Verifying checksum…", false, None);
    let digest = Command::new("sha256sum").arg(&part).output().map_err(|e| format!("SHA256_SPAWN:{e}"))?;
    let actual = String::from_utf8_lossy(&digest.stdout).split_whitespace().next().unwrap_or("").to_string();
    if actual != expected_sha { let _ = std::fs::remove_file(&part); return Err(format!("MODEL_CHECKSUM_MISMATCH:{actual}")); }
    std::fs::rename(&part, target).map_err(|e| format!("MODEL_RENAME:{e}"))?;
    set_download(cell, "ready", "Model ready.", true, None);
    Ok(target.to_string_lossy().into_owned())
}
fn run_model_download(state: &ModelDownloadState, model: &str) -> Result<String, String> {
    download_and_verify(&state.0, LLM_MODEL_URL, LLM_MODEL_SHA256, &llm_model_file(model))
}
#[tauri::command]
fn download_llm_model(state: tauri::State<'_, ModelDownloadState>) -> Result<(), String> {
    if let Ok(guard) = state.0.lock() { if let Some(progress) = guard.as_ref() { if !progress.done { return Ok(()); } } }
    let model = runtime_env().get("LOCAL_LLM_MODEL").cloned().filter(|m| !m.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    set_download(&state.0, "starting", "Preparing download…", false, None);
    let shared = state.inner().clone();
    thread::spawn(move || { if let Err(error) = run_model_download(&shared, &model) { set_download(&shared.0, "failed", "Model download failed.", true, Some(error)); } });
    Ok(())
}
#[tauri::command]
fn model_download_status(state: tauri::State<'_, ModelDownloadState>, app: tauri::AppHandle) -> serde_json::Value {
    let model = runtime_env().get("LOCAL_LLM_MODEL").cloned().filter(|m| !m.is_empty()).unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    let progress = state.0.lock().ok().and_then(|guard| guard.clone());
    let present = local_llm_model_path(&app, &model).is_some();
    serde_json::json!({ "progress": progress, "modelPresent": present, "model": model })
}

// --- Whisper model downloads: tiny/base ship inside the installer; "small"
// (~460 MB) is fetched on demand into the whisper CLI cache (~/.cache/whisper)
// using the same URL the openai-whisper CLI itself downloads from, so the file
// is cache-compatible and later runs need no re-download.
const WHISPER_SMALL_URL: &str = "https://openaipublic.azureedge.net/main/whisper/models/9ecf779972d90ba49c06d968637d720dd632c55bbf19d441fb42bf17a411e794/small.pt";
const WHISPER_SMALL_SHA256: &str = "9ecf779972d90ba49c06d968637d720dd632c55bbf19d441fb42bf17a411e794";
fn whisper_cache_file(model: &str) -> PathBuf {
    std::env::var("HOME").map(|h| Path::new(&h).join(".cache/whisper").join(format!("{model}.pt"))).unwrap_or_else(|_| std::env::temp_dir().join(format!("{model}.pt")))
}
#[tauri::command]
fn download_whisper_model(state: tauri::State<'_, WhisperDlState>, model: String) -> Result<(), String> {
    if model != "small" { return Err("WHISPER_DOWNLOAD_UNSUPPORTED: tiny and base ship inside the installer".into()); }
    if license_tier() != "pro" { return Err("PRO_REQUIRED: larger Whisper models need a Pro license".into()); }
    if let Ok(guard) = state.0.lock() { if let Some(progress) = guard.as_ref() { if !progress.done { return Ok(()); } } }
    set_download(&state.0, "starting", "Preparing download…", false, None);
    let shared = state.inner().0.clone();
    thread::spawn(move || {
        let dest = whisper_cache_file("small");
        if let Err(error) = download_and_verify(&shared, WHISPER_SMALL_URL, WHISPER_SMALL_SHA256, &dest) { set_download(&shared, "failed", "Whisper model download failed.", true, Some(error)); }
    });
    Ok(())
}
#[tauri::command]
fn whisper_model_download_status(state: tauri::State<'_, WhisperDlState>) -> serde_json::Value {
    let progress = state.0.lock().ok().and_then(|guard| guard.clone());
    let present = whisper_cache_file("small").is_file();
    serde_json::json!({ "progress": progress, "modelPresent": present, "model": "small" })
}

// --- YouTube "own channel" mode (Option C) -------------------------------------
// Customer connects their own Google account once (OAuth loopback flow in the
// system browser); the app then lists their uploads and pulls captions through
// the official API — legitimate for owned content, no scraping.
const YT_OAUTH_PORT: u16 = 14871;
const YT_SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct YtTokens { access_token: String, refresh_token: String, expires_at: u64 }
fn yt_tokens_file() -> PathBuf { state_dir().join("youtube-tokens.json") }
fn yt_client_creds() -> Result<(String, String), String> {
    // Developer ships credentials via build env; a state-dir file overrides for
    // local testing so customers never see either value.
    let file = state_dir().join("youtube-client.json");
    if let Ok(text) = std::fs::read_to_string(&file) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let id = v["clientId"].as_str().unwrap_or("").to_string();
            let secret = v["clientSecret"].as_str().unwrap_or("").to_string();
            if !id.is_empty() && !secret.is_empty() { return Ok((id, secret)); }
        }
    }
    let id = option_env!("GOOGLE_CLIENT_ID").unwrap_or("").to_string();
    let secret = option_env!("GOOGLE_CLIENT_SECRET").unwrap_or("").to_string();
    if id.is_empty() || secret.is_empty() { return Err("YT_CLIENT_MISSING: create a Google OAuth Desktop client and save {\"clientId\",\"clientSecret\"} to youtube-client.json in the app state dir".into()); }
    Ok((id, secret))
}
fn yt_load_tokens() -> Option<YtTokens> { serde_json::from_str(&std::fs::read_to_string(yt_tokens_file()).ok()?).ok() }
fn yt_save_tokens(tokens: &YtTokens) { let _ = std::fs::create_dir_all(state_dir()); let _ = std::fs::write(yt_tokens_file(), serde_json::to_string(tokens).unwrap_or_default()); }
fn b64url_nopad(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32; let b1 = *chunk.get(1).unwrap_or(&0) as u32; let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        if chunk.len() > 1 { out.push(T[(n >> 12) as usize & 63] as char); }
        if chunk.len() > 2 { out.push(T[(n >> 6) as usize & 63] as char); }
        if chunk.len() > 2 { out.push(T[n as usize & 63] as char); }
    }
    out
}
fn yt_token_request(form: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "-X", "POST", "https://oauth2.googleapis.com/token"]);
    for (k, v) in form { cmd.arg("--data-urlencode").arg(format!("{k}={v}")); }
    let out = cmd.output().map_err(|e| format!("CURL_SPAWN:{e}"))?;
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("TOKEN_JSON:{e}"))?;
    if value.get("error").is_some() { return Err(format!("TOKEN_ERROR:{}", value["error"].as_str().unwrap_or("?"))); }
    Ok(value)
}
fn yt_refresh_if_needed(tokens: &mut YtTokens) -> Result<(), String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if tokens.expires_at > now + 120 { return Ok(()); }
    let (client_id, client_secret) = yt_client_creds()?;
    let value = yt_token_request(&[("client_id", client_id.as_str()), ("client_secret", client_secret.as_str()), ("refresh_token", tokens.refresh_token.as_str()), ("grant_type", "refresh_token")])?;
    tokens.access_token = value["access_token"].as_str().unwrap_or("").to_string();
    let expires = value["expires_in"].as_u64().unwrap_or(3600);
    tokens.expires_at = now + expires.saturating_sub(60);
    yt_save_tokens(tokens);
    Ok(())
}
fn yt_bearer() -> Result<String, String> {
    let mut tokens = yt_load_tokens().ok_or("YT_NOT_CONNECTED: connect your YouTube channel first")?;
    yt_refresh_if_needed(&mut tokens)?;
    Ok(tokens.access_token)
}
fn yt_api_get(path_with_query: &str) -> Result<serde_json::Value, String> {
    let bearer = yt_bearer()?;
    let out = Command::new("curl").args(["-s", "-H", &format!("Authorization: Bearer {bearer}")]).arg(format!("https://www.googleapis.com{path_with_query}")).output().map_err(|e| format!("YT_API_SPAWN:{e}"))?;
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("YT_API_JSON:{e} — {}", String::from_utf8_lossy(&out.stdout).chars().take(120).collect::<String>()))?;
    if let Some(err) = value.get("error") { return Err(format!("YT_API:{}", err["message"].as_str().unwrap_or("?"))); }
    Ok(value)
}
#[tauri::command]
fn youtube_oauth_status() -> serde_json::Value {
    match yt_load_tokens() {
        Some(mut tokens) => {
            let connected = yt_refresh_if_needed(&mut tokens).is_ok();
            serde_json::json!({ "connected": connected, "expiresAt": tokens.expires_at })
        }
        None => serde_json::json!({ "connected": false }),
    }
}
#[tauri::command]
fn youtube_oauth_disconnect() -> Result<(), String> { std::fs::remove_file(yt_tokens_file()).unwrap_or(()); Ok(()) }
#[tauri::command]
fn youtube_oauth_start() -> Result<String, String> {
    let (client_id, client_secret) = yt_client_creds()?;
    // Deterministic PKCE verifier; challenge hashed with the same sha256sum CLI
    // the rest of this file already shells out to.
    let verifier = format!("{}{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let digest = Command::new("sha256sum").arg(&verifier).output().map_err(|e| format!("SHA_SPAWN:{e}"))?;
    let raw = String::from_utf8_lossy(&digest.stdout).split_whitespace().next().unwrap_or("").to_string();
    let challenge = b64url_nopad(&raw.as_bytes());
    let redirect = format!("http://127.0.0.1:{YT_OAUTH_PORT}/callback");
    let scope_enc = YT_SCOPE.replace(':', "%3A").replace('/', "%2F");
    let consent_url = format!("https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}&redirect_uri={redirect}&response_type=code&scope={scope_enc}&access_type=offline&prompt=consent&include_granted_scopes=true&code_challenge={challenge}&code_challenge_method=S256");
    // The loopback listener must exist BEFORE the browser navigates.
    let listener = std::net::TcpListener::bind(("127.0.0.1", YT_OAUTH_PORT)).map_err(|e| format!("YT_PORT_BUSY:{e} — an OAuth callback is already waiting"))?;
    let client_secret = client_secret.clone();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = String::new();
            use std::io::{Read, Write};
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            let _ = stream.read_to_string(&mut request);
            let code = request.split_whitespace().nth(1)
                .and_then(|path| path.split("code=").nth(1))
                .map(|rest| rest.split('&').next().unwrap_or("").to_string())
                .unwrap_or_default();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body style='font-family:sans-serif;background:#11100e;color:#f4efe4'><h2>&#9989; YouTube connected</h2><p>B&#7841;n c&oacute; th&#7875; &#273;&oacute;ng tab n&agrave;y v&agrave; quay l&#7841;i app.</p></body></html>");
            let _ = stream.flush();
            drop(stream);
            if code.is_empty() { return; }
            if let Ok((client_id, client_secret)) = yt_client_creds() {
                if let Ok(value) = yt_token_request(&[
                    ("client_id", client_id.as_str()), ("client_secret", client_secret.as_str()),
                    ("code", code.as_str()), ("grant_type", "authorization_code"), ("redirect_uri", redirect.as_str()),
                    ("code_verifier", verifier.as_str()),
                ]) {
                    let access = value["access_token"].as_str().unwrap_or("").to_string();
                    let refresh = value["refresh_token"].as_str().unwrap_or("").to_string();
                    let expires = value["expires_in"].as_u64().unwrap_or(3600);
                    if !access.is_empty() && !refresh.is_empty() {
                        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        yt_save_tokens(&YtTokens { access_token: access, refresh_token: refresh, expires_at: now + expires.saturating_sub(60) });
                    }
                }
            }
        }
    });
    // Open the system browser for the consent screen.
    let opened = if cfg!(target_os = "linux") { Command::new("xdg-open").arg(&consent_url).spawn().is_ok() }
        else if cfg!(target_os = "macos") { Command::new("open").arg(&consent_url).spawn().is_ok() }
        else { Command::new("cmd").args(["/C", "start"]).arg(&consent_url).spawn().is_ok() };
    if !opened { return Ok(format!("OPEN_MANUALLY:{consent_url}")); }
    Ok(consent_url)
}
#[tauri::command]
fn youtube_channel_videos() -> Result<serde_json::Value, String> {
    let channel = yt_api_get("/youtube/v3/channels?part=snippet,contentDetails&mine=true")?;
    let title = channel["items"][0]["snippet"]["title"].as_str().unwrap_or("Kênh của tôi").to_string();
    let uploads = channel["items"][0]["contentDetails"]["relatedPlaylists"]["uploads"].as_str().unwrap_or("").to_string();
    if uploads.is_empty() { return Err("YT_NO_UPLOADS: kênh này chưa có video nào".into()); }
    let items = yt_api_get(&format!("/youtube/v3/playlistItems?part=snippet,contentDetails&maxResults=50&playlistId={uploads}"))?;
    let videos: Vec<serde_json::Value> = items["items"].as_array().unwrap_or(&vec![]).iter().map(|item| {
        serde_json::json!({
            "videoId": item["contentDetails"]["videoId"],
            "title": item["snippet"]["title"],
            "publishedAt": item["contentDetails"]["videoPublishedAt"].as_str().or(item["snippet"]["publishedAt"].as_str()).unwrap_or(""),
            "thumbnail": item["snippet"]["thumbnails"]["medium"]["url"].as_str().unwrap_or(""),
        })
    }).collect();
    Ok(serde_json::json!({ "channel": title, "videos": videos }))
}
/// Parses SRT ("00:00:01,000 --> 00:00:04,000") into {start,end,text} segments.
fn parse_srt_segments(text: &str) -> Vec<serde_json::Value> {
    fn secs(stamp: &str) -> f64 {
        let parts: Vec<f64> = stamp.trim().split(':').filter_map(|p| p.replace(",", ".").parse::<f64>().ok()).collect();
        match parts.as_slice() { [h, m, s] => h * 3600.0 + m * 60.0 + s, [m, s] => m * 60.0 + s, [s] => *s, _ => 0.0 }
    }
    let mut segments = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < 2 { continue; }
        if let Some(arrow) = lines.iter().find(|l| l.contains("-->")) {
            let mut halves = arrow.split("-->");
            let start = secs(halves.next().unwrap_or(""));
            let end = secs(halves.next().unwrap_or("0"));
            let body: String = lines[lines.iter().position(|l| l.contains("-->")).unwrap() + 1..].join(" ").trim().to_string();
            if !body.is_empty() && end > start { segments.push(serde_json::json!({ "start": (start * 100.0).round() / 100.0, "end": (end * 100.0).round() / 100.0, "text": body })); }
        }
    }
    segments
}
/// Downloads captions for an OWN video through the official API and ingests the
/// transcript into the local RAG store via the sidecar.
#[tauri::command]
fn youtube_ingest_captions(app: tauri::AppHandle, video_id: String) -> Result<serde_json::Value, String> {
    let bearer = yt_bearer()?;
    let caps = {
        let out = Command::new("curl").args(["-s", "-H", &format!("Authorization: Bearer {bearer}")]).arg(format!("https://www.googleapis.com/youtube/v3/captions?part=snippet&videoId={video_id}")).output().map_err(|e| format!("YT_API_SPAWN:{e}"))?;
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("YT_API_JSON:{e}"))?;
        if let Some(err) = value.get("error") { return Err(format!("YT_CAPTIONS:{}", err["message"].as_str().unwrap_or("?"))); }
        value["items"].as_array().cloned().unwrap_or_default()
    };
    if caps.is_empty() { return Err("YT_NO_CAPTIONS: video này chưa có phụ đề nào trên YouTube (hãy bật phụ đề tự động hoặc tải lên phụ đề trước)".into()); }
    let cap_id = caps[0]["id"].as_str().unwrap_or("").to_string();
    let srt = {
        let out = Command::new("curl").args(["-s", "-L", "-H", &format!("Authorization: Bearer {bearer}")]).arg(format!("https://www.googleapis.com/youtube/v3/captions/{cap_id}?tfmt=srt")).output().map_err(|e| format!("CAP_SPAWN:{e}"))?;
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let segments = parse_srt_segments(&srt);
    if segments.is_empty() { return Err("CAP_PARSE_EMPTY: phụ đề tải về rỗng hoặc định dạng lạ".into()); }
    let base = ensure_rag_server(&RagState::default(), &app)?;
    let body = serde_json::json!({ "videoUrl": format!("https://www.youtube.com/watch?v={video_id}"), "segments": segments });
    let body_str = body.to_string();
    let out = Command::new("curl").args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-X", "POST", "-H", "content-type: application/json", "-d", &body_str, &format!("{base}/api/rag/ingest")]).output().map_err(|e| format!("INGEST_SPAWN:{e}"))?;
    if !String::from_utf8_lossy(&out.stdout).trim().starts_with('2') { return Err(format!("INGEST_HTTP:{}", String::from_utf8_lossy(&out.stdout).trim())); }
    Ok(serde_json::json!({ "videoId": video_id, "segments": segments.len() }))
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
            let mut cmd = Command::new(node_bin(&app));
            if is_remote_input(&input.to_string_lossy()) {
                // YouTube URL: text-only pipeline (captions via crawler), no Whisper chunks.
                let python = venv_python();
                if !python.is_file() { return Err("PYTHON_VENV_MISSING: run whisper bootstrap first".into()); }
                cmd.args(NODE_ARGS).arg(helper_script(&app, "analyze-youtube.ts")?)
                    .arg(&input).arg(format!("{duration:.3}")).arg(&job_id).arg(jobs_path())
                    .env("YT_PYTHON", &python).env("YT_CRAWLER", helper_script(&app, "yt-crawler.py")?);
            } else {
                let script = helper_script(&app, "analyze-video.ts")?;
                cmd.args(NODE_ARGS).arg(script).arg(&input).arg(input.to_string_lossy().as_ref()).arg(format!("{duration:.3}")).arg(format!("{range_start:.3}")).arg(format!("{range_end:.3}")).arg(&job_id).arg(jobs_path());
            }
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
        if !is_remote_input(input) && !path.is_file() {
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
    if !is_remote_input(input) && !path.is_file() { let _ = run_status_script(&app, &[id.clone(), "--fail".to_string(), "input file missing".to_string()]); return Err("INPUT_NOT_FILE".into()); }
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

/// Extracts a video id from a youtube.com/youtu.be link for validation only —
/// the original (case-preserved) URL is what reaches yt-dlp afterwards.
fn youtube_video_id(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let (host, tail) = rest.split_once(|c| c == '/' || c == '?')?;
    let id = if host == "youtu.be" {
        tail.split(['/', '?']).next().unwrap_or("")
    } else if host == "youtube.com" || host.ends_with(".youtube.com") {
        let (path, query) = match tail.split_once('?') { Some((p, q)) => (p, q), None => (tail, "") };
        if let Some(v) = query.split('&').find_map(|p| p.strip_prefix("v=")) { v }
        else if let Some(rest) = path.strip_prefix("embed/").or_else(|| path.strip_prefix("shorts/")) { rest.split('/').next().unwrap_or("") }
        else { "" }
    } else { return None; };
    if id.len() >= 6 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { Some(id.to_string()) } else { None }
}

/// Fetches (title, duration) for a YouTube URL — one metadata-only yt-dlp call (~1s).
fn yt_video_meta(app: &tauri::AppHandle, url: &str) -> Result<(String, f64), String> {
    let python = venv_python();
    if !python.is_file() { return Err("PYTHON_VENV_MISSING: run whisper bootstrap first".into()); }
    let mut cmd = Command::new(&python);
    cmd.args(["-m", "yt_dlp", "--skip-download", "--no-playlist", "--print", "%(title)s\t%(duration)s"]).arg(url);
    for (key, value) in helper_env(app, &[]) { cmd.env(key, value); }
    let output = cmd.output().map_err(|e| format!("YT_META_SPAWN:{e}"))?;
    if !output.status.success() { return Err(format!("YT_META:{}", String::from_utf8_lossy(&output.stderr).trim())); }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or("");
    let (title, duration_text) = line.rsplit_once('\t').ok_or("YT_META_PARSE")?;
    let duration: f64 = duration_text.trim().parse().map_err(|_| "YT_NO_DURATION".to_string())?;
    if !duration.is_finite() || duration <= 0.0 { return Err("YT_NO_DURATION".into()); }
    Ok((title.to_string(), duration))
}

/// Resolves fresh CDN stream URLs at export time: YouTube URLs expire (~6h), so
/// they must never be cached between analysis and render. YouTube is DASH-only
/// nowadays, so this usually returns [videoUrl, audioUrl] for ffmpeg's two inputs.
fn resolve_stream_url(app: &tauri::AppHandle, url: &str) -> Result<Vec<String>, String> {
    let python = venv_python();
    if !python.is_file() { return Err("PYTHON_VENV_MISSING: run whisper bootstrap first".into()); }
    let mut cmd = Command::new(&python);
    cmd.args(["-m", "yt_dlp", "--no-playlist", "-g", "-f", "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b[ext=mp4]/b"]).arg(url);
    for (key, value) in helper_env(app, &[]) { cmd.env(key, value); }
    let output = cmd.output().map_err(|e| format!("YT_STREAM_SPAWN:{e}"))?;
    if !output.status.success() { return Err(format!("YT_STREAM_RESOLVE:{}", String::from_utf8_lossy(&output.stderr).trim())); }
    let urls: Vec<String> = String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string).collect();
    if urls.is_empty() { return Err("YT_STREAM_EMPTY: yt-dlp returned no stream URL for this video".into()); }
    Ok(urls)
}

/// Starts a text-only analysis job from a YouTube URL: metadata (~1s) → init →
/// analyze-youtube.ts worker (captions, no Whisper, no download).
#[tauri::command]
fn start_youtube_analysis(app: tauri::AppHandle, jobs: tauri::State<'_, Jobs>, url: String) -> Result<serde_json::Value, String> {
    let url = url.trim().to_string();
    if youtube_video_id(&url).is_none() { return Err("INVALID_YOUTUBE_URL".into()); }
    let (title, duration) = yt_video_meta(&app, &url)?;
    let id = format!("analysis-{}", SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "CLOCK_ERROR")?.as_nanos());
    let init_script = helper_script(&app, "analysis-init.ts")?;
    let mut init = Command::new(node_bin(&app));
    init.args(NODE_ARGS).arg(init_script).arg(jobs_path()).arg(&id).arg(&url).arg(format!("{duration:.3}")).arg("0").arg(format!("{duration:.3}"));
    for (key, value) in helper_env(&app, &[]) { init.env(key, value); }
    let init = init.output().map_err(|e| format!("STORE_INIT:{e}"))?;
    if !init.status.success() { return Err(String::from_utf8_lossy(&init.stderr).trim().to_owned()); }
    spawn_analysis_worker(app.clone(), jobs.inner().clone(), id.clone(), PathBuf::from(&url), duration, 0.0, duration);
    Ok(serde_json::json!({ "id": id, "duration": duration, "title": title }))
}

// --- YouTube preview shim -------------------------------------------------------
// The packaged webview origin is tauri://localhost; WebKitGTK sends no Referer
// for that scheme and YouTube rejects such embeds with error 153
// (EMBEDDER_IDENTITY_MISSING_REFERRER). A loopback page around the player
// gives it a normal http://localhost Referer, which YouTube accepts.
const YT_PREVIEW_PORT: u16 = 14872;

/// Parses the shim's query into (videoId, start, end); anything else is rejected
/// so the served page is only ever built from validated pieces.
fn preview_query(query: &str) -> Option<(String, u32, u32)> {
    let mut id = String::new();
    let mut start: Option<u32> = None;
    let mut end: Option<u32> = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "id" => id = value.to_string(),
            "start" => start = value.parse().ok(),
            "end" => end = value.parse().ok(),
            _ => {}
        }
    }
    if id.len() != 11 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { return None; }
    let (start, end) = (start?, end?);
    if start >= end { return None; }
    Some((id, start, end))
}

fn preview_page(video_id: &str, start: u32, end: u32) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0;height:100%;background:#000}}iframe{{display:block;width:100%;height:100%;border:0}}</style></head><body>\
         <iframe src=\"https://www.youtube.com/embed/{video_id}?start={start}&end={end}&rel=0\" allow=\"accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture\" allowfullscreen></iframe>\
         </body></html>"
    )
}

fn preview_serve(listener: std::net::TcpListener) {
    use std::io::{Read, Write};
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let mut request: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        request.extend_from_slice(&chunk[..n]);
                        if request.len() > 8192 || request.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                    }
                    Err(_) => break,
                }
            }
            let text = String::from_utf8_lossy(&request);
            let target = text.split_whitespace().nth(1).unwrap_or("");
            let (path, query) = target.split_once('?').unwrap_or((target, ""));
            let (status, body) = match path {
                "/health" => ("200 OK", "yt-preview".to_string()),
                "/preview" => match preview_query(query) {
                    Some((id, start, end)) => ("200 OK", preview_page(&id, start, end)),
                    None => ("400 Bad Request", "bad preview request".to_string()),
                },
                _ => ("404 Not Found", "not found".to_string()),
            };
            let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            let _ = stream.flush();
        });
    }
}

/// Returns the loopback base URL for the YouTube preview iframe, starting the
/// shim on first use (reuses an instance already serving after a reload).
#[tauri::command]
fn preview_origin() -> Result<String, String> {
    let base = format!("http://localhost:{YT_PREVIEW_PORT}");
    if let Ok(mut probe) = std::net::TcpStream::connect(("localhost", YT_PREVIEW_PORT)) {
        use std::io::{Read, Write};
        let _ = probe.set_read_timeout(Some(std::time::Duration::from_secs(2)));
        let _ = probe.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        let mut reply = String::new();
        let _ = probe.read_to_string(&mut reply);
        if reply.starts_with("HTTP/1.1 200") && reply.contains("yt-preview") { return Ok(base); }
        return Err(format!("PREVIEW_PORT_BUSY:{YT_PREVIEW_PORT}"));
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", YT_PREVIEW_PORT)).map_err(|e| format!("PREVIEW_PORT_BUSY:{e}"))?;
    thread::spawn(move || preview_serve(listener));
    Ok(base)
}

#[tauri::command]
fn render_video(app: tauri::AppHandle, input: String, output: String, start: f64, duration: f64, aspect_ratio: Option<String>) -> Result<String, String> {
    if !start.is_finite() || start < 0.0 || !duration.is_finite() || duration <= 0.0 { return Err("INVALID_RENDER_REQUEST".into()); }
    let output_path = Path::new(&output).canonicalize().unwrap_or_else(|_| Path::new(&output).to_path_buf());
    // YouTube inputs skip the local file checks and resolve fresh CDN URLs;
    // -ss/-t before each -i on HTTP becomes a range request (only the section
    // is fetched), and a DASH pair needs the same seek on both inputs.
    let media: Vec<String> = if is_remote_input(&input) {
        resolve_stream_url(&app, &input)?
    } else {
        let input_path = Path::new(&input).canonicalize().map_err(|e| format!("INPUT_PATH:{e}"))?;
        if input_path == output_path { return Err("OUTPUT_MUST_DIFFER_FROM_INPUT".into()); }
        if !input_path.is_file() { return Err("INPUT_NOT_FILE".into()); }
        vec![input_path.to_string_lossy().into_owned()]
    };
    let ratio = aspect_ratio.unwrap_or_else(|| "9:16".into());
    let vf = match ratio.as_str() { "1:1" => "crop=ih:ih:(iw-ih)/2:0,scale=1080:1080", "16:9" => "crop=ih*16/9:ih:(iw-ih*16/9)/2:0,scale=1920:1080", _ => "crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=1080:1920" };
    let parent = output_path.parent().ok_or("OUTPUT_PARENT_MISSING")?; std::fs::create_dir_all(parent).map_err(|e| format!("OUTPUT_DIR:{e}"))?;
    let mut cmd = Command::new(ffmpeg_bin(&app));
    cmd.args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error"]);
    for source in &media {
        cmd.args(["-ss"]).arg(format!("{start:.3}")).args(["-t"]).arg(format!("{duration:.3}")).args(["-i"]).arg(source);
    }
    cmd.args(["-vf", vf, "-c:v", "libx264", "-preset", "fast", "-crf", "22", "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"]);
    if media.len() > 1 { cmd.args(["-map", "0:v:0", "-map", "1:a:0"]); }
    let status = cmd.arg(&output_path).status().map_err(|e| format!("FFMPEG_SPAWN:{e}"))?;
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
    let venv_python = venv_python();
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
    // WebKitGTK on some Linux GPUs (Intel iGPU laptops) fails EGL/DMABUF init
    // and renders an empty window — force software-safe compositing.
    if cfg!(target_os = "linux") {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let supervisor_jobs = jobs.clone();
    let llama_state: LlamaState = Arc::new(Mutex::new(None));
    let rag_state: RagState = RagState::default();
    let bootstrap_state = BootstrapState(Arc::new(Mutex::new(None)));
    let download_state = ModelDownloadState(Arc::new(Mutex::new(None)));
    let whisper_dl_state = WhisperDlState(Arc::new(Mutex::new(None)));
    let app = tauri::Builder::default()
        .manage(jobs)
        .manage(llama_state.clone())
        .manage(rag_state.clone())
        .manage(bootstrap_state)
        .manage(download_state)
        .manage(whisper_dl_state.clone())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .setup(move |app| { resume_queued_jobs(app.handle().clone(), supervisor_jobs.clone()); Ok(()) })
        .invoke_handler(tauri::generate_handler![app_info, license_status, set_runtime_model, set_highlight_strategy, highlight_strategy_status, start_local_llm, stop_local_llm, rag_server_url, bootstrap_whisper, whisper_bootstrap_status, download_llm_model, model_download_status, runtime_status, storage_status, clear_analysis_cache, preflight_analyze, start_analysis, start_youtube_analysis, preview_origin, analysis_status, list_analysis_jobs, retry_analysis, abandon_analysis, stop_analysis, probe_video, render_video, crawl_playlist, download_whisper_model, whisper_model_download_status, youtube_oauth_start, youtube_oauth_status, youtube_oauth_disconnect, youtube_channel_videos, youtube_ingest_captions])
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

        let ok = download_and_verify(&state.0, &url, &sha, &target);
        assert!(ok.is_ok(), "expected ok, got {ok:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"local-first-llm-model-bytes");
        assert!(!guard.dir.join("model.gguf.part").exists(), "partial file must be gone");
        let progress = state.0.lock().unwrap().clone().unwrap();
        assert!(progress.done && progress.phase == "ready", "{progress:?}");

        // Corrupted download: wrong checksum must fail loudly, leave nothing behind.
        std::fs::remove_file(&target).unwrap();
        let bad = download_and_verify(&state.0, &url, &"0".repeat(64), &target);
        let err = bad.expect_err("checksum mismatch must fail");
        assert!(err.contains("MODEL_CHECKSUM_MISMATCH"), "{err}");
        assert!(!target.exists(), "failed download must not leave a target file");
        assert!(!guard.dir.join("model.gguf.part").exists(), "failed download must clean the partial");
    }

    #[test]
    fn preview_query_accepts_valid_and_rejects_bad_requests() {
        // The shim builds an HTML page from this query — malformed or hostile
        // input must never reach the page.
        assert_eq!(preview_query("id=9bZkp7q19f0&start=9&end=26"), Some(("9bZkp7q19f0".to_string(), 9, 26)));
        assert_eq!(preview_query("id=short&start=9&end=26"), None, "video id must be 11 chars");
        assert_eq!(preview_query("id=<script>x&start=9&end=26"), None, "id charset is whitelisted");
        assert_eq!(preview_query("id=9bZkp7q19f0&start=26&end=9"), None, "start must precede end");
        assert_eq!(preview_query("id=9bZkp7q19f0&start=9"), None, "end is required");
        assert_eq!(preview_query(""), None);
        let page = preview_page("9bZkp7q19f0", 9, 26);
        assert!(page.contains("youtube.com/embed/9bZkp7q19f0?start=9&end=26"), "{page}");
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
        let ok = download_and_verify(&state.0, &format!("file://{}", source.display()), &sha, &target);
        assert!(ok.is_ok(), "expected ok, got {ok:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"local-first-llm-model-bytes");
        assert!(!part.exists(), "stale .part must be cleaned up");
    }
}
