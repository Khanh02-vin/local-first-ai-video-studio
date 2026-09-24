import type { Job } from "../../packages/contracts/job.ts";

export type CloudJobStore = { create(job: Job): Promise<Job>; get(id: string): Promise<Job | undefined>; update(id: string, patch: Partial<Job>): Promise<Job | undefined> };

// The cloud adapter is deliberately storage-agnostic. PostgreSQL/Redis/S3 implementations belong here later;
// API and media core depend only on this contract.
export class MemoryCloudJobStore implements CloudJobStore {
  private readonly jobs = new Map<string, Job>();
  async create(job: Job): Promise<Job> { this.jobs.set(job.id, structuredClone(job)); return structuredClone(job); }
  async get(id: string): Promise<Job | undefined> { const job = this.jobs.get(id); return job && structuredClone(job); }
  async update(id: string, patch: Partial<Job>): Promise<Job | undefined> { const current = this.jobs.get(id); if (!current) return; const updated = { ...current, ...patch, updatedAt: new Date().toISOString() }; this.jobs.set(id, updated); return structuredClone(updated); }
}
