import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";

const HOOK_WORDS = /\b(how|why|secret|mistake|important|best|never|first|problem|solution|learn|key)\b/i;
/** A gap this long between segments marks a topic boundary — a natural clip
 *  edge. Segment boundaries come straight from Whisper's word timings, so the
 *  signal is free: no model, no extra pass over the audio. */
const PAUSE_SECONDS = 0.7;

export function chooseHeuristicHighlights(transcript: Transcript, limit = 5): Highlight[] {
  const segments = transcript.segments;
  const candidates = segments.map((segment, index) => {
    const words = segment.text.trim().split(/\s+/).filter(Boolean).length;
    const hook = HOOK_WORDS.test(segment.text);
    const complete = /[.!?]$/.test(segment.text.trim());
    // Internal gaps only: video start/end are not pauses.
    const gapBefore = index === 0 ? 0 : segment.start - segments[index - 1].end;
    const gapAfter = index === segments.length - 1 ? 0 : segments[index + 1].start - segment.end;
    const pauseBoundary = gapBefore >= PAUSE_SECONDS || gapAfter >= PAUSE_SECONDS;
    const score = Math.min(100, words * 5 + (hook ? 25 : 0) + (complete ? 10 : 0) + (pauseBoundary ? 8 : 0));
    const reasons = ["sentence length"];
    if (hook) reasons.push("hook keyword");
    if (complete) reasons.push("complete sentence");
    if (pauseBoundary) reasons.push("pause boundary");
    return {
      version: 1 as const,
      id: `heuristic-${index}`,
      sourceArtifactId: transcript.sourceArtifactId,
      start: segment.start,
      end: segment.end,
      wordIds: segment.wordIds,
      title: segment.text.slice(0, 80),
      hook: hook ? segment.text.slice(0, 100) : undefined,
      score,
      reason: reasons.join(", "),
    };
  });
  const chosen = candidates.filter((item) => item.end - item.start >= 5).sort((a, b) => b.score - a.score || a.start - b.start).slice(0, limit).sort((a, b) => a.start - b.start);
  validateHighlights(chosen, transcript.duration);
  return chosen;
}
