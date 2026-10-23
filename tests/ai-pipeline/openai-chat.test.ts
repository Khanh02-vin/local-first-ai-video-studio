import assert from "node:assert/strict";
import type { Transcript } from "../../packages/contracts/transcript.ts";
import { OpenAIChatHighlightProvider } from "../../services/ai-pipeline/providers.ts";

/**
 * OpenAI-compatible chat highlight provider: POST {baseUrl}/chat/completions
 * with a bearer key, a configurable model, and a json_object response format.
 * Shared helpers (windowing, merge, proposal mapping, truncated-JSON repair,
 * circuit breaker) are reused from providers.ts — the same arrangement the
 * local llama.cpp provider already uses.
 */

const makeTranscript = (duration = 60): Transcript => ({
  version: 1,
  sourceArtifactId: "src",
  language: "en",
  duration,
  provider: "test",
  words: [
    { id: "w0", text: "This", start: 1, end: 1.4 },
    { id: "w1", text: "is", start: 1.4, end: 1.7 },
    { id: "w2", text: "a", start: 1.7, end: 1.9 },
    { id: "w3", text: "useful", start: 1.9, end: 2.4 },
    { id: "w4", text: "idea", start: 2.4, end: 3 },
    { id: "w5", text: "today", start: 10, end: 10.5 },
  ],
  segments: [],
});

const cannedBody = JSON.stringify({
  highlights: [
    { from: 0, to: 2, title: "Intro", score: 90 },
    { from: 3, to: 5, title: "Outro", score: 70 },
  ],
});

type Captured = { url: string; init: RequestInit };
function stubFetch(handler: (captured: Captured) => Response | Promise<Response>): Captured[] {
  const captured: Captured[] = [];
  globalThis.fetch = (async (url: unknown, init?: RequestInit) => {
    const entry = { url: String(url), init: init ?? {} };
    captured.push(entry);
    return handler(entry);
  }) as typeof fetch;
  return captured;
}

const okChoices = (content: string) =>
  new Response(JSON.stringify({ choices: [{ message: { role: "assistant", content } }] }), { status: 200 });

// 1. Request shape: URL, bearer auth, model, json response_format.
{
  const captured = stubFetch(() => okChoices(cannedBody));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1", "test-key", "my-model");
  const highlights = await provider.choose({ transcript: makeTranscript() });
  assert.equal(highlights.length, 2);
  assert.equal(highlights[0].title, "Intro");
  assert.equal(highlights[0].start, 1);
  assert.equal(highlights[1].end, 10.5);
  assert.equal(captured.length, 1);
  assert.equal(captured[0].url, "https://api.example.com/v1/chat/completions");
  const sent = JSON.parse(String(captured[0].init.body)) as { model: string; messages: Array<{ role: string }>; response_format: { type: string } };
  assert.equal(sent.model, "my-model");
  assert.equal(sent.messages[0].role, "user");
  assert.equal(sent.response_format.type, "json_object");
  const headers = new Headers(captured[0].init.headers);
  assert.equal(headers.get("authorization"), "Bearer test-key");
}

// 2. Trailing slashes on the base URL are trimmed.
{
  const captured = stubFetch(() => okChoices(cannedBody));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1///", "k", "m");
  await provider.choose({ transcript: makeTranscript() });
  assert.equal(captured[0].url, "https://api.example.com/v1/chat/completions");
}

// 3. Empty key omits the Authorization header (some gateways use IP allowlists).
{
  const captured = stubFetch(() => okChoices(cannedBody));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1", "", "m");
  await provider.choose({ transcript: makeTranscript() });
  const headers = new Headers(captured[0].init.headers);
  assert.equal(headers.get("authorization"), null);
}

// 4. Markdown-fenced JSON content parses.
{
  stubFetch(() => okChoices(`\`\`\`json\n${cannedBody}\n\`\`\``));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1", "k", "m");
  const highlights = await provider.choose({ transcript: makeTranscript() });
  assert.equal(highlights.length, 2);
}

// 5. Truncated JSON (cut off mid-title) is repaired like the llama provider does.
{
  const truncated = cannedBody.slice(0, cannedBody.indexOf('"Intro"') + 5);
  stubFetch(() => okChoices(truncated));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1", "k", "m");
  const highlights = await provider.choose({ transcript: makeTranscript() });
  assert.equal(highlights.length, 1);
  assert.equal(highlights[0].title, "Intr");
}

// 6. No word timings throws instead of sending an empty prompt.
{
  stubFetch(() => okChoices(cannedBody));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1", "k", "m");
  const empty: Transcript = { ...makeTranscript(), words: [] };
  await assert.rejects(() => provider.choose({ transcript: empty }), /SEMANTIC_REQUIRES_WORD_TIMESTAMPS/);
}

// 7. Non-OK status surfaces the status code.
{
  stubFetch(() => new Response("nope", { status: 500 }));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1", "k", "m");
  await assert.rejects(() => provider.choose({ transcript: makeTranscript() }), /OPENAI_CHAT_PROVIDER_500/);
}

// 8. Repeated failures open the circuit breaker.
{
  stubFetch(() => new Response("down", { status: 503 }));
  const provider = new OpenAIChatHighlightProvider("https://api.example.com/v1", "k", "m");
  for (let i = 0; i < 5; i++) await assert.rejects(() => provider.choose({ transcript: makeTranscript() }), /OPENAI_CHAT_PROVIDER_503/);
  await assert.rejects(() => provider.choose({ transcript: makeTranscript() }), /CIRCUIT_BREAKER_OPEN/);
}

console.log("openai-chat provider tests passed");
