import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { validateTranscript, type Transcript } from "../../packages/contracts/transcript.ts";
import { chooseHeuristicHighlights } from "../../services/ai-pipeline/heuristic.ts";
import { analyzeLocalVideo } from "../../services/ai-pipeline/index.ts";

// path resolution helpers shared by the two test halves below
function repoPath(...parts: string[]): string {
  return fileURLToPath(new URL(`../../${parts.join("/")}`, import.meta.url));
}

const fixturePath = repoPath("tests/desktop-e2e/fixtures/sample-transcript.json");
const transcript = JSON.parse(await readFile(fixturePath, "utf8")) as Transcript;

// The committed fixture must itself be a well-formed transcript.
validateTranscript(transcript);

// The heuristic selector the real pipeline runs after transcription must find at
// least one ≥5s clip — otherwise the E2E "highlight candidates ready" step fails.
const highlights = chooseHeuristicHighlights(transcript);
assert.ok(highlights.length >= 1, `fixture must yield ≥1 highlight, got ${highlights.length}`);
for (const highlight of highlights) {
  assert.ok(highlight.end <= transcript.duration, "highlight must not exceed fixture duration");
}

// Rebinding the artifact id (what loadTranscriptFixture does with the real job's
// id) must keep the transcript valid.
const rebound = { ...transcript, sourceArtifactId: "e2e-artifact" };
validateTranscript(rebound);

// analyzeLocalVideo with an injected transcript never spawns Whisper: the command
// points at a path that cannot exist, so any transcription attempt would fail loudly.
const analysis = await analyzeLocalVideo("/nonexistent/source.mp4", "e2e-artifact", 15.7, {
  command: "/definitely/not/whisper",
  transcript: rebound,
});
assert.equal(analysis.transcript.sourceArtifactId, "e2e-artifact");
assert.ok(analysis.highlights.length >= 1, "injected-transcript analysis must still pick highlights");

// Full worker path: run scripts/analyze-video.ts exactly the way the desktop app
// spawns it, with TRANSCRIPT_FIXTURE set and no Whisper installed. This needs the
// sample video (for the audio-track precheck) and ffprobe on PATH; skip otherwise.
const sampleVideo = repoPath("apps/desktop/src-tauri/yt-short.mp4");
const durationText = "15.765750";
let ffprobeOk = false;
try { await new Promise((resolve, reject) => spawn("ffprobe", ["-version"], { stdio: "ignore" }).on("close", (code) => code === 0 ? resolve(undefined) : reject(new Error(String(code))))); ffprobeOk = true; } catch {}
if (ffprobeOk) {
  const result = await new Promise<string>((resolve, reject) => {
    const child = spawn(process.execPath, [
      "--experimental-strip-types",
      repoPath("scripts/analyze-video.ts"),
      sampleVideo,
      "e2e-artifact",
      durationText,
      "0",
      durationText,
    ], { env: { ...process.env, TRANSCRIPT_FIXTURE: fixturePath, WHISPER_COMMAND: "/definitely/not/whisper" } });
    let stdout = "", stderr = "";
    child.stdout.on("data", (chunk: Buffer) => { stdout += chunk.toString(); });
    child.stderr.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });
    child.on("close", (code) => code === 0 ? resolve(stdout) : reject(new Error(`analyze-video exited ${code}: ${stderr.trim().slice(-300)}`)));
  });
  const payload = JSON.parse(result) as { transcript: Transcript; highlights: Array<{ start: number; end: number }> };
  assert.equal(payload.transcript.sourceArtifactId, "e2e-artifact");
  assert.ok(payload.highlights.length >= 1, `worker analysis must produce highlights, got ${payload.highlights.length}`);
  for (const highlight of payload.highlights) {
    assert.ok(highlight.end > highlight.start, "highlight range must be non-empty");
  }
  console.log("transcript fixture tests: ok (no Whisper needed)");
} else {
  console.log("transcript fixture tests: ok (unit paths; subprocess skipped, ffprobe unavailable)");
}
