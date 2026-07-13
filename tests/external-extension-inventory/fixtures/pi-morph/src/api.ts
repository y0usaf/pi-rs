import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { DEFAULT_API_KEY_PROVIDER, REQUEST_TIMEOUT_MS } from "./constants.js";
import type { MorphSettings } from "./types.js";
import { stripOuterCodeFence } from "./text.js";
import { isRecord } from "./utils.js";

export async function resolveApiKey(ctx: ExtensionContext, settings: MorphSettings): Promise<string | undefined> {
  const registry = ctx.modelRegistry as unknown as {
    getApiKeyForProvider?: (provider: string) => Promise<string | undefined>;
  };
  const fromRegistry = registry.getApiKeyForProvider
    ? await registry.getApiKeyForProvider(settings.apiKeyProvider).catch(() => undefined)
    : undefined;
  if (fromRegistry) return fromRegistry;

  if (settings.apiKeyProvider === DEFAULT_API_KEY_PROVIDER) return process.env.AI_GATEWAY_API_KEY;
  return process.env[settings.apiKeyProvider];
}

export function buildPrompt(filepath: string, originalCode: string, codeEdit: string, instructions: string): string {
  return [
    `<filepath>${filepath}</filepath>`,
    "",
    "<code>",
    originalCode,
    "</code>",
    "",
    "<update>",
    codeEdit,
    "</update>",
    "",
    "<instruction>",
    instructions,
    "",
    "Merge the update into the original file.",
    "Return only the complete merged file content.",
    "Do not return markdown fences, XML tags, explanations, or a diff.",
    "Preserve existing style and indentation.",
    "</instruction>",
  ].join("\n");
}

function extractAssistantText(payload: unknown): string {
  if (!isRecord(payload)) throw new Error("AI Gateway returned a non-object response.");
  const choices = payload.choices;
  if (!Array.isArray(choices) || choices.length === 0) throw new Error("AI Gateway returned no choices.");
  const first = choices[0];
  if (!isRecord(first)) throw new Error("AI Gateway returned a malformed choice.");
  const message = first.message;
  if (!isRecord(message)) throw new Error("AI Gateway returned a choice without a message.");
  const content = message.content;
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content.map((part) => (isRecord(part) && typeof part.text === "string" ? part.text : "")).join("");
  }
  throw new Error("AI Gateway returned a message without text content.");
}

export async function callAiGateway(settings: MorphSettings, apiKey: string, prompt: string, signal?: AbortSignal): Promise<string> {
  const timeout = AbortSignal.timeout(REQUEST_TIMEOUT_MS);
  const signals = signal ? [signal, timeout] : [timeout];

  const body: Record<string, unknown> = {
    model: settings.model,
    messages: [{ role: "user", content: prompt }],
    stream: false,
  };
  if (settings.provider !== undefined) body.provider = settings.provider;
  if (settings.providerOptions !== undefined) body.providerOptions = settings.providerOptions;

  const response = await fetch(`${settings.baseUrl}/chat/completions`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${apiKey}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
    signal: AbortSignal.any(signals),
  });

  const text = await response.text();
  if (!response.ok) {
    throw new Error(`AI Gateway request failed (${response.status} ${response.statusText}): ${text.slice(0, 1000)}`);
  }

  let json: unknown;
  try {
    json = JSON.parse(text) as unknown;
  } catch {
    throw new Error(`AI Gateway returned invalid JSON: ${text.slice(0, 500)}`);
  }

  return stripOuterCodeFence(extractAssistantText(json));
}
