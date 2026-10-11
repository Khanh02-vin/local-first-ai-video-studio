import { CONTRACT_VERSION } from "../../packages/contracts/transcript.ts";
import type { Highlight } from "../../packages/contracts/highlight.ts";
import { validateCollection, type Collection } from "../../packages/contracts/collection.ts";

/** Mirrors autoclip_mvp's MAX_CLIPS_PER_COLLECTION. */
export const MAX_CLIPS_PER_COLLECTION = 5;
/** autoclip_mvp scores clips 0..1 with 0.8/0.6 tiers; highlight scores are 0..100. */
const HIGH_SCORE = 80;
const MEDIUM_SCORE = 60;

/** One proposed group before it becomes a Collection. */
export type CollectionCluster = {
  title: string;
  topic?: string;
  highlightIds: string[];
};

/** Optional LLM (or test) clusterer. Returned clusters are re-validated here. */
export type CollectionClusterer = (highlights: Highlight[]) => Promise<CollectionCluster[]> | CollectionCluster[];

export type GroupCollectionsOptions = {
  projectId: string;
  sourceArtifactId?: string;
  clusterer?: CollectionClusterer;
  maxPerCollection?: number;
  now?: string;
};

const STOP_WORDS = new Set([
  "this", "that", "with", "from", "your", "have", "about", "what", "when", "they", "them",
  "will", "just", "like", "more", "than", "then", "into", "also", "some", "very", "here",
]);

function tokens(text: string): Set<string> {
  const found = text.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? [];
  return new Set(found.filter((t) => t.length >= 4 && !STOP_WORDS.has(t)));
}

/** Greedy shared-token grouping — autoclip's keyword pre-cluster, generalized. */
function clusterBySharedTokens(highlights: Highlight[]): CollectionCluster[] {
  const groups: { terms: Set<string>; ids: string[] }[] = [];
  for (const highlight of highlights) {
    const terms = tokens(`${highlight.title} ${highlight.hook ?? ""} ${highlight.reason ?? ""}`);
    if (terms.size === 0) continue;
    const group = groups.find((g) => [...terms].some((t) => g.terms.has(t)));
    if (group) {
      group.ids.push(highlight.id);
      for (const term of terms) group.terms.add(term);
    } else {
      groups.push({ terms, ids: [highlight.id] });
    }
  }
  return groups
    .filter((g) => g.ids.length >= 2)
    .map((g, index) => ({ title: `Topic ${index + 1}`, topic: `topic-${index + 1}`, highlightIds: g.ids }));
}

/** Last-resort grouping by score tier — autoclip's _create_default_collections. */
function scoreFallback(highlights: Highlight[]): CollectionCluster[] {
  const clusters: CollectionCluster[] = [];
  const high = highlights.filter((h) => h.score >= HIGH_SCORE).map((h) => h.id);
  const medium = highlights.filter((h) => h.score >= MEDIUM_SCORE && h.score < HIGH_SCORE).map((h) => h.id);
  if (high.length >= 2) clusters.push({ title: "Top-scoring highlights", topic: "high-score", highlightIds: high });
  if (medium.length >= 2) clusters.push({ title: "Recommended highlights", topic: "medium-score", highlightIds: medium });
  return clusters;
}

function build(drafts: CollectionCluster[], highlights: Highlight[], options: GroupCollectionsOptions): Collection[] {
  const max = options.maxPerCollection ?? MAX_CLIPS_PER_COLLECTION;
  const now = options.now ?? new Date().toISOString();
  const byId = new Map(highlights.map((h) => [h.id, h]));
  const collections: Collection[] = [];

  for (const draft of drafts) {
    const ids = [...new Set(draft.highlightIds)].filter((id) => byId.has(id)).slice(0, max);
    if (ids.length < 2) continue;
    const index = collections.length;
    const collection: Collection = {
      version: CONTRACT_VERSION,
      id: `collection-${index}`,
      projectId: options.projectId,
      sourceArtifactId: options.sourceArtifactId ?? byId.get(ids[0])!.sourceArtifactId,
      title: draft.title.trim() || `Collection ${index + 1}`,
      topic: draft.topic?.trim() || undefined,
      items: ids.map((id, position) => ({
        id: `collection-${index}-item-${position}`,
        position,
        reference: { kind: "highlight" as const, highlightId: id },
      })),
      state: "proposed",
      createdAt: now,
      updatedAt: now,
    };
    validateCollection(collection);
    collections.push(collection);
  }
  return collections;
}

/**
 * Groups highlights into proposed Collections, mirroring autoclip_mvp's step 5:
 * optional LLM clustering first, then keyword/score fallbacks when it yields
 * nothing usable. Each returned collection is contract-validated.
 */
export async function groupHighlightsIntoCollections(
  highlights: Highlight[],
  options: GroupCollectionsOptions
): Promise<Collection[]> {
  if (highlights.length === 0) return [];

  let drafts: CollectionCluster[] = [];
  if (options.clusterer) {
    try {
      drafts = await options.clusterer(highlights);
    } catch {
      drafts = [];
    }
  }

  let collections = build(drafts, highlights, options);
  if (collections.length === 0) collections = build(clusterBySharedTokens(highlights), highlights, options);
  if (collections.length === 0) collections = build(scoreFallback(highlights), highlights, options);
  return collections;
}
