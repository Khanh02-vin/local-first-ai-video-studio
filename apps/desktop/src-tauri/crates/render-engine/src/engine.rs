// The render engine: the one place that turns a RenderPlan into FFmpeg arguments.
//
// Both front doors — the desktop app and the headless CLI — call `render_segments`
// here, so there is exactly one final renderer (ADR-001: "FFmpeg là final renderer
// authoritative", "Một final renderer"). The engine knows nothing about Tauri: the
// caller supplies the progress sink and the cancel flag.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contracts::RenderProgress;
use crate::process::tool;

/// Where the engine reports render progress. The app forwards this to the webview
/// channel; the CLI prints it. The engine itself never talks to a UI.
pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: RenderProgress);
}

impl<F> ProgressSink for F
where
    F: Fn(RenderProgress) + Send + Sync,
{
    fn report(&self, progress: RenderProgress) {
        self(progress);
    }
}

/// Set while a render is being asked to stop; the FFmpeg child checks it while running.
pub static RENDER_CANCELLED: AtomicBool = AtomicBool::new(false);
/// The FFmpeg child currently running, so a cancel can kill it.
pub static ACTIVE_RENDER_CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);

/// FFmpeg's `-progress` file logs how much of the input it has encoded so far
/// (`out_time_us=`), which turns a render into a smooth 0→100 ramp instead of a single
/// jump when the child exits.
pub fn render_fraction(path: &Path, span_seconds: f64) -> Option<f64> {
    // `!(x > 0.0)` rather than `x <= 0.0`: the negated form also rejects NaN.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(span_seconds > 0.0) {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let encoded = text.lines().rev().find_map(|line| {
        line.strip_prefix("out_time_us=")
            .and_then(|value| value.trim().parse::<f64>().ok())
            .map(|micros| micros / 1_000_000.0)
            .or_else(|| {
                line.strip_prefix("out_time_ms=")
                    .and_then(|value| value.trim().parse::<f64>().ok())
                    .map(|millis| millis / 1000.0)
            })
    })?;
    Some((encoded / span_seconds).clamp(0.0, 1.0))
}

