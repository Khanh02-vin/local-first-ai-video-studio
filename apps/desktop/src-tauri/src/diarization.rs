// The diarization and face branch: local speaker separation, face samples for
// speaker mapping, and the crop/scene analysis the Editor previews.
//
// Everything here runs the local models on the machine; nothing leaves it.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths::{
    ffmpeg_bin, helper_env, helper_script, is_remote_input, node_bin, runtime_env, data_dir,
    tool, venv_python, write_runtime_settings,
};
use render_engine::engine::{create_temporary_render_dir, stable_subject_pair, subject_pair_continues, TemporaryRenderDir};
use crate::paths::NODE_ARGS;

/// Asks the AI pipeline to propose ordered transcript ranges as alternative edits.
///
/// The variant generator is a Node subprocess that may call an LLM, so the wait runs
/// on a blocking worker instead of the main thread.
#[tauri::command]
pub(crate) async fn generate_transcript_variants(
    app: tauri::AppHandle,
    transcript: serde_json::Value,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        generate_transcript_variants_blocking(&app, &transcript)
    })
    .await
    .map_err(|error| format!("VARIANT_WORKER:{error}"))?
}

fn generate_transcript_variants_blocking(
    app: &tauri::AppHandle,
    transcript: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let settings = runtime_env();
    let strategy = settings
        .get("HIGHLIGHT_STRATEGY")
        .cloned()
        .unwrap_or_else(|| "heuristic".into());
    if !["semantic-gemini", "semantic-local"].contains(&strategy.as_str()) {
        return Err("VARIANT_AI_STRATEGY_REQUIRED: choose Gemini or local LLM in Settings".into());
    }
    if strategy == "semantic-gemini"
        && settings
            .get("GEMINI_API_KEY")
            .map(|key| key.trim().is_empty())
            .unwrap_or(true)
    {
        return Err("SEMANTIC_REQUIRES_GEMINI_API_KEY".into());
    }
    let file = data_dir().join(format!(
        "transcript-variants-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "CLOCK_ERROR")?
            .as_nanos()
    ));
    std::fs::write(
        &file,
        serde_json::to_vec(transcript).map_err(|e| format!("VARIANT_TRANSCRIPT:{e}"))?,
    )
    .map_err(|e| format!("VARIANT_TRANSCRIPT:{e}"))?;
    let script = match helper_script(app, "generate-variants.ts") {
        Ok(script) => script,
        Err(error) => {
            let _ = std::fs::remove_file(&file);
            return Err(error);
        }
    };
    let mut cmd = tool(node_bin(app));
    cmd.args(NODE_ARGS)
        .arg(script)
        .arg(&file)
        .arg(&strategy)
        .arg(
            settings
                .get("LOCAL_LLM_BASE_URL")
                .map(String::as_str)
                .unwrap_or("http://127.0.0.1:8080"),
        )
        .arg(
            settings
                .get("LOCAL_LLM_MODEL")
                .map(String::as_str)
                .unwrap_or("qwen2.5-3b-instruct"),
        );
    for (key, value) in helper_env(app, &[]) {
        cmd.env(key, value);
    }
    if strategy == "semantic-gemini" {
        cmd.env(
            "GEMINI_API_KEY",
            settings
                .get("GEMINI_API_KEY")
                .map(String::as_str)
                .unwrap_or(""),
        );
    }
    let output = cmd.output();
    let _ = std::fs::remove_file(&file);
    let output = output.map_err(|e| format!("VARIANT_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("VARIANT_JSON:{e}"))
}

#[tauri::command]
pub(crate) fn diarization_setup_status() -> serde_json::Value {
    let python = venv_python();
    let token = runtime_env()
        .get("HF_TOKEN")
        .is_some_and(|value| !value.trim().is_empty());
    let installed = python.is_file()
        && tool(&python)
            .args(["-c", "import pyannote.audio"])
            .status()
            .is_ok_and(|status| status.success());
    serde_json::json!({ "installed": installed, "hasToken": token, "ready": installed && token })
}

#[tauri::command]
pub(crate) fn save_diarization_token(token: String) -> Result<(), String> {
    if token.trim().len() < 10 || token.contains(['\n', '\r']) {
        return Err("INVALID_HF_TOKEN".into());
    }
    let mut settings = runtime_env();
    settings.insert("HF_TOKEN".into(), token.trim().to_string());
    write_runtime_settings(&settings)
}

#[tauri::command]
pub(crate) fn install_diarization_runtime() -> Result<(), String> {
    let python = venv_python();
    if !python.is_file() {
        return Err("PYTHON_VENV_MISSING: run Whisper setup first".into());
    }
    let install = tool(&python)
        .args([
            "-m",
            "pip",
            "install",
            "pyannote.audio==3.3.2",
            "opencv-python-headless==4.10.0.84",
        ])
        .output()
        .map_err(|e| format!("DIARIZATION_INSTALL_SPAWN:{e}"))?;
    if !install.status.success() {
        return Err(format!(
            "DIARIZATION_INSTALL_FAILED:{}",
            String::from_utf8_lossy(&install.stderr)
                .lines()
                .last()
                .unwrap_or("pip failed")
        ));
    }
    Ok(())
}

/// Splits the audio into speaker intervals.
///
/// The model inference takes seconds, so it runs on a blocking worker instead of the
/// main thread, where it would freeze the webview.
#[tauri::command]
pub(crate) async fn diarize_video(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        diarize_video_blocking(&app, &path, duration)
    })
    .await
    .map_err(|error| format!("DIARIZATION_WORKER:{error}"))?
}

