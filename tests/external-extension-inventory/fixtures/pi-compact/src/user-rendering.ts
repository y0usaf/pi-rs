import { UserMessageComponent } from "@earendil-works/pi-coding-agent";
import { state } from "./state.js";
import { MAX_USER_MESSAGE_LENGTH, OSC133_ZONE_END, OSC133_ZONE_FINAL, OSC133_ZONE_START, USER_ORIGINAL_RENDER_KEY, USER_PROMPT_MARKER } from "./types.js";
import { clip, renderOneLine, replaceTabs, squash, stripAnsi } from "./shared.js";
import { getVerticalPaddingShell, withPaddingY, type BoxWithVerticalPadding, type UserMessageWithContentBox } from "./tool-rendering.js";

export function getUserMessageContentBox(component: UserMessageWithContentBox): BoxWithVerticalPadding | undefined {
  return getVerticalPaddingShell(component.contentBox);
}

export function getUserBgFn(component: UserMessageWithContentBox): ((text: string) => string) | undefined {
  const bgFn = getUserMessageContentBox(component)?.bgFn;
  return typeof bgFn === "function" ? (bgFn as (text: string) => string) : undefined;
}

export function getUserMessageTextFromComponent(component: UserMessageWithContentBox): string {
  const children = getUserMessageContentBox(component)?.children;
  if (!Array.isArray(children)) return "";

  for (const child of children) {
    if (typeof child?.text === "string") return child.text;
  }

  return "";
}

export function getUserMessageTextFromRendered(lines: string[]): string {
  return squash(stripAnsi(lines.join(" ")));
}

export function withUserZoneMarkers(lines: string[]): string[] {
  if (lines.length === 0) return lines;

  const marked = [...lines];
  marked[0] = OSC133_ZONE_START + marked[0];
  marked[marked.length - 1] = OSC133_ZONE_END + OSC133_ZONE_FINAL + marked[marked.length - 1];
  return marked;
}

export function withUserMessageGap(lines: string[]): string[] {
  return state.userRendering.gap && lines.length > 0 ? [...lines, ""] : lines;
}

export function renderBorderlessUserMessage(
  component: UserMessageWithContentBox,
  width: number,
  originalRender: (width: number) => string[],
): string[] {
  const contentBox = getUserMessageContentBox(component);
  if (!contentBox) return originalRender.call(component, width);

  return withPaddingY([contentBox], 0, () => {
    const lines = originalRender.call(component, width);
    // Preserve Pi's post-user-message separation, but keep the blank row outside the grey background.
    return withUserMessageGap(lines);
  });
}

export function renderCompactUserMessage(
  component: UserMessageWithContentBox,
  width: number,
  originalRender: (width: number) => string[],
): string[] {
  const text = getUserMessageTextFromComponent(component) || getUserMessageTextFromRendered(originalRender.call(component, width));
  const summary = clip(squash(text), MAX_USER_MESSAGE_LENGTH) || "…";
  return withUserMessageGap(withUserZoneMarkers(renderOneLine(`${USER_PROMPT_MARKER} ${summary}`, width, getUserBgFn(component))));
}

export function renderConfiguredUserMessage(
  component: UserMessageWithContentBox,
  width: number,
  originalRender: (width: number) => string[],
): string[] {
  if (state.userRendering.mode === "hidden" || !Number.isFinite(width) || width <= 0) return [];
  if (state.userRendering.mode === "compact") return renderCompactUserMessage(component, width, originalRender);
  if (state.userRendering.mode === "borderless") return renderBorderlessUserMessage(component, width, originalRender);
  return originalRender.call(component, width);
}


export function patchUserMessageComponent(): boolean {
  try {
    const proto = (UserMessageComponent as any)?.prototype;
    if (!proto || typeof proto.render !== "function") {
      throw new Error("UserMessageComponent unavailable");
    }

    const originalRender = typeof proto[USER_ORIGINAL_RENDER_KEY] === "function" ? proto[USER_ORIGINAL_RENDER_KEY] : proto.render;

    proto.render = function piCompactUserRender(this: UserMessageWithContentBox, width: number) {
      return renderConfiguredUserMessage(this, width, originalRender);
    };

    proto[USER_ORIGINAL_RENDER_KEY] = originalRender;
    state.lastUserPatchError = undefined;
    return true;
  } catch (error) {
    state.lastUserPatchError = error instanceof Error ? error.stack ?? error.message : String(error);
    return false;
  }
}

