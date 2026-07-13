import { StringEnum } from "@earendil-works/pi-ai";
import { Type } from "typebox";

import {
  CONTEXT_MODES,
  DEFAULT_MAX_BUDGET,
  DEFAULT_MAX_CALLS,
  DEFAULT_MAX_CONCURRENT,
  DEFAULT_MAX_DEPTH,
  DEFAULT_MAX_ERRORS,
  DEFAULT_MAX_QUERIES,
  DEFAULT_MAX_TIMEOUT_MS,
  DEFAULT_MAX_TOKENS,
  DEFAULT_MAX_TURNS,
  HARD_MAX_BUDGET,
  HARD_MAX_CALLS,
  HARD_MAX_CONCURRENT,
  HARD_MAX_DEPTH,
  HARD_MAX_ERRORS,
  HARD_MAX_QUERIES,
  HARD_MAX_TIMEOUT_MS,
  HARD_MAX_TOKENS,
  HARD_MAX_TURNS,
  RLM_CALLS,
} from "./constants.js";

// ── Params ──────────────────────────────────────────────────────────

export const LimitParams = {
  maxDepth: Type.Optional(Type.Number({ description: `Recursive depth cap. Default ${DEFAULT_MAX_DEPTH}; <=0 = unlimited. Positive values are clamped to hard cap ${HARD_MAX_DEPTH}. At the cap, rlm_query falls back to a plain LM call.` })),
  maxTurns: Type.Optional(Type.Number({ description: `Recursive child turn cap. Default ${DEFAULT_MAX_TURNS}; <=0 = unlimited. Positive values are clamped to hard cap ${HARD_MAX_TURNS}.` })),
  maxCalls: Type.Optional(Type.Number({ description: `Total recursive child RLM calls across this run. Default ${DEFAULT_MAX_CALLS}; <=0 = unlimited. Positive values are clamped to hard cap ${HARD_MAX_CALLS}.` })),
  maxQueries: Type.Optional(Type.Number({ description: `Total llm_query leaf calls across this run. Default ${DEFAULT_MAX_QUERIES}; <=0 = unlimited. Positive values are clamped to hard cap ${HARD_MAX_QUERIES}.` })),
  maxConcurrent: Type.Optional(Type.Number({ description: `Batch concurrency cap. Default ${DEFAULT_MAX_CONCURRENT}; <=0 = no explicit cap (runs up to batch size). Positive values are clamped to hard cap ${HARD_MAX_CONCURRENT}.` })),
  maxTimeoutMs: Type.Optional(Type.Number({ description: `Wall-clock timeout for the whole recursive RLM tree in milliseconds. Default ${DEFAULT_MAX_TIMEOUT_MS} (unlimited). Hard cap ${HARD_MAX_TIMEOUT_MS}.` })),
  maxTimeout: Type.Optional(Type.Number({ description: "Upstream-style wall-clock timeout in seconds. Alias for maxTimeoutMs." })),
  max_timeout: Type.Optional(Type.Number({ description: "Upstream-style wall-clock timeout in seconds. Alias for maxTimeoutMs." })),
  maxTokens: Type.Optional(Type.Number({ description: `Approximate total input+output token cap across tracked LM calls. Default ${DEFAULT_MAX_TOKENS} (unlimited). Hard cap ${HARD_MAX_TOKENS}.` })),
  max_tokens: Type.Optional(Type.Number({ description: "Upstream-style alias for maxTokens." })),
  maxBudget: Type.Optional(Type.Number({ description: `USD cost cap across tracked LM calls when providers report usage. Default ${DEFAULT_MAX_BUDGET} (unlimited). Hard cap ${HARD_MAX_BUDGET}.` })),
  max_budget: Type.Optional(Type.Number({ description: "Upstream-style alias for maxBudget." })),
  maxErrors: Type.Optional(Type.Number({ description: `Consecutive/aggregate RLM runtime error cap. Default ${DEFAULT_MAX_ERRORS} (unlimited). Hard cap ${HARD_MAX_ERRORS}.` })),
  max_errors: Type.Optional(Type.Number({ description: "Upstream-style alias for maxErrors." })),
  maxIterations: Type.Optional(Type.Number({ description: "Alias for maxTurns." })),
  max_iterations: Type.Optional(Type.Number({ description: "Upstream-style alias for maxTurns." })),
  max_depth: Type.Optional(Type.Number({ description: "Upstream-style alias for maxDepth." })),
  max_concurrent_subcalls: Type.Optional(Type.Number({ description: "Upstream-style alias for maxConcurrent." })),
};

export const ContextModeParam = Type.Optional(StringEnum(CONTEXT_MODES, {
  description:
    'Context handling for recursive RLM calls. "auto" keeps short inline context in chat but materializes large context into a temp file; paths are always file-backed. "inline" preserves old inline behavior for context. "file_backed" materializes context into the temp context store.',
}));

