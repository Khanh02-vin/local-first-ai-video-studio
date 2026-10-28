import { runAbortable } from "./abortable.ts";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

export type WhisperOptions = { engine?: "python" | "cpp"; command?: string; model?: string; modelPath?: string; language?: string; timeoutMs?: number; timestampOffset?: number; idPrefix?: string; fp16?: boolean; threads?: number; device?: string; shouldAbort?: () => boolean };

export function whisperCommand(options: WhisperOptions = {}): string { return options.command ?? process.env.WHISPER_COMMAND ?? "whisper"; }
export function whisperDevice(options: WhisperOptions = {}): string { return options.device ?? process.env.WHISPER_DEVICE ?? "cpu"; }
export function whisperFp16(options: WhisperOptions = {}): boolean { return options.fp16 ?? whisperDevice(options) === "cuda"; }

export async function transcribeWithWhisper(audioPath: string, sourceArtifactId: string, duration: number, options: WhisperOptions = {}): Promise<Transcript> {
  const dir = await mkdtemp(join(tmpdir(), "local-whisper-"));
  const output = join(dir, "result.json");
  try {
    const device = whisperDevice(options); const fp16 = whisperFp16(options);
    const args = [audioPath, "--model", options.model ?? "tiny", "--fp16", fp16 ? "True" : "False", "--device", device, "--word_timestamps", "True", "--output_format", "json", "--output_dir", dir];
    if (options.language) args.push("--language", options.language);
    await runAbortable(whisperCommand(options), args, {
      timeoutMs: options.timeoutMs ?? 20 * 60_000,
      abortLabel: "WHISPER_ABORTED",
      shouldAbort: options.shouldAbort,
    });
    const files = await (await import("node:fs/promises")).readdir(dir);
    const jsonName = files.find((name) => name.endsWith(".json"));
    if (!jsonName) throw new Error("WHISPER_OUTPUT_MISSING");
    const raw = JSON.parse(await readFile(join(dir, jsonName), "utf8")) as { language?: string; segments?: Array<{ id?: number; start: number; end: number; text: string; words?: Array<{ word: string; start: number; end: number }> }> };
    const words = (raw.segments ?? []).flatMap((segment, si) => (segment.words ?? []).map((word, wi) => ({ id: `w-${si}-${wi}`, text: word.word.trim(), start: word.start, end: word.end })));
    const offset = options.timestampOffset ?? 0; const prefix = options.idPrefix ?? "";
    const clamp = (value: number) => Math.max(0, Math.min(value, duration));
    const shiftedWords = words.map((word) => ({ ...word, id: `${prefix}${word.id}`, start: clamp(word.start + offset), end: clamp(word.end + offset) })).filter((word) => word.end > word.start && word.start < duration);
    const keptIds = new Set(shiftedWords.map((word) => word.id));
    const transcript: Transcript = { version: 1, sourceArtifactId, language: raw.language ?? options.language ?? "und", duration, provider: `openai-whisper:${options.model ?? "tiny"}`, words: shiftedWords, segments: (raw.segments ?? []).map((segment, index) => ({ id: `${prefix}s-${segment.id ?? index}`, text: segment.text.trim(), start: clamp(segment.start + offset), end: clamp(segment.end + offset), wordIds: shiftedWords.filter((word) => word.start >= segment.start + offset && word.end <= segment.end + offset).map((word) => word.id).filter((id) => keptIds.has(id)) })).filter((segment) => segment.end > segment.start && segment.start < duration) };
    validateTranscript(transcript); return transcript;
  } finally { await rm(dir, { recursive: true, force: true }); }
}
