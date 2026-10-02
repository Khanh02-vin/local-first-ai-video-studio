import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import type { Highlight } from "../../packages/contracts/highlight.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";
import { chooseHeuristicHighlights } from "../../services/ai-pipeline/heuristic.ts";
import { chooseContentHighlights } from "../../services/ai-pipeline/content-highlights.ts";
import { generateSemanticHighlights, createMockSemanticProvider, SemanticProviderAdapter } from "../../services/ai-pipeline/semantic.ts";
import { GeminiHighlightProvider, LlamaCppHighlightProvider, highlightPrompt, mapWordRangeProposals } from "../../services/ai-pipeline/providers.ts";

/**
 * Highlight-selection eval over curated fixtures (time-IoU + rank@k).
 *
 * Offline (CI): heuristic vs content vs two oracle semantic mocks. The exact
 * oracle validates the metric code (must score ~1.0); the jittered oracle
 * validates near-miss handling (IoU < 1 but hit@k = 1).
 *
 * Live rows (optional, auto-skipped when unavailable):
 *   GEMINI_API_KEY=...   → semantic-gemini row
 *   llama-server on LOCAL_LLM_BASE_URL (default 127.0.0.1:8080, probed via /health)
 *                        → semantic-local row (the bundled Qwen)
 *
 * That live comparison is the point of the harness: it answers whether the
 * bundled local model actually beats the heuristic on these transcripts.
 *
 * Run:  npm run test:eval-highlights
 */

type Expected = { start: number; end: number; title: string };
type Case = { id: string; note: string; segments: Array<{ start: number; end: number; text: string }>; expected: Expected[] };

const here = dirname(fileURLToPath(import.meta.url));
const dataset = JSON.parse(readFileSync(join(here, "fixtures", "highlight-eval.json"), "utf8")) as { iouThreshold: number; cases: Case[] };
const THRESHOLD = dataset.iouThreshold;
const LIMIT = 5;

// --- fixture → contract transcript ------------------------------------------
// Words are spread evenly inside each segment; deterministic, contract-valid.
function buildTranscript(item: Case): Transcript {
  const words: Transcript["words"] = [];
  const segments: Transcript["segments"] = [];
  let duration = 0;
  item.segments.forEach((segment, segIndex) => {
    const tokens = segment.text.trim().split(/\s+/).filter(Boolean);
    const step = (segment.end - segment.start) / tokens.length;
    const wordIds: string[] = [];
    tokens.forEach((token, wordIndex) => {
      const id = `w-${segIndex}-${wordIndex}`;
      words.push({ id, text: token, start: segment.start + wordIndex * step, end: segment.start + (wordIndex + 1) * step });
      wordIds.push(id);
    });
    segments.push({ id: `s-${segIndex}`, text: segment.text, start: segment.start, end: segment.end, wordIds });
    duration = Math.max(duration, segment.end);
  });
  const transcript: Transcript = { version: 1, sourceArtifactId: item.id, language: "en", duration, provider: "eval-fixture", words, segments };
  // Contract violations are fixture bugs — fail loudly.
  for (const word of transcript.words) { assert.ok(word.end > word.start, `${item.id}: bad word range`); }
  assert.ok(transcript.words.every((w, i) => i === 0 || w.start >= transcript.words[i - 1].end), `${item.id}: words not ordered`);
  return transcript;
}

// --- metrics ----------------------------------------------------------------
function timeIou(a: { start: number; end: number }, b: { start: number; end: number }): number {
  const intersection = Math.max(0, Math.min(a.end, b.end) - Math.max(a.start, b.start));
  const union = (a.end - a.start) + (b.end - b.start) - intersection;
  return union > 0 ? intersection / union : 0;
}

type CaseResult = { meanIou: number; hits: Record<number, boolean> };

function scoreCase(expected: Expected[], predictions: Highlight[], ks: number[]): CaseResult {
  const ranked = [...predictions].sort((a, b) => b.score - a.score || a.start - b.start);
  let sum = 0;
  const hits: Record<number, boolean> = {};
  for (const target of expected) {
    sum += Math.max(0, ...ranked.map((p) => timeIou(target, p)));
    for (const k of ks) hits[k] = (hits[k] ?? false) || ranked.slice(0, k).some((p) => timeIou(target, p) >= THRESHOLD);
  }
  return { meanIou: expected.length ? sum / expected.length : 0, hits };
}

function aggregate(results: CaseResult[]): { meanIou: number; hitAt: Record<number, number> } {
  const ks = Object.keys(results[0]?.hits ?? { 1: false }).map(Number);
  const total = results.length;
  return {
    meanIou: results.reduce((sum, r) => sum + r.meanIou, 0) / total,
    hitAt: Object.fromEntries(ks.map((k) => [k, results.filter((r) => r.hits[k]).length / total])),
  };
}

// --- providers --------------------------------------------------------------
type Row = { name: string; run: (transcript: Transcript) => Promise<Highlight[]> | Highlight[]; live?: boolean };

