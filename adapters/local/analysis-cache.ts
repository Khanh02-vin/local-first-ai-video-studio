import { createHash } from "node:crypto";
import { readFile, rename, stat, writeFile, mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

export const PIPELINE_VERSION = "content-v2";
export async function sourceFingerprint(path: string): Promise<string> { const info = await stat(path); return createHash("sha256").update(`${path}\0${info.size}\0${info.mtimeMs}`).digest("hex"); }
export async function chunkCacheKey(sourcePath: string, index: number, start: number, end: number, model: string, language?: string, device?: string, fp16?: boolean, initialSource?: string): Promise<string> {
  const current = await sourceFingerprint(sourcePath);
  // An immutable source-fingerprint is captured at init and carried through. If it diverges
  // from the current file, the source was replaced mid-run, so the cached transcript (if any)
  // is stale and the key itself must change to force a re-run rather than a stale cache hit.
  const source = initialSource && initialSource !== current ? `stale:\u0000${initialSource}\u0000${current}` : current;
  return createHash("sha256").update([PIPELINE_VERSION, source, index, start.toFixed(3), end.toFixed(3), model, language ?? "auto", `fp16=${Boolean(fp16)}`, `device=${device ?? "cpu"}`].join("\0")).digest("hex");
}
export async function sourceFingerprintAtInit(path: string): Promise<string> { return sourceFingerprint(path); }
export function cachePath(source: string, key: string): string { return join(process.env.LOCAL_FIRST_STATE_DIR ?? join(process.env.HOME ?? ".", ".cache/local-first-ai-video-studio"), "analysis", source, PIPELINE_VERSION, key + ".json"); }
export async function readCachedTranscript(path: string, duration: number): Promise<Transcript | undefined> { try { const value = JSON.parse(await readFile(path, "utf8")) as Transcript; validateTranscript(value); if (value.duration !== duration) return undefined; return value; } catch { return undefined; } }
export async function writeCachedTranscript(path: string, transcript: Transcript): Promise<void> { validateTranscript(transcript); await mkdir(dirname(path), { recursive: true }); const temp = `${path}.${process.pid}.tmp`; await writeFile(temp, JSON.stringify(transcript)); await rename(temp, path); }
