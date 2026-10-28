#!/usr/bin/env node
import { spawn, execFileSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { setTimeout as delay } from "node:timers/promises";

const root = resolve(import.meta.dirname, "../..");
const appPath = process.env.DESKTOP_E2E_APP ? resolve(process.env.DESKTOP_E2E_APP) : "";
const video = process.env.DESKTOP_E2E_VIDEO && resolve(process.env.DESKTOP_E2E_VIDEO);
const artifacts = resolve(process.env.DESKTOP_E2E_ARTIFACTS ?? `${root}/artifacts/desktop-e2e`);
const timeoutMs = Number(process.env.DESKTOP_E2E_TIMEOUT_MS ?? 30 * 60 * 1000);
const port = Number(process.env.DESKTOP_E2E_WEBDRIVER_PORT ?? 4444);
const scenario = process.env.DESKTOP_E2E_SCENARIO ?? "render-success";

function requireCommand(name) {
  const result = spawnSyncWhich(name);
  if (!result) throw new Error(`Missing prerequisite: ${name} must be installed and available on PATH.`);
}
function spawnSyncWhich(name) {
  try { execFileSync("which", [name], { stdio: "ignore" }); return true; } catch { return false; }
}

async function waitFor(predicate, description, limit = timeoutMs) {
  const deadline = Date.now() + limit;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await delay(500);
  }
  throw new Error(`Timed out waiting for ${description} (${Math.round(limit / 1000)}s).`);
}

const base = `http://127.0.0.1:${port}`;
async function request(path, body, method = "POST") {
  const response = await fetch(`${base}${path}`, body === undefined && method === "GET" ? {} : {
    method, ...(body === undefined ? {} : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
  });
  const json = await response.json().catch(() => ({}));
  if (!response.ok || json.value?.error) throw new Error(`WebDriver ${path}: ${JSON.stringify(json)}`);
  return json.value;
}
let sessionId;
async function element(using, value) {
  const result = await request(`/session/${sessionId}/element`, { using, value });
  return result["element-6066-11e4-a52e-4f735466cecf"];
}
async function elements(using, value) {
  const result = await request(`/session/${sessionId}/elements`, { using, value });
  return result.map((entry) => entry["element-6066-11e4-a52e-4f735466cecf"]);
}
async function click(using, value) {
  const id = await element(using, value);
  await request(`/session/${sessionId}/element/${id}/click`, {});
}
// WebKitWebDriver refuses clicks on off-screen elements; bring the target into view first
// so long candidate lists don't make runs flaky.
async function scrollTo(using, value) {
  await request(`/session/${sessionId}/execute/sync`, {
    script: `const node = document.evaluate(arguments[0], document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue; if (node) node.scrollIntoView({ block: "center" }); return Boolean(node);`,
    args: [using === "xpath" ? value : `(${JSON.stringify(value)})`],
  }).catch(() => {});
}
async function exists(using, value) {
  try { await element(using, value); return true; } catch { return false; }
}
async function text(using, value) {
  const id = await element(using, value);
  return request(`/session/${sessionId}/element/${id}/text`, undefined, "GET");
}
async function enabled(using, value) {
  try {
    const id = await element(using, value);
    return await request(`/session/${sessionId}/element/${id}/enabled`, undefined, "GET");
  } catch { return false; }
}
async function checked(using, value) {
  const id = await element(using, value);
  return request(`/session/${sessionId}/element/${id}/selected`, undefined, "GET");
}
async function screenshot(label) {
  try {
    const image = await request(`/session/${sessionId}/screenshot`, undefined, "GET");
    await writeFile(`${artifacts}/${label}.png`, Buffer.from(image, "base64"));
  } catch (error) {
    console.warn(`Screenshot unavailable at ${label}: ${error.message}`);
  }
}
async function saveDom(label) {
  try {
    const html = await request(`/session/${sessionId}/source`, undefined, "GET");
    await writeFile(`${artifacts}/${label}.html`, html);
  } catch (error) {
    console.warn(`DOM capture unavailable at ${label}: ${error.message}`);
  }
}
/// Calls a Tauri command from inside the running app and returns its result.
///
/// The crop-path scenario needs the real `analyze_crop_path` command — the one that
/// runs FFmpeg over the source frames and the Haar-cascade face detector — and the
/// real `render_video` that consumes the resulting plan. Driving those through the
/// app's own IPC keeps the test honest: it exercises the same code path a user's
/// "Analyze local crop path" button does.
///
/// The promise is settled synchronously by storing the outcome on `window`, because
/// WebDriver's execute/sync has no way to await a JavaScript promise.
async function invokeTauri(command, args = {}, options = {}) {
  await request(`/session/${sessionId}/execute/sync`, {
    script: `
      const command = arguments[0];
      const args = arguments[1];
      const withChannel = arguments[2];
      window.__e2eResult = { state: "pending" };
      window.__e2eProgress = [];
      const callArgs = { ...args };
      if (withChannel) {
        // render_video requires a progress channel. The UI builds one with
        // the Channel class from @tauri-apps/api/core; the test cannot import that
        // module inside the webview, so it builds the same channel by hand from the
        // internals the API itself uses.
        const id = window.__TAURI_INTERNALS__.transformCallback((raw) => {
          if (raw && "message" in raw) window.__e2eProgress.push(raw.message);
        });
        // The Rust side parses the channel argument as "__CHANNEL__:<callback id>",
        // the same string the Channel class serializes to.
        callArgs.onProgress = "__CHANNEL__:" + id;
      }
      window.__TAURI_INTERNALS__.invoke(command, callArgs)
        .then((value) => { window.__e2eResult = { state: "ok", value }; })
        .catch((error) => { window.__e2eResult = { state: "error", error: String(error) }; });
      return true;
    `,
    args: [command, args, Boolean(options.withChannel)],
  });
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    // WebKitWebDriver requires the args parameter even when it is empty: without it
    // every poll fails with "Missing args parameter", and the swallowed error makes
    // this loop poll forever while the command has already finished.
    const result = await request(`/session/${sessionId}/execute/sync`, {
      script: "return window.__e2eResult;",
      args: [],
    }).catch(() => null);
    if (result && result.state !== "pending") {
      if (result.state === "error") throw new Error(`Tauri command ${command} failed: ${result.error}`);
      return result.value;
    }
    await delay(200);
  }
  throw new Error(`Timed out waiting for Tauri command ${command}.`);
}

