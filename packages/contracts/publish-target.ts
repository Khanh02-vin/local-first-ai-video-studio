import { CONTRACT_VERSION } from "./transcript.ts";

export const PUBLISH_PLATFORMS = ["youtube", "youtube-shorts", "tiktok", "instagram-reels"] as const;
export type PublishPlatform = (typeof PUBLISH_PLATFORMS)[number];
export type PublishStatus = "draft" | "queued" | "publishing" | "published" | "failed" | "cancelled";

export type PublishTarget = {
  version: typeof CONTRACT_VERSION;
  id: string;
  artifactId: string;
  platform: PublishPlatform;
  accountId: string;
  status: PublishStatus;
  scheduledAt?: string;
  externalId?: string;
  error?: { code: string; message: string; retryable: boolean };
};

export function validatePublishTarget(target: PublishTarget): void {
  if (target.version !== CONTRACT_VERSION || !target.id || !target.artifactId || !target.accountId) {
    throw new Error("Invalid publish target metadata");
  }
  if (!PUBLISH_PLATFORMS.includes(target.platform)) throw new Error("Unsupported publish platform");
  if (!["draft", "queued", "publishing", "published", "failed", "cancelled"].includes(target.status)) {
    throw new Error("Invalid publish status");
  }
}
