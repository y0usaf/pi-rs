import { promises as fs } from "node:fs";
import * as path from "node:path";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

import {
  DEFAULT_MAX_CALLS,
  DEFAULT_MAX_CONCURRENT,
  DEFAULT_MAX_DEPTH,
  DEFAULT_MAX_QUERIES,
  DEFAULT_MAX_TIMEOUT_MS,
  DEFAULT_MAX_TOKENS,
  DEFAULT_MAX_BUDGET,
  DEFAULT_MAX_ERRORS,
  DEFAULT_MAX_TURNS,
  HARD_MAX_CALLS,
  HARD_MAX_CONCURRENT,
  HARD_MAX_DEPTH,
  HARD_MAX_QUERIES,
  HARD_MAX_TURNS,
  HARD_MAX_TIMEOUT_MS,
  HARD_MAX_TOKENS,
  HARD_MAX_BUDGET,
  HARD_MAX_ERRORS,
  MAX_RESULT_CHARS,
  MAX_TRACE_TEXT_CHARS,
  REPL_TOOL_NAME,
  RLM_CALLS,
} from "./constants.js";
import type { BatchItem, ContextMode, Details, RlmCall, RunState } from "./constants.js";
import { RLM_ITEM_KEYS, RLM_PARAM_KEYS } from "./params.js";
import { renderTemplate, xmlEscape } from "./prompt-render.js";
import { MAX_DEPTH_LEAF_PROMPT } from "./prompts.js";
import { loadRlmSettings, modelSelectorForRole, type RlmModelRole } from "./settings.js";

export interface NamedSourceInput { name?: string; path: string }

// ── Helpers ─────────────────────────────────────────────────────────

export function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

export function rejectUnknownKeys(label: string, value: unknown, allowed: Set<string>): void {
  if (!isRecord(value)) return;
  const unknown = Object.keys(value).filter((key) => !allowed.has(key));
  if (unknown.length > 0) {
    throw new Error(`${label} contains unsupported field(s): ${unknown.join(", ")}. This tool uses a strict schema; only documented RLM fields and aliases are accepted.`);
  }
}

export function rejectUnknownParams(params: unknown): void {
  rejectUnknownKeys("rlm params", params, RLM_PARAM_KEYS);
}

export function rejectUnknownItem(item: unknown, index: number): void {
  rejectUnknownKeys(`rlm batch item ${index}`, item, RLM_ITEM_KEYS);
}

export function clamp(v: unknown, fallback: number, lo: number, hi: number): number {
  if (typeof v !== "number" || !Number.isFinite(v)) return fallback;
  return Math.max(lo, Math.min(hi, Math.trunc(v)));
}

export function optionalClampedLimit(v: unknown, hard: number): number | undefined {
  if (v === undefined || v === null || v === "") return undefined;
  if (typeof v !== "number" || !Number.isFinite(v) || v <= 0) return undefined;
  return Math.max(1, Math.min(hard, Math.trunc(v)));
}

export function configuredLimit(v: unknown, fallback: number, hard: number): number | undefined {
  if (v === undefined || v === null || v === "") return fallback;
  if (typeof v !== "number" || !Number.isFinite(v)) return fallback;
  if (v <= 0) return undefined;
  return Math.max(1, Math.min(hard, Math.trunc(v)));
}

export function clip(text: string, max = MAX_RESULT_CHARS): string {
  if (text.length <= max) return text;
  return `${text.slice(0, max)}\n\n[truncated: ${text.length - max} chars omitted]`;
}

export function normPaths(paths: unknown): string[] {
  if (!Array.isArray(paths)) return [];
  return [...new Set(
    paths.filter((p): p is string => typeof p === "string" && p.trim().length > 0).map((p) => p.trim()),
  )];
}

