import { spawn } from "node:child_process";
import { mkdir, stat } from "node:fs/promises";
import { dirname } from "node:path";
import { validateProbe, type MediaProbe } from "../../packages/media-engine/probe/index.ts";

export class FfmpegError extends Error {
  readonly code: string;
  constructor(code: string, message: string) { super(message); this.code = code; this.name = "FfmpegError"; }
}

export type FfmpegPaths = { ffmpegPath: string; ffprobePath: string };
export type ProbeOptions = FfmpegPaths & { maxDurationSeconds?: number; timeoutMs?: number };

export async function probe(input: string, options: ProbeOptions): Promise<MediaProbe & { codec: string; hasAudio: boolean; container: string }> {
  const output = await run(options.ffprobePath, ["-v", "error", "-show_entries", "format=duration,format_name:stream=codec_name,codec_type,width,height", "-of", "json", input], options.timeoutMs ?? 15_000, "PROBE");
  try {
    const data = JSON.parse(output);
    const streams = data.streams ?? [];
    const video = streams.find((s: { codec_type?: string }) => s.codec_type === "video");
    const audio = streams.find((s: { codec_type?: string }) => s.codec_type === "audio");
    if (!video) throw new FfmpegError("NO_VIDEO_STREAM", "No video stream");
    if (!audio) throw new FfmpegError("NO_AUDIO_STREAM", "No audio stream");
    const result = { duration: Number(data.format?.duration), width: Number(video.width), height: Number(video.height), codec: video.codec_name ?? "unknown", hasAudio: true, container: data.format?.format_name ?? "unknown" };
    validateProbe(result);
    if (options.maxDurationSeconds !== undefined && result.duration > options.maxDurationSeconds) throw new FfmpegError("DURATION_LIMIT", "Input duration exceeds limit");
    return result;
  } catch (error) {
    if (error instanceof FfmpegError) throw error;
    throw new FfmpegError("CORRUPTED_CONTAINER", "Invalid ffprobe output");
  }
}

export type RenderOptions = FfmpegPaths & { input: string; output: string; start: number; duration: number; aspectRatio?: "9:16" | "1:1" | "16:9"; timeoutMs?: number; signal?: AbortSignal; onProgress?: (ratio: number) => void };

export async function render(options: RenderOptions): Promise<void> {
  if (options.start < 0 || options.duration <= 0) throw new FfmpegError("INVALID_RANGE", "Invalid render range");
  await mkdir(dirname(options.output), { recursive: true });
  const ratio = options.aspectRatio ?? "9:16";
  const crop = ratio === "9:16" ? "crop=ih*9/16:ih:(iw-ih*9/16)/2:0" : ratio === "1:1" ? "crop=ih:ih:(iw-ih)/2:0" : "crop=iw:iw*9/16:0:(ih-iw*9/16)/2";
  const args = ["-y", "-nostdin", "-progress", "pipe:1", "-nostats", "-ss", options.start.toFixed(3), "-t", options.duration.toFixed(3), "-i", options.input, "-vf", `${crop},scale=1080:1920`, "-c:v", "libx264", "-preset", "fast", "-crf", "22", "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart", options.output];
  await run(options.ffmpegPath, args, options.timeoutMs ?? 10 * 60_000, "RENDER", options.signal, (line) => {
    const match = /^out_time_ms=(\d+)$/.exec(line);
    if (match) options.onProgress?.(Math.max(0, Math.min(1, Number(match[1]) / 1_000_000 / options.duration)));
  });
  const output = await stat(options.output);
  if (output.size <= 0) throw new FfmpegError("EMPTY_OUTPUT", "FFmpeg produced an empty output");
  options.onProgress?.(1);
}

function run(command: string, args: string[], timeoutMs: number, operation: string, signal?: AbortSignal, onLine?: (line: string) => void): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "", stderr = "", settled = false, buffer = "";
    const finish = (error?: Error) => { if (settled) return; settled = true; clearTimeout(timer); signal?.removeEventListener("abort", abort); error ? reject(error) : resolve(stdout); };
    const abort = () => { child.kill("SIGKILL"); finish(new FfmpegError(`${operation}_CANCELLED`, `${operation} cancelled`)); };
    const timer = setTimeout(() => { child.kill("SIGKILL"); finish(new FfmpegError(`${operation}_TIMEOUT`, `${operation} timed out`)); }, timeoutMs);
    if (signal?.aborted) return abort();
    signal?.addEventListener("abort", abort, { once: true });
    child.stdout.on("data", (chunk: Buffer) => { stdout += chunk.toString(); buffer += chunk.toString(); const lines = buffer.split("\n"); buffer = lines.pop() ?? ""; lines.forEach((line) => onLine?.(line.trim())); });
    child.stderr.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });
    child.on("error", (error) => finish(new FfmpegError(`${operation}_SPAWN_ERROR`, error.message)));
    child.on("close", (code) => code === 0 ? finish() : finish(new FfmpegError(`${operation}_FAILED`, stderr.slice(-500))));
  });
}
