import { assertRange, CONTRACT_VERSION, type Timestamp } from "./transcript.ts";

export type Highlight = {
  version: typeof CONTRACT_VERSION;
  id: string;
  sourceArtifactId: string;
  start: Timestamp;
  end: Timestamp;
  wordIds: string[];
  title: string;
  hook?: string;
  score: number;
  reason?: string;
};

export function validateHighlights(highlights: Highlight[], duration: number): void {
  const ordered = [...highlights].sort((a, b) => a.start - b.start);
  let previousEnd = 0;
  for (const highlight of ordered) {
    if (highlight.version !== CONTRACT_VERSION || !highlight.id || !highlight.sourceArtifactId) {
      throw new Error("Invalid highlight metadata");
    }
    assertRange(highlight.start, highlight.end, duration);
    if (!Number.isFinite(highlight.score) || highlight.score < 0 || highlight.score > 100) {
      throw new Error("Invalid highlight score");
    }
    if (highlight.start < previousEnd) throw new Error("Highlights overlap");
    previousEnd = highlight.end;
  }
}
