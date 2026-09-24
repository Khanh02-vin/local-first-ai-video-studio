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

function distance(a: FaceTrackPoint, b: FaceTrackPoint): number {
  return Math.hypot(a.x + a.width / 2 - b.x - b.width / 2, a.y + a.height / 2 - b.y - b.height / 2);
}
