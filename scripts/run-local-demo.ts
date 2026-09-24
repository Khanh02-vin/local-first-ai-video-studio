/**
 * One-command local demo: video in -> transcript -> highlights -> 9:16 short out.
 *
 * Usage:
 *   node --experimental-strip-types scripts/run-local-demo.ts <input.mp4> [output.mp4] [rangeStart] [rangeEnd]
 *
 * Requires: WHISPER_COMMAND env (default "whisper") and an ffmpeg/ffprobe on PATH
 * (or set FFMPEG_PATH / FFPROBE_PATH).
 */
import { mkdir, stat } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";

const [input, outputArg, rangeStartArg, rangeEndArg] = process.argv.slice(2);
if (!input) {
  console.error("usage: node --experimental-strip-types scripts/run-local-demo.ts <input.mp4> [output.mp4] [rangeStart] [rangeEnd]");
  process.exit(2);
}
const inputPath = resolve(input);
const outputPath = outputArg ? resolve(outputArg) : join(dirname(inputPath), "short_9x16.mp4");
await mkdir(dirname(outputPath), { recursive: true });

const ffmpegPath = process.env.FFMPEG_PATH ?? "ffmpeg";
const ffprobePath = process.env.FFPROBE_PATH ?? "ffprobe";
const whisperCommand = process.env.WHISPER_COMMAND ?? "whisper";
const whisperModel = process.env.WHISPER_MODEL ?? "tiny";

// 1. probe duration
import { execFile } from "node:child_process";
import { promisify } from "node:util";
const exec = promisify(execFile);
const probeOut = (await exec(ffprobePath, ["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", inputPath])).stdout.trim();
const duration = Number(probeOut);
if (!Number.isFinite(duration) || duration <= 0) {
  console.error(`INVALID_DURATION from probe: ${probeOut}`);
  process.exit(2);
}
const rangeStart = Number(rangeStartArg ?? 0);
const rangeEnd = Math.min(duration, Number(rangeEndArg ?? duration));
if (rangeStart < 0 || rangeEnd <= rangeStart) {
  console.error("INVALID_RANGE");
  process.exit(2);
}
console.log(`\n[probe] ${inputPath}\n        duration=${duration.toFixed(2)}s, analyzing [${rangeStart.toFixed(1)}, ${rangeEnd.toFixed(1)}]\n`);

// 2. extract 16k mono wav for whisper
const { join: pjoin, basename } = await import("node:path");
const workDir = pjoin(process.env.LOCAL_FIRST_STATE_DIR ?? pjoin(process.env.HOME ?? ".", ".cache", "local-first-ai-video-studio"), "demo");
await mkdir(workDir, { recursive: true });
const wavPath = pjoin(workDir, `${basename(inputPath)}.wav`);
await exec(ffmpegPath, ["-y", "-nostdin", "-hide_banner", "-loglevel", "error", "-i", inputPath, "-ss", String(rangeStart), "-t", String(rangeEnd - rangeStart), "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", wavPath]);
console.log(`[audio] extracted ${wavPath}`);

// 3. real whisper
const { transcribeWithWhisper } = await import("../adapters/local/whisper.ts");
const t0 = Date.now();
const transcript = await transcribeWithWhisper(wavPath, basename(inputPath), rangeEnd - rangeStart, { model: whisperModel, command: whisperCommand });
console.log(`[whisper] ${transcript.words.length} words, ${transcript.segments.length} segments in ${((Date.now() - t0) / 1000).toFixed(1)}s (model=${whisperModel}, language=${transcript.language})`);
if (transcript.words.length === 0) {
  console.log("         no words detected (silent audio? tiny model on unclear audio?) — continuing with highlight selection on empty segments");
}

// 4. real heuristic highlights
const { chooseHeuristicHighlights } = await import("../services/ai-pipeline/heuristic.ts");
const { chooseContentHighlights } = await import("../services/ai-pipeline/content-highlights.ts");
const highlights = rangeEnd - rangeStart > 120 ? chooseContentHighlights(transcript) : chooseHeuristicHighlights(transcript);
console.log(`[highlight] picked ${highlights.length} candidate(s):`);
for (const h of highlights) {
  console.log(`          [${h.start.toFixed(1)}-${h.end.toFixed(1)}s] score=${h.score}  ${h.title.slice(0, 60)}`);
}

// 5. real render of the top highlight (or the full range if no candidate)
const top = highlights[0];
const { LocalRunner } = await import("../adapters/local/runner.ts");
const statePath = pjoin(workDir, "demo-jobs.sqlite");
const runner = new LocalRunner({ ffmpegPath, ffprobePath, maxDurationSeconds: 4 * 3600, statePath });
const enq = runner.enqueue({ input: inputPath, output: outputPath, start: top?.start ?? rangeStart, end: top?.end ?? rangeEnd, aspectRatio: "9:16" });
await runner.drain();
const job = runner.store.get(enq.id);
if (job.status !== "completed") {
  console.error(`[render] FAILED: ${job.error ?? job.status}`);
  runner.close();
  process.exit(1);
}
const outBytes = (await stat(outputPath)).size;
console.log(`\n[render] job=${job.status}, ${job.outputBytes ?? outBytes} bytes -> ${outputPath}`);
console.log(`\n=== DEMO DONE ===`);
runner.close();
