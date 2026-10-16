import assert from "node:assert/strict";
import { applySpeakerIntervals, transcriptSpeakerIds, speakerExamples } from "../../services/ai-pipeline/diarization.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";

const transcript: Transcript = {
  version: 1, sourceArtifactId: "src", language: "en", duration: 8, provider: "test",
  words: [
    { id: "w1", text: "Hello", start: 0, end: 1 },
    { id: "w2", text: "there", start: 1, end: 2 },
    { id: "w3", text: "other", start: 3, end: 4 },
    { id: "w4", text: "voice", start: 4, end: 5 },
    { id: "w5", text: "tie", start: 6, end: 7 },
  ],
  segments: [
    { id: "s1", text: "Hello there", start: 0, end: 2, wordIds: ["w1", "w2"] },
    { id: "s2", text: "other voice", start: 3, end: 5, wordIds: ["w3", "w4"] },
    { id: "s3", text: "tie", start: 6, end: 7, wordIds: ["w5"] },
  ],
};
const intervals = [
  { start: 3, end: 5, speaker: "SPEAKER_B" },
  { start: 0, end: 2, speaker: "SPEAKER_A" },
  { start: 6, end: 6.5, speaker: "SPEAKER_B" },
  { start: 6.5, end: 7, speaker: "SPEAKER_C" },
];
const result = applySpeakerIntervals(transcript, intervals);
assert.deepEqual(transcriptSpeakerIds(result), ["Speaker 1", "Speaker 2"]);
assert.deepEqual(result.words.map((word) => word.speakerId ?? null), ["Speaker 1", "Speaker 1", "Speaker 2", "Speaker 2", null]);
assert.deepEqual(result.segments.map((segment) => segment.speakerId ?? null), ["Speaker 1", "Speaker 2", null]);
assert.deepEqual(result.words.map(({ id, text, start, end }) => ({ id, text, start, end })), transcript.words.map(({ id, text, start, end }) => ({ id, text, start, end })));
assert.deepEqual(speakerExamples(result).map((entry) => entry.speakerId), ["Speaker 1", "Speaker 2"]);
assert.throws(() => applySpeakerIntervals(transcript, [{ start: -1, end: 2, speaker: "x" }]), /DIARIZATION_INTERVALS_INVALID/);
assert.throws(() => applySpeakerIntervals(transcript, [{ start: 1, end: 9, speaker: "x" }]), /DIARIZATION_INTERVALS_INVALID/);
console.log("speaker diarization mapping tests: ok");
