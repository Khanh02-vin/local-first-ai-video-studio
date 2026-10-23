// Turns the raw crop path that the Rust `analyze_crop_path` command returns into the
// `CropKeyframe` entries a RenderPlan stores.
//
// The Rust command already does the tracking work: it picks the speaker's face,
// smooths the path (alpha 0.35) and assigns a scene id. This module only maps that
// output onto the contract — it must not smooth, re-track or invent positions.
//
// Why it matters: the Editor used to feed the Rust output back through the TypeScript
// `trackFacePath`, which smoothed the path a second time and re-derived scene cuts
// with a different rule. That made the stored crop drift away from the subject — with
// the subject moving from x=0.2 to x=0.8, Rust alone converged to 0.73 while the
// double-smoothed path stopped at 0.61. The renderer reads the plan, so the plan must
// carry the position the tracker actually computed.
import type { CropKeyframe } from "../../contracts/render-plan.ts";

export type RawCropPathPoint = {
  time: number;
  centerX: number;
  centerY: number;
  speakerId?: string | null;
  faceIndex: number;
  confidence: number;
  sourceArtifactId: string;
  sceneId: number;
};

export type CropPathMapping = {
  /** speakerId → face id, as confirmed by the user in the Speaker mapping panel. */
  speakerFaceMatches?: Record<string, string>;
  /** True only when the mapping was confirmed against this exact source artifact. */
  mappingConfirmed: boolean;
  sourceArtifactId: string;
};

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0.5));
}

/**
 * Maps the raw crop path onto contract keyframes.
 *
 * A keyframe is only trusted as a *speaker* keyframe when the speaker→face mapping was
 * explicitly confirmed for this source artifact; otherwise its confidence is capped
 * below the renderer's 0.5 threshold, so the render falls back to a centre crop instead
 * of following an unconfirmed face. The user still has to approve the tracking
 * (`confirmed: false`) before the renderer uses it.
 */
export function cropKeyframesFromPath(raw: RawCropPathPoint[], mapping: CropPathMapping): CropKeyframe[] {
  return raw.map((item) => {
    const mapped = Boolean(
      item.speakerId &&
      mapping.mappingConfirmed &&
      mapping.speakerFaceMatches?.[item.speakerId],
    );
    return {
      time: item.time,
      centerX: clamp01(item.centerX),
      centerY: clamp01(item.centerY),
      speakerId: mapped ? (item.speakerId as string) : undefined,
      sourceFaceId: mapped ? mapping.speakerFaceMatches![item.speakerId!] : undefined,
      sourceArtifactId: mapping.sourceArtifactId,
      sceneId: item.sceneId,
      // An unconfirmed mapping must not survive the renderer's confidence gate.
      confidence: mapped ? item.confidence : Math.min(0.49, item.confidence),
      confirmed: false,
    };
  });
}
