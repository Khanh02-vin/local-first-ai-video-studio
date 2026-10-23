import assert from "node:assert/strict";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { execFile as execFileCb } from "node:child_process";
import { promisify } from "node:util";
import { assertAudioTrack } from "../../adapters/local/probe-audio.ts";

const execFile = promisify(execFileCb);
const root = await mkdtemp(join(tmpdir(), "audio-probe-test-"));
const originalProbePath = process.env.FFPROBE_PATH;
try {
  const audio = join(root, "audio.wav");
  const silentVideo = join(root, "silent.mp4");
  await execFile(process.env.FFMPEG_PATH ?? "ffmpeg", ["-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", audio]);
  await execFile(process.env.FFMPEG_PATH ?? "ffmpeg", ["-y", "-f", "lavfi", "-i", "color=c=black:s=64x64:d=1", "-an", silentVideo]);
  process.env.FFPROBE_PATH = originalProbePath ?? "ffprobe";
  assert.doesNotThrow(() => assertAudioTrack(audio));
  assert.throws(() => assertAudioTrack(silentVideo), /NO_AUDIO_TRACK/);
  process.env.FFPROBE_PATH = join(root, "missing-ffprobe");
  assert.throws(() => assertAudioTrack(audio), /AUDIO_PROBE_SPAWN/);
  const failedProbe = join(root, "failed-ffprobe");
  await writeFile(failedProbe, "#!/bin/sh\necho 'probe rejected input' >&2\nexit 1\n");
  await chmod(failedProbe, 0o700);
  process.env.FFPROBE_PATH = failedProbe;
  assert.throws(() => assertAudioTrack(audio), /AUDIO_PROBE_FAILED: probe rejected input/);
  console.log("analysis audio probe tests: ok");
} finally {
  if (originalProbePath === undefined) delete process.env.FFPROBE_PATH;
  else process.env.FFPROBE_PATH = originalProbePath;
  await rm(root, { recursive: true, force: true });
}
