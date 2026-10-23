export type RelatedVideo = { videoId: string; title: string; thumbnail: string; url: string };

export function rankCandidates(source: number[], vectors: number[][], candidates: RelatedVideo[]): RelatedVideo[] {
  if (!source.length || vectors.length !== candidates.length || vectors.some((vector) => vector.length !== source.length)) return candidates;
  const magnitude = Math.sqrt(source.reduce((sum, value) => sum + value * value, 0));
  if (!magnitude) return candidates;
  return candidates.map((video, index) => {
    const vector = vectors[index];
    const otherMagnitude = Math.sqrt(vector.reduce((sum, value) => sum + value * value, 0));
    const dot = source.reduce((sum, value, i) => sum + value * vector[i], 0);
    return { video, score: otherMagnitude ? dot / (magnitude * otherMagnitude) : 0 };
  }).sort((a, b) => b.score - a.score).map(({ video }) => video);
}

// Browser guard: `process` does not exist in a webview, and evaluating the
// right-hand side at import time would crash hydration for every UI that
// imports this module.
if (typeof process !== "undefined" && process.argv && import.meta.url === `file://${process.argv[1]}`) {
  const videos = [{ videoId: "a", title: "a", thumbnail: "", url: "" }, { videoId: "b", title: "b", thumbnail: "", url: "" }];
  if (rankCandidates([1, 0], [[0, 1], [1, 0]], videos)[0].videoId !== "b") throw new Error("related ranking self-check failed");
}