fn diarize_video_blocking(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
) -> Result<serde_json::Value, String> {
    if is_remote_input(path) {
        return Err("DIARIZATION_LOCAL_SOURCE_ONLY".into());
    }
    if !duration.is_finite() || duration <= 0.0 || duration > 1800.0 {
        return Err("DIARIZATION_DURATION_LIMIT".into());
    }
    let input = Path::new(&path)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let settings = runtime_env();
    let token = settings
        .get("HF_TOKEN")
        .filter(|value| !value.trim().is_empty())
        .ok_or("DIARIZATION_HF_TOKEN_REQUIRED")?;
    let python = venv_python();
    if !python.is_file() {
        return Err("PYTHON_VENV_MISSING: run Whisper setup first".into());
    }
    if !tool(&python)
        .args(["-c", "import pyannote.audio, cv2"])
        .status()
        .is_ok_and(|status| status.success())
    {
        return Err("DIARIZATION_RUNTIME_MISSING: install local diarization in Settings".into());
    }
    let temp = create_temporary_render_dir()?;
    let audio = temp.0.join("audio.wav");
    let extract = tool(ffmpeg_bin(&app))
        .args(["-y", "-nostdin", "-v", "error", "-i"])
        .arg(&input)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-t"])
        .arg(format!("{duration:.3}"))
        .arg(&audio)
        .status()
        .map_err(|e| format!("DIARIZATION_FFMPEG:{e}"))?;
    if !extract.success() {
        return Err("DIARIZATION_AUDIO_EXTRACT_FAILED".into());
    }
    let audio_bytes = std::fs::metadata(&audio)
        .map_err(|e| format!("DIARIZATION_AUDIO_STAT:{e}"))?
        .len();
    if audio_bytes == 0 || audio_bytes > 600 * 1024 * 1024 {
        return Err("DIARIZATION_AUDIO_SIZE_LIMIT".into());
    }
    let script = helper_script(&app, "diarize-audio.py")?;

    let output = tool(&python)
        .arg(script)
        .arg(&audio)
        .env("HF_TOKEN", token)
        .output()
        .map_err(|e| format!("DIARIZATION_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("DIARIZATION_JSON:{e}"))
}

/// Samples a face thumbnail per speaker so the user can confirm the speaker mapping.
///
/// One frame extraction plus one detection per sample: blocking work, so it runs on a
/// blocking worker rather than the main thread.
#[tauri::command]
pub(crate) async fn sample_speaker_faces(
    app: tauri::AppHandle,
    path: String,
    speakers: serde_json::Value,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        sample_speaker_faces_blocking(&app, &path, &speakers)
    })
    .await
    .map_err(|error| format!("FACE_SAMPLE_WORKER:{error}"))?
}

