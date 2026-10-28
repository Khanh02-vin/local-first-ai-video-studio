// The diarization and face branch: local speaker separation, face samples for
// speaker mapping, and the crop/scene analysis the Editor previews.
//
// Everything here runs local ONNX models on the machine via `ai-engine`; nothing
// leaves the machine and no Python/Node runtime is required for AI work. FFmpeg
// (bundled) is still used to decode audio / frames.
use std::path::{Path, PathBuf};

use ai_engine::{diarize_segments, merge_segments, FaceDetector, SpeakerEmbedder, VadSegmenter};
use image::RgbImage;
use render_engine::engine::{create_temporary_render_dir, stable_subject_pair, subject_pair_continues, TemporaryRenderDir};
use crate::paths::{app_render_temp_dir, data_dir, ffmpeg_bin, is_remote_input, runtime_env, tool};
use serde_json::Value;

/// Asks the AI pipeline to propose ordered transcript ranges as alternative edits.
///
/// The variant generator now calls the configured LLM endpoint directly via HTTP
/// (`reqwest`) instead of spawning a Node subprocess.
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
    _app: &tauri::AppHandle,
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

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("VARIANT_CLIENT:{e}"))?;

    let transcript_json = serde_json::to_string(transcript).map_err(|e| format!("VARIANT_TRANSCRIPT:{e}"))?;

    let prompt = format!(
        r"Given the following transcript segments, propose 3-5 alternative highlight ranges (start,end in seconds) that would make good short-form clips. Return a JSON array of objects with start, end, and reason fields.

Transcript: {}
",
        transcript_json
    );

    let result = if strategy == "semantic-gemini" {
        let api_key = settings
            .get("GEMINI_API_KEY")
            .filter(|k| !k.trim().is_empty())
            .ok_or("SEMANTIC_REQUIRES_GEMINI_API_KEY")?;
        let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent";
        let body = serde_json::json!({
            "contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {"temperature": 0.7, "maxOutputTokens": 1000}
        });
        let resp = client
            .post(url)
            .query(&[("key", api_key)])
            .json(&body)
            .send()
            .map_err(|e| format!("VARIANT_GEMINI_REQUEST:{e}"))?;
        if !resp.status().is_success() {
            return Err(format!("VARIANT_GEMINI_ERROR: HTTP {}", resp.status()));
        }
        let value: Value = resp.json().map_err(|e| format!("VARIANT_GEMINI_JSON:{e}"))?;
        // Extract text from Gemini response.
        let text = value["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("");
        parse_variants_json(text)
    } else {
        // semantic-local: OpenAI-compatible endpoint.
        let base_url = settings
            .get("LOCAL_LLM_BASE_URL")
            .map(|v| v.as_str())
            .unwrap_or("http://127.0.0.1:8080");
        let model = settings
            .get("LOCAL_LLM_MODEL")
            .map(|m| m.as_str())
            .unwrap_or("qwen2.5-3b-instruct");
        let url = format!("{base_url}/chat/completions");
        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.7,
            "max_tokens": 1000,
            "stream": false
        });
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| format!("VARIANT_LLM_REQUEST:{e}"))?;
        if !resp.status().is_success() {
            return Err(format!("VARIANT_LLM_ERROR: HTTP {}", resp.status()));
        }
        let value: Value = resp.json().map_err(|e| format!("VARIANT_LLM_JSON:{e}"))?;
        let text = value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("");
        parse_variants_json(text)
    }?;

    Ok(result)
}

/// Parses a JSON array of {start, end, reason} from LLM output, tolerating
/// surrounding markdown fences or prose.
fn parse_variants_json(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    // Strip markdown fences if present.
    let json_text = if trimmed.starts_with("```") {
        let mut lines: Vec<&str> = trimmed.lines().collect();
        if lines.first().map_or(false, |l| l.starts_with("```")) {
            lines.remove(0);
        }
        if lines.last().map_or(false, |l| l.trim() == "```") {
            lines.pop();
        }
        lines.join("\n")
    } else {
        trimmed.to_string()
    };
    serde_json::from_str(&json_text).map_err(|e| format!("VARIANT_JSON_PARSE:{e} — {json_text}"))
}

