import type { Highlight } from "../../packages/contracts/highlight.ts";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { validateHighlights } from "../../packages/contracts/highlight.ts";

export type SemanticHighlightProposal = {
  start: number;
  end: number;
  wordIds: string[];
  title: string;
  hook?: string;
  score: number;
  reason?: string;
};

export type SemanticHighlightResult = {
  highlights: Highlight[];
  provider: string;
  rawProposals?: SemanticHighlightProposal[];
};

export type SemanticHighlightOptions = {
  category?: string;
  limit?: number;
  minDuration?: number;
  maxDuration?: number;
};

export interface SemanticHighlightProvider {
  generate(
    input: { transcript: Transcript; options?: SemanticHighlightOptions; signal?: AbortSignal }
  ): Promise<SemanticHighlightResult>;
}

export class NotImplementedSemanticProvider implements SemanticHighlightProvider {
  async generate(): Promise<SemanticHighlightResult> {
    throw new Error("SEMANTIC_HIGHLIGHT_NOT_IMPLEMENTED");
  }
}

function clampScore(score: number): number {
  return Math.max(0, Math.min(100, Math.round(score)));
}

function normalizeProposals(
  transcript: Transcript,
  proposals: SemanticHighlightProposal[],
  options: Required<SemanticHighlightOptions>
): Highlight[] {
  const duration = transcript.duration;
  const wordIdSet = new Set(transcript.words.map((w) => w.id));

  const normalized: Highlight[] = [];
  for (let i = 0; i < proposals.length; i++) {
    const p = proposals[i];
    if (!Number.isFinite(p.start) || !Number.isFinite(p.end) || p.start < 0 || p.end <= p.start) {
      continue;
    }
    if (p.end > duration) {
      continue;
    }
    if (p.end - p.start < options.minDuration || p.end - p.start > options.maxDuration) {
      continue;
    }
    if (p.wordIds.some((id) => !wordIdSet.has(id))) {
      continue;
    }
    const words = transcript.words.filter((w) => w.start >= p.start && w.end <= p.end);
    const wordIds = words.length > 0 ? words.map((w) => w.id) : p.wordIds;
    if (wordIds.length === 0) {
      continue;
    }
    normalized.push({
      version: 1 as const,
      id: `semantic-${i}`,
      sourceArtifactId: transcript.sourceArtifactId,
      start: p.start,
      end: p.end,
      wordIds,
      title: p.title.trim().slice(0, 120),
      hook: p.hook?.trim().slice(0, 200),
      score: clampScore(p.score),
      reason: p.reason?.trim().slice(0, 500) ?? "semantic proposal",
    });
  }
  return normalized;
}

function deoverlapAndRank(highlights: Highlight[], limit: number): Highlight[] {
  const sorted = [...highlights].sort((a, b) => b.score - a.score || a.start - b.start);
  const chosen: Highlight[] = [];
  for (const candidate of sorted) {
    if (chosen.some((other) => candidate.start < other.end && other.start < candidate.end)) {
      continue;
    }
    chosen.push(candidate);
    if (chosen.length >= limit) break;
  }
  return chosen.sort((a, b) => a.start - b.start);
}

export async function generateSemanticHighlights(
  transcript: Transcript,
  provider: SemanticHighlightProvider,
  options: SemanticHighlightOptions = {}
): Promise<Highlight[]> {
  const mergedOptions: Required<SemanticHighlightOptions> = {
    category: options.category ?? "default",
    limit: options.limit ?? 5,
    minDuration: options.minDuration ?? 10,
    maxDuration: options.maxDuration ?? 90,
  };

  const result = await provider.generate({ transcript, options: mergedOptions });

  if (!result?.highlights || result.highlights.length === 0) {
    if (result?.rawProposals?.length) {
      const normalized = normalizeProposals(transcript, result.rawProposals, mergedOptions);
      const ranked = deoverlapAndRank(normalized, mergedOptions.limit);
      validateHighlights(ranked, transcript.duration);
      return ranked;
    }
    return [];
  }

  const ranked = deoverlapAndRank(result.highlights, mergedOptions.limit);
  validateHighlights(ranked, transcript.duration);
  return ranked;
}

export function createMockSemanticProvider(
  proposals: SemanticHighlightProposal[],
  providerName = "mock-semantic"
): SemanticHighlightProvider {
  return {
    async generate({ transcript }) {
      const proposalsWithinDuration = proposals.filter(
        (p) => Number.isFinite(p.start) && Number.isFinite(p.end) && p.end <= transcript.duration
      );
      const highlights: Highlight[] = proposalsWithinDuration.map((p, i) => ({
        version: 1 as const,
        id: `semantic-${i}`,
        sourceArtifactId: transcript.sourceArtifactId,
        start: p.start,
        end: p.end,
        wordIds: p.wordIds,
        title: p.title,
        hook: p.hook,
        score: p.score,
        reason: p.reason,
      }));
      return { highlights, provider: providerName, rawProposals: proposalsWithinDuration };
    },
  };
}

/**
 * Adapts the existing LLM HighlightProvider interface (Gemini, etc.) to the
 * SemanticHighlightProvider shape expected by the local-first pipeline.
 */
export class SemanticProviderAdapter implements SemanticHighlightProvider {
  private readonly inner: { choose(input: { transcript: Transcript; signal?: AbortSignal }): Promise<Highlight[]> };
  private readonly name: string;
  constructor(inner: { choose(input: { transcript: Transcript; signal?: AbortSignal }): Promise<Highlight[]> }, name = "semantic-llm") { this.inner = inner; this.name = name; }

  async generate(input: { transcript: Transcript; options?: SemanticHighlightOptions; signal?: AbortSignal }): Promise<SemanticHighlightResult> {
    const highlights = await this.inner.choose({ transcript: input.transcript, signal: input.signal });
    return { highlights, provider: this.name, rawProposals: highlights.map((h) => ({ start: h.start, end: h.end, wordIds: h.wordIds, title: h.title, hook: h.hook, score: h.score, reason: h.reason })) };
  }
}