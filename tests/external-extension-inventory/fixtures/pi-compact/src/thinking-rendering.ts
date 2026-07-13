import { AssistantMessageComponent } from "@earendil-works/pi-coding-agent";
import { state, thinkingTimings } from "./state.js";
import { ASSISTANT_ORIGINAL_RENDER_KEY, ASSISTANT_ORIGINAL_UPDATE_CONTENT_KEY, ASSISTANT_THINKING_APPLIED_MODE_KEY, ASSISTANT_THINKING_STATE_KEY, ASSISTANT_THINKING_TIMING_KEY, THINKING_MARKER, type CompactThinkingState, type CompactThinkingTiming, type ThinkingMode } from "./types.js";
import { clip, getThemeToolBgFn, isCompactThinkingTiming, isRecord, renderOneLine, stripAnsi } from "./shared.js";

export function assistantThinkingTimingKeys(message: any): string[] {
  if (message?.role !== "assistant") return [];

  const api = typeof message.api === "string" ? message.api : "";
  const provider = typeof message.provider === "string" ? message.provider : "";
  const model = typeof message.model === "string" ? message.model : "";
  const keys: string[] = [];

  if (typeof message.timestamp === "number" && Number.isFinite(message.timestamp)) {
    keys.push(`${api}:${provider}:${model}:ts:${message.timestamp}`);
  }
  if (typeof message.responseId === "string" && message.responseId.length > 0) {
    keys.push(`${api}:${provider}:${model}:response:${message.responseId}`);
  }

  return [...new Set(keys)];
}

export function attachThinkingTiming(message: any, timing: CompactThinkingTiming): void {
  if (!isRecord(message)) return;

  try {
    Object.defineProperty(message, ASSISTANT_THINKING_TIMING_KEY, {
      value: timing,
      enumerable: false,
      configurable: true,
      writable: true,
    });
  } catch {
    try {
      message[ASSISTANT_THINKING_TIMING_KEY] = timing;
    } catch {
      // Ignore non-extensible message objects.
    }
  }
}

export function storeThinkingTiming(message: any, timing: CompactThinkingTiming): void {
  for (const key of assistantThinkingTimingKeys(message)) {
    thinkingTimings.set(key, timing);
  }
  attachThinkingTiming(message, timing);
}

export function getThinkingTiming(message: any): CompactThinkingTiming | undefined {
  const attached = message?.[ASSISTANT_THINKING_TIMING_KEY];
  if (isCompactThinkingTiming(attached)) return attached;

  for (const key of assistantThinkingTimingKeys(message)) {
    const timing = thinkingTimings.get(key);
    if (timing) return timing;
  }

  return undefined;
}

export function recordAssistantThinkingTiming(message: any, assistantEvent?: any, final = false): void {
  if (message?.role !== "assistant") return;

  const eventType = assistantEvent?.type;
  const startsThinking = eventType === "start" || eventType === "thinking_start" || eventType === "thinking_delta";
  let timing = getThinkingTiming(message);
  if (!timing) {
    if (!startsThinking) return;
    timing = {};
  }

  const now = Date.now();
  if (eventType === "start") {
    timing.startedAtMs ??= now;
    timing.completedAtMs = undefined;
  } else if (eventType === "thinking_start") {
    timing.startedAtMs ??= now;
    timing.completedAtMs = undefined;
  } else if (eventType === "thinking_delta") {
    timing.startedAtMs ??= now;
  } else if (eventType === "thinking_end") {
    timing.startedAtMs ??= now;
  }

  const completesThinking =
    final ||
    eventType === "thinking_end" ||
    eventType === "text_start" ||
    eventType === "text_delta" ||
    eventType === "text_end" ||
    eventType === "toolcall_start" ||
    eventType === "toolcall_delta" ||
    eventType === "toolcall_end" ||
    eventType === "done" ||
    eventType === "error";
  if (completesThinking && timing.startedAtMs !== undefined && timing.completedAtMs === undefined) {
    timing.completedAtMs = now;
  }

  if (timing.startedAtMs !== undefined) storeThinkingTiming(message, timing);
}

