import type { PublishTarget } from "../../packages/contracts/publish-target.ts";

export interface Publisher { publish(target: PublishTarget): Promise<PublishTarget>; }

export class ApprovalRequiredPublisher implements Publisher {
  async publish(target: PublishTarget): Promise<PublishTarget> {
    if (target.status !== "queued") throw new Error("PUBLISH_APPROVAL_REQUIRED");
    return { ...target, status: "published", externalId: `dry-run-${target.id}` };
  }
}
