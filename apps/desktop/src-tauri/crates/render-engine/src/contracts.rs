// The wire contract between the render engine and its callers.
//
// These structs are the single source of truth for the render IPC shapes: the ones marked
// with the typeshare attribute are what `npm run contracts:generate` turns into
// packages/contracts/generated/generated.ts, and a parity test fails if the two sides
// drift apart.
//
// The engine crate holds them (not the desktop app) because both the app and the
// headless CLI speak the same contract — one renderer, two front doors.
use std::collections::HashMap;

use typeshare::typeshare;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisStatus {
    pub status: String,
    pub phase: String,
    pub progress: f64,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub chunk_completed: u64,
    pub chunk_total: u64,
    pub current_chunk: Option<u64>,
}

// ffprobe metadata is a passthrough of ffprobe's own JSON keys (snake_case), so these
// structs keep ffprobe's naming instead of renaming on the way in. The TS counterpart
// in packages/contracts/generated/ is produced from these declarations: run
// `npm run contracts:generate`, and `npm run test:contracts-published` fails if the
// committed copy has drifted from it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[typeshare]
pub struct ProbeMetadata {
    pub format: Option<ProbeFormat>,
    #[serde(default)]
    pub streams: Vec<ProbeStream>,
}

// #[serde(rename = "format_name")] documents the one key that is not a plain
// lowercase field name, so both Rust and the generated TS agree on it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[typeshare]
pub struct ProbeFormat {
    pub duration: Option<String>,
    #[serde(rename = "format_name")]
    pub format_name: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[typeshare]
pub struct ProbeStream {
    pub codec_name: Option<String>,
    pub codec_type: Option<String>,
    // Audio streams omit both keys in ffprobe's filtered output.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[typeshare]
#[serde(rename_all = "camelCase")]
pub struct RenderRange {
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderPlan {
    pub version: u32,
    pub source_artifact_id: String,
    pub ranges: Vec<RenderRange>,
    pub aspect_ratio: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRequest {
    pub input: String,
    pub output: String,
    pub plan: RenderPlan,
}

#[derive(Clone, serde::Serialize)]
#[typeshare]
#[serde(rename_all = "camelCase")]
pub struct RenderProgress {
    pub percent: f64,
    pub phase: String,
}
