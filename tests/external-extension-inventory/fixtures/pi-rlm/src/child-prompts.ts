import { MAX_INLINE_CHILD_CONTEXT_CHARS, REPL_TOOL_NAME } from "./constants.js";
import type { ContextStore, RunState } from "./constants.js";
import { contextMaterialized, contextStorePromptBlock } from "./context-store.js";
import { renderTemplate, xmlEscape } from "./prompt-render.js";
import { CHILD_SYSTEM_PROMPT, CHILD_TASK_PROMPT, DETERMINISTIC_FINAL_PROMPT } from "./prompts.js";
import { clip, textOf } from "./utils.js";

function limitLabel(value: number | undefined): string {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? String(value) : "∞";
}

export function childSystemPrompt(depth: number, state: RunState, hasContextStore: boolean): string {
  const contextLine = hasContextStore
    ? "A file-backed context source set has been loaded into the REPL as context/context_0/context_N values. Use SHOW_VARS() and normal Python inspection; do not print huge values back into chat."
    : "Use the REPL context/history/state variables when present.";
  const turnRule = state.maxTurns === undefined
    ? "There is no pi-rlm turn cap for this run; still finalize promptly once the answer is ready."
    : "If turn budget runs low, call FINAL_VAR with a partial answer and remaining work.";

  return renderTemplate(CHILD_SYSTEM_PROMPT, {
    depth,
    maxDepth: limitLabel(state.maxDepth),
    callsUsed: state.budget.calls,
    maxCalls: limitLabel(state.budget.maxCalls),
    queriesUsed: state.budget.queries,
    maxQueries: limitLabel(state.budget.maxQueries),
    toolName: REPL_TOOL_NAME,
    contextLine,
    turnRule,
  });
}

export function childPrompt(prompt: string, context?: string, paths?: string[], store?: ContextStore, rootPrompt?: string): string {
  const pathLines = paths?.length
    ? paths.map((p) => `    <path>${xmlEscape(p)}</path>`).join("\n")
    : "    <path>(none)</path>";
  const inlineContextBlock = context?.trim() && !contextMaterialized(store)
    ? `  <inlineContext>${xmlEscape(clip(context, MAX_INLINE_CHILD_CONTEXT_CHARS))}</inlineContext>`
    : "";
  const rootPromptBlock = rootPrompt?.trim()
    ? `  <rootPrompt>${xmlEscape(rootPrompt)}</rootPrompt>`
    : "";
  const contextStoreBlock = store ? contextStorePromptBlock(store) : "";

  return renderTemplate(CHILD_TASK_PROMPT, {
    prompt,
    rootPromptBlock,
    inlineContextBlock,
    contextStoreBlock,
    pathLines,
    toolName: REPL_TOOL_NAME,
  });
}

export function childToolList(): string[] {
  return [REPL_TOOL_NAME];
}

function childTranscript(messages: any[], maxChars = 120_000): string {
  const lines: string[] = [];
  for (const m of messages) {
    const role = typeof m?.role === "string" ? m.role : "?";
    const tool = typeof m?.toolName === "string" ? `:${m.toolName}` : "";
    const body = textOf(m?.content).trim();
    if (!body) continue;
    lines.push(`## ${role}${tool}\n${body}`);
  }
  return clip(lines.join("\n\n"), maxChars);
}

export function deterministicFinalPrompt(originalPrompt: string, messages: any[], reason: string): string {
  return renderTemplate(DETERMINISTIC_FINAL_PROMPT, {
    reason,
    originalPrompt,
    transcript: childTranscript(messages),
  });
}
