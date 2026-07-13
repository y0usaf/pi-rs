import { MAX_RESULT_LENGTH } from "../types.js";
import { clip, firstTextLine, formatScalar, normalizePath } from "../shared.js";
import { isReplTool } from "./repl-adapter.js";

const PREFERRED_ARG_KEYS = ["path", "url", "query", "id", "name", "command", "pattern", "glob", "prompt", "message"];

export function fallbackSummarizeArgs(args: any): string {
  if (args === undefined || args === null) return "";
  if (typeof args !== "object") return formatScalar(args);
  if (Array.isArray(args)) return `[${args.length}]`;

  const parts: string[] = [];

  for (const key of PREFERRED_ARG_KEYS) {
    if (!(key in args) || args[key] === undefined) continue;
    const value = key === "path" ? normalizePath(args[key]) : formatScalar(args[key]);
    if (!value) continue;
    parts.push(key === "path" ? value : `${key}=${value}`);
    if (parts.length >= 3) break;
  }

  if (parts.length === 0) {
    for (const [key, value] of Object.entries(args)) {
      const formatted = formatScalar(value);
      if (!formatted) continue;
      parts.push(`${key}=${formatted}`);
      if (parts.length >= 3) break;
    }
  }

  return parts.join(" • ");
}

export function summarizeErrorResult(result: any): string {
  const line = firstTextLine(result);
  return line ? ` → ${clip(line, MAX_RESULT_LENGTH)}` : " → error";
}

export function fallbackSummarizeResult(toolName: string, result: any): string {
  const line = firstTextLine(result);
  if (!line || line === "done" || (isReplTool(toolName) && line === "(no output)")) return "";
  return ` → ${clip(line, MAX_RESULT_LENGTH)}`;
}
