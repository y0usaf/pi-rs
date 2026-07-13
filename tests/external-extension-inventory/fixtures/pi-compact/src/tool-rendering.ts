import { ToolExecutionComponent } from "@earendil-works/pi-coding-agent";
import { state } from "./state.js";
import { MAX_SUMMARY_LENGTH, TOOL_ORIGINAL_RENDER_KEY, TOOL_ORIGINAL_SET_EXPANDED_KEY, TOOL_RULE, TOOL_SPINNER_FRAME_KEY, TOOL_SPINNER_INTERVAL_KEY, type ToolBgToken } from "./types.js";
import { summarizeArgs, summarizeResult, filterToolViewLines, isReplBootstrapOnlyArgs, isReplTool, summarizeReplProgress } from "./tool-summaries/index.js";
import { clip, getThemeToolBgFn, isBlankRenderedLine, isRecord, renderOneLine } from "./shared.js";
export { filterToolViewLines, isReplBootstrapImportLine, isReplBootstrapOnlyArgs, summarizeArgs, summarizeReplCode, summarizeResult } from "./tool-summaries/index.js";

export function getToolSpinnerFrame(_state: any): string {
  // Keep pending compact tool rows static. An animated spinner repeatedly
  // requested renders while tools were collapsed, which made browser-side
  // scroll anchoring/observers loop when Ctrl+O expanded tool uses.
  return "⠋";
}

export function toolStatusPrefix(state: any): string {
  if (state?.isPartial) return getToolSpinnerFrame(state);
  return state?.result?.isError ? "✗" : "✓";
}

export function buildToolLine(state: any): string {
  const toolName = state?.toolName ?? "tool";
  const prefix = toolStatusPrefix(state);

  if (state?.isPartial && isReplTool(toolName)) {
    const live = summarizeReplProgress(state?.result, state?.args);
    if (live) return `${prefix} ${toolName} ${TOOL_RULE} ${clip(live, MAX_SUMMARY_LENGTH)}`;
  }

  const summary = clip(summarizeArgs(toolName, state?.args), MAX_SUMMARY_LENGTH);
  const suffix = summarizeResult(toolName, state?.result);
  const detail = summary || suffix ? ` ${TOOL_RULE} ${summary || "…"}${suffix}` : "";
  return `${prefix} ${toolName}${detail}`;
}

export function getToolBgToken(state: any): ToolBgToken {
  if (state?.isPartial) return "toolPendingBg";
  return state?.result?.isError ? "toolErrorBg" : "toolSuccessBg";
}

export function getToolBgFn(state: any): ((text: string) => string) | undefined {
  const token = getToolBgToken(state);

  // Do not inherit from ToolExecutionComponent internals: self-shell tools keep
  // contentBox at the pending colour, which makes settled compact rows look grey.
  return getThemeToolBgFn(token);
}

export function renderCompactToolLine(state: any, width: number): string[] {
  return renderOneLine(buildToolLine(state), width, getToolBgFn(state), true);
}

export type UserMessageWithContentBox = {
  contentBox?: unknown;
};

export type BoxWithVerticalPadding = Record<string, unknown> & {
  paddingY: number;
  bgFn?: unknown;
  children?: unknown;
  cache?: unknown;
  cachedText?: unknown;
  cachedWidth?: unknown;
  cachedLines?: unknown;
};

export type ToolExecutionWithShells = {
  contentBox?: unknown;
  contentText?: unknown;
  /**
   * Older Pi builds exposed expansion as a public field. Current Pi stores it
   * in a private #expanded slot, so pi-compact must not rely on this existing.
   */
  expanded?: boolean;
  setExpanded?: (expanded: boolean) => void;
  isPartial?: boolean;
  toolName?: string;
  args?: any;
  result?: { isError?: boolean };
  ui?: { requestRender?: (force?: boolean) => void };
  [TOOL_SPINNER_INTERVAL_KEY]?: ReturnType<typeof setInterval>;
  [TOOL_SPINNER_FRAME_KEY]?: number;
};

const toolExpandedState = new WeakMap<object, boolean>();

export function hasToolExpandedState(component: ToolExecutionWithShells): boolean {
  return toolExpandedState.has(component);
}

export function getToolExpanded(component: ToolExecutionWithShells): boolean {
  const tracked = toolExpandedState.get(component);
  if (tracked !== undefined) return tracked;
  return component.expanded === true;
}

export function setToolExpandedState(component: ToolExecutionWithShells, expanded: boolean): void {
  toolExpandedState.set(component, expanded);
}

export function requestToolRender(component: ToolExecutionWithShells, force = false): void {
  const requestRender = component.ui?.requestRender;
  if (typeof requestRender !== "function") {
    stopToolSpinner(component);
    return;
  }

  try {
    requestRender.call(component.ui, force);
  } catch {
    stopToolSpinner(component);
  }
}

export function startToolSpinner(component: ToolExecutionWithShells): void {
  // Historical cleanup/no-op: compact pending tools used to start a render
  // interval here. The interval could keep mutating output while expansion was
  // toggled, causing an infinite scrolling loop in the UI. Leave the exported
  // helper in place, but ensure any old interval is cleared and render static
  // pending indicators instead.
  stopToolSpinner(component);
}

export function stopToolSpinner(component: ToolExecutionWithShells): void {
  const interval = component[TOOL_SPINNER_INTERVAL_KEY];
  if (interval !== undefined) clearInterval(interval);
  component[TOOL_SPINNER_INTERVAL_KEY] = undefined;
  component[TOOL_SPINNER_FRAME_KEY] = 0;
}

export function shouldRenderCompactToolLine(component: ToolExecutionWithShells): boolean {
  return state.toolRendering.mode === "compact" && !getToolExpanded(component);
}

