/// Transcript-engine parity benchmark (E layer 1, measurement only).
///
/// Decides whether swapping openai-whisper for whisper.cpp (E layer 2, ADR pending)
/// would change what the product produces. The eval-highlights harness cannot answer
/// that — its fixtures carry curated text with synthesized word timings, so they
/// measure the *selectors*, not the *transcript engine*.
///
/// This harness transcribes the same real-speech clips through both engines
/// (openai-whisper and whisper.cpp, same tiny model), normalizes both into the
/// Transcript contract, and measures:
///   - WER        — word-level text parity (normalized, punctuation stripped)
///   - boundary Δ — segment start/end deltas for time-overlapping segments
///                  (captions and the pause-boundary heuristic live on these)
///   - selector parity — the heuristic highlight selector run on both transcripts,
///     scored by time-IoU (what the user actually sees)
///
/// Verdict gates for the E2 adapter swap:
///   G1  WER ≤ 0.15 on every clip
///   G2  mean segment Δstart ≤ 0.35s, max ≤ 1.0s
///   G3  selector parity: same highlight count and every cpp pick matched by an
///       openai-whisper pick with IoU ≥ 0.7
///
/// Inputs (the harness skips with instructions when anything is missing):
///   PARITY_CLIP           — a real-speech video (default: the desktop-E2E dialogue fixture)
///   PARITY_SLICES         — how many slices to cut it into (default 3)
///   WHISPER_COMMAND       — the openai-whisper CLI (default .venv/bin/whisper)
///   LF_WHISPERCPP_BIN     — whisper.cpp CLI (default .tools/whisper.cpp/build/bin/whisper-cli)
///   LF_WHISPERCPP_MODEL   — ggml model        (default .tools/whisper.cpp/ggml-tiny.bin)
///
/// Run: npm run test:transcript-parity
///
/// Layer-1 findings (2026-10-23, dialogue-180s × 3 slices, tiny vs tiny ggml):
///   - Text parity excellent: WER 2.4–4.2% (defaults) / 2.4–8.3% (-bs 1 greedy).
///   - Speed: whisper.cpp 3.5–5.8× faster on CPU (11–23s vs 39–67s per 60s clip).
///   - Segmentation policy differs structurally: ~90% of matched segment starts
///     agree within 0.5s, but sentence splits diverge locally (e.g. one tail
///     sentence split at 51.76s vs 53.72s → Δstart 3.88s on one pair). No flag
///     tried (-bs 1) removes it.
///   - Because chooseHeuristicHighlights is segment-driven (segment gaps, segment
///     word counts, segment text), the pick sets differ on 1 of 3 clips (2 vs 3
///     picks, worst IoU 0.548). Clip-level: 2 of 3 pass G3 with -bs 1.
///   → Verdict for E2 as a direct adapter swap: NO-GO with the current selector.
///     E2 becomes viable only after the selector is made segmentation-robust (its
///     own measured change, affecting current-engine transcripts too), or the
///     product accepts that highlight boundaries shift slightly with the engine.
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { promisify } from "node:util";
import { transcribeWithWhisper } from "../../adapters/local/whisper.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";
import { chooseHeuristicHighlights } from "../../services/ai-pipeline/heuristic.ts";

const exec = promisify(execFile);
const root = resolve(import.meta.dirname, "../..");
const ffmpeg = process.env.FFMPEG_PATH ?? "ffmpeg";

const clip = process.env.PARITY_CLIP
  ? resolve(process.env.PARITY_CLIP)
  : resolve(root, "artifacts/desktop-e2e-run-20261019-followup/dialogue-180s.mp4");
const slices = Number(process.env.PARITY_SLICES ?? 3);
const whisperBin = process.env.LF_WHISPERCPP_BIN ?? resolve(root, ".tools/whisper.cpp/build/bin/whisper-cli");
const whisperModel = process.env.LF_WHISPERCPP_MODEL ?? resolve(root, ".tools/whisper.cpp/ggml-tiny.bin");
// Extra whisper.cpp flags, for segmentation-policy iterations during the parity
// decision (e.g. LF_WHISPERCPP_ARGS="-bs 1 -ml 200"). Reported in the output.
const cppExtraArgs = (process.env.LF_WHISPERCPP_ARGS ?? "").split(" ").filter(Boolean);
const openaiWhisper = process.env.WHISPER_COMMAND ?? resolve(root, ".venv/bin/whisper");