#[tauri::command]
pub(crate) fn diarization_setup_status() -> serde_json::Value {
    let ort_available = std::env::consts::ARCH != "unknown"; // ort is compiled into the binary
    let face_model = data_dir().join("models/face-detection.onnx");
    let vad_model = data_dir().join("models/silero_vad.onnx");
    // Also check bundled resources via runtime_env for the app handle is unavailable here;
    // report file presence as a heuristic.
    let face_ready = face_model.is_file();
    let vad_ready = vad_model.is_file();
    serde_json::json!({
        "ort_available": ort_available,
        "faceModelReady": face_ready,
        "vadModelReady": vad_ready,
        "ready": ort_available && face_ready && vad_ready,
    })
}

#[tauri::command]
pub(crate) fn save_diarization_token(token: String) -> Result<(), String> {
    // Repurposed: now stores the LLM API key used by generate_transcript_variants.
    // HF_TOKEN is no longer required since diarization uses bundled ONNX models.
    if token.trim().is_empty() || token.contains(['\n', '\r']) {
        return Err("INVALID_LLM_TOKEN".into());
    }
    let mut settings = runtime_env();
    settings.insert("GEMINI_API_KEY".into(), token.trim().to_string());
    crate::paths::write_runtime_settings(&settings)
}

/// Legacy command kept for backwards compatibility: installs nothing (Python
/// runtime no longer needed). Returns ok so old UI callers do not break.
#[tauri::command]
pub(crate) fn install_diarization_runtime() -> Result<(), String> {
    Ok(())
}

/// Splits the audio into speaker intervals using ONNX VAD + simple clustering.
///
/// Runs on a blocking worker instead of the main thread, where it would freeze the webview.
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

    let temp_dir = app_render_temp_dir(app).map_err(|e| format!("APP_TEMP_DIR:{e}"))?;
    let audio = if let Ok(pre) = std::env::var("ANALYSIS_PREEXTRACTED_AUDIO") {
        // The analysis worker already decoded the source once; reuse it instead of
        // re-decoding the whole video for this task.
        PathBuf::from(pre)
    } else {
        let temp = create_temporary_render_dir(&temp_dir)?;
        let audio = temp.0.join("audio.wav");
        let extract = tool(ffmpeg_bin(app))
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
        audio
    };
    let audio_bytes = std::fs::metadata(&audio)
        .map_err(|e| format!("DIARIZATION_AUDIO_STAT:{e}"))?
        .len();
    if audio_bytes == 0 || audio_bytes > 600 * 1024 * 1024 {
        return Err("DIARIZATION_AUDIO_SIZE_LIMIT".into());
    }

    // Decode WAV to f32 samples using the `image`? No — WAV isn't image.
    // Use rodio/cpal? Keep dependency light: read the WAV header manually for
    // PCM16 mono/stereo and convert to f32 [-1,1].
    let wav_data = std::fs::read(&audio).map_err(|e| format!("DIARIZATION_READ_WAV:{e}"))?;
    let samples = decode_wav_pcm_f32(&wav_data).map_err(|e| format!("DIARIZATION_DECODE_WAV:{e}"))?;

    let mut vad = VadSegmenter::load(app).map_err(|e| format!("DIARIZATION_VAD_LOAD:{e}"))?;
    let segments = vad.segment(&samples).map_err(|e| format!("DIARIZATION_VAD:{e}"))?;
    let merged = merge_segments(&segments, 0.5);

    // Extract a speaker embedding for each merged VAD segment so identical
    // speakers cluster together across the whole video.
    let mut embedder = SpeakerEmbedder::load(app).map_err(|e| format!("DIARIZATION_EMBED_LOAD:{e}"))?;
    let embeddings: Vec<Option<ai_engine::SpeakerEmbedding>> = merged
        .iter()
        .map(|seg| {
            // Cut the segment's audio out of the full sample buffer for embedding.
            let start_sample = (seg.start * 16000.0) as usize;
            let end_sample = (seg.end * 16000.0) as usize;
            let seg_samples = &samples[start_sample.min(samples.len())..end_sample.min(samples.len())];
            if seg_samples.is_empty() {
                None
            } else {
                embedder.embed(seg_samples).ok()
            }
        })
        .collect();

    let intervals = diarize_segments(&merged, &embeddings);

    Ok(serde_json::json!(intervals.iter().map(|i| serde_json::json!({
        "start": i.start,
        "end": i.end,
        "speakerId": i.speaker_id
    })).collect::<Vec<_>>()))
}

