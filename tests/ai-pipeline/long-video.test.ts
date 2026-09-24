import assert from "node:assert/strict";
import { highlightWindows, mergeWindowHighlights, GeminiHighlightProvider } from "../../services/ai-pipeline/providers.ts";
import type { Transcript, TranscriptSegment, Word } from "../../packages/contracts/transcript.ts";

// Build a synthetic 3-hour transcript: 10800 seconds, one word per 2s, 20 words per segment.
function buildLongTranscript(): Transcript {
  const duration = 3 * 3600;
  const words: Word[] = [];
  const segments: TranscriptSegment[] = [];
  for (let t = 0; t < duration; t += 2) {
    words.push({ id: `w-${t}`, text: `word-${t}`, start: t, end: t + 1 });
  }
  for (let t = 0; t < duration; t += 40) {
    const ids: string[] = [];
    for (let i = 0; i < 20; i++) ids.push(`w-${t + i * 2}`);
    segments.push({ id: `s-${t}`, text: `segment ${t}`, start: t, end: t + 40, wordIds: ids });
  }
  return { version: 1, sourceArtifactId: "long", language: "en", duration, provider: "fixture", words, segments };
}

const transcript = buildLongTranscript();

// 20-min windows with 5-min overlap -> step 15 min. 10800s / 900s = 12 windows.
const windows = highlightWindows(transcript, 20 * 60, 5 * 60);
assert.ok(windows.length >= 11 && windows.length <= 13, `expected ~12 windows, got ${windows.length}`);
assert.equal(windows[0].from, 0);
assert.equal(windows[windows.length - 1].to, transcript.duration);
// Every window transcript is internally valid (timestamps shifted to window-local)
for (const win of windows) {
  assert.ok(win.window.duration > 0);
  assert.ok(win.window.words.every((w) => w.start >= 0 && w.end <= win.window.duration), "window-local word times must be shifted and in-bounds");
}
console.log("windows ok:", windows.length, "total span", transcript.duration, "s");

// Simulated per-window provider: each window proposes 1 highlight in its own local time,
// high score for the "middle" window. Merge must shift back to absolute time and dedupe.
const fakePerWindow = windows.map((win, i) => [{
  version: 1 as const,
  id: `h-${i}`,
  sourceArtifactId: "long",
  start: Math.max(0, win.window.duration / 2 - 15),
  end: Math.max(0, win.window.duration / 2 + 15),
  wordIds: win.window.words.slice(0, 5).map((w) => w.id),
  title: `clip ${i}`,
  score: i === Math.floor(windows.length / 2) ? 95 : 50 + i,
  reason: "fixture",
}]);
const merged = mergeWindowHighlights(fakePerWindow.map((h) => [{ ...h, start: h.start + (windows.findIndex(w => w === windows[0]) === 0 ? 0 : 0), end: h.end }]), transcript);
// NOTE: mergeWindowHighlights takes absolute-time highlights; build them by shifting.
const absolutePerWindow = windows.map((win, i) => [{
  version: 1 as const,
  id: `h-${i}`,
  sourceArtifactId: "long",
  start: win.from + (win.window.duration / 2 - 15),
  end: win.from + (win.window.duration / 2 + 15),
  wordIds: win.window.words.slice(0, 5).map((w) => w.id),
  title: `clip ${i}`,
  score: i === Math.floor(windows.length / 2) ? 95 : 50 + i,
  reason: "fixture",
}]);
const finalMerged = mergeWindowHighlights(absolutePerWindow, transcript);
assert.ok(finalMerged.length > 0 && finalMerged.length <= 5);
assert.ok(finalMerged.every((h) => h.start >= 0 && h.end <= transcript.duration), "merged highlights stay within source bounds");
// Highest-scored window should win.
const top = finalMerged[0];
assert.equal(top.title, `clip ${Math.floor(windows.length / 2)}`);
console.log("merge ok:", finalMerged.length, "highlights; top:", top.title, `[${top.start}-${top.end}s]`);

console.log("long-video map-reduce tests: ok");
