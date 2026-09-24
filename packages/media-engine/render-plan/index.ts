import type { RenderPlan } from "../../contracts/render-plan.ts";
import { validateRenderPlan } from "../../contracts/render-plan.ts";

export function validateEngineRenderPlan(plan: RenderPlan, duration: number): RenderPlan {
  validateRenderPlan(plan, duration);
  return plan;
}

export type CropWindow = { width: number; height: number; x: number; y: number };

export function cropWindow(sourceWidth: number, sourceHeight: number, aspectRatio: "9:16" | "1:1" | "16:9", center = 0.5): CropWindow {
  const target = aspectRatio === "9:16" ? 9 / 16 : aspectRatio === "1:1" ? 1 : 16 / 9;
  const width = Math.min(sourceWidth, sourceHeight * target);
  const height = Math.min(sourceHeight, sourceWidth / target);
  const x = Math.max(0, Math.min(sourceWidth - width, sourceWidth * center - width / 2));
  const y = Math.max(0, (sourceHeight - height) / 2);
  return { width, height, x, y };
}
