import type { ToolSummaryRegistry } from "./types.js";
import { clip, squash } from "../shared.js";

export const extensionToolSummaries: ToolSummaryRegistry = {
  agent_task: {
    summarizeArgs(args) {
      const action = squash(args?.action) || "?";
      const id = squash(args?.id);
      const task = squash(args?.task);
      if (action === "start") {
        const label = id ? `${id} • ` : "";
        return task ? `start ${label}${clip(task, 64)}` : `start${id ? ` ${id}` : ""}`;
      }
      return id ? `${action} ${id}` : action;
    },
    summarizeResult(result) {
      const details = result?.details ?? {};
      if (typeof details?.status === "string") return ` → ${details.status}`;
      if (Array.isArray(details?.tasks)) return ` → ${details.tasks.length}`;
      if (Array.isArray(details?.cancelled)) return ` → ${details.cancelled.length} cancelled`;
      return undefined;
    },
  },
  report: {
    summarizeArgs(args) {
      return clip(squash(args?.message) || "report", 80);
    },
  },
  web_fetch: {
    summarizeArgs(args) {
      const url = squash(args?.url) || "?";
      const prompt = squash(args?.prompt);
      return prompt ? `${url} • ${clip(prompt, 48)}` : url;
    },
    summarizeResult(result) {
      const details = result?.details ?? {};
      if (details?.fromCache) return " → cache";
      return undefined;
    },
  },
  web_search: {
    summarizeArgs(args) {
      const query = squash(args?.query) || "?";
      const engine = squash(args?.engine);
      return engine ? `${query} • ${engine}` : query;
    },
    summarizeResult(result) {
      const details = result?.details ?? {};
      if (typeof details?.resultCount === "number") return ` → ${details.resultCount} results`;
      return undefined;
    },
  },
  web_browse: {
    summarizeArgs(args) {
      const url = squash(args?.url) || "?";
      return args?.extract ? `${url} • extract` : url;
    },
    summarizeResult(result) {
      const details = result?.details ?? {};
      if (typeof details?.contentLength === "number") return ` → ${details.contentLength} chars`;
      return undefined;
    },
  },
};
