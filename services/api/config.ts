export type AppConfig = { nodeEnv: "development" | "test" | "production"; maxUploadBytes: number; signedUrlTtlSeconds: number; rateLimitPerMinute: number; databaseUrl?: string; redisUrl?: string; s3Endpoint?: string; s3Bucket?: string; geminiApiKey?: string; openAiApiKey?: string };

export function loadConfig(env: NodeJS.ProcessEnv = process.env): AppConfig {
  const nodeEnv = (env.NODE_ENV ?? "development") as AppConfig["nodeEnv"];
  const config: AppConfig = { nodeEnv, maxUploadBytes: Number(env.MAX_UPLOAD_BYTES ?? 2_000_000_000), signedUrlTtlSeconds: Math.min(86_400, Number(env.SIGNED_URL_TTL_SECONDS ?? 900)), rateLimitPerMinute: Number(env.RATE_LIMIT_PER_MINUTE ?? 60), databaseUrl: env.DATABASE_URL, redisUrl: env.REDIS_URL, s3Endpoint: env.S3_ENDPOINT, s3Bucket: env.S3_BUCKET, geminiApiKey: env.GEMINI_API_KEY, openAiApiKey: env.OPENAI_API_KEY };
  if (!Number.isFinite(config.maxUploadBytes) || config.maxUploadBytes <= 0 || !Number.isFinite(config.rateLimitPerMinute) || config.rateLimitPerMinute <= 0) throw new Error("INVALID_RUNTIME_CONFIG");
  if (nodeEnv === "production" && (!config.databaseUrl || !config.redisUrl || !config.s3Bucket)) throw new Error("PRODUCTION_CLOUD_CONFIG_REQUIRED");
  return config;
}