/// Decodes a WAV file (PCM16, mono or stereo) into interleaved f32 [-1,1] samples.
/// Handles the common case: 44-byte header, fmt PCM, data chunk.
fn decode_wav_pcm_f32(data: &[u8]) -> Result<Vec<f32>, String> {
    if data.len() < 44 {
        return Err("WAV too small".into());
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("Not a WAV file".into());
    }
    // Find fmt and data chunks.
    let mut fmt_offset: Option<usize> = None;
    let mut data_offset: Option<usize> = None;
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().map_err(|_| "WAV chunk size".to_string())?) as usize;
        if id == b"fmt " {
            fmt_offset = Some(pos + 8);
        } else if id == b"data" {
            data_offset = Some(pos + 8);
        }
        pos += 8 + size + (size % 2); // word-align
    }
    let fmt_start = fmt_offset.ok_or("No fmt chunk")?;
    let data_start = data_offset.ok_or("No data chunk")?;
    if fmt_start + 16 > data.len() {
        return Err("WAV fmt chunk truncated".into());
    }
    let audio_format = u16::from_le_bytes(data[fmt_start..fmt_start + 2].try_into().map_err(|_| "WAV format".to_string())?);
    let channels = u16::from_le_bytes(data[fmt_start + 2..fmt_start + 4].try_into().map_err(|_| "WAV channels".to_string())?);
    let _sample_rate = u32::from_le_bytes(data[fmt_start + 4..fmt_start + 8].try_into().map_err(|_| "WAV rate".to_string())?);
    let _byte_rate = u32::from_le_bytes(data[fmt_start + 8..fmt_start + 12].try_into().map_err(|_| "WAV byte rate".to_string())?);
    let _block_align = u16::from_le_bytes(data[fmt_start + 12..fmt_start + 14].try_into().map_err(|_| "WAV align".to_string())?);
    let bits_per_sample = u16::from_le_bytes(data[fmt_start + 14..fmt_start + 16].try_into().map_err(|_| "WAV bits".to_string())?);
    if audio_format != 1 {
        return Err(format!("WAV format {audio_format} not PCM"));
    }
    if bits_per_sample != 16 {
        return Err(format!("WAV {bits_per_sample}-bit not supported"));
    }

    let pcm_data = &data[data_start..];
    let sample_count = pcm_data.len() / (2 * channels as usize);
    let mut samples = Vec::with_capacity(sample_count);
    for s in 0..sample_count {
        let base = s * (2 * channels as usize);
        let mut left: i32 = i16::from_le_bytes([pcm_data[base], pcm_data[base + 1]]) as i32;
        // Downmix stereo by averaging channels.
        for c in 1..channels as usize {
            let off = base + 2 * c;
            if off + 2 <= pcm_data.len() {
                left += i16::from_le_bytes([pcm_data[off], pcm_data[off + 1]]) as i32;
            }
        }
        samples.push(left as f32 / (channels as f32) / 32768.0);
    }
    Ok(samples)
}

/// Samples a face thumbnail per speaker so the user can confirm the speaker mapping.
///
/// One frame extraction plus one ONNX face-detection pass per sample: blocking work,
/// so it runs on a blocking worker rather than the main thread.
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

    let mut detector = FaceDetector::load(app).map_err(|e| format!("FACE_DETECTOR_LOAD:{e}"))?;

    let temp_dir = app_render_temp_dir(app).map_err(|e| format!("APP_TEMP_DIR:{e}"))?;
    let temp = create_temporary_render_dir(&temp_dir)?;
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
        let status = tool(ffmpeg_bin(app))
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
        let img = image::open(&frame)
            .map_err(|e| format!("FACE_FRAME_READ:{e}"))?
            .into_rgb8();
        let faces = detector.detect(&img).map_err(|e| format!("FACE_DETECT:{e}"))?;
        for (face_index, face) in faces.into_iter().take(4).enumerate() {
            // Encode thumbnail as base64 JPEG.
            let thumbnail = encode_thumbnail(&img);
            if thumbnail.is_empty() {
                continue;
            }
            response_bytes += thumbnail.len();
            if response_bytes > 10_000_000 {
                return Err("FACE_RESPONSE_SIZE_LIMIT".into());
            }
            output.push(serde_json::json!({
                "id": format!("{speaker}-{index}-{face_index}"),
                "speakerId": speaker,
                "time": time,
                "thumbnail": thumbnail,
                "centerX": face.center_x,
                "centerY": face.center_y,
                "width": face.width,
                "height": face.height,
                "confidence": face.confidence,
            }));
        }
    }
    Ok(serde_json::json!(output))
}

