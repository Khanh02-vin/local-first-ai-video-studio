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
export type CropKeyframe = { time: Timestamp; centerX: number; centerY?: number; speakerId?: string; sourceFaceId?: string; sourceArtifactId?: string; sceneId?: number; confidence: number; confirmed: boolean };
export type SceneSubjectBox = { x: number; y: number; width: number; height: number; confidence: number };
export type SceneLayout = { start: Timestamp; end: Timestamp; layout: Layout; confidence: number; confirmed: boolean; sourceArtifactId?: string; faceCount?: number; activeSpeakerId?: string; subjectBoxes?: SceneSubjectBox[] };

export type RenderPlan = {
  version: number;
  sourceArtifactId: string;
  ranges: RenderRange[];
  aspectRatio: AspectRatio;
  cropMode: CropMode;
  layout?: Layout;
  speakerMap?: Record<string, string>;
  cropKeyframes?: CropKeyframe[];
  sceneLayouts?: SceneLayout[];
  captions: Caption[];
  hook?: { text: string; start: Timestamp; end: Timestamp };
  effects?: string[];
  outputProfile: { container: "mp4"; codec: "h264" };
};

export type EditVariant = {
  id: string;
  sourceArtifactId: string;
  title: string;
  reason?: string;
  ranges: Array<RenderRange & { wordIds: string[] }>;
  plan?: RenderPlan;
};


export function validateRenderPlan(plan: RenderPlan, duration: number): void {
  if (!Number.isInteger(plan.version) || plan.version < 1) throw new Error("Invalid RenderPlan version");
  if (!plan.sourceArtifactId || !ASPECT_RATIOS.includes(plan.aspectRatio)) throw new Error("Invalid RenderPlan metadata");
  if (plan.outputProfile.container !== "mp4" || plan.outputProfile.codec !== "h264") throw new Error("Invalid output profile");

  if (plan.ranges.length === 0) throw new Error("RenderPlan must contain at least one range");
  for (let index = 0; index < plan.ranges.length; index++) {
    const range = plan.ranges[index];
    assertRange(range.start, range.end, duration);
    for (let other = 0; other < index; other++) {
      const previous = plan.ranges[other];
      if (range.start < previous.end && previous.start < range.end) throw new Error("Render ranges overlap");
    }
  }
  if (plan.cropKeyframes) {
    if (plan.cropKeyframes.length > 600) throw new Error("Too many crop keyframes");
    let previous = -Infinity;
    for (const keyframe of plan.cropKeyframes) {
      assertRange(keyframe.time, Math.min(duration, keyframe.time + 0.001), duration);
      if (keyframe.time <= previous || !Number.isFinite(keyframe.centerX) || keyframe.centerX < 0 || keyframe.centerX > 1 || (keyframe.centerY !== undefined && (!Number.isFinite(keyframe.centerY) || keyframe.centerY < 0 || keyframe.centerY > 1)) || !Number.isFinite(keyframe.confidence) || keyframe.confidence < 0 || keyframe.confidence > 1 || typeof keyframe.confirmed !== "boolean") throw new Error("Invalid crop keyframe");
      if (keyframe.speakerId !== undefined && (!keyframe.speakerId.trim() || keyframe.speakerId.length > 64)) throw new Error("Invalid crop speaker");
      if (keyframe.sourceFaceId !== undefined && (!keyframe.sourceFaceId.trim() || keyframe.sourceFaceId.length > 128)) throw new Error("Invalid crop source face");
      if (keyframe.sourceArtifactId !== undefined && keyframe.sourceArtifactId !== plan.sourceArtifactId) throw new Error("Crop keyframe source mismatch");
      if (keyframe.sourceFaceId !== undefined && keyframe.speakerId !== undefined && plan.speakerMap?.[keyframe.speakerId] !== keyframe.sourceFaceId) throw new Error("Crop keyframe speaker mapping mismatch");
      previous = keyframe.time;
    }
  }
  if (plan.sceneLayouts) {
    if (plan.sceneLayouts.length > 600) throw new Error("Too many scene layouts");
    let previousEnd = -Infinity;
    for (const scene of plan.sceneLayouts) {
      assertRange(scene.start, scene.end, duration);
      if (scene.start < previousEnd || !["solo", "side-by-side", "picture-in-picture"].includes(scene.layout) || !Number.isFinite(scene.confidence) || scene.confidence < 0 || scene.confidence > 1 || typeof scene.confirmed !== "boolean" || (scene.sourceArtifactId !== undefined && scene.sourceArtifactId !== plan.sourceArtifactId)) throw new Error("Invalid scene layout");
      if (scene.activeSpeakerId !== undefined && !scene.activeSpeakerId.trim()) throw new Error("Invalid scene speaker");
      if (scene.faceCount !== undefined && (!Number.isInteger(scene.faceCount) || scene.faceCount < 0 || scene.faceCount > 4)) throw new Error("Invalid scene face count");
      if (scene.subjectBoxes !== undefined && (scene.subjectBoxes.length !== 2 || scene.subjectBoxes.some((box) => ![box.x, box.y, box.width, box.height, box.confidence].every(Number.isFinite) || box.x < 0 || box.y < 0 || box.width <= 0 || box.height <= 0 || box.x + box.width > 1 || box.y + box.height > 1 || box.confidence < 0 || box.confidence > 1))) throw new Error("Invalid scene subject boxes");
      previousEnd = scene.end;
    }
  }
  for (const caption of plan.captions) {
    if (!caption.id || !caption.text) throw new Error("Invalid caption");
    assertRange(caption.start, caption.end, duration);
  }
  if (plan.hook) assertRange(plan.hook.start, plan.hook.end, duration);
}
