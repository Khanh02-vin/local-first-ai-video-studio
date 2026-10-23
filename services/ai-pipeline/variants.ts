import type { Transcript } from "../../packages/contracts/transcript.ts";
import type { EditVariant } from "../../packages/contracts/render-plan.ts";
import { validateTranscript } from "../../packages/contracts/transcript.ts";

export type { EditVariant } from "../../packages/contracts/render-plan.ts";
export type EditVariantRange = EditVariant["ranges"][number];
type VariantProvider = { generate(input: { transcript: Transcript; signal?: AbortSignal }): Promise<EditVariant[]> };

export const VARIANT_OUTPUT_SCHEMA = {
  type: "object",
  properties: {
    variants: {
      type: "array",
      items: {
        type: "object",
        properties: {
          title: { type: "string" },
          reason: { type: "string" },
          ranges: { type: "array", items: { type: "object", properties: { from: { type: "integer" }, to: { type: "integer" } }, required: ["from", "to"] } },
        },
        required: ["title", "ranges"],
      },
    },
  },
  required: ["variants"],
} as const;

export const VARIANT_PROMPT_VERSION = "transcript-edit-variants-v1";
export const VARIANT_MAX_COUNT = 4;
export const VARIANT_MAX_RANGES = 8;

export function editVariantPrompt(transcript: Transcript): string {
  const lines = transcript.words.map((word, index) => `${index} [${word.start.toFixed(2)}-${word.end.toFixed(2)}] ${word.text}`);
  return `Create up to ${VARIANT_MAX_COUNT} distinct short edit variants from this transcript. Each variant is an ordered sequence of up to ${VARIANT_MAX_RANGES} non-overlapping contiguous word ranges. Reply only as JSON {"variants":[{"title":string,"reason":string,"ranges":[{"from":integer,"to":integer}]}]}. from/to are inclusive word indices. Keep each edit coherent and concise. Do not invent words, timestamps or word IDs; do not include timestamps or word IDs in the response. Transcript lines: index [source-start-source-end] word.\n${lines.join("\n")}`;
}

export function mapEditVariantProposals(transcript: Transcript, value: unknown): EditVariant[] {
  validateTranscript(transcript);
  const parsed = Array.isArray(value) ? value : value && typeof value === "object" && Array.isArray((value as { variants?: unknown }).variants) ? (value as { variants: unknown[] }).variants : null;
  if (!parsed) throw new Error("LLM_VARIANT_SHAPE");
  const variants: EditVariant[] = [];
  for (const candidate of parsed.slice(0, VARIANT_MAX_COUNT)) {
    if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) continue;
    const item = candidate as Record<string, unknown>;
    if (!Array.isArray(item.ranges) || item.ranges.length === 0 || item.ranges.length > VARIANT_MAX_RANGES) continue;
    const ranges: Array<{ start: number; end: number; wordIds: string[] }> = [];
    let invalid = false;
    for (const raw of item.ranges) {
      if (!raw || typeof raw !== "object" || Array.isArray(raw)) { invalid = true; break; }
      const proposal = raw as Record<string, unknown>;
      const from = proposal.from;
      const to = proposal.to;
      if (!Number.isInteger(from) || !Number.isInteger(to) || (from as number) < 0 || (to as number) < (from as number) || (to as number) >= transcript.words.length) { invalid = true; break; }
      const words = transcript.words.slice(from as number, (to as number) + 1);
    const range = { start: words[0].start, end: words.at(-1)!.end, wordIds: words.map((word) => word.id) };
      if (ranges.some((existing) => range.start < existing.end && existing.start < range.end)) { invalid = true; break; }
      ranges.push(range);
    }
    if (invalid) continue;
    variants.push({
      id: `variant-${variants.length + 1}`,
      sourceArtifactId: transcript.sourceArtifactId,
      title: (typeof item.title === "string" ? item.title.trim().slice(0, 100) : "") || `Variant ${variants.length + 1}`,
      reason: typeof item.reason === "string" ? item.reason.trim().slice(0, 300) : undefined,
      ranges,
    });
  }
  return variants;
}

