import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

import { HARD_MAX_CONCURRENT } from "./constants.js";
import type { Details, ExecutionKind } from "./constants.js";
import { runRlmQuery } from "./child-session.js";
import { runLlmQuery } from "./llm.js";
import {
  batchItemsFromParams,
  budgetDetails,
  checkRunLimits,
  clip,
  errorText,
  modelLabel,
  modelNameFromDetails,
  normPaths,
  normSources,
  optionalClampedLimit,
  recordError,
  resolveModel,
  runLimited,
  textOf,
  uniquePathsFromDetails,
  uniqueSourcesFromDetails,
} from "./utils.js";
import type { RunState } from "./constants.js";

// ── Batched calls ───────────────────────────────────────────────────

export async function runBatch(
  ctx: ExtensionContext,
  params: any,
  call: "llm_query_batched" | "rlm_query_batched",
  depth: number,
  state: RunState,
  signal: AbortSignal | undefined,
  onUpdate: any,
): Promise<{ content: Array<{ type: "text"; text: string }>; details: Details }> {
  const primitiveCall = call === "llm_query_batched" ? "llm_query" : "rlm_query";
  const kind: ExecutionKind = call === "llm_query_batched" ? "llm" : "rlm";
  const items = batchItemsFromParams(params, call);
  const rawMaxConcurrent = params?.maxConcurrent ?? params?.max_concurrent_subcalls;
  const requestedMaxConcurrent = optionalClampedLimit(rawMaxConcurrent, HARD_MAX_CONCURRENT);
  const hasMaxConcurrentOverride = typeof rawMaxConcurrent === "number" && Number.isFinite(rawMaxConcurrent);
  const maxConcurrent = hasMaxConcurrentOverride
    ? (requestedMaxConcurrent ?? items.length)
    : (state.maxConcurrent ?? items.length);
  checkRunLimits(state);

  onUpdate?.({ content: [{ type: "text", text: `${call}: ${items.length} item(s), concurrency=${maxConcurrent}` }] });

  const results = await runLimited(items, maxConcurrent, async (item, index) => {
    try {
      if (signal?.aborted) throw new Error("Aborted.");
      onUpdate?.({ content: [{ type: "text", text: `${call}: item ${index + 1}/${items.length}` }] });
      if (call === "llm_query_batched") {
        return await runLlmQuery(ctx, item, state.budget, depth, state, signal, onUpdate, "llm_query");
      }
      return await runRlmQuery(ctx, item, depth, state, signal, onUpdate);
    } catch (e) {
      try { recordError(state); } catch { /* keep original error details */ }
      const model = resolveModel(ctx, state);
      const msg = `Error: ${errorText(e)}`;
      const details: Details = {
        call: primitiveCall,
        kind,
        depth,
        maxDepth: state.maxDepth,
        callsUsed: state.budget.calls,
        maxCalls: state.budget.maxCalls,
        queriesUsed: state.budget.queries,
        maxQueries: state.budget.maxQueries,
        turns: 0,
        maxTurns: kind === "rlm" ? state.maxTurns : 0,
        model: modelLabel(model),
        status: "error",
        ...budgetDetails(state),
        prompt: item.prompt,
        rootPrompt: item.rootPrompt,

        paths: normPaths(item.paths),
        sources: normSources(item.sources),
        answer: msg,
        error: errorText(e),
        incomplete: true,
      };
      return { content: [{ type: "text" as const, text: msg }], details };
    }
  });

  const childDetails = results.map((r) => r.details);
  const body = results
    .map((r, i) => {
      const text = textOf(r.content).trim();
      const prompt = clip(items[i].prompt.replace(/\s+/g, " ").trim(), 160);
      return `## ${i + 1}. ${prompt}\n\n${text}`;
    })
    .join("\n\n---\n\n");

  const answer = clip(body);
  const details: Details = {
    call,
    kind,
    depth,
    maxDepth: state.maxDepth,
    callsUsed: state.budget.calls,
    maxCalls: state.budget.maxCalls,
    queriesUsed: state.budget.queries,
    maxQueries: state.budget.maxQueries,
    turns: childDetails.reduce((sum, d) => sum + (d.turns || 0), 0),
    maxTurns: kind === "rlm" ? state.maxTurns : 0,
    model: modelNameFromDetails(childDetails),
    prompt: `${call} (${items.length} item${items.length === 1 ? "" : "s"})`,
    paths: uniquePathsFromDetails(childDetails),
    sources: uniqueSourcesFromDetails(childDetails),
    status: childDetails.some((d) => d.incomplete || d.status === "partial" || d.status === "error" || d.status === "aborted" || d.status === "budget_exhausted") ? "partial" : "completed",
    ...budgetDetails(state),
    answer,
    batch: true,
    batchSize: items.length,
    maxConcurrent,
    results: childDetails,
    incomplete: childDetails.some((d) => d.incomplete || d.status === "partial"),
  };

  return { content: [{ type: "text", text: answer }], details };
}

