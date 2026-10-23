// The preview proxy: a browser-friendly copy of the source for the in-app player.

use std::time::UNIX_EPOCH;

use crate::paths::{ffmpeg_bin, ffprobe_bin, local_preview_source, preview_proxy_cache_key, cache_dir, transcode_preview_proxy};

#[tauri::command]
pub(crate) async fn prepare_video_preview(app: tauri::AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = local_preview_source(&path)?;
        let metadata = std::fs::metadata(&source).map_err(|e| format!("INPUT_PATH:{e}"))?;
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let key = preview_proxy_cache_key(&source, metadata.len(), modified_nanos);
        let cache_dir = cache_dir().join("preview-cache");
        std::fs::create_dir_all(&cache_dir).map_err(|e| format!("PREVIEW_CACHE:{e}"))?;
        let output = cache_dir.join(format!("{key}.mp4"));
        if output.is_file() {
            return Ok(output.to_string_lossy().into_owned());
        }
        let temporary = cache_dir.join(format!("{key}.{}.tmp.mp4", std::process::id()));
        transcode_preview_proxy(&ffmpeg_bin(&app), &ffprobe_bin(&app), &source, &temporary)?;
        std::fs::rename(&temporary, &output).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            format!("PREVIEW_CACHE_PUBLISH:{e}")
        })?;
        Ok(output.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| format!("PREVIEW_WORKER:{error}"))?
}

