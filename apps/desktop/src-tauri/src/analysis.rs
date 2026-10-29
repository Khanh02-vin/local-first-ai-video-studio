// The analysis branch: starting a job, spawning the Node worker, polling its status
// and cancelling it.
//
// The worker lifecycle (pid registry, process-group kill) lives here because it is
// only ever driven from the analysis commands.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contracts::AnalysisStatus;
use crate::runtime::storage;
use crate::analysis_store::{job_with_result, read_status, AnalysisStore, MAX_JOB_ATTEMPTS};
use crate::paths::{
    analysis_range_allowed, app_render_temp_dir, ffmpeg_bin, helper_env, helper_script,
    is_remote_input, jobs_path, node_bin, probe_duration, runtime_env, tool, venv_python,
};
use crate::paths::NODE_ARGS;
use crate::{guard_pro_features, io_error};

pub(crate) static ANALYSIS_RUNNING: AtomicBool = AtomicBool::new(false);

/// Pids of analysis workers that are still running, keyed by job id so a cancel can
/// find the process Rust itself spawned (the DB copy is written by the worker and may
/// not exist yet) and an app exit can take down the whole tree.
pub(crate) static ANALYSIS_WORKERS: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) struct RunningGuard(pub(crate) &'static AtomicBool);

pub(crate) type Jobs = Arc<Mutex<HashMap<String, AnalysisStatus>>>;

#[tauri::command]
pub(crate) fn preflight_analyze(path: String) -> Result<serde_json::Value, String> {
    let input = Path::new(&path)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let result = storage(
        input.parent().unwrap_or(Path::new("/")),
        2 * 1024 * 1024 * 1024u64,
    );
    if !result["ready"].as_bool().unwrap_or(false) {
        return Err("STORAGE_FULL:analysis requires at least 2 GiB free".into());
    }
    Ok(result)
}

#[tauri::command]
pub(crate) fn start_analysis(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, Jobs>,
    path: String,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<String, String> {
    let file = runtime_env();
    let whisper_model = file
        .get("WHISPER_MODEL")
        .cloned()
        .unwrap_or_else(|| "tiny".into());
    let whisper_device = file
        .get("WHISPER_DEVICE")
        .cloned()
        .unwrap_or_else(|| "cpu".into());
    guard_pro_features(&whisper_model, &whisper_device)?;
    let input = Path::new(&path)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let id = format!(
        "analysis-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "CLOCK_ERROR")?
            .as_nanos()
    );
    let duration = probe_duration(&app, &input)?;
    let range_start = start.unwrap_or(0.0);
    let range_end = end.unwrap_or(duration);
    let init_script = helper_script(&app, "analysis-init.ts")?;
    if !analysis_range_allowed(range_start, range_end) || range_end > duration {
        return Err(if range_end - range_start > 600.0 {
            "ANALYZE_RANGE_EXCEEDS_600_SECONDS: select a shorter range".into()
        } else {
            "INVALID_ANALYZE_RANGE".into()
        });
    }

    // Decode the source audio exactly once (16 kHz mono WAV) into the app-private
    // temp dir, so diarize_video / sample_speaker_faces / analyze_crop_path can reuse it
    // instead of re-decoding the whole video on every task. The path is handed to the
    // worker via ANALYSIS_PREEXTRACTED_AUDIO; the Rust-side AI commands read the same env.
    let temp_dir = app_render_temp_dir(&app).map_err(|e| format!("APP_TEMP_DIR:{e}"))?;
    let audio_path = temp_dir.join(format!("analysis-{id}-audio.wav"));
    let audio_extract = tool(ffmpeg_bin(&app))
        .args(["-y", "-nostdin", "-v", "error", "-i"])
        .arg(&input)
        .args(["-vn", "-ac", "1", "-ar", "16000"])
        .arg(&audio_path)
        .status()
        .map_err(|e| format!("ANALYSIS_AUDIO_EXTRACT:{e}"))?;
    if !audio_extract.success() {
        return Err("ANALYSIS_AUDIO_EXTRACT_FAILED".into());
    }

    let mut init = tool(node_bin(&app));
    init.args(NODE_ARGS)
        .arg(init_script)
        .arg(jobs_path())
        .arg(&id)
        .arg(&input)
        .arg(format!("{duration:.3}"))
        .arg(format!("{range_start:.3}"))
        .arg(format!("{range_end:.3}"))
        .env("ANALYSIS_PREEXTRACTED_AUDIO", &audio_path);
    for (key, value) in helper_env(&app, &[]) {
        init.env(key, value);
    }
    let init = init.output().map_err(|e| format!("STORE_INIT:{e}"))?;
    if !init.status.success() {
        return Err(String::from_utf8_lossy(&init.stderr).trim().to_owned());
    }
    if !spawn_analysis_worker(
        app.clone(),
        jobs.inner().clone(),
        id.clone(),
        input,
        duration,
        range_start,
        range_end,
    ) {
        let _ = AnalysisStore::open(&jobs_path())
            .and_then(|store| store.fail(&id, "ANALYSIS_BUSY"));
        return Err("ANALYSIS_BUSY: another analysis is already running".into());
    }
    Ok(id)
}

/// Spawns the Node analysis worker for `job_id`. The worker owns SQLite state updates;
/// this only guards against duplicate workers for the same job.
pub(crate) fn spawn_analysis_worker(
    app: tauri::AppHandle,
    jobs: Jobs,
    job_id: String,
    input: PathBuf,
    duration: f64,
    range_start: f64,
    range_end: f64,
) -> bool {
    if ANALYSIS_RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return false;
    }
    spawn_analysis_worker_reserved(
        app,
        jobs,
        job_id,
        input,
        duration,
        range_start,
        range_end,
        RunningGuard(&ANALYSIS_RUNNING),
    )
}

pub(crate) fn spawn_analysis_worker_reserved(
    app: tauri::AppHandle,
    jobs: Jobs,
    job_id: String,
    input: PathBuf,
    duration: f64,
    range_start: f64,
    range_end: f64,
    slot: RunningGuard,
) -> bool {
    {
        let mut all = match jobs.lock() {
            Ok(all) => all,
            Err(_) => return false,
        };
        all.insert(
            job_id.clone(),
            AnalysisStatus {
                status: "running".into(),
                phase: "probe".into(),
                progress: 0.02,
                result: None,
                error: None,
                chunk_completed: 0,
                chunk_total: 0,
                current_chunk: None,
            },
        );
    }
    thread::spawn(move || {
        let _slot = slot;
        let run = (|| -> Result<String, String> {
            let mut cmd = tool(node_bin(&app));
            if is_remote_input(&input.to_string_lossy()) {
                // YouTube URL: text-only pipeline (captions via crawler), no Whisper chunks.
                let python = venv_python();
                if !python.is_file() {
                    // Local files still work in this state; only YouTube links need
                    // the app venv (yt-dlp + youtube-transcript-api).
                    return Err("PYTHON_VENV_MISSING: YouTube links need the app Python environment — click \"Setup Whisper\" in Settings to prepare it".into());
                }
                cmd.args(NODE_ARGS)
                    .arg(helper_script(&app, "analyze-youtube.ts")?)
                    .arg(&input)
                    .arg(format!("{duration:.3}"))
                    .arg(&job_id)
                    .arg(jobs_path())
                    .env("YT_PYTHON", &python)
                    .env("YT_CRAWLER", helper_script(&app, "yt-crawler.py")?);
            } else {
                let script = helper_script(&app, "analyze-video.ts")?;
                cmd.args(NODE_ARGS)
                    .arg(script)
                    .arg(&input)
                    .arg(input.to_string_lossy().as_ref())
                    .arg(format!("{duration:.3}"))
                    .arg(format!("{range_start:.3}"))
                    .arg(format!("{range_end:.3}"))
                    .arg(&job_id)
                    .arg(jobs_path());
            }
            for (key, value) in helper_env(&app, &[]) {
                cmd.env(key, value);
            }
            // BELOW_NORMAL keeps the webview responsive while whisper/ffmpeg saturate
            // the CPU — every grandchild inherits the parent's priority class. Must go
            // through the group builder: its spawn() REPLACES the Command's creation
            // flags (it starts from zero), so any flags set on cmd itself are lost —
            // including tool()'s CREATE_NO_WINDOW, which is re-added here.
            #[cfg(windows)]
            let group_flags = 0x0800_0000 | 0x0000_4000; // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
            // group_spawn puts the worker in its own process group (Unix) or a Job
            // Object with KILL_ON_JOB_CLOSE (Windows): one kill reaches the whisper
            // and chunk-ffmpeg grandchildren on every platform we ship, and a crashed
            // app can no longer leave an orphaned tree behind.
            use command_group::CommandGroup;
            let mut group = cmd.group();
            #[cfg(windows)]
            group.creation_flags(group_flags);
            let child = group.spawn().map_err(|e| io_error("NODE_SPAWN", e))?;
            let worker_pid = child.id();
            register_analysis_worker(&job_id, worker_pid);
            // The GroupChild owns the Windows Job Object, so it must outlive the
            // worker: wait through it (which also drains both pipes) instead of
            // unwrapping it into a plain Child.
            let waited = child.wait_with_output().map_err(|e| io_error("NODE_WAIT", e));
            unregister_analysis_worker(&job_id);
            let output = waited?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
            }
            Ok(String::from_utf8(output.stdout).map_err(|e| format!("ANALYZE_OUTPUT:{e}"))?)
        })();
        if let Ok(mut all) = jobs.lock() {
            if let Some(job) = all.get_mut(&job_id) {
                if job.status != "cancelled" {
                    match run {
                        Ok(result) => match serde_json::from_str(&result) {
                            Ok(value) => {
                                job.status = "completed".into(); job.phase = "completed".into(); job.progress = 1.0; job.result = Some(value);
                            }
                            Err(error) => { job.status = "failed".into(); job.phase = "failed".into(); job.error = Some(format!("ANALYSIS_RESULT_JSON:{error}")); }
                        },
                        Err(error) => { job.status = "failed".into(); job.phase = "failed".into(); job.error = Some(error); }
                    }
                }
            }
        }
    });
    true
}