fn sample_speaker_faces_blocking(
    app: &tauri::AppHandle,
    path: &str,
    speakers: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    if is_remote_input(path) {
        return Err("DIARIZATION_LOCAL_SOURCE_ONLY".into());
    }
    let input = Path::new(&path)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let samples = speakers.as_array().ok_or("FACE_SPEAKERS_INVALID")?;
    if samples.is_empty() || samples.len() > 8 {
        return Err("FACE_SAMPLE_LIMIT".into());
    }
    let python = venv_python();
    if !python.is_file()
        || !tool(&python)
            .args(["-c", "import cv2"])
            .status()
            .is_ok_and(|status| status.success())
    {
        return Err("FACE_RUNTIME_MISSING: install local diarization in Settings".into());
    }
    let temp = create_temporary_render_dir()?;
    let mut output = Vec::new();
    let mut response_bytes = 0usize;
    for (index, sample) in samples.iter().enumerate() {
        let speaker = sample["speakerId"]
            .as_str()
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 64
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
            })
            .ok_or("FACE_SPEAKER_INVALID")?;
        let time = sample["time"]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0 && *value < 1800.0)
            .ok_or("FACE_TIME_INVALID")?;
        let frame = temp.0.join(format!("frame-{index:02}.jpg"));
        let status = tool(ffmpeg_bin(&app))
            .args(["-y", "-nostdin", "-v", "error", "-ss"])
            .arg(format!("{time:.3}"))
            .args(["-i"])
            .arg(&input)
            .args(["-frames:v", "1", "-vf", "scale='min(640,iw)':-2"])
            .arg(&frame)
            .status()
            .map_err(|e| format!("FACE_FRAME_SPAWN:{e}"))?;
        if !status.success() {
            continue;
        }
        let script = helper_script(&app, "detect-faces.py")?;
        let detected = tool(&python)
            .arg(script)
            .arg(&frame)
            .output()
            .map_err(|e| format!("FACE_DETECT_SPAWN:{e}"))?;
        if !detected.status.success() {
            return Err(String::from_utf8_lossy(&detected.stderr).trim().to_owned());
        }
        let faces: Vec<serde_json::Value> = serde_json::from_slice(&detected.stdout)
            .map_err(|e| format!("FACE_DETECT_JSON:{e}"))?;
        for (face_index, face) in faces.into_iter().take(4).enumerate() {
            let thumbnail = face["thumbnail"]
                .as_str()
                .filter(|value| {
                    value.len() <= 350_000
                        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                        && value.len() % 2 == 0
                })
                .unwrap_or("");
            if thumbnail.is_empty() {
                continue;
            }
            response_bytes += thumbnail.len();
            if response_bytes > 10_000_000 {
                return Err("FACE_RESPONSE_SIZE_LIMIT".into());
            }
            output.push(serde_json::json!({ "id": format!("{speaker}-{}-{face_index}", index), "speakerId": speaker, "time": time, "thumbnail": thumbnail }));
        }
    }
    Ok(serde_json::json!(output))
}

/// Traces the speaker's face across the source video.
///
/// The work — one FFmpeg frame extraction plus one face detection per sample — is
/// blocking and takes seconds, so it runs on a blocking worker: on the main thread it
/// would freeze the webview for the whole scan.
#[tauri::command]
pub(crate) async fn analyze_crop_path(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
    speaker_intervals: serde_json::Value,
    face_mapping: serde_json::Value,
    source_artifact_id: String,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
    analyze_crop_path_blocking(&app, &path, duration, &speaker_intervals, &face_mapping, &source_artifact_id)
    })
    .await
    .map_err(|error| format!("CROP_WORKER:{error}"))?
}

