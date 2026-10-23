// ffprobe metadata: a passthrough of ffprobe's own JSON keys, so the struct keeps
// ffprobe's snake_case naming instead of renaming on the way in.
use std::path::Path;

use crate::contracts::ProbeMetadata;
use crate::process::tool;

/// Reads the container/stream metadata of a media file.
pub fn probe_metadata(ffprobe: &Path, input: &Path) -> Result<ProbeMetadata, String> {
    let output = tool(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration,format_name:stream=codec_name,codec_type,width,height",
            "-of",
            "json",
        ])
        .arg(input)
        .output()
        .map_err(|e| format!("FFPROBE_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("FFPROBE_JSON:{e}"))
}
