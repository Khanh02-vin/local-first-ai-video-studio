// Paths, process helpers and the small utilities every other module needs.
//
// Everything here was inline in main.rs; the analysis, render, preview and YouTube
// paths all use it, so it lives in one place instead of a monolith.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::thread;

use tauri::Manager;


use crate::contracts::AnalysisStatus;


/// Builds a helper command. The Windows release build is a `windows_subsystem`
/// GUI process with no console of its own, so every spawned console tool (node,
/// curl, kill, ...) would open a visible console window — CREATE_NO_WINDOW
/// keeps helper runs invisible.
pub(crate) fn tool(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))] // mutation happens in the Windows block only
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Durable app data: databases, settings, tokens, RAG playlists, licence.
///
/// This is the directory the OS and cleanup tools must never treat as disposable.
/// The previous single-root layout lived under `~/.cache/…`, which cleanup utilities
/// (Storage Sense, CleanMyMac, systemd-tmpfiles) empty whenever disk runs low —
/// taking every project with it. `LOCAL_FIRST_STATE_DIR` still overrides everything
/// into one root, which is what tests and the E2E harness rely on.
pub(crate) fn data_dir() -> PathBuf {
    if let Some(dir) = env_lookup("LOCAL_FIRST_STATE_DIR") {
        return PathBuf::from(dir);
    }
    platform_dir(std::env::consts::OS, &|name| env_lookup(name), DirKind::Data)
}

/// Regenerable caches: preview proxies, the LLM cache, analysis chunk caches, the
/// llama runtime + model and the Python venv. Losing this directory costs downloads
/// and re-encodes, never user data.
pub(crate) fn cache_dir() -> PathBuf {
    if let Some(dir) = env_lookup("LOCAL_FIRST_STATE_DIR") {
        return PathBuf::from(dir);
    }
    platform_dir(std::env::consts::OS, &|name| env_lookup(name), DirKind::Cache)
}

#[derive(Clone, Copy, PartialEq)]
enum DirKind {
    Data,
    Cache,
}

