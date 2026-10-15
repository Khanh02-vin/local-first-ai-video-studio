import { readFileSync } from "node:fs";
import { VARIANT_PROMPT_VERSION, type EditVariant } from "../services/ai-pipeline/variants.ts";
import type { Transcript } from "../packages/contracts/transcript.ts";

const [inputPath, strategy, llamaUrl, llamaModel] = process.argv.slice(2);
if (!inputPath || !strategy) throw new Error("VARIANT_GENERATION_ARGUMENTS");
const transcript = JSON.parse(readFileSync(inputPath, "utf8")) as Transcript;
if (strategy !== "semantic-gemini" && strategy !== "semantic-local") throw new Error("VARIANT_AI_STRATEGY_REQUIRED");
const provider = strategy === "semantic-gemini"
  ? await import("../services/ai-pipeline/variants.ts").then(({ GeminiTranscriptVariantProvider }) => new GeminiTranscriptVariantProvider(process.env.GEMINI_API_KEY ?? ""))
  : await import("../services/ai-pipeline/variants.ts").then(({ LlamaCppTranscriptVariantProvider }) => new LlamaCppTranscriptVariantProvider(llamaUrl, llamaModel));
const generated = await provider.generate({ transcript });
if (!generated.length) throw new Error("LLM_VARIANTS_EMPTY");
const checked: EditVariant[] = generated;
process.stdout.write(JSON.stringify({ promptVersion: VARIANT_PROMPT_VERSION, variants: checked }));
