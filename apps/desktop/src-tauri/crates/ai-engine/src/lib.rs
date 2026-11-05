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

use image::RgbImage;
use ort::environment::Environment;
use ort::session::Session;
use ort::value::ValueType;
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
        let env = builder
            .build()
            .map_err(|e| AiError::Session(e.to_string()))?;
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
        let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("models")
            .join(name);
        if dev.is_file() {
            return Ok(dev);
        }
        // Workspace resource fallback for desktop development builds, where
        // Tauri may not have copied bundle resources into its runtime directory.
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/models")
            .join(name);
        if workspace.is_file() {
            return Ok(workspace);
        }
        Err(AiError::ModelNotFound(
            app.path()
                .resource_dir()
                .map(|d| d.join("models").join(name))
                .unwrap_or_else(|_| PathBuf::from("models").join(name)),
        ))
    }
}

/// Runs the bundled PP-OCRv6 DB text detector and returns one normalized union
/// box for text regions in the frame. No recognized text is produced or retained.
pub struct TextRegionDetector {
    session: Session,
    input_name: String,
    output_name: String,
}

impl TextRegionDetector {
    pub fn load(app: &tauri::AppHandle) -> AiResult<Self> {
        let engine = AiEngine::new()?;
        let path = AiEngine::model_path(app, "PP-OCRv6_det_small.onnx")?;
        let session = Session::builder(&engine.env)
            .map_err(|e| AiError::Session(e.to_string()))?
            .commit_from_file(&path)
            .map_err(|e| AiError::Session(e.to_string()))?;
        let input = session
            .inputs()
            .first()
            .ok_or_else(|| AiError::InvalidInput("OCR model has no input".into()))?;
        if input.name() != "x" || !matches!(input.dtype(), ValueType::Tensor { .. }) {
            return Err(AiError::InvalidInput(
                "unsupported PP-OCRv6 input contract".into(),
            ));
        }
        let input_name = input.name().to_string();
        let output = session
            .outputs()
            .first()
            .ok_or_else(|| AiError::InvalidInput("OCR model has no output".into()))?;
        if output.name() != "fetch_name_0" {
            return Err(AiError::InvalidInput(
                "unsupported PP-OCRv6 output contract".into(),
            ));
        }
        let output_name = output.name().to_string();
        Ok(Self {
            session,
            input_name,
            output_name,
        })
    }

    pub fn detect(&mut self, image: &RgbImage) -> AiResult<Option<TextBox>> {
        let (width, height) = (image.width() as usize, image.height() as usize);
        if width == 0 || height == 0 {
            return Err(AiError::InvalidInput("OCR image must not be empty".into()));
        }
        const TARGET_SHORT_SIDE: usize = 736;
        const MAX_LONG_SIDE: usize = 2000;
        let scale = (TARGET_SHORT_SIDE as f64 / width.min(height) as f64)
            .min(MAX_LONG_SIDE as f64 / width.max(height) as f64);
        let resized_w =
            (((width as f64 * scale).round() as usize + 31) / 32 * 32).clamp(32, MAX_LONG_SIDE);
        let resized_h =
            (((height as f64 * scale).round() as usize + 31) / 32 * 32).clamp(32, MAX_LONG_SIDE);
        let resized = image::imageops::resize(
            image,
            resized_w as u32,
            resized_h as u32,
            image::imageops::FilterType::Triangle,
        );
        let plane = resized_w * resized_h;
        let mut input_data = vec![0.0f32; 3 * plane];
        for y in 0..resized_h {
            for x in 0..resized_w {
                let pixel = resized.get_pixel(x as u32, y as u32);
                let offset = y * resized_w + x;
                for channel in 0..3 {
                    let normalized = f32::from(pixel[channel]) / 255.0;
                    input_data[channel * plane + offset] = (normalized - 0.5) / 0.5;
                }
            }
        }
        let input = ort::value::Tensor::from_array(([1usize, 3, resized_h, resized_w], input_data))
            .map_err(|e| AiError::InvalidInput(format!("OCR input tensor: {e}")))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => &input])
            .map_err(|e| AiError::Inference(e.to_string()))?;
        let (shape, values) = outputs
            .get(self.output_name.as_str())
            .ok_or_else(|| AiError::Inference("PP-OCRv6 output missing".into()))?
            .try_extract_tensor::<f32>()
            .map_err(|e| AiError::Inference(format!("PP-OCRv6 output: {e}")))?;
        if shape.as_ref() != [1, 1, resized_h as i64, resized_w as i64] {
            return Err(AiError::Inference(format!(
                "unexpected PP-OCRv6 output shape {shape:?}"
            )));
        }
        decode_db_text_regions(values, resized_w, resized_h, width, height)
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