fn env_lookup(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// Resolves one standard OS location. Dispatch happens at runtime (not `cfg!`) so all
/// three branches are unit-testable on any host.
fn platform_dir(os: &str, get: &dyn Fn(&str) -> Option<String>, kind: DirKind) -> PathBuf {
    let home = |get: &dyn Fn(&str) -> Option<String>| get("HOME").or_else(|| get("USERPROFILE"));
    let app_name = "local-first-ai-video-studio";
    match (os, kind) {
        ("windows", DirKind::Data) => get("APPDATA")
            .or_else(|| get("LOCALAPPDATA"))
            .map(|root| Path::new(&root).join(app_name))
            // Windows always sets APPDATA/LOCALAPPDATA in a user session; this last
            // resort exists so the app degrades to a writable dir instead of panicking.
            .unwrap_or_else(std::env::temp_dir),
        ("windows", DirKind::Cache) => get("LOCALAPPDATA")
            .map(|root| Path::new(&root).join(app_name).join("cache"))
            .unwrap_or_else(std::env::temp_dir),
        ("macos", DirKind::Data) => home(get)
            .map(|h| Path::new(&h).join("Library/Application Support").join(app_name))
            .unwrap_or_else(std::env::temp_dir),
        ("macos", DirKind::Cache) => home(get)
            .map(|h| Path::new(&h).join("Library/Caches").join(app_name))
            .unwrap_or_else(std::env::temp_dir),
        // Linux and the BSDs: XDG base directories.
        (_, DirKind::Data) => get("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home(get).map(|h| Path::new(&h).join(".local/share")))
            .map(|root| root.join(app_name))
            .unwrap_or_else(std::env::temp_dir),
        (_, DirKind::Cache) => get("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home(get).map(|h| Path::new(&h).join(".cache")))
            .map(|root| root.join(app_name))
            .unwrap_or_else(std::env::temp_dir),
    }
}

/// Durable files the pre-split layout kept under `~/.cache/local-first-ai-video-studio`.
const MIGRATED_FILES: &[&str] = &[
    "projects.sqlite",
    "projects.sqlite-wal",
    "projects.sqlite-shm",
    "analysis.sqlite",
    "analysis.sqlite-wal",
    "analysis.sqlite-shm",
    "runtime.env",
    "license.lic",
    "providers.json",
    "youtube-tokens.json",
    "youtube-client.json",
];

/// Regenerable-but-expensive directories that are worth renaming across rather than
/// re-downloading: the llama runtime + model and the Python venv.
const MIGRATED_DIRS: &[&str] = &["llama", "models", ".venv"];

/// Moves pre-split-layout data into the separated data/cache directories.
///
/// Copy-only for the durable files (the legacy directory is left untouched as a
/// backup), rename for the regenerable multi-GB directories (rename is instant on the
/// same volume; when it fails — cross-device — the app simply re-downloads later).
/// Idempotent: a file is only touched when the source exists and the target does not.
pub(crate) fn migrate_legacy_state() {
    if env_lookup("LOCAL_FIRST_STATE_DIR").is_some() {
        return; // single-root mode: the user (or the E2E) manages the layout
    }
    let legacy = platform_dir(std::env::consts::OS, &|name| env_lookup(name), DirKind::Cache);
    migrate_legacy_state_into(&legacy, &data_dir(), &cache_dir());
}

fn migrate_legacy_state_into(legacy: &Path, data: &Path, cache: &Path) {
    // On Linux the legacy root *is* the new cache root, so there is nothing cache-side
    // to move; the data files still migrate out of it.
    if data == legacy && cache == legacy {
        return;
    }
    // First launch after an upgrade: the target roots do not exist yet.
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::create_dir_all(cache);
    for file in MIGRATED_FILES {
        let source = legacy.join(file);
        let target = data.join(file);
        if source.is_file() && !target.exists() {
            let _ = std::fs::copy(&source, &target);
        }
    }
    // The RAG playlists are user data: copy the whole directory.
    let legacy_youtube = legacy.join("youtube");
    let target_youtube = data.join("youtube");
    if legacy_youtube.is_dir() && !target_youtube.exists() {
        let _ = copy_dir_recursive(&legacy_youtube, &target_youtube);
    }
    // The LLM cache is small and regenerable, but copying it spares token spend.
    let legacy_llm = legacy.join("llm-cache");
    let target_llm = cache.join("llm-cache");
    if legacy_llm.is_dir() && !target_llm.exists() {
        let _ = copy_dir_recursive(&legacy_llm, &target_llm);
    }
    for dir in MIGRATED_DIRS {
        let source = legacy.join(dir);
        let target = cache.join(dir);
        if source.is_dir() && !target.exists() {
            // Same volume → instant. Cross-device → give up; the bootstrap re-fetches.
            let _ = std::fs::rename(&source, &target);
        }
    }
}

fn copy_dir_recursive(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let to = target.join(entry.file_name());
        if kind.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Generates a per-session secret for the loopback sidecars (RAG, llama, preview).
///
/// The threat is a web page in the user's browser probing localhost: an
/// unauthenticated sidecar would happily answer drive-by requests to its HTTP API
/// (playlists, embeddings, YouTube-backed ranking). The token only has to be
/// unguessable for the lifetime of a page, so std's per-process `RandomState`
/// (seeded from OS entropy) mixed with clock, pid and an ASLR address is enough —
/// this is not a defence against local malware running as the same user.
pub(crate) fn session_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let marker = 0u8;
    let mut hasher = RandomState::new().build_hasher();
    std::process::id().hash(&mut hasher);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    (&marker as *const u8 as usize).hash(&mut hasher);
    format!(
        "{:016x}{:016x}",
        hasher.finish(),
        RandomState::new().build_hasher().finish()
    )
}

/// Picks a free loopback port by binding an ephemeral listener and releasing it.
///
/// A tiny TOCTOU window remains (another process could claim the port before the
/// sidecar binds it); the sidecar's health-wait turns that into a clean timeout
/// error instead of a silent cross-talk with a stranger's server.
pub(crate) fn free_loopback_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    listener.local_addr().ok().map(|addr| addr.port())
}

pub(crate) fn jobs_path() -> PathBuf {
    data_dir().join("analysis.sqlite")
}

/// Prefer a bundled binary in the app resources, then fall back to PATH.
/// Windows ships `node.exe`/`ffmpeg.exe`; Unix packages use the extensionless
/// name. Probe the platform-native name first so one config serves both.
pub(crate) fn bundled_bin(app: &tauri::AppHandle, rel: &str, fallback: &str) -> PathBuf {
    if let Ok(dir) = app.path().resource_dir() {
        let candidates: Vec<PathBuf> = if cfg!(windows) {
            vec![dir.join(format!("{rel}.exe")), dir.join(rel)]
        } else {
            vec![dir.join(rel)]
        };
        for candidate in candidates {
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(fallback)
}

/// Reads KEY=VALUE lines from the runtime env file (written by scripts/setup-machine.sh).
pub(crate) fn runtime_env() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(text) = std::fs::read_to_string(data_dir().join("runtime.env")) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                map.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }
    map
}

pub(crate) fn write_runtime_settings(
    settings: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    if settings
        .iter()
        .any(|(key, value)| key.contains(['=', '\n', '\r']) || value.contains(['\n', '\r']))
    {
        return Err("INVALID_RUNTIME_SETTING".into());
    }
    let text = settings
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    write_private_file(&data_dir().join("runtime.env"), text.as_bytes())
}

#[cfg(unix)]
pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let parent = path.parent().ok_or("INVALID_SECRET_PATH")?;
    std::fs::DirBuilder::new().mode(0o700).recursive(true).create(parent)
        .or_else(|error| if error.kind() == std::io::ErrorKind::AlreadyExists { Ok(()) } else { Err(error) })
        .map_err(|e| format!("STATE_DIR:{e}"))?;
    let mut file = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(path).map_err(|e| format!("SECRET_WRITE:{e}"))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("SECRET_WRITE:{e}"))?;
    file.write_all(bytes).map_err(|e| format!("SECRET_WRITE:{e}"))?;
    file.sync_all().map_err(|e| format!("SECRET_WRITE:{e}"))
}