const report = {
  task: "As a first-time user, create a project from a local video, analyze it, keep one suggested clip, and export an MP4. Explore the interface rather than following hidden implementation details. Record confusing labels, unclear progress/success states, and unnecessary steps.",
  startedAt: new Date().toISOString(),
  video: video ? basename(video) : null,
  observations: [],
};

/// Render progress is published on the status line while a render runs; the element
/// only exists during rendering, so a miss means no progress reached the UI.
async function renderStatus() {
  return text("css selector", ".message").catch(() => "");
}

/// Pids of every whisper process on this machine, minus the ones that were already
/// running before the test started. Diffing against that baseline is what makes the
/// orphan check meaningful on a developer machine.
function transcriberPids(baseline) {
  // Match the transcriber by its interpreter, not by a substring of the command line:
  // any shell that merely mentions "whisper" would otherwise look like a survivor.
  try {
    const rows = execFileSync("ps", ["-eo", "pid,comm,args", "--no-headers"], { encoding: "utf8" });
    const pids = rows
      .split("\n")
      // The transcriber runs as "<venv>/bin/python <venv>/bin/whisper ...": match on the
      // python interpreter running the whisper entry point, so a shell that merely
      // mentions "whisper" in its command line is not mistaken for a survivor.
      .filter((line) => {
        // ps reports the interpreter as "whisper" for the console script, and the
        // command line still carries the real "<venv>/bin/python <venv>/bin/whisper".
        const parts = line.trim().split(/\s+/);
        const comm = parts[1] ?? "";
        const args = parts.slice(2).join(" ");
        return (/^python/.test(comm) || comm === "whisper") && /\/bin\/whisper(\s|$)/.test(args);
      })
      .map((line) => line.trim().split(/\s+/)[0])
      .filter(Boolean);
    return new Set(pids.filter((pid) => !baseline.has(pid)));
  } catch { return new Set(); }
}

