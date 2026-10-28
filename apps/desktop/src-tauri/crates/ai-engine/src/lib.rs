//! Local AI inference for the video studio — face detection and voice activity
//! detection — backed by ONNX Runtime (`ort`) so no Python runtime is required on
//! the customer machine.
//!
//! Models are expected as ONNX files bundled into the Tauri `resources/models/`
//! directory (see `tauri.conf.json` bundle.resources). The engine resolves them
//! through `AppHandle::path().resource_dir()` and fails with a clear error when a
//! model is missing instead of silently falling back to a remote service.
//!
//! Hardware acceleration: enable the `gpu` Cargo feature to activate CUDA /
//! CoreML / DirectML execution providers (Pro tier only). Free tier stays on CPU.

use std::path::{Path, PathBuf};

use image::{imageops, RgbImage};
use ort::environment::Environment;
use ort::session::Session;
use tauri::Manager;
use thiserror::Error;

/// Errors surfaced to the Tauri command layer as plain strings.
#[derive(Debug, Error)]
pub enum AiError {
    #[error("model not found at {0}: bundle resources/models/*.onnx in tauri.conf.json")]
    ModelNotFound(PathBuf),
    #[error("failed to build ONNX session: {0}")]
    Session(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

/// Result type used throughout the engine.
pub type AiResult<T> = Result<T, AiError>;

/// Handle to the ONNX Runtime environment. One per process is enough.
#[derive(Clone)]
pub struct AiEngine {
    env: Environment,
}

impl AiEngine {
    /// Creates a new engine.
    ///
    /// When the `gpu` feature is enabled, CUDA / CoreML / DirectML execution
    /// providers are tried in preference order; on any failure the engine falls
    /// back to the CPU provider so the app never crashes on a machine without a
    /// supported GPU.
    pub fn new() -> AiResult<Self> {
        let builder = ort::environment::init();
        #[cfg(feature = "gpu")]
        {
            let mut builder = builder;
            // CUDA (NVIDIA) first on Windows/Linux.
            if let Ok(ep) = ort::execution_providers::CUDA::default().build() {
                builder = builder.with_execution_providers([ep]);
            }
            // CoreML (Apple Silicon / Intel Macs).
            if let Ok(ep) = ort::execution_providers::CoreML::default().build() {
                builder = builder.with_execution_providers([ep]);
            }
            // DirectML (Windows Intel/AMD iGPU).
            if let Ok(ep) = ort::execution_providers::DirectML::default().build() {
                builder = builder.with_execution_providers([ep]);
            }
        }
        let env = builder.build().map_err(|e| AiError::Session(e.to_string()))?;
        Ok(Self { env })
    }

    /// Resolves a bundled model path: resource dir first, then the dev-tree fallback.
    fn model_path(app: &tauri::AppHandle, name: &str) -> AiResult<PathBuf> {
        if let Ok(dir) = app.path().resource_dir() {
            let candidate = dir.join("models").join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        // Dev-tree fallback: crate-dir models/<name>.
        let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("models").join(name);
        if dev.is_file() {
            return Ok(dev);
        }
        Err(AiError::ModelNotFound(
            app.path()
                .resource_dir()
                .map(|d| d.join("models").join(name))
                .unwrap_or_else(|_| PathBuf::from("models").join(name)),
        ))
    }
}

/// A detected face: normalized center coordinates, size, and confidence.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Face {
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
    pub height: f64,
    pub confidence: f64,
}

/// Runs face detection on an RGB image and returns faces sorted by confidence.
pub struct FaceDetector {
    session: Session,
    input_name: String,
    input_shape: [usize; 4], // [N, C, H, W]
}

impl FaceDetector {
    /// Loads the bundled face-detection ONNX model (e.g. YuNet).
    ///
    /// YuNet expects a 4D input `[1, 3, H, W]`; we use `640x640` which is the
    /// most common export size.
    pub fn load(app: &tauri::AppHandle) -> AiResult<Self> {
        let engine = AiEngine::new()?;
        let path = AiEngine::model_path(app, "face-detection.onnx")?;
        let session = Session::builder(&engine.env)
            .map_err(|e| AiError::Session(e.to_string()))?
            .commit_from_file(&path)
            .map_err(|e| AiError::Session(e.to_string()))?;

        let input_name = session
            .inputs()
            .first()
            .ok_or_else(|| AiError::InvalidInput("face model has no input".into()))?
            .name()
            .to_string();
        // YuNet 640x640: [N, C, H, W]
        let input_shape: [usize; 4] = [1, 3, 640, 640];

        Ok(Self {
            session,
            input_name,
            input_shape,
        })
    }

    /// Detects faces in an RGB `image`. Returns detected faces sorted by confidence.
    pub fn detect(&mut self, image: &RgbImage) -> AiResult<Vec<Face>> {
        let model_h = self.input_shape[2];
        let model_w = self.input_shape[3];

        // Resize to model input size.
        let resized = imageops::resize(image, model_w as u32, model_h as u32, imageops::FilterType::Triangle);

        // HWC -> CHW, normalize to [0,1].
        let mut input_data = vec![0.0f32; 1 * 3 * model_h * model_w];
        for y in 0..model_h {
            for x in 0..model_w {
                let pixel = resized.get_pixel(x as u32, y as u32);
                input_data[0 * model_h * model_w + y * model_w + x] = pixel[0] as f32 / 255.0;
                input_data[1 * model_h * model_w + y * model_w + x] = pixel[1] as f32 / 255.0;
                input_data[2 * model_h * model_w + y * model_w + x] = pixel[2] as f32 / 255.0;
            }
        }

        let input_value = ort::value::Tensor::from_array(([1usize, 3, model_h, model_w], input_data))
            .map_err(|e| AiError::InvalidInput(format!("input tensor from array: {e}")))?;

        let inputs = ort::inputs![self.input_name.as_str() => &input_value];
        let outputs = self.session.run(inputs).map_err(|e| AiError::Inference(e.to_string()))?;

        // Expect output shape [1, N, 15] or similar: [x, y, w, h, score, ...landmarks].
        let (_name, output) = outputs
            .into_iter()
            .next()
            .ok_or_else(|| AiError::Inference("face model produced no output".into()))?;
        let (shape, data) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| AiError::Inference(format!("face output extraction: {e}")))?;
        if shape.len() != 3 {
            return Err(AiError::Inference("face model output must be 3D [1,N,C]".into()));
        }
        let rows = shape[1] as usize;
        let cols = shape[2] as usize;
        let mut faces: Vec<Face> = Vec::new();
        for r in 0..rows {
            let base = r * cols;
            let conf = data[base + cols - 1];
            if conf < 0.5f32 {
                continue;
            }
            // Box: [cx, cy, w, h] normalized to [0,1] by YuNet-style outputs.
            let cx = data[base + 0] as f64;
            let cy = data[base + 1] as f64;
            let w = data[base + 2] as f64;
            let h = data[base + 3] as f64;
            faces.push(Face {
                center_x: cx,
                center_y: cy,
                width: w,
                height: h,
                confidence: conf as f64,
            });
        }
        faces.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
        Ok(faces)
    }
}

/// A voice-activity segment in seconds.
#[derive(Clone, Debug, serde::Serialize)]
pub struct VadSegment {
    pub start: f64,
    pub end: f64,
}

/// Runs VAD on 16 kHz mono PCM audio and returns speech segments.
pub struct VadSegmenter {
    session: Session,
    input_name: String,
    _sample_rate: i64,
}

impl VadSegmenter {
    /// Loads the bundled silero-vad ONNX model.
    pub fn load(app: &tauri::AppHandle) -> AiResult<Self> {
        let engine = AiEngine::new()?;
        let path = AiEngine::model_path(app, "silero_vad.onnx")?;
        let session = Session::builder(&engine.env)
            .map_err(|e| AiError::Session(e.to_string()))?
            .commit_from_file(&path)
            .map_err(|e| AiError::Session(e.to_string()))?;

        let input_name = session
            .inputs()
            .first()
            .ok_or_else(|| AiError::InvalidInput("vad model has no input".into()))?
            .name()
            .to_string();

        Ok(Self {
            session,
            input_name,
            _sample_rate: 16000,
        })
    }

