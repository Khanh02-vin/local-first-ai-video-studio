// Headless front door to the render engine.
//
// The desktop app and this CLI are the two front doors to the same engine, so the
// FFmpeg arguments exist in exactly one place (ADR-001: one final renderer). The CLI
// is what the Node-side job queue talks to; it speaks the same RenderRequest contract
// the app does.
//
// Usage:
//   lfai-render probe  <input>
//   lfai-render render <request.json> [--ffmpeg PATH] [--ffprobe PATH]
//
// `render` writes the resulting file path to stdout as JSON. Progress is reported on
// stderr as `{"percent":..,"phase":".."}` lines so a caller can stream it.
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use render_engine::contracts::{RenderProgress, RenderRequest};
use render_engine::{engine, probe};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");
    let result = match command {
        "probe" => probe_command(&args[1..]),
        "render" => render_command(&args[1..]),
        "version" => Ok(env!("CARGO_PKG_VERSION").to_string()),
        _ => Err("USAGE: lfai-render <probe|render|version>".to_string()),
    };
    match result {
        Ok(output) => {
            println!("{output}");
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

/// The FFmpeg/ffprobe locations: explicit flags win, then the environment, then PATH.
fn binaries(flags: &[String]) -> (PathBuf, PathBuf) {
    let mut ffmpeg = None;
    let mut ffprobe = None;
    let mut index = 0;
    while index < flags.len() {
        match flags[index].as_str() {
            "--ffmpeg" if index + 1 < flags.len() => {
                ffmpeg = Some(PathBuf::from(&flags[index + 1]));
                index += 2;
            }
            "--ffprobe" if index + 1 < flags.len() => {
                ffprobe = Some(PathBuf::from(&flags[index + 1]));
                index += 2;
            }
            _ => index += 1,
        }
    }
    (
        ffmpeg
            .or_else(|| std::env::var("FFMPEG_PATH").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("ffmpeg")),
        ffprobe
            .or_else(|| std::env::var("FFPROBE_PATH").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("ffprobe")),
    )
}

fn probe_command(flags: &[String]) -> Result<String, String> {
    let input = flags
        .iter()
        .find(|value| !value.starts_with("--"))
        .ok_or("INPUT_REQUIRED")?;
    let (_, ffprobe) = binaries(flags);
    let path = Path::new(input)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if !path.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let metadata = probe::probe_metadata(&ffprobe, &path)?;
    serde_json::to_string(&metadata).map_err(|e| format!("PROBE_JSON:{e}"))
}

fn render_command(flags: &[String]) -> Result<String, String> {
    let request_path = flags
        .iter()
        .find(|value| !value.starts_with("--"))
        .ok_or("REQUEST_JSON_REQUIRED")?;
    let request: RenderRequest = serde_json::from_str(
        &std::fs::read_to_string(request_path).map_err(|e| format!("REQUEST_READ:{e}"))?,
    )
    .map_err(|e| format!("REQUEST_JSON:{e}"))?;
    let (ffmpeg, ffprobe) = binaries(flags);

    if request.plan.version == 0
        || request.plan.source_artifact_id.trim().is_empty()
        || !["9:16", "1:1", "16:9"].contains(&request.plan.aspect_ratio.as_str())
    {
        return Err("INVALID_RENDER_PLAN".into());
    }
    let mut plan = serde_json::to_value(&request.plan).map_err(|error| format!("RENDER_PLAN_JSON:{error}"))?;
    if let Some(fields) = plan.as_object_mut() {
        for (key, value) in &request.plan.extra {
            fields.insert(key.clone(), value.clone());
        }
    }
    let output_path = Path::new(&request.output)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&request.output).to_path_buf());
    let input_path = Path::new(&request.input)
        .canonicalize()
        .map_err(|e| format!("INPUT_PATH:{e}"))?;
    if input_path == output_path {
        return Err("OUTPUT_MUST_DIFFER_FROM_INPUT".into());
    }
    if !input_path.is_file() {
        return Err("INPUT_NOT_FILE".into());
    }
    let ranges: Vec<(f64, f64)> = request
        .plan
        .ranges
        .iter()
        .map(|range| (range.start, range.end))
        .collect();

    engine::RENDER_CANCELLED.store(false, Ordering::SeqCst);
    let on_progress = |progress: RenderProgress| {
        // stderr keeps stdout clean for the single JSON result line.
        eprintln!(
            "{}",
            serde_json::json!({ "percent": progress.percent, "phase": progress.phase })
        );
    };
    let rendered = engine::render_segments(
        &ffmpeg,
        &ffprobe,
        &[input_path.to_string_lossy().into_owned()],
        &output_path,
        &ranges,
        Some(request.plan.aspect_ratio.clone()),
        Some(&plan),
        &on_progress,
    )?;
    serde_json::to_string(&rendered).map_err(|e| format!("RENDER_JSON:{e}"))
}