fn analyze_crop_path_blocking(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    speaker_intervals: &serde_json::Value,
    face_mapping: &serde_json::Value,
    source_artifact_id: &str,
) -> Result<serde_json::Value, String> {
    if is_remote_input(path) {
        return Err("CROP_LOCAL_SOURCE_ONLY".into());
    }
    if !duration.is_finite() || duration <= 0.0 || duration > 1800.0 {
        return Err("CROP_DURATION_LIMIT".into());
    }
    let intervals = speaker_intervals
        .as_array()
        .ok_or("CROP_SPEAKERS_INVALID")?;
    if intervals.len() > 10_000 {
        return Err("CROP_SPEAKER_LIMIT".into());
    }
    let input = Path::new(&path)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let python = venv_python();
    if !python.is_file() {
        return Err("FACE_RUNTIME_MISSING".into());
    }
    let temp = create_temporary_render_dir()?;
    let frames: Vec<serde_json::Value> = extract_and_detect(app, &input, 5.0, 360, duration, &temp, "crop")?
        .into_iter()
        .map(|(time, faces)| serde_json::json!({"time": time, "faces": faces}))
        .collect();
    let mapping_confirmed = face_mapping["confirmed"].as_bool() == Some(true)
        && face_mapping["sourceArtifactId"].as_str() == Some(source_artifact_id);
    let matches = face_mapping["matches"].as_object();
    let output = track_crop_path(&frames, intervals, matches, mapping_confirmed, source_artifact_id);
    Ok(serde_json::json!(output))
}

/// The timestamps a batched extraction produces: one frame every `interval`
/// seconds from zero, capped by the extraction itself, never at/after `duration`.
fn sample_times(count: usize, interval: f64, duration: f64) -> Vec<f64> {
    (0..count)
        .map(|index| index as f64 * interval)
        .take_while(|time| *time < duration)
        .collect()
}