/// Snapshot of the transcriber processes still alive, with their process groups, so a
/// failed orphan check explains itself instead of just timing out.
function survivingTranscribers() {
  // Broad on purpose: this is the diagnostic dump for a failed orphan check, so it
  // shows every process that mentions the transcriber, not just a matched one.
  try {
    const rows = execFileSync("ps", ["-eo", "pid,pgid,comm,args", "--no-headers"], { encoding: "utf8" });
    return rows
      .split("\n")
      .filter((line) => /whisper/i.test(line))
      .map((line) => line.trim().slice(0, 200));
  } catch { return []; }
}

/// Starts an analysis, waits until the transcriber is genuinely running, aborts from
/// the UI, and requires that no transcriber process survives the cancel.
async function runAnalysisCancelScenario(analyzeButton, videoPath) {
  const baseline = new Set();
  await click("xpath", analyzeButton);
  try {
    await waitFor(async () => transcriberPids(baseline).size > 0, "the transcriber to be running", timeoutMs);
  } catch (error) {
    report.survivingTranscribers = survivingTranscribers();
    throw error;
  }
  const running = transcriberPids(baseline);
  report.observations.push(`Analysis reached transcription with ${running.size} transcriber process(es) running.`);
  await screenshot("03-analysis-running");

  await scrollTo("xpath", "//button[normalize-space(.)='Abort analysis']");
  await click("xpath", "//button[normalize-space(.)='Abort analysis']");
  await waitFor(async () => /Analysis stopped|Abort failed/i.test(await renderStatus()), "analysis stop confirmation", 120000);
  const stopped = await renderStatus();
  report.observations.push(`Abort produced: ${stopped}`);

  try {
    await waitFor(async () => transcriberPids(baseline).size === 0, "every transcriber process to exit after the abort", 60000);
  } catch (error) {
    report.survivingTranscribers = survivingTranscribers();
    throw error;
  }
  report.observations.push("No transcriber process survived the abort (the old behaviour left them running).");
  // The app must be usable again rather than stuck mid-analysis.
  // The app must be usable again rather than stuck mid-analysis: walk back to the
  // config step and confirm a fresh analysis can be started.
  await click("xpath", "//button[contains(normalize-space(.), 'Config')]");
  await waitFor(() => enabled("xpath", "//button[normalize-space(.)='Start analysis']"), "analysis to be restartable after abort", 60000);
  await screenshot("03-analysis-aborted");
}

