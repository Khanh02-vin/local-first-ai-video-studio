// The crop centre a plan resolves to at a given time.
//
// The renderer resolves crop keyframes itself, in the Rust render engine
// (crates/render-engine). This function is the editor-side view of the same data: it
// tells the user where the crop currently sits, so the "Crop X" slider shows a
// meaningful value. It is not part of the render path.
//
// The tracking itself (picking the speaker's face, smoothing, scene cuts) happens in
// the Rust `analyze_crop_path` command. `crop-keyframes.ts` maps its output onto the
// contract. Nothing re-tracks or re-smooths that path on the TypeScript side — doing
// so once made the stored crop drift away from the subject.
import type { RenderPlan } from "../../contracts/render-plan.ts";

export type CropKeyframe = NonNullable<RenderPlan["cropKeyframes"]>[number];

export type CropCenter = { x: number; y: number };

export function cropCenterAt(keyframes: CropKeyframe[], time: number): CropCenter {
  const sorted = [...keyframes].sort((a, b) => a.time - b.time);
  if (!sorted.length) return { x: 0.5, y: 0.5 };
  const center = (key: CropKeyframe) => ({ x: clamp(key.centerX), y: clamp(key.centerY ?? 0.5) });
  if (time <= sorted[0].time) return center(sorted[0]);
  for (let index = 1; index < sorted.length; index++) {
    const right = sorted[index];
    const left = sorted[index - 1];
    if (time <= right.time) {
      const amount = (time - left.time) / (right.time - left.time);
      return { x: clamp(left.centerX + amount * (right.centerX - left.centerX)), y: clamp((left.centerY ?? 0.5) + amount * ((right.centerY ?? 0.5) - (left.centerY ?? 0.5))) };
    }
  }
  return center(sorted[sorted.length - 1]);
}

function clamp(value: number): number { return Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0.5)); }
