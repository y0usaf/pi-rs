import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

import type { ContextMode, Details, RunState } from "./constants.js";
import { budgetDetails, checkRunLimits, clip, normPaths, normSources, textOf } from "./utils.js";
import { runLlmQuery } from "./llm.js";
import { renderTemplate, xmlEscape } from "./prompt-render.js";
import { DECOMPOSE_SYSTEM_PROMPT, DECOMPOSE_USER_PROMPT, SYNTHESIZE_PROMPT } from "./prompts.js";

// ── Structural decomposition ─────────────────────────────────────────

const DECOMPOSE_SYSTEM = DECOMPOSE_SYSTEM_PROMPT;

export function decomposeUserPrompt(prompt: string, context?: string, paths?: string[]): string {
  const pathsBlock = paths?.length
    ? `  <paths>
${paths.map((p) => `    <path>${xmlEscape(p)}</path>`).join("\n")}
  </paths>`
    : "";
  const contextBlock = context?.trim()
    ? `  <contextExcerpt>${xmlEscape(context.slice(0, 2000))}</contextExcerpt>`
    : "";
  return renderTemplate(DECOMPOSE_USER_PROMPT, {
    prompt,
    pathsBlock,
    contextBlock,
  });
}

interface DecomposeResult {
  decompose: boolean;
  subtasks?: string[];
  reason?: string;
}

export function parseDecomposeResponse(text: string): DecomposeResult | undefined {
  try {
    // Strip markdown fences if present
    const cleaned = text.replace(/^```(?:json)?\s*/m, "").replace(/```\s*$/m, "").trim();
    const parsed = JSON.parse(cleaned);
    if (typeof parsed?.decompose !== "boolean") return undefined;
    if (parsed.decompose && (!Array.isArray(parsed.subtasks) || parsed.subtasks.length < 2)) return undefined;
    if (parsed.decompose && parsed.subtasks.some((s: unknown) => typeof s !== "string" || !s)) return undefined;
    return parsed as DecomposeResult;
  } catch {
    return undefined;
  }
}

/**
 * Attempt structural decomposition: ask a leaf LLM whether this task should
 * be split, and if so, automatically fan out child rlm_query calls.
 *
 * Returns the synthesized result if decomposition was performed, or undefined
 * if the task should be handled as a single interactive session.
 */
export async function tryStructuralDecompose(
  ctx: ExtensionContext,
  params: { prompt: string; rootPrompt?: string; model?: string; context?: string; contextMode?: ContextMode; paths?: string[]; sources?: Array<{ name?: string; path: string }>; contextName?: string },
  depth: number,
  state: RunState,
  signal: AbortSignal | undefined,
  onUpdate: any,
): Promise<{ content: Array<{ type: "text"; text: string }>; details: Details } | undefined> {
  // Don't decompose if we're already near limits
  if (state.maxDepth !== undefined && depth + 1 >= state.maxDepth) return undefined;
  if (state.budget.maxCalls !== undefined && state.budget.calls + 2 >= state.budget.maxCalls) return undefined;
  if (state.budget.maxQueries !== undefined && state.budget.queries + 2 >= state.budget.maxQueries) return undefined;

  onUpdate?.({ content: [{ type: "text", text: `depth ${depth}: checking structural decomposition...` }] });

  // Ask leaf LLM to decompose
  const decomposeResult = await runLlmQuery(ctx, {
    prompt: DECOMPOSE_SYSTEM + "\n\n" + decomposeUserPrompt(params.prompt, params.context, params.paths),
  }, state.budget, depth, state, signal, onUpdate, "llm_query");

  const decomposeText = textOf(decomposeResult.content).trim();
  const decision = parseDecomposeResponse(decomposeText);

  if (!decision?.decompose || !decision.subtasks?.length) {
    onUpdate?.({ content: [{ type: "text", text: `depth ${depth}: no decomposition needed${decision?.reason ? ` (${decision.reason})` : ""}` }] });
    return undefined;
  }

  // ── Fan out: spawn parallel child rlm_query calls ──
  const subtasks = decision.subtasks;
  onUpdate?.({ content: [{ type: "text", text: `depth ${depth}: structural decomposition into ${subtasks.length} subtasks` }] });

  const { runBatch } = await import("./batch.js");
  const batchParams = {
    call: "rlm_query_batched" as const,
    items: subtasks.map((subtask) => ({
      prompt: subtask,
      rootPrompt: params.rootPrompt ?? params.prompt,
      model: params.model,
      context: params.context,
      contextMode: params.contextMode,
      paths: params.paths,
      sources: params.sources,
      contextName: params.contextName,
    })),
  };

  const batchResult = await runBatch(ctx, batchParams, "rlm_query_batched", depth + 1, state, signal, onUpdate);

  // ── Synthesize: combine child results ──
  const childAnswers = textOf(batchResult.content).trim();
  onUpdate?.({ content: [{ type: "text", text: `depth ${depth}: synthesizing ${subtasks.length} child results...` }] });

  const synthesizePrompt = renderTemplate(SYNTHESIZE_PROMPT, {
    subtaskCount: subtasks.length,
    prompt: params.prompt,
    childAnswers,
  });

  const synthesized = await runLlmQuery(ctx, {
    prompt: synthesizePrompt,
    rootPrompt: params.rootPrompt,
  }, state.budget, depth, state, signal, onUpdate, "llm_query");

  const answer = clip(textOf(synthesized.content).trim());
  const details: Details = {
    call: "rlm_query",
    kind: "rlm",
    depth,
    maxDepth: state.maxDepth,
    callsUsed: state.budget.calls,
    maxCalls: state.budget.maxCalls,
    queriesUsed: state.budget.queries,
    maxQueries: state.budget.maxQueries,
    turns: (batchResult.details.turns || 0) + 2, // decompose + synthesize
    maxTurns: state.maxTurns,
    model: batchResult.details.model,
    status: batchResult.details.incomplete ? "partial" : "completed",
    ...budgetDetails(state),
    prompt: params.prompt,
    rootPrompt: params.rootPrompt,
    paths: normPaths(params.paths),
    sources: normSources(params.sources),
    answer,
    incomplete: batchResult.details.incomplete,
  };

  return { content: [{ type: "text", text: answer }], details };
}
