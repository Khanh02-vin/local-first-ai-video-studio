import { CONTRACT_VERSION } from "./transcript.ts";

export const JOB_STATES = ["created", "queued", "processing", "review", "rendering", "completed", "failed", "cancelled", "retrying"] as const;
export type JobState = (typeof JOB_STATES)[number];
export type ExecutionTarget = "local" | "cloud";
export type JobType = "transcription" | "highlight" | "render" | "publish";

const TRANSITIONS: Record<JobState, readonly JobState[]> = {
  created: ["queued", "cancelled"],
  queued: ["processing", "cancelled"],
  processing: ["review", "rendering", "failed", "cancelled"],
  review: ["rendering", "cancelled"],
  rendering: ["completed", "failed", "cancelled"],
  completed: [],
  failed: ["retrying"],
  cancelled: [],
  retrying: ["queued", "failed"],
};

export type Job = {
  version: typeof CONTRACT_VERSION;
  id: string;
  type: JobType;
  state: JobState;
  executionTarget: ExecutionTarget;
  inputArtifactIds: string[];
  outputArtifactIds: string[];
  idempotencyKey: string;
  retryCount: number;
  providerUsage?: { provider: string; units?: number; estimatedCost?: number };
  error?: { code: string; message: string; retryable: boolean };
  createdAt: string;
  updatedAt: string;
};

export function canTransition(from: JobState, to: JobState): boolean {
  return TRANSITIONS[from].includes(to);
}

export function assertTransition(from: JobState, to: JobState): void {
  if (!canTransition(from, to)) throw new Error(`Invalid job transition: ${from} -> ${to}`);
}

export function validateJob(job: Job): void {
  if (job.version !== CONTRACT_VERSION || !job.id || !job.idempotencyKey) throw new Error("Invalid job metadata");
  if (!JOB_STATES.includes(job.state) || !["local", "cloud"].includes(job.executionTarget)) throw new Error("Invalid job state or target");
  if (!Number.isInteger(job.retryCount) || job.retryCount < 0) throw new Error("Invalid retry count");
  if (!job.createdAt || !job.updatedAt) throw new Error("Invalid job timestamps");
}