/// Runs the bundled YuNet face detector and returns faces sorted by confidence.
pub struct FaceDetector {
    session: Session,
    input_name: String,
    input_shape: [usize; 4], // [N, C, H, W]
}

impl FaceDetector {
    /// Loads the bundled YuNet model with cls/obj/bbox heads at strides 8/16/32.
    pub fn load(app: &tauri::AppHandle) -> AiResult<Self> {
        let engine = AiEngine::new()?;
        let path = AiEngine::model_path(app, "face-detection.onnx")?;
        let session = Session::builder(&engine.env)
            .map_err(|e| AiError::Session(e.to_string()))?
            .commit_from_file(&path)
            .map_err(|e| AiError::Session(e.to_string()))?;

        let input = session
            .inputs()
            .first()
            .ok_or_else(|| AiError::InvalidInput("face model has no input".into()))?;
        let input_name = input.name().to_string();
        let input_shape = match input.dtype() {
            ValueType::Tensor { shape, .. } if shape.len() == 4 => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|dim| {
                        usize::try_from(*dim).map_err(|_| {
                            AiError::InvalidInput(
                                "face model input dimensions must be static".into(),
                            )
                        })
                    })
                    .collect::<AiResult<_>>()?;
                [dims[0], dims[1], dims[2], dims[3]]
            }
            _ => {
                return Err(AiError::InvalidInput(
                    "face model input must be a 4D tensor".into(),
                ))
            }
        };
        if input_shape[0] != 1 || input_shape[1] != 3 || input_shape[2] == 0 || input_shape[3] == 0
        {
            return Err(AiError::InvalidInput(format!(
                "unsupported face model input shape {input_shape:?}"
            )));
        }
        for name in [
            "cls_8", "cls_16", "cls_32", "obj_8", "obj_16", "obj_32", "bbox_8", "bbox_16",
            "bbox_32",
        ] {
            if !session.outputs().iter().any(|output| output.name() == name) {
                return Err(AiError::InvalidInput(format!(
                    "face model is missing YuNet output {name}"
                )));
            }
        }
        for stride in [8usize, 16, 32] {
            if input_shape[2] % stride != 0 || input_shape[3] % stride != 0 {
                return Err(AiError::InvalidInput(format!(
                    "face model input shape is not divisible by stride {stride}"
                )));
            }
        }

        Ok(Self {
            session,
            input_name,
            input_shape,
        })
    }

    /// Detects faces in an RGB image using YuNet's BGR 0..255 input convention.
    pub fn detect(&mut self, image: &RgbImage) -> AiResult<Vec<Face>> {
        if image.width() == 0 || image.height() == 0 {
            return Err(AiError::InvalidInput(
                "face detector image must not be empty".into(),
            ));
        }
        let [_, _, model_h, model_w] = self.input_shape;
        let scale =
            (model_w as f64 / image.width() as f64).min(model_h as f64 / image.height() as f64);
        let resized_w = ((image.width() as f64 * scale).round() as usize).clamp(1, model_w);
        let resized_h = ((image.height() as f64 * scale).round() as usize).clamp(1, model_h);
        let resized = image::imageops::resize(
            image,
            resized_w as u32,
            resized_h as u32,
            image::imageops::FilterType::Triangle,
        );
        let plane = model_h * model_w;
        let mut input_data = vec![0.0f32; 3 * plane];
        for y in 0..resized_h {
            for x in 0..resized_w {
                let pixel = resized.get_pixel(x as u32, y as u32);
                // OpenCV YuNet feeds an unnormalized BGR blob, scale=1, mean=0.
                let offset = y * model_w + x;
                input_data[offset] = pixel[2] as f32;
                input_data[plane + offset] = pixel[1] as f32;
                input_data[2 * plane + offset] = pixel[0] as f32;
            }
        }

        let input_value =
            ort::value::Tensor::from_array(([1usize, 3, model_h, model_w], input_data))
                .map_err(|e| AiError::InvalidInput(format!("input tensor from array: {e}")))?;
        let inputs = ort::inputs![self.input_name.as_str() => &input_value];
        let outputs = self
            .session
            .run(inputs)
            .map_err(|e| AiError::Inference(e.to_string()))?;
        decode_yunet_outputs(
            &outputs,
            model_w,
            model_h,
            resized_w,
            resized_h,
            image.width() as usize,
            image.height() as usize,
            scale,
        )
    }
}