export function normSources(sources: unknown): NamedSourceInput[] {
  if (!Array.isArray(sources)) return [];
  const out: NamedSourceInput[] = [];
  const seen = new Set<string>();
  for (const src of sources) {
    if (!isRecord(src) || typeof src.path !== "string" || !src.path.trim()) continue;
    const path = src.path.trim();
    const name = typeof src.name === "string" && src.name.trim() ? src.name.trim() : undefined;
    const key = `${name ?? ""}\0${path}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ name, path });
  }
  return out;
}

export function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  const parts: string[] = [];
  for (const c of content) {
    if (!isRecord(c)) continue;
    if (c.type === "text" && typeof c.text === "string") parts.push(c.text);
    else if (c.type === "thinking" && typeof c.thinking === "string") parts.push(c.thinking);
    else if (c.type === "image") parts.push("[image]");
    else if (c.type === "toolCall" && typeof c.name === "string") parts.push(`[toolCall:${c.name}]`);
  }
  return parts.join("\n");
}

export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function isRlmReplToolName(toolName: unknown): boolean {
  return typeof toolName === "string" && toolName.trim().toLowerCase() === REPL_TOOL_NAME.toLowerCase();
}

function isReplFinalResult(m: any): boolean {
  return m?.role === "toolResult" && isRlmReplToolName(m.toolName) && m.details?.final === true;
}

function hasOwn(obj: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(obj, key);
}

function formatStructuredFinalValue(value: unknown): string {
  if (typeof value === "string") return value.trim();
  try {
    return (JSON.stringify(value, null, 2) ?? String(value)).trim();
  } catch {
    return String(value).trim();
  }
}

function replFinalText(m: any): string | undefined {
  const details = m?.details;
  if (details && typeof details === "object") {
    if (typeof details.finalText === "string") return details.finalText.trim();
    if (hasOwn(details, "finalValue")) return formatStructuredFinalValue(details.finalValue);
  }

  // Legacy fallback for pre-variable-only pi-rlm results.
  const t = textOf(m?.content).trim();
  const match = t.match(/(?:^|\n)FINAL:\s*\n?([\s\S]*)$/);
  const legacy = (match?.[1] ?? t).trim();
  return legacy || undefined;
}

export function hasReturn(messages: any[]): boolean {
  return messages.some((m) => isReplFinalResult(m));
}

export function extractAnswer(messages: any[]): string {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (isReplFinalResult(m)) {
      return replFinalText(m) ?? "";
    }
  }
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role === "assistant") {
      const t = textOf(m.content).trim();
      if (t) return t;
    }
  }
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role === "toolResult") {
      const t = textOf(m.content).trim();
      if (t) return t;
    }
  }
  return "(no output)";
}

export function traceOf(messages: any[]) {
  return messages.map((m) => ({
    role: typeof m?.role === "string" ? m.role : "?",
    toolName: typeof m?.toolName === "string" ? m.toolName : undefined,
    text: clip(textOf(m?.content).replace(/\s+/g, " ").trim(), MAX_TRACE_TEXT_CHARS),
  }));
}

export function modelLabel(model: any): string {
  return model ? `${model.provider}/${model.id}` : "unknown";
}

function findConfiguredModel(ctx: ExtensionContext, selector: string) {
  const slash = selector.indexOf("/");
  if (slash > 0) {
    const provider = selector.slice(0, slash);
    const id = selector.slice(slash + 1);
    const found = ctx.modelRegistry.find(provider, id);
    if (found) return found;
  }

  const all = ctx.modelRegistry.getAll();
  return all.find((m: any) => m.id === selector || m.name === selector || `${m.provider}/${m.id}` === selector || `${m.provider}/${m.name}` === selector);
}

export function resolveModel(ctx: ExtensionContext, state: RunState, role: RlmModelRole = "default", override?: string) {
  if (!state.model) state.model = ctx.model;
  const selector = override?.trim() || modelSelectorForRole(loadRlmSettings(ctx.cwd), role);
  if (!selector) return state.model;
  const found = findConfiguredModel(ctx, selector);
  if (found) return found;
  throw new Error(`Unknown pi-rlm model selector ${JSON.stringify(selector)}. Use provider/model-id or a model id/name known to Pi.`);
}

function optionalCap(raw: unknown, fallback: number, hard: number): number {
  if (typeof raw !== "number" || !Number.isFinite(raw) || raw <= 0) return fallback;
  return Math.max(1, Math.min(hard, raw));
}

function timeoutMsFromParams(params: any): number {
  if (typeof params?.maxTimeoutMs === "number") return optionalCap(params.maxTimeoutMs, DEFAULT_MAX_TIMEOUT_MS, HARD_MAX_TIMEOUT_MS);
  const seconds = params?.maxTimeout ?? params?.max_timeout;
  if (typeof seconds === "number" && Number.isFinite(seconds) && seconds > 0) {
    return optionalCap(seconds * 1000, DEFAULT_MAX_TIMEOUT_MS, HARD_MAX_TIMEOUT_MS);
  }
  return DEFAULT_MAX_TIMEOUT_MS;
}

function runId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function createRunState(params: any, model?: any, cwd?: string): RunState {
  const settings = cwd ? loadRlmSettings(cwd) : {};
  return {
    runId: runId(),
    maxDepth: configuredLimit(params?.maxDepth ?? params?.max_depth ?? settings.maxDepth, DEFAULT_MAX_DEPTH, HARD_MAX_DEPTH),
    maxConcurrent: configuredLimit(params?.maxConcurrent ?? params?.max_concurrent_subcalls ?? settings.maxConcurrent, DEFAULT_MAX_CONCURRENT, HARD_MAX_CONCURRENT),
    maxTurns: configuredLimit(params?.maxTurns ?? params?.maxIterations ?? params?.max_iterations, DEFAULT_MAX_TURNS, HARD_MAX_TURNS),
    budget: {
      calls: 0,
      maxCalls: configuredLimit(params?.maxCalls, DEFAULT_MAX_CALLS, HARD_MAX_CALLS),
      queries: 0,
      maxQueries: configuredLimit(params?.maxQueries, DEFAULT_MAX_QUERIES, HARD_MAX_QUERIES),
      tokens: 0,
      maxTokens: optionalCap(params?.maxTokens ?? params?.max_tokens, DEFAULT_MAX_TOKENS, HARD_MAX_TOKENS),
      cost: 0,
      maxBudget: optionalCap(params?.maxBudget ?? params?.max_budget, DEFAULT_MAX_BUDGET, HARD_MAX_BUDGET),
      errors: 0,
      maxErrors: optionalCap(params?.maxErrors ?? params?.max_errors, DEFAULT_MAX_ERRORS, HARD_MAX_ERRORS),
      startTimeMs: Date.now(),
      maxTimeoutMs: timeoutMsFromParams(params),
    },
    model,

  };
}

export function stateFor(params: any, inherited?: RunState, model?: any, cwd?: string): RunState {
  return inherited ?? createRunState(params, model, cwd);
}
export function elapsedMs(state: RunState): number {
  return Math.max(0, Date.now() - state.budget.startTimeMs);
}

export function remainingTimeoutMs(state: RunState): number | undefined {
  if (!state.budget.maxTimeoutMs) return undefined;
  return Math.max(0, state.budget.maxTimeoutMs - elapsedMs(state));
}

export function checkRunLimits(state: RunState): void {
  const b = state.budget;
  if (b.maxTimeoutMs && elapsedMs(state) > b.maxTimeoutMs) throw new Error(`RLM maxTimeoutMs exhausted (${b.maxTimeoutMs}ms).`);
  if (b.maxTokens && b.tokens > b.maxTokens) throw new Error(`RLM maxTokens exhausted (${b.tokens}/${b.maxTokens}).`);
  if (b.maxBudget && b.cost > b.maxBudget) throw new Error(`RLM maxBudget exhausted ($${b.cost.toFixed(6)}/$${b.maxBudget}).`);
  if (b.maxErrors && b.errors >= b.maxErrors) throw new Error(`RLM maxErrors exhausted (${b.errors}/${b.maxErrors}).`);
}

export function recordError(state: RunState): void {
  state.budget.errors++;
  checkRunLimits(state);
}

export interface UsageSummary {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  totalTokens: number;
  cost: number;
}

function n(v: unknown): number {
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}

export function usageSummary(usage: any): UsageSummary {
  const input = n(usage?.input);
  const output = n(usage?.output);
  const cacheRead = n(usage?.cacheRead);
  const cacheWrite = n(usage?.cacheWrite);
  const totalTokens = n(usage?.totalTokens) || input + output + cacheRead + cacheWrite;
  const cost = n(typeof usage?.cost === "number" ? usage.cost : usage?.cost?.total);
  return { input, output, cacheRead, cacheWrite, totalTokens, cost };
}

export function addUsage(a: UsageSummary, b: UsageSummary): UsageSummary {
  return {
    input: a.input + b.input,
    output: a.output + b.output,
    cacheRead: a.cacheRead + b.cacheRead,
    cacheWrite: a.cacheWrite + b.cacheWrite,
    totalTokens: a.totalTokens + b.totalTokens,
    cost: a.cost + b.cost,
  };
}

export function recordUsage(state: RunState, usage: any): UsageSummary {
  const summary = usageSummary(usage);
  state.budget.tokens += summary.totalTokens;
  state.budget.cost += summary.cost;
  checkRunLimits(state);
  return summary;
}

export function usageFromMessages(messages: any[]): UsageSummary {
  return messages.reduce((acc, m) => {
    if (m?.role !== "assistant" || !m.usage) return acc;
    return addUsage(acc, usageSummary(m.usage));
  }, { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: 0 });
}

export function budgetDetails(state: RunState) {
  return {
    tokensUsed: state.budget.tokens,
    maxTokens: state.budget.maxTokens,
    costUsed: state.budget.cost,
    maxBudget: state.budget.maxBudget,
    errorsUsed: state.budget.errors,
    maxErrors: state.budget.maxErrors,
    elapsedMs: elapsedMs(state),
    maxTimeoutMs: state.budget.maxTimeoutMs,
  };
}

export function withTimeoutSignal(signal: AbortSignal | undefined, state: RunState): { signal?: AbortSignal; dispose: () => void } {
  checkRunLimits(state);
  const remaining = remainingTimeoutMs(state);
  if (remaining === undefined) return { signal, dispose: () => undefined };
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const abortFromParent = () => {
    if (!controller.signal.aborted) controller.abort(signal?.reason ?? new Error("Aborted."));
  };
  if (signal?.aborted) abortFromParent();
  else signal?.addEventListener("abort", abortFromParent, { once: true });
  timer = setTimeout(() => {
    if (!controller.signal.aborted) controller.abort(new Error(`RLM maxTimeoutMs exhausted (${state.budget.maxTimeoutMs}ms).`));
  }, Math.max(1, remaining));
  return {
    signal: controller.signal,
    dispose: () => {
      if (timer) clearTimeout(timer);
      signal?.removeEventListener("abort", abortFromParent);
    },
  };
}
function safeLogFilePart(value: string): string {
  return value.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "run";
}

export function configureRunLogging(cwd: string, params: any, state: RunState): void {
  if (state.logPath) return;
  const explicitPath = typeof params?.logPath === "string" && params.logPath.trim() ? params.logPath.trim() : process.env.PI_RLM_LOG_PATH?.trim();
  if (explicitPath) {
    state.logPath = path.isAbsolute(explicitPath) ? explicitPath : path.resolve(cwd, explicitPath);
    return;
  }

  const rawDir = typeof params?.logDir === "string" && params.logDir.trim() ? params.logDir.trim() : process.env.PI_RLM_LOG_DIR?.trim();
  if (!rawDir) return;
  const dir = path.isAbsolute(rawDir) ? rawDir : path.resolve(cwd, rawDir);
  state.logPath = path.join(dir, `${safeLogFilePart(state.runId)}.jsonl`);
}

export async function logEvent(state: RunState, type: string, payload: Record<string, unknown> = {}): Promise<void> {
  if (!state.logPath) return;
  const event = {
    ts: new Date().toISOString(),
    runId: state.runId,
    type,
    ...payload,
  };
  try {
    await fs.mkdir(path.dirname(state.logPath), { recursive: true });
    await fs.appendFile(state.logPath, `${JSON.stringify(event, (_key, value) => typeof value === "string" ? clip(value, 20_000) : value)}\n`, "utf8");
  } catch (e) {
    process.emitWarning?.(`pi-rlm logEvent failed: ${errorText(e)}`);
  }
}

export function currentDepth(parentDepth?: number): number {
  return parentDepth ?? 0;
}

export function childDepth(parentDepth?: number): number {
  return (parentDepth ?? 0) + 1;
}

export function requiredPrompt(params: any): string {
  if (typeof params?.prompt !== "string" || !params.prompt.trim()) {
    throw new Error("Missing required prompt.");
  }
  return params.prompt;
}

export function normalizeCall(raw: unknown): RlmCall {
  if (RLM_CALLS.includes(raw as RlmCall)) return raw as RlmCall;
  throw new Error(`Unknown RLM call: ${String(raw)}. Expected one of: ${RLM_CALLS.join(", ")}.`);
}

export function normalizeContextMode(raw: unknown): ContextMode {
  if (raw === undefined || raw === null || raw === "") return "auto";
  if (CONTEXT_MODES.includes(raw as ContextMode)) return raw as ContextMode;
  throw new Error(`Unknown contextMode: ${String(raw)}. Expected one of: ${CONTEXT_MODES.join(", ")}.`);
}

export function rejectPathsForLlm(call: RlmCall, paths: unknown, contextMode?: unknown, sources?: unknown): void {
  if (call !== "llm_query" && call !== "llm_query_batched") return;
  if (normPaths(paths).length > 0 || normSources(sources).length > 0) {
    throw new Error(`${call} has no REPL environment and cannot consume paths/sources. Extract text first, pass it as context/prompt, or use rlm_query.`);
  }
  if (normalizeContextMode(contextMode) === "file_backed") {
    throw new Error(`${call} has no environment and cannot use contextMode:"file_backed". Use inline context or rlm_query.`);
  }
}

export function singleItemFromParams(params: any): BatchItem {
  const call = normalizeCall(params?.call);
  const contextMode = normalizeContextMode(params?.contextMode);
  rejectPathsForLlm(call, params?.paths, contextMode, params?.sources);
  return {
    prompt: requiredPrompt(params),
    rootPrompt: typeof params?.rootPrompt === "string" ? params.rootPrompt : undefined,
    model: typeof params?.model === "string" ? params.model : undefined,

    context: typeof params?.context === "string" ? params.context : undefined,
    contextMode,
    paths: normPaths(params?.paths),
    sources: normSources(params?.sources),
    contextName: typeof params?.contextName === "string" ? params.contextName : undefined,
  };
}

export function batchItemsFromParams(params: any, call: RlmCall): BatchItem[] {
  const sharedContextMode = normalizeContextMode(params?.contextMode);
  rejectPathsForLlm(call, params?.paths, sharedContextMode, params?.sources);
  const shared = {
    rootPrompt: typeof params?.rootPrompt === "string" ? params.rootPrompt : undefined,
    model: typeof params?.model === "string" ? params.model : undefined,

    context: typeof params?.context === "string" ? params.context : undefined,
    contextMode: sharedContextMode,
    paths: normPaths(params?.paths),
    sources: normSources(params?.sources),
    contextName: typeof params?.contextName === "string" ? params.contextName : undefined,
  };

  if (Array.isArray(params?.items) && params.items.length > 0) {
    return params.items.map((item: any, index: number) => {
      rejectUnknownItem(item, index);
      if (typeof item?.prompt !== "string" || !item.prompt.trim()) {
        throw new Error(`Batch item ${index} missing required prompt.`);
      }
      const contextMode = normalizeContextMode(item?.contextMode ?? shared.contextMode);
      rejectPathsForLlm(call, item?.paths, contextMode, item?.sources);
      const itemPaths = normPaths(item?.paths);
      const itemSources = normSources(item?.sources);
      return {
        prompt: item.prompt,
        rootPrompt: typeof item?.rootPrompt === "string" ? item.rootPrompt : shared.rootPrompt,
        model: typeof item?.model === "string" ? item.model : shared.model,

        context: typeof item?.context === "string" ? item.context : shared.context,
        contextMode,
        paths: itemPaths.length ? itemPaths : shared.paths,
        sources: itemSources.length ? itemSources : shared.sources,
        contextName: typeof item?.contextName === "string" ? item.contextName : shared.contextName,
      };
    });
  }

  if (Array.isArray(params?.prompts) && params.prompts.length > 0) {
    return params.prompts.map((prompt: unknown, index: number) => {
      if (typeof prompt !== "string" || !prompt.trim()) throw new Error(`Prompt ${index} must be a non-empty string.`);
      return { prompt, ...shared };
    });
  }

  throw new Error(`${call} requires prompts or items.`);
}


export async function runLimited<T, R>(items: T[], limit: number, fn: (item: T, index: number) => Promise<R>): Promise<R[]> {
  const results = new Array<R>(items.length);
  let next = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await fn(items[index], index);
    }
  });
  await Promise.all(workers);
  return results;
}

export function modelNameFromDetails(details: Details[]): string {
  const names = [...new Set(details.map((d) => d.model).filter(Boolean))];
  if (names.length === 0) return "unknown";
  if (names.length === 1) return names[0];
  return `mixed(${names.length})`;
}

export function uniquePathsFromDetails(details: Details[]): string[] {
  return [...new Set(details.flatMap((d) => d.paths || []))];
}

export function uniqueSourcesFromDetails(details: Details[]): NamedSourceInput[] {
  return normSources(details.flatMap((d) => d.sources || []));
}

export function leafPrompt(prompt: string, paths?: string[], sources?: unknown): string {
  const ps = normPaths(paths);
  const ss = normSources(sources);
  if (!ps.length && !ss.length) return prompt;

  const pathsBlock = ps.length
    ? `  <paths>
${ps.map((p) => `    <path>${xmlEscape(p)}</path>`).join("\n")}
  </paths>`
    : "";
  const sourcesBlock = ss.length
    ? `  <sources>
${ss.map((s) => `    <source>${xmlEscape(`${s.name ? `${s.name}: ` : ""}${s.path}`)}</source>`).join("\n")}
  </sources>`
    : "";

  return renderTemplate(MAX_DEPTH_LEAF_PROMPT, {
    prompt,
    pathsBlock,
    sourcesBlock,
  });
}
