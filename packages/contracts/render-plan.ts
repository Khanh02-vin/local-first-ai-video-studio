import { assertRange, CONTRACT_VERSION, type Timestamp } from "./transcript.ts";

export const ASPECT_RATIOS = ["9:16", "1:1", "16:9"] as const;
export type AspectRatio = (typeof ASPECT_RATIOS)[number];
export type CropMode = "track" | "general" | "split" | "screencast" | "none";
export type Layout = "solo" | "picture-in-picture" | "side-by-side";

export type Caption = {
  id: string;
  text: string;
  start: Timestamp;
  end: Timestamp;
};

export type RenderRange = { start: Timestamp; end: Timestamp };

export type RenderPlan = {
  version: number;
  sourceArtifactId: string;
  ranges: RenderRange[];
  aspectRatio: AspectRatio;
  cropMode: CropMode;
  layout?: Layout;
  speakerMap?: Record<string, string>;
  captions: Caption[];
  hook?: { text: string; start: Timestamp; end: Timestamp };
  effects?: string[];
  outputProfile: { container: "mp4"; codec: "h264" };
};

export function validateRenderPlan(plan: RenderPlan, duration: number): void {
  if (!Number.isInteger(plan.version) || plan.version < 1) throw new Error("Invalid RenderPlan version");
  if (!plan.sourceArtifactId || !ASPECT_RATIOS.includes(plan.aspectRatio)) throw new Error("Invalid RenderPlan metadata");
  if (plan.outputProfile.container !== "mp4" || plan.outputProfile.codec !== "h264") throw new Error("Invalid output profile");

  let previousEnd = 0;
  for (const range of plan.ranges) {
    assertRange(range.start, range.end, duration);
    if (range.start < previousEnd) throw new Error("Render ranges overlap");
    previousEnd = range.end;
  }
  for (const caption of plan.captions) {
    if (!caption.id || !caption.text) throw new Error("Invalid caption");
    assertRange(caption.start, caption.end, duration);
  }
  if (plan.hook) assertRange(plan.hook.start, plan.hook.end, duration);
}