/// Resumes durable queued jobs left behind by a crash or restart.
///
/// Mirrors the status script's `--resumable` claim: a running job whose worker pid
/// is gone (or a queued job with no live worker) counts one crash against its
/// budget and is requeued, until the budget (`MAX_JOB_ATTEMPTS`) runs out.
pub(crate) fn resume_queued_jobs(app: tauri::AppHandle, jobs: Jobs) {
    let Ok(store) = AnalysisStore::open(&jobs_path()) else { return };
    let Ok(all) = store.all_jobs() else { return };
    // Claim: bump the crash budget; a job that keeps dying without progress stops
    // being auto-resumed until the user retries it manually.
    let claim = |store: &AnalysisStore, job: &serde_json::Value| -> Option<serde_json::Value> {
        let id = job["id"].as_str()?;
        let attempts = store.bump_attempts(id).ok()?;
        if attempts >= MAX_JOB_ATTEMPTS {
            let _ = store.fail(id, &format!("worker died {attempts} times without progress; retry manually"));
            return None;
        }
        store.mark_worker_lost(id).ok()?;
        store.get(id).ok().flatten()
    };
    let alive = |store: &AnalysisStore, job: &serde_json::Value| -> bool {
        store
            .worker_pid(job["id"].as_str().unwrap_or(""))
            .ok()
            .flatten()
            .map(|pid| process_alive(pid.max(0) as u32))
            .unwrap_or(false)
    };
    for job in all.iter() {
        if job["status"] == "running" && !alive(&store, job) {
            claim(&store, job);
        }
    }
    let Ok(queued) = store.all_jobs() else { return };
    for job in queued.iter().filter(|job| job["status"] == "queued") {
        if alive(&store, job) {
            continue;
        }
        let Some(claimed) = claim(&store, job) else { continue };
        let (Some(id), Some(input)) = (claimed["id"].as_str(), claimed["input"].as_str()) else {
            continue;
        };
        let duration = claimed["duration"].as_f64().unwrap_or(0.0);
        if duration <= 0.0 {
            continue;
        }
        let path = PathBuf::from(input);
        if !is_remote_input(input) && !path.is_file() {
            let _ = store.fail(id, "input file missing");
            continue;
        }
        let range_start = claimed["rangeStart"].as_f64().unwrap_or(0.0);
        let range_end = claimed["rangeEnd"]
            .as_f64()
            .filter(|end| *end > range_start)
            .unwrap_or(duration);
        if !spawn_analysis_worker(
            app.clone(),
            jobs.clone(),
            id.to_string(),
            path,
            duration,
            range_start,
            range_end,
        ) {
            break;
        }
    }
}

