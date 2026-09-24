import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript, TranscriptSegment } from "../../packages/contracts/transcript.ts";

const HOOK = /\b(how|why|problem|mistake|secret|important|key|solution|example|first|never|best)\b/i;
const VALUE = /\b(problem|solution|step|example|result|learn|fix|use|because|therefore)\b/i;
const OPEN = /^(and|but|so|because|then|this|it)\b/i;
const WEAK_END = /\b(and|but|because|if|to|is|are)$/i;
export type CandidateQuality = { completeness: number; coherence: number; hook: number; value: number; ending: number; abruptCutPenalty: number; confidence: number };

export function chooseContentHighlights(transcript: Transcript, limit = 5): Highlight[] {
  const segments = transcript.segments.filter((segment) => segment.end > segment.start && segment.text.trim()); const candidates: Highlight[] = [];
  for (let i = 0; i < segments.length; i++) for (let j = i; j < segments.length && segments[j].end - segments[i].start <= 60; j++) {
    const window = segments.slice(i, j + 1); const duration = window.at(-1)!.end - window[0].start; if (duration < 15) continue;
    const text = window.map((segment) => segment.text).join(" ").trim(); const quality = score(window, text); if (quality.completeness < 45 || quality.abruptCutPenalty > 55) continue;
    candidates.push({ version: 1, id: `content-${i}-${j}`, sourceArtifactId: transcript.sourceArtifactId, start: window[0].start, end: window.at(-1)!.end, wordIds: window.flatMap((segment) => segment.wordIds), title: text.slice(0, 80), hook: HOOK.test(window.slice(0, 1)[0].text) ? window[0].text.slice(0, 100) : undefined, score: Math.max(0, Math.min(100, Math.round(quality.completeness * .25 + quality.coherence * .2 + quality.hook * .2 + quality.value * .2 + quality.ending * .15 - quality.abruptCutPenalty * .25))), reason: "content window with hook, context, value and complete ending", quality });
  }
  const ranked = candidates.sort((a, b) => b.score - a.score || a.start - b.start);
  const chosen: Highlight[] = [];
  for (const candidate of ranked) { if (chosen.some((other) => candidate.start < other.end && other.start < candidate.end)) continue; chosen.push(candidate); if (chosen.length >= limit) break; }
  return chosen.sort((a, b) => a.start - b.start) as Highlight[];
}

function score(segments: TranscriptSegment[], text: string): CandidateQuality { const first = segments[0].text.trim(); const last = segments.at(-1)!.text.trim(); const tokens = segments.map((segment) => new Set(segment.text.toLowerCase().split(/\W+/).filter((token) => token.length > 3))); const shared = tokens.length < 2 ? 50 : Math.round((tokens.slice(1).reduce((sum, set) => sum + [...set].filter((token) => tokens[0].has(token)).length, 0) / Math.max(1, tokens.length - 1)) * 20); const completeness = Math.min(100, 45 + Math.min(30, segments.length * 10) + (OPEN.test(first) ? -30 : 15) + (WEAK_END.test(last) ? -30 : 15)); const ending = /[.!?]$/.test(last) ? 90 : 25; const hook = HOOK.test(first) ? 90 : 35; const value = VALUE.test(text) ? 80 : 35; const abruptCutPenalty = (OPEN.test(first) ? 35 : 0) + (WEAK_END.test(last) ? 35 : 0); return { completeness: Math.max(0, completeness), coherence: Math.min(100, 50 + shared), hook, value, ending, abruptCutPenalty, confidence: Math.max(0, Math.min(100, completeness - abruptCutPenalty / 2)) }; }
