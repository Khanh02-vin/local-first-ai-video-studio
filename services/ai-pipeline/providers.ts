import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

export interface TranscriptionProvider { transcribe(input: { audio: Uint8Array; sourceArtifactId: string; duration: number; signal?: AbortSignal }): Promise<Transcript>; }
/** choose() may receive window bounds (seconds) so callers control how a long transcript
 *  is sliced per LLM call; omit them for the shared defaults. */
export interface HighlightProvider { choose(input: { transcript: Transcript; signal?: AbortSignal; windowSeconds?: number; overlapSeconds?: number }): Promise<Highlight[]>; }

export class OpenAITranscriptionProvider implements TranscriptionProvider {
  private readonly apiKey: string;
  private readonly endpoint: string;
  private readonly model: string;
  constructor(apiKey: string, endpoint = "https://api.openai.com/v1/audio/transcriptions", model = "whisper-1") { this.apiKey = apiKey; this.endpoint = endpoint; this.model = model; }
  async transcribe(input: { audio: Uint8Array; sourceArtifactId: string; duration: number; signal?: AbortSignal }): Promise<Transcript> {
    const form = new FormData(); form.append("file", new Blob([input.audio], { type: "audio/wav" }), "audio.wav"); form.append("model", this.model); form.append("response_format", "verbose_json");
    const response = await fetch(this.endpoint, { method: "POST", headers: { Authorization: `Bearer ${this.apiKey}` }, body: form, signal: input.signal });
    if (!response.ok) throw new Error(`TRANSCRIPTION_PROVIDER_${response.status}`);
    const raw = await response.json() as { language?: string; duration?: number; segments?: Array<{ id?: number; start: number; end: number; text: string; words?: Array<{ word: string; start: number; end: number }> }> };
    const words = (raw.segments ?? []).flatMap((segment, segmentIndex) => (segment.words ?? []).map((word, wordIndex) => ({ id: `w-${segmentIndex}-${wordIndex}`, text: word.word, start: word.start, end: word.end })));
    const transcript: Transcript = { version: 1, sourceArtifactId: input.sourceArtifactId, language: raw.language ?? "und", duration: raw.duration ?? input.duration, provider: this.model, words, segments: (raw.segments ?? []).map((segment, index) => ({ id: `s-${segment.id ?? index}`, text: segment.text, start: segment.start, end: segment.end, wordIds: words.filter((word) => word.start >= segment.start && word.end <= segment.end).map((word) => word.id) })) };
    validateTranscript(transcript); return transcript;
  }
}

/** Shared output shape for both LLM providers: a wrapper object around the proposal
 *  array so llama.cpp's json_object constrained decoding (object root required) and
 *  Gemini's responseSchema can enforce the same structure. Proposals reference
 *  inclusive word indices (from/to), never word ids or timestamps — the server
 *  re-derives both, keeping prompts and completions small. */
export const HIGHLIGHT_OUTPUT_SCHEMA = {
  type: "object",
  properties: {
    highlights: {
      type: "array",
      items: {
        type: "object",
        properties: {
          from: { type: "integer" },
          to: { type: "integer" },
          title: { type: "string" },
          score: { type: "number" },
          reason: { type: "string" },
        },
        required: ["from", "to", "title", "score"],
      },
    },
  },
  required: ["highlights"],
} as const;

/** Gemini's responseSchema dialect uses uppercase OpenAPI-style type names. */
const GEMINI_HIGHLIGHT_SCHEMA = {
  type: "OBJECT",
  properties: {
    highlights: {
      type: "ARRAY",
      items: {
        type: "OBJECT",
        properties: {
          from: { type: "INTEGER" },
          to: { type: "INTEGER" },
          title: { type: "STRING" },
          score: { type: "NUMBER" },
          reason: { type: "STRING" },
        },
        required: ["from", "to", "title", "score"],
      },
    },
  },
  required: ["highlights"],
} as const;