#[tauri::command]
pub(crate) fn analysis_status(
    _app: tauri::AppHandle,
    _jobs: tauri::State<'_, Jobs>,
    id: String,
) -> Result<AnalysisStatus, String> {
    read_status(&id)
}

#[tauri::command]
pub(crate) fn list_analysis_jobs(_app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let store = AnalysisStore::open(&jobs_path())?;
    Ok(serde_json::Value::Array(store.all_jobs()?))
}

/// Clears the crash budget and requeues failed chunks, then restarts the worker.
#[tauri::command]
pub(crate) fn retry_analysis(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, Jobs>,
    id: String,
) -> Result<serde_json::Value, String> {
    let file = runtime_env();
    let whisper_model = file.get("WHISPER_MODEL").cloned().unwrap_or_else(|| "tiny".into());
    let whisper_device = file.get("WHISPER_DEVICE").cloned().unwrap_or_else(|| "cpu".into());
    guard_pro_features(&whisper_model, &whisper_device)?;
    if let Ok(all) = jobs.lock() {
        if all.get(&id).is_some_and(|job| job.status == "running") {
            return Err("ANALYSIS_ALREADY_RUNNING: job already has a worker".into());
        }
    } else {
        return Err("ANALYSIS_STATE_LOCK_POISONED".into());
    }
    if ANALYSIS_RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("ANALYSIS_BUSY: another analysis is already running".into());
    }
    let slot = RunningGuard(&ANALYSIS_RUNNING);
    let store = AnalysisStore::open(&jobs_path())?;
    if !store.retry(&id)? {
        return Err("JOB_NOT_RETRYABLE: only failed jobs can be retried".into());
    }
    let job = store.get(&id)?.ok_or("JOB_NOT_FOUND")?;
    let (Some(input), Some(duration)) = (job["input"].as_str(), job["duration"].as_f64()) else {
        return Err("JOB_NOT_FOUND".into());
    };
    let path = PathBuf::from(input);
    if !is_remote_input(input) && !path.is_file() {
        let _ = store.fail(&id, "input file missing");
        return Err("INPUT_NOT_FILE".into());
    }
    let range_start = job["rangeStart"].as_f64().unwrap_or(0.0);
    let range_end = job["rangeEnd"].as_f64().unwrap_or(duration);
    if !spawn_analysis_worker_reserved(
        app.clone(),
        jobs.inner().clone(),
        id.clone(),
        path,
        duration,
        range_start,
        range_end,
        slot,
    ) {
        return Err("ANALYSIS_ALREADY_RUNNING: job already has a worker".into());
    }
    Ok(job)
}