export function recordAssistantThinkingTimingForEvent(event: any, final = false): void {
  const assistantEvent = event?.assistantMessageEvent;
  recordAssistantThinkingTiming(event?.message, assistantEvent, final);

  const eventMessage = assistantEvent?.partial ?? assistantEvent?.message ?? assistantEvent?.error;
  if (eventMessage && eventMessage !== event?.message) recordAssistantThinkingTiming(eventMessage, assistantEvent, final);
}

export function getThinkingBlocks(message: any): string[] {
  if (!Array.isArray(message?.content)) return [];
  return message.content
    .filter((content: any) => content?.type === "thinking" && typeof content.thinking === "string")
    .map((content: any) => content.thinking.trim())
    .filter((thinking: string) => thinking.length > 0);
}

export function cloneWithContentFilter(message: any, predicate: (content: any, index: number) => boolean): any {
  if (!Array.isArray(message?.content)) return message;
  return {
    ...message,
    content: message.content.filter(predicate),
  };
}

export function cloneWithoutThinking(message: any): any {
  return cloneWithContentFilter(message, (content: any) => content?.type !== "thinking");
}

export function cloneWithoutPostToolCallText(message: any): any {
  if (!Array.isArray(message?.content)) return message;

  const firstToolCallIndex = message.content.findIndex((content: any) => content?.type === "toolCall");
  if (firstToolCallIndex < 0) return message;

  // Some providers can stream text after a tool call in the same assistant
  // message. Pi then continues after tool execution, so that post-tool text can
  // appear as a duplicate/provisional final answer. Keep pre-tool text, hide
  // text emitted after the first tool call from display only.
  return cloneWithContentFilter(message, (content: any, index: number) => {
    return !(index > firstToolCallIndex && content?.type === "text");
  });
}

export function cloneAssistantForDisplay(message: any, hideThinking: boolean): any {
  const withoutDuplicateText = cloneWithoutPostToolCallText(message);
  return hideThinking ? cloneWithoutThinking(withoutDuplicateText) : withoutDuplicateText;
}

export function isThinkingActive(message: any): boolean {
  if (message?.stopReason === "error" || message?.stopReason === "aborted" || !Array.isArray(message?.content)) return false;

  for (let index = message.content.length - 1; index >= 0; index--) {
    const content = message.content[index];
    if (content?.type === "thinking" && typeof content.thinking === "string") return true;
    if (content?.type === "text" || content?.type === "toolCall") return false;
  }

  return false;
}

export function createThinkingState(
  message: any,
  blocks: string[],
  previous?: CompactThinkingState,
  timing?: CompactThinkingTiming,
): CompactThinkingState {
  const text = blocks.join("\n");
  const now = Date.now();
  const active = timing?.completedAtMs === undefined && isThinkingActive(message);
  const state: CompactThinkingState = {
    charCount: stripAnsi(text).length,
    startedAtMs: timing?.startedAtMs ?? previous?.startedAtMs ?? now,
  };

  const completedAtMs = timing?.completedAtMs ?? (!active ? previous?.completedAtMs ?? now : undefined);
  if (completedAtMs !== undefined) state.completedAtMs = completedAtMs;
  if (message?.stopReason === "error" || message?.stopReason === "aborted") state.stopReason = message.stopReason;
  return state;
}

export function formatCount(value: number, singular: string, plural = `${singular}s`): string {
  return `${value} ${value === 1 ? singular : plural}`;
}

export function elapsedThinkingSeconds(state: CompactThinkingState): number {
  const end = state.completedAtMs ?? Date.now();
  return Math.max(0, (end - state.startedAtMs) / 1000);
}

export function formatElapsedSeconds(seconds: number): string {
  return `${seconds.toFixed(1)}s`;
}