/// Decodes YuNet class/object scores, exponential box parameters and keypoints
/// at strides 8, 16 and 32. Input dimensions are padded with zeros on the right
/// and bottom, so all boxes are mapped back through the same resize scale.
fn decode_yunet_outputs(
    outputs: &ort::session::SessionOutputs<'_>,
    model_w: usize,
    model_h: usize,
    valid_w: usize,
    valid_h: usize,
    image_w: usize,
    image_h: usize,
    scale: f64,
) -> AiResult<Vec<Face>> {
    let mut faces = Vec::new();
    for stride in [8usize, 16, 32] {
        let (cls_shape, cls) = outputs
            .get(format!("cls_{stride}"))
            .ok_or_else(|| AiError::Inference(format!("missing cls_{stride}")))?
            .try_extract_tensor::<f32>()
            .map_err(|error| AiError::Inference(format!("cls_{stride} extraction: {error}")))?;
        let (obj_shape, obj) = outputs
            .get(format!("obj_{stride}"))
            .ok_or_else(|| AiError::Inference(format!("missing obj_{stride}")))?
            .try_extract_tensor::<f32>()
            .map_err(|error| AiError::Inference(format!("obj_{stride} extraction: {error}")))?;
        let (bbox_shape, bbox) = outputs
            .get(format!("bbox_{stride}"))
            .ok_or_else(|| AiError::Inference(format!("missing bbox_{stride}")))?
            .try_extract_tensor::<f32>()
            .map_err(|error| AiError::Inference(format!("bbox_{stride} extraction: {error}")))?;
        faces.extend(decode_yunet_head(
            stride,
            model_w,
            model_h,
            valid_w,
            valid_h,
            image_w,
            image_h,
            scale,
            cls_shape.as_ref(),
            cls,
            obj_shape.as_ref(),
            obj,
            bbox_shape.as_ref(),
            bbox,
        )?);
    }
    faces.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(faces)
}

fn decode_yunet_head(
    stride: usize,
    model_w: usize,
    model_h: usize,
    valid_w: usize,
    valid_h: usize,
    image_w: usize,
    image_h: usize,
    scale: f64,
    cls_shape: &[i64],
    cls: &[f32],
    obj_shape: &[i64],
    obj: &[f32],
    bbox_shape: &[i64],
    bbox: &[f32],
) -> AiResult<Vec<Face>> {
    let grid_w = model_w / stride;
    let grid_h = model_h / stride;
    let count = grid_w
        .checked_mul(grid_h)
        .ok_or_else(|| AiError::Inference(format!("YuNet grid overflows at stride {stride}")))?;
    if cls_shape != [1, count as i64, 1]
        || obj_shape != [1, count as i64, 1]
        || bbox_shape != [1, count as i64, 4]
        || cls.len() != count
        || obj.len() != count
        || bbox.len() != count.checked_mul(4).unwrap_or(usize::MAX)
        || !scale.is_finite()
        || scale <= 0.0
        || valid_w == 0
        || valid_h == 0
        || valid_w > model_w
        || valid_h > model_h
        || image_w == 0
        || image_h == 0
    {
        return Err(AiError::Inference(format!(
            "unexpected YuNet tensor shapes at stride {stride}"
        )));
    }

    let mut faces = Vec::new();
    for index in 0..count {
        let cls_score = cls[index].clamp(0.0, 1.0);
        let obj_score = obj[index].clamp(0.0, 1.0);
        let confidence = (cls_score * obj_score).sqrt();
        if !confidence.is_finite() || confidence < 0.5 {
            continue;
        }
        let base = index * 4;
        let values = &bbox[base..base + 4];
        if values.iter().any(|value| !value.is_finite()) {
            continue;
        }
        let row = index / grid_w;
        let col = index % grid_w;
        let center_x = (col as f32 + values[0]) * stride as f32;
        let center_y = (row as f32 + values[1]) * stride as f32;
        let width = values[2].exp() * stride as f32;
        let height = values[3].exp() * stride as f32;
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            continue;
        }
        let x1 = (center_x - width / 2.0).clamp(0.0, valid_w as f32) as f64 / scale;
        let y1 = (center_y - height / 2.0).clamp(0.0, valid_h as f32) as f64 / scale;
        let x2 = (center_x + width / 2.0).clamp(0.0, valid_w as f32) as f64 / scale;
        let y2 = (center_y + height / 2.0).clamp(0.0, valid_h as f32) as f64 / scale;
        if x2 <= x1 || y2 <= y1 {
            continue;
        }
        faces.push(Face {
            center_x: (((x1 + x2) / 2.0) / image_w as f64).clamp(0.0, 1.0),
            center_y: (((y1 + y2) / 2.0) / image_h as f64).clamp(0.0, 1.0),
            width: ((x2 - x1) / image_w as f64).clamp(0.0, 1.0),
            height: ((y2 - y1) / image_h as f64).clamp(0.0, 1.0),
            confidence: confidence as f64,
        });
    }
    Ok(faces)
}