/** Unwrap the {"highlights":[...]} envelope (accepting a bare array for robustness). */
export function extractHighlightProposals(value: unknown): Array<Record<string, unknown>> {
  if (Array.isArray(value)) return value as Array<Record<string, unknown>>;
  if (value && typeof value === "object" && Array.isArray((value as { highlights?: unknown }).highlights)) {
    return (value as { highlights: Array<Record<string, unknown>> }).highlights;
  }
  throw new Error("LLM_HIGHLIGHT_SHAPE");
}

/** Compact transcript rendering: one "wordIndex [start-end] text" line per word.
 *  A full word-object JSON prompt costs ~4k tokens for a 96s clip; these lines
 *  cost ~6x less, which is what lets small local models (and the production
 *  llama-server context) handle a 180s window at all. */
export function highlightPrompt(transcript: Transcript): string {
  const lines = transcript.words.map((word, index) => `${index} [${word.start.toFixed(1)}-${word.end.toFixed(1)}] ${word.text}`);
  return `Choose up to 5 complete short-video highlights from this transcript. Lines are "wordIndex [starts-end] text". Reply with JSON {"highlights":[{"from":int,"to":int,"title":string,"score":0-100,"reason":string}]} where from/to are inclusive word indices spanning each highlight. Pick self-contained, interesting moments — not greetings or filler. Transcript:\n${lines.join("\n")}`;
}

/** Maps word-index proposals back to contract highlights: times and wordIds are
 *  re-derived from the transcript, out-of-range indices are dropped, and scores
 *  that arrive as probabilities (0..1) are rescaled to 0..100. */
export function mapWordRangeProposals(transcript: Transcript, parsed: Array<Record<string, unknown>>): Highlight[] {
  const out: Highlight[] = [];
  for (const item of parsed) {
    const from = Math.trunc(Number(item.from));
    const to = Math.trunc(Number(item.to));
    if (!Number.isFinite(from) || !Number.isFinite(to) || from < 0 || to < from || to >= transcript.words.length) continue;
    const slice = transcript.words.slice(from, to + 1);
    const rawScore = Number(item.score);
    const score = Number.isFinite(rawScore) ? (rawScore <= 1 ? Math.round(rawScore * 100) : Math.round(rawScore)) : 0;
    out.push({
      version: 1, id: `h-${out.length}`, sourceArtifactId: transcript.sourceArtifactId,
      start: slice[0].start, end: slice.at(-1)!.end, wordIds: slice.map((word) => word.id),
      title: (typeof item.title === "string" ? item.title : "").trim().slice(0, 120) || "Highlight",
      score: Math.max(0, Math.min(100, score)),
      reason: typeof item.reason === "string" ? item.reason.trim().slice(0, 500) : undefined,
    });
  }
  return out;
}

