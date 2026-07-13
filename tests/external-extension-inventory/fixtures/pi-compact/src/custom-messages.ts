import { CustomMessageComponent, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { truncateToWidth, type Component } from "@earendil-works/pi-tui";
import { state } from "./state.js";
import { CUSTOM_ORIGINAL_RENDER_KEY, JANITOR_CUSTOM_TYPES, JANITOR_INDEX_CUSTOM_TYPE, JANITOR_MARKER, JANITOR_NOTICE_CUSTOM_TYPE, JANITOR_RESTORE_CUSTOM_TYPE, JANITOR_SUMMARY_CUSTOM_TYPE, MAX_SUMMARY_LENGTH } from "./types.js";
import { clip, getThemeToolBgFn, isBlankRenderedLine, isRecord, renderOneLine, replaceTabs, squash } from "./shared.js";

export type CustomThemeLike = {
  fg(color: string, text: string): string;
  bold?(text: string): string;
};

export type CustomMessageLike = {
  customType?: string;
  content?: unknown;
  details?: unknown;
};

export function themeFg(theme: CustomThemeLike, color: string, text: string): string {
  try {
    return theme.fg(color, text);
  } catch {
    return text;
  }
}

export function textFromMessageContent(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";

  const parts: string[] = [];
  for (const part of content) {
    if (!isRecord(part)) continue;
    if (part.type === "text" && typeof part.text === "string") parts.push(part.text);
    else if (part.type === "image") parts.push("[image]");
    else if (part.type === "thinking" && typeof part.thinking === "string") parts.push(part.thinking);
  }
  return parts.join("\n");
}

export function numberFromDetails(details: unknown, key: string): number | undefined {
  if (!isRecord(details)) return undefined;
  const value = details[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

export function arrayLengthFromDetails(details: unknown, key: string): number | undefined {
  if (!isRecord(details)) return undefined;
  const value = details[key];
  return Array.isArray(value) ? value.length : undefined;
}

export function plural(value: number, singular: string, pluralText = `${singular}s`): string {
  return `${value} ${value === 1 ? singular : pluralText}`;
}

export function formatCompactCount(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

export function formatCompactChars(value: number): string {
  return `${formatCompactCount(value)}ch`;
}

export function compactJanitorNoticeLine(message: CustomMessageLike): string {
  const details = message.details;
  const rawText = squash(textFromMessageContent(message.content));
  const rawChars = numberFromDetails(details, "rawChars");
  const projectedChars = numberFromDetails(details, "projectedChars");
  const savedChars = rawChars !== undefined && projectedChars !== undefined ? Math.max(0, rawChars - projectedChars) : undefined;
  const toolCalls = numberFromDetails(details, "toolCalls");
  const summaryId = isRecord(details) && typeof details.summaryId === "string" ? details.summaryId : undefined;
  const restoreCount = arrayLengthFromDetails(details, "summaryIds");

  if (restoreCount !== undefined) {
    return `${JANITOR_MARKER} restored ${plural(restoreCount, "janitor run")}`;
  }

  if (toolCalls !== undefined) {
    const parts = [`${JANITOR_MARKER} truncated ${plural(toolCalls, "tool output")}`];
    if (savedChars !== undefined) parts.push(`saved ≈${formatCompactChars(savedChars)}`);
    if (summaryId) parts.push(summaryId);
    return parts.join(" · ");
  }

  if (rawText) return `${JANITOR_MARKER} ${clip(rawText, MAX_SUMMARY_LENGTH)}`;
  return `${JANITOR_MARKER} Context Janitor`;
}

export class HiddenCustomMessageComponent implements Component {
  invalidate(): void {}
  render(_width: number): string[] {
    return [];
  }
}

export class CompactJanitorNoticeComponent implements Component {
  constructor(
    private readonly message: CustomMessageLike,
    private readonly theme: CustomThemeLike,
  ) {}

  invalidate(): void {}

  render(width: number): string[] {
    if (!Number.isFinite(width) || width <= 0) return [];
    const line = themeFg(this.theme, "muted", compactJanitorNoticeLine(this.message));
    return [truncateToWidth(replaceTabs(line), Math.max(1, width), "…")];
  }
}

export function registerJanitorMessageRenderers(pi: ExtensionAPI): void {
  const hidden = () => new HiddenCustomMessageComponent();
  pi.registerMessageRenderer(JANITOR_INDEX_CUSTOM_TYPE, hidden);
  pi.registerMessageRenderer(JANITOR_RESTORE_CUSTOM_TYPE, hidden);
  pi.registerMessageRenderer(JANITOR_SUMMARY_CUSTOM_TYPE, hidden);
  pi.registerMessageRenderer(JANITOR_NOTICE_CUSTOM_TYPE, (message, _state, theme) => {
    return new CompactJanitorNoticeComponent(message as CustomMessageLike, theme as CustomThemeLike);
  });
}

export type CustomMessageComponentWithMessage = {
  message?: { customType?: unknown };
};

export function janitorCustomType(component: CustomMessageComponentWithMessage): string | undefined {
  const customType = component.message?.customType;
  return typeof customType === "string" && JANITOR_CUSTOM_TYPES.has(customType) ? customType : undefined;
}

export function withoutLeadingBlankLines(lines: string[]): string[] {
  let start = 0;
  while (start < lines.length && isBlankRenderedLine(lines[start] ?? "")) start += 1;
  return start === 0 ? lines : lines.slice(start);
}

export function renderConfiguredCustomMessage(
  component: CustomMessageComponentWithMessage,
  width: number,
  originalRender: (width: number) => string[],
): string[] {
  const lines = originalRender.call(component, width);
  const customType = janitorCustomType(component);
  if (!customType) return lines;

  const content = withoutLeadingBlankLines(lines);
  if (customType !== JANITOR_NOTICE_CUSTOM_TYPE || !Number.isFinite(width) || width <= 0) return content;

  const bgFn = getThemeToolBgFn("toolSuccessBg");
  return content.flatMap((line) => renderOneLine(replaceTabs(line), width, bgFn, true));
}

export function patchCustomMessageComponent(): boolean {
  try {
    const proto = (CustomMessageComponent as any)?.prototype;
    if (!proto || typeof proto.render !== "function") {
      throw new Error("CustomMessageComponent unavailable");
    }

    const originalRender = typeof proto[CUSTOM_ORIGINAL_RENDER_KEY] === "function" ? proto[CUSTOM_ORIGINAL_RENDER_KEY] : proto.render;

    proto.render = function piCompactCustomRender(this: CustomMessageComponentWithMessage, width: number) {
      return renderConfiguredCustomMessage(this, width, originalRender);
    };

    proto[CUSTOM_ORIGINAL_RENDER_KEY] = originalRender;
    state.lastCustomPatchError = undefined;
    return true;
  } catch (error) {
    state.lastCustomPatchError = error instanceof Error ? error.stack ?? error.message : String(error);
    return false;
  }
}
