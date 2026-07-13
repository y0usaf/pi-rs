import { EXISTING_CODE_MARKER, MORPH_ROUTING_HINT_HEADER } from "./constants.js";
import type { MorphSettings } from "./types.js";
import { isRecord } from "./utils.js";

export function buildMorphRoutingHint(settings: MorphSettings, apiKeyAvailable: boolean): string {
  if (!settings.enabled) {
    return [
      MORPH_ROUTING_HINT_HEADER,
      "- pi-morph is disabled by extensionSettings.morph.enabled=false; do not call morph_edit.",
      "- Use edit for exact existing-file changes and write for new files/full rewrites.",
    ].join("\n");
  }

  if (!apiKeyAvailable) {
    return [
      MORPH_ROUTING_HINT_HEADER,
      `- morph_edit is unavailable because no API key was found for ${settings.apiKeyProvider}; do not call morph_edit unless credentials become available.`,
      "- Use edit for exact existing-file changes and write for new files/full rewrites.",
    ].join("\n");
  }

  return [
    MORPH_ROUTING_HINT_HEADER,
    "- Use morph_edit for large existing files, multiple scattered edits, whitespace-sensitive edits, repetitive changes, or ambiguous/structural rewrites inside one existing file.",
    "- Use edit for small exact anchor-based replacements, single-line/few-line changes, and deterministic patches.",
    "- Use write for new files or intentional full-file rewrites.",
    `- morph_edit requires ${JSON.stringify(EXISTING_CODE_MARKER)} markers around unchanged sections, ideally with 1-2 unique context lines around each changed region.`,
    "- If morph_edit fails, retry with more concrete context or fall back to edit/write.",
  ].join("\n");
}

function appendTextContent(content: unknown, hint: string): unknown {
  if (typeof content === "string") {
    if (content.includes(MORPH_ROUTING_HINT_HEADER)) return content;
    return `${content}\n\n${hint}`;
  }

  if (Array.isArray(content)) {
    const serialized = JSON.stringify(content);
    if (serialized.includes(MORPH_ROUTING_HINT_HEADER)) return content;
    return [...content, { type: "text", text: hint }];
  }

  return content;
}

export function appendMorphRoutingHint(payload: unknown, hint: string): unknown {
  if (!isRecord(payload)) return undefined;

  if (typeof payload.system === "string" || Array.isArray(payload.system)) {
    payload.system = appendTextContent(payload.system, hint);
    return payload;
  }

  const messages = payload.messages;
  if (!Array.isArray(messages)) return undefined;

  const systemMessage = messages.find((message) => isRecord(message) && message.role === "system");
  if (isRecord(systemMessage)) {
    systemMessage.content = appendTextContent(systemMessage.content, hint);
    return payload;
  }

  messages.unshift({ role: "system", content: hint });
  return payload;
}