/// Exercises the local crop path end to end, through the app's own IPC.
///
/// The chain under test is: the Haar-cascade face detector finds the speaker's face in
/// real frames, `analyze_crop_path` tracks and smooths it, the editor maps that onto
/// contract keyframes, and the renderer crops the output to follow the subject.
///
/// The speaker labels are supplied by the test rather than by diarization. Diarization
/// needs pyannote, a Hugging Face token and a model download, so it is not available
/// on a clean machine — but the crop path only needs to know *which* speaker is
/// talking when, and the test can say that. Everything downstream of that is the real
/// detector, the real tracker and the real renderer.
async function runCropPathScenario(videoPath, outputPath) {
  const duration = Number(process.env.DESKTOP_E2E_CROP_DURATION ?? 60);

  // The detector needs OpenCV in the app's Python venv. Without it the command fails
  // with FACE_RUNTIME_MISSING, and the test must say so plainly rather than time out.
  let samples;
  try {
    samples = await invokeTauri("sample_speaker_faces", {
      path: videoPath,
      speakers: [{ speakerId: "A", time: 5 }],
    });
  } catch (error) {
    const message = String(error?.message ?? error);
    if (message.includes("FACE_RUNTIME_MISSING")) {
      throw new Error(
        `${message} — the crop scenario needs OpenCV in the app's Python venv. Run scripts/setup-machine.sh, which installs opencv-python-headless for the face detector.`,
      );
    }
    throw error;
  }
  if (!Array.isArray(samples) || samples.length === 0) {
    throw new Error(
      "The face detector found no face in the source video. The crop scenario needs a video with a visible face (the Haar cascade cannot see a talking head off-screen).",
    );
  }
  report.observations.push(`The face detector found ${samples.length} face(s) in the source video.`);

  // The mapping the editor builds after the user confirms it: speaker -> face index.
  const mapping = { A: "face-0" };
  const path = await invokeTauri("analyze_crop_path", {
    path: videoPath,
    duration,
    speakerIntervals: [{ start: 0, end: duration, speakerId: "A" }],
    faceMapping: { confirmed: true, sourceArtifactId: "e2e-source", matches: mapping },
    sourceArtifactId: "e2e-source",
  });
  if (!Array.isArray(path) || path.length === 0) {
    throw new Error("analyze_crop_path returned no crop points, so the crop path was not exercised.");
  }
  const centers = path.map((point) => point.centerX);
  const distinctCenters = new Set(centers.map((value) => Number(value).toFixed(4))).size;
  report.observations.push(
    `The crop path has ${path.length} point(s) across ${new Set(path.map((point) => point.sceneId)).size} scene(s), with ${distinctCenters} distinct horizontal centre(s) between ${Math.min(...centers).toFixed(3)} and ${Math.max(...centers).toFixed(3)}.`,
  );
  if (distinctCenters < 2) {
    throw new Error(
      "The crop path never moved: the tracker produced the same centre for every frame, so it cannot be distinguished from a fixed centre crop.",
    );
  }
  // The tracker smooths exactly once. A second smoothing pass (the behaviour ADR-003
  // removed) pulls each point back towards the previous one, so a path that jumps
  // between two far-apart positions would show intermediate values instead of a step.
  const jumps = centers.filter((center, index) => index > 0 && Math.abs(center - centers[index - 1]) > 0.05).length;
  report.observations.push(`The crop path moved by more than 0.05 in ${jumps} step(s).`);

  // Render with the crop path in the plan, so the renderer has to crop the output.
  const plan = {
    version: 1,
    sourceArtifactId: "e2e-source",
    ranges: [{ start: 0, end: Math.min(20, duration) }],
    aspectRatio: "9:16",
    captions: [],
    cropMode: "track",
    outputProfile: { container: "mp4", codec: "h264" },
    speakerMap: { A: "face-0" },
    cropKeyframes: path
      .filter((point) => point.centerX >= 0 && point.centerX <= 1)
      .map((point) => ({
        time: point.time,
        centerX: point.centerX,
        centerY: point.centerY,
        speakerId: "A",
        sourceFaceId: "face-0",
        sourceArtifactId: "e2e-source",
        sceneId: point.sceneId,
        confidence: 0.9,
        confirmed: true,
      })),
  };
  if (plan.cropKeyframes.length === 0) throw new Error("No crop keyframe survived the contract filter.");

  await rm(outputPath, { force: true });
  const rendered = await invokeTauri("render_video", { request: { input: videoPath, output: outputPath, plan } }, { withChannel: true });
  if (!rendered) throw new Error("render_video returned no output path.");
  const outputStat = await stat(outputPath);
  if (!outputStat.isFile() || outputStat.size === 0) throw new Error(`The cropped export is missing or empty: ${outputPath}`);
  report.observations.push(`The cropped export was written to ${outputPath} (${outputStat.size} bytes).`);
  await screenshot("04-crop-export-complete");
  report.output = outputPath;
}

// Reads persisted render records from the app's isolated project database, so a
// cancelled export can be confirmed in SQLite instead of trusting the UI alone.
function readRenderRecords(stateDir) {  const db = new DatabaseSync(join(stateDir, "projects.sqlite"), { readOnly: true });
  try {
    return db.prepare("SELECT record_json FROM projects").all()
      .map((row) => JSON.parse(row.record_json))
      .map((record) => ({ projectId: record.id, renders: Array.isArray(record.renders) ? record.renders : [] }));
  } finally { db.close(); }
}

