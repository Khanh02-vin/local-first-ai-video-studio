import type { Highlight } from "../contracts/highlight.ts";
import type { Transcript } from "../contracts/transcript.ts";
import type { Caption, EditVariant, RenderPlan, RenderRange } from "../contracts/render-plan.ts";
import { validateRenderPlan } from "../contracts/render-plan.ts";

export function createEditorStateFromVariant(transcript: Transcript, variant: EditVariant): EditorState {
  if (variant.sourceArtifactId !== transcript.sourceArtifactId) throw new Error("VARIANT_SOURCE_MISMATCH");
  const plan: RenderPlan = {
    version: 1,
    sourceArtifactId: transcript.sourceArtifactId,
    ranges: variant.ranges.map(({ start, end }) => ({ start, end })),
    aspectRatio: "9:16",
    cropMode: "track",
    layout: "solo",
    captions: variant.plan?.captions ?? variant.ranges.flatMap((range, index) => captionsFromTranscript(transcript, range).map((caption) => ({ ...caption, id: `${caption.id}-range-${index}` }))),
    hook: variant.plan?.hook,
    outputProfile: { container: "mp4", codec: "h264" },
  };
  validateRenderPlan(plan, transcript.duration);
  return { sourceArtifactId: transcript.sourceArtifactId, duration: transcript.duration, plan };
}

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

export function setSpeakerMap(state: EditorState, speakerMap: Record<string, string>): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, speakerMap: { ...speakerMap } }));
}

export function setCropKeyframes(state: EditorState, cropKeyframes: NonNullable<RenderPlan["cropKeyframes"]>): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, cropKeyframes: cropKeyframes.map((keyframe) => ({ ...keyframe })) }));
}

export function setSceneLayouts(state: EditorState, sceneLayouts: NonNullable<RenderPlan["sceneLayouts"]>): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, sceneLayouts: sceneLayouts.map((scene) => ({ ...scene, subjectBoxes: scene.subjectBoxes?.map((box) => ({ ...box })) })) }));
}

export function updateAspectRatio(state: EditorState, aspectRatio: RenderPlan["aspectRatio"]): EditorState {
  return updatePlan(state, (plan) => ({ ...plan, aspectRatio }));
}

export function updateRange(state: EditorState, range: RenderRange, index = 0): EditorState {
  if (!Number.isInteger(index) || index < 0 || index >= state.plan.ranges.length) throw new Error("Invalid render range index");
  return updatePlan(state, (plan) => ({ ...plan, ranges: plan.ranges.map((item, itemIndex) => itemIndex === index ? { ...range } : item) }));
}

export function addRange(state: EditorState, range: RenderRange, index = state.plan.ranges.length): EditorState {
  if (!Number.isInteger(index) || index < 0 || index > state.plan.ranges.length) throw new Error("Invalid render range index");
  return updatePlan(state, (plan) => ({ ...plan, ranges: [...plan.ranges.slice(0, index), { ...range }, ...plan.ranges.slice(index)] }));
}

export function removeRange(state: EditorState, index: number): EditorState {
  if (state.plan.ranges.length <= 1) throw new Error("At least one render range is required");
  if (!Number.isInteger(index) || index < 0 || index >= state.plan.ranges.length) throw new Error("Invalid render range index");
  return updatePlan(state, (plan) => ({ ...plan, ranges: plan.ranges.filter((_, itemIndex) => itemIndex !== index) }));
}

export function moveRange(state: EditorState, index: number, offset: -1 | 1): EditorState {
  const target = index + offset;
  if (!Number.isInteger(index) || index < 0 || index >= state.plan.ranges.length || target < 0 || target >= state.plan.ranges.length) return state;
  return updatePlan(state, (plan) => {
    const ranges = [...plan.ranges];
    [ranges[index], ranges[target]] = [ranges[target], ranges[index]];
    return { ...plan, ranges };
  });
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
  return updatePlan(state, (plan) => ({ ...plan, captions: plan.ranges.flatMap((range) => captionsFromTranscript(transcript, range)) }));
}
