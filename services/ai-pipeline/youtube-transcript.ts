import { validateTranscript, type Transcript, type TranscriptSegment, type Word } from "../../packages/contracts/transcript.ts";

export type CaptionPayload = {
  videoId: string;
  duration: number;
  language?: string;
  segments: Array<{ start: number; end: number; text: string }>;
};

/**
 * Builds a contract-valid Transcript from YouTube caption segments, which only
 * carry segment-level timestamps. Each segment's time is split across its
 * tokens in proportion to character count so words stay ordered and inside
 * their segment; highlight boundaries still use the exact caption times.
 * Segments that overlap by a few milliseconds are clamped to the previous end
 * so the word-order invariant holds across segment boundaries.
 */
export function transcriptFromCaptions(payload: CaptionPayload): Transcript {
  const duration = payload.duration;
  const words: Word[] = [];
  const segments: TranscriptSegment[] = [];
  let cursor = 0;
  const ordered = [...payload.segments].sort((a, b) => a.start - b.start);
  for (const [index, segment] of ordered.entries()) {
    if (!Number.isFinite(segment.start) || !Number.isFinite(segment.end)) continue;
    const text = segment.text.replace(/\s+/g, " ").trim();
    if (!text) continue;
    const start = Math.max(cursor, Math.min(segment.start, duration));
    const end = Math.min(segment.end, duration);
    if (end <= start) continue;
    const tokens = text.split(" ");
    const totalChars = tokens.reduce((sum, token) => sum + token.length, 0) || 1;
    const wordIds: string[] = [];
    let at = start;
    for (const [tokenIndex, token] of tokens.entries()) {
      // Last token absorbs any rounding so the segment's final word reaches its end.
      const wordEnd = tokenIndex === tokens.length - 1 ? end : at + (end - start) * (token.length / totalChars);
      if (wordEnd <= at) continue;
      const id = `w-${index}-${tokenIndex}`;
      words.push({ id, text: token, start: at, end: wordEnd });
      wordIds.push(id);
      at = wordEnd;
    }
    if (!wordIds.length) continue;
    segments.push({ id: `s-${index}`, text, start, end, wordIds });
    cursor = end;
  }
  const transcript: Transcript = {
    version: 1,
    sourceArtifactId: payload.videoId,
    language: payload.language?.trim() || "und",
    duration,
    words,
    segments,
    provider: "youtube-captions",
  };
  validateTranscript(transcript);
  return transcript;
}
