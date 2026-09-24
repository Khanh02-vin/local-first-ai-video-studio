import { generateKeyPairSync, createPublicKey } from "node:crypto";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

// Generates an Ed25519 keypair for offline licensing.
//   - private key: saved next to keys/license-private.pem (KEEP SECRET, never ship)
//   - public key : base64 (32 bytes) -> paste into LICENSE_PUBLIC_KEY_B64 in main.rs
const dir = join(dirname(new URL(import.meta.url).pathname), "..", "keys");
const { publicKey, privateKey } = generateKeyPairSync("ed25519");
const privatePem = privateKey.export({ type: "pkcs8", format: "pem" });
const publicPem = publicKey.export({ type: "spki", format: "pem" });
writeFileSync(join(dir, "license-private.pem"), privatePem, { mode: 0o600 });
writeFileSync(join(dir, "license-public.pem"), publicPem);
const raw = createPublicKey(publicPem).export({ type: "spki", format: "der" }).subarray(-32);
console.log("private key :", join(dir, "license-private.pem"));
console.log("public PEM  :", join(dir, "license-public.pem"));
console.log("PUBLIC_B64  :", raw.toString("base64"));