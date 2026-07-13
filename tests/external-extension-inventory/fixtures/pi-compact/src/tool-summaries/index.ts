import type { ToolSummaryRegistry } from "./types.js";
import { extensionToolSummaries } from "./extension-adapters.js";
import { fallbackSummarizeArgs, fallbackSummarizeResult, summarizeErrorResult } from "./fallback.js";
import { nativePiToolSummaries } from "./native-pi.js";
import { isReplTool, replToolSummary } from "./repl-adapter.js";

export {
  filterToolViewLines,
  isReplBootstrapImportLine,
  isReplBootstrapOnlyArgs,
  isReplTool,
  summarizeReplCode,
  summarizeReplProgress,
} from "./repl-adapter.js";
export type { ToolSummaryAdapter, ToolSummaryRegistry } from "./types.js";

const toolSummaries: ToolSummaryRegistry = {
  ...nativePiToolSummaries,
  ...extensionToolSummaries,
};

export function summarizeArgs(toolName: string, args: any): string {
  const adapter = isReplTool(toolName) ? replToolSummary : toolSummaries[toolName];
  const summary = adapter?.summarizeArgs?.(args);
  return summary !== undefined ? summary : fallbackSummarizeArgs(args);
}

export function summarizeResult(toolName: string, result: any): string {
  if (!result) return "";
  if (result?.isError) return summarizeErrorResult(result);

  const adapter = isReplTool(toolName) ? replToolSummary : toolSummaries[toolName];
  const summary = adapter?.summarizeResult?.(result);
  return summary !== undefined ? summary : fallbackSummarizeResult(toolName, result);
}
