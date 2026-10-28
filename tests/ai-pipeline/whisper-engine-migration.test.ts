import assert from "node:assert/strict";
import { resolveWhisperCommand, resolveWhisperModelPath } from "../../adapters/local/whisper-engine.ts";

assert.equal(resolveWhisperCommand("cpp", "/home/user/.cache/app/.venv/bin/whisper", "/bundle/whisper-cli"), "/bundle/whisper-cli");
assert.equal(resolveWhisperCommand("cpp", "python", "/bundle/whisper-cli"), "/bundle/whisper-cli");
assert.equal(resolveWhisperCommand("cpp", "/cache/models/small.pt", "/bundle/whisper-cli"), "/bundle/whisper-cli");
assert.equal(resolveWhisperCommand("cpp", "/bundle/whisper-cpp/whisper-cli", "/fallback"), "/bundle/whisper-cpp/whisper-cli");
assert.equal(resolveWhisperCommand("cpp", undefined, "/bundle/whisper-cli"), "/bundle/whisper-cli");
assert.equal(resolveWhisperCommand("python", "/home/user/.venv/bin/whisper", "/bundle/whisper-cli"), "/home/user/.venv/bin/whisper");
assert.equal(resolveWhisperModelPath("cpp", "/cache/whisper/small.pt", "/bundle/ggml-tiny.bin"), "/bundle/ggml-tiny.bin");
assert.equal(resolveWhisperModelPath("cpp", undefined, "/bundle/ggml-tiny.bin"), "/bundle/ggml-tiny.bin");
assert.equal(resolveWhisperModelPath("cpp", "/bundle/ggml-small.bin", "/bundle/ggml-tiny.bin"), "/bundle/ggml-small.bin");
assert.equal(resolveWhisperModelPath("python", "/cache/whisper/small.pt", "/bundle/ggml-tiny.bin"), "/cache/whisper/small.pt");
console.log("whisper engine migration tests: ok");
