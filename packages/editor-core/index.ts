import type { Highlight } from "../contracts/highlight.ts";
import type { Transcript } from "../contracts/transcript.ts";
import type { Caption, RenderPlan, RenderRange } from "../contracts/render-plan.ts";
import { validateRenderPlan } from "../contracts/render-plan.ts";

export type EditorState = { sourceArtifactId: string; duration: number; selected?: Highlight; plan: RenderPlan };

type PlanPatch = (plan: RenderPlan) => RenderPlan;

function updatePlan(state: EditorState, patch: PlanPatch): EditorState {
  const plan = patch({ ...state.plan, version: state.plan.version + 1, ranges: state.plan.ranges.map((range) => ({ ...range })), captions: state.plan.captions.map((caption) => ({ ...caption })) });
  validateRenderPlan(plan, state.duration);
  return { ...state, plan };
}

export function createEditorState(sourceArtifactId: string, duration: number, selected: Highlight): EditorState {
  const plan: RenderPlan = { version: 1, sourceArtifactId, ranges: [{ start: selected.start, end: selected.end }], aspectRatio: "9:16", cropMode: "track", layout: "solo", captions: [], hook: selected.hook ? { text: selected.hook, start: selected.start, end: Math.min(selected.start + 5, selected.end) } : undefined, outputProfile: { container: "mp4", codec: "h264" } };
  validateRenderPlan(plan, duration);
  return { sourceArtifactId, duration, selected, plan };
}

export function updateAspectRatio(state: EditorState, aspectRatio: RenderPlan["aspectRatio"]): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, aspectRatio }));
}

export function updateRange(state: EditorState, range: RenderRange): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, ranges: plan.ranges.map((item, index) => index === 0 ? { ...range } : item) }));
}

export function updateHook(state: EditorState, hook: RenderPlan["hook"]): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, hook: hook ? { ...hook } : undefined }));
}

export function upsertCaption(state: EditorState, caption: Caption): EditorState {
  return updatePlan(state, (plan) => {
    const found = plan.captions.some((item) => item.id === caption.id);
    return { ...plan, captions: found ? plan.captions.map((item) => item.id === caption.id ? { ...caption } : item) : [...plan.captions, { ...caption }].sort((a, b) => a.start - b.start) };
  });
}

export function removeCaption(state: EditorState, id: string): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, captions: plan.captions.filter((caption) => caption.id !== id) }));
}

export function captionsFromTranscript(transcript: Transcript | null, selected: RenderRange): Caption[] {
  if (!transcript) return [];
  return transcript.segments
    .filter((segment) => segment.end > selected.start && segment.start < selected.end)
    .map((segment) => ({ id: `caption-${segment.id}`, text: segment.text, start: Math.max(segment.start, selected.start), end: Math.min(segment.end, selected.end) }));
}

export function loadTranscriptCaptions(state: EditorState, transcript: Transcript | null): EditorState {
  const range = state.plan.ranges[0];
  return updatePlan(state, (plan) => ({ ...plan, captions: captionsFromTranscript(transcript, range) }));
}