/// Stops a job from being auto-resumed without deleting its transcript cache.
#[tauri::command]
pub(crate) fn abandon_analysis(_app: tauri::AppHandle, id: String) -> Result<serde_json::Value, String> {
    let store = AnalysisStore::open(&jobs_path())?;
    store.fail(&id, "abandoned by user")?;
    Ok(store.get(&id)?.unwrap_or(serde_json::Value::Null))
}

/// Kills the analysis worker for a job and marks it cancelled cleanly (user abort).
#[tauri::command]
pub(crate) fn stop_analysis(_app: tauri::AppHandle, id: String) -> Result<serde_json::Value, String> {
    let store = AnalysisStore::open(&jobs_path())?;
    // Prefer the pid Rust spawned (authoritative); fall back to the worker's own
    // record for a job started by a previous app run.
    let spawned = ANALYSIS_WORKERS.lock().ok().and_then(|guard| guard.get(&id).copied());
    let recorded = store
        .worker_pid(&id)?
        .filter(|pid| *pid > 0)
        .map(|pid| pid as u32);
    for pid in spawned.into_iter().chain(recorded) {
        kill_analysis_worker(pid);
    }
    store.cancel(&id)?;
    job_with_result(&store, &id)?.ok_or_else(|| "JOB_NOT_FOUND".into())
}