export class GeminiTranscriptVariantProvider implements VariantProvider {
  private readonly apiKey: string;
  private readonly model: string;
  private readonly endpoint: string;
  constructor(apiKey: string, model = "gemini-2.0-flash", endpoint = "https://generativelanguage.googleapis.com/v1beta/models") { this.apiKey = apiKey; this.model = model; this.endpoint = endpoint; }
  async generate({ transcript, signal }: { transcript: Transcript; signal?: AbortSignal }): Promise<EditVariant[]> {
    const schema = { ...VARIANT_OUTPUT_SCHEMA, type: "OBJECT", properties: { variants: { ...VARIANT_OUTPUT_SCHEMA.properties.variants, type: "ARRAY", items: { type: "OBJECT", properties: { title: { type: "STRING" }, reason: { type: "STRING" }, ranges: { type: "ARRAY", items: { type: "OBJECT", properties: { from: { type: "INTEGER" }, to: { type: "INTEGER" } }, required: ["from", "to"] } } }, required: ["title", "ranges"] } } }, required: ["variants"] };
    const response = await fetch(`${this.endpoint}/${this.model}:generateContent?key=${encodeURIComponent(this.apiKey)}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ contents: [{ parts: [{ text: editVariantPrompt(transcript) }] }], generationConfig: { responseMimeType: "application/json", responseSchema: schema } }), signal });
    if (!response.ok) throw new Error(`GEMINI_VARIANT_PROVIDER_${response.status}`);
    const raw = await response.json() as { candidates?: Array<{ content?: { parts?: Array<{ text?: string }> } }> };
    const text = raw.candidates?.[0]?.content?.parts?.map((part) => part.text ?? "").join("") ?? "";
    return mapEditVariantProposals(transcript, JSON.parse(text.replace(/^```json\s*|\s*```$/g, "")));
  }
}

export class LlamaCppTranscriptVariantProvider implements VariantProvider {
  private readonly baseUrl: string;
  private readonly model: string;
  private modelId?: string;
  constructor(baseUrl = "http://127.0.0.1:8080", model = "qwen2.5-3b-instruct") { this.baseUrl = baseUrl.replace(/\/+$/, ""); this.model = model; }
  async generate({ transcript, signal }: { transcript: Transcript; signal?: AbortSignal }): Promise<EditVariant[]> {
    let modelId = this.modelId;
    if (!modelId) {
      const models = await fetch(`${this.baseUrl}/v1/models`, { signal });
      if (!models.ok) throw new Error(`LLAMA_CPP_MODELS_${models.status}`);
      const raw = await models.json() as { data?: Array<{ id?: string }>; models?: Array<{ name?: string }> };
      modelId = raw.data?.[0]?.id ?? raw.models?.[0]?.name;
      if (!modelId) throw new Error("LLAMA_CPP_NO_MODEL_LOADED");
      this.modelId = modelId;
    }
    const response = await fetch(`${this.baseUrl}/v1/chat/completions`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: modelId, temperature: 0.4, max_tokens: 1200, response_format: { type: "json_object", schema: VARIANT_OUTPUT_SCHEMA }, messages: [{ role: "user", content: editVariantPrompt(transcript) }] }), signal });
    if (!response.ok) throw new Error(`LLAMA_CPP_VARIANT_PROVIDER_${response.status}`);
    const raw = await response.json() as { choices?: Array<{ message?: { content?: string } }> };
    const content = raw.choices?.[0]?.message?.content ?? "";
    const fenced = content.match(/```(?:json)?\s*([\s\S]*?)\s*```/)?.[1] ?? content;
    const start = fenced.search(/[\[{]/);
    if (start < 0) throw new Error("LLAMA_CPP_VARIANT_BAD_JSON");
    let parsed: unknown;
    try { parsed = JSON.parse(fenced.slice(start)); }
    catch { throw new Error("LLAMA_CPP_VARIANT_BAD_JSON"); }
    return mapEditVariantProposals(transcript, parsed);
  }
}

export type { VariantProvider };
