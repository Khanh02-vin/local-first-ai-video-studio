import { readFile, mkdtemp, rm } from "node:fs/promises";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { runAbortable } from "./abortable.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

export type WhisperCppOptions = {
  command: string;
  modelPath: string;
  language?: string;
  timeoutMs?: number;
  timestampOffset?: number;
  idPrefix?: string;
  shouldAbort?: () => boolean;
};

type Token = { text?: string; offsets?: { from?: number; to?: number } };
type Segment = { offsets?: { from?: number; to?: number }; text?: string; tokens?: Token[] };
type Output = { transcription?: Segment[]; result?: { language?: string } };

/** Parse whisper.cpp -ojf output into the shared Transcript contract. */
export function transcriptFromWhisperCpp(
  json: Output,
  sourceArtifactId: string,
  duration: number,
  options: Pick<WhisperCppOptions, "language" | "timestampOffset" | "idPrefix"> = {},
): Transcript {
  const offset = options.timestampOffset ?? 0;
  const prefix = options.idPrefix ?? "";
  const clamp = (value: number) => Math.max(0, Math.min(value, duration));
  const segments = json.transcription ?? [];
  const words = segments.flatMap((segment, si) => {
    const tokens = segment.tokens ?? [];
    const output: Array<{ id: string; text: string; start: number; end: number }> = [];
    for (const [ti, token] of tokens.entries()) {
      const raw = token.text ?? "";
      const text = raw.trim();
      if (!text || /^_\[|^\[_|^__/.test(raw)) continue;
      const start = clamp((token.offsets?.from ?? 0) / 1000 + offset);
      const end = clamp((token.offsets?.to ?? 0) / 1000 + offset);
      if (end <= start) continue;
      const previous = output[output.length - 1];
      if (previous && !raw.startsWith(" ")) {
        previous.text += text;
        previous.end = Math.max(previous.end, end);
      } else {
        output.push({ id: `${prefix}w-${si}-${ti}`, text, start, end });
      }
    }
    return output;
  });
  const wordIds = new Set(words.map((word) => word.id));
  const transcript: Transcript = {
    version: 1,
    sourceArtifactId,
    language: json.result?.language ?? options.language ?? "und",
    duration,
    provider: "whisper.cpp:tiny",
    words,
    segments: segments
      .map((segment, index) => {
        const start = clamp((segment.offsets?.from ?? 0) / 1000 + offset);
        const end = clamp((segment.offsets?.to ?? 0) / 1000 + offset);
        const segmentWordIds = words
          .filter((word) => word.start >= start && word.end <= end)
          .map((word) => word.id)
          .filter((id) => wordIds.has(id));
        return { id: `${prefix}s-${index}`, text: (segment.text ?? "").trim(), start, end, wordIds: segmentWordIds };
      })
      .filter((segment) => segment.end > segment.start),
  };
  validateTranscript(transcript);
  return transcript;
}

export async function transcribeWithWhisperCpp(
  audioPath: string,
  sourceArtifactId: string,
  duration: number,
  options: WhisperCppOptions,
): Promise<Transcript> {
  const dir = await mkdtemp(join(tmpdir(), "local-whisper-cpp-"));
  const outputPrefix = join(dir, "result");
  try {
    await runAbortable(
      options.command,
      ["-m", options.modelPath, "-f", audioPath, "-ojf", "-of", outputPrefix, ...(options.language ? ["-l", options.language] : [])],
      { timeoutMs: options.timeoutMs ?? 20 * 60_000, abortLabel: "WHISPER_CPP_ABORTED", shouldAbort: options.shouldAbort, env: { LD_LIBRARY_PATH: dirname(options.command) } },
    );
    const json = JSON.parse(await readFile(`${outputPrefix}.json`, "utf8")) as Output;
    return transcriptFromWhisperCpp(json, sourceArtifactId, duration, options);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}