#[cfg(not(unix))]
pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("INVALID_SECRET_PATH")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("STATE_DIR:{e}"))?;
    let mut file = std::fs::File::create(path).map_err(|e| format!("SECRET_WRITE:{e}"))?;
    file.write_all(bytes).map_err(|e| format!("SECRET_WRITE:{e}"))?;
    file.sync_all().map_err(|e| format!("SECRET_WRITE:{e}"))
}

/// Environment for spawned helpers: bundled binaries + runtime.env defaults, real env wins.
pub(crate) fn helper_env(app: &tauri::AppHandle, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let file = runtime_env();
    let mut env: Vec<(String, String)> = file
        .iter()
        .filter(|(key, _)| key.as_str() != "HF_TOKEN")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    env.push((
        "LOCAL_FIRST_STATE_DIR".into(),
        data_dir().to_string_lossy().into_owned(),
    ));
    env.push((
        "LOCAL_FIRST_CACHE_DIR".into(),
        cache_dir().to_string_lossy().into_owned(),
    ));
    env.push(("LICENSE_TIER".into(), license_tier()));
    env.push((
        "FFMPEG_COMMAND".into(),
        ffmpeg_bin(app).to_string_lossy().into_owned(),
    ));
    for (key, value) in extra {
        env.push((key.to_string(), value.to_string()));
    }
    env
}

/// True for remote inputs (YouTube URLs) that must skip local file checks.
pub(crate) fn is_remote_input(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://")
}

pub(crate) fn local_preview_source(input: &str) -> Result<PathBuf, String> {
    if is_remote_input(input) {
        return Err("REMOTE_PREVIEW_UNSUPPORTED".into());
    }
    let source = Path::new(input)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !source.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    Ok(source)
}

pub(crate) fn preview_proxy_cache_key(source: &Path, size: u64, modified_nanos: u128) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.to_string_lossy().hash(&mut hasher);
    size.hash(&mut hasher);
    modified_nanos.hash(&mut hasher);
    1_u32.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn transcode_preview_proxy(
    ffmpeg: &Path,
    ffprobe: &Path,
    source: &Path,
    output: &Path,
) -> Result<(), String> {
    let result = tool(ffmpeg)
        .args(["-y", "-v", "error", "-i"])
        .arg(source)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
        ])
        .arg(output)
        .output()
        .map_err(|e| format!("PREVIEW_FFMPEG_SPAWN:{e}"))?;
    if !result.status.success() {
        let _ = std::fs::remove_file(output);
        return Err(format!(
            "PREVIEW_FFMPEG:{}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    let probe = tool(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,pix_fmt",
            "-of",
            "json",
        ])
        .arg(output)
        .output()
        .map_err(|e| format!("PREVIEW_FFPROBE_SPAWN:{e}"))?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&probe.stdout).map_err(|e| format!("PREVIEW_FFPROBE_OUTPUT:{e}"))?;
    let valid = probe.status.success()
        && metadata["streams"].as_array().is_some_and(|streams| {
            streams
                .iter()
                .any(|stream| stream["codec_name"] == "h264" && stream["pix_fmt"] == "yuv420p")
        });
    if !valid {
        let _ = std::fs::remove_file(output);
        return Err("PREVIEW_OUTPUT_INVALID: expected H.264 yuv420p video".into());
    }
    Ok(())
}