/// Stops the detached llama-server and RAG sidecar processes, if running.
/// Terminates an analysis worker and everything it spawned.
///
/// Three prongs on purpose, because each has failed in the field on its own: the
/// negative pid reaches the worker's whole process group (whisper, chunk ffmpeg), the
/// bare pid covers a worker that never became a group leader, and the delayed SIGKILL
/// covers a worker whose signal handler only records the request and keeps running.
pub(crate) fn kill_analysis_worker(pid: u32) {
    #[cfg(unix)]
    {
        // The worker runs in its own process group, so the negative pid reaches the
        // whole tree. TERM first, with a delayed KILL as the enforcement backstop.
        let group = format!("-{pid}");
        let bare = pid.to_string();
        let signal = |name: &str, target: &str| {
            for candidate in ["/usr/bin/kill", "/bin/kill", "kill"] {
                if tool(candidate).args([name, target]).status().is_ok() { return; }
            }
        };
        signal("-TERM", &group);
        signal("-TERM", &bare);
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(3));
            if process_alive(pid) {
                signal("-KILL", &group);
                signal("-KILL", &bare);
            }
        });
    }
    #[cfg(windows)]
    {
        // The worker runs inside a Job Object, but this call site only has the pid:
        // taskkill's /T walks the full child tree (whisper, ffmpeg grandchildren) and
        // /F forces it — there is no graceful TERM for a detached tree on Windows.
        let _ = tool("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

pub(crate) fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    { tool("kill").args(["-0", &pid.to_string()]).status().map(|s| s.success()).unwrap_or(false) }
    #[cfg(windows)]
    {
        // tasklist prints the pid row while the process exists; a filtered empty
        // table means it is gone.
        tool("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
    #[cfg(not(any(unix, windows)))]
    { false }
}

/// Drains the registry on a clean exit: no analysis worker (or its children) outlives
/// the app.
pub(crate) fn stop_analysis_workers() {
    let entries = ANALYSIS_WORKERS
        .lock()
        .map(|mut guard| guard.drain().collect::<Vec<_>>())
        .unwrap_or_default();
    for (_, pid) in entries {
        kill_analysis_worker(pid);
    }
}

/// Registers a worker so cancel/exit can reach it, and returns a guard that removes it.
pub(crate) fn register_analysis_worker(job_id: &str, pid: u32) {
    if let Ok(mut guard) = ANALYSIS_WORKERS.lock() {
        guard.insert(job_id.to_string(), pid);
    }
}

pub(crate) fn unregister_analysis_worker(job_id: &str) {
    if let Ok(mut guard) = ANALYSIS_WORKERS.lock() {
        guard.remove(job_id);
    }
}