export const SourceParam = Type.Object({
  name: Type.Optional(Type.String({ description: "Optional stable source name/alias for context source selection." })),
  path: Type.String({ description: "File or directory path for this file-backed context source." }),
});

export const RlmBatchItem = Type.Object({
  prompt: Type.String({ description: "Prompt for this batch item." }),
  rootPrompt: Type.Optional(Type.String({ description: "Small visible/root prompt or question for this item; analogous to upstream root_prompt. Appended separately from large context." })),
  model: Type.Optional(Type.String({ description: "Optional model selector for this call, matching upstream llm_query(..., model=...)." })),

  context: Type.Optional(Type.String({ description: "Optional inline context for this item." })),
  contextMode: ContextModeParam,
  paths: Type.Optional(Type.Array(Type.String(), { description: "Paths for this child RLM to inspect. Used by rlm_query_batched only. Paths are file-backed context sources." })),
  sources: Type.Optional(Type.Array(SourceParam, { description: "Named file-backed sources for this child RLM. Not accepted for llm_query calls." })),
  contextName: Type.Optional(Type.String({ description: "Optional name/label for materialized inline context." })),
});

export const RlmParams = Type.Object({
  call: StringEnum(RLM_CALLS, {
    description:
      'RLM call to run: "llm_query", "llm_query_batched", "rlm_query", or "rlm_query_batched".',
  }),
  prompt: Type.Optional(Type.String({ description: "Prompt for llm_query or rlm_query." })),
  rootPrompt: Type.Optional(Type.String({ description: "Small visible/root prompt or question; analogous to upstream root_prompt. Appended separately from large context." })),
  model: Type.Optional(Type.String({ description: "Optional model selector for this call, matching upstream llm_query(..., model=...)." })),

  context: Type.Optional(
    Type.String({ description: "Optional context. For llm_query this is inlined. For recursive RLM calls, large context is materialized into the file-backed context store when contextMode='auto' or 'file_backed'." }),
  ),
  contextMode: ContextModeParam,
  paths: Type.Optional(
    Type.Array(Type.String(), { description: "Paths for rlm_query/rlm_query_batched children. They are kept outside chat and loaded into the child REPL context; not accepted for llm_query calls." }),
  ),
  sources: Type.Optional(Type.Array(SourceParam, { description: "Named file-backed sources for rlm_query/rlm_query_batched children. Not accepted for llm_query calls." })),
  contextName: Type.Optional(Type.String({ description: "Optional source name/label for materialized inline context." })),
  prompts: Type.Optional(
    Type.Array(Type.String(), { description: "Prompts for batched calls. Shared context/paths apply to each item." }),
  ),
  items: Type.Optional(
    Type.Array(RlmBatchItem, { description: "Structured batch items with per-item prompt/context/contextMode/paths/sources/contextName." }),
  ),
  ...LimitParams,
  logPath: Type.Optional(Type.String({ description: "Optional JSONL trajectory log path for this RLM run." })),
  logDir: Type.Optional(Type.String({ description: "Optional directory for JSONL trajectory logs; creates one file per run." })),
});

export const ReplParams = Type.Object({
  code: Type.String({ description: "Python code to run inside the upstream-style RLM REPL. Public helpers: llm_query, llm_query_batched, rlm_query, rlm_query_batched, FINAL_VAR, SHOW_VARS; use globals/state/history/context for persistence and context." }),
  reset: Type.Optional(Type.Boolean({ description: "Clear persistent REPL state before running this code. Default false." })),
  timeoutMs: Type.Optional(Type.Number({ description: "Local Python execution timeout. Paused while synchronous model-call helpers are running. Default 30000, hard cap 120000." })),
  data: Type.Optional(Type.Record(Type.String(), Type.Any(), { description: "Optional JSON-serializable variables to inject into the Python REPL globals before running code." })),
  setup: Type.Optional(Type.String({ description: "Optional Python setup code to execute before the main code in this eval." })),
  resetHistory: Type.Optional(Type.Boolean({ description: "Clear REPL history variables before running this code. Default false." })),
});

export const RLM_PARAM_KEYS = new Set([
  "call",
  "prompt",
  "rootPrompt",
  "model",

  "context",
  "contextMode",
  "paths",
  "sources",
  "contextName",
  "prompts",
  "items",
  "maxDepth",
  "maxTurns",
  "maxCalls",
  "maxQueries",
  "maxConcurrent",
  "maxTimeoutMs",
  "maxTimeout",
  "max_timeout",
  "maxTokens",
  "max_tokens",
  "maxBudget",
  "max_budget",
  "maxErrors",
  "max_errors",
  "maxIterations",
  "max_iterations",
  "max_depth",
  "max_concurrent_subcalls",
  "logPath",
  "logDir",
]);

export const RLM_ITEM_KEYS = new Set(["model", "prompt", "rootPrompt", "context", "contextMode", "paths", "sources", "contextName"]);
export const REPL_PARAM_KEYS = new Set(["code", "reset", "timeoutMs", "data", "setup", "resetHistory"]);