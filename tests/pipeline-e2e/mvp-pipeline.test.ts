import assert from "node:assert/strict";
import { analyzeVideo } from "../../services/ai-pipeline/index.ts";
import { createEditorState, updateAspectRatio } from "../../packages/editor-core/index.ts";
import { validateRenderPlan } from "../../packages/contracts/render-plan.ts";
import { ApprovalRequiredPublisher } from "../../services/publishing/index.ts";

const analysis = analyzeVideo("source-1", 60);
assert.equal(analysis.highlights.length, 1);
let editor = createEditorState("source-1", 60, analysis.highlights[0]);
editor = updateAspectRatio(editor, "9:16");
validateRenderPlan(editor.plan, 60);
const publisher = new ApprovalRequiredPublisher();
await assert.rejects(() => publisher.publish({ version: 1, id: "p1", artifactId: "a1", platform: "youtube-shorts", accountId: "account", status: "draft" }));
const published = await publisher.publish({ version: 1, id: "p1", artifactId: "a1", platform: "youtube-shorts", accountId: "account", status: "queued" });
assert.equal(published.status, "published");
console.log("MVP pipeline tests: ok");
