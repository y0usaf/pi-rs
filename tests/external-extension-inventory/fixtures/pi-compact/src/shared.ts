import { homedir } from "node:os";
import { truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";
import { state } from "./state.js";
import { ANSI_PATTERN, BG_MARKER, FULL_SGR_RESET_PATTERN, type GapRendering, type ToolBgToken, type CompactThinkingTiming } from "./types.js";

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function isCompactThinkingTiming(value: unknown): value is CompactThinkingTiming {
  if (!isRecord(value)) return false;

  const started = value.startedAtMs;
  const completed = value.completedAtMs;
  return (
    (started === undefined || (typeof started === "number" && Number.isFinite(started))) &&
    (completed === undefined || (typeof completed === "number" && Number.isFinite(completed)))
  );
}

export function cloneGapRendering(value: GapRendering): GapRendering {
  return { mode: value.mode, gap: value.gap };
}

export function squash(value: unknown): string {
  return typeof value === "string" ? stripAnsi(value).replace(/\s+/g, " ").trim() : "";
}

export function stripAnsi(value: string): string {
  return value.replace(ANSI_PATTERN, "");
}

export function replaceTabs(value: string): string {
  return value.replace(/\t/g, "    ");
}

export function clip(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

export function shortenPath(path: string): string {
  const home = homedir();
  return path.startsWith(home) ? `~${path.slice(home.length)}` : path;
}

export function normalizePath(path: unknown, fallback = "."): string {
  if (typeof path !== "string" || path.length === 0) return fallback;
  const raw = stripAnsi(path);
  const clean = raw.startsWith("@") ? raw.slice(1) : raw;
  return shortenPath(clean);
}

export function lineCount(value: unknown): number {
  return typeof value === "string" && value.length > 0 ? value.split("\n").length : 0;
}

export function formatScalar(value: unknown): string {
  if (typeof value === "string") return squash(value);
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (value === null) return "null";
  if (Array.isArray(value)) return `[${value.length}]`;
  if (typeof value === "object") return "{…}";
  return "";
}

export function firstTextLine(result: any): string {
  if (!result?.content || !Array.isArray(result.content)) return "";
  for (const block of result.content) {
    if (block?.type === "text" && typeof block.text === "string") {
      const line = squash(block.text.split("\n")[0] ?? "");
      if (line) return line;
    }
  }
  return "";
}

export function textLineCount(result: any): number {
  if (!result?.content || !Array.isArray(result.content)) return 0;
  let total = 0;
  for (const block of result.content) {
    if (block?.type !== "text" || typeof block.text !== "string") continue;
    total += block.text.split("\n").filter((line: string) => line.trim().length > 0).length;
  }
  return total;
}

export type LineDiffCounts = { added: number; removed: number };

export function asLineCount(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? Math.trunc(value) : undefined;
}

export function hasLineDiff(counts: LineDiffCounts): boolean {
  return counts.added > 0 || counts.removed > 0;
}

export function countDiffLines(diff: unknown): LineDiffCounts | undefined {
  if (typeof diff !== "string" || diff.length === 0) return undefined;

  let added = 0;
  let removed = 0;
  for (const line of diff.split("\n")) {
    if (/^\+\s*\d+\s/.test(line)) added++;
    else if (/^-\s*\d+\s/.test(line)) removed++;
  }

  const counts = { added, removed };
  return hasLineDiff(counts) ? counts : undefined;
}

export function countMetricLines(metrics: unknown): LineDiffCounts | undefined {
  if (!isRecord(metrics)) return undefined;

  const added = asLineCount(metrics.added_lines ?? metrics.addedLines ?? metrics.added);
  const removed = asLineCount(metrics.removed_lines ?? metrics.removedLines ?? metrics.removed);
  if (added === undefined && removed === undefined) return undefined;

  const counts = { added: added ?? 0, removed: removed ?? 0 };
  return hasLineDiff(counts) ? counts : undefined;
}

export function countDetailsLineDiff(details: unknown): LineDiffCounts | undefined {
  if (!isRecord(details)) return undefined;
  return countMetricLines(details.metrics) ?? countMetricLines(details) ?? countDiffLines(details.diff);
}

export function colourDiffAdded(text: string): string {
  return state.activeTheme?.fg("toolDiffAdded", text) ?? text;
}

export function colourDiffRemoved(text: string): string {
  return state.activeTheme?.fg("toolDiffRemoved", text) ?? text;
}
export function getThemeToolBgFn(token: ToolBgToken): ((text: string) => string) | undefined {
  if (!state.activeTheme) return undefined;
  return (text: string) => state.activeTheme?.bg(token, text) ?? text;
}

export function splitWrappingAnsi(wrapper: (text: string) => string): { prefix: string; suffix: string } | undefined {
  const wrapped = wrapper(BG_MARKER);
  const markerIndex = wrapped.indexOf(BG_MARKER);
  if (markerIndex < 0) return undefined;
  return {
    prefix: wrapped.slice(0, markerIndex),
    suffix: wrapped.slice(markerIndex + BG_MARKER.length),
  };
}

export function applyBackgroundPreservingResets(text: string, bgFn: (text: string) => string): string {
  const wrapping = splitWrappingAnsi(bgFn);
  if (!wrapping) return bgFn(text);

  // truncateToWidth() inserts full SGR resets around ellipses to close active
  // foreground styles. Full resets also clear the row background, so re-apply
  // the background after each one before the final background-only reset.
  return `${wrapping.prefix}${text.replace(FULL_SGR_RESET_PATTERN, (reset) => `${reset}${wrapping.prefix}`)}${wrapping.suffix}`;
}

export function renderOneLine(rawLine: string, width: number, bgFn?: (text: string) => string, preserveAnsi = false): string[] {
  if (!Number.isFinite(width) || width <= 0) return [];

  const line = truncateToWidth(preserveAnsi ? rawLine : stripAnsi(rawLine), Math.max(1, width), "…");
  const padded = `${line}${" ".repeat(Math.max(0, width - visibleWidth(line)))}`;
  return [bgFn ? applyBackgroundPreservingResets(padded, bgFn) : padded];
}

export function isBlankRenderedLine(line: string): boolean {
  return stripAnsi(line).trim().length === 0;
}