export function syncToolSpinner(component: ToolExecutionWithShells, _compactLine: boolean): void {
  stopToolSpinner(component);
}

export function syncToolSpinnerForCurrentExpansion(component: ToolExecutionWithShells): void {
  syncToolSpinner(component, shouldRenderCompactToolLine(component));
}

export function getVerticalPaddingShell(value: unknown): BoxWithVerticalPadding | undefined {
  return isRecord(value) && typeof value.paddingY === "number" ? (value as BoxWithVerticalPadding) : undefined;
}

export function clearShellCache(shell: BoxWithVerticalPadding): void {
  shell.cache = undefined;
  shell.cachedText = undefined;
  shell.cachedWidth = undefined;
  shell.cachedLines = undefined;
}

export function withPaddingY<T>(shells: BoxWithVerticalPadding[], paddingY: number, render: () => T): T {
  const previous = shells.map((shell) => shell.paddingY);
  for (const shell of shells) {
    shell.paddingY = paddingY;
    clearShellCache(shell);
  }

  try {
    return render();
  } finally {
    shells.forEach((shell, index) => {
      shell.paddingY = previous[index] ?? shell.paddingY;
      clearShellCache(shell);
    });
  }
}

export function clearToolShellCaches(component: ToolExecutionWithShells): void {
  const shells = [getVerticalPaddingShell(component.contentBox), getVerticalPaddingShell(component.contentText)].filter(
    (shell): shell is BoxWithVerticalPadding => shell !== undefined,
  );
  for (const shell of shells) clearShellCache(shell);
}

export function withoutLeadingBlankLine(lines: string[]): string[] {
  return lines.length > 0 && isBlankRenderedLine(lines[0]) ? lines.slice(1) : lines;
}

export function withToolGap(lines: string[]): string[] {
  const content = withoutLeadingBlankLine(lines);
  return state.toolRendering.gap && content.length > 0 ? ["", ...content] : content;
}

export function renderBorderlessTool(
  component: ToolExecutionWithShells,
  width: number,
  originalRender: (width: number) => string[],
): string[] {
  const shells = [getVerticalPaddingShell(component.contentBox), getVerticalPaddingShell(component.contentText)].filter(
    (shell): shell is BoxWithVerticalPadding => shell !== undefined,
  );
  return withPaddingY(shells, 0, () => withToolGap(filterToolViewLines(originalRender.call(component, width))));
}

export function renderConfiguredTool(component: ToolExecutionWithShells, width: number, originalRender: (width: number) => string[]): string[] {
  const compactLine = shouldRenderCompactToolLine(component);
  syncToolSpinner(component, compactLine);

  if (isReplTool(component.toolName ?? "") && isReplBootstrapOnlyArgs(component.args) && component.result?.isError !== true) return [];
  if (state.toolRendering.mode === "hidden" || !Number.isFinite(width) || width <= 0) return [];

  if (compactLine) return withToolGap(renderCompactToolLine(component, width));
  if (state.toolRendering.mode === "borderless") return renderBorderlessTool(component, width, originalRender);
  return withToolGap(filterToolViewLines(originalRender.call(component, width)));
}


export function patchToolExecutionComponent(): boolean {
  try {
    const proto = (ToolExecutionComponent as any)?.prototype;
    if (!proto || typeof proto.render !== "function" || typeof proto.setExpanded !== "function") {
      throw new Error("ToolExecutionComponent unavailable");
    }

    const originalRender = typeof proto[TOOL_ORIGINAL_RENDER_KEY] === "function" ? proto[TOOL_ORIGINAL_RENDER_KEY] : proto.render;
    const originalSetExpanded =
      typeof proto[TOOL_ORIGINAL_SET_EXPANDED_KEY] === "function" ? proto[TOOL_ORIGINAL_SET_EXPANDED_KEY] : proto.setExpanded;

    proto.render = function piCompactToolRender(this: ToolExecutionWithShells & { hideComponent?: boolean }, width: number) {
      if (this.hideComponent) {
        stopToolSpinner(this);
        return [];
      }
      return renderConfiguredTool(this, width, originalRender);
    };

    proto.setExpanded = function piCompactToolSetExpanded(this: ToolExecutionWithShells, expanded: boolean) {
      // Ctrl+O should still use Pi's native expansion implementation. We only
      // mirror the requested state because modern ToolExecutionComponent keeps
      // its actual expansion flag in a private #expanded field that patches
      // cannot read. Without this mirror, compact rendering always thinks the
      // tool is collapsed and fights the native expanded display.
      const hadTrackedExpansion = hasToolExpandedState(this);
      const wasExpanded = getToolExpanded(this);
      const result = originalSetExpanded.call(this, expanded);
      setToolExpandedState(this, expanded);
      syncToolSpinnerForCurrentExpansion(this);

      // Collapsing expanded tool output back to one compact row can shrink the
      // rendered chat by many off-screen rows. Pi's differential renderer usually
      // clears visible rows, but terminal scrollback can retain old expanded
      // output and make the session look duplicated until it is reopened. Force a
      // single full redraw on real compact-mode collapses, matching the clean
      // state users get after reopening without making every render expensive.
      if (state.toolRendering.mode === "compact" && hadTrackedExpansion && wasExpanded !== expanded && !expanded) {
        clearToolShellCaches(this);
        requestToolRender(this, true);
      }

      return result;
    };

    proto[TOOL_ORIGINAL_RENDER_KEY] = originalRender;
    proto[TOOL_ORIGINAL_SET_EXPANDED_KEY] = originalSetExpanded;
    state.lastToolPatchError = undefined;
    return true;
  } catch (error) {
    state.lastToolPatchError = error instanceof Error ? error.stack ?? error.message : String(error);
    return false;
  }
}
