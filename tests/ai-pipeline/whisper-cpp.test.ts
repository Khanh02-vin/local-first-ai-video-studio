import assert from "node:assert/strict";
import { transcriptFromWhisperCpp } from "../../adapters/local/whisper-cpp.ts";

const transcript = transcriptFromWhisperCpp(
  {
    result: { language: "en" },
    transcription: [
      {
        offsets: { from: 0, to: 4200 },
        text: " hello world",
        tokens: [
          { text: " hello", offsets: { from: 0, to: 2000 } },
          { text: " world", offsets: { from: 2000, to: 4200 } },
        ],
      },
      {
        offsets: { from: 5000, to: 9000 },
        text: " important solution",
        tokens: [
          { text: " important", offsets: { from: 5000, to: 6500 } },
          { text: " solution", offsets: { from: 6500, to: 9000 } },
        ],
      },
    ],
  },
  "asset-1",
  10,
);
assert.equal(transcript.provider, "whisper.cpp:tiny");
assert.equal(transcript.language, "en");
assert.equal(transcript.words.length, 4);
assert.deepEqual(transcript.words.map((word) => word.text), ["hello", "world", "important", "solution"]);
assert.equal(transcript.words[0].start, 0);
assert.equal(transcript.words[3].end, 9);
assert.equal(transcript.segments.length, 2);

const empty = transcriptFromWhisperCpp(
  { transcription: [{ offsets: { from: 0, to: 0 }, text: "", tokens: [] }] },
  "asset-2",
  10,
);
assert.equal(empty.words.length, 0);
assert.equal(empty.segments.length, 0);

console.log("whisper.cpp parser tests: ok");