/// Parse and constrain the local model endpoint to loopback before persisting it.
pub(crate) fn valid_local_llm_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value.trim()) else { return false; };
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

pub(crate) fn valid_provider_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    { return None; }
    let mut normalized = url;
    let path = normalized.path().trim_end_matches('/').to_string();
    normalized.set_path(&path);
    Some(normalized.to_string().trim_end_matches('/').to_string())
}

pub(crate) fn node_bin(app: &tauri::AppHandle) -> PathBuf {
    bundled_bin(app, "node/bin/node", "node")
}

pub(crate) fn ffmpeg_bin(app: &tauri::AppHandle) -> PathBuf {
    bundled_bin(app, "ffmpeg", "ffmpeg")
}

pub(crate) fn ffprobe_bin(app: &tauri::AppHandle) -> PathBuf {
    bundled_bin(app, "ffprobe", "ffprobe")
}

/// Resolve the whisper CLI the way the local whisper adapter will find it:
/// env/runtime.env first, then a venv next to the app, then plain PATH.
/// True when a whisper CLI string can actually be executed: a path that exists, or a
/// bare name resolvable via PATH. Guards stale runtime.env entries like `whisper` when
/// whisper is not installed system-wide.
pub(crate) fn whisper_usable(cmd: &str) -> bool {
    // Native separators: Windows writes absolute paths with backslashes, so both
    // separators mark "this is a path" — otherwise a valid C:\…\whisper.exe would
    // be misrouted to a PATH lookup and the bootstrap would reinstall over it.
    if cmd.contains('/') || cmd.contains('\\') {
        Path::new(cmd).is_file()
    } else {
        command_ready(cmd)
    }
}

pub(crate) fn whisper_command() -> String {
    if let Ok(cmd) = std::env::var("WHISPER_COMMAND") {
        if !cmd.is_empty() && whisper_usable(&cmd) {
            return cmd;
        }
    }
    // runtime.env is authoritative only while the command it names still resolves.
    if let Some(cmd) = runtime_env()
        .get("WHISPER_COMMAND")
        .cloned()
        .filter(|c| !c.is_empty())
    {
        if whisper_usable(&cmd) {
            return cmd;
        }
    }
    // App-owned venv created by the first-run bootstrap (client machines).
    let app_venv = whisper_venv_bin("whisper");
    if app_venv.is_file() {
        return app_venv.to_string_lossy().into_owned();
    }
    // Project venv: repo root is three levels up from src-tauri; also accept the legacy
    // apps/desktop/.venv location from earlier checkouts.
    for rel in ["../../../.venv/bin/whisper", "../.venv/bin/whisper"] {
        let candidate = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if Path::new(&home).join(".local/bin/whisper").is_file() {
            return format!("{home}/.local/bin/whisper");
        }
    }
    "whisper".into()
}

