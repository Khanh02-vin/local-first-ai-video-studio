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
use crate::paths::{
    analysis_range_allowed, helper_env, helper_script, is_remote_input, jobs_path, node_bin,
    probe_duration, read_sqlite_status, run_status_script, runtime_env, tool, venv_python,
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
    let mut init = tool(node_bin(&app));
    init.args(NODE_ARGS)
        .arg(init_script)
        .arg(jobs_path())
        .arg(&id)
        .arg(&input)
        .arg(format!("{duration:.3}"))
        .arg(format!("{range_start:.3}"))
        .arg(format!("{range_end:.3}"));
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
        let _ = run_status_script(&app, &[id, "--fail".into(), "ANALYSIS_BUSY".into()]);
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
            // group_spawn puts the worker in its own process group (Unix) or a Job
            // Object with KILL_ON_JOB_CLOSE (Windows): one kill reaches the whisper
            // and chunk-ffmpeg grandchildren on every platform we ship, and a crashed
            // app can no longer leave an orphaned tree behind.
            use command_group::CommandGroup;
            let child = cmd.group_spawn().map_err(|e| io_error("NODE_SPAWN", e))?;
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
pub(crate) fn resume_queued_jobs(app: tauri::AppHandle, jobs: Jobs) {
    let resumable = match run_status_script(&app, &["--resumable".to_string()]) {
        Ok(value) => value,
        Err(_) => return,
    };
    let queued = resumable["resumable"].clone();
    let entries = match queued.as_array() {
        Some(entries) => entries.clone(),
        None => return,
    };
    for entry in entries {
        let (Some(id), Some(input)) = (entry["id"].as_str(), entry["input"].as_str()) else {
            continue;
        };
        let duration = entry["duration"].as_f64().unwrap_or(0.0);
        if duration <= 0.0 {
            continue;
        }
        let path = PathBuf::from(input);
        if !is_remote_input(input) && !path.is_file() {
            let _ = run_status_script(
                &app,
                &[
                    id.to_string(),
                    "--fail".to_string(),
                    "input file missing".to_string(),
                ],
            );
            continue;
        }
        let range_start = entry["rangeStart"].as_f64().unwrap_or(0.0);
        let range_end = entry["rangeEnd"]
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
    app: tauri::AppHandle,
    _jobs: tauri::State<'_, Jobs>,
    id: String,
) -> Result<AnalysisStatus, String> {
    read_sqlite_status(&app, &id)
}

#[tauri::command]
pub(crate) fn list_analysis_jobs(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    run_status_script(&app, &["--active".to_string()])
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
    let job = run_status_script(&app, &[id.clone(), "--retry".to_string()])?;
    let (Some(input), Some(duration)) = (job["input"].as_str(), job["duration"].as_f64()) else {
        return Err("JOB_NOT_FOUND".into());
    };
    let path = PathBuf::from(input);
    if !is_remote_input(input) && !path.is_file() {
        let _ = run_status_script(
            &app,
            &[
                id.clone(),
                "--fail".to_string(),
                "input file missing".to_string(),
            ],
        );
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
pub(crate) fn abandon_analysis(app: tauri::AppHandle, id: String) -> Result<serde_json::Value, String> {
    run_status_script(
        &app,
        &[id, "--fail".to_string(), "abandoned by user".to_string()],
    )
}

/// Kills the analysis worker for a job and marks it failed cleanly (user abort).
#[tauri::command]
pub(crate) fn stop_analysis(app: tauri::AppHandle, id: String) -> Result<serde_json::Value, String> {
    let status = run_status_script(&app, &[id.clone()])?;
    run_status_script(&app, &[id.clone(), "--cancel".to_string()])?;
    // Prefer the pid Rust spawned (authoritative); fall back to the worker's own
    // record for a job started by a previous app run.
    let spawned = ANALYSIS_WORKERS.lock().ok().and_then(|guard| guard.get(&id).copied());
    let recorded = status["workerPid"].as_i64().filter(|pid| *pid > 0).map(|pid| pid as u32);
    for pid in spawned.into_iter().chain(recorded) {
        kill_analysis_worker(pid);
    }
    run_status_script(&app, &[id])
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

