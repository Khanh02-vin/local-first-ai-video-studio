/// The Node side of the render engine: a client of the Rust renderer.
///
/// The Node job queue (adapters/local/runner.ts) keeps its own bookkeeping — enqueue,
/// dedupe, retry, attempt budget — but it does not build FFmpeg arguments. The
/// arguments live in the Rust render engine (apps/desktop/src-tauri/crates/render-engine),
/// which the desktop app and this client both call. That is the "one final renderer"
/// rule from ADR-002: there is exactly one place that decides how a clip is encoded.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export type EnginePaths = { enginePath: string; ffmpegPath?: string; ffprobePath?: string };

export class EngineError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
    this.name = "EngineError";
  }
}

/// Locates the engine binary. `LF_AI_RENDER_BIN` wins, then the cargo build output of
/// this checkout, then PATH.
export function resolveEnginePath(): string {
  const configured = process.env.LF_AI_RENDER_BIN;
  if (configured) return configured;
  const name = process.platform === "win32" ? "lfai-render.exe" : "lfai-render";
  const built = join(
    fileURLToPath(new URL("../../apps/desktop/src-tauri/target/debug", import.meta.url)),
    name,
  );
  if (existsSync(built)) return built;
  return name;
}

export type EngineRenderRequest = {
  input: string;
  output: string;
  plan: {
    version: number;
    sourceArtifactId: string;
    ranges: Array<{ start: number; end: number }>;
    aspectRatio: "9:16" | "1:1" | "16:9";
    [key: string]: unknown;
  };
};

export type EngineProbe = {
  format?: { duration?: string; format_name?: string };
  streams?: Array<{ codec_name?: string; codec_type?: string; width?: number; height?: number }>;
};

/// Runs one render through the engine and reports progress as a 0→1 ratio.
export async function render(
  request: EngineRenderRequest,
  options: { enginePath?: string; ffmpegPath?: string; ffprobePath?: string; signal?: AbortSignal; onProgress?: (ratio: number) => void },
): Promise<string> {
  const engine = options.enginePath ?? resolveEnginePath();
  const scratch = await mkdtemp(join(tmpdir(), "lfai-engine-"));
  const requestPath = join(scratch, "request.json");
  try {
    await writeFile(requestPath, JSON.stringify(request), "utf8");
    const args = ["render", requestPath];
    if (options.ffmpegPath) args.push("--ffmpeg", options.ffmpegPath);
    if (options.ffprobePath) args.push("--ffprobe", options.ffprobePath);
    await runEngine(engine, args, options.signal, (line) => {
      const progress = parseProgress(line);
      if (progress !== undefined) options.onProgress?.(Math.max(0, Math.min(1, progress / 100)));
    });
    return request.output;
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

/// Reads the container metadata of a media file through the engine.
export async function probe(
  input: string,
  options: { enginePath?: string; ffprobePath?: string } = {},
): Promise<EngineProbe> {
  const engine = options.enginePath ?? resolveEnginePath();
  const args = ["probe", input];
  if (options.ffprobePath) args.push("--ffprobe", options.ffprobePath);
  const stdout = await runEngine(engine, args, undefined, undefined);
  try {
    return JSON.parse(stdout) as EngineProbe;
  } catch (error) {
    throw new EngineError("PROBE_JSON", `the engine returned unreadable probe output: ${String(error)}`);
  }
}

function parseProgress(line: string): number | undefined {
  if (!line.startsWith("{")) return undefined;
  try {
    const value = JSON.parse(line) as { percent?: number };
    return typeof value.percent === "number" ? value.percent : undefined;
  } catch {
    return undefined;
  }
}

function runEngine(
  command: string,
  args: string[],
  signal: AbortSignal | undefined,
  onStderrLine: ((line: string) => void) | undefined,
): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "", stderr = "", buffer = "", settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener("abort", abort);
      error ? reject(error) : resolve(stdout);
    };
    const abort = () => {
      child.kill("SIGKILL");
      finish(new EngineError("RENDER_CANCELLED", "the render was cancelled"));
    };
    if (signal?.aborted) return abort();
    signal?.addEventListener("abort", abort, { once: true });
    child.stdout.on("data", (chunk: Buffer) => { stdout += chunk.toString(); });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
      buffer += chunk.toString();
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";
      for (const line of lines) onStderrLine?.(line.trim());
    });
    child.on("error", (cause) => finish(new EngineError("ENGINE_SPAWN_ERROR", cause.message)));
    child.on("close", (code) => {
      if (settled) return;
      if (code === 0) return finish();
      finish(new EngineError("RENDER_FAILED", stderr.trim().slice(-500) || `the engine exited with code ${code}`));
    });
  });
}