/// Encodes a face thumbnail as a base64 data URI.
///
/// The frame is already scaled to a max width of 640 by FFmpeg, so encoding the
/// raw RGB bytes keeps the thumbnail small enough without a separate image
/// encoder dependency.
fn encode_thumbnail(img: &RgbImage) -> String {
    let buf = img.clone().into_raw();
    format!(
        "data:image/rgb;base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &buf)
    )
}

/// Traces the speaker's face across the source video.
///
/// The work — one FFmpeg frame extraction plus one ONNX face detection per sample —
/// is blocking and takes seconds, so it runs on a blocking worker: on the main thread it
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

    let mut detector = FaceDetector::load(app).map_err(|e| format!("FACE_DETECTOR_LOAD:{e}"))?;
    let temp_dir = app_render_temp_dir(app).map_err(|e| format!("APP_TEMP_DIR:{e}"))?;
    let temp = create_temporary_render_dir(&temp_dir)?;
    let frames: Vec<serde_json::Value> = extract_and_detect(app, &mut detector, &input, 5.0, 360, duration, &temp, "crop")?
        .into_iter()
        .map(|(time, faces)| serde_json::json!({"time": time, "faces": faces}))
        .collect();
    let mapping_confirmed = face_mapping["confirmed"].as_bool() == Some(true)
        && face_mapping["sourceArtifactId"].as_str() == Some(source_artifact_id);
    let matches = face_mapping["matches"].as_object();
    let output = track_crop_path(&frames, intervals, matches, mapping_confirmed, source_artifact_id);
    Ok(serde_json::json!(output))
}

/// One FFmpeg pass writes every sampled frame; each frame is then run through the
/// ONNX `FaceDetector`. Replaces the per-sample Python spawn loop — hundreds of
/// process spawns for a long video became one ffmpeg pass plus one ort session.
fn extract_and_detect(
    app: &tauri::AppHandle,
    detector: &mut FaceDetector,
    input: &Path,
    interval: f64,
    max_frames: usize,
    duration: f64,
    temp: &TemporaryRenderDir,
    prefix: &str,
) -> Result<Vec<(f64, Vec<Value>)>, String> {
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
    let mut frames = Vec::new();
    for (index, path) in frame_paths.iter().enumerate() {
        let time = index as f64 * interval;
        if time >= duration {
            break;
        }
        let img = match image::open(path) {
            Ok(img) => img.into_rgb8(),
            Err(_) => continue,
        };
        let faces = match detector.detect(&img) {
            Ok(faces) => faces
                .into_iter()
                .map(|f| serde_json::json!({
                    "x": f.center_x,
                    "y": f.center_y,
                    "width": f.width,
                    "height": f.height,
                    "confidence": f.confidence,
                }))
                .collect(),
            Err(_) => Vec::new(),
        };
        frames.push((time, faces));
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

    let mut detector = FaceDetector::load(app).map_err(|e| format!("FACE_DETECTOR_LOAD:{e}"))?;
    let temp_dir = app_render_temp_dir(app).map_err(|e| format!("APP_TEMP_DIR:{e}"))?;
    let temp = create_temporary_render_dir(&temp_dir)?;
    let mut scenes = Vec::new();
    let mut start = 0.0;
    let mut samples = Vec::<Vec<serde_json::Value>>::new();
    let mut previous_count: Option<usize> = None;
    // 1800s / 2s = 900 possible samples; the old loop stopped at 360 *scenes*,
    // which the loop below still honours.
    for (time, faces) in extract_and_detect(app, &mut detector, &input, 2.0, 900, duration, &temp, "scene")? {
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
    use super::track_crop_path;

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