/// Spawns one FFmpeg child and watches it until it exits, the cancel flag flips, or it
/// has to be killed. `window` is the (start, end) slice of the overall 0→100 progress
/// this child owns; `span_seconds` is how much source this child encodes, which is what
/// turns FFmpeg's `out_time` into a percentage of that slice.
pub fn run_render_child(
    command: &mut std::process::Command,
    on_progress: &dyn ProgressSink,
    window: (f64, f64),
    span_seconds: f64,
    phase: &str,
) -> Result<(), String> {
    if RENDER_CANCELLED.load(Ordering::SeqCst) {
        return Err("RENDER_CANCELLED".into());
    }
    let progress_log = std::env::temp_dir().join(format!("local-first-render-progress-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&progress_log);
    let progress_arg = progress_log.to_string_lossy().into_owned();
    command.args(["-progress", progress_arg.as_str()]);
    let child = command.spawn().map_err(|error| format!("FFMPEG_SPAWN:{error}"))?;
    {
        let mut active = ACTIVE_RENDER_CHILD.lock().map_err(|_| "RENDER_STATE_LOCK_POISONED")?;
        *active = Some(child);
    }
    loop {
        if RENDER_CANCELLED.load(Ordering::SeqCst) {
            if let Ok(mut active) = ACTIVE_RENDER_CHILD.lock() {
                if let Some(child) = active.as_mut() { let _ = child.kill(); }
            }
        }
        let status = {
            let mut active = ACTIVE_RENDER_CHILD.lock().map_err(|_| "RENDER_STATE_LOCK_POISONED")?;
            match active.as_mut() {
                Some(child) => child.try_wait().map_err(|error| format!("FFMPEG_WAIT:{error}"))?,
                None => return Err("RENDER_CHILD_MISSING".into()),
            }
        };
        if let Some(status) = status {
            if let Ok(mut active) = ACTIVE_RENDER_CHILD.lock() { active.take(); }
            if RENDER_CANCELLED.load(Ordering::SeqCst) { return Err("RENDER_CANCELLED".into()); }
            if !status.success() { return Err(format!("FFMPEG_EXIT:{status}")); }
            on_progress.report(RenderProgress { percent: window.1, phase: phase.to_string() });
            return Ok(());
        }
        if let Some(fraction) = render_fraction(&progress_log, span_seconds) {
            let percent = window.0 + fraction * (window.1 - window.0);
            on_progress.report(RenderProgress { percent, phase: phase.to_string() });
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Reads the container duration of a media file with ffprobe.
pub fn probe_duration(ffprobe: &Path, input: &str) -> Result<f64, String> {
    let output = tool(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
        .output()
        .map_err(|e| format!("FFPROBE_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "INVALID_SOURCE_DURATION".to_string())
}

/// True when the media file carries an audio stream; a video without audio gets a
/// synthetic silent track so the output still has an audio stream.
fn audio_present(ffprobe: &Path, media: &[String]) -> bool {
    let index = if media.len() > 1 { 1 } else { 0 };
    let probe = tool(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(&media[index])
        .output()
        .map_err(|e| format!("FFPROBE_SPAWN:{e}"));
    matches!(probe, Ok(probe) if probe.status.success() && !probe.stdout.is_empty())
}

pub fn remapped_captions_for_range(
    captions: Option<&Vec<serde_json::Value>>,
    ranges: &[(f64, f64)],
    index: usize,
) -> Result<Option<String>, String> {
    let Some(captions) = captions else {
        return Ok(None);
    };
    let (start, end) = ranges[index];
    ass_dialogue(captions, start, end - start)
}

pub fn remap_hook(
    hook: &serde_json::Value,
    ranges: &[(f64, f64)],
) -> Result<Vec<(usize, f64, f64)>, String> {
    let start = hook["start"].as_f64().ok_or("INVALID_RENDER_HOOK")?;
    let end = hook["end"].as_f64().ok_or("INVALID_RENDER_HOOK")?;
    if !start.is_finite() || !end.is_finite() || end <= start {
        return Err("INVALID_RENDER_HOOK".into());
    }
    Ok(ranges
        .iter()
        .enumerate()
        .filter_map(|(index, (range_start, range_end))| {
            let from = start.max(*range_start);
            let to = end.min(*range_end);
            (to > from).then_some((index, from - range_start, to - range_start))
        })
        .collect())
}

pub struct TemporarySubtitle(pub PathBuf);

pub struct TemporaryRenderDir(pub PathBuf);

pub fn create_temporary_render_dir() -> Result<TemporaryRenderDir, String> {
    for attempt in 0..100 {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("CLOCK:{e}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lfai-render-{}-{suffix}-{attempt}",
            std::process::id()
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(TemporaryRenderDir(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("RENDER_TEMP_CREATE:{error}")),
        }
    }
    Err("RENDER_TEMP_COLLISION".into())
}

pub fn subject_pair_continues(previous: &[serde_json::Value], current: &[serde_json::Value]) -> bool {
    if previous.len() != 2 || current.len() != 2 {
        return false;
    }
    let order = |items: &[serde_json::Value]| {
        let mut v = items.to_vec();
        v.sort_by(|a, b| {
            a["x"]
                .as_f64()
                .unwrap_or(0.5)
                .total_cmp(&b["x"].as_f64().unwrap_or(0.5))
        });
        v
    };
    let (a, b) = (order(previous), order(current));
    (0..2).all(|i| {
        let center = |v: &serde_json::Value| {
            (
                v["x"].as_f64().unwrap_or(0.0) + v["width"].as_f64().unwrap_or(0.0) / 2.0,
                v["y"].as_f64().unwrap_or(0.0) + v["height"].as_f64().unwrap_or(0.0) / 2.0,
            )
        };
        let (ax, ay) = center(&a[i]);
        let (bx, by) = center(&b[i]);
        (ax - bx).hypot(ay - by) <= 0.2
    })
}

pub fn stable_subject_pair(samples: &[Vec<serde_json::Value>]) -> Option<[serde_json::Value; 2]> {
    if samples.len() < 3 || samples.iter().any(|sample| sample.len() != 2) {
        return None;
    }
    let ordered: Vec<Vec<serde_json::Value>> = samples
        .iter()
        .map(|sample| {
            let mut values = sample.clone();
            values.sort_by(|a, b| {
                a["x"]
                    .as_f64()
                    .unwrap_or(0.5)
                    .total_cmp(&b["x"].as_f64().unwrap_or(0.5))
            });
            values
        })
        .collect();
    for pair in ordered.windows(2) {
        for (first, second) in pair[0].iter().zip(pair[1].iter()) {
            let center = |b: &serde_json::Value| {
                (
                    b["x"].as_f64().unwrap_or(0.0) + b["width"].as_f64().unwrap_or(0.0) / 2.0,
                    b["y"].as_f64().unwrap_or(0.0) + b["height"].as_f64().unwrap_or(0.0) / 2.0,
                )
            };
            let (ax, ay) = center(first);
            let (bx, by) = center(second);
            if (ax - bx).hypot(ay - by) > 0.2 {
                return None;
            }
        }
    }
    let median = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        let middle = values.len() / 2;
        if values.len().is_multiple_of(2) {
            (values[middle - 1] + values[middle]) / 2.0
        } else {
            values[middle]
        }
    };
    let representative = std::array::from_fn(|index| {
        let mut box_value = ordered[0][index].clone();
        for field in ["x", "y", "width", "height", "confidence"] {
            let value = median(
                ordered
                    .iter()
                    .filter_map(|sample| sample[index][field].as_f64())
                    .collect(),
            );
            box_value[field] = serde_json::json!(value);
        }
        box_value
    });
    Some(representative)
}

pub fn side_by_side_graph(
    boxes: &[serde_json::Value],
    post_filters: &[String],
    ratio: &str,
) -> Option<String> {
    if boxes.len() != 2 {
        return None;
    }
    let center = |b: &serde_json::Value| {
        let (x, y, w, h, c) = (
            b["x"].as_f64()?,
            b["y"].as_f64()?,
            b["width"].as_f64()?,
            b["height"].as_f64()?,
            b["confidence"].as_f64()?,
        );
        (x.is_finite()
            && y.is_finite()
            && w.is_finite()
            && h.is_finite()
            && c.is_finite()
            && x >= 0.0
            && y >= 0.0
            && w > 0.0
            && h > 0.0
            && x + w <= 1.0
            && y + h <= 1.0
            && (0.0..=1.0).contains(&c))
        .then_some((x + w / 2.0, y + h / 2.0))
    };
    let centers = [center(&boxes[0])?, center(&boxes[1])?];
    if centers[0].0 >= centers[1].0 {
        return None;
    }
    let (panel_w, panel_h, out_size) = match ratio {
        "1:1" => ("540", "1080", "1080:1080"),
        "16:9" => ("960", "1080", "1920:1080"),
        _ => ("540", "1920", "1080:1920"),
    };
    let mut graph = String::from("[0:v]split=2[lin][rin];");
    for (index, (cx, cy)) in centers.iter().enumerate() {
        let (input, output) = if index == 0 {
            ("lin", "left")
        } else {
            ("rin", "right")
        };
        graph.push_str(&format!("[{input}]crop=ih*{panel_w}/{panel_h}:ih:max(0\\,min(iw-ih*{panel_w}/{panel_h}\\,{cx:.6}*iw-ih*{panel_w}/{panel_h}/2)):max(0\\,min(ih-ih\\,{cy:.6}*ih-ih/2)),scale={panel_w}:{panel_h}[{output}];"));
    }
    graph.push_str(&format!(
        "[left][right]hstack=inputs=2,scale={out_size},setsar=1{}[vout]",
        if post_filters.is_empty() {
            String::new()
        } else {
            format!(",{}", post_filters.join(","))
        }
    ));
    Some(graph)
}

/// Renders the given source ranges into `output`.
///
/// `media` is the resolved source list: one local path, or two when a separate audio
/// track was downloaded. `plan` carries the optional crop/caption/hook information the
/// editor produced; `None` renders a plain centre-cropped clip.
#[allow(clippy::too_many_arguments)]
pub fn render_segments(
    ffmpeg: &Path,
    ffprobe: &Path,
    media: &[String],
    output: &Path,
    ranges: &[(f64, f64)],
    aspect_ratio: Option<String>,
    plan: Option<&serde_json::Value>,
    on_progress: &dyn ProgressSink,
) -> Result<String, String> {
    if ranges.is_empty() {
        return Err("INVALID_RENDER_RANGES".into());
    }
    for (index, (start, end)) in ranges.iter().enumerate() {
        if !start.is_finite()
            || !end.is_finite()
            || *start < 0.0
            || *end <= *start
            || ranges[..index]
                .iter()
                .any(|(from, to)| *start < *to && *from < *end)
        {
            return Err("INVALID_RENDER_RANGES".into());
        }
    }
    let output_path = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    if media.is_empty() || media.len() > 2 {
        return Err("INVALID_RENDER_MEDIA".into());
    }
    if !media[0].starts_with("http://") && !media[0].starts_with("https://") {
        let input_path = Path::new(&media[0])
            .canonicalize()
            .map_err(|e| format!("INPUT_PATH:{e}"))?;
        if input_path == output_path {
            return Err("OUTPUT_MUST_DIFFER_FROM_INPUT".into());
        }
        if !input_path.is_file() {
            return Err("INPUT_NOT_FILE".into());
        }
    }
    let total_duration: f64 = ranges.iter().map(|(start, end)| end - start).sum();
    if !total_duration.is_finite() || total_duration <= 0.0 {
        return Err("INVALID_RENDER_RANGES".into());
    }
    if let Ok(probe) = tool(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(&media[0])
        .output()
    {
        if probe.status.success() {
            if let Ok(source_duration) =
                String::from_utf8_lossy(&probe.stdout).trim().parse::<f64>()
            {
                if ranges.iter().any(|(_, end)| *end > source_duration + 0.05) {
                    return Err("INVALID_RENDER_RANGES".into());
                }
            }
        }
    }
    let has_audio = audio_present(ffprobe, media);
    let ratio = aspect_ratio.unwrap_or_else(|| "9:16".into());
    let crop = match ratio.as_str() {
        "1:1" => "crop=ih:ih:(iw-ih)/2:0,scale=1080:1080".to_string(),
        "16:9" => "crop=ih*16/9:ih:(iw-ih*16/9)/2:0,scale=1920:1080".to_string(),
        _ => "crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=1080:1920".to_string(),
    };
    let keyframes = plan
        .and_then(|value| value.get("cropKeyframes"))
        .and_then(serde_json::Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= 600)
        .cloned()
        .unwrap_or_default();
    let keyframes: Vec<_> = keyframes
        .into_iter()
        .filter(|key| {
            key["confirmed"].as_bool() == Some(true)
                && key["sourceArtifactId"].as_str()
                    == plan.and_then(|value| value["sourceArtifactId"].as_str())
                && key["confidence"]
                    .as_f64()
                    .is_some_and(|confidence| confidence >= 0.5)
                && key["centerX"]
                    .as_f64()
                    .is_some_and(|center| (0.0..=1.0).contains(&center))
                && key["time"].as_f64().is_some_and(|time| time.is_finite())
                && key["speakerId"].as_str().is_some_and(|speaker| {
                    plan.and_then(|value| value["speakerMap"][speaker].as_str())
                        .is_some_and(|face| key["sourceFaceId"].as_str() == Some(face))
                })
        })
        .collect();
    let scene_layouts: Vec<_> = plan
        .and_then(|value| value["sceneLayouts"].as_array())
        .map(|items| {
            items
                .iter()
                .filter(|scene| {
                    scene["confirmed"].as_bool() == Some(true)
                        && scene["sourceArtifactId"].as_str()
                            == plan.and_then(|value| value["sourceArtifactId"].as_str())
                        && scene["start"]
                            .as_f64()
                            .is_some_and(|start| start.is_finite() && start >= 0.0)
                        && scene["end"].as_f64().is_some_and(|end| {
                            end.is_finite() && end > scene["start"].as_f64().unwrap_or(end)
                        })
                        && ["solo", "side-by-side"]
                            .contains(&scene["layout"].as_str().unwrap_or(""))
                })
                .take(600)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let crop_for_range = |start: f64| -> String {
        if keyframes.is_empty() {
            return crop.clone();
        }
        let expression = |axis: &str| {
            let value = |key: &serde_json::Value| {
                key[axis].as_f64().unwrap_or(0.5).clamp(0.0, 1.0)
            };
            let mut result = format!("{}", value(keyframes.last().unwrap()));
            for index in (0..keyframes.len()).rev() {
                let key = &keyframes[index];
                let time = key["time"].as_f64().unwrap_or(start) - start;
                if index == 0 {
                    result = format!("if(lt(t\\,{time:.3})\\,{}\\,{result})", value(key));
                } else {
                    let previous = &keyframes[index - 1];
                    let from = (previous["time"].as_f64().unwrap_or(start) - start).max(0.0);
                    let to = time.max(0.0);
                    if to <= 0.0 {
                        continue;
                    }
                    let t1 = previous["time"].as_f64().unwrap_or(start);
                    let t2 = key["time"].as_f64().unwrap_or(t1 + 1.0);
                    let slope = (value(key) - value(previous)) / (t2 - t1);
                    let intercept = value(previous) - slope * t1 + slope * start;
                    result = format!("if(lt(t\\,{from:.3})\\,{}\\,if(lt(t\\,{to:.3})\\,{slope:.6}*t+{intercept:.6}\\,{result}))", value(previous));
                }
            }
            result
        };
        let dimensions = match ratio.as_str() {
            "1:1" => ("ih", "ih", "1080:1080"),
            "16:9" => ("ih*16/9", "ih", "1920:1080"),
            _ => ("ih*9/16", "ih", "1080:1920"),
        };
        format!(
            "crop={}:{}:max(0\\,min(iw-({})\\,({}*iw-({})/2))):max(0\\,min(ih-({})\\,({}*ih-({})/2))),scale={}",
            dimensions.0,
            dimensions.1,
            dimensions.0,
            expression("centerX"),
            dimensions.0,
            dimensions.1,
            expression("centerY"),
            dimensions.1,
            dimensions.2
        )
    };
    fn drawtext_escape(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace(':', "\\:")
            .replace('\'', "\\'")
            .replace('%', "\\%")
            .replace(',', "\\,")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace(';', "\\;")
    }
    let captions = plan
        .and_then(|value| value.get("captions"))
        .and_then(serde_json::Value::as_array);
    let hook_intervals = if let Some(hook) = plan
        .and_then(|value| value.get("hook"))
        .filter(|hook| !hook.is_null())
    {
        remap_hook(hook, ranges)?
    } else {
        Vec::new()
    };
    let temp = create_temporary_render_dir()?;
    let mut segments = Vec::new();
    for (index, (start, end)) in ranges.iter().enumerate() {
        let mut boundaries: Vec<f64> = vec![*start, *end];
        let mut cut_scenes: Vec<u64> = keyframes
            .iter()
            .filter_map(|key| key["sceneId"].as_u64())
            .collect();
        cut_scenes.sort_unstable();
        cut_scenes.dedup();
        for key in &keyframes {
            let key_time = key["time"].as_f64().unwrap_or(*start);
            if key["sceneId"]
                .as_u64()
                .is_some_and(|scene| cut_scenes.contains(&scene))
                && key_time > *start
                && key_time < *end
                && keyframes.iter().any(|other| {
                    other["time"]
                        .as_f64()
                        .is_some_and(|other_time| other_time < key_time)
                        && other["sceneId"] != key["sceneId"]
                })
            {
                boundaries.push(key_time);
            }
        }
        for scene in &scene_layouts {
            let scene_start = scene["start"].as_f64().unwrap_or(*start);
            let scene_end = scene["end"].as_f64().unwrap_or(*end);
            if scene_start > *start && scene_start < *end {
                boundaries.push(scene_start);
            }
            if scene_end > *start && scene_end < *end {
                boundaries.push(scene_end);
            }
        }
        boundaries.sort_by(f64::total_cmp);
        boundaries.dedup_by(|a, b| (*a - *b).abs() < 0.001);
        let windows_count = boundaries.len().saturating_sub(1).max(1) as f64;
        for (piece_index, window) in boundaries.windows(2).enumerate() {
            let piece_start = window[0];
            let piece_end = window[1];
            let duration = piece_end - piece_start;
            let segment = temp
                .0
                .join(format!("segment-{index:04}-{piece_index:03}.mp4"));
            let active_scene = scene_layouts.iter().find(|scene| {
                scene["start"]
                    .as_f64()
                    .is_some_and(|value| piece_start >= value)
                    && scene["end"]
                        .as_f64()
                        .is_some_and(|value| piece_start < value)
            });
            let side_by_side = active_scene
                .filter(|scene| scene["layout"] == "side-by-side")
                .and_then(|scene| scene["subjectBoxes"].as_array())
                .and_then(|boxes| side_by_side_graph(boxes, &[], &ratio));
            let mut filters = if side_by_side.is_some() {
                Vec::new()
            } else {
                vec![crop_for_range(piece_start)]
            };
            let mut complex_graph_filters = Vec::new();
            let mut subtitle_file = None;
            let piece_hook = hook_intervals
                .iter()
                .filter(|(hook_index, _, _)| *hook_index == index)
                .filter_map(|(_, from, to)| {
                    let absolute_from = start + from;
                    let absolute_to = start + to;
                    let clipped_from = absolute_from.max(piece_start);
                    let clipped_to = absolute_to.min(piece_end);
                    (clipped_to > clipped_from)
                        .then_some((clipped_from - piece_start, clipped_to - piece_start))
                });
            for (from, to) in piece_hook {
                let text = plan
                    .and_then(|value| value.get("hook"))
                    .and_then(|hook| hook.get("text"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or("INVALID_RENDER_HOOK")?;
                filters.push(format!("drawtext=text='{}':x=(w-text_w)/2:y=h*0.12:fontsize=h/18:fontcolor=white:borderw=3:enable='between(t,{from:.3},{to:.3})'", drawtext_escape(text)));
            }
            let scene_subtitle_ranges = [(piece_start, piece_end)];
            if let Some(contents) =
                remapped_captions_for_range(captions, &scene_subtitle_ranges, 0)?
            {
                let subtitle = write_temporary_ass(&contents)?;
                let filter = format!(
                    "ass='{}'",
                    escape_ass_filter_path(&subtitle.0.to_string_lossy())
                );
                if side_by_side.is_some() {
                    complex_graph_filters.push(filter);
                } else {
                    filters.push(filter);
                }
                subtitle_file = Some(subtitle);
            }
            filters.push("setsar=1".to_string());
            let complex_graph = side_by_side.as_ref().and_then(|_| {
                side_by_side_graph(
                    active_scene?.get("subjectBoxes")?.as_array()?,
                    &complex_graph_filters,
                    &ratio,
                )
            });
            let mut cmd = tool(ffmpeg);
            cmd.args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error"]);
            for source in media {
                cmd.args(["-ss"])
                    .arg(format!("{piece_start:.3}"))
                    .args(["-t"])
                    .arg(format!("{duration:.3}"))
                    .args(["-i"])
                    .arg(source);
            }
            if let Some(graph) = complex_graph.as_ref() {
                cmd.args(["-filter_complex", graph, "-map", "[vout]"]);
            } else {
                cmd.args(["-vf", &filters.join(","), "-map", "0:v:0"]);
            }
            cmd.args([
                "-c:v", "libx264", "-preset", "fast", "-crf", "22", "-pix_fmt", "yuv420p", "-r",
                "30",
            ]);
            if has_audio {
                let audio_index = if media.len() > 1 { 1 } else { 0 };
                cmd.args(["-map"])
                    .arg(format!("{audio_index}:a:0"))
                    .args(["-c:a", "aac", "-b:a", "128k"]);
            } else {
                let audio_input_index = media.len();
                cmd.args([
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=channel_layout=stereo:sample_rate=44100",
                    "-t",
                ])
                .arg(format!("{duration:.3}"))
                .args(["-map", "0:v:0", "-map"])
                .arg(format!("{audio_input_index}:a:0"))
                .args(["-c:a", "aac", "-b:a", "128k"]);
            }
            let window_start = ((index as f64 + piece_index as f64 / windows_count) / ranges.len() as f64) * 90.0;
            let window_end = ((index as f64 + (piece_index + 1) as f64 / windows_count) / ranges.len() as f64) * 90.0;
            run_render_child(cmd.arg(&segment), on_progress, (window_start, window_end), (end - start) / windows_count as f64, "segments")?;
            drop(subtitle_file);
            segments.push(segment);
        }
    }
    let parent = output_path.parent().ok_or("OUTPUT_PARENT_MISSING")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("OUTPUT_DIR:{e}"))?;
    let partial = temp.0.join("joined.mp4");
    let mut cmd = tool(ffmpeg);
    cmd.args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error"]);
    for segment in &segments {
        cmd.arg("-i").arg(segment);
    }
    let count = segments.len();
    let mut graph = String::new();
    if has_audio {
        for index in 0..count {
            graph.push_str(&format!("[{index}:v:0][{index}:a:0]"));
        }
        graph.push_str(&format!(
            "concat=n={count}:v=1:a=1[v0][aout];[v0]setsar=1[vout]"
        ));
    } else {
        for index in 0..count {
            graph.push_str(&format!("[{index}:v:0]"));
        }
        graph.push_str(&format!("concat=n={count}:v=1:a=0[vout]"));
    }
    cmd.args([
        "-filter_complex",
        &graph,
        "-map",
        "[vout]",
        "-c:v",
        "libx264",
        "-preset",
        "fast",
        "-crf",
        "22",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
    ]);
    if has_audio {
        cmd.args(["-map", "[aout]", "-c:a", "aac", "-b:a", "128k"]);
    }
    run_render_child(cmd.arg(&partial), on_progress, (90.0, 98.0), total_duration, "joining")?;
    std::fs::copy(&partial, &output_path).map_err(|e| format!("OUTPUT_COPY:{e}"))?;
    on_progress.report(RenderProgress { percent: 100.0, phase: "completed".into() });
    Ok(output_path.to_string_lossy().into_owned())
}

pub fn ass_timestamp(seconds: f64) -> String {
    let centiseconds = (seconds.max(0.0) * 100.0).floor() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        centiseconds / 360_000,
        centiseconds / 6_000 % 60,
        centiseconds / 100 % 60,
        centiseconds % 100
    )
}

pub fn escape_ass_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace(['\r', '\n'], " ")
}

pub fn ass_dialogue(
    captions: &[serde_json::Value],
    start: f64,
    duration: f64,
) -> Result<Option<String>, String> {
    if !start.is_finite() || start < 0.0 || !duration.is_finite() || duration <= 0.0 {
        return Err("INVALID_RENDER_REQUEST".into());
    }
    let end = start + duration;
    let mut events = Vec::new();
    for caption in captions {
        let text = caption["text"].as_str().ok_or("INVALID_RENDER_CAPTION")?;
        let cue_start = caption["start"].as_f64().ok_or("INVALID_RENDER_CAPTION")?;
        let cue_end = caption["end"].as_f64().ok_or("INVALID_RENDER_CAPTION")?;
        if !cue_start.is_finite() || !cue_end.is_finite() || cue_end <= cue_start {
            return Err("INVALID_RENDER_CAPTION".into());
        }
        let from = cue_start.max(start);
        let to = cue_end.min(end);
        if to > from {
            events.push(format!(
                "Dialogue: 0,{},{},Shorts,,0,0,0,,{}",
                ass_timestamp(from - start),
                ass_timestamp(to - start),
                escape_ass_text(text)
            ));
        }
    }
    if events.is_empty() {
        return Ok(None);
    }
    let header = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1080\nPlayResY: 1920\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Shorts,Arial,70,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,1,0,1,6,0,2,40,40,650,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
    Ok(Some(format!("{header}{}\n", events.join("\n"))))
}

pub fn write_temporary_ass(contents: &str) -> Result<TemporarySubtitle, String> {
    use std::io::Write;
    for attempt in 0..100 {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("CLOCK:{e}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lfai-subtitle-{}-{suffix}-{attempt}.ass",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(contents.as_bytes()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("SUBTITLE_WRITE:{error}"));
                }
                return Ok(TemporarySubtitle(path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("SUBTITLE_CREATE:{error}")),
        }
    }
    Err("SUBTITLE_TEMP_COLLISION".into())
}

pub fn escape_ass_filter_path(path: &str) -> String {
    path.replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace(',', "\\,")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(';', "\\;")
}

impl Drop for TemporaryRenderDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Drop for TemporarySubtitle {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
