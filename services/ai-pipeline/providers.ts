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

export class GeminiHighlightProvider implements HighlightProvider {
  private readonly apiKey: string;
  private readonly model: string;
  private readonly endpoint: string;
  constructor(apiKey: string, model = "gemini-2.0-flash", endpoint = "https://generativelanguage.googleapis.com/v1beta/models") { this.apiKey = apiKey; this.model = model; this.endpoint = endpoint; }
  async choose(input: { transcript: Transcript; signal?: AbortSignal }): Promise<Highlight[]> {
    const prompt = `Return JSON array only. Choose 3-5 complete short-video highlights. Each item has start,end,title,hook,score,reason,wordIds. Use only the supplied word IDs and timestamps. Transcript: ${JSON.stringify(input.transcript)}`;
    const response = await fetch(`${this.endpoint}/${this.model}:generateContent?key=${encodeURIComponent(this.apiKey)}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ contents: [{ parts: [{ text: prompt }] }] }), signal: input.signal });
    if (!response.ok) throw new Error(`GEMINI_PROVIDER_${response.status}`);
    const raw = await response.json() as { candidates?: Array<{ content?: { parts?: Array<{ text?: string }> } }> };
    const text = raw.candidates?.[0]?.content?.parts?.map((part) => part.text ?? "").join("") ?? "";
    const parsed = JSON.parse(text.replace(/^```json\s*|\s*```$/g, "")) as Array<Omit<Highlight, "version" | "sourceArtifactId">>;
    const highlights = parsed.map((item, index) => ({ ...item, version: 1 as const, id: `h-${index}`, sourceArtifactId: input.transcript.sourceArtifactId }));
    validateHighlights(highlights, input.transcript.duration); return highlights;
  }
}
