import { defineTool } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";

import { contextSourceSummary } from "./context-store.js";
import { MAX_RESULT_CHARS, REPL_TOOL_NAME } from "./constants.js";
import type { RunState } from "./constants.js";
import { ReplParams, REPL_PARAM_KEYS } from "./params.js";
import { renderTemplate, splitPromptBlock } from "./prompt-render.js";
import { REPL_TOOL_PROMPT_GUIDELINES, REPL_TOOL_PROMPT_SNIPPET } from "./prompts.js";
import { PythonReplWorker, resolveReplStore, type ReplStoreProvider, renderCodePreview, formatPythonValue, finalStoredMessage, splitFinalOutput } from "./repl-runtime.js";
import { clip, rejectUnknownKeys, textOf } from "./utils.js";

export function createRlmReplTool(inherited?: RunState, parentDepth?: number, store?: ReplStoreProvider, emitFinalOutput?: (output: { text: string; variableName?: string; toolCallId?: string; timestamp: number }) => void | Promise<void>) {
  let worker: PythonReplWorker | undefined;
  let workerCwd: string | undefined;
  let evals = 0;

  return defineTool({
    name: REPL_TOOL_NAME,
    label: REPL_TOOL_NAME,
    description: "Python REPL using the upstream RLM helper contract: llm_query, llm_query_batched, rlm_query, rlm_query_batched, FINAL_VAR, SHOW_VARS, state/history/context variables, and injected custom data.",
    promptSnippet: renderTemplate(REPL_TOOL_PROMPT_SNIPPET, { toolName: REPL_TOOL_NAME }),
    promptGuidelines: splitPromptBlock(renderTemplate(REPL_TOOL_PROMPT_GUIDELINES, { toolName: REPL_TOOL_NAME })),
    parameters: ReplParams,
    async execute(toolCallId, params, signal, onUpdate, ctx) {
      rejectUnknownKeys("repl params", params, REPL_PARAM_KEYS);
      if (params.reset === true) {
        worker?.shutdown();
        worker = undefined;
        workerCwd = undefined;
      }
      if (typeof params.code !== "string" || !params.code.trim()) throw new Error("Missing required code.");

      const timeoutMs = Math.max(100, Math.min(120_000, Math.trunc(typeof params.timeoutMs === "number" && Number.isFinite(params.timeoutMs) ? params.timeoutMs : 30_000)));
      if (!worker || !worker.isAlive() || workerCwd !== ctx.cwd) {
        worker?.shutdown();
        worker = new PythonReplWorker(ctx.cwd);
        workerCwd = ctx.cwd;
      }
      evals++;

      onUpdate?.({ content: [{ type: "text", text: `${REPL_TOOL_NAME}: evaluating Python via ${process.env.PI_RLM_PYTHON?.trim() || "python3"} (${timeoutMs}ms local timeout; bridge calls excluded)...` }], details: { kind: "repl", language: "python", evals, final: false, timeoutMs, cwd: ctx.cwd } });

      const effectiveStore = await resolveReplStore(store, ctx);
      const result = await worker.eval(params.code, timeoutMs, { ctx, signal, onUpdate, inherited, parentDepth, store: effectiveStore }, { data: params.data, setup: params.setup, resetHistory: params.resetHistory === true });
      if (!result.ok) {
        const text = clip([result.logs?.trim(), result.traceback || result.error].filter(Boolean).join("\n\n"), MAX_RESULT_CHARS);
        return {
          content: [{ type: "text", text }],
          details: {
            kind: "repl",
            language: "python",
            evals,
            final: false,
            timeoutMs,
            cwd: ctx.cwd,
            stateKeys: result.stateKeys ?? [],
            varKeys: result.varKeys ?? [],
            historyLength: result.historyLength ?? 0,
            contextKeys: result.contextKeys ?? [],
            error: result.error,
            scratchDir: effectiveStore?.scratchDir,
            contextSources: effectiveStore?.sources.map(contextSourceSummary),
          },
        };
      }

      const sections: string[] = [];
      const finalText = result.final === true ? formatPythonValue(result.value).trim() : undefined;
      const logsText = result.logs?.trim();
      if (logsText) sections.push(`Console:\n${logsText}`);
      if (result.final) sections.push(finalStoredMessage(result.finalName));
      else if (result.value !== undefined && result.value !== null) sections.push(`Result:\n${formatPythonValue(result.value)}`);
      if (sections.length === 0) sections.push("(no output)");

      let finalMirrored = false;
      if (result.final === true && ctx.hasUI && emitFinalOutput) {
        try {
          await emitFinalOutput({
            text: finalText ?? "",
            variableName: result.finalName,
            toolCallId,
            timestamp: Date.now(),
          });
          finalMirrored = true;
        } catch {
          finalMirrored = false;
        }
      }

      const text = clip(sections.join("\n\n"), MAX_RESULT_CHARS);
      return {
        content: [{ type: "text", text }],
        details: {
          kind: "repl",
          language: "python",
          evals,
          final: result.final === true,
          finalName: result.finalName,
          finalVar: result.finalName,
          finalText,
          finalValue: result.final === true ? result.value : undefined,
          finalMirrored,
          timeoutMs,
          cwd: ctx.cwd,
          stateKeys: result.stateKeys ?? [],
          varKeys: result.varKeys ?? [],
          historyLength: result.historyLength ?? 0,
          contextKeys: result.contextKeys ?? [],
          scratchDir: effectiveStore?.scratchDir,
          contextSources: effectiveStore?.sources.map(contextSourceSummary),
        },
        terminate: result.final === true,
      };
    },
    renderCall(args, theme) {
      return new Text(
        `${theme.fg("toolTitle", theme.bold(REPL_TOOL_NAME))} ${theme.fg("muted", renderCodePreview(args?.code))}`,
        0,
        0,
      );
    },
    renderResult(result, { isPartial }: any, theme) {
      const text = textOf(result.content).trim();
      const details: any = result.details ?? {};
      if (isPartial) return new Text(theme.fg("warning", text || "running..."), 0, 0);
      const final = details.final ? theme.fg("success", " FINAL") : "";
      const mirrored = details.finalMirrored ? theme.fg("muted", " → rlm_final") : "";
      const vars = Array.isArray(details.varKeys) && details.varKeys.length
        ? ` vars=${details.varKeys.join(",")}`
        : Array.isArray(details.stateKeys) && details.stateKeys.length
          ? ` state=${details.stateKeys.join(",")}`
          : "";
      const err = details.error ? theme.fg("error", " error") : "";
      const { preFinal } = splitFinalOutput(text);
      const body = details.finalMirrored
        ? [
            preFinal ? theme.fg("toolOutput", clip(preFinal.replace(/\s+/g, " "), 800)) : "",
            theme.fg("muted", "mirrored as rlm_final"),
          ].filter(Boolean).join(" ")
        : theme.fg("toolOutput", clip(text.replace(/\s+/g, " "), 800));
      const newline = String.fromCharCode(10);
      return new Text(
        `${theme.fg("success", "✓")} ${theme.fg("toolTitle", theme.bold(REPL_TOOL_NAME))}${final}${mirrored}${err}${theme.fg("muted", vars)}${newline}${body}`,
        0,
        0,
      );
    },
  });
}
