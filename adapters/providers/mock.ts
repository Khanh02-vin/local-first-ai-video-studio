import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";

export function mockTranscribe(sourceArtifactId: string, duration: number): Transcript {
  const words = [
    { id: "w1", text: "This", start: 1, end: 1.4 },
    { id: "w2", text: "is", start: 1.4, end: 1.7 },
    { id: "w3", text: "a", start: 1.7, end: 1.9 },
    { id: "w4", text: "useful", start: 1.9, end: 2.4 },
    { id: "w5", text: "idea", start: 2.4, end: 3 },
  ];
  return { version: 1, sourceArtifactId, language: "en", duration, provider: "mock", words, segments: [{ id: "s1", text: "This is a useful idea", start: 1, end: 3, wordIds: words.map((word) => word.id) }] };
}

export function mockHighlights(sourceArtifactId: string, transcript: Transcript): Highlight[] {
  const segment = transcript.segments[0];
  return segment ? [{ version: 1, id: "h1", sourceArtifactId, start: segment.start, end: segment.end, wordIds: segment.wordIds, title: "Useful idea", hook: "A useful idea", score: 80, reason: "Complete sentence" }] : [];
}