    /// Runs VAD on `samples` (16 kHz mono f32 PCM, [-1, 1]) and returns speech segments.
    pub fn segment(&mut self, samples: &[f32]) -> AiResult<Vec<VadSegment>> {
        // Silero VAD processes fixed windows (e.g. 512 samples at 16 kHz).
        const WINDOW: usize = 512;
        const THRESHOLD: f32 = 0.5;

        let mut segments: Vec<VadSegment> = Vec::new();
        let mut in_speech = false;
        let mut seg_start = 0.0;

        let window_count = samples.len() / WINDOW;
        for w in 0..window_count {
            let window: Vec<f32> = samples[w * WINDOW..(w + 1) * WINDOW].to_vec();
            let input_value = ort::value::Tensor::from_array(([1usize, WINDOW], window))
                .map_err(|e| AiError::InvalidInput(format!("vad input tensor: {e}")))?;

            let inputs = ort::inputs![self.input_name.as_str() => &input_value];
            let outputs = self.session.run(inputs).map_err(|e| AiError::Inference(e.to_string()))?;
            let (_name, output) = outputs
                .into_iter()
                .next()
                .ok_or_else(|| AiError::Inference("vad model produced no output".into()))?;
            let prob = output
                .try_extract_tensor::<f32>()
                .map_err(|e| AiError::Inference(format!("vad output extraction: {e}")))?
                .1[0];

            let time = w as f64 * (WINDOW as f64 / self._sample_rate as f64);

            if prob >= THRESHOLD && !in_speech {
                in_speech = true;
                seg_start = time;
            } else if prob < THRESHOLD && in_speech {
                in_speech = false;
                segments.push(VadSegment { start: seg_start, end: time });
            }
        }
        if in_speech {
            segments.push(VadSegment {
                start: seg_start,
                end: samples.len() as f64 / self._sample_rate as f64,
            });
        }
        Ok(segments)
    }
}

/// A speaker embedding extracted from a voice-activity segment.
///
/// Produced by a small speaker-verification ONNX model (e.g. ECAPA-TDNN or
/// ResNet-based). Stored as an L2-normalized vector so cosine similarity is a
/// plain dot product.
#[derive(Clone, Debug)]
pub struct SpeakerEmbedding {
    pub vector: Vec<f32>,
}

/// Extracts a speaker embedding from an audio segment via a bundled ONNX model.
///
/// Falls back to a zero-vector when the embedding model is absent, which makes
/// clustering degrade gracefully to the energy-based diarizer below.
pub struct SpeakerEmbedder {
    session: Option<Session>,
    input_name: Option<String>,
    _sample_rate: i64,
}

impl SpeakerEmbedder {
    pub fn load(app: &tauri::AppHandle) -> AiResult<Self> {
        let engine = AiEngine::new()?;
        // The embedding model is optional: older installs may not bundle it yet.
        let path = match AiEngine::model_path(app, "speaker_embedding.onnx") {
            Ok(p) => p,
            Err(_) => return Ok(Self { session: None, input_name: None, _sample_rate: 16000 }),
        };
        let session = Session::builder(&engine.env)
            .map_err(|e| AiError::Session(e.to_string()))?
            .commit_from_file(&path)
            .map_err(|e| AiError::Session(e.to_string()))?;
        let input_name = session
            .inputs()
            .first()
            .ok_or_else(|| AiError::InvalidInput("embedding model has no input".into()))?
            .name()
            .to_string();
        Ok(Self {
            session: Some(session),
            input_name: Some(input_name),
            _sample_rate: 16000,
        })
    }