export class GeminiHighlightProvider implements HighlightProvider {
  private readonly apiKey: string;
  private readonly model: string;
  private readonly endpoint: string;
  constructor(apiKey: string, model = "gemini-2.0-flash", endpoint = "https://generativelanguage.googleapis.com/v1beta/models") { this.apiKey = apiKey; this.model = model; this.endpoint = endpoint; }
  async choose(input: { transcript: Transcript; signal?: AbortSignal; windowSeconds?: number; overlapSeconds?: number }): Promise<Highlight[]> {
    // Long videos exceed a single comfortable LLM context, so split the transcript into
    // windows, get per-window proposals, then merge and de-overlap into the final 3-5.
    const windows = highlightWindows(input.transcript, input.windowSeconds ?? 180, input.overlapSeconds ?? 30);
    const perWindow: Highlight[][] = [];
    for (let i = 0; i < windows.length; i++) {
      const { from, window } = windows[i];
      const local = await this.chooseTranscript(window, input.signal);
      const shifted = local.map((item, index) => ({ ...item, id: `h-${i}-${index}`, start: from + item.start, end: from + item.end }));
      perWindow.push(shifted);
    }
    const highlights = mergeWindowHighlights(perWindow, input.transcript);
    validateHighlights(highlights, input.transcript.duration); return highlights;
  }
  async chooseTranscript(transcript: Transcript, signal?: AbortSignal): Promise<Highlight[]> {
    const prompt = highlightPrompt(transcript);
    // Structured output: the API guarantees a valid JSON body matching the schema,
    // so no fence-stripping or truncation repair is needed on this path.
    const response = await fetch(`${this.endpoint}/${this.model}:generateContent?key=${encodeURIComponent(this.apiKey)}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ contents: [{ parts: [{ text: prompt }] }], generationConfig: { responseMimeType: "application/json", responseSchema: GEMINI_HIGHLIGHT_SCHEMA } }), signal });
    if (!response.ok) throw new Error(`GEMINI_PROVIDER_${response.status}`);
    const raw = await response.json() as { candidates?: Array<{ content?: { parts?: Array<{ text?: string }> } }> };
    const text = raw.candidates?.[0]?.content?.parts?.map((part) => part.text ?? "").join("") ?? "";
    return mapWordRangeProposals(transcript, extractHighlightProposals(JSON.parse(text.replace(/^```json\s*|\s*```$/g, ""))));
  }
}

/** Slice a transcript into consecutive windows (with overlap) so a long recording never
 *  exceeds a single LLM call. Windows are offset in seconds; each carries the original
 *  transcript-relative time so proposals can be shifted back to absolute timestamps.
 */
export function highlightWindows(transcript: Transcript, windowSeconds = 180, overlapSeconds = 30): Array<{ from: number; to: number; window: Transcript }> {
  const duration = transcript.duration;
  const windows: Array<{ from: number; to: number; window: Transcript }> = [];
  const step = Math.max(1, windowSeconds - overlapSeconds);
  for (let from = 0; from < duration; from += step) {
    const to = Math.min(duration, from + windowSeconds);
    // LLM highlight selection only needs the words (the text + timings). Segments are
    // dropped when slicing a window: a segment that straddles the window boundary cannot be
    // shifted reliably, and a truncated segment would break word-order validation.
    const inWindow = transcript.words.filter((word) => word.end > from && word.start < to);
    const window: Transcript = { version: transcript.version, sourceArtifactId: transcript.sourceArtifactId, language: transcript.language, duration: to - from, words: inWindow.map((word) => ({ ...word, start: word.start - from, end: word.end - from })), segments: [], provider: transcript.provider };
    validateTranscript(window);
    windows.push({ from, to, window });
    if (to === duration) break;
  }
  return windows.length ? windows : [{ from: 0, to: duration, window: transcript }];
}

/** Merge per-window proposals: clamp to the source, sort by score, drop overlaps, cap at 5. */
export function mergeWindowHighlights(perWindow: Highlight[][], source: Transcript): Highlight[] {
  const all = perWindow.flat();
  const ranked = all
    .filter((item) => item.start >= 0 && item.end <= source.duration && item.end > item.start)
    .sort((a, b) => b.score - a.score || a.start - b.start)
    .slice(0, 5);
  const chosen: Highlight[] = [];
  for (const candidate of ranked) {
    if (chosen.some((other) => candidate.start < other.end && other.start < candidate.end)) continue;
    chosen.push(candidate);
    if (chosen.length >= 5) break;
  }
  return chosen.sort((a, b) => a.start - b.start);
}

/**
 * Local LLM via llama.cpp's llama-server (OpenAI-compatible /v1/chat/completions).
 * No API key, no network egress — the model runs on this machine. Uses the same
 * window + merge strategy as the cloud Gemini provider, so long videos stay bounded.
 */
export class LlamaCppHighlightProvider implements HighlightProvider {
  private readonly baseUrl: string;
  private readonly model: string;
  constructor(baseUrl = "http://127.0.0.1:8080", model = "qwen2.5-3b-instruct") { this.baseUrl = baseUrl.replace(/\/+$/, ""); this.model = model; }
  private modelId?: string;
  private async resolvedModelId(signal?: AbortSignal): Promise<string> {
    if (this.modelId) return this.modelId;
    const response = await fetch(`${this.baseUrl}/v1/models`, { signal });
    if (!response.ok) throw new Error(`LLAMA_CPP_MODELS_${response.status}`);
    // OpenAI shape is data[0].id; llama-server also serves models[0].name.
    const raw = await response.json() as { data?: Array<{ id?: string }>; models?: Array<{ name?: string }> };
    const id = raw.data?.[0]?.id ?? raw.models?.[0]?.name;
    if (!id) throw new Error("LLAMA_CPP_NO_MODEL_LOADED");
    this.modelId = id; return id;
  }
  async choose(input: { transcript: Transcript; signal?: AbortSignal; windowSeconds?: number; overlapSeconds?: number }): Promise<Highlight[]> {
    const windows = highlightWindows(input.transcript, input.windowSeconds ?? 180, input.overlapSeconds ?? 30);
    const perWindow: Highlight[][] = [];
    for (let i = 0; i < windows.length; i++) {
      const { from, window } = windows[i];
      const local = await this.chooseTranscript(window, input.signal);
      perWindow.push(local.map((item, index) => ({ ...item, id: `h-${i}-${index}`, start: from + item.start, end: from + item.end })));
    }
    const highlights = mergeWindowHighlights(perWindow, input.transcript);
    validateHighlights(highlights, input.transcript.duration); return highlights;
  }
  async chooseTranscript(transcript: Transcript, signal?: AbortSignal): Promise<Highlight[]> {
    const modelId = await this.resolvedModelId(signal);
    const prompt = highlightPrompt(transcript);
    // Local CPU inference can take minutes per window; give it room without letting a
    // hung server block forever.
    const timeout = AbortSignal.timeout(15 * 60_000);
    const combined = signal ? AbortSignal.any([signal, timeout]) : timeout;
    // Constrained decoding: llama-server enforces the JSON schema, so the reply parses.
    const response = await fetch(`${this.baseUrl}/v1/chat/completions`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: modelId, temperature: 0.2, max_tokens: 800, response_format: { type: "json_object", schema: HIGHLIGHT_OUTPUT_SCHEMA }, messages: [{ role: "user", content: prompt }] }), signal: combined });
    if (!response.ok) {
      const body = await response.text().catch(() => "");
      throw new Error(`LLAMA_CPP_PROVIDER_${response.status}:${body.slice(0, 200)}`);
    }
    const raw = await response.json() as { choices?: Array<{ message?: { content?: string } }> };
    const text = raw.choices?.[0]?.message?.content ?? "";
    // Schema-constrained replies parse directly; the repair path stays as a safety net
    // for older llama-server builds without response_format support.
    let parsed: Array<Record<string, unknown>>;
    const fenced = text.match(/```(?:json)?\s*([\s\S]*?)\s*```/)?.[1] ?? text;
    const start = fenced.search(/[[{]/);
    if (start === -1) throw new Error(`LLAMA_CPP_BAD_JSON:${text.slice(0, 120)}`);
    let candidate = fenced.slice(start);
    try { parsed = extractHighlightProposals(JSON.parse(candidate)); }
    catch {
      const repaired = repairTruncatedJsonArray(candidate.trimEnd());
      try { parsed = extractHighlightProposals(JSON.parse(repaired)); }
      catch { throw new Error(`LLAMA_CPP_BAD_JSON:${text.slice(0, 120)}`); }
    }
    return mapWordRangeProposals(transcript, parsed);
  }
}

/** Best-effort repair of a JSON array that was cut off mid-generation: closes any open
 *  string, drops a dangling `:`/`,`, and appends missing `}`/`]` in correct nesting order. */
export function repairTruncatedJsonArray(input: string): string {
  const withoutFence = input.replace(/```+\s*$/, "");
  const stack: string[] = [];
  let inString = false;
  let escaped = false;
  for (const ch of withoutFence) {
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "[") stack.push("]");
    else if (ch === "{") stack.push("}");
    else if (ch === "]" || ch === "}") stack.pop();
  }
  let out = withoutFence;
  if (inString) out += '"'; // close the truncated string
  // Drop a dangling key (`,"key"` or `,"key":`) but never a completed quoted value
  // (which is preceded by `:` rather than `{`/`,`).
  out = out.replace(/([,{])\s*"[^"]*"\s*:?\s*$/, "$1").replace(/[:,\s]+$/, "");
  if (out === "[" || out === "") return "[]";
  return out + stack.reverse().join("");
}
