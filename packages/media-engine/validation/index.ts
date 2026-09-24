import { validateArtifact, type Artifact } from "../../contracts/artifact.ts";
import { validateHighlights, type Highlight } from "../../contracts/highlight.ts";
import { validateRenderPlan, type RenderPlan } from "../../contracts/render-plan.ts";
import { validateTranscript, type Transcript } from "../../contracts/transcript.ts";

export function validateMediaInput(probe: { duration: number; width: number; height: number }): void {
  if (!Number.isFinite(probe.duration) || probe.duration <= 0) throw new Error("Invalid media duration");
  if (!Number.isInteger(probe.width) || probe.width <= 0 || !Number.isInteger(probe.height) || probe.height <= 0) throw new Error("Invalid media dimensions");
}

export function validateMediaBundle(input: { transcript?: Transcript; highlights?: Highlight[]; renderPlan?: RenderPlan; artifact?: Artifact; duration: number }): void {
  if (input.transcript) validateTranscript(input.transcript);
  if (input.highlights) validateHighlights(input.highlights, input.duration);
  if (input.renderPlan) validateRenderPlan(input.renderPlan, input.duration);
  if (input.artifact) validateArtifact(input.artifact);
}
