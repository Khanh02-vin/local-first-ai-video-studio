import type { Artifact } from "../../packages/contracts/artifact.ts";
import type { Job } from "../../packages/contracts/job.ts";
import type { SignedUrl, UploadDescriptor } from "../../packages/contracts/cloud.ts";

export interface CloudJobStore { create(job: Job): Promise<Job>; get(id: string): Promise<Job | undefined>; update(id: string, patch: Partial<Job>): Promise<Job | undefined>; }
export interface ObjectStorage { initiateMultipart(input: UploadDescriptor): Promise<{ uploadId: string; key: string }>; presignPart(key: string, uploadId: string, partNumber: number): Promise<SignedUrl>; complete(key: string, uploadId: string, parts: Array<{ partNumber: number; etag: string }>): Promise<Artifact>; presignedDownload(artifact: Artifact): Promise<SignedUrl>; delete(artifact: Artifact): Promise<void>; }
export interface QueuePublisher { enqueue(job: Job): Promise<void>; }
