export interface RateLimiter { allow(key: string, limit: number, windowMs: number): Promise<{ allowed: boolean; retryAfterSeconds: number }>; }

export class MemoryRateLimiter implements RateLimiter {
  private readonly counters = new Map<string, { count: number; resetAt: number }>();
  async allow(key: string, limit: number, windowMs: number): Promise<{ allowed: boolean; retryAfterSeconds: number }> {
    const now = Date.now(); const current = this.counters.get(key);
    const bucket = !current || current.resetAt <= now ? { count: 0, resetAt: now + windowMs } : current;
    bucket.count += 1; this.counters.set(key, bucket);
    return { allowed: bucket.count <= limit, retryAfterSeconds: Math.ceil(Math.max(0, bucket.resetAt - now) / 1000) };
  }
}