pub(crate) fn status_script(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // Bundled resources come first: in a dev build the tauri dev runtime wires
    // the resource dir to src-tauri/resources, which does not contain the
    // analysis scripts. Installed builds ship scripts/ (bundle.resources), so
    // the resource-dir hit only succeeds there. Fall back to the dev-tree
    // scripts/ next to the repo root.
    let candidate = app
        .path()
        .resource_dir()
        .map_err(|e| format!("RESOURCE_DIR:{e}"))?
        .join("scripts/analysis-status.ts");
    if candidate.is_file() {
        return Ok(candidate);
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/analysis-status.ts");
    if dev.is_file() {
        return Ok(dev);
    }
    Err(
        "STATUS_SCRIPT_MISSING: scripts/analysis-status.ts not found in resources or the dev tree"
            .into(),
    )
}

/// Resolves a bundled helper script by name with the same resource-dir-first,
/// dev-tree-fallback strategy as status_script().
pub(crate) fn helper_script(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let candidate = app
        .path()
        .resource_dir()
        .map_err(|e| format!("RESOURCE_DIR:{e}"))?
        .join(format!("scripts/{name}"));
    if candidate.is_file() {
        return Ok(candidate);
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts")
        .join(name);
    if dev.is_file() {
        return Ok(dev);
    }
    Err(format!(
        "SCRIPT_MISSING:scripts/{name} not found in resources or the dev tree"
    ))
}

pub(crate) fn run_status_script(app: &tauri::AppHandle, args: &[String]) -> Result<serde_json::Value, String> {
    let script = status_script(app)?;
    let mut cmd = tool(node_bin(app));
    cmd.args(NODE_ARGS).arg(&script).arg(jobs_path()).args(args);
    for (key, value) in helper_env(app, &[]) {
        cmd.env(key, value);
    }
    let output = cmd.output().map_err(|e| format!("STATUS_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("STATUS_JSON:{e}"))
}

pub(crate) fn status_from_json(job: &serde_json::Value) -> Result<AnalysisStatus, String> {
    let result = match job.get("result") {
        Some(serde_json::Value::String(encoded)) => Some(serde_json::from_str(encoded).map_err(|error| format!("ANALYSIS_RESULT_JSON:{error}"))?),
        Some(serde_json::Value::Null) | None => None,
        Some(value) => Some(value.clone()),
    };
    Ok(AnalysisStatus {
        status: job["status"].as_str().unwrap_or("failed").into(),
        phase: job["phase"].as_str().unwrap_or("unknown").into(),
        progress: job["progress"].as_f64().unwrap_or(0.0),
        result,
        error: job["error"].as_str().map(String::from),
        chunk_completed: job["chunkCompleted"].as_u64().unwrap_or(0),
        chunk_total: job["chunkTotal"].as_u64().unwrap_or(0),
        current_chunk: job["currentChunk"].as_u64(),
    })
}

pub(crate) fn read_sqlite_status(app: &tauri::AppHandle, id: &str) -> Result<AnalysisStatus, String> {
    let value = run_status_script(app, &[id.to_string()])?;
    if value.is_null() {
        return Err("JOB_NOT_FOUND".into());
    }
    status_from_json(&value)
}

pub(crate) fn probe_duration(app: &tauri::AppHandle, input: &Path) -> Result<f64, String> {
    let output = Command::new(ffprobe_bin(app))
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
        .output()
        .map_err(|e| format!("FFPROBE_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "INVALID_DURATION".to_string())?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err("INVALID_DURATION".into());
    }
    Ok(duration)
}

pub(crate) fn command_ready(command: &str) -> bool {
    // Same native-separator rule as whisper_usable: a Windows absolute path uses
    // backslashes and must be probed as a file, not via `which`.
    if command.contains('/') || command.contains('\\') {
        return Path::new(command).is_file();
    }
    tool("which")
        .arg(command)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Copies the bundled Whisper model into ~/.cache/whisper so the openai-whisper CLI
/// (which looks there by default) finds it without downloading. Runs in the
/// background — `runtime_status` used to do this synchronously, freezing page
/// loads for the seconds a 75–147 MB copy takes on first run. Returns true when
/// the model is ready on disk; a missing source (dev/LIGHTWEIGHT builds) counts
/// as ready so status checks never block on a copy that cannot happen.
pub(crate) fn ensure_bundled_whisper_model<R: tauri::Runtime>(app: &tauri::AppHandle<R>, model: &str) -> bool {
    let dest = match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home)
            .join(".cache/whisper")
            .join(format!("{model}.pt")),
        Err(_) => return true,
    };
    if dest.is_file() {
        return true;
    }
    let source = match app.path().resource_dir() {
        Ok(dir) => dir.join(format!("models/{model}.pt")),
        Err(_) => return true,
    };
    if !source.is_file() {
        return true;
    }
    if let Ok(mut copies) = MODEL_COPIES.lock() {
        if copies.iter().any(|m| m == model) {
            return false;
        } // copy already in flight
        copies.push(model.to_string());
    }
    let model_name = model.to_string();
    thread::spawn(move || {
        let ok = (|| -> Result<(), String> {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("WHISPER_MODEL_DIR:{e}"))?;
            }
            std::fs::copy(&source, &dest).map_err(|e| format!("WHISPER_MODEL_COPY:{e}"))?;
            Ok(())
        })();
        if ok.is_err() {
            if let Ok(mut copies) = MODEL_COPIES.lock() {
                copies.retain(|m| *m != model_name);
            } // allow retry
        }
    });
    false
}

/// Where the app-owned whisper venv lives (created on first run for machines that have
/// neither a project venv nor a whisper on PATH).
///
/// `LOCAL_FIRST_VENV` overrides it, the same variable scripts/setup-machine.sh reads:
/// a machine that keeps its venv outside the state directory would otherwise have the
/// app looking in a place that does not exist.
pub(crate) fn whisper_venv_dir() -> PathBuf {
    std::env::var("LOCAL_FIRST_VENV")
        .map(PathBuf::from)
        .unwrap_or_else(|_| cache_dir().join(".venv"))
}

pub(crate) fn whisper_venv_bin(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        whisper_venv_dir()
            .join("Scripts")
            .join(format!("{name}.exe"))
    }
    #[cfg(not(windows))]
    {
        whisper_venv_dir().join("bin").join(name)
    }
}

/// Python interpreter inside the app venv (yt-dlp + youtube-transcript-api live there).
pub(crate) fn venv_python() -> PathBuf {
    #[cfg(windows)]
    {
        whisper_venv_dir().join("Scripts").join("python.exe")
    }
    #[cfg(not(windows))]
    {
        whisper_venv_dir().join("bin").join("python3")
    }
}

/// Picks a Python interpreter that can build a venv, preferring a version torch still
/// supports (3.10–3.13); falls back to python3 on PATH.
pub(crate) fn python_binary() -> String {
    for candidate in [
        "python3.12",
        "python3.11",
        "python3.13",
        "python3.10",
        "python3",
    ] {
        if tool(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return candidate.to_string();
        }
    }
    "python3".into()
}

pub(crate) fn analysis_range_allowed(start: f64, end: f64) -> bool {
    start.is_finite() && end.is_finite() && start >= 0.0 && end > start && end - start <= 600.0
}

/// Ed25519 public key (32 bytes, base64) used to verify offline licenses.
/// Leave empty to run unlicensed; paste the output of `scripts/license-keys.ts`.
/// Never commit keys/license-private.pem — generate your own keypair for production.
pub(crate) const LICENSE_PUBLIC_KEY_B64: &str = "dj1xRsYANUUgntXmdcTd9Kv+hoG5ENBT3HyPvRmKtic=";

/// Verifies license.lic against the embedded public key and expiry; returns tier ("free"|"pro").
pub(crate) fn license_tier() -> String {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use ed25519_dalek::{Signature, VerifyingKey};
    if LICENSE_PUBLIC_KEY_B64.is_empty() {
        return "free".into();
    }
    let (Ok(pub_bytes), Ok(raw)) = (
        BASE64.decode(LICENSE_PUBLIC_KEY_B64),
        std::fs::read(data_dir().join("license.lic")),
    ) else {
        return "free".into();
    };
    let Ok(pub_key) = VerifyingKey::from_bytes(
        pub_bytes
            .as_slice()
            .try_into()
            .map_err(|_| ())
            .unwrap_or(&[0u8; 32]),
    ) else {
        return "free".into();
    };
    let Ok(text) = String::from_utf8(raw) else {
        return "free".into();
    };
    let mut lines = text.lines();
    let (Some(payload_b64), Some(sig_b64)) = (lines.next(), lines.next()) else {
        return "free".into();
    };
    let (Ok(payload), Ok(sig_bytes)) = (BASE64.decode(payload_b64), BASE64.decode(sig_b64)) else {
        return "free".into();
    };
    let sig = Signature::from_bytes(sig_bytes.as_slice().try_into().unwrap_or(&[0u8; 64]));
    if pub_key.verify_strict(&payload, &sig).is_err() {
        return "free".into();
    }
    let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&payload) else {
        return "free".into();
    };
    let expired = meta["expiresAt"]
        .as_str()
        .and_then(|s| {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
        })
        .is_some_and(|t| t <= time::OffsetDateTime::now_utc());
    if expired {
        return "free".into();
    }
    meta["tier"].as_str().unwrap_or("free").into()
}

