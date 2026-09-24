import { CONTRACT_VERSION } from "./transcript.ts";

export type Artifact = {
  version: typeof CONTRACT_VERSION;
  id: string;
  artifactVersion: number;
  mediaType: "video" | "audio" | "transcript" | "thumbnail" | "metadata";
  storageLocator: string;
  checksum: string;
  duration?: number;
  width?: number;
  height?: number;
  codec?: string;
  ownerId: string;
  projectId: string;
  retentionUntil?: string;
};

export function validateArtifact(artifact: Artifact): void {
  if (artifact.version !== CONTRACT_VERSION || !artifact.id || !artifact.storageLocator || !artifact.ownerId || !artifact.projectId) {
    throw new Error("Invalid artifact metadata");
  }
  if (!Number.isInteger(artifact.artifactVersion) || artifact.artifactVersion < 1) throw new Error("Invalid artifact version");
  if (!artifact.checksum || !/^[a-f0-9]{32,128}$/i.test(artifact.checksum)) throw new Error("Artifact checksum is required");
  if (artifact.duration !== undefined && (!Number.isFinite(artifact.duration) || artifact.duration < 0)) throw new Error("Invalid artifact duration");
  if (artifact.width !== undefined && (!Number.isInteger(artifact.width) || artifact.width <= 0)) throw new Error("Invalid artifact width");
  if (artifact.height !== undefined && (!Number.isInteger(artifact.height) || artifact.height <= 0)) throw new Error("Invalid artifact height");
}
