import { transcribeWithWhisper, type WhisperOptions } from "../../adapters/local/whisper.ts";
import { transcribeChunkedWithWhisper, type ChunkedWhisperOptions } from "../../adapters/local/chunked-whisper.ts";
import { chooseContentHighlights } from "./content-highlights.ts";
import { chooseHeuristicHighlights } from "./heuristic.ts";
import { validateHighlights, type Highlight } from "../../packages/contracts/highlight.ts";
import { validateTranscript, type Transcript } from "../../packages/contracts/transcript.ts";
// ponytail: mockTranscribe/mockHighlights deleted — analyzeVideo now returns deterministic placeholder.
// Real callers use analyzeLocalVideo/analyzeLocalVideoWithSemantic.
export type ClipAnalysis = { transcript: Transcript; highlights: Highlight[] };
export type LocalClipAnalysis = { transcript: Transcript; highlights: Highlight[] };

export function analyzeVideo(sourceArtifactId: string, duration: number): ClipAnalysis {
  const transcript: Transcript = { version: 1, sourceArtifactId, language: "und", duration, provider: "mock", words: [{ id: "w1", text: "placeholder", start: 0, end: duration }], segments: [{ id: "s1", text: "placeholder", start: 0, end: duration, wordIds: ["w1"] }] };
  validateTranscript(transcript);
  const highlights: Highlight[] = [{ version: 1, id: "h1", sourceArtifactId, start: 0, end: duration, wordIds: ["w1"], title: "Placeholder highlight", score: 50 }];
  validateHighlights(highlights, duration);
  return { transcript, highlights };
}

export type LocalAnalyzeOptions = WhisperOptions & Pick<ChunkedWhisperOptions, "onProgress" | "onChunkStart" | "onChunkComplete" | "onChunkFail" | "chunkSeconds" | "overlapSeconds" | "offsetSeconds">;

export async function analyzeLocalVideo(audioPath: string, sourceArtifactId: string, duration: number, options: LocalAnalyzeOptions = {}): Promise<LocalClipAnalysis> {
  const sourceDuration = options.sourceDuration ?? duration;
  const transcript = duration > 120 ? await transcribeChunkedWithWhisper(audioPath, sourceArtifactId, duration, { ...options, sourceDuration }) : await transcribeWithWhisper(audioPath, sourceArtifactId, sourceDuration, options);
  const highlights = duration > 120 ? chooseContentHighlights(transcript) : chooseHeuristicHighlights(transcript);
  validateTranscript(transcript);
  validateHighlights(highlights, duration);
  return { transcript, highlights };
}

export type SemanticAnalyzeOptions = LocalAnalyzeOptions & {
  semanticProvider?: SemanticHighlightProvider;
  semanticCategory?: string;
  semanticLimit?: number;
};

export async function analyzeLocalVideoWithSemantic(
  audioPath: string,
  sourceArtifactId: string,
  duration: number,
  options: SemanticAnalyzeOptions = {}
): Promise<LocalClipAnalysis> {
  const sourceDuration = options.sourceDuration ?? duration;
  const transcript = duration > 120
    ? await transcribeChunkedWithWhisper(audioPath, sourceArtifactId, duration, { ...options, sourceDuration })
    : await transcribeWithWhisper(audioPath, sourceArtifactId, sourceDuration, options);

  const provider = options.semanticProvider ?? new NotImplementedSemanticProvider();
  let highlights: Highlight[];
  try {
    highlights = await generateSemanticHighlights(transcript, provider, {
      category: options.semanticCategory,
      limit: options.semanticLimit,
      minDuration: 10,
      maxDuration: 90,
    });
  } catch (error) {
    // Provider (e.g. Gemini) failure or unsupported input: fall back to the
    // deterministic selector so the user still gets candidates instead of a hard error.
    const fallback = duration > 120 ? chooseContentHighlights(transcript) : chooseHeuristicHighlights(transcript);
    console.warn(`[ai-pipeline] semantic provider failed (${error instanceof Error ? error.message : error}); fell back to ${duration > 120 ? "content" : "heuristic"} selector`);
    highlights = fallback;
  }
  validateTranscript(transcript);
  validateHighlights(highlights, duration);
  return { transcript, highlights };
}

// Re-export all symbols from semantic.ts so they are available at package boundary
export * from "./semantic.ts";