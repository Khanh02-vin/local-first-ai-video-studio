import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

export type SpeakerInterval = { start: number; end: number; speaker: string };

export function applySpeakerIntervals(transcript: Transcript, intervals: unknown): Transcript {
  validateTranscript(transcript);
  if (!Array.isArray(intervals)) throw new Error("DIARIZATION_INTERVALS_INVALID");
  const valid: SpeakerInterval[] = intervals.map((value) => {
    if (!value || typeof value !== "object") throw new Error("DIARIZATION_INTERVALS_INVALID");
    const interval = value as Record<string, unknown>;
    if (typeof interval.start !== "number" || typeof interval.end !== "number" || !Number.isFinite(interval.start) || !Number.isFinite(interval.end) || interval.start < 0 || interval.end <= interval.start || interval.end > transcript.duration || typeof interval.speaker !== "string" || !interval.speaker.trim()) throw new Error("DIARIZATION_INTERVALS_INVALID");
    return { start: interval.start, end: interval.end, speaker: interval.speaker };
  });
  const order = new Map<string, number>();
  for (const interval of valid) order.set(interval.speaker, Math.min(order.get(interval.speaker) ?? Infinity, interval.start));
  const speakerNames = new Map([...order].sort((a, b) => a[1] - b[1] || a[0].localeCompare(b[0])).map(([speaker], index) => [speaker, `Speaker ${index + 1}`]));
  const words = transcript.words.map((word) => {
    let best: SpeakerInterval | undefined;
    let bestOverlap = 0;
    let tied = false;
    for (const interval of valid) {
      const overlap = Math.max(0, Math.min(word.end, interval.end) - Math.max(word.start, interval.start));
      if (overlap > bestOverlap + 1e-9) { best = interval; bestOverlap = overlap; tied = false; }
      else if (overlap > 0 && Math.abs(overlap - bestOverlap) <= 1e-9 && best?.speaker !== interval.speaker) tied = true;
    }
    const speakerId = best && !tied ? speakerNames.get(best.speaker) : undefined;
    const { speakerId: _old, ...original } = word;
    return speakerId ? { ...original, speakerId } : original;
  });
  const wordById = new Map<string, Transcript["words"][number]>(words.map((word) => [word.id, word]));
  const segments = transcript.segments.map((segment) => {
    const counts = new Map<string, number>();
    for (const id of segment.wordIds) { const speakerId = wordById.get(id)?.speakerId; if (speakerId) counts.set(speakerId, (counts.get(speakerId) ?? 0) + 1); }
    const ranked = [...counts].sort((a, b) => b[1] - a[1]);
    const speakerId = ranked.length && ranked[0][1] > (ranked[1]?.[1] ?? 0) ? ranked[0][0] : undefined;
    const { speakerId: _old, ...original } = segment;
    return speakerId ? { ...original, speakerId } : original;
  });
  const result = { ...transcript, words, segments };
  validateTranscript(result);
  return result;
}

export function transcriptSpeakerIds(transcript: Transcript): string[] {
  return [...new Set(transcript.words.flatMap((word) => word.speakerId ? [word.speakerId] : []))];
}

export function speakerExamples(transcript: Transcript): Array<{ speakerId: string; text: string }> {
  return transcript.segments.flatMap((segment) => segment.speakerId ? [{ speakerId: segment.speakerId, text: segment.text }] : []);
}
