import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";

const HOOK_WORDS = /\b(how|why|secret|mistake|important|best|never|first|problem|solution|learn|key)\b/i;

export function chooseHeuristicHighlights(transcript: Transcript, limit = 5): Highlight[] {
  const candidates = transcript.segments.map((segment, index) => {
    const words = segment.text.trim().split(/\s+/).filter(Boolean).length;
    const score = Math.min(100, words * 5 + (HOOK_WORDS.test(segment.text) ? 25 : 0) + (/[.!?]$/.test(segment.text.trim()) ? 10 : 0));
    return { version: 1 as const, id: `heuristic-${index}`, sourceArtifactId: transcript.sourceArtifactId, start: segment.start, end: segment.end, wordIds: segment.wordIds, title: segment.text.slice(0, 80), hook: HOOK_WORDS.test(segment.text) ? segment.text.slice(0, 100) : undefined, score, reason: "sentence length, hook keywords and complete-sentence boundary" };
  }).filter((item) => item.end - item.start >= 5).sort((a, b) => b.score - a.score || a.start - b.start).slice(0, limit).sort((a, b) => a.start - b.start);
  validateHighlights(candidates, transcript.duration);
  return candidates;
}