const missing = [
  [clip, "PARITY_CLIP (a real-speech video)"],
  [whisperBin, "whisper.cpp CLI (LF_WHISPERCPP_BIN)"],
  [whisperModel, "whisper.cpp ggml model (LF_WHISPERCPP_MODEL)"],
  [openaiWhisper, "openai-whisper (WHISPER_COMMAND)"],
].filter(([path]) => !existsSync(path as string));
if (missing.length > 0) {
  console.log(`transcript-parity: SKIPPED — missing ${missing.map(([, what]) => what).join(", ")}`);
  process.exit(0);
}

// --- whisper.cpp JSON → Transcript contract -------------------------------------------
type WhisperCppToken = { text?: string; offsets?: { from?: number; to?: number } };
type WhisperCppSegment = { offsets?: { from?: number; to?: number }; text?: string; tokens?: WhisperCppToken[] };
type WhisperCppJson = { transcription?: WhisperCppSegment[]; result?: { language?: string } };

/** Groups whisper.cpp tokens into words: a token beginning with a space (or the
 *  first token of a segment) starts a new word; timestamp/special markers are
 *  skipped. Mirrors what the openai-whisper adapter receives for word timings. */
function wordsFromTokens(tokens: WhisperCppToken[]): Array<{ text: string; start: number; end: number }> {
  const words: Array<{ text: string; start: number; end: number }> = [];
  for (const token of tokens) {
    const raw = token.text ?? "";
    if (!raw || /^_\[|^\[_|^__/.test(raw)) continue; // _[_BEG_]_, __TT__ markers
    const text = raw.trim();
    if (!text) continue;
    const start = (token.offsets?.from ?? 0) / 1000;
    const end = (token.offsets?.to ?? 0) / 1000;
    const startsWord = raw.startsWith(" ") || words.length === 0;
    if (startsWord || words.length === 0) {
      words.push({ text, start, end });
    } else {
      const current = words[words.length - 1];
      current.text += text;
      current.end = Math.max(current.end, end);
    }
  }
  return words;
}

function transcriptFromWhisperCpp(json: WhisperCppJson, clipId: string, duration: number): Transcript {
  const segments = json.transcription ?? [];
  const clamp = (value: number) => Math.max(0, Math.min(value, duration));
  const words = segments.flatMap((segment, si) =>
    wordsFromTokens(segment.tokens ?? []).map((word, wi) => ({
      id: `w-${si}-${wi}`,
      text: word.text,
      start: clamp(word.start),
      end: clamp(word.end),
    })),
  );
  // Zero-duration words drop out of the contract list; segments must not keep
  // referencing them (same treatment as the openai-whisper adapter's keptIds).
  const keptWords = words.filter((w) => w.end > w.start);
  const keptIds = new Set(keptWords.map((w) => w.id));
  const transcript: Transcript = {
    version: 1,
    sourceArtifactId: clipId,
    language: json.result?.language ?? "en",
    duration,
    provider: "whisper-cpp:tiny",
    words: keptWords,
    segments: segments
      .map((segment, index) => ({
        id: `s-${index}`,
        text: (segment.text ?? "").trim(),
        start: clamp((segment.offsets?.from ?? 0) / 1000),
        end: clamp((segment.offsets?.to ?? 0) / 1000),
        wordIds: words
          .filter((w) => w.start >= clamp((segment.offsets?.from ?? 0) / 1000) && w.end <= clamp((segment.offsets?.to ?? 0) / 1000))
          .map((w) => w.id)
          .filter((id) => keptIds.has(id)),
      }))
      .filter((segment) => segment.end > segment.start),
  };
  validateTranscript(transcript);
  return transcript;
}

// --- Metrics ---------------------------------------------------------------------------
const normalizeWord = (word: string) => word.toLowerCase().replace(/[^\p{L}\p{N}']/gu, "");

function wordLists(transcript: Transcript): string[] {
  return transcript.words.map((w) => normalizeWord(w.text)).filter(Boolean);
}

function levenshtein(a: string[], b: string[]): number {
  const previous = new Array(b.length + 1).fill(0).map((_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    let carry = previous[0];
    previous[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const memo = previous[j];
      previous[j] = Math.min(previous[j] + 1, previous[j - 1] + 1, carry + (a[i - 1] === b[j - 1] ? 0 : 1));
      carry = memo;
    }
  }
  return previous[b.length];
}

const wer = (reference: string[], candidate: string[]) =>
  reference.length === 0 ? 0 : levenshtein(reference, candidate) / reference.length;

const iou = (a: { start: number; end: number }, b: { start: number; end: number }) => {
  const intersection = Math.max(0, Math.min(a.end, b.end) - Math.max(a.start, b.start));
  const union = Math.max(a.end, b.end) - Math.min(a.start, b.start);
  return union > 0 ? intersection / union : 0;
};

/** Greedy time-overlap matching: every candidate segment pairs with the reference
 *  segment it overlaps most, when the overlap covers at least half of both. */
function boundaryDeltas(reference: Transcript, candidate: Transcript): { meanStart: number; maxStart: number; meanEnd: number; maxEnd: number; matched: number } {
  const deltas: Array<{ start: number; end: number }> = [];
  for (const candidateSegment of candidate.segments) {
    let best: { overlap: number; segment: Transcript["segments"][number] } | undefined;
    for (const referenceSegment of reference.segments) {
      const overlap = Math.min(candidateSegment.end, referenceSegment.end) - Math.max(candidateSegment.start, referenceSegment.start);
      if (overlap <= 0) continue;
      const smaller = Math.min(candidateSegment.end - candidateSegment.start, referenceSegment.end - referenceSegment.start);
      if (overlap < 0.5 * smaller) continue;
      if (!best || overlap > best.overlap) best = { overlap, segment: referenceSegment };
    }
    if (best) deltas.push({ start: Math.abs(candidateSegment.start - best.segment.start), end: Math.abs(candidateSegment.end - best.segment.end) });
  }
  const mean = (pick: (d: { start: number; end: number }) => number) =>
    deltas.length === 0 ? Number.POSITIVE_INFINITY : deltas.reduce((sum, d) => sum + pick(d), 0) / deltas.length;
  const max = (pick: (d: { start: number; end: number }) => number) =>
    deltas.length === 0 ? Number.POSITIVE_INFINITY : Math.max(...deltas.map(pick));
  return { meanStart: mean((d) => d.start), maxStart: max((d) => d.start), meanEnd: mean((d) => d.end), maxEnd: max((d) => d.end), matched: deltas.length };
}

/** Selector parity: every whisper.cpp pick must be matched by an openai-whisper pick
 *  with IoU ≥ 0.7 — the user-visible "did the product pick the same moments". */
function selectorParity(reference: Transcript, candidate: Transcript): { matched: number; countRef: number; countCand: number; worstIou: number } {
  const referenceHighlights = chooseHeuristicHighlights(reference);
  const candidateHighlights = chooseHeuristicHighlights(candidate);
  const ious = candidateHighlights.map((candidateHighlight) =>
    Math.max(0, ...referenceHighlights.map((referenceHighlight) => iou(candidateHighlight, referenceHighlight))),
  );
  return {
    matched: ious.filter((value) => value >= 0.7).length,
    countRef: referenceHighlights.length,
    countCand: candidateHighlights.length,
    worstIou: ious.length === 0 ? 1 : Math.min(...ious),
  };
}

// --- Run -------------------------------------------------------------------------------
const scratch = await mkdtemp(join(tmpdir(), "transcript-parity-"));
const report: Array<Record<string, unknown>> = [];
try {
  const sourceStat = await stat(clip);
  if (!sourceStat.isFile()) throw new Error(`PARITY_CLIP is not a file: ${clip}`);
  const sliceDuration = 60;
  const results: Array<Record<string, unknown>> = [];
  for (let index = 0; index < slices; index++) {
    const clipId = `parity-${basename(clip, ".mp4")}-${index}`;
    const wav = join(scratch, `${clipId}.wav`);
    const start = index * sliceDuration;
    await exec(ffmpeg, ["-y", "-v", "error", "-ss", String(start), "-t", String(sliceDuration), "-i", clip, "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", wav]);

    const startedOpenai = process.hrtime.bigint();
    const openaiTranscript = await transcribeWithWhisper(wav, clipId, sliceDuration, { model: "tiny", command: openaiWhisper, fp16: false });
    const openaiMs = Number(process.hrtime.bigint() - startedOpenai) / 1e6;

    const jsonPrefix = join(scratch, `${clipId}-cpp`);
    const startedCpp = process.hrtime.bigint();
    // -ojf (output JSON full) is required: plain -oj omits the per-segment tokens
    // the word-timing comparison depends on.
    await exec(whisperBin, ["-m", whisperModel, ...cppExtraArgs, "-f", wav, "-ojf", "-of", jsonPrefix], { maxBuffer: 32 * 1024 * 1024 });
    const cppMs = Number(process.hrtime.bigint() - startedCpp) / 1e6;
    const cppJson = JSON.parse(await readFile(`${jsonPrefix}.json`, "utf8")) as WhisperCppJson;
    const cppTranscript = transcriptFromWhisperCpp(cppJson, clipId, sliceDuration);

    const referenceWords = wordLists(openaiTranscript);
    const candidateWords = wordLists(cppTranscript);
    const textWer = wer(referenceWords, candidateWords);
    const boundaries = boundaryDeltas(openaiTranscript, cppTranscript);
    const selectors = selectorParity(openaiTranscript, cppTranscript);

    results.push({
      clip: clipId,
      words: { openai: referenceWords.length, cpp: candidateWords.length },
      wer: Number(textWer.toFixed(4)),
      boundaryDeltas: {
        matchedSegments: boundaries.matched,
        meanStartSeconds: Number(boundaries.meanStart.toFixed(3)),
        maxStartSeconds: Number(boundaries.maxStart.toFixed(3)),
        meanEndSeconds: Number(boundaries.meanEnd.toFixed(3)),
        maxEndSeconds: Number(boundaries.maxEnd.toFixed(3)),
      },
      selectors: {
        openaiPicks: selectors.countRef,
        cppPicks: selectors.countCand,
        matched: selectors.matched,
        worstIou: Number(selectors.worstIou.toFixed(3)),
      },
      wallMs: { openai: Math.round(openaiMs), cpp: Math.round(cppMs) },
      openaiSample: openaiTranscript.words.slice(0, 5).map((w) => w.text),
      cppSample: cppTranscript.words.slice(0, 5).map((w) => w.text),
    });

  }

  const gates = [
    { name: "G1 WER ≤ 0.15 on every clip", pass: results.every((r) => (r["wer"] as number) <= 0.15) },
    { name: "G2 mean Δstart ≤ 0.35s and max Δstart ≤ 1.0s on every clip", pass: results.every((r) => (r["boundaryDeltas"] as Record<string, number>).meanStartSeconds <= 0.35 && (r["boundaryDeltas"] as Record<string, number>).maxStartSeconds <= 1.0) },
    { name: "G3 selector parity: same pick count and worst IoU ≥ 0.7 on every clip", pass: results.every((r) => {
      const selectors = r["selectors"] as Record<string, number>;
      return selectors.openaiPicks === selectors.cppPicks && selectors.worstIou >= 0.7;
    }) },
  ];
  const verdict = gates.every((gate) => gate.pass) ? "GO" : "NO-GO";
  report.push({ benchmark: "transcript-engine-parity", engineModel: "tiny vs tiny (ggml)", cppFlags: cppExtraArgs.join(" ") || "(defaults)", clips: results.length, results, gates, verdict });
  console.log(JSON.stringify(report[0], null, 2));
  if (verdict === "NO-GO") process.exitCode = 1;
} finally {
  await rm(scratch, { recursive: true, force: true });
}
