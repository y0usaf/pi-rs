import type { ToolSummaryRegistry } from "./types.js";
import { colourDiffAdded, colourDiffRemoved, countDetailsLineDiff, lineCount, normalizePath, squash, textLineCount } from "../shared.js";

export const nativePiToolSummaries: ToolSummaryRegistry = {
  read: {
    summarizeArgs(args) {
      const path = normalizePath(args?.path, "?");
      if (args?.offset === undefined && args?.limit === undefined) return path;
      const start = Number(args?.offset ?? 1);
      if (args?.limit === undefined) return `${path}:${start}`;
      return `${path}:${start}-${start + Number(args.limit) - 1}`;
    },
  },
  bash: {
    summarizeArgs(args) {
      const command = squash(args?.command) || "…";
      const timeout = args?.timeout !== undefined ? ` • timeout=${args.timeout}s` : "";
      return `${command}${timeout}`;
    },
    summarizeResult(result) {
      const details = result?.details ?? {};
      if (typeof details.exitCode === "number") return details.exitCode === 0 ? "" : ` → exit ${details.exitCode}`;
      return undefined;
    },
  },
  edit: {
    summarizeArgs(args) {
      const path = normalizePath(args?.path, "?");
      const edits = Array.isArray(args?.edits)
        ? args.edits.length
        : args?.oldText !== undefined || args?.newText !== undefined
          ? 1
          : 0;
      return edits > 0 ? `${path} • ${edits} edit${edits === 1 ? "" : "s"}` : path;
    },
    summarizeResult(result) {
      const counts = countDetailsLineDiff(result?.details ?? {});
      if (counts) return ` ${colourDiffAdded(`+${counts.added}`)} ${colourDiffRemoved(`-${counts.removed}`)}`;
      return undefined;
    },
  },
  write: {
    summarizeArgs(args) {
      const path = normalizePath(args?.path, "?");
      const lines = lineCount(args?.content);
      return lines > 0 ? `${path} • ${lines} lines` : path;
    },
  },
  find: {
    summarizeArgs(args) {
      const pattern = squash(args?.pattern) || "*";
      const path = normalizePath(args?.path, ".");
      const limit = args?.limit !== undefined ? ` • limit=${args.limit}` : "";
      return `${pattern} @ ${path}${limit}`;
    },
    summarizeResult(result) {
      const count = textLineCount(result);
      return count > 0 ? ` → ${count}` : undefined;
    },
  },
  grep: {
    summarizeArgs(args) {
      const pattern = squash(args?.pattern) || ".*";
      const path = normalizePath(args?.path, ".");
      const glob = squash(args?.glob);
      const limit = args?.limit !== undefined ? ` • limit=${args.limit}` : "";
      return `/${pattern}/ @ ${path}${glob ? ` • ${glob}` : ""}${limit}`;
    },
    summarizeResult(result) {
      const count = textLineCount(result);
      return count > 0 ? ` → ${count}` : undefined;
    },
  },
  ls: {
    summarizeArgs(args) {
      const path = normalizePath(args?.path, ".");
      const limit = args?.limit !== undefined ? ` • limit=${args.limit}` : "";
      return `${path}${limit}`;
    },
    summarizeResult(result) {
      const count = textLineCount(result);
      return count > 0 ? ` → ${count}` : undefined;
    },
  },
};
