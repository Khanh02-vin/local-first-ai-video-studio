import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";
import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { SemanticHighlightOptions, SemanticHighlightProvider, SemanticHighlightResult } from "./semantic.ts";
import { HIGHLIGHT_PROMPT_VERSION } from "./providers.ts";

/**
 * LLM result cache: same transcript + same prompt version → stored highlights,
 * no repeat Gemini/Qwen call. Re-running analysis (retry, strategy flip back,
 * reopening a project) becomes free.
 *
 * Files live under <state>/llm-cache/<sha256>.json; entries are a few KB and
 * keyed immutably, so there is no eviction to manage. A corrupt or
 * contract-invalid entry is treated as a miss and overwritten.
 */

function cacheDir(): string {
  return join(process.env.LOCAL_FIRST_STATE_DIR ?? join(process.env.HOME ?? ".", ".cache/local-first-ai-video-studio"), "llm-cache");
}

/** Key covers the prompt version, the provider identity (model included), the
 *  options that shape the output, and the transcript the LLM actually sees. */
export function llmCacheKey(identity: string, transcript: Transcript, options?: SemanticHighlightOptions): string {
  return createHash("sha256").update([HIGHLIGHT_PROMPT_VERSION, identity, JSON.stringify(options ?? {}), JSON.stringify({ sourceArtifactId: transcript.sourceArtifactId, duration: transcript.duration, words: transcript.words })].join("\0")).digest("hex");
}

type CachedEntry = { highlights: Highlight[]; identity: string; promptVersion: string; cachedAt: string };

function readEntry(path: string, identity: string): SemanticHighlightResult | undefined {
  try {
    const entry = JSON.parse(readFileSync(path, "utf8")) as CachedEntry;
    if (entry.promptVersion !== HIGHLIGHT_PROMPT_VERSION || entry.identity !== identity || !Array.isArray(entry.highlights) || !entry.highlights.length) return undefined;
    validateHighlights(entry.highlights, Number.MAX_SAFE_INTEGER);
    return { highlights: entry.highlights, provider: entry.identity };
  } catch { return undefined; }
}

function writeEntry(path: string, identity: string, result: SemanticHighlightResult): void {
  try {
    mkdirSync(cacheDir(), { recursive: true });
    const temp = `${path}.${process.pid}.tmp`;
    writeFileSync(temp, JSON.stringify({ highlights: result.highlights, identity, promptVersion: HIGHLIGHT_PROMPT_VERSION, cachedAt: new Date().toISOString() } satisfies CachedEntry));
    renameSync(temp, path);
  } catch { /* a failed cache write must never fail the analysis */ }
}

/** Memoizes any SemanticHighlightProvider. The inner provider still validates
 *  its own output; this layer only skips the network/model call on a hit. */
export class CachedSemanticProvider implements SemanticHighlightProvider {
  private readonly inner: SemanticHighlightProvider;
  private readonly identity: string;
  constructor(inner: SemanticHighlightProvider, identity: string) { this.inner = inner; this.identity = identity; }

  async generate(input: { transcript: Transcript; options?: SemanticHighlightOptions; signal?: AbortSignal }): Promise<SemanticHighlightResult> {
    const key = llmCacheKey(this.identity, input.transcript, input.options);
    const path = join(cacheDir(), `${key}.json`);
    const cached = readEntry(path, this.identity);
    if (cached) return cached;
    const result = await this.inner.generate(input);
    // Only successful, non-empty results are worth replaying; empty output may
    // be a transient provider quirk and should retry next time.
    if (result.highlights.length) writeEntry(path, this.identity, result);
    return result;
  }
}
