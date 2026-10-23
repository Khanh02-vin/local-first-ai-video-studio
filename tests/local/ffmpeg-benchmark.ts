import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir, cpus, arch, platform } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { render } from "../../adapters/local/engine.ts";

const exec = promisify(execFile);
const ffmpegPath = process.env.FFMPEG_PATH ?? "ffmpeg";
const ffprobePath = process.env.FFPROBE_PATH ?? "ffprobe";
const root = await mkdtemp(join(tmpdir(), "lf-ffmpeg-bench-"));
const input = join(root, "fixture.mp4");
const output = join(root, "render.mp4");

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

try {
  const [ffmpegVersion, ffprobeVersion] = await Promise.all([
    exec(ffmpegPath, ["-version"]),
    exec(ffprobePath, ["-version"]),
  ]);
  await exec(ffmpegPath, [
    "-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=12",
    "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=12",
    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", input,
  ]);
  const inputBytes = await stat(input);
  const inputHash = createHash("sha256").update(await readFile(input)).digest("hex");

  // Warm up the same render path, but exclude it from measured samples.
  const request = {
    input,
    output,
    plan: { version: 1, sourceArtifactId: "bench", ranges: [{ start: 1, end: 9 }], aspectRatio: "16:9" as const },
  };
  await render(request, { ffmpegPath, ffprobePath });
  const samples: Array<{ elapsedMs: number; outputBytes: number; renderSpeed: number }> = [];
  for (let trial = 0; trial < 5; trial++) {
    const started = process.hrtime.bigint();
    await render(request, { ffmpegPath, ffprobePath });
    const elapsedMs = Number(process.hrtime.bigint() - started) / 1e6;
    const outputBytes = (await stat(output)).size;
    const probe = await exec(ffprobePath, ["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height", "-of", "json", output]);
    const metadata = JSON.parse(probe.stdout) as { format?: { duration?: string }; streams?: Array<{ codec_type?: string; width?: number; height?: number }> };
    const video = metadata.streams?.find((stream) => stream.codec_type === "video");
    const audio = metadata.streams?.find((stream) => stream.codec_type === "audio");
    assert.equal(video?.width, 1920);
    assert.equal(video?.height, 1080);
    assert.ok(audio, "render must preserve audio");
    assert.ok(Math.abs(Number(metadata.format?.duration) - 8) < 0.5, "render duration must match requested range");
    await exec(ffmpegPath, ["-v", "error", "-i", output, "-f", "null", "-"]);
    samples.push({ elapsedMs, outputBytes, renderSpeed: 8000 / elapsedMs });
  }

  const quality = await exec(ffmpegPath, [
    "-i", input, "-i", output,
    "-filter_complex", "[0:v]trim=start=1:duration=8,setpts=PTS-STARTPTS,crop=iw:iw*9/16:0:(ih-iw*9/16)/2,scale=1920:1080[ref];[1:v]setpts=PTS-STARTPTS[out];[ref][out]ssim",
    "-f", "null", "-",
  ]);
  const ssim = Number(quality.stderr.match(/All:([0-9.]+)/)?.[1]);
  assert.ok(Number.isFinite(ssim), "FFmpeg must report SSIM");
  assert.ok(ssim >= 0.9, `SSIM quality gate failed: ${ssim}`);

  const elapsed = samples.map((sample) => sample.elapsedMs);
  const sizes = samples.map((sample) => sample.outputBytes);
  const speeds = samples.map((sample) => sample.renderSpeed);
  process.stdout.write(`${JSON.stringify({
    benchmark: "ffmpeg-render",
    platform: `${platform()}-${arch()}`,
    cpu: cpus()[0]?.model ?? "unknown",
    ffmpeg: ffmpegVersion.stdout.split("\n")[0],
    ffprobe: ffprobeVersion.stdout.split("\n")[0],
    input: { sha256: inputHash, bytes: inputBytes.size, durationSeconds: 12, width: 640, height: 360, fps: 24 },
    settings: { preset: "fast", crf: 22, aspectRatio: "16:9", startSeconds: 1, durationSeconds: 8, warmupTrials: 1, measuredTrials: samples.length },
    trials: samples,
    elapsedMs: { median: median(elapsed), min: Math.min(...elapsed), max: Math.max(...elapsed) },
    renderSpeed: { median: median(speeds), min: Math.min(...speeds), max: Math.max(...speeds) },
    outputBytes: { median: median(sizes), min: Math.min(...sizes), max: Math.max(...sizes) },
    quality: { metric: "SSIM", value: ssim, minimum: 0.9, decodeCheck: "passed" },
  }, null, 2)}\n`);
} finally {
  await rm(root, { recursive: true, force: true });
}