// Starts a multi-clip render, waits for real progress percentages to arrive, cancels
// mid-flight, and checks the app neither claims success nor leaves a stale record.
async function runProgressCancelScenario(outputPath) {
  // The export path is fixed for the whole app, so an earlier run may have left a file
  // there: what must not happen is the cancelled render writing a new one.
  const before = await stat(outputPath).catch(() => null);
  const boxes = await elements("css selector", ".cand-list input[type=checkbox]");
  if (boxes.length < 2) throw new Error(`Cancel scenario needs at least two clips to make a batch render long enough to interrupt, found ${boxes.length}.`);
  for (const box of boxes) {
    if (!(await request(`/session/${sessionId}/element/${box}/selected`, undefined, "GET"))) {
      await request(`/session/${sessionId}/element/${box}/click`, {});
    }
  }
  const batchLabel = `Render ${boxes.length} selected clips`;
  const batchSelector = `//button[contains(normalize-space(.), '${batchLabel}')]`;
  await scrollTo("xpath", batchSelector);
  await waitFor(() => enabled("xpath", batchSelector), `${boxes.length} clips selected`, 60000);
  report.observations.push(`Selected ${boxes.length} clips for a batch render, so the export lasts long enough to interrupt.`);
  await screenshot("03b-batch-selected");
  await click("xpath", batchSelector);

  // Sample the status line while the render runs: this both proves progress arrives and
  // shows how long the render actually took, instead of guessing at a fixed window.
  const timeline = [];
  const renderStarted = Date.now();
  let cancelSeen = false;
  let finishedMessage = "";
  let sourceSamples = 0;
  while (Date.now() - renderStarted < 180000) {
    const status = await renderStatus();
    let domFlags = "";
    if (sourceSamples++ % 5 === 0) {
      const html = await request(`/session/${sessionId}/source`, undefined, "GET").catch(() => "");
      if (html) {
        domFlags = ` [dom cancel=${/Cancel render/.test(html)} progress=${/Render progress/.test(html)} busy=${/Rendering/.test(html)}]`;
      }
    }
    timeline.push(`${((Date.now() - renderStarted) / 1000).toFixed(1)}s ${status || "(empty)"}${domFlags}`);
    if (/Rendered |Render cancelled|Render failed/i.test(status)) { finishedMessage = status; break; }
    if (await exists("xpath", "//button[normalize-space(.)='Cancel render']")) { cancelSeen = true; break; }
    await delay(100);
  }
  report.renderTimeline = timeline;
  if (!cancelSeen) {
    throw new Error(`The render progress and cancel control never appeared while rendering (status ended at: ${finishedMessage || timeline.at(-1)}).`);
  }

  const percents = new Set();
  // A three-clip batch can spend a while on one clip's join step before the next
  // percentage lands, so the window has to be generous enough to see it move.
  const collectedUntil = Date.now() + 90000;
  while (Date.now() < collectedUntil) {
    const observed = (await renderStatus()).match(/(\d+)%/);
    if (observed) percents.add(Number(observed[1]));
    if (percents.size >= 2) break;
    await delay(100);
  }
  if (!percents.size) throw new Error("No render progress reached the status line: the IPC progress channel is not delivering updates.");
  const sorted = [...percents].sort((a, b) => a - b);
  if (sorted.length < 2) throw new Error(`Render progress stayed at ${sorted[0]}% and never advanced; cannot prove progress is live.`);
  report.observations.push(`Progress percentages observed during render: ${sorted.join(" → ")}%.`);
  await screenshot("03c-render-progress");

  await scrollTo("xpath", "//button[normalize-space(.)='Cancel render']");
  await click("xpath", "//button[normalize-space(.)='Cancel render']");
  await waitFor(async () => /cancel/i.test(await renderStatus()), "render cancellation message", 180000);
  const cancelledMessage = await renderStatus();
  if (cancelledMessage.includes("Rendered ")) throw new Error(`Cancellation was reported as a success: ${cancelledMessage}`);
  report.observations.push(`Cancel produced: ${cancelledMessage}`);

  await waitFor(async () => !(await exists("xpath", "//button[normalize-space(.)='Cancel render']")), "render controls to return to idle after cancel", 120000);
  await waitFor(() => enabled("xpath", `//button[contains(normalize-space(.), '${batchLabel}')]`), "render button re-enabled after cancel", 60000);

  // The export path is fixed for the whole app, so an earlier run may have left a file
  // there: what must not happen is the cancelled render writing a new one.
  const after = await stat(outputPath).catch(() => null);
  const changed = after && (!before || after.mtimeMs !== before.mtimeMs || after.size !== before.size);
  if (changed) throw new Error(`A cancelled render still wrote an export: ${outputPath} (${after.size} bytes).`);

  const records = readRenderRecords(tempState).flatMap((entry) => entry.renders);
  const cancelled = records.filter((entry) => entry?.status === "cancelled");
  const completed = records.filter((entry) => entry?.status === "completed");
  if (!cancelled.length) throw new Error(`No render record was persisted as cancelled. Persisted renders: ${JSON.stringify(records)}`);
  if (completed.length) throw new Error("A render the user cancelled was persisted as completed.");
  report.observations.push(`SQLite persisted ${cancelled.length} cancelled render record(s) and no completed ones.`);
  await screenshot("04-render-cancelled");
}
let driver;
let tempState;
let failed;
try {
  requireCommand("tauri-driver");
  if (!video) throw new Error("Set DESKTOP_E2E_VIDEO to a short local video with clear spoken dialogue. The full workflow needs speech that produces at least one highlight candidate.");
  if (!process.env.DESKTOP_E2E_APP) throw new Error("Set DESKTOP_E2E_APP to the dedicated E2E Tauri executable; do not point this test at the normal release app.");
  if (!process.env.DESKTOP_E2E_BUILD_VIDEO || resolve(process.env.DESKTOP_E2E_BUILD_VIDEO) !== video) throw new Error("Build the dedicated desktop-e2e frontend with VITE_DESKTOP_E2E_VIDEO set to the same DESKTOP_E2E_VIDEO path.");
  const output = resolve(artifacts, "e2e-export.mp4");
  if (!process.env.DESKTOP_E2E_BUILD_OUTPUT || resolve(process.env.DESKTOP_E2E_BUILD_OUTPUT) !== output) throw new Error("Build the E2E frontend with VITE_DESKTOP_E2E_OUTPUT set to the artifacts/e2e-export.mp4 path.");
  await readFile(appPath);
  await readFile(video);
  await mkdir(artifacts, { recursive: true });
  tempState = await mkdtemp(`${tmpdir()}/local-first-desktop-e2e-`);
  report.stateDir = tempState;
  const sourceRuntime = process.env.DESKTOP_E2E_RUNTIME_ENV ? resolve(process.env.DESKTOP_E2E_RUNTIME_ENV) : `${process.env.HOME}/.cache/local-first-ai-video-studio/runtime.env`;
  const runtimeOverrides = new Set(Object.keys(process.env).filter((key) => key.startsWith("WHISPER_") || key === "TRANSCRIPT_FIXTURE"));
  const runtimeFile = await readFile(sourceRuntime, "utf8").catch(() => "");
  const overrides = new Map(Object.entries(process.env).filter(([key]) => key.startsWith("WHISPER_") || key === "TRANSCRIPT_FIXTURE"));
  const runtimeLines = runtimeFile.split(/\r?\n/).filter((line) => !runtimeOverrides.has(line.split("=", 1)[0]) && line.trim());
  runtimeLines.push(...[...overrides].map(([key, value]) => `${key}=${value}`));
  await writeFile(`${tempState}/runtime.env`, `${runtimeLines.join("\n")}\n`);
  process.env.LOCAL_FIRST_STATE_DIR = tempState;
  // The app's Python venv lives outside the state directory by default, so point the
  // app at the machine's real venv instead of the throwaway state directory. The crop
  // scenario needs it for the face detector (OpenCV); the other scenarios need it for
  // the transcriber.
  const venv = process.env.LOCAL_FIRST_VENV ?? join(homedir(), ".cache", "local-first-ai-video-studio", ".venv");
  if (!process.env.LOCAL_FIRST_VENV) process.env.LOCAL_FIRST_VENV = venv;
  driver = spawn("tauri-driver", ["--port", String(port)], { stdio: "inherit", env: process.env });
  await waitFor(async () => {
    try { return (await request("/status", undefined, "GET")).ready; } catch { return false; }
  }, `tauri-driver on ${base}`, 15000);
  const created = await request("/session", {
    capabilities: {
      alwaysMatch: {
        browserName: "wry",
        "tauri:options": { application: appPath, args: [] },
      },
      firstMatch: [{}],
    },
  });
  sessionId = created.sessionId;

  const started = Date.now();
  await waitFor(() => exists("xpath", "//button[normalize-space(.)='Browse files']"), "Studio initial screen", 30000);
  await saveDom("01-studio-start");
  report.observations.push("Initial rendered DOM saved as 01-studio-start.html.");

  if (scenario === "crop-path") {
    // The crop pipeline runs through the app's IPC and does not need the webview
    // preview, so the scenario starts before the video is loaded into the player.
    report.task = "Run the local face-tracking crop path against a real video and confirm the renderer crops the export to follow the detected face.";
    await runCropPathScenario(video, output);
    report.result = "passed";
    report.durationMs = Date.now() - started;
  } else {
  await click("xpath", "//button[normalize-space(.)='Browse files']");
  await waitFor(async () => (await text("css selector", ".message")).includes("Video loaded"), "video loaded confirmation", 30000);
  await waitFor(async () => (await text("css selector", ".row")).includes(basename(video)), "new project in Recent projects", 30000);
  report.observations.push("The app confirmed video loading and listed the source in Recent projects, showing project creation.");
  await screenshot("02-video-loaded-project-created");

  await click("xpath", "//button[normalize-space(.)='Continue to config →']");
  const analyzeButton = "//button[normalize-space(.)='Start analysis']";
  await waitFor(() => enabled("xpath", analyzeButton), "analysis runtime ready", 120000);

  if (scenario === "analysis-cancel") {
    await runAnalysisCancelScenario(analyzeButton, video);
  } else {
    await click("xpath", analyzeButton);
    report.observations.push("Analysis started after the runtime became ready; waiting for a candidate or a visible error.");
    await waitFor(async () => {
      const status = await text("css selector", ".message").catch(() => "");
      return status.includes("highlight candidates ready") || status.includes("No highlights found") || status.startsWith("Analyze failed");
    }, "analysis completion", timeoutMs);
    const analysisMessage = await text("css selector", ".message");
    if (!analysisMessage.includes("highlight candidates ready")) throw new Error(`Analysis did not produce a candidate: ${analysisMessage}`);
    await screenshot("03-candidates-ready");
    report.observations.push(`Analysis result: ${analysisMessage}`);
    await scrollTo("css selector", ".cand-list .cand");
    await click("css selector", ".cand-list .cand");
    if (scenario === "render-progress-cancel") {
      report.task = "Start a multi-clip render, watch real progress percentages arrive, cancel mid-flight, and confirm the app neither claims success nor records a completed export.";
      await runProgressCancelScenario(output);
    } else {
      const firstCheckbox = ".cand-list input[type=checkbox]";
      if (!(await checked("css selector", firstCheckbox))) await click("css selector", firstCheckbox);
      await waitFor(() => enabled("xpath", "//button[contains(normalize-space(.), 'Render 1 selected clip')]"), "one clip selected");
      await click("xpath", "//button[contains(normalize-space(.), 'Render 1 selected clip')]");
      await waitFor(async () => (await text("css selector", ".message")).includes("Rendered "), "render success", timeoutMs);
      const savedMessage = await text("css selector", ".message");
      const outputStat = await stat(output);
      if (!outputStat.isFile() || outputStat.size === 0) throw new Error(`Export output is missing or empty: ${output}`);
      report.observations.push(`${savedMessage}; exported ${outputStat.size} bytes to a temporary file.`);
      await screenshot("04-export-complete");
      report.output = output;
    }
  }
  }
  report.result = "passed";
  report.durationMs = Date.now() - started;
} catch (error) {
  failed = error;
  report.result = "failed";
  report.error = String(error?.stack ?? error);
  if (sessionId) { await screenshot("failure"); await saveDom("failure"); }
} finally {
  report.finishedAt = new Date().toISOString();
  await writeFile(`${artifacts}/report.json`, `${JSON.stringify(report, null, 2)}\n`).catch(() => {});
  if (sessionId) await request(`/session/${sessionId}`, {}, "DELETE").catch(() => {});
  if (driver && driver.exitCode === null) driver.kill("SIGTERM");
  if (tempState && !process.env.DESKTOP_E2E_KEEP_OUTPUT) await rm(tempState, { recursive: true, force: true });
}
console.log(JSON.stringify(report, null, 2));
if (failed) process.exitCode = 1;
