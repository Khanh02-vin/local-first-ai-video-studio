export type EngineWord = { text: string; start: number; end: number; speakerId?: string };
export type EngineSegment = { id: string; text: string; start: number; end: number; words: EngineWord[]; speakerId?: string };

export function validateTranscriptTiming(segments: EngineSegment[], duration: number): void {
  if (!Number.isFinite(duration) || duration <= 0) throw new Error("Invalid transcript duration");
  let previous = 0;
  for (const segment of segments) {
    if (segment.start < 0 || segment.end <= segment.start || segment.end > duration) throw new Error("Invalid transcript segment range");
    let wordPrevious = segment.start;
    for (const word of segment.words) {
      if (word.start < segment.start || word.end <= word.start || word.end > segment.end || word.start < wordPrevious) {
        throw new Error("Invalid transcript word timing");
      }
      wordPrevious = word.end;
    }
    if (segment.start < previous) throw new Error("Transcript segments are not ordered");
    previous = segment.end;
  }
}
