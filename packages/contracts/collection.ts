import { CONTRACT_VERSION, type Timestamp } from "./transcript.ts";
import type { Highlight } from "./highlight.ts";

export type CollectionItemReference = 
  | { kind: "highlight"; highlightId: string }
  | { kind: "renderPlan"; renderPlanId: string }
  | { kind: "artifact"; artifactId: string };

export type CollectionItem = {
  id: string;
  position: number;
  reference: CollectionItemReference;
};

export type CollectionState = "proposed" | "approved" | "rejected";

export type Collection = {
  version: typeof CONTRACT_VERSION;
  id: string;
  projectId: string;
  sourceArtifactId?: string;
  title: string;
  topic?: string;
  items: CollectionItem[];
  state: CollectionState;
  createdAt: string;
  updatedAt: string;
};

export function validateCollection(collection: Collection, sourceDuration?: number): void {
  if (collection.version !== CONTRACT_VERSION || !collection.id || !collection.projectId) {
    throw new Error("Invalid collection metadata");
  }
  if (!collection.title || !collection.title.trim()) {
    throw new Error("Collection title is required");
  }
  if (!["proposed", "approved", "rejected"].includes(collection.state)) {
    throw new Error("Invalid collection state");
  }
  if (!collection.createdAt || !collection.updatedAt) {
    throw new Error("Collection timestamps are required");
  }

  const seenIds = new Set<string>();
  const seenPositions = new Set<number>();
  let previousPosition = -1;

  for (const item of collection.items) {
    if (!item.id || !item.id.trim()) throw new Error("Collection item id is required");
    if (seenIds.has(item.id)) throw new Error("Duplicate collection item id");
    seenIds.add(item.id);

    if (!Number.isInteger(item.position) || item.position < 0) {
      throw new Error("Collection item position must be a non-negative integer");
    }
    if (seenPositions.has(item.position)) throw new Error("Duplicate collection item position");
    if (item.position !== previousPosition + 1) throw new Error("Collection item positions must be contiguous starting from 0");
    seenPositions.add(item.position);
    previousPosition = item.position;

    const ref = item.reference;
    if (!ref.kind || !["highlight", "renderPlan", "artifact"].includes(ref.kind)) {
      throw new Error("Collection item reference kind must be highlight, renderPlan, or artifact");
    }
    const targetId = ref.kind === "highlight" ? ref.highlightId : ref.kind === "renderPlan" ? ref.renderPlanId : ref.artifactId;
    if (!targetId || !targetId.trim()) throw new Error("Collection item reference id is required");
  }

  if (sourceDuration !== undefined && !Number.isFinite(sourceDuration)) {
    throw new Error("Invalid source duration for collection validation");
  }
}