function oracleProposals(item: Case, transcript: Transcript, jitter: boolean): Array<{ start: number; end: number; wordIds: string[]; title: string; score: number }> {
  return item.expected.map((expected, index) => {
    // Deterministic ±2s boundary jitter so the jittered oracle is reproducible.
    const d1 = jitter ? (((index + item.id.length) % 3) - 1) * 2 : 0;
    const d2 = jitter ? ((((index + 1) * 2 + item.id.length) % 3) - 1) * 2 : 0;
    const start = Math.max(0, expected.start + d1);
    const end = Math.min(transcript.duration, Math.max(start + 4, expected.end + d2));
    return {
      start, end,
      wordIds: transcript.words.filter((w) => w.start >= start && w.end <= end).map((w) => w.id),
      title: expected.title,
      score: 90 - index * 5,
    };
  });
}

async function main(): Promise<void> {
  const cases = dataset.cases;
  const transcripts = cases.map((item) => ({ item, transcript: buildTranscript(item) }));
  const expectedCount = cases.reduce((sum, item) => sum + item.expected.length, 0);
  const ks = [1, 3, 5];

  const rows: Row[] = [
    { name: "heuristic", run: (t) => chooseHeuristicHighlights(t, LIMIT) },
    { name: "content", run: (t) => chooseContentHighlights(t, LIMIT) },
    {
      name: "semantic-oracle",
      run: (t) => {
        const item = cases.find((c) => c.id === t.sourceArtifactId)!;
        return generateSemanticHighlights(t, createMockSemanticProvider(oracleProposals(item, t, false)), { limit: LIMIT, minDuration: 10, maxDuration: 90 });
      },
    },
    {
      name: "semantic-oracle-jit",
      run: (t) => {
        const item = cases.find((c) => c.id === t.sourceArtifactId)!;
        return generateSemanticHighlights(t, createMockSemanticProvider(oracleProposals(item, t, true)), { limit: LIMIT, minDuration: 10, maxDuration: 90 });
      },
    },
  ];

  // Live rows — only when the backend is actually reachable.
  const geminiKey = process.env.GEMINI_API_KEY;
  if (geminiKey) {
    rows.push({ name: "semantic-gemini", live: true, run: (t) => generateSemanticHighlights(t, new SemanticProviderAdapter(new GeminiHighlightProvider(geminiKey), "gemini"), { limit: LIMIT, minDuration: 10, maxDuration: 90 }) });
  }
  const llamaBase = (process.env.LOCAL_LLM_BASE_URL ?? "http://127.0.0.1:8080").replace(/\/+$/, "");
  let llamaUp = false;
  // Probe /v1/models, not /health — unrelated local services can answer /health.
  try { llamaUp = (await fetch(`${llamaBase}/v1/models`, { signal: AbortSignal.timeout(1500) })).ok; } catch { llamaUp = false; }
  if (llamaUp) {
    rows.push({ name: "semantic-local", live: true, run: (t) => generateSemanticHighlights(t, new SemanticProviderAdapter(new LlamaCppHighlightProvider(llamaBase), "llama-cpp"), { limit: LIMIT, minDuration: 10, maxDuration: 90 }) });
  }

  console.log(`== highlight eval: ${cases.length} cases, ${expectedCount} expected, IoU threshold ${THRESHOLD} ==`);
  if (!geminiKey) console.log("   (semantic-gemini skipped: GEMINI_API_KEY not set)");
  if (!llamaUp) console.log(`   (semantic-local skipped: no llama-server at ${llamaBase}/v1/models)`);

  const aggregates = new Map<string, { meanIou: number; hitAt: Record<number, number>; failures: number; worst: { id: string; iou: number } }>();

  for (const row of rows) {
    const results: CaseResult[] = [];
    let failures = 0;
    let worst = { id: "", iou: Infinity };
    let aborted = false;
    for (const { item, transcript } of transcripts) {
      if (aborted) { failures += 1; continue; }
      try {
        const pending = row.run(transcript);
        const predictions = typeof (pending as Promise<Highlight[]>).then === "function"
          ? await Promise.race([pending, new Promise<Highlight[]>((_, reject) => setTimeout(() => reject(new Error("EVAL_CASE_TIMEOUT")), 240_000).unref())])
          : (pending as Highlight[]);
        for (const p of predictions) validateHighlights([p], transcript.duration);
        const result = scoreCase(item.expected, predictions, ks);
        results.push(result);
        if (result.meanIou < worst.iou) worst = { id: item.id, iou: result.meanIou };
      } catch (error) {
        failures += 1;
        if (row.live) { console.log(`   ${row.name}: aborting remaining cases (${String(error).slice(0, 80)})`); aborted = true; }
        else throw error; // offline rows must never fail a case
      }
    }
    if (!results.length) { console.log(`${row.name.padEnd(20)} all cases failed`); continue; }
    const agg = aggregate(results);
    aggregates.set(row.name, { ...agg, failures, worst });
    const hits = ks.map((k) => `${((agg.hitAt[k] ?? 0) * 100).toFixed(0)}%`.padStart(5)).join("  ");
    console.log(`${row.name.padEnd(20)} IoU ${agg.meanIou.toFixed(2)}   hit@1/3/5:${hits}   worst case ${worst.id} (${worst.iou.toFixed(2)})${failures ? `   failures ${failures}` : ""}`);
  }

  // Pause-boundary signal: fixtures are contiguous, so verify directly that a
  // ≥0.7s internal gap (Whisper word-timing derived) lifts a segment's score.
  // A gap marks the boundary BETWEEN two segments, so both sides earn the bonus;
  // compare against segments in a contiguous run (no bonus) and a sub-threshold gap.
  const mkPauseCase = (gapAfterC: number): Transcript => ({
    version: 1, sourceArtifactId: "gap-probe", language: "en", duration: 40, provider: "test",
    words: [],
    segments: [
      { id: "a", text: "We talked about the roadmap for a while.", start: 0, end: 6, wordIds: [] },
      { id: "b", text: "We talked about the roadmap for a while.", start: 6, end: 12, wordIds: [] },   // fully contiguous
      { id: "c", text: "We talked about the roadmap for a while.", start: 12, end: 18, wordIds: [] },  // gap after → pause
      { id: "d", text: "We talked about the roadmap for a while.", start: 18 + gapAfterC, end: 24 + gapAfterC, wordIds: [] },
    ],
  });
  const scoreOf = (t: Transcript, start: number) => chooseHeuristicHighlights(t, 5).find((h) => h.start === start)!.score;
  const gapped = mkPauseCase(3);      // 3s pause between c and d
  assert.ok(scoreOf(gapped, 12) > scoreOf(gapped, 6), `pause boundary must lift score (c adjacent to 3s gap vs contiguous b)`);
  assert.equal(scoreOf(gapped, 6), scoreOf(gapped, 0), "contiguous segments must not earn the pause bonus");
  const tight = mkPauseCase(0.2);     // 0.2s gap — below threshold
  assert.equal(scoreOf(tight, 12), scoreOf(tight, 6), "sub-threshold gap must not lift score");

  // --- CI assertions (offline rows only) ------------------------------------
  // Prompt shape: compact word lines, never full word-object JSON — this is
  // what keeps a 180s window inside the production llama-server context.
  const sample = transcripts[0].transcript;
  const prompt = highlightPrompt(sample);
  assert.ok(prompt.includes("0 ["), "prompt must use compact wordIndex [start-end] text lines");
  assert.ok(!prompt.includes('"words"'), "prompt must not embed full word JSON");
  assert.ok(prompt.length < JSON.stringify(sample).length, "compact prompt must be smaller than raw transcript JSON");
  // Mapping: word-index proposals re-derive times/wordIds; junk indices are dropped.
  const mapped = mapWordRangeProposals(sample, [{ from: 0, to: 2, title: "ok", score: 90 }, { from: 9999, to: 10000, title: "junk", score: 50 }, { from: "x", to: 2, title: "junk2", score: 50 }]);
  assert.equal(mapped.length, 1, "only in-range integer proposals survive");
  assert.equal(mapped[0].start, sample.words[0].start);
  assert.equal(mapped[0].end, sample.words[2].end);
  assert.deepEqual(mapped[0].wordIds, sample.words.slice(0, 3).map((w) => w.id));
  assert.equal(mapped[0].score, 90);
  // Probability-style scores rescale to the 0..100 contract.
  assert.equal(mapWordRangeProposals(sample, [{ from: 0, to: 1, title: "p", score: 0.9 }])[0].score, 90);

  const oracle = aggregates.get("semantic-oracle")!;
  const jittered = aggregates.get("semantic-oracle-jit")!;
  const heuristic = aggregates.get("heuristic")!;
  const content = aggregates.get("content")!;

  // Metric sanity: exact oracle must be near-perfect; jittered oracle must
  // rank every expected inside top-3 while scoring clearly below 1.0.
  assert.ok(oracle.meanIou >= 0.99, `oracle IoU ${oracle.meanIou} — metric or fixture broken`);
  assert.equal(oracle.hitAt[3], 1, "oracle hit@3 must be 100%");
  assert.ok(jittered.meanIou > 0.4 && jittered.meanIou < 0.95, `jittered oracle IoU ${jittered.meanIou} outside plausible band`);
  assert.equal(jittered.hitAt[3], 1, "jittered oracle hit@3 must be 100%");

  // Baselines must be non-degenerate on this dataset.
  assert.ok(heuristic.meanIou > 0.2, `heuristic IoU ${heuristic.meanIou} too low — fixtures or heuristic regressed`);
  assert.ok(content.meanIou > 0.2, `content IoU ${content.meanIou} too low — fixtures or content regressed`);

  console.log("eval highlight tests: ok");
}

await main();
