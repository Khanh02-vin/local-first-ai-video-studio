import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { LocalRunner, type Enqueue } from "../adapters/local/runner.ts";
import type { LocalJob } from "../adapters/local/store.ts";

const ASPECT_RATIOS = ["9:16", "1:1", "16:9"] as const;
type Command = "enqueue" | "list" | "get" | "retry" | "drain" | "status";

function stateDirectory(): string {
  const configured = process.env.LOCAL_FIRST_STATE_DIR;
  const directory = resolve(configured || join(homedir(), ".local", "state", "local-first-ai-video-studio"));
  if (!isAbsolute(directory) || directory === "/") throw new Error("INVALID_LOCAL_FIRST_STATE_DIR");
  return directory;
}

function parseRequest(raw: string | undefined): Enqueue {
  if (!raw) throw new Error("REQUEST_JSON_REQUIRED");
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("INVALID_REQUEST");
  const request = value as Record<string, unknown>;
  if (typeof request.input !== "string" || !request.input.trim()) throw new Error("INPUT_REQUIRED");
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(request.input)) throw new Error("LOCAL_INPUT_REQUIRED");
  if (typeof request.output !== "string" || !request.output.trim()) throw new Error("OUTPUT_REQUIRED");
  if (request.aspectRatio !== undefined && !ASPECT_RATIOS.includes(request.aspectRatio as typeof ASPECT_RATIOS[number])) throw new Error("UNSUPPORTED_ASPECT_RATIO");
  for (const field of ["start", "end"] as const) {
    if (request[field] !== undefined && (typeof request[field] !== "number" || !Number.isFinite(request[field]) || request[field] < 0)) throw new Error("INVALID_CLIP_RANGE");
  }
  if (typeof request.start === "number" && typeof request.end === "number" && request.end <= request.start) throw new Error("INVALID_CLIP_RANGE");
  if (request.maxAttempts !== undefined && (!Number.isInteger(request.maxAttempts) || (request.maxAttempts as number) < 1 || (request.maxAttempts as number) > 10)) throw new Error("INVALID_MAX_ATTEMPTS");
  for (const field of ["idempotencyKey", "analysisJobId"] as const) {
    if (request[field] !== undefined && (typeof request[field] !== "string" || !(request[field] as string).trim())) throw new Error(`INVALID_${field.replace(/[A-Z]/g, (c) => `_${c}`).toUpperCase()}`);
  }
  return { ...request, input: resolve(request.input), output: resolve(request.output) } as Enqueue;
}

export function retryLocalJob(runner: LocalRunner, id: string): LocalJob {
  const job = runner.store.get(id);
  if (!job) throw new Error("JOB_NOT_FOUND");
  const retried = runner.store.retry(id);
  if (!retried) throw new Error("JOB_NOT_RETRYABLE");
  return retried;
}

async function main(args: string[]): Promise<unknown> {
  const command = args[0] as Command | undefined;
  if (!command || !["enqueue", "list", "get", "retry", "drain", "status"].includes(command)) throw new Error("UNKNOWN_COMMAND");
  const stateDir = stateDirectory();
  const runner = new LocalRunner({
    statePath: join(stateDir, "local-render.sqlite"),
    analysisStatePath: join(stateDir, "analysis.sqlite"),
    ffmpegPath: process.env.FFMPEG_PATH ?? "ffmpeg",
    ffprobePath: process.env.FFPROBE_PATH ?? "ffprobe",
    maxDurationSeconds: 4 * 60 * 60,
  });
  try {
    switch (command) {
      case "enqueue": return runner.enqueue(parseRequest(args[1]));
      case "list": return runner.store.list();
      case "get": {
        if (!args[1]) throw new Error("JOB_ID_REQUIRED");
        return runner.store.get(args[1]) ?? null;
      }
      case "retry": {
        if (!args[1]) throw new Error("JOB_ID_REQUIRED");
        return retryLocalJob(runner, args[1]);
      }
      case "drain":
        runner.recover();
        return await runner.drain();
      case "status": {
        const jobs = runner.store.list();
        return {
          mode: "local-render-only",
          stateDirectory: stateDir,
          database: join(stateDir, "local-render.sqlite"),
          ffmpegPath: process.env.FFMPEG_PATH ?? "ffmpeg",
          ffprobePath: process.env.FFPROBE_PATH ?? "ffprobe",
          supportedAspectRatios: ASPECT_RATIOS,
          renderedCaptions: false,
          renderedHooks: false,
          jobs: Object.fromEntries(["queued", "running", "completed", "failed", "cancelled", "interrupted"].map((status) => [status, jobs.filter((job) => job.status === status).length])),
        };
      }
    }
  } finally {
    runner.close();
  }
}

try {
  const result = await main(process.argv.slice(2));
  process.stdout.write(`${JSON.stringify(result)}\n`);
} catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
}
