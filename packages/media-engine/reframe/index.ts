import type { AspectRatio, RenderPlan } from "../../contracts/render-plan.ts";

export type FaceTrackPoint = {
  time: number;
  x: number;
  y: number;
  width: number;
  height: number;
  confidence: number;
  sceneId?: number;
  snap?: boolean;
  fallback?: boolean;
};

export type TrackOptions = { cutDistance: number; cutSizeRatio: number; noFaceHoldSeconds: number; centerDriftAlpha: number };
export const defaultTrackOptions: TrackOptions = { cutDistance: 0.35, cutSizeRatio: 0.8, noFaceHoldSeconds: 1.5, centerDriftAlpha: 0.08 };

export function smoothFacePath(points: FaceTrackPoint[], alpha = 0.35): FaceTrackPoint[] {
  let previous: FaceTrackPoint | undefined;
  return points.map((point) => {
    if (!previous || point.snap || point.sceneId !== previous.sceneId) { previous = point; return point; }
    const next = { ...point, x: previous.x + alpha * (point.x - previous.x), y: previous.y + alpha * (point.y - previous.y), width: previous.width + alpha * (point.width - previous.width), height: previous.height + alpha * (point.height - previous.height) };
    previous = next;
    return next;
  });
}

export function choosePrimaryFace(points: FaceTrackPoint[], previous?: FaceTrackPoint): FaceTrackPoint | undefined {
  if (!points.length) return undefined;
  return [...points].sort((a, b) => previous ? distance(a, previous) - distance(b, previous) : b.confidence - a.confidence || b.width * b.height - a.width * a.height)[0];
}

export function isSceneCut(previous: FaceTrackPoint | undefined, current: FaceTrackPoint, options = defaultTrackOptions): boolean {
  if (!previous) return false;
  const displacement = distance(previous, current);
  const previousArea = Math.max(0.0001, previous.width * previous.height);
  const currentArea = current.width * current.height;
  return displacement >= options.cutDistance || Math.abs(Math.log(Math.max(0.0001, currentArea) / previousArea)) >= Math.abs(Math.log(options.cutSizeRatio));
}

export function trackFacePath(frames: Array<{ time: number; detections: FaceTrackPoint[] }>, options = defaultTrackOptions) {
  const output: FaceTrackPoint[] = [];
  let previous: FaceTrackPoint | undefined;
  let sceneId = 0;
  let missingSince: number | undefined;
  let sceneCuts = 0;
  for (const frame of frames) {
    const selected = choosePrimaryFace(frame.detections, previous);
    if (selected) {
      const cut = isSceneCut(previous, selected, options);
      if (cut) { sceneId += 1; sceneCuts += 1; }
      const point = { ...selected, time: frame.time, sceneId, snap: cut, fallback: false };
      output.push(point); previous = point; missingSince = undefined;
    } else if (previous) {
      missingSince ??= frame.time;
      const useCenter = frame.time - missingSince >= options.noFaceHoldSeconds;
      const point = { ...previous, time: frame.time, sceneId, snap: false, fallback: true, x: useCenter ? previous.x + options.centerDriftAlpha * (0.5 - previous.x) : previous.x, y: useCenter ? previous.y + options.centerDriftAlpha * (0.5 - previous.y) : previous.y };
      output.push(point);
    }
  }
  return { points: smoothFacePath(output), sceneCuts, fallbackCount: output.filter((point) => point.fallback).length };
}

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

export function ffmpegCropCenterExpression(keyframes: CropKeyframe[], rangeStart: number, axis: "x" | "y" = "x"): string {
  if (!keyframes.length) return "0.5";
  const position = (key: CropKeyframe) => clamp(axis === "x" ? key.centerX : key.centerY ?? 0.5);
  const sorted = [...keyframes].sort((a, b) => a.time - b.time);
  let expression = String(position(sorted[sorted.length - 1]));
  for (let index = sorted.length - 1; index >= 0; index--) {
    const key = sorted[index];
    const localTime = Math.max(0, key.time - rangeStart);
    if (index === 0) expression = `if(lt(t\\,${localTime.toFixed(3)})\\,${position(key)}\\,${expression})`;
    else {
      const previous = sorted[index - 1];
      const from = Math.max(0, previous.time - rangeStart);
      const to = Math.max(0, key.time - rangeStart);
      if (to <= 0) continue;
      const slope = (position(key) - position(previous)) / (key.time - previous.time);
      const intercept = position(previous) - slope * previous.time + slope * rangeStart;
      const linear = `${slope.toFixed(6)}*t+${intercept.toFixed(6)}`;
      expression = `if(lt(t\\,${from.toFixed(3)})\\,${position(previous)}\\,if(lt(t\\,${to.toFixed(3)})\\,${linear}\\,${expression}))`;
    }
  }
  return expression;
}

export function sceneLayoutAt(plan: RenderPlan, time: number): "solo" | "side-by-side" | "picture-in-picture" {
  return plan.sceneLayouts?.find((scene) => time >= scene.start && time < scene.end)?.layout ?? plan.layout ?? "solo";
}

export function aspectTarget(aspectRatio: AspectRatio): number {
  return aspectRatio === "9:16" ? 9 / 16 : aspectRatio === "1:1" ? 1 : 16 / 9;
}

function clamp(value: number): number { return Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0.5)); }
function distance(a: FaceTrackPoint, b: FaceTrackPoint): number { return Math.hypot(a.x + a.width / 2 - b.x - b.width / 2, a.y + a.height / 2 - b.y - b.height / 2); }