/// A text-region box in normalized source-image coordinates.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct TextBox {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Converts PP-OCRv6's DB probability map into one conservative union box.
/// The DB map is thresholded at 0.3, dilated with the configured 2x2 kernel,
/// and connected regions whose mean confidence is below 0.5 are discarded.
fn decode_db_text_regions(
    map: &[f32],
    map_w: usize,
    map_h: usize,
    image_w: usize,
    image_h: usize,
) -> AiResult<Option<TextBox>> {
    use std::collections::VecDeque;

    let expected_len = map_w
        .checked_mul(map_h)
        .ok_or_else(|| AiError::Inference("PP-OCRv6 output dimensions overflow".into()))?;
    if map_w < 3 || map_h < 3 || image_w == 0 || image_h == 0 || map.len() != expected_len {
        return Err(AiError::Inference(
            "malformed PP-OCRv6 probability map".into(),
        ));
    }
    if map
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(AiError::Inference(
            "invalid PP-OCRv6 probability values".into(),
        ));
    }

    let mut active: Vec<bool> = map.iter().map(|value| *value > 0.3).collect();
    let thresholded = active.clone();
    for y in 0..map_h {
        for x in 0..map_w {
            let i = y * map_w + x;
            if thresholded[i] {
                if x + 1 < map_w {
                    active[i + 1] = true;
                }
                if y + 1 < map_h {
                    active[i + map_w] = true;
                }
                if x + 1 < map_w && y + 1 < map_h {
                    active[i + map_w + 1] = true;
                }
            }
        }
    }

    let mut seen = vec![false; active.len()];
    let mut union: Option<(usize, usize, usize, usize)> = None;
    for seed in 0..active.len() {
        if !active[seed] || seen[seed] {
            continue;
        }
        let mut queue = VecDeque::from([seed]);
        seen[seed] = true;
        let (mut left, mut top, mut right, mut bottom) = (map_w, map_h, 0usize, 0usize);
        while let Some(index) = queue.pop_front() {
            let x = index % map_w;
            let y = index / map_w;
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
            for next in [
                x.checked_sub(1).map(|nx| y * map_w + nx),
                (x + 1 < map_w).then_some(y * map_w + x + 1),
                y.checked_sub(1).map(|ny| ny * map_w + x),
                (y + 1 < map_h).then_some((y + 1) * map_w + x),
            ]
            .into_iter()
            .flatten()
            {
                if active[next] && !seen[next] {
                    seen[next] = true;
                    queue.push_back(next);
                }
            }
        }
        if right - left < 3 || bottom - top < 3 {
            continue;
        }
        let mut score = 0.0f64;
        let mut count = 0usize;
        for y in top..bottom {
            for x in left..right {
                score += f64::from(map[y * map_w + x]);
                count += 1;
            }
        }
        if count == 0 || score / (count as f64) < 0.5 {
            continue;
        }
        union = Some(match union {
            Some((ul, ut, ur, ub)) => (ul.min(left), ut.min(top), ur.max(right), ub.max(bottom)),
            None => (left, top, right, bottom),
        });
    }

    Ok(union.map(|(left, top, right, bottom)| TextBox {
        left: (left as f64 / map_w as f64).clamp(0.0, 1.0),
        top: (top as f64 / map_h as f64).clamp(0.0, 1.0),
        right: (right as f64 / map_w as f64).clamp(0.0, 1.0),
        bottom: (bottom as f64 / map_h as f64).clamp(0.0, 1.0),
    }))
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
            let outputs = self
                .session
                .run(inputs)
                .map_err(|e| AiError::Inference(e.to_string()))?;
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
                segments.push(VadSegment {
                    start: seg_start,
                    end: time,
                });
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
            Err(_) => {
                return Ok(Self {
                    session: None,
                    input_name: None,
                    _sample_rate: 16000,
                })
            }
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

                let input_value = ort::value::Tensor::from_array(([1usize, TARGET], padded))
                    .map_err(|e| AiError::InvalidInput(format!("embedding input tensor: {e}")))?;
                let inputs = ort::inputs![input_name.as_str() => &input_value];
                let outputs = session
                    .run(inputs)
                    .map_err(|e| AiError::Inference(e.to_string()))?;
                let (_name, output) = outputs.into_iter().next().ok_or_else(|| {
                    AiError::Inference("embedding model produced no output".into())
                })?;
                let (shape, data) = output
                    .try_extract_tensor::<f32>()
                    .map_err(|e| AiError::Inference(format!("embedding output extraction: {e}")))?;
                if shape.len() != 2 {
                    return Err(AiError::Inference(
                        "embedding model output must be 2D [1,D]".into(),
                    ));
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
        let rotated_silence = last_end
            .map(|e| seg.start - e > SILENCE_ROTATE_GAP)
            .unwrap_or(false);

        // Pick the best matching existing speaker, or start a new one.
        let best = speakers
            .iter()
            .enumerate()
            .filter_map(|(si, (_id, centroid))| {
                let sim = embedding.and_then(|e| Some(cosine_similarity(&e.vector, centroid)));
                sim.filter(|&s| s >= SIM_THRESHOLD).map(|s| (si, s))
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

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

#[cfg(test)]
mod yunet_tests {
    use super::{decode_db_text_regions, decode_yunet_head, AiError, Face};

    fn decode(cls: &[f32], obj: &[f32], bbox: &[f32], scale: f64) -> Result<Vec<Face>, AiError> {
        decode_yunet_head(
            8,
            32,
            32,
            32,
            32,
            (32.0 / scale) as usize,
            (32.0 / scale) as usize,
            scale,
            &[1, 16, 1],
            cls,
            &[1, 16, 1],
            obj,
            &[1, 16, 4],
            bbox,
        )
    }

    #[test]
    fn decodes_yunet_box_and_geometric_mean_score() {
        let mut cls = vec![0.0; 16];
        let mut obj = vec![0.0; 16];
        let mut bbox = vec![0.0; 64];
        cls[5] = 0.81;
        obj[5] = 0.64;
        bbox[20..24].copy_from_slice(&[0.5, 0.5, 2.0f32.ln(), 2.0f32.ln()]);
        let faces = decode(&cls, &obj, &bbox, 1.0).unwrap();
        assert_eq!(faces.len(), 1);
        let face = &faces[0];
        assert!((face.confidence - 0.72).abs() < 1e-6);
        assert!((face.center_x - 0.375).abs() < 1e-6);
        assert!((face.center_y - 0.375).abs() < 1e-6);
        assert!((face.width - 0.5).abs() < 1e-6);
        assert!((face.height - 0.5).abs() < 1e-6);
    }

    #[test]
    fn maps_letterboxed_boxes_back_to_source_coordinates() {
        let mut cls = vec![0.0; 16];
        let mut obj = vec![0.0; 16];
        let mut bbox = vec![0.0; 64];
        cls[5] = 0.81;
        obj[5] = 0.64;
        bbox[20..24].copy_from_slice(&[0.5, 0.5, 2.0f32.ln(), 2.0f32.ln()]);
        let face = decode(&cls, &obj, &bbox, 2.0).unwrap().remove(0);
        assert!((face.center_x - 0.375).abs() < 1e-6);
        assert!((face.center_y - 0.375).abs() < 1e-6);
        assert!((face.width - 0.5).abs() < 1e-6);
        assert!((face.height - 0.5).abs() < 1e-6);
    }

    #[test]
    fn decodes_ocr_probability_map_into_normalized_union_box() {
        let mut map = vec![0.0f32; 20 * 12];
        for y in 3..6 {
            for x in 2..7 {
                map[y * 20 + x] = 0.9;
            }
            for x in 11..17 {
                map[y * 20 + x] = 0.9;
            }
        }
        let text = decode_db_text_regions(&map, 20, 12, 40, 24)
            .unwrap()
            .unwrap();
        assert!((text.left - 0.1).abs() < 1e-6);
        assert!((text.right - 0.9).abs() < 1e-6);
        assert!((text.top - 0.25).abs() < 1e-6);
        assert!((text.bottom - (7.0 / 12.0)).abs() < 1e-6);
    }

    #[test]
    fn rejects_weak_and_rejects_malformed_ocr_probability_maps() {
        assert!(decode_db_text_regions(&vec![0.0; 64], 8, 8, 8, 8)
            .unwrap()
            .is_none());
        assert!(decode_db_text_regions(&[0.9; 15], 4, 4, 4, 4).is_err());
        assert!(decode_db_text_regions(&[f32::NAN; 16], 4, 4, 4, 4).is_err());
    }
}
