import { randomUUID } from "node:crypto";
import { resolve } from "node:path";
import { probe, render, type FfmpegPaths } from "../ffmpeg/index.ts";
import { LocalStore, type LocalJob } from "./store.ts";

export type Enqueue = { input: string; output: string; aspectRatio?: "9:16" | "1:1" | "16:9"; start?: number; end?: number; maxAttempts?: number; idempotencyKey?: string; analysisJobId?: string };
export type RunnerOptions = FfmpegPaths & { maxDurationSeconds?: number; statePath?: string; analysisStatePath?: string };

export class LocalRunner {
  readonly store: LocalStore;
  private active?: AbortController;
  private readonly options: RunnerOptions;
  constructor(options: RunnerOptions) { this.options = options; this.store = LocalStore.open(options.statePath ?? ":memory:", options.analysisStatePath); }
  enqueue(request: Enqueue): LocalJob { if (resolve(request.input) === resolve(request.output)) throw new Error("OUTPUT_MUST_DIFFER_FROM_INPUT"); return this.store.create({ id: randomUUID(), input: request.input, output: request.output, start: request.start ?? null, end: request.end ?? null, aspectRatio: request.aspectRatio ?? "9:16", maxAttempts: request.maxAttempts, idempotencyKey: request.idempotencyKey, analysisJobId: request.analysisJobId }); }
  recover(): void { this.store.recover(); }
  cancel(id: string): boolean { if (this.store.get(id)?.status === "queued") { this.store.update(id, { status: "cancelled", error: "Cancelled by user" }); return true; } this.active?.abort(); return Boolean(this.store.get(id)); }
  async drain(): Promise<LocalJob[]> { const completed: LocalJob[] = []; let job: LocalJob | undefined; while ((job = this.store.claim())) { const controller = new AbortController(); this.active = controller; try { const media = await probe(job.input, this.options); const start = Math.max(0, job.start ?? 0); const end = Math.min(media.duration, job.end ?? media.duration); if (end <= start) throw new Error("EMPTY_CLIP_RANGE"); await render({ ...this.options, input: job.input, output: job.output, start, duration: end - start, aspectRatio: job.aspectRatio as "9:16" | "1:1" | "16:9", signal: controller.signal, onProgress: (progress) => this.store.update(job!.id, { progress }) }); const bytes = (await import("node:fs/promises")).stat(job.output).then((s) => s.size); this.store.update(job.id, { status: "completed", progress: 1, outputBytes: await bytes }); completed.push(this.store.get(job.id)!); } catch (error) { if (controller.signal.aborted) this.store.update(job.id, { status: "cancelled", error: "Cancelled by user" }); else if (job.attempts < job.maxAttempts) this.store.update(job.id, { status: "queued", error: error instanceof Error ? error.message : String(error) }); else this.store.update(job.id, { status: "failed", error: error instanceof Error ? error.message : String(error) }); } finally { this.active = undefined; } } return completed; }
  close(): void { this.active?.abort(); this.store.close(); }
}