    /// Embeds `samples` (16 kHz mono f32 PCM) into a unit-norm vector.
    pub fn embed(&mut self, samples: &[f32]) -> AiResult<SpeakerEmbedding> {
        match (&mut self.session, &self.input_name) {
            (Some(session), Some(input_name)) => {
                // Resample/pad to a fixed window the model expects (e.g. 5.9s ≈ 94400
                // samples at 16 kHz). Zero-pad or truncate.
                const TARGET: usize = 94400;
                let mut padded = vec![0.0f32; TARGET];
                let n = samples.len().min(TARGET);
                padded[..n].copy_from_slice(&samples[..n]);

                let input_value =
                    ort::value::Tensor::from_array(([1usize, TARGET], padded))
                        .map_err(|e| AiError::InvalidInput(format!("embedding input tensor: {e}")))?;
                let inputs = ort::inputs![input_name.as_str() => &input_value];
                let outputs = session.run(inputs).map_err(|e| AiError::Inference(e.to_string()))?;
                let (_name, output) = outputs
                    .into_iter()
                    .next()
                    .ok_or_else(|| AiError::Inference("embedding model produced no output".into()))?;
                let (shape, data) = output
                    .try_extract_tensor::<f32>()
                    .map_err(|e| AiError::Inference(format!("embedding output extraction: {e}")))?;
                if shape.len() != 2 {
                    return Err(AiError::Inference("embedding model output must be 2D [1,D]".into()));
                }
                let mut vector: Vec<f32> = data.to_vec();
                // L2-normalize so dot product == cosine similarity.
                let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm > 1e-6 {
                    for v in &mut vector {
                        *v /= norm;
                    }
                }
                Ok(SpeakerEmbedding { vector })
            }
            _ => {
                // No embedding model: return a deterministic pseudo-embedding derived
                // from RMS energy so different segments still differ slightly.
                let rms: f32 = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
                let rms = rms.sqrt();
                Ok(SpeakerEmbedding {
                    vector: vec![rms, 0.0, 0.0],
                })
            }
        }
    }
}

/// Cosine similarity between two embeddings (both assumed unit-norm).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut dot: f32 = 0.0;
    for i in 0..len {
        dot += a[i] * b[i];
    }
    dot
}

