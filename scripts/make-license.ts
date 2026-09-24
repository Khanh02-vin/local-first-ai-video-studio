import { readFileSync, writeFileSync } from "node:fs";
import { createPrivateKey, sign } from "node:crypto";
import { join } from "node:path";

// Signs an offline license file.
// Usage: node --experimental-strip-types scripts/make-license.ts <licensee> <tier> [days]
//   LICENSE_PRIVATE_KEY=<path to keys/license-private.pem> (default keys/license-private.pem)
// Writes <state-dir>/license.lic by default; override with LICENSE_OUT.
const [licensee, tier, daysText] = process.argv.slice(2);
if (!licensee || !["free", "pro"].includes(tier ?? "")) { console.error("usage: make-license.ts <licensee> <free|pro> [days]"); process.exit(2); }
const days = Number(daysText ?? 365);
const privatePath = process.env.LICENSE_PRIVATE_KEY ?? join(process.cwd(), "keys", "license-private.pem");
const privateKey = createPrivateKey(readFileSync(privatePath));
const payload = JSON.stringify({ licensee, tier, issuedAt: new Date().toISOString(), expiresAt: new Date(Date.now() + days * 86_400_000).toISOString() });
const signature = sign(null, Buffer.from(payload, "utf8"), privateKey);
const out = process.env.LICENSE_OUT ?? join(process.env.HOME ?? ".", ".cache", "local-first-ai-video-studio", "license.lic");
const content = Buffer.from(payload, "utf8").toString("base64") + "\n" + signature.toString("base64") + "\n";
writeFileSync(out, content, { mode: 0o600 });
console.log("license written:", out);
console.log("payload:", payload);