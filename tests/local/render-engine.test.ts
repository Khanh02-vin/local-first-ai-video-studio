/// The render engine is the single final renderer (ADR-002), so this test pins the
/// property that makes that claim true: the Node client and the desktop app go through
/// the same engine, and the engine's output is a real, playable MP4.
///
/// Before ADR-002 the Node side had its own FFmpeg argument builder (adapters/ffmpeg),
/// which silently drifted from the app's renderer — it could not render captions, hooks
/// or crop keyframes. These assertions are the guard against that coming back.
import assert from "node:assert/strict";
import { mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFile as execFileCb } from "node:child_process";
import { promisify } from "node:util";
import { probe, render, resolveEnginePath } from "../../adapters/local/engine.ts";

const execFile = promisify(execFileCb);
const root = await mkdtemp(join(tmpdir(), "lfai-engine-"));
const ffmpeg = process.env.FFMPEG_PATH ?? "ffmpeg";
const ffprobe = process.env.FFPROBE_PATH ?? "ffprobe";
const enginePath = resolveEnginePath();

try {
  const input = join(root, "input.mp4");
  await execFile(ffmpeg, [
    "-y", "-v", "error",
    "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=10:duration=6",
    "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=6",
    "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
    input,
  ]);

  // The engine reads the same ffprobe metadata the app's probe_video command returns.
  const metadata = await probe(input, { enginePath, ffprobePath: ffprobe });
  assert.ok(metadata.streams?.some((stream) => stream.codec_type === "video"), "probe must report a video stream");
  assert.ok(metadata.streams?.some((stream) => stream.codec_type === "audio"), "probe must report an audio stream");
  assert.ok(Number(metadata.format?.duration) > 5, "probe must report the container duration");
  assert.equal(metadata.format?.format_name?.includes("mp4"), true, "probe must report the container name");

  // A render through the engine produces a real 9:16 MP4 with audio, not just a file.
  const output = join(root, "output.mp4");
  const progress: number[] = [];
  await render(
    {
      input,
      output,
      plan: {
        version: 1,
        sourceArtifactId: "engine-test",
        ranges: [{ start: 0.5, end: 3.5 }],
        aspectRatio: "9:16",
      },
    },
    { enginePath, ffmpegPath: ffmpeg, ffprobePath: ffprobe, onProgress: (ratio) => progress.push(ratio) },
  );
  assert.ok((await stat(output)).size > 0, "the engine must write a non-empty output");
  const rendered = JSON.parse(
    (await execFile(ffprobe, ["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height", "-of", "json", output])).stdout,
  ) as { format: { duration: string }; streams: Array<{ codec_type: string; width?: number; height?: number }> };
  assert.deepEqual(rendered.streams.map((stream) => stream.codec_type), ["video", "audio"]);
  assert.equal(rendered.streams[0].width, 1080);
  assert.equal(rendered.streams[0].height, 1920);
  assert.ok(Math.abs(Number(rendered.format.duration) - 3) < 0.3, `expected the requested range, got ${rendered.format.duration}`);
  assert.ok(progress.length > 0, "the engine must report progress while rendering");
  assert.equal(progress[progress.length - 1], 1, "the last progress report must be the completed 100%");

  // The engine refuses to overwrite its own input instead of corrupting the source.
  const sameOutput = join(root, "same.mp4");
  await writeFile(sameOutput, Buffer.alloc(4));
  await assert.rejects(
    render(
      {
        input,
        output: input,
        plan: { version: 1, sourceArtifactId: "engine-test", ranges: [{ start: 0, end: 1 }], aspectRatio: "9:16" },
      },
      { enginePath, ffmpegPath: ffmpeg, ffprobePath: ffprobe },
    ),
    /OUTPUT_MUST_DIFFER_FROM_INPUT/,
  );

  // An invalid plan is rejected before FFmpeg is ever started.
  await assert.rejects(
    render(
      {
        input,
        output: join(root, "never.mp4"),
        plan: { version: 0, sourceArtifactId: "", ranges: [], aspectRatio: "9:16" },
      },
      { enginePath, ffmpegPath: ffmpeg, ffprobePath: ffprobe },
    ),
    /INVALID_RENDER_PLAN/,
  );
  assert.equal(await stat(join(root, "never.mp4")).then(() => true, () => false), false, "a rejected plan must not leave an output behind");

  console.log("render engine tests: ok (one renderer, real MP4 output)");
} finally {
  await rm(root, { recursive: true, force: true });
}
