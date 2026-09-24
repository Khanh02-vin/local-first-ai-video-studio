export type OAuthState = { provider: "youtube"; state: string; codeVerifier: string; redirectUri: string; createdAt: number };
export interface OAuthProvider { authorize(state: OAuthState): string; callback(input: { code: string; state: OAuthState }): Promise<{ accessToken: string; refreshToken: string; expiresAt: number }>; refresh(refreshToken: string): Promise<{ accessToken: string; expiresAt: number }>; revoke(token: string): Promise<void>; }
export interface PublishProvider { publish(input: { accessToken: string; artifactUrl: string; title: string; description?: string; idempotencyKey: string; signal?: AbortSignal }): Promise<{ externalId: string; status: "published" }>; }

export function assertOAuthState(expected: OAuthState, received: string): void { if (expected.state !== received || Date.now() - expected.createdAt > 10 * 60_000) throw new Error("OAUTH_STATE_INVALID"); }
