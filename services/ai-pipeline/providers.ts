import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

export interface TranscriptionProvider { transcribe(input: { audio: Uint8Array; sourceArtifactId: string; duration: number; signal?: AbortSignal }): Promise<Transcript>; }
export interface HighlightProvider { choose(input: { transcript: Transcript; signal?: AbortSignal }): Promise<Highlight[]>; }

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

type LlmHighlightProposal = { start: number; end: number; wordIds: string[]; title: string; hook?: string; score: number; reason?: string };

export class GeminiHighlightProvider implements HighlightProvider {
  private readonly apiKey: string;
  private readonly model: string;
  private readonly endpoint: string;
  constructor(apiKey: string, model = "gemini-2.0-flash", endpoint = "https://generativelanguage.googleapis.com/v1beta/models") { this.apiKey = apiKey; this.model = model; this.endpoint = endpoint; }
  async choose(input: { transcript: Transcript; signal?: AbortSignal }): Promise<Highlight[]> {
    // Long videos exceed a single comfortable LLM context, so split the transcript into
    // windows, get per-window proposals, then merge and de-overlap into the final 3-5.
    const windows = highlightWindows(input.transcript, 20 * 60, 5 * 60);
    const perWindow: Highlight[][] = [];
    for (let i = 0; i < windows.length; i++) {
      const { from, to, window } = windows[i];
      const local = await this.chooseTranscript(window, input.signal);
      const shifted = local.map((item, index) => ({ ...item, id: `h-${i}-${index}`, start: from + item.start, end: from + item.end }));
      perWindow.push(shifted);
    }
    const highlights = mergeWindowHighlights(perWindow, input.transcript);
    validateHighlights(highlights, input.transcript.duration); return highlights;
  }
  async chooseTranscript(transcript: Transcript, signal?: AbortSignal): Promise<Highlight[]> {
    const prompt = `Return JSON array only. Choose up to 5 complete short-video highlights from this transcript window. Each item has start,end,title,hook,score,reason,wordIds, with times relative to the window start. Use only the supplied word IDs and timestamps. Transcript: ${JSON.stringify(transcript)}`;
    const response = await fetch(`${this.endpoint}/${this.model}:generateContent?key=${encodeURIComponent(this.apiKey)}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ contents: [{ parts: [{ text: prompt }] }] }), signal });
    if (!response.ok) throw new Error(`GEMINI_PROVIDER_${response.status}`);
    const raw = await response.json() as { candidates?: Array<{ content?: { parts?: Array<{ text?: string }> } }> };
    const text = raw.candidates?.[0]?.content?.parts?.map((part) => part.text ?? "").join("") ?? "";
    const parsed = JSON.parse(text.replace(/^```json\s*|\s*```$/g, "")) as Array<Omit<Highlight, "version" | "sourceArtifactId">>;
    const highlights = parsed.map((item, index) => ({ ...item, version: 1 as const, id: `h-${index}`, sourceArtifactId: transcript.sourceArtifactId, start: item.start, end: item.end }));
    for (const item of highlights) if (item.end - item.start > transcript.duration) { item.end = item.start + Math.min(transcript.duration, Math.max(5, item.end - item.start)); }
    return highlights;
  }
}

/** Slice a transcript into consecutive windows (with overlap) so a long recording never
 *  exceeds a single LLM call. Windows are offset in seconds; each carries the original
 *  transcript-relative time so proposals can be shifted back to absolute timestamps.
 */
export function highlightWindows(transcript: Transcript, windowSeconds = 20 * 60, overlapSeconds = 5 * 60): Array<{ from: number; to: number; window: Transcript }> {
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
  async choose(input: { transcript: Transcript; signal?: AbortSignal }): Promise<Highlight[]> {
    const windows = highlightWindows(input.transcript, 20 * 60, 5 * 60);
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
    const prompt = `Return a JSON array only. Choose up to 5 complete short-video highlights from this transcript window. Each item: {"start":number,"end":number,"wordIds":string[],"title":string,"hook":string,"score":0-100,"reason":string}. Times are relative to window start. Use only the supplied word IDs. Transcript: ${JSON.stringify(transcript)}`;
    const response = await fetch(`${this.baseUrl}/v1/chat/completions`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: this.model, temperature: 0.2, messages: [{ role: "user", content: prompt }] }), signal });
    if (!response.ok) throw new Error(`LLAMA_CPP_PROVIDER_${response.status}`);
    const raw = await response.json() as { choices?: Array<{ message?: { content?: string } }> };
    const text = raw.choices?.[0]?.message?.content ?? "";
    const parsed = JSON.parse(text.replace(/^```json\s*|\s*```$/g, "")) as Array<Omit<Highlight, "version" | "sourceArtifactId">>;
    return parsed.map((item, index) => ({ ...item, version: 1 as const, id: `h-${index}`, sourceArtifactId: transcript.sourceArtifactId, start: item.start, end: item.end }));
  }
}
