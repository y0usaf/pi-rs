import { DefaultResourceLoader, SessionManager, SettingsManager, createAgentSession, getAgentDir, type ExtensionContext } from "@earendil-works/pi-coding-agent";

import type { ContextMode, Details, RunState } from "./constants.js";
import { cleanupContextStore, contextSourceSummary, prepareContextStore } from "./context-store.js";
import { childPrompt, childSystemPrompt, childToolList, deterministicFinalPrompt } from "./child-prompts.js";
import { tryStructuralDecompose } from "./child-decompose.js";
import { runLlmQuery } from "./llm.js";
import { budgetDetails, checkRunLimits, clip, extractAnswer, hasReturn, isRlmReplToolName, normalizeContextMode, normPaths, normSources, resolveModel, modelLabel, recordError, recordUsage, textOf, traceOf, usageFromMessages, withTimeoutSignal, leafPrompt } from "./utils.js";

    // ── Main entry point ─────────────────────────────────────────────────

    export async function runRlmQuery(
      ctx: ExtensionContext,
      params: { prompt: string; rootPrompt?: string; model?: string; context?: string; contextMode?: ContextMode; paths?: string[]; sources?: Array<{ name?: string; path: string }>; contextName?: string },
      depth: number,
      state: RunState,
      signal: AbortSignal | undefined,
      onUpdate: any,
    ): Promise<{ content: Array<{ type: "text"; text: string }>; details: Details }> {
      const contextMode = normalizeContextMode(params.contextMode);
      checkRunLimits(state);

      // RLM semantics: at max depth, rlm_query falls back to a plain LM leaf call.
      if (state.maxDepth !== undefined && depth >= state.maxDepth) {
        return runLlmQuery(ctx, {
          prompt: leafPrompt(params.prompt, params.paths, params.sources),
          rootPrompt: params.rootPrompt,

          model: params.model,
          context: params.context,
          contextMode,
          paths: params.paths,
          sources: params.sources,
          contextName: params.contextName,
        }, state.budget, depth, state, signal, onUpdate, "rlm_query");
      }

      // ── Structural decomposition: try auto-decompose before interactive session ──
      const structuralResult = await tryStructuralDecompose(ctx, { ...params, contextMode }, depth, state, signal, onUpdate);
      if (structuralResult) return structuralResult;

      // ── Interactive child session (single-unit tasks) ──

      state.budget.calls++;
      if (state.budget.maxCalls !== undefined && state.budget.calls > state.budget.maxCalls) throw new Error(`Max recursive child RLM calls (${state.budget.maxCalls}).`);

      const model = resolveModel(ctx, state, "rlm", params.model);
      if (!model) throw new Error("Cannot resolve current session model for RLM call.");


      const contextStore = await prepareContextStore(ctx.cwd, { ...params, contextMode });
      const timed = withTimeoutSignal(signal, state);
      const effectiveSignal = timed.signal;
      const hasContextStore = Boolean(contextStore);
      const tools = childToolList();
      const systemPrompt = childSystemPrompt(depth, state, hasContextStore);

      let session: any | undefined;
      let unsub: (() => void) | undefined;
      let turns = 0;
      let abortedByTurnLimit = false;
      let finalizationRequested = false;

      const sourceSummaries = contextStore?.sources.map(contextSourceSummary) ?? [];
      const emit = (text: string) =>
        onUpdate?.({
          content: [{ type: "text", text }],
          details: {
            call: "rlm_query" as const,
            kind: "rlm" as const,
            depth,
            maxDepth: state.maxDepth,
            callsUsed: state.budget.calls,
            maxCalls: state.budget.maxCalls,
            queriesUsed: state.budget.queries,
            maxQueries: state.budget.maxQueries,
            turns,
            maxTurns: state.maxTurns,
            model: modelLabel(model),
            status: "partial" as const,
            ...budgetDetails(state),
            prompt: params.prompt,
            rootPrompt: params.rootPrompt,

            paths: normPaths(params.paths),
            sources: normSources(params.sources),
            contextMode,
            scratchDir: contextStore?.scratchDir,
            contextSources: sourceSummaries,
            finalizationRequested,
          },
        });

      const kill = () => {
        if (session) void session.abort();
      };

      try {
        const loader = new DefaultResourceLoader({
          cwd: ctx.cwd,
          agentDir: getAgentDir(),
          noExtensions: true,
          noSkills: true,
          noPromptTemplates: true,
          noThemes: true,
          noContextFiles: true,
          // A recursive child must be a true RLM child, not a normal Pi agent with
          // the default Pi prompt plus an appended note. Pass the child prompt as
          // the actual system prompt/instructions payload and suppress appends so
          // providers such as OpenAI Codex Responses receive `instructions`.
          systemPrompt,
          appendSystemPrompt: [],
          systemPromptOverride: () => systemPrompt,
          appendSystemPromptOverride: () => [],
        });
        await loader.reload();

        const { createRlmReplTool } = await import("./repl.js");
        const customTools: any[] = [createRlmReplTool(state, depth, contextStore)];

        const created = await createAgentSession({
          cwd: ctx.cwd,
          agentDir: getAgentDir(),
          authStorage: ctx.modelRegistry.authStorage,
          modelRegistry: ctx.modelRegistry,
          model,
          resourceLoader: loader,
          sessionManager: SessionManager.inMemory(),
          settingsManager: SettingsManager.inMemory({ compaction: { enabled: false } }),
          noTools: "all",
          tools,
          customTools,
        });
        session = created.session;

        const activeTools = typeof session.getActiveToolNames === "function" ? session.getActiveToolNames() : tools;
        if (activeTools.length !== 1 || !isRlmReplToolName(activeTools[0])) {
          throw new Error(`Child RLM session must be REPL-only; active tools were: ${activeTools.join(", ") || "(none)"}`);
        }

        unsub = session.subscribe((ev: any) => {
          if (ev.type === "tool_execution_start") {
            emit(`depth ${depth}: ${ev.toolName}...`);
          } else if (ev.type === "turn_end") {
            turns++;
            emit(`depth ${depth}: turn ${turns}${state.maxTurns === undefined ? "" : `/${state.maxTurns}`}`);
            const ret = Array.isArray(ev.toolResults) && ev.toolResults.some((r: any) => isRlmReplToolName(r?.toolName) && r?.details?.final === true);
            const more = ev.message?.stopReason === "toolUse" && !ret;
            if (state.maxTurns !== undefined && turns >= state.maxTurns && more) {
              abortedByTurnLimit = true;
              emit(`depth ${depth}: turn budget reached; aborting child for deterministic parent-side finalization`);
              void session.abort();
            }
          }
        });

        if (effectiveSignal?.aborted) kill();
        else effectiveSignal?.addEventListener("abort", kill, { once: true });

        emit(`depth ${depth}: starting${contextStore ? ` with context (${contextStore.sources.length} source${contextStore.sources.length === 1 ? "" : "s"})` : ""}`);
        await session.prompt(childPrompt(params.prompt, params.context, params.paths, contextStore, params.rootPrompt), { expandPromptTemplates: false, source: "extension" });

        let msgs = [...(session.messages as any[])];
        let completed = hasReturn(msgs);
        let deterministicFinalized = false;
        let deterministicFinalizationReason: string | undefined;
        let answer = "";

        if (completed) {
          answer = clip(extractAnswer(msgs));
        } else if (!effectiveSignal?.aborted) {
          finalizationRequested = true;
          deterministicFinalized = true;
          deterministicFinalizationReason = abortedByTurnLimit ? `maxTurns=${state.maxTurns}` : `missing FINAL_VAR`;
          emit(`depth ${depth}: synthesizing deterministic final answer (${deterministicFinalizationReason})`);
          const synthesized = await runLlmQuery(ctx, {
            prompt: deterministicFinalPrompt(params.prompt, msgs, deterministicFinalizationReason),
            rootPrompt: params.rootPrompt,
            model: params.model,

            contextMode: "inline",
          }, state.budget, depth, state, effectiveSignal, onUpdate, "rlm_query");
          answer = clip(textOf(synthesized.content).trim() || extractAnswer(msgs));
        } else {
          answer = clip(extractAnswer(msgs));
        }

        const usage = recordUsage(state, usageFromMessages(msgs));
        if (!completed) {
          try { recordError(state); } catch { /* keep synthesized partial details */ }
        }
        const incomplete = !completed;
        const details: Details = {
          call: "rlm_query",
          kind: "rlm",
          depth,
          maxDepth: state.maxDepth,
          callsUsed: state.budget.calls,
          maxCalls: state.budget.maxCalls,
          queriesUsed: state.budget.queries,
          maxQueries: state.budget.maxQueries,
          turns,
          maxTurns: state.maxTurns,
          model: modelLabel(model),
          status: effectiveSignal?.aborted ? "aborted" : incomplete ? "partial" : "completed",
          ...budgetDetails(state),
          prompt: params.prompt,
          rootPrompt: params.rootPrompt,

          usage,
          paths: normPaths(params.paths),
          sources: normSources(params.sources),
          contextMode,
          scratchDir: contextStore?.scratchDir,
          contextSources: sourceSummaries,
          answer,
          trace: traceOf(msgs),
          completedWithReturn: completed,
          finalizationRequested,
          deterministicFinalized,
          deterministicFinalizationReason,
          abortedByTurnLimit,
          incomplete,
        };

        const note = abortedByTurnLimit
          ? `\n\n[stopped after maxTurns=${state.maxTurns}; synthesized checkpoint may be partial]`
          : !completed
            ? deterministicFinalized
              ? `\n\n[child ended without FINAL_VAR; synthesized checkpoint from transcript]`
              : `\n\n[child ended without FINAL_VAR; using last available text]`
            : "";
        return { content: [{ type: "text", text: `${answer}${note}` }], details };
      } finally {
        effectiveSignal?.removeEventListener("abort", kill);
        timed.dispose();
        unsub?.();
        session?.dispose();
        await cleanupContextStore(contextStore);
      }
    }
