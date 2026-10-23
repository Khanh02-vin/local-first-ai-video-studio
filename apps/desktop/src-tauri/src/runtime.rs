// The runtime branch: the local model runtimes the app drives itself.
//
// llama-server, the RAG sidecar, the Whisper bootstrap and the model downloads all
// manage a detached process plus a Tauri-managed progress cell, so they live together
// instead of being scattered through the shell.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use tauri::Manager;

use crate::paths::{
    command_ready, ensure_bundled_whisper_model, ffmpeg_bin, ffprobe_bin, free_loopback_port,
    node_bin, python_binary, runtime_env, data_dir, cache_dir, session_token, tool,
    whisper_command, whisper_usable, whisper_venv_bin, whisper_venv_dir,
    write_runtime_settings,
};
use crate::youtube::prepare_youtube_venv;
use crate::{license_tier, run_status_script};

pub(crate) type LlamaState = Arc<Mutex<Option<u32>>>; // pid of the detached llama-server, if running

/// Finds the platform's llama-server inside the app-owned state directory.
pub(crate) fn existing_llama(dir: PathBuf) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let candidate = dir.join(name);
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

pub(crate) fn llama_binary() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("LOCAL_LLM_BINARY") {
        if Path::new(&path).is_file() {
            return Ok(PathBuf::from(path));
        }
    }
    existing_llama(cache_dir().join("llama/runtime"))
        .ok_or_else(|| "LLAMA_RUNTIME_MISSING: install the local LLM runtime in Settings".into())
}

pub(crate) fn local_llm_model_path<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
    model: &str,
) -> Option<PathBuf> {
    if !["qwen2.5-3b-instruct", "qwen2.5-3b-instruct-q4_k_m"].contains(&model) {
        return None;
    }
    let candidate = llm_model_file("qwen2.5-3b-instruct-q4_k_m");
    candidate.is_file().then_some(candidate)
}

pub(crate) fn ensure_llama_server<R: tauri::Runtime>(
    state: &LlamaState,
    app: &tauri::AppHandle<R>,
    port: u16,
    api_key: Option<&str>,
) -> Result<(), String> {
    // If a llama-server is already answering on this port, reuse it. The key is sent
    // when we have one: a server started with --api-key refuses unlabelled requests.
    let health_url = format!("http://127.0.0.1:{port}/health");
    let mut health = tool("curl");
    health.args([
        "-s",
        "-f",
        "-o",
        "/dev/null",
        "-w",
        "%{http_code}",
    ]);
    if let Some(key) = api_key {
        health.args(["-H", &format!("Authorization: Bearer {key}")]);
    }
    if health
        .arg(&health_url)
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "200")
        .unwrap_or(false)
    {
        return Ok(());
    }
    let binary = llama_binary()?;
    let model = runtime_env()
        .get("LOCAL_LLM_MODEL")
        .filter(|m| ["qwen2.5-3b-instruct", "qwen2.5-3b-instruct-q4_k_m"].contains(&m.as_str()))
        .cloned()
        .unwrap_or_else(|| "qwen2.5-3b-instruct".into());
    let model_path = local_llm_model_path(app, &model)
        .ok_or_else(|| format!("LLAMA_MODEL_MISSING:{model}; install the model in Settings"))?;
    let mut cmd = tool(binary);
    cmd.arg("-m")
        .arg(&model_path)
        .arg("--port")
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        // One slot, 8192 tokens: highlight prompts render a 180s window as compact
        // word lines (~4-5k tokens); the old 4096/parallel-2 split left 2048 per
        // slot, so every real window overflowed and semantic-local silently fell
        // back to the heuristic. Analysis is sequential — a second slot buys nothing.
        .arg("--ctx-size")
        .arg("8192")
        .arg("--parallel")
        .arg("1")
        // --pooling mean: Qwen chat GGUFs default to pooling "none", which the
        // OpenAI-compatible /v1/embeddings endpoint rejects (400/501). Forcing
        // mean pooling lets the same server embed RAG segments and still chat.
        .arg("--embeddings")
        .arg("--pooling")
        .arg("mean");
    // A per-session key keeps drive-by web pages from using the model API over
    // loopback: without it, any page in any browser tab could POST /v1/chat/completions.
    if let Some(key) = api_key {
        cmd.arg("--api-key").arg(key);
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let spawned = cmd.spawn().map_err(|e| format!("LLAMA_SPAWN:{e}"))?;
    let pid = spawned.id();
    // Detach: drop our handle but keep the pid so we can stop it later.

    drop(spawned);
    let mut ready = false;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut probe = tool("curl");
        probe.args(["-s", "-f", "-o", "/dev/null", "-w", "%{http_code}"]);
        if let Some(key) = api_key {
            probe.args(["-H", &format!("Authorization: Bearer {key}")]);
        }
        if probe
            .arg(&health_url)
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "200")
            .unwrap_or(false)
        {
            ready = true;
            break;
        }
    }
    if !ready {
        #[cfg(unix)]
        let _ = tool("kill").args(["-TERM", &pid.to_string()]).status();
        return Err("LLAMA_SERVER_TIMEOUT".into());
    }
    if let Ok(mut guard) = state.lock() {
        let _ = guard.insert(pid);
    }
    Ok(())
}

/// Extracts the port of a loopback URL ("http://127.0.0.1:8123") — the URL host was
/// already validated to loopback by `valid_local_llm_url`.
fn loopback_port_of(url: &str) -> Option<u16> {
    url.rsplit(':').next()?.parse().ok()
}

