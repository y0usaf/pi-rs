import { getMarkdownTheme, type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Box, Markdown, Spacer, Text } from "@earendil-works/pi-tui";

import { RLM_FINAL_OUTPUT_CUSTOM_TYPE } from "./constants.js";
import { isRlmReplToolName, textOf } from "./utils.js";

export type PendingFinalOutput = {
  text: string;
  variableName?: string;
  toolCallId?: string;
  timestamp: number;
};

const pendingFinalOutputs: PendingFinalOutput[] = [];
let finalOutputFlushScheduled = false;

function textFromCustomContent(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part: any) => {
      if (part?.type === "text" && typeof part.text === "string") return part.text;
      if (part?.type === "image") return "[image]";
      return "";
    })
    .filter(Boolean)
    .join("\n");
}

function hasOwn(obj: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(obj, key);
}

function formatStructuredFinalValue(value: unknown): string {
  if (typeof value === "string") return value.trim();
  try {
    return (JSON.stringify(value, null, 2) ?? String(value)).trim();
  } catch {
    return String(value).trim();
  }
}

function rlmFinalText(message: any): string {
  const details = message?.details;
  if (details && typeof details === "object") {
    if (typeof details.finalText === "string") return details.finalText.trim();
    if (hasOwn(details, "finalValue")) return formatStructuredFinalValue(details.finalValue);
  }

  // Legacy fallback for pre-variable-only pi-rlm tool results.
  const text = textOf(message?.content).trim();
  const match = text.match(/(?:^|\n)FINAL:\s*\n?([\s\S]*)$/);
  return (match?.[1] ?? text).trim();
}

function rlmFinalVariableName(message: any): string | undefined {
  const details = message?.details;
  if (!details || typeof details !== "object") return undefined;
  const name = typeof details.finalVar === "string" ? details.finalVar : typeof details.finalName === "string" ? details.finalName : undefined;
  return name?.trim() || undefined;
}

export function collectRlmFinalOutputs(messages: any[]): PendingFinalOutput[] {
  const outputs: PendingFinalOutput[] = [];
  for (const message of messages) {
    if (message?.role !== "toolResult") continue;
    if (!isRlmReplToolName(message.toolName)) continue;
    if (message.details?.final !== true) continue;
    if (message.details?.finalMirrored === true) continue;
    const text = rlmFinalText(message);
    if (!text) continue;
    outputs.push({
      text,
      variableName: rlmFinalVariableName(message),
      toolCallId: typeof message.toolCallId === "string" ? message.toolCallId : undefined,
      timestamp: Date.now(),
    });
  }
  return outputs;
}

export function emitRlmFinalOutput(pi: ExtensionAPI, output: PendingFinalOutput): void {
  pi.sendMessage({
    customType: RLM_FINAL_OUTPUT_CUSTOM_TYPE,
    content: output.text,
    display: true,
    details: {
      toolName: "rlm_final",
      variableName: output.variableName,
      toolCallId: output.toolCallId,
      emittedAt: output.timestamp,
    },
  }, { triggerTurn: false });
}

export function registerRlmFinalOutputRenderer(pi: ExtensionAPI): void {
  pi.registerMessageRenderer(RLM_FINAL_OUTPUT_CUSTOM_TYPE, (message, _options, theme) => {
    const text = textFromCustomContent(message.content).trim();
    if (!text) return undefined;

    const variableName = typeof (message.details as any)?.variableName === "string" && (message.details as any).variableName.trim()
      ? (message.details as any).variableName.trim()
      : undefined;
    const label = variableName ? `rlm_final:${variableName}` : "rlm_final";

    // Use the custom-message palette so the variable final output stands apart
    // from tool-success styling and matches VCC-style custom messages.
    const box = new Box(1, 1, (value) => theme.bg("customMessageBg", value));
    box.addChild(new Text(
      `${theme.fg("customMessageLabel", "✓")} ${theme.fg("customMessageLabel", theme.bold(label))}`,
      0,
      0,
    ));
    box.addChild(new Spacer(1));
    box.addChild(new Markdown(text, 0, 0, getMarkdownTheme(), {
      color: (value) => theme.fg("customMessageText", value),
    }));
    return box;
  });
}

function scheduleFinalOutputFlush(pi: ExtensionAPI, ctx: ExtensionContext): void {
  if (finalOutputFlushScheduled) return;
  finalOutputFlushScheduled = true;

  setTimeout(() => {
    finalOutputFlushScheduled = false;
    if (pendingFinalOutputs.length === 0) return;

    let idle = false;
    try {
      idle = ctx.isIdle();
    } catch {
      pendingFinalOutputs.length = 0;
      return;
    }

    if (!idle) {
      scheduleFinalOutputFlush(pi, ctx);
      return;
    }

    const outputs = pendingFinalOutputs.splice(0);
    for (const output of outputs) {
      try {
        emitRlmFinalOutput(pi, output);
      } catch {
        // Session may have been replaced or shut down before the deferred UI mirror ran.
      }
    }
  }, 0);
}

export function enqueueRlmFinalOutputs(pi: ExtensionAPI, ctx: ExtensionContext, messages: any[]): void {
  const outputs = collectRlmFinalOutputs(messages);
  if (outputs.length === 0) return;
  pendingFinalOutputs.push(...outputs);
  scheduleFinalOutputFlush(pi, ctx);
}
