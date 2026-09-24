export const CONTRACT_VERSION = 1 as const;

export type Timestamp = number;

export type Word = {
  id: string;
  text: string;
  start: Timestamp;
  end: Timestamp;
  speakerId?: string;
  confidence?: number;
};

export type TranscriptSegment = {
  id: string;
  text: string;
  start: Timestamp;
  end: Timestamp;
  wordIds: string[];
  speakerId?: string;
};

export type Transcript = {
  version: typeof CONTRACT_VERSION;
  sourceArtifactId: string;
  language: string;
  duration: Timestamp;
  words: Word[];
  segments: TranscriptSegment[];
  provider: string;
};

export function assertRange(start: number, end: number, duration?: number): void {
  if (!Number.isFinite(start) || !Number.isFinite(end) || start < 0 || end <= start) {
    throw new Error("Invalid timestamp range");
  }
  if (duration !== undefined && (!Number.isFinite(duration) || end > duration)) {
    throw new Error("Timestamp exceeds duration");
  }
}

export function validateTranscript(value: Transcript): void {
  if (value.version !== CONTRACT_VERSION || !value.sourceArtifactId || !value.language) {
    throw new Error("Invalid transcript metadata");
  }
  if (!Number.isFinite(value.duration) || value.duration <= 0) throw new Error("Invalid transcript duration");
  let previousEnd = 0;
  for (const word of value.words) {
    if (!word.id || !word.text) throw new Error("Invalid transcript word");
    assertRange(word.start, word.end, value.duration);
    if (word.start < previousEnd) throw new Error("Transcript words are not ordered");
    previousEnd = word.end;
  }
  const wordIds = new Set(value.words.map((word) => word.id));
  for (const segment of value.segments) {
    assertRange(segment.start, segment.end, value.duration);
    if (segment.wordIds.some((id) => !wordIds.has(id))) throw new Error("Transcript segment references unknown word");
  }
}