#[tauri::command]
pub(crate) fn start_local_llm(
    app: tauri::AppHandle,
    state: tauri::State<'_, LlamaState>,
) -> Result<String, String> {
    let file = runtime_env();
    // Reuse a server that already answers on the configured endpoint — a leftover
    // managed instance from a crashed session, or an external server the user points
    // at. Nothing is rewritten in this branch.
    if let Some(url) = file
        .get("LOCAL_LLM_BASE_URL")
        .filter(|u| u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost"))
    {
        if let Some(port) = loopback_port_of(url) {
            let key = file.get("LOCAL_LLM_API_KEY").cloned();
            if ensure_llama_server(&state, &app, port, key.as_deref()).is_ok() {
                return Ok(url.clone());
            }
        }
    }
    // Spawn the bundled server on an ephemeral port with a per-session key, then
    // record the endpoint in runtime.env so the Node helpers and the RAG sidecar
    // reach the same server (helper_env forwards those entries to every worker).
    let port = free_loopback_port().ok_or("NO_FREE_PORT: cannot bind an ephemeral port")?;
    let api_key = session_token();
    ensure_llama_server(&state, &app, port, Some(&api_key))?;
    let url = format!("http://127.0.0.1:{port}");
    let mut settings = runtime_env();
    settings.insert("LOCAL_LLM_BASE_URL".into(), url.clone());
    settings.insert("LOCAL_LLM_API_KEY".into(), api_key);
    settings.insert("LOCAL_LLM_MANAGED".into(), "1".into());
    write_runtime_settings(&settings)?;
    Ok(url)
}

#[tauri::command]
pub(crate) fn stop_local_llm(state: tauri::State<'_, LlamaState>) -> Result<(), String> {
    if let Ok(mut guard) = state.lock() {
        if let Some(pid) = guard.take() {
            #[cfg(unix)]
            {
                let _ = nix_kill(pid);
            }
            #[cfg(not(unix))]
            {
                let _ = tool("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .status();
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn nix_kill(pid: u32) -> std::io::Result<()> {
    tool("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map(|_| ())
}

// Newtype: LlamaState is also Arc<Mutex<Option<u32>>>. Tauri keys managed state
// by the concrete type, so aliasing both would register the same type twice and
// panic ("already being managed"). This wrapper keeps them distinct.
#[derive(Clone, Default)]
pub(crate) struct RagState(pub(crate) Arc<Mutex<Option<u32>>>); // pid of the detached RAG sidecar, if running

/// The endpoint handed to the webview: base URL plus the per-session token every
/// sidecar request must carry.
#[derive(Clone, serde::Serialize)]
pub(crate) struct RagEndpoint {
    pub(crate) base: String,
    pub(crate) token: String,
}

/// Session record for a leftover sidecar: after a crash the app cannot know the
/// ephemeral port or the token any other way, so they are persisted (data dir,
/// 0600) and verified before reuse.
fn rag_session_file() -> PathBuf {
    data_dir().join("rag-session.json")
}

fn rag_load_session() -> Option<RagEndpoint> {
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rag_session_file()).ok()?).ok()?;
    let port = value["port"].as_u64()? as u16;
    let token = value["token"].as_str()?.to_string();
    if token.is_empty() {
        return None;
    }
    Some(RagEndpoint { base: format!("http://127.0.0.1:{port}"), token })
}

fn rag_save_session(endpoint: &RagEndpoint) {
    let port = endpoint.base.rsplit(':').next().and_then(|p| p.parse::<u16>().ok());
    let Some(port) = port else { return };
    let body = serde_json::json!({ "port": port, "token": endpoint.token });
    let _ = crate::paths::write_private_file(&rag_session_file(), body.to_string().as_bytes());
}

fn rag_clear_session() {
    let _ = std::fs::remove_file(rag_session_file());
}

/// Where the bundled adapter-node RAG server entry lives (web/build-node/index.js
/// bundled as the `build-node` resource). Falls back to the dev-tree build so a
/// `tauri dev` run without a packaged resource still works.
pub(crate) fn rag_entry(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let candidate = app
        .path()
        .resource_dir()
        .map_err(|e| format!("RESOURCE_DIR:{e}"))?
        .join("build-node/index.js");
    if candidate.is_file() {
        return Ok(candidate);
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/build-node/index.js");
    if dev.is_file() {
        return Ok(dev);
    }
    Err("RAG_SERVER_MISSING: web/build-node/index.js not found in resources or dev tree (run: npm run build:node)".into())
}

/// True when the RAG sidecar is answering on this port with this token.
pub(crate) fn rag_healthy(port: u16, token: &str) -> bool {
    let url = format!("http://127.0.0.1:{port}/api/rag/playlists");
    tool("curl")
        .args([
            "-s",
            "-f",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-H",
            &format!("Authorization: Bearer {token}"),
            &url,
        ])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "200")
        .unwrap_or(false)
}

/// Idempotently spawn the RAG sidecar (bundled node + build-node entry) on a free
/// loopback port and wait until it answers. Reuses a healthy leftover from the
/// session file (a crashed app cannot otherwise be reattached to its sidecar).
/// Returns the base URL plus the per-session token the static webview must send
/// on every request via plugin-http.
pub(crate) fn ensure_rag_server(
    state: &RagState,
    app: &tauri::AppHandle,
) -> Result<RagEndpoint, String> {
    // Reuse a leftover whose token we still hold and that still answers.
    if let Some(session) = rag_load_session() {
        if rag_healthy(
            session.base.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(0),
            &session.token,
        ) {
            return Ok(session);
        }
        rag_clear_session();
    }
    let entry = rag_entry(app)?;
    let port = free_loopback_port().ok_or("NO_FREE_PORT: cannot bind an ephemeral port")?;
    let token = session_token();
    // Point the sidecar at the local LLM so ingest/query can use embeddings +
    // answer synthesis; the sidecar falls back to TF-IDF when it is not running.
    let env_file = runtime_env();
    let llama_url = env_file
        .get("LOCAL_LLM_BASE_URL")
        .cloned()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8080".into());
    let mut cmd = tool(node_bin(app));
    cmd.arg(&entry)
        .env("PORT", port.to_string())
        .env("HOST", "127.0.0.1")
        // The token gate: without the matching bearer the sidecar answers 401, so a
        // drive-by web page gets nothing from loopback.
        .env("LFAI_RAG_TOKEN", &token)
        .env(
            "LOCAL_FIRST_STATE_DIR",
            data_dir().to_string_lossy().into_owned(),
        )
        .env("LOCAL_LLM_BASE_URL", llama_url)
        .env("LOCAL_LLM_API_KEY", env_file.get("LOCAL_LLM_API_KEY").cloned().unwrap_or_default())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let spawned = cmd.spawn().map_err(|e| format!("RAG_SPAWN:{e}"))?;
    let pid = spawned.id();
    drop(spawned); // detach: keep only the pid so we can stop it later
    let mut ready = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if rag_healthy(port, &token) {
            ready = true;
            break;
        }
    }
    if !ready {
        #[cfg(unix)]
        let _ = tool("kill").args(["-TERM", &pid.to_string()]).status();
        return Err("RAG_SERVER_TIMEOUT".into());
    }
    if let Ok(mut guard) = state.0.lock() {
        let _ = guard.insert(pid);
    }
    let endpoint = RagEndpoint { base: format!("http://127.0.0.1:{port}"), token };
    rag_save_session(&endpoint);
    Ok(endpoint)
}

#[tauri::command]
pub(crate) fn rag_server_url(
    app: tauri::AppHandle,
    state: tauri::State<'_, RagState>,
) -> Result<RagEndpoint, String> {
    ensure_rag_server(state.inner(), &app)
}

#[derive(Clone)]
pub(crate) struct BootstrapState(pub(crate) Arc<Mutex<Option<BootstrapProgress>>>);

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BootstrapProgress {
    pub(crate) phase: String,
    pub(crate) message: String,
    pub(crate) done: bool,
    pub(crate) error: Option<String>,
}

pub(crate) fn set_bootstrap(
    state: &BootstrapState,
    phase: &str,
    message: &str,
    done: bool,
    error: Option<String>,
) {
    if let Ok(mut guard) = state.0.lock() {
        let _ = guard.insert(BootstrapProgress {
            phase: phase.into(),
            message: message.into(),
            done,
            error,
        });
    }
}

/// Idempotent first-run setup: create the app venv, install openai-whisper, then write
/// runtime.env so every helper resolves the same whisper. Progress is polled by the UI.
pub(crate) fn run_whisper_bootstrap<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &BootstrapState,
) -> Result<String, String> {
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
        let model = file
            .get("WHISPER_MODEL")
            .cloned()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "tiny".into());
        let _ = ensure_bundled_whisper_model(app, &model);
        // YouTube analysis runs yt-dlp + youtube-transcript-api out of the app venv,
        // so make sure it exists even though whisper itself was found elsewhere.
        // Only the light crawler deps go in here — never the PyTorch stack.
        let venv = whisper_venv_dir();
        let mut youtube_ready = true;
        if !venv.join("pyvenv.cfg").is_file() {
            set_bootstrap(state, "venv", "Preparing YouTube support…", false, None);
            youtube_ready = prepare_youtube_venv(&venv);
        }
        // A failed runtime.env write must surface: reporting success without it would
        // leave the analysis worker resolving the wrong whisper later.
        write_runtime_env(&existing, &model)?;
        let note = if youtube_ready {
            "Whisper found on this machine. YouTube support ready."
        } else {
            "Whisper found on this machine."
        };
        set_bootstrap(state, "ready", note, true, None);
        return Ok(existing);
    }
    let python = python_binary();
    if !venv.join("pyvenv.cfg").is_file() {
        set_bootstrap(
            state,
            "venv",
            &format!("Creating Python environment with {python}…"),
            false,
            None,
        );
        std::fs::create_dir_all(cache_dir()).map_err(|e| format!("STATE_DIR:{e}"))?;
        let out = tool(&python)
            .args(["-m", "venv"])
            .arg(&venv)
            .output()
            .map_err(|e| format!("PYTHON_SPAWN:{e}"))?;
        if !out.status.success() {
            return Err(format!(
                "VENV_FAILED:{}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    let pip = whisper_venv_bin("pip");
    set_bootstrap(state, "pip", "Upgrading pip…", false, None);
    let _ = tool(&pip).args(["install", "--upgrade", "pip"]).output();
    set_bootstrap(
        state,
        "whisper",
        "Installing openai-whisper + crawler deps (this downloads PyTorch, a few GB)…",
        false,
        None,
    );
    let install = tool(&pip)
        .args([
            "install",
            "openai-whisper",
            "yt-dlp",
            "youtube-transcript-api",
        ])
        .output()
        .map_err(|e| format!("PIP_SPAWN:{e}"))?;
    if !install.status.success() {
        return Err(format!(
            "WHISPER_INSTALL_FAILED:{}",
            String::from_utf8_lossy(&install.stderr)
                .lines()
                .last()
                .unwrap_or("")
                .trim()
        ));
    }
    if !whisper.is_file() {
        return Err("WHISPER_BINARY_MISSING".into());
    }
    let model = runtime_env()
        .get("WHISPER_MODEL")
        .cloned()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "tiny".into());
    let _ = ensure_bundled_whisper_model(app, &model);
    set_bootstrap(state, "env", "Writing runtime configuration…", false, None);
    write_runtime_env(&whisper.to_string_lossy(), &model)?;
    set_bootstrap(state, "ready", "Whisper is ready.", true, None);
    Ok(whisper.to_string_lossy().into_owned())
}

/// Writes runtime.env preserving every previously chosen option (strategy, keys, endpoints).
pub(crate) fn write_runtime_env(whisper_path: &str, model: &str) -> Result<(), String> {
    let file = runtime_env();
    let strategy = file
        .get("HIGHLIGHT_STRATEGY")
        .cloned()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "heuristic".into());
    let gemini = file.get("GEMINI_API_KEY").cloned().unwrap_or_default();
    let llama_url = file
        .get("LOCAL_LLM_BASE_URL")
        .cloned()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8080".into());
    let llama_model = file
        .get("LOCAL_LLM_MODEL")
        .cloned()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    let hf_token = file.get("HF_TOKEN").cloned().unwrap_or_default();
    let device = std::env::var("WHISPER_DEVICE")
        .ok()
        .unwrap_or_else(|| "cpu".into());
    let fp16 = if device == "cuda" { "True" } else { "False" };
    let mut settings = file.clone();
    settings.insert("WHISPER_COMMAND".into(), whisper_path.into());
    settings.insert("WHISPER_MODEL".into(), model.into());
    settings.insert("WHISPER_DEVICE".into(), device);
    settings.insert("WHISPER_FP16".into(), fp16.into());
    settings.insert("HIGHLIGHT_STRATEGY".into(), strategy);
    settings.insert("GEMINI_API_KEY".into(), gemini);
    settings.insert("LOCAL_LLM_BASE_URL".into(), llama_url);
    settings.insert("LOCAL_LLM_MODEL".into(), llama_model);
    settings.insert("HF_TOKEN".into(), hf_token);
    // The bundled server's per-session endpoint and key must survive a bootstrap
    // rewrite, or the running llama-server becomes unreachable for the helpers.
    if let Some(key) = file.get("LOCAL_LLM_API_KEY") {
        settings.insert("LOCAL_LLM_API_KEY".into(), key.clone());
    }
    if let Some(managed) = file.get("LOCAL_LLM_MANAGED") {
        settings.insert("LOCAL_LLM_MANAGED".into(), managed.clone());
    }
    write_runtime_settings(&settings)
}

/// Starts the whisper bootstrap in the background (first run on a clean machine).
#[tauri::command]
pub(crate) fn bootstrap_whisper(
    app: tauri::AppHandle,
    state: tauri::State<'_, BootstrapState>,
) -> Result<(), String> {
    if let Ok(guard) = state.0.lock() {
        if let Some(progress) = guard.as_ref() {
            if !progress.done {
                return Ok(());
            }
        }
    }
    set_bootstrap(&state, "starting", "Preparing Whisper…", false, None);
    let app_handle = app.clone();
    let shared = state.inner().clone();
    thread::spawn(move || {
        if let Err(error) = run_whisper_bootstrap(&app_handle, &shared) {
            set_bootstrap(
                &shared,
                "failed",
                "Whisper setup failed.",
                true,
                Some(error),
            );
        }
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn whisper_bootstrap_status(state: tauri::State<'_, BootstrapState>) -> Option<BootstrapProgress> {
    state.0.lock().ok().and_then(|guard| guard.clone())
}

pub(crate) type ProgressCell = Arc<Mutex<Option<BootstrapProgress>>>;

#[derive(Clone)]
pub(crate) struct ModelDownloadState(pub(crate) ProgressCell);

#[derive(Clone)]
pub(crate) struct LlamaDownloadState(pub(crate) ProgressCell);

/// Separate managed state so Whisper-model downloads never collide with the
/// local-LLM download progress (Tauri manages state by type).
#[derive(Clone)]
pub(crate) struct WhisperDlState(pub(crate) ProgressCell);

pub(crate) const LLAMA_VERSION: &str = "b11160";

pub(crate) const LLAMA_RUNTIME_ASSETS: &[(&str, &str, &str)] = &[
    (
        "linux-x64",
        "llama-b11160-bin-ubuntu-x64.tar.gz",
        "48ece24283876fc3401b737724008c03cbc4c7ba335b6c1aa2a7b6ce2d49e435",
    ),
    (
        "linux-arm64",
        "llama-b11160-bin-ubuntu-arm64.tar.gz",
        "4ffc2f68959e102219eaa8d007c9fd5b6e7d1cb298672c8945ec80b9f57b6076",
    ),
    (
        "macos-x64",
        "llama-b11160-bin-macos-x64.tar.gz",
        "8c9029bb2491c9c39a497bbd3499d38df0b1c134006bc0e3ec6b5b37319955c8",
    ),
    (
        "macos-arm64",
        "llama-b11160-bin-macos-arm64.tar.gz",
        "5679b3e952772a9f9a39f9d42d7f0eb3d4c424103fe56f5516507583a0c6e3fa",
    ),
    (
        "windows-x64",
        "llama-b11160-bin-win-cpu-x64.zip",
        "b144d125972c57eb30062524269b31bf981dfb81d36fad6a1494e18814a06acc",
    ),
];

pub(crate) fn llama_runtime_asset() -> Result<(&'static str, &'static str), String> {
    let key = if cfg!(target_os = "windows") {
        "windows-x64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos-arm64"
    } else if cfg!(target_os = "macos") {
        "macos-x64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x64"
    } else {
        return Err("LLAMA_PLATFORM_UNSUPPORTED".into());
    };
    let (_, name, sha) = LLAMA_RUNTIME_ASSETS
        .iter()
        .find(|(platform, _, _)| *platform == key)
        .unwrap();
    Ok((*name, *sha))
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    #[cfg(windows)]
    let output = tool("certutil")
        .args(["-hashfile", path.to_string_lossy().as_ref(), "SHA256"])
        .output();
    #[cfg(target_os = "macos")]
    let output = tool("shasum").args(["-a", "256"]).arg(path).output();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let output = tool("sha256sum").arg(path).output();
    let output = output.map_err(|e| format!("SHA256_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err("SHA256_FAILED".into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    #[cfg(windows)]
    let digest = stdout
        .lines()
        .nth(1)
        .unwrap_or("")
        .replace(' ', "")
        .to_ascii_lowercase();
    #[cfg(not(windows))]
    let digest = stdout
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("SHA256_OUTPUT_INVALID".into());
    }
    Ok(digest)
}

pub(crate) fn install_llama_runtime(state: &LlamaDownloadState) -> Result<(), String> {
    let result = install_llama_runtime_inner(state);
    if let Err(error) = &result {
        if let Ok((asset, _)) = llama_runtime_asset() {
            let root = cache_dir().join("llama");
            let _ = std::fs::remove_file(root.join(format!("{asset}.part")));
            let _ = std::fs::remove_dir_all(root.join("installing"));
            let _ = std::fs::remove_dir_all(root.join("runtime-new"));
            let _ = std::fs::remove_dir_all(root.join("runtime-new"));
        }
        set_download(
            &state.0,
            "failed",
            "Local LLM runtime setup failed.",
            true,
            Some(error.clone()),
        );
    }
    result
}

pub(crate) fn install_llama_runtime_inner(state: &LlamaDownloadState) -> Result<(), String> {
    let (asset, expected_sha) = llama_runtime_asset()?;
    let root = cache_dir().join("llama");
    let binary_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    if existing_llama(root.join("runtime")).is_some() {
        set_download(
            &state.0,
            "ready",
            "Local LLM runtime already installed.",
            true,
            None,
        );
        return Ok(());
    }
    std::fs::create_dir_all(&root).map_err(|e| format!("LLAMA_DIR:{e}"))?;
    let archive = root.join(format!("{asset}.part"));
    let url =
        format!("https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_VERSION}/{asset}");
    set_download(
        &state.0,
        "download",
        "Downloading local LLM runtime…",
        false,
        None,
    );
    let curl = tool("curl")
        .args(["-L", "--fail", "--retry", "3", "-o"])
        .arg(&archive)
        .arg(url)
        .output()
        .map_err(|e| format!("CURL_SPAWN:{e}"))?;
    if !curl.status.success() {
        let _ = std::fs::remove_file(&archive);
        return Err(format!(
            "LLAMA_DOWNLOAD_FAILED:{}",
            String::from_utf8_lossy(&curl.stderr).trim()
        ));
    }
    set_download(
        &state.0,
        "verify",
        "Verifying runtime checksum…",
        false,
        None,
    );
    let runtime_hash = sha256_file(&archive);
    if !matches!(&runtime_hash, Ok(actual) if actual == expected_sha) {
        let _ = std::fs::remove_file(&archive);
        return Err(match runtime_hash {
            Ok(_) => "LLAMA_CHECKSUM_MISMATCH".into(),
            Err(error) => error,
        });
    }
    let staging = root.join("installing");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("LLAMA_STAGE:{e}"))?;
    set_download(
        &state.0,
        "install",
        "Installing local LLM runtime…",
        false,
        None,
    );
    #[cfg(windows)]
    let extract = tool("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Expand-Archive -LiteralPath $args[0] -DestinationPath $args[1] -Force",
            archive.to_string_lossy().as_ref(),
            staging.to_string_lossy().as_ref(),
        ])
        .status();
    #[cfg(not(windows))]
    let extract = tool("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&staging)
        .status();
    let extraction = extract.map_err(|e| format!("LLAMA_EXTRACT:{e}"));
    let _ = std::fs::remove_file(&archive);
    if !extraction?.success() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("LLAMA_EXTRACT_FAILED".into());
    }
    let server = find_runtime_server(&staging, binary_name)
        .ok_or_else(|| "LLAMA_SERVER_MISSING".to_string())?;
    let server_parent = server
        .parent()
        .ok_or_else(|| "LLAMA_SERVER_PARENT_MISSING".to_string())?;
    let runtime_stage = root.join("runtime-new");
    let _ = std::fs::remove_dir_all(&runtime_stage);
    std::fs::create_dir_all(&runtime_stage).map_err(|e| format!("LLAMA_STAGE:{e}"))?;
    std::fs::copy(&server, runtime_stage.join(binary_name))
        .map_err(|e| format!("LLAMA_COPY:{e}"))?;
    for entry in std::fs::read_dir(server_parent).map_err(|e| format!("LLAMA_READ:{e}"))? {
        let entry = entry.map_err(|e| format!("LLAMA_READ:{e}"))?;
        let path = entry.path();
        if path.is_file()
            && (path
                .extension()
                .is_some_and(|ext| ["dll", "so", "dylib"].iter().any(|e| ext == *e))
                || path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("lib")))
        {
            std::fs::copy(&path, runtime_stage.join(path.file_name().unwrap()))
                .map_err(|e| format!("LLAMA_COPY:{}:{e}", path.display()))?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(runtime_stage.join(binary_name))
            .map_err(|e| format!("LLAMA_PERMISSIONS:{e}"))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(runtime_stage.join(binary_name), permissions)
            .map_err(|e| format!("LLAMA_PERMISSIONS:{e}"))?;
    }
    let final_dir = root.join("runtime");
    std::fs::rename(&runtime_stage, &final_dir).map_err(|e| format!("LLAMA_INSTALL:{e}"))?;
    let _ = std::fs::remove_dir_all(&staging);
    set_download(&state.0, "ready", "Local LLM runtime ready.", true, None);
    Ok(())
}

pub(crate) fn find_runtime_server(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_runtime_server(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|file| file == name) {
            return Some(path);
        }
    }
    None
}

#[tauri::command]
pub(crate) fn install_local_llm(
    state: tauri::State<'_, LlamaDownloadState>,
    model_state: tauri::State<'_, ModelDownloadState>,
) -> Result<(), String> {
    if state
        .0
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .is_some_and(|p| !p.done)
    {
        return Ok(());
    }
    set_download(
        &state.0,
        "starting",
        "Preparing Local LLM setup…",
        false,
        None,
    );
    let runtime = state.inner().clone();
    let model = model_state.inner().clone();
    let model_name = "qwen2.5-3b-instruct-q4_k_m".to_string();
    thread::spawn(move || {
        if let Err(error) = install_llama_runtime(&runtime) {
            set_download(
                &runtime.0,
                "failed",
                "Local LLM runtime setup failed.",
                true,
                Some(error),
            );
            return;
        }
        if let Err(error) = run_model_download(&model, &model_name) {
            set_download(
                &runtime.0,
                "failed",
                "Local LLM model setup failed.",
                true,
                Some(error),
            );
        } else {
            set_download(&runtime.0, "ready", "Local LLM is ready.", true, None);
        }
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn local_llm_setup_status(
    state: tauri::State<'_, LlamaDownloadState>,
    model_state: tauri::State<'_, ModelDownloadState>,
    app: tauri::AppHandle,
) -> serde_json::Value {
    let model = "qwen2.5-3b-instruct";
    let binary = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let runtime_present = existing_llama(cache_dir().join("llama/runtime")).is_some();
    let model_path = local_llm_model_path(&app, model);
    let model_present = model_path
        .as_ref()
        .is_some_and(|path| sha256_file(path).is_ok_and(|sha| sha == LLM_MODEL_SHA256));
    let progress = state.0.lock().ok().and_then(|g| g.clone());
    let model_progress = model_state.0.lock().ok().and_then(|g| g.clone());
    let effective_progress = model_progress.clone().filter(|p| !p.done).or(progress);
    serde_json::json!({ "runtimePresent": runtime_present, "modelPresent": model_present, "progress": effective_progress, "modelProgress": model_progress, "model": model, "binary": binary })
}

pub(crate) const LLM_MODEL_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf";

pub(crate) const LLM_MODEL_SHA256: &str = "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d";

pub(crate) fn llm_model_file(model: &str) -> PathBuf {
    cache_dir()
        .join("models/llama")
        .join(format!("{model}.gguf"))
}

pub(crate) fn set_download(
    cell: &ProgressCell,
    phase: &str,
    message: &str,
    done: bool,
    error: Option<String>,
) {
    if let Ok(mut guard) = cell.lock() {
        let _ = guard.insert(BootstrapProgress {
            phase: phase.into(),
            message: message.into(),
            done,
            error,
        });
    }
}

/// Downloads the local-LLM weights on demand (2.1 GB) and verifies the SHA-256 before use.
/// Fetch `url` into `target`, verify its SHA-256 against `expected_sha`, then rename the
/// partial file into place. Leftover partials are removed on any failure so a retry
/// starts clean. Kept separate from run_model_download so tests can drive it with
/// file:// URLs instead of a 2.1 GB network download.
pub(crate) fn download_and_verify(
    cell: &ProgressCell,
    url: &str,
    expected_sha: &str,
    target: &Path,
) -> Result<String, String> {
    if target.is_file() {
        let existing_sha = sha256_file(target);
        if matches!(&existing_sha, Ok(digest) if digest == expected_sha) {
            set_download(cell, "ready", "Model already verified.", true, None);
            return Ok(target.to_string_lossy().into_owned());
        }
        if existing_sha.is_err() {
            return Err(existing_sha.unwrap_err());
        }
        std::fs::remove_file(target).map_err(|e| format!("MODEL_REMOVE_INVALID:{e}"))?;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("MODEL_DIR:{e}"))?;
    }
    let part = PathBuf::from(format!("{}.part", target.display()));
    // A leftover .part means a previous download was killed mid-flight; start clean so
    // we never resume into bytes the checksum would then reject.
    if part.is_file() {
        let _ = std::fs::remove_file(&part);
    }
    set_download(cell, "download", "Downloading model…", false, None);
    let curl = tool("curl")
        .args(["-L", "--fail", "--retry", "3", "-o"])
        .arg(&part)
        .arg(url)
        .output()
        .map_err(|e| format!("CURL_SPAWN:{e}"))?;
    if !curl.status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "MODEL_DOWNLOAD_FAILED:{}",
            String::from_utf8_lossy(&curl.stderr).trim()
        ));
    }
    set_download(cell, "verify", "Verifying checksum…", false, None);
    let actual = sha256_file(&part);
    if !matches!(&actual, Ok(digest) if digest == expected_sha) {
        let _ = std::fs::remove_file(&part);
        return Err(match actual {
            Ok(digest) => format!("MODEL_CHECKSUM_MISMATCH:{digest}"),
            Err(error) => error,
        });
    }
    std::fs::rename(&part, target).map_err(|e| format!("MODEL_RENAME:{e}"))?;
    set_download(cell, "ready", "Model ready.", true, None);
    Ok(target.to_string_lossy().into_owned())
}

pub(crate) fn run_model_download(state: &ModelDownloadState, model: &str) -> Result<String, String> {
    download_and_verify(
        &state.0,
        LLM_MODEL_URL,
        LLM_MODEL_SHA256,
        &llm_model_file(model),
    )
}

#[tauri::command]
pub(crate) fn download_llm_model(state: tauri::State<'_, ModelDownloadState>) -> Result<(), String> {
    if let Ok(guard) = state.0.lock() {
        if let Some(progress) = guard.as_ref() {
            if !progress.done {
                return Ok(());
            }
        }
    }
    let model = runtime_env()
        .get("LOCAL_LLM_MODEL")
        .cloned()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    set_download(&state.0, "starting", "Preparing download…", false, None);
    let shared = state.inner().clone();
    thread::spawn(move || {
        if let Err(error) = run_model_download(&shared, &model) {
            set_download(
                &shared.0,
                "failed",
                "Model download failed.",
                true,
                Some(error),
            );
        }
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn model_download_status(
    state: tauri::State<'_, ModelDownloadState>,
    app: tauri::AppHandle,
) -> serde_json::Value {
    let model = runtime_env()
        .get("LOCAL_LLM_MODEL")
        .cloned()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "qwen2.5-3b-instruct-q4_k_m".into());
    let progress = state.0.lock().ok().and_then(|guard| guard.clone());
    let present = local_llm_model_path(&app, &model).is_some();
    serde_json::json!({ "progress": progress, "modelPresent": present, "model": model })
}

// --- Whisper model downloads: tiny/base ship inside the installer; "small"
// (~460 MB) is fetched on demand into the whisper CLI cache (~/.cache/whisper)
// using the same URL the openai-whisper CLI itself downloads from, so the file
// is cache-compatible and later runs need no re-download.
pub(crate) const WHISPER_SMALL_URL: &str = "https://openaipublic.azureedge.net/main/whisper/models/9ecf779972d90ba49c06d968637d720dd632c55bbf19d441fb42bf17a411e794/small.pt";

pub(crate) const WHISPER_SMALL_SHA256: &str =
    "9ecf779972d90ba49c06d968637d720dd632c55bbf19d441fb42bf17a411e794";

pub(crate) fn whisper_cache_file(model: &str) -> PathBuf {
    std::env::var("HOME")
        .map(|h| {
            Path::new(&h)
                .join(".cache/whisper")
                .join(format!("{model}.pt"))
        })
        .unwrap_or_else(|_| std::env::temp_dir().join(format!("{model}.pt")))
}

#[tauri::command]
pub(crate) fn download_whisper_model(
    state: tauri::State<'_, WhisperDlState>,
    model: String,
) -> Result<(), String> {
    if model != "small" {
        return Err("WHISPER_DOWNLOAD_UNSUPPORTED: tiny and base ship inside the installer".into());
    }
    if license_tier() != "pro" {
        return Err("PRO_REQUIRED: larger Whisper models need a Pro license".into());
    }
    if let Ok(guard) = state.0.lock() {
        if let Some(progress) = guard.as_ref() {
            if !progress.done {
                return Ok(());
            }
        }
    }
    set_download(&state.0, "starting", "Preparing download…", false, None);
    let shared = state.inner().0.clone();
    thread::spawn(move || {
        let dest = whisper_cache_file("small");
        if let Err(error) =
            download_and_verify(&shared, WHISPER_SMALL_URL, WHISPER_SMALL_SHA256, &dest)
        {
            set_download(
                &shared,
                "failed",
                "Whisper model download failed.",
                true,
                Some(error),
            );
        }
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn whisper_model_download_status(state: tauri::State<'_, WhisperDlState>) -> serde_json::Value {
    let progress = state.0.lock().ok().and_then(|guard| guard.clone());
    let present = whisper_cache_file("small").is_file();
    serde_json::json!({ "progress": progress, "modelPresent": present, "model": "small" })
}

pub(crate) fn storage(path: &Path, required: u64) -> serde_json::Value {
    let output = tool("df").args(["-Pk"]).arg(path).output();
    let parsed = output
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|text| {
            text.lines().nth(1).and_then(|line| {
                line.split_whitespace()
                    .nth(3)
                    .and_then(|kb| kb.parse::<u64>().ok())
            })
        });
    let free = parsed.map(|kb| kb * 1024);
    serde_json::json!({"path": path, "freeBytes": free, "requiredBytes": required, "ready": free.map(|n| n >= required).unwrap_or(false), "quotaKnown": false})
}

#[tauri::command]
pub(crate) fn storage_status() -> serde_json::Value {
    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    serde_json::json!({"home": storage(&home, 2 * 1024 * 1024 * 1024u64), "temp": storage(&std::env::temp_dir(), 512 * 1024 * 1024u64), "modelCache": home.join(".cache/whisper")})
}

#[tauri::command]
pub(crate) fn clear_analysis_cache(app: tauri::AppHandle, id: String) -> Result<u64, String> {
    let status = run_status_script(&app, &[id.clone()])?;
    if let Some(pid) = status["workerPid"].as_i64() {
        if pid > 0 {
            let _ = tool("kill").args(["-TERM", &pid.to_string()]).status();
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = tool("kill").args(["-KILL", &pid.to_string()]).status();
        }
    }
    let result = run_status_script(&app, &[id, "--clear".into()])?;
    Ok(result["bytes"].as_u64().unwrap_or(0))
}

#[tauri::command]
pub(crate) fn runtime_status(app: tauri::AppHandle) -> serde_json::Value {
    let file = runtime_env();
    // A clean machine has neither a venv nor a PATH whisper; runtime_status
    // reports that truthfully so the UI can point the user at the bootstrap
    // instead of assuming whisper_command() resolved something usable.
    let whisper_env = std::env::var("WHISPER_COMMAND")
        .ok()
        .filter(|c| !c.is_empty());
    let whisper_file = file
        .get("WHISPER_COMMAND")
        .cloned()
        .filter(|c| !c.is_empty());
    let whisper = match (whisper_env, whisper_file) {
        (Some(e), _) => e,
        (None, Some(f)) => f,
        (None, None) => whisper_venv_bin("whisper").to_string_lossy().into_owned(),
    };
    let model = std::env::var("WHISPER_MODEL")
        .ok()
        .or_else(|| file.get("WHISPER_MODEL").cloned())
        .unwrap_or_else(|| "tiny".into());
    // Kick the bundled-model copy in the background (first run only) and report
    // readiness from what is actually on disk — the old synchronous copy froze
    // this command for seconds on page loads.
    let model_copied = ensure_bundled_whisper_model(&app, &model);
    let model_ready = model_copied
        && (std::env::var("WHISPER_MODEL_PATH")
            .is_ok_and(|p| !p.is_empty() && Path::new(&p).exists())
            || std::env::var("HOME")
                .map(|h| {
                    Path::new(&h)
                        .join(".cache/whisper")
                        .join(format!("{model}.pt"))
                        .exists()
                })
                .unwrap_or(false));
    let device = std::env::var("WHISPER_DEVICE")
        .ok()
        .or_else(|| file.get("WHISPER_DEVICE").cloned())
        .unwrap_or_else(|| "cpu".into());
    let ffmpeg = ffmpeg_bin(&app);
    let ffprobe = ffprobe_bin(&app);
    let node = node_bin(&app);
    let whisper_present = Path::new(&whisper).exists()
        || (whisper_venv_bin("whisper").is_file()
            && whisper == whisper_venv_bin("whisper").to_string_lossy().into_owned())
        || command_ready(&whisper);
    serde_json::json!({ "ffmpeg": ffmpeg.as_path().is_file() || command_ready(ffmpeg.to_str().unwrap_or("ffmpeg")), "ffprobe": ffprobe.as_path().is_file() || command_ready(ffprobe.to_str().unwrap_or("ffprobe")), "node": node.as_path().is_file() || command_ready(node.to_str().unwrap_or("node")), "whisper": whisper_present, "model": model, "modelReady": model_ready, "device": device, "bundled": { "node": node.as_path().is_file(), "ffmpeg": ffmpeg.as_path().is_file(), "ffprobe": ffprobe.as_path().is_file() } })
}

/// Persists the chosen Whisper model into runtime.env so init/worker agree on cache keys.
#[tauri::command]
pub(crate) fn set_runtime_model(_app: tauri::AppHandle, model: String) -> Result<String, String> {
    if !["tiny", "base", "small"].contains(&model.as_str()) {
        return Err("INVALID_MODEL: choose tiny, base or small".into());
    }
    if license_tier() != "pro" && model != "tiny" {
        return Err("PRO_REQUIRED: larger Whisper models need a Pro license".into());
    }
    let mut file = runtime_env();
    let command = file
        .get("WHISPER_COMMAND")
        .cloned()
        .filter(|c| !c.is_empty() && whisper_usable(c))
        .unwrap_or_else(whisper_command);
    let device = std::env::var("WHISPER_DEVICE")
        .ok()
        .or_else(|| file.get("WHISPER_DEVICE").cloned())
        .unwrap_or_else(|| "cpu".into());
    let fp16 = if device == "cuda" { "True" } else { "False" };
    file.insert("WHISPER_COMMAND".into(), command);
    file.insert("WHISPER_MODEL".into(), model.clone());
    file.insert("WHISPER_DEVICE".into(), device);
    file.insert("WHISPER_FP16".into(), fp16.into());
    write_runtime_settings(&file)?;
    Ok(model)
}

/// Called on a clean exit (RunEvent::Exit) and after a caught panic.
pub(crate) fn stop_sidecars(llama: &LlamaState, rag: &RagState) {
    if let Ok(mut guard) = llama.lock() {
        if let Some(pid) = guard.take() {
            #[cfg(unix)]
            {
                let _ = tool("kill").args(["-TERM", &pid.to_string()]).status();
            }
            #[cfg(not(unix))]
            {
                let _ = tool("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .status();
            }
        }
    }
    if let Ok(mut guard) = rag.0.lock() {
        if let Some(pid) = guard.take() {
            #[cfg(unix)]
            {
                let _ = tool("kill").args(["-TERM", &pid.to_string()]).status();
            }
            #[cfg(not(unix))]
            {
                let _ = tool("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .status();
            }
        }
    }
    // The sidecar is gone; a stale session record must not point the next session
    // at a dead endpoint.
    rag_clear_session();
}