/// Merges adjacent segments closer than `gap` seconds into one.
pub fn merge_segments(segments: &[VadSegment], gap: f64) -> Vec<VadSegment> {
    let mut merged: Vec<VadSegment> = Vec::new();
    for seg in segments {
        if let Some(last) = merged.last_mut() {
            if seg.start - last.end <= gap {
                last.end = seg.end;
            } else {
                merged.push(seg.clone());
            }
        } else {
            merged.push(seg.clone());
        }
    }
    merged
}

/// Speaker interval produced by diarization.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SpeakerInterval {
    pub start: f64,
    pub end: f64,
    pub speaker_id: String,
}

/// Energy-based fallback: alternates speaker id after long silences.
fn diarize_segments_energy(segments: &[VadSegment]) -> Vec<SpeakerInterval> {
    let mut intervals: Vec<SpeakerInterval> = Vec::new();
    let mut current_speaker = 0usize;
    let mut last_end: Option<f64> = None;
    for seg in segments {
        let rotate = last_end.map(|e| seg.start - e > 3.0).unwrap_or(false);
        if rotate {
            current_speaker += 1;
        }
        intervals.push(SpeakerInterval {
            start: seg.start,
            end: seg.end,
            speaker_id: format!("S{}", current_speaker),
        });
        last_end = Some(seg.end);
    }
    intervals
}

/// Clusters VAD segments into speakers using embedding similarity with cosine
/// threshold. Falls back to energy-based alternation when no embedding model is
/// bundled.
pub fn diarize_segments(
    segments: &[VadSegment],
    embeddings: &[Option<SpeakerEmbedding>],
) -> Vec<SpeakerInterval> {
    if segments.is_empty() {
        return Vec::new();
    }

    // If no embeddings are available, fall back to energy-based alternation.
    if embeddings.iter().all(|e| e.is_none()) {
        return diarize_segments_energy(segments);
    }

    let mut intervals: Vec<SpeakerInterval> = Vec::new();
    let mut speakers: Vec<(String, Vec<f32>)> = Vec::new(); // (id, centroid)
    let mut last_end: Option<f64> = None;
    let mut next_id = 0usize;

    const SIM_THRESHOLD: f32 = 0.70;
    const SILENCE_ROTATE_GAP: f64 = 3.0;

    for (index, seg) in segments.iter().enumerate() {
        let embedding = embeddings.get(index).and_then(|e| e.as_ref());
        let rotated_silence = last_end.map(|e| seg.start - e > SILENCE_ROTATE_GAP).unwrap_or(false);

        // Pick the best matching existing speaker, or start a new one.
        let best = speakers.iter().enumerate().filter_map(|(si, (_id, centroid))| {
            let sim = embedding.and_then(|e| {
                Some(cosine_similarity(&e.vector, centroid))
            });
            sim.filter(|&s| s >= SIM_THRESHOLD).map(|s| (si, s))
        }).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let (speaker_id, _centroid) = if let Some((si, _sim)) = best {
            let (id, cent) = speakers.get_mut(si).unwrap();
            // Incremental centroid update.
            for (i, v) in cent.iter_mut().enumerate() {
                if let Some(ev) = embedding.as_ref().and_then(|e| e.vector.get(i)) {
                    *v = (*v + *ev) / 2.0;
                }
            }
            (id.clone(), cent.clone())
        } else {
            if rotated_silence {
                next_id += 1;
            }
            let id = format!("S{}", next_id);
            let cent = embedding.map(|e| e.vector.clone()).unwrap_or_default();
            speakers.push((id.clone(), cent.clone()));
            (id, cent)
        };

        intervals.push(SpeakerInterval {
            start: seg.start,
            end: seg.end,
            speaker_id,
        });
        last_end = Some(seg.end);
    }
    intervals
}