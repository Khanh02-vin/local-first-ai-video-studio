import { mkdir, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { transcribeWithWhisper, whisperDevice, whisperFp16, type WhisperOptions } from "./whisper.ts";
import { transcribeWithWhisperCpp } from "./whisper-cpp.ts";
import { resolveWhisperCommand, resolveWhisperModelPath } from "./whisper-engine.ts";
import { runAbortable } from "./abortable.ts";
import type { Transcript, Word, TranscriptSegment } from "../../packages/contracts/transcript.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";
import { sourceFingerprint, chunkCacheKey, cachePath, readCachedTranscript, writeCachedTranscript } from "./analysis-cache.ts";

export type ChunkedWhisperOptions = WhisperOptions & { ffmpegCommand?: string; chunkSeconds?: number; overlapSeconds?: number; offsetSeconds?: number; sourceDuration?: number; chunkTimeoutMs?: number; onProgress?: (completed: number, total: number) => void; onChunkStart?: (index: number, total: number) => void; onChunkComplete?: (index: number, total: number, cacheHit: boolean) => void; onChunkFail?: (index: number, total: number, error: string) => void };

export async function transcribeChunkedWithWhisper(audioPath: string, sourceArtifactId: string, duration: number, options: ChunkedWhisperOptions = {}): Promise<Transcript> {
  const engine = options.engine ?? process.env.WHISPER_ENGINE ?? "cpp";
  const cppCommand = resolveWhisperCommand("cpp", options.command ?? process.env.WHISPER_COMMAND, "whisper-cli");
  const cppModel = resolveWhisperModelPath("cpp", options.modelPath ?? process.env.WHISPER_MODEL_PATH, process.env.WHISPER_CPP_MODEL ?? "");
  const chunkSeconds = Math.max(30, options.chunkSeconds ?? Number(process.env.WHISPER_CHUNK_SECONDS ?? 120));
  const overlap = Math.min(5, Math.max(0, options.overlapSeconds ?? 1));
  const offsetSeconds = options.offsetSeconds ?? 0;
  const total = Math.max(1, Math.ceil(duration / chunkSeconds));
  const source = await sourceFingerprint(audioPath); const root = await import("node:fs/promises").then(({ mkdtemp }) => mkdtemp(join(tmpdir(), "local-whisper-chunks-")));
  const words: Word[] = []; const wordIndex = new Map<string, Word>(); const segments: TranscriptSegment[] = []; const segmentIndex = new Map<string, TranscriptSegment>(); let language = options.language ?? "und";
  try {
    for (let index = 0; index < total; index++) {
      // A cancel must stop the remaining chunks, not just the one in flight.
      if (options.shouldAbort?.()) throw new Error("ANALYZE_INTERRUPTED");
      const start = offsetSeconds + Math.max(0, index * chunkSeconds - (index ? overlap : 0));
      const end = Math.min(offsetSeconds + duration, offsetSeconds + (index + 1) * chunkSeconds);
      const key = await chunkCacheKey(audioPath, index, start, end, `${engine}:${cppModel || options.modelPath || options.model || "tiny"}`, options.language, whisperDevice(options), whisperFp16(options), source);
      options.onChunkStart?.(index, total);
      let cacheHit = false;
      let part: Transcript;
      try {
        const cached = await readCachedTranscript(cachePath(source, key), options.sourceDuration ?? duration);
        cacheHit = Boolean(cached);
        part = cached ?? await (async () => {
          const chunk = join(root, `chunk-${index}.wav`);
          try {
            await runAbortable(options.ffmpegCommand ?? process.env.FFMPEG_COMMAND ?? "ffmpeg", ["-y", "-nostdin", "-hide_banner", "-loglevel", "error", "-ss", start.toFixed(3), "-t", (end - start).toFixed(3), "-i", audioPath, "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", chunk], { timeoutMs: options.chunkTimeoutMs ?? 120_000, abortLabel: "CHUNK_FFMPEG_ABORTED", shouldAbort: options.shouldAbort });
            const result = engine === "cpp"
              ? await transcribeWithWhisperCpp(chunk, sourceArtifactId, options.sourceDuration ?? duration, { command: cppCommand ?? "whisper-cli", modelPath: cppModel ?? "", language: options.language, timestampOffset: start, idPrefix: `c${index}-`, shouldAbort: options.shouldAbort })
              : await transcribeWithWhisper(chunk, sourceArtifactId, options.sourceDuration ?? duration, { ...options, timestampOffset: start, idPrefix: `c${index}-` });
            await writeCachedTranscript(cachePath(source, key), result); return result;
          } finally { await rm(chunk, { force: true }); }
        })();
      } catch (error) {
        options.onChunkFail?.(index, total, error instanceof Error ? error.message : String(error));
        throw error;
      }
      if (index === 0 && part.language !== "und") language = part.language;
      const wordMap = new Map<string, string>();
      for (const word of part.words) { const key = `${word.text.trim().toLowerCase()}|${word.start.toFixed(2)}|${word.end.toFixed(2)}`; const existing = wordIndex.get(key); const canonical = existing ?? word; wordIndex.set(key, canonical); if (!existing) words.push(word); wordMap.set(word.id, canonical.id); }
      for (const segment of part.segments) { const mapped = { ...segment, wordIds: segment.wordIds.map((id) => wordMap.get(id) ?? id).filter((id, position, ids) => ids.indexOf(id) === position) }; const key = `${mapped.text.trim().toLowerCase()}|${mapped.start.toFixed(2)}|${mapped.end.toFixed(2)}`; if (!segmentIndex.has(key)) { segmentIndex.set(key, mapped); segments.push(mapped); } }
      options.onChunkComplete?.(index, total, cacheHit);
      options.onProgress?.(index + 1, total);
    }
    words.sort((a, b) => a.start - b.start || a.end - b.end);
    // Chunk overlaps can leave a boundary word starting before the previous
    // word ends (whisper's per-chunk timings are approximate); the transcript
    // contract requires ordered, non-overlapping words. Keep the earlier word,
    // drop the overlapping one, and prune segment references so they never dangle.
    const orderedWords: Word[] = [];
    for (const word of words) {
      const previous = orderedWords[orderedWords.length - 1];
      if (previous && word.start < previous.end) continue;
      orderedWords.push(word);
    }
    if (orderedWords.length !== words.length) {
      const kept = new Set(orderedWords.map((word) => word.id));
      for (const segment of segments) segment.wordIds = segment.wordIds.filter((id) => kept.has(id));
    }
    words.length = 0;
    words.push(...orderedWords);
    segments.sort((a, b) => a.start - b.start);
    const transcript: Transcript = { version: 1, sourceArtifactId, language, duration: options.sourceDuration ?? duration, provider: `openai-whisper:${options.model ?? "tiny"}`, words, segments };
    validateTranscript(transcript); return transcript;
  } finally { await rm(root, { recursive: true, force: true }); }
}
