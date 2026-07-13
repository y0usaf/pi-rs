import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

import { runBatch } from "./batch.js";
import { runRlmQuery } from "./child-session.js";
import type { RunState } from "./constants.js";
import { runLlmQuery } from "./llm.js";

import {
  configureRunLogging,
  logEvent,
  childDepth,
  currentDepth,
  normalizeCall,
  rejectUnknownParams,
  singleItemFromParams,
  stateFor,
} from "./utils.js";

export async function dispatchRlmCall(
  ctx: ExtensionContext,
  params: any,
  inherited?: RunState,
  parentDepth?: number,
  signal?: AbortSignal,
  onUpdate?: any,
) {
  rejectUnknownParams(params);
  const state = stateFor(params, inherited, ctx.model, ctx.cwd);
  configureRunLogging(ctx.cwd, params, state);
  await logEvent(state, "dispatch_start", { call: params.call, depth: parentDepth ?? 0, prompt: typeof params.prompt === "string" ? params.prompt : undefined });
  const call = normalizeCall(params.call);

  try {
    if (call === "llm_query") {
      const result = await runLlmQuery(ctx, singleItemFromParams(params), state.budget, currentDepth(parentDepth), state, signal, onUpdate, "llm_query");
      await logEvent(state, "dispatch_end", { call, depth: currentDepth(parentDepth), details: result.details });
      return result;
    }
    if (call === "llm_query_batched") {
      const result = await runBatch(ctx, params, "llm_query_batched", currentDepth(parentDepth), state, signal, onUpdate);
      await logEvent(state, "dispatch_end", { call, depth: currentDepth(parentDepth), details: result.details });
      return result;
    }
    if (call === "rlm_query") {
      const result = await runRlmQuery(ctx, singleItemFromParams(params), childDepth(parentDepth), state, signal, onUpdate);
      await logEvent(state, "dispatch_end", { call, depth: childDepth(parentDepth), details: result.details });
      return result;
    }
    const result = await runBatch(ctx, params, "rlm_query_batched", childDepth(parentDepth), state, signal, onUpdate);
    await logEvent(state, "dispatch_end", { call, depth: childDepth(parentDepth), details: result.details });
    return result;
  } catch (e) {
    await logEvent(state, "dispatch_error", { call, depth: parentDepth ?? 0, error: e instanceof Error ? e.message : String(e) });
    throw e;
  }
}