export function buildThinkingLine(state: CompactThinkingState): string {
  const elapsed = formatElapsedSeconds(elapsedThinkingSeconds(state));
  const characters = formatCount(state.charCount, "char");
  const suffix = state.stopReason === "error" ? " → error" : state.stopReason === "aborted" ? " → aborted" : "";

  return `${THINKING_MARKER} ${elapsed} · ${characters}${suffix}`;
}

export function getThinkingBgFn(state: CompactThinkingState): ((text: string) => string) | undefined {
  if (state.stopReason === "error" || state.stopReason === "aborted") return getThemeToolBgFn("toolErrorBg");
  return getThemeToolBgFn(state.completedAtMs === undefined ? "toolPendingBg" : "toolSuccessBg");
}

export function renderCompactThinkingLine(compactState: CompactThinkingState, width: number): string[] {
  const line = state.activeTheme?.fg("muted", buildThinkingLine(compactState)) ?? buildThinkingLine(compactState);
  return renderOneLine(line, width, getThinkingBgFn(compactState), true);
}

// Honor Pi's Ctrl+T visibility toggle: when the core UI hides thinking blocks,
// pi-compact should suppress its compact thinking row too.
export function getAssistantThinkingMode(component: any): ThinkingMode {
  return component?.hideThinkingBlock ? "hidden" : state.thinkingMode;
}

export function patchAssistantMessageComponent(): boolean {
  try {
    const proto = (AssistantMessageComponent as any)?.prototype;
    if (!proto || typeof proto.render !== "function" || typeof proto.updateContent !== "function") {
      throw new Error("AssistantMessageComponent unavailable");
    }

    const originalRender = typeof proto[ASSISTANT_ORIGINAL_RENDER_KEY] === "function" ? proto[ASSISTANT_ORIGINAL_RENDER_KEY] : proto.render;
    const originalUpdateContent =
      typeof proto[ASSISTANT_ORIGINAL_UPDATE_CONTENT_KEY] === "function"
        ? proto[ASSISTANT_ORIGINAL_UPDATE_CONTENT_KEY]
        : proto.updateContent;

    proto.updateContent = function piCompactAssistantUpdateContent(this: any, message: any) {
      const thinkingMode = getAssistantThinkingMode(this);
      const blocks = getThinkingBlocks(message);
      const hideThinking = blocks.length > 0 && thinkingMode !== "normal";
      this[ASSISTANT_THINKING_APPLIED_MODE_KEY] = thinkingMode;
      this.lastMessage = message;

      if (!hideThinking) {
        this[ASSISTANT_THINKING_STATE_KEY] = undefined;
        return originalUpdateContent.call(this, cloneAssistantForDisplay(message, false));
      }

      this[ASSISTANT_THINKING_STATE_KEY] = createThinkingState(
        message,
        blocks,
        this[ASSISTANT_THINKING_STATE_KEY],
        getThinkingTiming(message),
      );

      return originalUpdateContent.call(this, cloneAssistantForDisplay(message, true));
    };

    proto.render = function piCompactAssistantRender(this: any, width: number) {
      const thinkingMode = getAssistantThinkingMode(this);
      if (this[ASSISTANT_THINKING_APPLIED_MODE_KEY] !== thinkingMode && this.lastMessage) {
        this.updateContent(this.lastMessage);
      }

      const lines = originalRender.call(this, width);
      const thinkingState = this[ASSISTANT_THINKING_STATE_KEY] as CompactThinkingState | undefined;
      if (thinkingMode !== "compact" || !thinkingState) return lines;

      const thinkingLines = renderCompactThinkingLine(thinkingState, width);
      return thinkingLines.length > 0 ? [...thinkingLines, ...lines] : lines;
    };

    proto[ASSISTANT_ORIGINAL_RENDER_KEY] = originalRender;
    proto[ASSISTANT_ORIGINAL_UPDATE_CONTENT_KEY] = originalUpdateContent;
    state.lastAssistantPatchError = undefined;
    return true;
  } catch (error) {
    state.lastAssistantPatchError = error instanceof Error ? error.stack ?? error.message : String(error);
    return false;
  }
}
