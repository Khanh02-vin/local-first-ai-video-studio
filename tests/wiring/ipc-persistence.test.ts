import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
// The Rust shell is split into modules now, so the contract is asserted against every
// source file rather than main.rs alone. The shared IPC structs live in the render
// engine crate, so that tree is read as well.
const rustDirs = [
  join(root, "apps/desktop/src-tauri/src"),
  join(root, "apps/desktop/src-tauri/crates/render-engine/src"),
];
const rust = rustDirs
  .flatMap((dir) => readdirSync(dir).filter((name) => name.endsWith(".rs")).map((name) => join(dir, name)))
  .map((path) => readFileSync(path, "utf8"))
  .join("\n");
const store = readFileSync(join(root, "apps/web/src/lib/project-save-queue.ts"), "utf8");
const studio = readFileSync(join(root, "apps/web/src/lib/pages/Studio.svelte"), "utf8");
const editor = readFileSync(join(root, "apps/web/src/lib/pages/Editor.svelte"), "utf8");
const projects = readFileSync(join(root, "apps/web/src/lib/pages/Project.svelte"), "utf8");
const youtube = readFileSync(join(root, "apps/web/src/lib/pages/Youtube.svelte"), "utf8");

assert.match(rust, /async fn probe_video[\s\S]*?spawn_blocking/, "probe_video I/O must run on a blocking worker");
assert.match(rust, /async fn project_save[\s\S]*?spawn_blocking/, "project save must run on a blocking worker");
// The analysis and media commands each spawn FFmpeg and Python processes, which takes
// seconds. Run on the main thread they freeze the whole webview — the crop-path E2E
// found exactly that, with WebDriver dropping its session while the scan ran. Every
// command that waits on a child process must hand the wait to a blocking worker.
const blockingCommands = ["diarize_video", "sample_speaker_faces", "analyze_crop_path", "analyze_scene_layouts", "generate_transcript_variants"];
for (const command of blockingCommands) {
  assert.match(
    rust,
    new RegExp(`async fn ${command}[\\s\\S]{0,400}?spawn_blocking`),
    `${command} must run on a blocking worker: it spawns FFmpeg/Python and would otherwise block the main thread`,
  );
}
assert.match(rust, /struct ProbeMetadata[\s\S]*struct ProbeFormat[\s\S]*struct ProbeStream/, "probe IPC must return typed metadata");
assert.match(rust, /struct RenderRange[\s\S]*struct RenderPlan[\s\S]*struct RenderRequest/, "render IPC must use a typed request and plan");
assert.match(rust, /on_progress: tauri::ipc::Channel<RenderProgress>/, "render IPC must provide progress via channel");
assert.match(rust, /fn cancel_render[\s\S]*RENDER_CANCEL/, "render IPC must expose cancellation");
assert.match(store, /enqueueProjectSave/, "a shared project save queue must exist");
assert.match(studio, /enqueueProjectSave[\s\S]*project_save/, "Studio project writes must be queued");
assert.match(editor, /enqueueProjectSave[\s\S]*project_save/, "Editor project writes must share the save queue");
assert.match(projects, /studioState\.selectedIds\s*=\s*project\.selectedIds/, "Projects page must restore shared selectedIds");
assert.match(studio, /new Channel<RenderProgress>/, "Studio must receive render progress");
assert.match(studio, /invoke\("cancel_render"\)/, "Studio must expose render cancellation");
assert.match(studio, /if \(rendering\) \{ await new Promise\(\(resolve\) => setTimeout\(resolve, 1000\)\); continue; \}/, "a finished analysis poll must not clear busy while a render runs");
assert.match(studio, /!watchStarted && !rendering/, "the analysis watcher must not restart while a render runs");
assert.match(studio, /!watchStarted && !rendering/, "the analysis watcher must not restart while a render runs");
// System Doctor panel in Settings must expose one-click check + Whisper bootstrap
const settings = readFileSync(join(root, "apps/web/src/lib/pages/Settings.svelte"), "utf8");
assert.match(settings, /System Doctor/, "Settings must have a System Doctor panel");
assert.match(settings, /runSystemDoctor/, "System Doctor must have a runSystemDoctor function");
assert.match(settings, /startWhisperBootstrap/, "System Doctor must invoke the shared bootstrap function");
assert.match(settings, /whisperBootstrap/, "Settings must display Whisper bootstrap progress");
assert.match(settings, /runtime_status/, "System Doctor must call runtime_status for the component check");
assert.match(settings, /bootstrap_whisper/, "System Doctor must be able to start Whisper bootstrap");
assert.match(rust, /LFAI_RAG_TOKEN/, "the RAG sidecar must be spawned with a session token");
const ragSidecar = readFileSync(join(root, "apps/web/src/routes/api/rag/+server.ts"), "utf8");
assert.match(ragSidecar, /authorized\(request\)/, "every RAG handler must check the session token");
assert.match(ragSidecar, /LFAI_RAG_TOKEN/, "the RAG sidecar must read its token from the spawn env");
assert.match(youtube, /authorization/, "the UI must send the sidecar token on RAG calls");
assert.match(youtube, /invoke<\{ base: string; token: string \}>\("rag_server_url"\)/, "rag_server_url must hand the webview both the base URL and its session token");
// Ephemeral ports: the sidecars must resolve a free port at spawn time. A fallback
// *URL* string is fine (external servers); a hardcoded bind is not.
assert.match(rust, /free_loopback_port\(\)/, "sidecar ports must come from an ephemeral bind, not a constant");
assert.match(rust, /session_token\(\)/, "sidecar sessions must carry a per-session token");
assert.doesNotMatch(rust, /RAG_PORT: u16 = /, "the fixed RAG port must stay gone");

// Windows worker control (ADR-004): cancel/liveness use taskkill/tasklist through
// std::process::Command — the cfg(windows) blocks only compile on Windows, so these
// source guards pin the shapes that a typo would break. The ci workflow's windows job
// compiles and runs them for real.
assert.match(rust, /taskkill[\s\S]{0,120}?\/F[\s\S]{0,40}?\/T[\s\S]{0,40}?\/PID/, "windows cancel must kill the whole process tree (/T)");
assert.match(rust, /tasklist[\s\S]{0,80}?PID eq/, "the windows liveness probe must filter tasklist by pid");
assert.match(rust, /cmd\.group\(\)[\s\S]{0,200}?group\.spawn\(\)/, "the analysis worker must be spawned in a group/job object");
assert.match(rust, /group\.creation_flags\(/, "worker creation flags must go through the group builder — its spawn() overwrites the Command's own flags");

console.log("IPC and shared persistence contract tests: ok");