/// Pro feature gate: larger models and GPU are paid features.
pub(crate) fn guard_pro_features(whisper_model: &str, whisper_device: &str) -> Result<(), String> {
    if license_tier() == "pro" {
        return Ok(());
    }
    let model = if whisper_model.is_empty() {
        std::env::var("WHISPER_MODEL")
            .ok()
            .or_else(|| {
                let file = runtime_env();
                file.get("WHISPER_MODEL").cloned().filter(|c| !c.is_empty())
            })
            .unwrap_or_else(|| "tiny".into())
    } else {
        whisper_model.to_string()
    };
    let device = if whisper_device.is_empty() {
        std::env::var("WHISPER_DEVICE")
            .ok()
            .or_else(|| {
                let file = runtime_env();
                file.get("WHISPER_DEVICE")
                    .cloned()
                    .filter(|c| !c.is_empty())
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

/// Arguments every Node sidecar is spawned with (the scripts are TypeScript).
pub(crate) const NODE_ARGS: &[&str] = &["--experimental-strip-types"];

/// Model files currently being copied into the whisper CLI cache.
pub(crate) static MODEL_COPIES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(crate) fn io_error(code: &str, message: impl std::fmt::Display) -> String {
    let text = message.to_string();
    if text.contains("ENOSPC") || text.contains("No space left") {
        format!("STORAGE_FULL:{text}")
    } else if text.contains("EDQUOT") || text.contains("Disk quota") {
        format!("QUOTA_EXCEEDED:{text}")
    } else {
        format!("{code}:{text}")
    }
}


#[cfg(test)]
mod tests {
    use super::{migrate_legacy_state_into, platform_dir, DirKind};
    use std::path::Path;

    fn get<'a>(mapping: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name: &str| {
            mapping
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }
    // The closures below only read; static tables need no lifetime plumbing.
    #[allow(unused_variables)]

    /// The Windows data root must come from APPDATA (roaming), never from TEMP: the
    /// old `HOME`-based lookup silently fell back to `std::env::temp_dir()` on Windows
    /// because Windows does not set HOME, putting the project databases where disk
    /// cleanup deletes them.
    #[test]
    fn windows_data_dir_uses_appdata_not_temp() {
        let dir = platform_dir("windows", &get(&[("APPDATA", r"C:\Users\u\AppData\Roaming")]), DirKind::Data);
        assert_eq!(dir, Path::new(r"C:\Users\u\AppData\Roaming").join("local-first-ai-video-studio"));
        // APPDATA missing → LOCALAPPDATA is the fallback.
        let dir = platform_dir("windows", &get(&[("LOCALAPPDATA", r"C:\Users\u\AppData\Local")]), DirKind::Data);
        assert_eq!(dir, Path::new(r"C:\Users\u\AppData\Local").join("local-first-ai-video-studio"));
        // Both missing → the last resort is temp, but that is a broken machine.
        let dir = platform_dir("windows", &get(&[]), DirKind::Data);
        assert_eq!(dir, std::env::temp_dir());
    }

    #[test]
    fn windows_cache_dir_lives_under_localappdata() {
        let dir = platform_dir("windows", &get(&[("LOCALAPPDATA", r"C:\Users\u\AppData\Local")]), DirKind::Cache);
        assert_eq!(dir, Path::new(r"C:\Users\u\AppData\Local").join("local-first-ai-video-studio").join("cache"));
    }

    #[test]
    fn macos_dirs_follow_library_conventions() {
        let home = [("HOME", "/Users/u")];
        let data = platform_dir("macos", &get(&home), DirKind::Data);
        assert_eq!(data, Path::new("/Users/u").join("Library/Application Support").join("local-first-ai-video-studio"));
        let cache = platform_dir("macos", &get(&home), DirKind::Cache);
        assert_eq!(cache, Path::new("/Users/u").join("Library/Caches").join("local-first-ai-video-studio"));
    }

    #[test]
    fn linux_dirs_follow_xdg() {
        let explicit = get(&[("XDG_DATA_HOME", "/xdg-data"), ("XDG_CACHE_HOME", "/xdg-cache")]);
        assert_eq!(platform_dir("linux", &explicit, DirKind::Data), Path::new("/xdg-data").join("local-first-ai-video-studio"));
        assert_eq!(platform_dir("linux", &explicit, DirKind::Cache), Path::new("/xdg-cache").join("local-first-ai-video-studio"));
        // Without XDG variables the HOME fallbacks apply; USERPROFILE backs HOME up.
        let bare = get(&[("USERPROFILE", "/home/u")]);
        assert_eq!(platform_dir("linux", &bare, DirKind::Data), Path::new("/home/u").join(".local/share").join("local-first-ai-video-studio"));
        assert_eq!(platform_dir("linux", &bare, DirKind::Cache), Path::new("/home/u").join(".cache").join("local-first-ai-video-studio"));
    }

    /// The migration copies durable files out of the legacy cache root without
    /// deleting anything, renames the regenerable directories, and does nothing the
    /// second time around.
    #[test]
    fn migration_copies_data_renames_caches_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!("lfai-migrate-test-{}", std::process::id()));
        let legacy = root.join("legacy");
        let data = root.join("data");
        let cache = root.join("cache");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(legacy.join("youtube")).unwrap();
        std::fs::create_dir_all(legacy.join("llm-cache")).unwrap();
        std::fs::create_dir_all(legacy.join("llama/runtime")).unwrap();
        std::fs::write(legacy.join("projects.sqlite"), b"project db").unwrap();
        std::fs::write(legacy.join("runtime.env"), b"WHISPER_MODEL=tiny").unwrap();
        std::fs::write(legacy.join("youtube/playlist.json"), b"{}").unwrap();
        std::fs::write(legacy.join("llm-cache/abc.json"), b"{}").unwrap();
        std::fs::write(legacy.join("llama/runtime/llama-server"), b"bin").unwrap();

        migrate_legacy_state_into(&legacy, &data, &cache);

        assert_eq!(std::fs::read(data.join("projects.sqlite")).unwrap(), b"project db");
        assert_eq!(std::fs::read(data.join("runtime.env")).unwrap(), b"WHISPER_MODEL=tiny");
        assert!(data.join("youtube/playlist.json").is_file());
        assert!(cache.join("llm-cache/abc.json").is_file());
        assert!(cache.join("llama/runtime/llama-server").is_file());
        // Copy-only: the legacy root is a backup, not a sacrifice.
        assert!(legacy.join("projects.sqlite").is_file());
        assert!(legacy.join("runtime.env").is_file());

        // Idempotent: a second run must not fail or duplicate anything.
        migrate_legacy_state_into(&legacy, &data, &cache);
        assert_eq!(std::fs::read(data.join("projects.sqlite")).unwrap(), b"project db");

        // An existing target file is never overwritten by a stale legacy copy.
        std::fs::write(data.join("runtime.env"), b"WHISPER_MODEL=base").unwrap();
        std::fs::write(legacy.join("runtime.env"), b"WHISPER_MODEL=tiny").unwrap();
        migrate_legacy_state_into(&legacy, &data, &cache);
        assert_eq!(std::fs::read(data.join("runtime.env")).unwrap(), b"WHISPER_MODEL=base");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Sidecar tokens are unguessable per session and differ between calls.
    #[test]
    fn session_token_is_hex_and_varies() {
        let first = super::session_token();
        let second = super::session_token();
        assert_eq!(first.len(), 32, "128-bit token as 32 hex chars");
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second, "two tokens in one process must differ");
    }

    #[test]
    fn free_loopback_port_returns_a_usable_port() {
        let port = super::free_loopback_port().expect("an ephemeral port must be bindable");
        assert_ne!(port, 0);
        // The port came from the OS, so rebinding it right away normally works —
        // but never assert on a race; only assert the shape.
    }

    /// A project database already living in the data directory must never be replaced
    /// by the migration, even when the legacy copy is newer-looking.
    #[test]
    fn migration_never_touches_an_existing_database() {
        let root = std::env::temp_dir().join(format!("lfai-migrate-keep-{}", std::process::id()));
        let legacy = root.join("legacy");
        let data = root.join("data");
        let cache = root.join("cache");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(legacy.join("projects.sqlite"), b"old").unwrap();
        std::fs::write(data.join("projects.sqlite"), b"new").unwrap();

        migrate_legacy_state_into(&legacy, &data, &cache);
        assert_eq!(std::fs::read(data.join("projects.sqlite")).unwrap(), b"new");

        let _ = std::fs::remove_dir_all(&root);
    }
}
