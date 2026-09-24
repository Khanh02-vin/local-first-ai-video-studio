import { randomUUID } from "node:crypto";
import type { Job } from "../../packages/contracts/job.ts";
import type { CloudJobStore } from "../../adapters/cloud/interfaces.ts";
import { MemoryCloudJobStore } from "../../adapters/cloud/index.ts";

export class ApiService {
  private readonly jobs: CloudJobStore;
  constructor(jobs: CloudJobStore = new MemoryCloudJobStore()) { this.jobs = jobs; }
  async createRenderJob(inputArtifactIds: string[], executionTarget: Job["executionTarget"] = "cloud", idempotencyKey?: string): Promise<Job> {
    const now = new Date().toISOString();
    const job: Job = { version: 1, id: randomUUID(), type: "render", state: "created", executionTarget, inputArtifactIds, outputArtifactIds: [], idempotencyKey: idempotencyKey ?? randomUUID(), retryCount: 0, createdAt: now, updatedAt: now };
    return this.jobs.create(job);
  }
  getJob(id: string): Promise<Job | undefined> { return this.jobs.get(id); }
}
