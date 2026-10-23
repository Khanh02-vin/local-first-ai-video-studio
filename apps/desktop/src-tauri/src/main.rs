#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::Manager;
mod analysis;
mod app;
mod contracts;
mod diarization;
mod paths;
mod preview;
mod providers;
mod project_store;
mod render;
mod runtime;
mod shell;
mod youtube;

use app::{
    app_info, last_panic, license_status, probe_video, project_create, project_get, project_list,
    project_save,
};
use analysis::{
    abandon_analysis, analysis_status, list_analysis_jobs, preflight_analyze, resume_queued_jobs,
    retry_analysis, spawn_analysis_worker, start_analysis, stop_analysis, stop_analysis_workers,
    Jobs, RunningGuard,
};
use diarization::{
    analyze_crop_path, analyze_scene_layouts, diarization_setup_status, diarize_video,
    generate_transcript_variants, install_diarization_runtime, sample_speaker_faces,
    save_diarization_token,
};
use paths::*;
use preview::prepare_video_preview;
use runtime::{set_runtime_model, stop_sidecars};
use shell::{install_panic_hook, APP_HANDLE};
use providers::{
    activate_ai_provider, delete_ai_provider, highlight_strategy_status, list_ai_providers,
    save_ai_provider, set_highlight_strategy,
};
use runtime::{
    bootstrap_whisper, clear_analysis_cache, download_llm_model, download_whisper_model,
    install_local_llm, local_llm_setup_status, model_download_status, rag_server_url,
    runtime_status, start_local_llm, stop_local_llm, storage_status,
    whisper_bootstrap_status, whisper_model_download_status, BootstrapState,
    LlamaDownloadState, LlamaState, ModelDownloadState, RagState, WhisperDlState,
};

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
fn main() {
    install_panic_hook();
    // Move durable data out of the legacy ~/.cache root before anything reads it
    // (copy-only; see paths::migrate_legacy_state). Single-root overrides skip this.
    paths::migrate_legacy_state();
    // WebKitGTK on some Linux GPUs (Intel iGPU laptops) fails EGL/DMABUF init
    // and renders an empty window — force software-safe compositing.
    if cfg!(target_os = "linux") {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
    #[cfg(windows)]
    {
        // Mirrors the WebKitGTK workaround for machines that reproduce WebView2
        // renderer crashes (blank window / silent exit): set WEBVIEW2_GPU_DISABLE=1
        // in runtime.env to force software compositing. Must be set before the
        // webview is created.
        if runtime_env()
            .get("WEBVIEW2_GPU_DISABLE")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            std::env::set_var(
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                "--disable-gpu-compositing --disable-features=UseChromeOSDirectVideoDecoder",
            );
        }
    }
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let supervisor_jobs = jobs.clone();
    let llama_state: LlamaState = Arc::new(Mutex::new(None));
    let rag_state: RagState = RagState::default();
    let bootstrap_state = BootstrapState(Arc::new(Mutex::new(None)));
    let download_state = ModelDownloadState(Arc::new(Mutex::new(None)));
    let llama_download_state = LlamaDownloadState(Arc::new(Mutex::new(None)));
    let whisper_dl_state = WhisperDlState(Arc::new(Mutex::new(None)));
    let app = tauri::Builder::default()
        // Registered first: a second launch exits here (after handing its args
        // to this instance) instead of reaching GTK's app-id activation, which
        // used to re-run Tauri's setup and panic on the duplicate `main` webview.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(jobs)
        .manage(llama_state.clone())
        .manage(rag_state.clone())
        .manage(bootstrap_state)
        .manage(download_state)
        .manage(llama_download_state)
        .manage(whisper_dl_state.clone())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .setup(move |app| {
            resume_queued_jobs(app.handle().clone(), supervisor_jobs.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            last_panic,
            license_status,
            set_runtime_model,
            set_highlight_strategy,
            highlight_strategy_status,
            list_ai_providers,
            save_ai_provider,
            activate_ai_provider,
            delete_ai_provider,
            start_local_llm,
            stop_local_llm,
            rag_server_url,
            bootstrap_whisper,
            whisper_bootstrap_status,
            download_llm_model,
            model_download_status,
            install_local_llm,
            local_llm_setup_status,
            runtime_status,
            storage_status,
            clear_analysis_cache,
            preflight_analyze,
            start_analysis,
            youtube::start_youtube_analysis,
            youtube::preview_origin,
            analysis_status,
            list_analysis_jobs,
            retry_analysis,
            abandon_analysis,
            stop_analysis,
            probe_video,
            prepare_video_preview,
            render::render_video,
            render::cancel_render,
            youtube::crawl_playlist,
            download_whisper_model,
            whisper_model_download_status,
            youtube::youtube_search_videos,
            youtube::youtube_oauth_start,
            youtube::youtube_oauth_status,
            youtube::youtube_oauth_disconnect,
            youtube::youtube_channel_videos,
            youtube::youtube_ingest_captions,
            project_list,
            project_get,
            project_save,
            project_create,
            generate_transcript_variants,
            diarization_setup_status,
            save_diarization_token,
            install_diarization_runtime,
            diarize_video,
            sample_speaker_faces,
            analyze_crop_path,
            analyze_scene_layouts
        ])
        .build(tauri::generate_context!())
        .expect("error while building desktop application");
    let _ = APP_HANDLE.set(app.handle().clone());
    let llama_shutdown = llama_state.clone();
    let rag_shutdown = rag_state.clone();
    app.run(move |_handle, event| {
        if let tauri::RunEvent::Exit = event {
            stop_sidecars(&llama_shutdown, &rag_shutdown);
            stop_analysis_workers();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
use std::sync::{Arc, Mutex};

    use contracts::ProbeMetadata;
    use render_engine::contracts::RenderProgress;
    use render_engine::engine::{
        create_temporary_render_dir, stable_subject_pair, subject_pair_continues,
    };
    use runtime::sha256_file;
    use crate::analysis::kill_analysis_worker;
    use std::path::PathBuf;
    use crate::runtime::{
        download_and_verify, find_runtime_server, llama_runtime_asset, run_whisper_bootstrap,
        write_runtime_env, LLAMA_RUNTIME_ASSETS, LLAMA_VERSION,
    };
    use crate::youtube::{
        preview_page, preview_query, youtube_search_query,
    };
    use crate::render::RENDER_CANCELLED;
    use render_engine::engine::{
        ass_dialogue, ass_timestamp, escape_ass_filter_path, remap_hook, render_segments,
        side_by_side_graph, write_temporary_ass,
    };
    use std::process::Command;

    /// Cargo runs unit tests in parallel threads inside one process; these tests mutate
    /// process-wide env vars, so they take turns on this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn endpoint_validation_allows_custom_provider_but_limits_local_llm_to_loopback() {
        assert_eq!(valid_provider_url("https://api.example.test/v1/"), Some("https://api.example.test/v1".into()));
        assert!(valid_provider_url("https://api.example.test").is_some());
        assert_eq!(valid_provider_url("http://127.0.0.1:1234/v1"), Some("http://127.0.0.1:1234/v1".into()));
        assert!(valid_provider_url("ftp://api.example.test").is_none());
        assert!(valid_provider_url("https://user:pass@example.test/v1").is_none());
        assert!(valid_provider_url("https://api.example.test/v1?x=1").is_none());
        assert!(valid_provider_url("https://api.example.test/v1#fragment").is_none());
        assert!(valid_local_llm_url("http://127.0.0.1:8080/v1"));
        assert!(valid_local_llm_url("http://[::1]:8080"));
        assert!(valid_local_llm_url("http://localhost:8080"));
        assert!(!valid_local_llm_url("https://example.test/v1"));
        assert!(!valid_local_llm_url("http://user@localhost:8080"));
        assert!(!valid_local_llm_url("file:///tmp/model"));
    }

    #[test]
    fn private_file_writes_owner_only_on_unix() {
        let dir = create_temporary_render_dir().unwrap();
        let file = dir.0.join("secret.txt");
        write_private_file(&file, b"secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn analysis_range_limit_accepts_boundary_and_rejects_excess() {
        assert!(analysis_range_allowed(0.0, 600.0));
        assert!(!analysis_range_allowed(0.0, 600.001));
        assert!(!analysis_range_allowed(10.0, 9.0));
    }

    #[test]
    /// The preview cache must evict oldest-first under the cap and never touch
    /// the newest file — every import adds a regenerable transcode, and nothing
    /// else ever deleted one.
    #[test]
    fn preview_cache_quota_evicts_oldest_first() {
        let dir = create_temporary_render_dir().unwrap();
        let cache = dir.0.join("preview-cache");
        std::fs::create_dir_all(&cache).unwrap();
        // Three 4-byte previews with ordered mtimes: successive writes a few
        // milliseconds apart give a stable oldest→newest order.
        std::fs::write(cache.join("a.mp4"), b"1234").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(cache.join("b.mp4"), b"1234").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(cache.join("c.mp4"), b"1234").unwrap();

        // Quota 8 bytes over three 4-byte files: one must go, the oldest.
        preview::enforce_preview_cache_quota_at(&cache, 8);
        assert!(!cache.join("a.mp4").exists(), "the oldest preview must be evicted");
        assert!(cache.join("b.mp4").exists());
        assert!(cache.join("c.mp4").exists());

        // Under the cap: nothing is touched.
        preview::enforce_preview_cache_quota_at(&cache, 8);
        assert!(cache.join("b.mp4").exists() && cache.join("c.mp4").exists());

        // A foreign file type in the cache dir is not evicted (only *.mp4 entries).
        std::fs::write(cache.join("notes.txt"), b"keep me around").unwrap();
        preview::enforce_preview_cache_quota_at(&cache, 0);
        assert!(cache.join("notes.txt").exists(), "non-mp4 entries must be left alone");
    }

    fn preview_proxy_cache_key_tracks_source_version_and_rejects_remote_inputs() {
        let dir = create_temporary_render_dir().unwrap();
        let source = dir.0.join("source.webm");
        std::fs::write(&source, b"source").unwrap();
        let source = source.canonicalize().unwrap();
        let key = preview_proxy_cache_key(&source, 128, 42);
        assert_eq!(key, preview_proxy_cache_key(&source, 128, 42));
        assert_ne!(key, preview_proxy_cache_key(&source, 129, 42));
        assert_ne!(key, preview_proxy_cache_key(&source, 128, 43));
        assert!(local_preview_source("https://youtube.com/watch?v=abc123").is_err());
        assert_eq!(
            local_preview_source(source.to_str().unwrap()).unwrap(),
            source
        );
    }

    #[test]
    fn preview_transcode_creates_verified_browser_friendly_copy_without_touching_source() {
        let ffmpeg = Path::new("ffmpeg");
        let ffprobe = Path::new("ffprobe");
        if Command::new(ffmpeg).arg("-version").output().is_err()
            || Command::new(ffprobe).arg("-version").output().is_err()
        {
            eprintln!("skipping preview proxy fixture: ffmpeg/ffprobe unavailable");
            return;
        }
        let dir = create_temporary_render_dir().unwrap();
        let source = dir.0.join("source.mp4");
        let generated = Command::new(ffmpeg)
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=5:duration=1",
                "-c:v",
                "mpeg4",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(generated.success());
        let original = std::fs::read(&source).unwrap();
        let proxy = dir.0.join("proxy.mp4");
        transcode_preview_proxy(ffmpeg, ffprobe, &source, &proxy).unwrap();
        assert!(proxy.is_file());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let metadata = Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,pix_fmt",
                "-of",
                "json",
            ])
            .arg(proxy)
            .output()
            .unwrap();
        let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
        assert_eq!(metadata["streams"][0]["codec_name"], "h264");
        assert_eq!(metadata["streams"][0]["pix_fmt"], "yuv420p");
    }

    #[test]
    fn stable_subject_pair_requires_three_consistent_samples() {
        let a = serde_json::json!({"x":0.1,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8});
        let b = serde_json::json!({"x":0.7,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8});
        let samples = vec![
            vec![a.clone(), b.clone()],
            vec![a.clone(), b.clone()],
            vec![a.clone(), b.clone()],
        ];
        assert!(stable_subject_pair(&samples).is_some());
        assert!(stable_subject_pair(&samples[..2]).is_none());
        let jittered = vec![
            vec![
                serde_json::json!({"x":0.1,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8}),
                serde_json::json!({"x":0.7,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8}),
            ],
            vec![
                serde_json::json!({"x":0.12,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8}),
                serde_json::json!({"x":0.68,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8}),
            ],
            vec![
                serde_json::json!({"x":0.11,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8}),
                serde_json::json!({"x":0.69,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8}),
            ],
        ];
        let representative = stable_subject_pair(&jittered).unwrap();
        assert_eq!(representative[0]["x"], 0.11);
        assert_eq!(representative[1]["x"], 0.69);
        let moved = serde_json::json!({"x":0.45,"y":0.2,"width":0.1,"height":0.2,"confidence":0.8});
        assert!(!subject_pair_continues(&[a, b], &[moved.clone(), moved]));
    }

    #[test]
    fn side_by_side_graph_requires_valid_distinct_subject_boxes() {
        let boxes = vec![
            serde_json::json!({"x":0.1,"y":0.2,"width":0.2,"height":0.3,"confidence":0.8}),
            serde_json::json!({"x":0.7,"y":0.2,"width":0.2,"height":0.3,"confidence":0.8}),
        ];
        let graph =
            side_by_side_graph(&boxes, &["ass='/tmp/captions.ass'".into()], "9:16").unwrap();
        assert!(graph.contains("split=2"));
        assert!(graph.contains("hstack=inputs=2"));
        assert!(graph.contains("ass='/tmp/captions.ass'"));
        assert!(side_by_side_graph(&boxes[..1], &[], "9:16").is_none());
        let invalid = vec![
            boxes[0].clone(),
            serde_json::json!({"x":0.9,"y":0.2,"width":0.2,"height":0.3,"confidence":0.8}),
        ];
        assert!(side_by_side_graph(&invalid, &[], "9:16").is_none());
    }

    /// The engine reports progress through its own sink; the renderer tests only
    /// care that the render finishes, so a sink that drops the updates is enough.
    struct SilentProgress;

    impl render_engine::engine::ProgressSink for SilentProgress {
        fn report(&self, _progress: RenderProgress) {}
    }

    fn silent_progress() -> SilentProgress {
        SilentProgress
    }

    /// Cancelling must take down the worker *and* its children. This starts a shell
    /// that spawns a grandchild, then kills only the worker's process group.
    #[cfg(unix)]
    #[test]
    fn kill_analysis_worker_terminates_the_whole_process_group() {
        let dir = create_temporary_render_dir().unwrap();
        let parent_marker = dir.0.join("parent.pid");
        let child_marker = dir.0.join("child.pid");
        let mut cmd = Command::new("bash");
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.args([
            "-c",
            &format!(
                "echo $$ > '{}'; sleep 30 & echo $! > '{}'; wait",
                parent_marker.display(),
                child_marker.display()
            ),
        ]);
        let mut child = cmd.spawn().expect("fixture process must spawn");
        let parent_pid = child.id();        let wait_for_pid = |path: &Path| -> Option<u32> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if let Ok(pid) = text.trim().parse::<u32>() { return Some(pid); }
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            None
        };
        let worker = wait_for_pid(&parent_marker).expect("fixture worker never reported its pid");
        let grandchild = wait_for_pid(&child_marker).expect("fixture grandchild never reported its pid");
        assert_ne!(parent_pid, grandchild, "the grandchild must be a separate process");

        kill_analysis_worker(parent_pid);
        let _ = child.wait();

        // A surviving grandchild is exactly the orphan this guard exists to prevent.
        let settle = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < settle && process_alive(grandchild) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(worker, parent_pid, "the fixture's own pid must be the group leader");
        assert!(!process_alive(grandchild), "grandchild pid {grandchild} survived the group kill");
        assert!(!process_alive(parent_pid), "worker pid {parent_pid} survived its own kill");
    }

    #[cfg(unix)]
    fn process_alive(pid: u32) -> bool {
        // Signal 0 probes existence without sending anything.
        Command::new("kill").args(["-0", &pid.to_string()]).status().map(|s| s.success()).unwrap_or(false)
    }

    /// The renderer tests drive real FFmpeg through a process-wide "current child" slot,
    /// so they must not run at the same time.
    static RENDER_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// ffprobe emits snake_case keys and omits width/height on audio streams; the typed
    /// probe result must survive that payload or the app rejects verified media.
    #[test]
    fn probe_metadata_decodes_ffprobe_stream_payload() {
        let probe: ProbeMetadata = serde_json::from_value(serde_json::json!({
            "streams": [
                {"codec_name": "h264", "codec_type": "video", "width": 1280, "height": 720},
                {"codec_name": "aac", "codec_type": "audio"}
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "180.000000"}
        }))
        .expect("ffprobe payload must decode into typed metadata");
        let video = probe.streams.iter().find(|stream| stream.codec_type.as_deref() == Some("video"));
        assert_eq!(video.and_then(|stream| stream.width), Some(1280));
        assert_eq!(video.and_then(|stream| stream.height), Some(720));
        assert_eq!(probe.format.as_ref().and_then(|format| format.duration.as_deref()), Some("180.000000"));
    }

    /// A cancel request must abort an in-flight render and be reported as cancelled rather
    /// than left to finish (or swallowed into a generic failure).
    #[test]
    fn render_cancellation_stops_a_running_render() {
        let _render_guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let ffmpeg = Path::new("ffmpeg");
        let ffprobe = Path::new("ffprobe");
        if Command::new(ffmpeg).arg("-version").output().is_err() || Command::new(ffprobe).arg("-version").output().is_err() {
            eprintln!("skipping cancellation fixture: ffmpeg/ffprobe unavailable");
            return;
        }
        let dir = create_temporary_render_dir().unwrap();
        let source = dir.0.join("source.mp4");
        let fixture = Command::new(ffmpeg)
            .args([
                "-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=10:duration=30",
                "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=30",
                "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(fixture.success(), "source fixture generation failed");
        let output = dir.0.join("cancelled.mp4");
        let canceller = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            RENDER_CANCELLED.store(true, Ordering::SeqCst);
        });
        let result = render_segments(
            ffmpeg,
            ffprobe,
            &[source.to_string_lossy().into_owned()],
            &output,
            &[(0.0, 30.0)],
            None,
            None,
            &silent_progress(),
        );
        canceller.join().unwrap();
        RENDER_CANCELLED.store(false, Ordering::SeqCst);
        let error = result.expect_err("a cancelled render must not report success");
        assert!(error.contains("RENDER_CANCELLED"), "expected RENDER_CANCELLED, got: {error}");
    }

    #[test]
    fn ffmpeg_renderer_handles_ranges_captions_audio_and_side_by_side() {
        let _render_guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let ffmpeg = Path::new("ffmpeg");
        let ffprobe = Path::new("ffprobe");
        if Command::new(ffmpeg).arg("-version").output().is_err()
            || Command::new(ffprobe).arg("-version").output().is_err()
        {
            eprintln!("skipping FFmpeg render fixture: ffmpeg/ffprobe unavailable");
            return;
        }
        let dir = create_temporary_render_dir().unwrap();
        let source = dir.0.join("source.mp4");
        let fixture = Command::new(ffmpeg)
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=10:duration=8",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100:duration=8",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(fixture.success(), "source fixture generation failed");
        let boxes = vec![
            serde_json::json!({"x":0.1,"y":0.2,"width":0.2,"height":0.3,"confidence":0.8}),
            serde_json::json!({"x":0.7,"y":0.2,"width":0.2,"height":0.3,"confidence":0.8}),
        ];
        let plan = serde_json::json!({"sourceArtifactId":"src","captions":[{"id":"c1","text":"fixture caption","start":0.25,"end":1.25},{"id":"c2","text":"second range","start":4.25,"end":5.25}],"sceneLayouts":[{"start":0.0,"end":2.0,"layout":"side-by-side","confidence":0.8,"confirmed":true,"sourceArtifactId":"src","subjectBoxes":boxes}]});
        let output = dir.0.join("render.mp4");
        render_segments(
            ffmpeg,
            ffprobe,
            &[source.to_string_lossy().into_owned()],
            &output,
            &[(0.0, 2.0), (4.0, 6.0)],
            Some("9:16".into()),
            Some(&plan),
            &silent_progress(),
        )
        .unwrap();
        let probe = Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type,width,height",
                "-of",
                "json",
            ])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "ffprobe failed: {}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let duration = metadata["format"]["duration"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(
            (duration - 4.0).abs() < 0.2,
            "unexpected output duration: {duration}"
        );
        let streams = metadata["streams"].as_array().unwrap();
        assert!(streams.iter().any(|s| s["codec_type"] == "audio"));
        let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
        assert_eq!(video["width"], 1080);
        assert_eq!(video["height"], 1920);
        let captioned = dir.0.join("captioned.mp4");
        let plain_output = dir.0.join("plain.mp4");
        let caption_plan = serde_json::json!({"sourceArtifactId":"src","captions":[{"id":"c1","text":"CAPTIONVISIBLE","start":0.0,"end":2.0}]});
        let plain_plan = serde_json::json!({"sourceArtifactId":"src","captions":[]});
        let side_by_side_plan = serde_json::json!({"sourceArtifactId":"src","captions":[],"sceneLayouts":[{"start":0.0,"end":2.0,"layout":"side-by-side","confidence":0.8,"confirmed":true,"sourceArtifactId":"src","subjectBoxes":boxes}]});
        render_segments(
            ffmpeg,
            ffprobe,
            &[source.to_string_lossy().into_owned()],
            &captioned,
            &[(0.0, 2.0)],
            Some("9:16".into()),
            Some(&caption_plan),
            &silent_progress(),
        )
        .unwrap();
        render_segments(
            ffmpeg,
            ffprobe,
            &[source.to_string_lossy().into_owned()],
            &plain_output,
            &[(0.0, 2.0)],
            Some("9:16".into()),
            Some(&plain_plan),
            &silent_progress(),
        )
        .unwrap();
        let multi_output = dir.0.join("multi.mp4");
        render_segments(
            ffmpeg,
            ffprobe,
            &[source.to_string_lossy().into_owned()],
            &multi_output,
            &[(0.0, 2.0), (4.0, 6.0)],
            Some("9:16".into()),
            Some(&side_by_side_plan),
            &silent_progress(),
        )
        .unwrap();
        let frame_hash = |path: &Path| {
            let output = Command::new(ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(path)
                .args([
                    "-vf",
                    "select='eq(n,24)'",
                    "-vsync",
                    "0",
                    "-frames:v",
                    "1",
                    "-f",
                    "framemd5",
                    "-",
                ])
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .last()
                .unwrap_or("")
                .to_owned()
        };
        assert_ne!(
            frame_hash(&captioned),
            frame_hash(&plain_output),
            "captioned render should differ from the uncaptained frame while the cue is active"
        );
        let multi_probe = Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&multi_output)
            .output()
            .unwrap();
        assert!(multi_probe.status.success());
        let multi_duration: f64 = String::from_utf8_lossy(&multi_probe.stdout)
            .trim()
            .parse()
            .unwrap();
        assert!(
            (multi_duration - 4.0).abs() < 0.2,
            "unexpected multi-range duration: {multi_duration}"
        );
    }

    #[test]
    fn ass_dialogue_clips_cues_to_clip_and_escapes_text() {
        let captions = vec![
            serde_json::json!({"text": "before", "start": 1.0, "end": 2.0}),
            serde_json::json!({"text": "hello {world}\nline", "start": 4.5, "end": 6.0}),
            serde_json::json!({"text": "after", "start": 9.0, "end": 10.0}),
        ];
        let ass = ass_dialogue(&captions, 5.0, 3.0).unwrap().unwrap();
        assert!(!ass.contains("before"));
        assert!(!ass.contains("after"));
        assert!(
            ass.contains("Dialogue: 0,0:00:00.00,0:00:01.00,Shorts,,0,0,0,,hello \\{world\\} line"),
            "{ass}"
        );
    }

    #[test]
    fn ass_dialogue_uses_centiseconds_and_returns_none_when_empty() {
        assert_eq!(ass_timestamp(3661.239), "1:01:01.23");
        let captions = vec![serde_json::json!({"text": "outside", "start": 1.0, "end": 2.0})];
        assert_eq!(ass_dialogue(&captions, 2.0, 3.0).unwrap(), None);
        assert_eq!(ass_dialogue(&[], 2.0, 3.0).unwrap(), None);
    }

    #[test]
    fn ass_dialogue_rejects_invalid_caption_timings() {
        let captions = vec![serde_json::json!({"text": "bad", "start": 2.0, "end": 1.0})];
        assert_eq!(
            ass_dialogue(&captions, 0.0, 3.0).unwrap_err(),
            "INVALID_RENDER_CAPTION"
        );
    }

    #[test]
    fn ass_filter_path_escapes_filter_syntax() {
        assert_eq!(
            escape_ass_filter_path(r"C:\clips,[one];'two'.ass"),
            r"C\:/clips\,\[one\]\;\'two\'.ass"
        );
    }

    #[test]
    fn remapped_captions_split_across_ranges_and_follow_output_order() {
        let captions = vec![
            serde_json::json!({"text":"kept", "start":1.0, "end":8.0}),
            serde_json::json!({"text":"gap", "start":4.0, "end":5.0}),
        ];
        let ranges = vec![(6.0, 8.0), (1.0, 3.0)];
        let first = ass_dialogue(&captions, ranges[0].0, ranges[0].1 - ranges[0].0)
            .unwrap()
            .unwrap();
        let second = ass_dialogue(&captions, ranges[1].0, ranges[1].1 - ranges[1].0)
            .unwrap()
            .unwrap();
        assert!(first.contains("0:00:00.00,0:00:02.00"), "{first}");
        assert!(second.contains("0:00:00.00,0:00:02.00"), "{second}");
        let hook = serde_json::json!({"text":"kept", "start":1.0, "end":8.0});
        assert_eq!(
            remap_hook(&hook, &ranges).unwrap(),
            vec![(0, 0.0, 2.0), (1, 0.0, 2.0)]
        );
    }

    #[test]
    fn hook_remapping_tracks_each_source_range_and_concat_offset() {
        let hook = serde_json::json!({"text":"hook", "start":1.0, "end":8.0});
        assert_eq!(
            remap_hook(&hook, &[(6.0, 8.0), (1.0, 3.0)]).unwrap(),
            vec![(0, 0.0, 2.0), (1, 0.0, 2.0)]
        );
    }

    #[test]
    fn temporary_ass_is_removed_when_dropped() {
        let subtitle = write_temporary_ass("test").unwrap();
        assert!(subtitle.0.is_file());
        let path = subtitle.0.clone();
        drop(subtitle);
        assert!(!path.exists());
    }

    struct EnvGuard {
        dir: PathBuf,
        old_state: Option<String>,
        old_whisper: Option<String>,
    }
    impl EnvGuard {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("lfai-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let old_state = std::env::var("LOCAL_FIRST_STATE_DIR").ok();
            let old_whisper = std::env::var("WHISPER_COMMAND").ok();
            std::env::set_var("LOCAL_FIRST_STATE_DIR", &dir);
            EnvGuard {
                dir,
                old_state,
                old_whisper,
            }
        }
        fn state_file(&self) -> PathBuf {
            self.dir.join("runtime.env")
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old_state {
                Some(v) => std::env::set_var("LOCAL_FIRST_STATE_DIR", v),
                None => std::env::remove_var("LOCAL_FIRST_STATE_DIR"),
            }
            match &self.old_whisper {
                Some(v) => std::env::set_var("WHISPER_COMMAND", v),
                None => std::env::remove_var("WHISPER_COMMAND"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn youtube_search_query_is_trimmed_and_bounded() {
        assert_eq!(
            youtube_search_query("  related topic  ").unwrap(),
            "related topic"
        );
        assert_eq!(
            youtube_search_query(" \n ").unwrap_err(),
            "YT_SEARCH_QUERY_EMPTY"
        );
        assert!(youtube_search_query(&"x".repeat(201))
            .unwrap_err()
            .contains("YT_SEARCH_QUERY_TOO_LONG"));
    }

    #[test]
    fn llama_runtime_platform_assets_are_pinned_and_supported() {
        assert_eq!(LLAMA_VERSION, "b11160");
        assert_eq!(LLAMA_RUNTIME_ASSETS.len(), 5);
        for (_, asset, digest) in LLAMA_RUNTIME_ASSETS {
            assert!(asset.starts_with("llama-b11160-bin-"), "{asset}");
            assert_eq!(digest.len(), 64, "{asset}");
            assert!(
                digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{asset}"
            );
        }
        assert_eq!(
            llama_runtime_asset().unwrap().0,
            LLAMA_RUNTIME_ASSETS
                .iter()
                .find(|(platform, _, _)| *platform
                    == if cfg!(target_os = "windows") {
                        "windows-x64"
                    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
                        "macos-arm64"
                    } else if cfg!(target_os = "macos") {
                        "macos-x64"
                    } else if cfg!(target_arch = "aarch64") {
                        "linux-arm64"
                    } else {
                        "linux-x64"
                    })
                .unwrap()
                .1
        );
    }

    #[test]
    fn runtime_server_search_finds_expected_server_name() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("runtime-server");
        let nested = guard.dir.join("bundle/bin");
        std::fs::create_dir_all(&nested).unwrap();
        let server = nested.join(if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        });
        std::fs::write(&server, b"server").unwrap();
        assert_eq!(
            find_runtime_server(&guard.dir, server.file_name().unwrap().to_str().unwrap()),
            Some(server)
        );
    }

    #[test]
    fn sha256_file_hashes_files_without_loading_entire_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("sha256");
        let file = guard.dir.join("hash.txt");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn write_runtime_env_preserves_strategy_key_and_endpoints() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("preserve");
        std::fs::write(guard.state_file(), "HIGHLIGHT_STRATEGY=semantic-gemini\nGEMINI_API_KEY=sk-test-123\nLOCAL_LLM_BASE_URL=http://127.0.0.1:9999\nLOCAL_LLM_MODEL=qwen-test\nHF_TOKEN=hf-preserved-token\n").unwrap();
        write_runtime_env("/opt/custom/whisper", "base").unwrap();
        let text = std::fs::read_to_string(guard.state_file()).unwrap();
        assert!(
            text.contains("WHISPER_COMMAND=/opt/custom/whisper"),
            "{text}"
        );
        assert!(text.contains("WHISPER_MODEL=base"), "{text}");
        // Rebooting the app must not lose the choices the user already made.
        assert!(
            text.contains("HIGHLIGHT_STRATEGY=semantic-gemini"),
            "{text}"
        );
        assert!(text.contains("GEMINI_API_KEY=sk-test-123"), "{text}");
        assert!(
            text.contains("LOCAL_LLM_BASE_URL=http://127.0.0.1:9999"),
            "{text}"
        );
        assert!(text.contains("LOCAL_LLM_MODEL=qwen-test"), "{text}");
        assert!(text.contains("HF_TOKEN=hf-preserved-token"), "{text}");
    }

    #[test]
    fn bootstrap_fast_path_adopts_existing_whisper_without_installing() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("fastpath");
        // A usable whisper (absolute path to an existing file) stands in for a machine
        // that already has one; the fast path must adopt it and never start pip.
        let fake = guard.dir.join("whisper-bin");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("WHISPER_COMMAND", &fake);
        // Any pre-existing choices must survive the bootstrap.
        std::fs::write(guard.state_file(), "HIGHLIGHT_STRATEGY=semantic-local\n").unwrap();

        let app = tauri::test::mock_app();
        let state = BootstrapState(Arc::new(Mutex::new(None)));
        let resolved = run_whisper_bootstrap(app.handle(), &state).expect("fast path must succeed");
        assert_eq!(
            resolved,
            fake.to_string_lossy(),
            "must adopt the existing whisper"
        );

        let progress = state.0.lock().unwrap().clone().expect("progress reported");
        assert!(progress.done && progress.error.is_none(), "{progress:?}");
        let text = std::fs::read_to_string(guard.state_file()).unwrap();
        assert!(text.contains("HIGHLIGHT_STRATEGY=semantic-local"), "{text}");
        assert!(text.contains("WHISPER_COMMAND="), "{text}");
    }

    #[test]
    fn download_and_verify_accepts_good_checksum_and_rejects_bad_one() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("download");
        // file:// URL keeps the test offline: same curl/sha path as the real 2.1 GB download.
        let source = guard.dir.join("weights.bin");
        std::fs::write(&source, b"local-first-llm-model-bytes").unwrap();
        let sha = sha256_file(&source).unwrap();
        let url = format!("file://{}", source.display());
        let target = guard.dir.join("model.gguf");
        let state = ModelDownloadState(Arc::new(Mutex::new(None)));

        let ok = download_and_verify(&state.0, &url, &sha, &target);
        assert!(ok.is_ok(), "expected ok, got {ok:?}");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"local-first-llm-model-bytes"
        );
        assert!(
            !guard.dir.join("model.gguf.part").exists(),
            "partial file must be gone"
        );
        let progress = state.0.lock().unwrap().clone().unwrap();
        assert!(progress.done && progress.phase == "ready", "{progress:?}");

        // Corrupted download: wrong checksum must fail loudly, leave nothing behind.
        std::fs::remove_file(&target).unwrap();
        let bad = download_and_verify(&state.0, &url, &"0".repeat(64), &target);
        let err = bad.expect_err("checksum mismatch must fail");
        assert!(err.contains("MODEL_CHECKSUM_MISMATCH"), "{err}");
        assert!(
            !target.exists(),
            "failed download must not leave a target file"
        );
        assert!(
            !guard.dir.join("model.gguf.part").exists(),
            "failed download must clean the partial"
        );
    }

    #[test]
    fn preview_query_accepts_valid_and_rejects_bad_requests() {
        // The shim builds an HTML page from this query — malformed or hostile
        // input must never reach the page.
        assert_eq!(
            preview_query("id=9bZkp7q19f0&start=9&end=26"),
            Some(("9bZkp7q19f0".to_string(), 9, 26))
        );
        assert_eq!(
            preview_query("id=short&start=9&end=26"),
            None,
            "video id must be 11 chars"
        );
        assert_eq!(
            preview_query("id=<script>x&start=9&end=26"),
            None,
            "id charset is whitelisted"
        );
        assert_eq!(
            preview_query("id=9bZkp7q19f0&start=26&end=9"),
            None,
            "start must precede end"
        );
        assert_eq!(
            preview_query("id=9bZkp7q19f0&start=9"),
            None,
            "end is required"
        );
        assert_eq!(preview_query(""), None);
        let page = preview_page("9bZkp7q19f0", 9, 26);
        assert!(
            page.contains("youtube.com/embed/9bZkp7q19f0?start=9&end=26"),
            "{page}"
        );
    }

    #[test]
    fn download_and_verify_removes_stale_partial_before_starting() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new("stale-part");
        let source = guard.dir.join("weights.bin");
        std::fs::write(&source, b"local-first-llm-model-bytes").unwrap();
        let sha = sha256_file(&source).unwrap();
        let target = guard.dir.join("model.gguf");
        // Simulate a leftover .part from an interrupted download.
        let part = PathBuf::from(format!("{}.part", target.display()));
        std::fs::write(&part, b"stale-bytes").unwrap();
        let state = ModelDownloadState(Arc::new(Mutex::new(None)));
        let ok = download_and_verify(
            &state.0,
            &format!("file://{}", source.display()),
            &sha,
            &target,
        );
        assert!(ok.is_ok(), "expected ok, got {ok:?}");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"local-first-llm-model-bytes"
        );
        assert!(!part.exists(), "stale .part must be cleaned up");
    }
}
