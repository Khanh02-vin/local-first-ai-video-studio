// The app-side render commands: a thin bridge between the Tauri IPC and the render
// engine.
//
// The engine (crates/render-engine) owns every FFmpeg argument; this file only resolves
// paths, owns the "one render at a time" guard, and forwards progress to the webview
// channel. The headless CLI (crates/lfai-render) is the second front door to the same
// engine, so there is exactly one final renderer.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::analysis::RunningGuard;
use crate::paths::{app_render_temp_dir, data_dir, ffmpeg_bin, ffprobe_bin, is_remote_input, render_progress_log};
use crate::project_store::{save};
use crate::youtube::resolve_stream_url;
use render_engine::contracts::{RenderProgress, RenderRequest};
use render_engine::engine::{render_segments};
use serde_json::Value;

/// Set while a render is being asked to stop. This is the engine's own flag: the app
/// and the CLI share it, so a cancel reaches whichever renderer is running.
pub(crate) use render_engine::engine::RENDER_CANCELLED;

/// One render at a time: the app and the CLI share the engine's own cancel flag, so a
/// second render must not start while one is in flight.
pub(crate) static RENDER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Forwards engine progress to the webview channel.
struct ChannelSink<'a>(&'a tauri::ipc::Channel<RenderProgress>);

impl render_engine::engine::ProgressSink for ChannelSink<'_> {
    fn report(&self, progress: RenderProgress) {
        let _ = self.0.send(progress);
    }
}

#[tauri::command]
pub(crate) async fn render_video(
    app: tauri::AppHandle,
    request: RenderRequest,
    on_progress: tauri::ipc::Channel<RenderProgress>,
) -> Result<String, String> {
    let ranges: Vec<(f64, f64)> = request
        .plan
        .ranges
        .iter()
        .map(|range| (range.start, range.end))
        .collect();
    tauri::async_runtime::spawn_blocking(move || {
        if RENDER_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("RENDER_BUSY: another render is already running".to_string());
        }
        let _guard = RunningGuard(&RENDER_RUNNING);
        RENDER_CANCELLED.store(false, Ordering::SeqCst);
        let temp_dir = app_render_temp_dir(&app).map_err(|e| format!("APP_TEMP_DIR:{e}"))?;
        let progress_log = render_progress_log(&app).map_err(|e| format!("APP_TEMP_DIR:{e}"))?;
        let result = (|| {
            if request.plan.version == 0
                || request.plan.source_artifact_id.trim().is_empty()
                || !["9:16", "1:1", "16:9"].contains(&request.plan.aspect_ratio.as_str())
            {
                return Err("INVALID_RENDER_PLAN".to_string());
            }
            let mut plan = serde_json::to_value(&request.plan)
                .map_err(|error| format!("RENDER_PLAN_JSON:{error}"))?;
            if let Some(fields) = plan.as_object_mut() {
                for (key, value) in &request.plan.extra {
                    fields.insert(key.clone(), value.clone());
                }
            }
            let output_path = Path::new(&request.output)
                .canonicalize()
                .unwrap_or_else(|_| Path::new(&request.output).to_path_buf());
            let media: Vec<String> = if is_remote_input(&request.input) {
                resolve_stream_url(&app, &request.input)?
            } else {
                let input_path = Path::new(&request.input)
                    .canonicalize()
                    .map_err(|e| format!("INPUT_PATH:{e}"))?;
                if input_path == output_path {
                    return Err("OUTPUT_MUST_DIFFER_FROM_INPUT".into());
                }
                if !input_path.is_file() {
                    return Err("INPUT_NOT_FILE".into());
                }
                vec![input_path.to_string_lossy().into_owned()]
            };
            render_segments(
                &ffmpeg_bin(&app),
                &ffprobe_bin(&app),
                &media,
                &output_path,
                &ranges,
                Some(request.plan.aspect_ratio.clone()),
                Some(&plan),
                &ChannelSink(&on_progress),
                &temp_dir,
                &progress_log,
                request.plan.hardware_acceleration.unwrap_or(false),
            )
        })();
        result
    })
    .await
    .map_err(|error| format!("RENDER_WORKER:{error}"))?
}

#[tauri::command]
pub(crate) fn cancel_render() -> Result<(), String> {
    if !RENDER_RUNNING.load(Ordering::SeqCst) {
        return Err("RENDER_NOT_RUNNING".into());
    }
    RENDER_CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}

/// Startup reconciliation: scans every project in `projects.sqlite` and resets any
/// render record still stuck in `running` (app was killed mid-render) to `interrupted`,
/// so the UI never shows a forever-running render after a crash or force-quit.
#[tauri::command]
pub(crate) fn recover_render_jobs() -> Result<usize, String> {
    let dir = data_dir();
    let mut projects = crate::project_store::list(&dir)?;
    let mut recovered = 0;
    for project in projects.iter_mut() {
        let mut changed = false;
        for render in project.renders.iter_mut() {
            if render.get("status").and_then(|v| v.as_str()) == Some("running") {
                render["status"] = Value::String("interrupted".into());
                render["error"] = Value::String("recovered after restart".into());
                changed = true;
                recovered += 1;
            }
        }
        if changed {
            save(&dir, project.clone())?;
        }
    }
    Ok(recovered)
}
