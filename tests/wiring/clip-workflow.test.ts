import { strict as assert } from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const studio = readFileSync(join(root, "apps/web/src/lib/pages/Studio.svelte"), "utf8");
const editor = readFileSync(join(root, "apps/web/src/lib/pages/Editor.svelte"), "utf8");
const rustDirs = [
  join(root, "apps/desktop/src-tauri/src"),
  join(root, "apps/desktop/src-tauri/crates/render-engine/src"),
];
// The Rust shell lives in modules now and the render path lives in the render engine
// crate, so the bridge is read from both trees.
const bridge = rustDirs
  .flatMap((dir) => readdirSync(dir).filter((name) => name.endsWith(".rs")).map((name) => join(dir, name)))
  .map((path) => readFileSync(path, "utf8"))
  .join("\n");
const store = readFileSync(join(root, "apps/desktop/src-tauri/src/project_store.rs"), "utf8");
const ranking = readFileSync(join(root, "packages/media-engine/related-videos/index.ts"), "utf8");

assert.match(studio, /selectedIds\s*=\s*\$state<string\[]>/, "Studio must support persisted multi-selection");
assert.match(studio, /function toggleCandidate/, "candidate selection must be toggleable");
assert.match(studio, /async function renderSelected/, "selected clips must render as a batch");
assert.match(studio, /request:\s*\{ input:\s*inputPath,\s*output,\s*plan\s*\}\s*satisfies\s*RenderRequest/, "render must pass input, output and the selected RenderPlan as one typed request");
assert.match(studio, /selectedIds\s*,\s*plans:/, "multi-selection must be saved on the project");
assert.match(bridge, /fn render_video\([\s\S]*request:\s*RenderRequest[\s\S]*on_progress:\s*tauri::ipc::Channel<RenderProgress>/, "render command must take a typed request and a progress channel");
assert.match(studio, /generate_transcript_variants/, "Studio must request AI transcript variants via Tauri");
assert.match(studio, /async function renderVariant/, "each transcript variant must export independently");
assert.match(editor, /createEditorStateFromVariant/, "Editor must build a RenderPlan from transcript variants");
assert.match(editor, /variant:\$\{editingVariantId\}/, "variant edits must not overwrite highlight plans");
assert.match(bridge, /fn render_segments[\s\S]*concat=n=\{count\}/, "desktop export must concatenate rendered ranges");
assert.match(studio, /async function runDiarization[\s\S]*applySpeakerIntervals/, "Studio must map local diarization intervals onto transcript words");
assert.match(studio, /async function sampleFaces[\s\S]*sample_speaker_faces/, "Studio must request temporary face samples");
assert.match(studio, /function confirmSpeakerMatches[\s\S]*confirmed: true/, "face matching must require explicit confirmation");
assert.match(studio, /speakerFaceMatches\s*:\s*\{ version: 1/, "confirmed mapping must be project-persisted");
assert.match(store, /speaker_face_matches:\s*Option<Value>/, "speaker mapping must remain optional for legacy projects");
assert.match(bridge, /DIARIZATION_LOCAL_SOURCE_ONLY/, "remote sources must be refused by local analysis commands");
assert.match(studio, /diarize_video/, "Studio must invoke local speaker diarization");
assert.match(studio, /sample_speaker_faces/, "Studio must request bounded local face samples");
assert.match(studio, /speakerMatchesConfirmed/, "speaker-to-face matching needs explicit user confirmation");
assert.match(bridge, /DIARIZATION_LOCAL_SOURCE_ONLY/, "remote media must not enter local diarization or frame extraction");
assert.match(bridge, /HF_TOKEN/, "diarization token must only be passed to the local worker environment");
assert.match(bridge, /speaker_face_matches/, "speaker-to-face confirmation must persist on the project");
assert.match(studio, /diarization_setup_status/, "Studio must check opt-in diarization runtime status");
assert.match(studio, /speaker-face-match/, "Studio must expose speaker/face confirmation UI");
assert.match(bridge, /\.arg\(&frame\)/, "face detector must read frames from a temporary directory");
assert.match(bridge, /ass_dialogue\(captions, start, end - start\)/, "each segment must clip captions to its source interval");
assert.match(bridge, /drawtext=text=/, "hook overlay must remain available for each segment");
assert.doesNotMatch(bridge, /drawtext=text='\{\}':x=\(w-text_w\)\/2:y=h\*0\.78/, "captions must not be rendered as per-cue drawtext");
assert.match(ranking, /rankCandidates/, "related results must be reranked from source semantics");

console.log("clip workflow wiring tests: ok");
