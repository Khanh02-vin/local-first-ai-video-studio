import type { Highlight } from "../../../../packages/contracts/highlight.ts";
import type { Transcript } from "../../../../packages/contracts/transcript.ts";
import type { RenderPlan, EditVariant } from "../../../../packages/contracts/render-plan.ts";

// Shared between the Studio workbench, the Editor and the YouTube Studio.
// Runes-based mutable module store (`$state` works in .svelte.ts).
export const studioState = $state<{
  sourcePath: string;
  sourceName: string;
  sourceDuration: number;
  highlights: Highlight[];
  transcript: Transcript | null;
  selectedId: string;
  plan: RenderPlan | null;
  projectId: string;
  plans: Record<string, RenderPlan>;
  variants: EditVariant[];
  selectedVariantId: string;
  editingVariantId: string;
  speakerFaceMatches: Record<string, string>;
  speakerFaceConfirmed: boolean;
}>({ sourcePath: "", sourceName: "", sourceDuration: 0, highlights: [], transcript: null, selectedId: "", plan: null, projectId: "", plans: {}, variants: [], selectedVariantId: "", editingVariantId: "", speakerFaceMatches: {}, speakerFaceConfirmed: false });