/// One FFmpeg pass writes every sampled frame; one Python call detects faces in
/// all of them. Replaces the per-sample spawn loop — 720 process spawns for a
/// 30-minute video became two; the Python + cv2 import cost (~1s) is paid once
/// instead of once per sample.
///
/// An unreadable frame comes back as `None` from the detector and is skipped
/// here rather than killing the scan. A failed extraction batch returns an empty
/// result, matching the old per-sample loop's "skip what failed" behaviour.
fn extract_and_detect(
    app: &tauri::AppHandle,
    input: &Path,
    interval: f64,
    max_frames: usize,
    duration: f64,
    temp: &TemporaryRenderDir,
    prefix: &str,
) -> Result<Vec<(f64, Vec<serde_json::Value>)>, String> {
    let python = venv_python();
    if !python.is_file() {
        return Err("FACE_RUNTIME_MISSING".into());
    }
    let script = helper_script(app, "detect-face-boxes.py")?;
    let pattern = temp.0.join(format!("{prefix}-%04d.jpg"));
    // One sequential decode instead of hundreds of seeks: comparable CPU time,
    // but a single process with a warm decoder instead of a spawn storm.
    let status = tool(ffmpeg_bin(app))
        .args(["-y", "-nostdin", "-v", "error", "-i"])
        .arg(input)
        .args([
            "-vf",
            &format!("fps=1/{interval},scale='min(640,iw)':-2"),
            "-frames:v",
            &max_frames.to_string(),
        ])
        .arg(&pattern)
        .status()
        .map_err(|e| format!("FRAME_SPAWN:{e}"))?;
    if !status.success() {
        return Ok(Vec::new());
    }
    let mut frame_paths: Vec<PathBuf> = std::fs::read_dir(&temp.0)
        .map_err(|e| format!("FRAME_LIST:{e}"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "jpg")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    frame_paths.sort();
    if frame_paths.is_empty() {
        return Ok(Vec::new());
    }
    let output = tool(&python)
        .arg(&script)
        .args(&frame_paths)
        .output()
        .map_err(|e| format!("DETECT_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let detected: Vec<Option<Vec<serde_json::Value>>> =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("DETECT_JSON:{e}"))?;
    let mut frames = Vec::new();
    for (index, time) in sample_times(frame_paths.len(), interval, duration).into_iter().enumerate() {
        if let Some(Some(faces)) = detected.get(index) {
            frames.push((time, faces.clone()));
        }
    }
    Ok(frames)
}

/// Tracks the speaker's face across frames and returns the smoothed crop path.
///
/// This is the one source of truth for the crop path (ADR-003). The editor maps this
/// output onto contract keyframes; nothing smooths or re-tracks it a second time.
///
/// A frame with no usable face is skipped rather than filled in: the path must only
/// contain positions that were actually observed.
pub(crate) fn track_crop_path(
    frames: &[serde_json::Value],
    intervals: &[serde_json::Value],
    matches: Option<&serde_json::Map<String, serde_json::Value>>,
    mapping_confirmed: bool,
    source_artifact_id: &str,
) -> Vec<serde_json::Value> {
    let mut output = Vec::new();
    let mut scene_id = 0usize;
    let mut previous_selected: Option<(f64, f64)> = None;
    let mut previous_time = 0.0;
    for frame in frames {
        let at = frame["time"].as_f64().unwrap_or(0.0);
        let active = intervals.iter().find(|interval| {
            interval["start"].as_f64().is_some_and(|start| start <= at)
                && interval["end"].as_f64().is_some_and(|end| at < end)
        });
        let speaker = active
            .and_then(|value| value["speakerId"].as_str())
            .unwrap_or("");
        let faces = frame["faces"].as_array().cloned().unwrap_or_default();
        let mapped_index = if mapping_confirmed && !speaker.is_empty() {
            matches
                .and_then(|map| map.get(speaker))
                .and_then(serde_json::Value::as_str)
                .and_then(|id| id.rsplit('-').next())
                .and_then(|value| value.parse::<usize>().ok())
        } else {
            None
        };
        let selected = mapped_index
            .filter(|index| *index < faces.len())
            .and_then(|index| faces.get(index).map(|face| (index, face)));
        let Some((index, face)) = selected else {
            continue;
        };
        let confidence = face["confidence"].as_f64().unwrap_or(0.0);
        let x = face["x"].as_f64().unwrap_or(0.5) + face["width"].as_f64().unwrap_or(0.0) / 2.0;
        let y = face["y"].as_f64().unwrap_or(0.5) + face["height"].as_f64().unwrap_or(0.0) / 2.0;
        if previous_selected.is_some_and(|(px, py)| (x - px).hypot(y - py) >= 0.35)
            || (previous_time > 0.0 && at - previous_time > 10.0)
        {
            scene_id += 1;
        }
        let (smooth_x, smooth_y) = previous_selected
            .map(|(px, py)| (px + 0.35 * (x - px), py + 0.35 * (y - py)))
            .unwrap_or((x, y));
        previous_selected = Some((smooth_x, smooth_y));
        previous_time = at;
        output.push(serde_json::json!({"time": at, "centerX": smooth_x, "centerY": smooth_y, "speakerId": if speaker.is_empty() { serde_json::Value::Null } else { serde_json::json!(speaker) }, "faceIndex": index, "confidence": confidence, "sourceArtifactId": source_artifact_id, "sceneId": scene_id}));
    }
    output
}

/// Finds the scene intervals (solo, side-by-side) in the source video.
///
/// The scan extracts and analyses a frame every two seconds, so it runs on a blocking
/// worker: on the main thread it would freeze the webview for the whole scan.
#[tauri::command]
pub(crate) async fn analyze_scene_layouts(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
    source_artifact_id: String,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        analyze_scene_layouts_blocking(&app, &path, duration, &source_artifact_id)
    })
    .await
    .map_err(|error| format!("SCENE_WORKER:{error}"))?
}

fn analyze_scene_layouts_blocking(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    source_artifact_id: &str,
) -> Result<serde_json::Value, String> {
    if is_remote_input(path) {
        return Err("SCENE_LOCAL_SOURCE_ONLY".into());
    }
    if !duration.is_finite() || duration <= 0.0 || duration > 1800.0 {
        return Err("SCENE_DURATION_LIMIT".into());
    }
    let input = Path::new(&path)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !input.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let temp = create_temporary_render_dir()?;
    let mut scenes = Vec::new();
    let mut start = 0.0;
    let mut samples = Vec::<Vec<serde_json::Value>>::new();
    let mut previous_count: Option<usize> = None;
    // 1800s / 2s = 900 possible samples; the old loop stopped at 360 *scenes*,
    // which the loop below still honours.
    for (time, faces) in extract_and_detect(app, &input, 2.0, 900, duration, &temp, "scene")? {
        if scenes.len() >= 360 {
            break;
        }
        let count = faces.len();
        let stable = count == 2
            && samples
                .last()
                .is_some_and(|previous| subject_pair_continues(previous, &faces));
        if (previous_count.is_some_and(|previous| previous != count)
            || (count == 2 && !samples.is_empty() && !stable))
            && time - start >= 2.0
        {
            let pair = stable_subject_pair(&samples);
            scenes.push(serde_json::json!({"start": start, "end": time, "faceCount": previous_count.unwrap_or(0), "layout": if pair.is_some() { "side-by-side" } else { "solo" }, "confidence": if pair.is_some() { 0.65 } else { 0.4 }, "confirmed": false, "sourceArtifactId": source_artifact_id, "subjectBoxes": pair}));
            start = time;
            samples.clear();
        }
        samples.push(faces);
        previous_count = Some(count);
    }
    let pair = stable_subject_pair(&samples);
    scenes.push(serde_json::json!({"start": start, "end": duration, "faceCount": previous_count.unwrap_or(0), "layout": if pair.is_some() { "side-by-side" } else { "solo" }, "confidence": if pair.is_some() { 0.65 } else { 0.4 }, "confirmed": false, "sourceArtifactId": source_artifact_id, "subjectBoxes": pair}));
    Ok(serde_json::json!(scenes))
}


#[cfg(test)]
mod tests {
    use super::{sample_times, track_crop_path};

    /// The batched extraction's timestamps: one frame every `interval` seconds,
    /// never at or past the end of the video.
    #[test]
    fn sample_times_are_interval_spaced_and_clamped_to_duration() {
        assert_eq!(sample_times(3, 5.0, 12.0), vec![0.0, 5.0, 10.0]);
        // A trailing frame at exactly the duration is not a sample.
        assert_eq!(sample_times(3, 5.0, 10.0), vec![0.0, 5.0]);
        // Zero-frame edge: no samples, no panic.
        assert!(sample_times(0, 5.0, 60.0).is_empty());
    }

    fn face(x: f64, y: f64, width: f64, height: f64, confidence: f64) -> serde_json::Value {
        serde_json::json!({"x": x, "y": y, "width": width, "height": height, "confidence": confidence})
    }

    fn frame(time: f64, faces: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({"time": time, "faces": faces})
    }

    /// The speaker intervals and the confirmed face mapping a tracking run needs.
    fn tracking_setup() -> (Vec<serde_json::Value>, serde_json::Map<String, serde_json::Value>) {
        let intervals = vec![serde_json::json!({"start": 0.0, "end": 600.0, "speakerId": "A"})];
        let mut matches = serde_json::Map::new();
        // The face id's trailing number is the index of the face within a frame.
        matches.insert("A".to_string(), serde_json::json!("face-0"));
        (intervals, matches)
    }

    /// A frame where the tracker cannot see the speaker's face must be skipped, not
    /// filled in: the crop path may only contain positions that were actually observed
    /// (ADR-003).
    #[test]
    fn crop_path_skips_frames_without_a_usable_face() {
        let (intervals, matches) = tracking_setup();
        let frames = vec![
            frame(0.0, vec![face(0.4, 0.4, 0.2, 0.2, 0.9)]),
            frame(5.0, vec![]),
            frame(10.0, vec![face(0.4, 0.4, 0.2, 0.2, 0.9)]),
        ];
        let path = track_crop_path(&frames, &intervals, Some(&matches), true, "src");
        assert_eq!(path.len(), 2, "a frame with no face must not produce a keyframe");
        assert_eq!(path[0]["time"], serde_json::json!(0.0));
        assert_eq!(path[1]["time"], serde_json::json!(10.0));
    }

    /// The tracker smooths the path once. The editor used to smooth it a second time,
    /// which pulled the crop away from the subject — the renderer follows the plan, so
    /// the plan has to carry the position the tracker computed.
    #[test]
    fn crop_path_smooths_once_and_keeps_the_tracked_position() {
        let (intervals, matches) = tracking_setup();
        // The subject jumps from x=0.2 to x=0.8 between two frames.
        let frames = vec![
            frame(0.0, vec![face(0.15, 0.4, 0.1, 0.1, 0.9)]),
            frame(5.0, vec![face(0.75, 0.4, 0.1, 0.1, 0.9)]),
        ];
        let path = track_crop_path(&frames, &intervals, Some(&matches), true, "src");
        assert_eq!(path.len(), 2);
        // The first point is unsmoothed (nothing to smooth against).
        assert!((path[0]["centerX"].as_f64().unwrap() - 0.2).abs() < 1e-9);
        // The second point is smoothed exactly once towards the new position:
        // 0.2 + 0.35 * (0.8 - 0.2) = 0.41. A second smoothing pass would give 0.2735.
        let second = path[1]["centerX"].as_f64().unwrap();
        assert!((second - 0.41).abs() < 1e-9, "expected one smoothing pass, got {second}");
        assert!(
            second > 0.4,
            "the crop must keep following the subject, not lag behind it"
        );
    }

    /// A subject that moves far enough is a new scene: the renderer cuts the clip there.
    #[test]
    fn crop_path_marks_a_new_scene_when_the_subject_moves() {
        let (intervals, matches) = tracking_setup();
        let frames = vec![
            frame(0.0, vec![face(0.15, 0.4, 0.1, 0.1, 0.9)]),
            frame(5.0, vec![face(0.15, 0.4, 0.1, 0.1, 0.9)]),
            frame(10.0, vec![face(0.85, 0.4, 0.1, 0.1, 0.9)]),
        ];
        let path = track_crop_path(&frames, &intervals, Some(&matches), true, "src");
        assert_eq!(path[0]["sceneId"], serde_json::json!(0));
        assert_eq!(path[1]["sceneId"], serde_json::json!(0));
        assert_eq!(path[2]["sceneId"], serde_json::json!(1), "a subject that jumps is a new scene");
    }

    /// Without a confirmed speaker→face mapping the tracker has no way to pick a face,
    /// so it must not invent a crop.
    #[test]
    fn crop_path_needs_a_confirmed_mapping_to_track_a_speaker() {
        let (intervals, matches) = tracking_setup();
        let frames = vec![frame(0.0, vec![face(0.4, 0.4, 0.2, 0.2, 0.9)])];

        let unconfirmed = track_crop_path(&frames, &intervals, Some(&matches), false, "src");
        assert!(unconfirmed.is_empty(), "an unconfirmed mapping must not produce a crop");

        let confirmed = track_crop_path(&frames, &intervals, Some(&matches), true, "src");
        assert_eq!(confirmed.len(), 1);
        assert_eq!(confirmed[0]["speakerId"], serde_json::json!("A"));
        assert_eq!(confirmed[0]["faceIndex"], serde_json::json!(0));
    }
}
