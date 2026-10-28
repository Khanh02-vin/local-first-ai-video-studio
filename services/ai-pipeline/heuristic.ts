import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";

const HOOK_WORDS = /\b(how|why|secret|mistake|important|best|never|first|problem|solution|learn|key)\b/i;
const PAUSE_SECONDS = 0.7;
// Each candidate window covers this many seconds. Long enough to cover typical
// highlight lengths, short enough to stay inside a single sentence.
// Step equals window length, so windows are non-overlapping and stable across
// engines that segment the same audio differently (e.g. openai-whisper vs
// whisper.cpp).
const WINDOW_SECONDS = 10.0;
const WINDOW_STEP_SECONDS = 5.0;
// A window must contain at least this many seconds of audio to be a candidate.
const MIN_WINDOW_SECONDS = 4.0;
// Two chosen highlights may not overlap at all. Keeps highlights distinct and
// satisfies validateHighlights (no overlapping ranges).
const NMS_IOU_THRESHOLD = 0.0;

/** Builds candidate windows over a transcript. Two strategies:
 *  1) Word-based sliding windows: slide a fixed-size window across the word
 *     timeline at WINDOW_STEP_SECONDS. Stable across engines that segment the
 *     same audio differently (e.g. openai-whisper vs whisper.cpp).
 *  2) Segment-anchored fallback: when there are no words at all (e.g. manual
 *     transcript), use each segment as one candidate window.
 * The result is a set of non-overlapping candidates selected by score via NMS. */
function buildWindows(transcript: Transcript): Array<{
  start: number;
  end: number;
  wordIds: string[];
  text: string;
  hook: boolean;
  complete: boolean;
  pauseBoundary: boolean;
  score: number;
}> {
  const words = transcript.words;
  const windows: Array<{
    start: number;
    end: number;
    wordIds: string[];
    text: string;
    hook: boolean;
    complete: boolean;
    pauseBoundary: boolean;
    score: number;
  }> = [];

  const addWindow = (
    start: number,
    end: number,
    wordIds: string[],
    text: string,
    hook: boolean,
    complete: boolean,
    pauseBoundary: boolean,
  ) => {
    const wordCount = wordIds.length || text.split(/\s+/).filter(Boolean).length;
    const score = Math.min(
      100,
      wordCount * 5 + (hook ? 25 : 0) + (complete ? 10 : 0) +
        (pauseBoundary ? 8 : 0),
    );
    windows.push({ start, end, wordIds, text, hook, complete, pauseBoundary, score });
  };

  if (words.length > 0) {
    // Sliding-window mode: slide a fixed-size window across the word timeline.
    // Step equals window length, so windows are non-overlapping and stable.
    let windowStart = 0;
    while (windowStart < transcript.duration) {
      const windowEnd = Math.min(windowStart + WINDOW_SECONDS, transcript.duration);
      if (transcript.duration - windowStart < MIN_WINDOW_SECONDS) break;
      const windowWords = words.filter(
        (w) => w.start >= windowStart && w.start < windowEnd,
      );
      if (windowWords.length === 0) {
        windowStart += WINDOW_STEP_SECONDS;
        continue;
      }
      const s = windowWords[0].start;
      const e = Math.min(
        windowWords[windowWords.length - 1].end,
        windowEnd,
      );
      if (e - s < MIN_WINDOW_SECONDS) {
        windowStart += WINDOW_STEP_SECONDS;
        continue;
      }
      const text = windowWords.map((w) => w.text).join(" ");
      const hook = windowWords.some((w) => HOOK_WORDS.test(w.text));
      const complete = /[.!?]$/.test(
        windowWords[windowWords.length - 1].text.trim(),
      );
      const prevWord = words.findLast((w) => w.end <= windowStart);
      const nextWord = words.find((w) => w.start >= windowEnd);
      const gapBefore = prevWord ? windowStart - prevWord.end : 0;
      const gapAfter = nextWord ? nextWord.start - windowEnd : 0;
      const pauseBoundary =
        gapBefore >= PAUSE_SECONDS || gapAfter >= PAUSE_SECONDS;
      addWindow(s, e, windowWords.map((w) => w.id), text, hook, complete, pauseBoundary);
      windowStart += WINDOW_STEP_SECONDS;
    }
  } else {
    // No words at all: each segment is one candidate window.
    for (const segment of transcript.segments) {
      const duration = segment.end - segment.start;
      if (duration < MIN_WINDOW_SECONDS) continue;
      const text = segment.text.trim();
      const hook = HOOK_WORDS.test(text);
      const complete = /[.!?]$/.test(text);
      const prevSegment = transcript.segments.findLast(
        (s) => s.end <= segment.start,
      );
      const nextSegment = transcript.segments.find(
        (s) => s.start >= segment.end,
      );
      const gapBefore = prevSegment ? segment.start - prevSegment.end : 0;
      const gapAfter = nextSegment ? nextSegment.start - segment.end : 0;
      const pauseBoundary =
        gapBefore >= PAUSE_SECONDS || gapAfter >= PAUSE_SECONDS;
      addWindow(
        segment.start,
        segment.end,
        segment.wordIds,
        text,
        hook,
        complete,
        pauseBoundary,
      );
    }
  }

  return windows;
}

function iou(a: { start: number; end: number }, b: { start: number; end: number }): number {
  const intersection = Math.max(0, Math.min(a.end, b.end) - Math.max(a.start, b.start));
  const union = Math.max(a.end, b.end) - Math.min(a.start, b.start);
  return union > 0 ? intersection / union : 0;
}

/** Non-maximum suppression: keep the highest-scoring window, drop any later window
 *  that overlaps it at all, repeat. Result is sorted by start time so highlights
 *  appear in chronological order and never overlap (satisfies validateHighlights). */
function nonMaximumSuppression(
  windows: Array<{ start: number; end: number; score: number }>,
  limit: number,
): Array<{ start: number; end: number; score: number }> {
  const sorted = [...windows].sort((a, b) => b.score - a.score || a.start - b.start);
  const chosen: Array<{ start: number; end: number; score: number }> = [];
  for (const w of sorted) {
    const overlaps = chosen.some((c) => w.start < c.end && c.start < w.end);
    if (!overlaps) chosen.push(w);
    if (chosen.length >= limit) break;
  }
  return chosen.sort((a, b) => a.start - b.start);
}

export function chooseHeuristicHighlights(transcript: Transcript, limit = 5): Highlight[] {
  const windows = buildWindows(transcript);
  const chosen = nonMaximumSuppression(windows, limit);
  const highlights = chosen.map((w, index) => ({
    version: 1 as const,
    id: `heuristic-${index}`,
    sourceArtifactId: transcript.sourceArtifactId,
    start: w.start,
    end: w.end,
    wordIds: w.wordIds,
    title: w.text.slice(0, 80),
    hook: w.hook ? w.text.slice(0, 100) : undefined,
    score: w.score,
    reason: [
      ...(w.hook ? ["hook keyword"] : []),
      ...(w.complete ? ["complete sentence"] : []),
      ...(w.pauseBoundary ? ["pause boundary"] : []),
      "word window",
    ]
      .filter(Boolean)
      .join(", "),
  }));
  validateHighlights(highlights, transcript.duration);
  return highlights;
}
