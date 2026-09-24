import assert from "node:assert/strict";
import { loadConfig } from "../../services/api/config.ts";
import { assertProjectAccess } from "../../packages/contracts/cloud.ts";
import { MemoryRateLimiter } from "../../services/api/rate-limit.ts";
import { assertOAuthState } from "../../services/publishing/oauth.ts";

assert.throws(() => loadConfig({ NODE_ENV: "production" }), /PRODUCTION_CLOUD_CONFIG_REQUIRED/);
assert.doesNotThrow(() => assertProjectAccess({ userId: "u", tenantId: "t" }, "t", "p"));
assert.throws(() => assertProjectAccess({ userId: "u", tenantId: "t" }, "other", "p"), /TENANT_ACCESS_DENIED/);
const limiter = new MemoryRateLimiter();
assert.equal((await limiter.allow("u", 1, 60_000)).allowed, true);
assert.equal((await limiter.allow("u", 1, 60_000)).allowed, false);
const state = { provider: "youtube" as const, state: "s", codeVerifier: "v", redirectUri: "https://example.test/callback", createdAt: Date.now() };
assert.doesNotThrow(() => assertOAuthState(state, "s"));
assert.throws(() => assertOAuthState(state, "wrong"), /OAUTH_STATE_INVALID/);
console.log("platform boundary tests: ok");
