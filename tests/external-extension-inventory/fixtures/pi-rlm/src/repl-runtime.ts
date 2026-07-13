import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createInterface } from "node:readline";

import { dispatchRlmCall } from "./dispatcher.js";
import { inheritSessionContextParams } from "./session-context.js";
import { MAX_RESULT_CHARS, RLM_CALLS } from "./constants.js";
import type { ContextStore, RunState } from "./constants.js";
import { REPL_PARAM_KEYS } from "./params.js";
import { clip, errorText, isRecord, rejectUnknownKeys, textOf } from "./utils.js";

const PYTHON_WORKER = String.raw`
import ast as _ast
import json as _json
import sys as _sys
import traceback as _traceback

_ORIG_STDIN = _sys.stdin
_ORIG_STDOUT = _sys.stdout
_logs = []
_call_seq = 0
_final_called = False
_final_value = None
_final_name = None
_last = None
state = {}
history = []
context = None
context_0 = None
_context_count = 0
_context_keys = set()
_RESERVED_VALUES = {}

class _Capture:
    def write(self, text):
        if text:
            _logs.append(str(text))
        return len(text) if text else 0
    def flush(self):
        pass

_sys.stdout = _Capture()
_sys.stderr = _Capture()

def _send(obj):
    _ORIG_STDOUT.write(_json.dumps(obj, ensure_ascii=False, default=str) + "\n")
    _ORIG_STDOUT.flush()

def _call(method, params=None):
    global _call_seq
    _call_seq += 1
    call_id = _call_seq
    _send({"type": "call", "id": call_id, "method": method, "params": params or {}})
    while True:
        line = _ORIG_STDIN.readline()
        if not line:
            raise RuntimeError("RLM REPL bridge closed")
        msg = _json.loads(line)
        if msg.get("type") != "call_result" or msg.get("id") != call_id:
            raise RuntimeError("Unexpected bridge response: " + repr(msg))
        if msg.get("ok"):
            return msg.get("result")
        raise RuntimeError(msg.get("error") or "RLM REPL bridge call failed")

def _require_prompt(prompt, name):
    if not isinstance(prompt, str) or not prompt.strip():
        raise TypeError(name + " expects a non-empty prompt string")
    return prompt

def _require_prompts(prompts, name):
    if not isinstance(prompts, list) or not all(isinstance(p, str) and p.strip() for p in prompts):
        raise TypeError(name + " expects a list of non-empty prompt strings")
    return prompts

def _single_params(prompt, model=None):
    params = {"prompt": prompt}
    if model is not None:
        params["model"] = model
    return params

def _batch_params(prompts, model=None):
    params = {"prompts": prompts}
    if model is not None:
        params["model"] = model
    return params

def _text(result):
    if isinstance(result, dict):
        return result.get("text", "")
    return str(result)

def _batch_answers(result):
    details = result.get("details") if isinstance(result, dict) else None
    child_results = details.get("results") if isinstance(details, dict) else None
    if isinstance(child_results, list):
        return [d.get("answer", "") if isinstance(d, dict) else "" for d in child_results]
    if isinstance(result, list):
        return [str(x) for x in result]
    return [_text(result)]

def llm_query(prompt, model=None):
    return _text(_call("llm_query", _single_params(_require_prompt(prompt, "llm_query"), model)))

def llm_query_batched(prompts, model=None):
    return _batch_answers(_call("llm_query_batched", _batch_params(_require_prompts(prompts, "llm_query_batched"), model)))

def rlm_query(prompt, model=None):
    return _text(_call("rlm_query", _single_params(_require_prompt(prompt, "rlm_query"), model)))

def rlm_query_batched(prompts, model=None):
    return _batch_answers(_call("rlm_query_batched", _batch_params(_require_prompts(prompts, "rlm_query_batched"), model)))

def _set_final(value, name=None):
    global _final_called, _final_value, _final_name, _last
    _final_called = True
    _final_value = value
    _final_name = name
    _last = value
    return value

def FINAL_VAR(variable_name):
    if not isinstance(variable_name, str) or not variable_name.strip():
        raise TypeError("FINAL_VAR(name) requires a variable/state key string")
    name = variable_name.strip().strip("\"'")
    g = globals()
    if name in g and not _is_protected_name(name):
        return _set_final(g[name], name)
    if name in state:
        return _set_final(state[name], name)
    available = _visible_var_keys()
    raise KeyError(name + " is not defined. Available variables: " + repr(available))

def SHOW_VARS():
    available = {k: type(globals()[k]).__name__ for k in _visible_var_keys()}
    if not available:
        return "No variables created yet. Use Python code to create variables."
    return "Available variables: " + repr(available)

_HELPER_NAMES = {
    "llm_query",
    "llm_query_batched",
    "rlm_query",
    "rlm_query_batched",
    "FINAL_VAR",
    "SHOW_VARS",
}
_PROTECTED_DATA_NAMES = {"state", "history", "context"}

def _is_context_name(name):
    return name.startswith("context_") or name.startswith("history_")

def _is_protected_name(name):
    return name in _HELPER_NAMES or name in _PROTECTED_DATA_NAMES or _is_context_name(name)

def _visible_var_keys():
    keys = []
    for key in globals().keys():
        if key.startswith("_") or key in _HELPER_NAMES:
            continue
        keys.append(key)
    return sorted(keys)

def _safe_repr(value, limit=1000):
    try:
        text = repr(value)
    except Exception as exc:
        text = "<repr failed: " + str(exc) + ">"
    return text if len(text) <= limit else text[:limit] + "..."

def _compile_user(code):
    tree = _ast.parse(code, filename="<pi-rlm-repl>", mode="exec")
    captures_expr = bool(tree.body and isinstance(tree.body[-1], _ast.Expr))
    if captures_expr:
        expr = tree.body[-1]
        tree.body[-1] = _ast.Assign(targets=[_ast.Name(id="_last", ctx=_ast.Store())], value=expr.value)
        _ast.fix_missing_locations(tree)
    return compile(tree, "<pi-rlm-repl>", "exec"), captures_expr

def _refresh_reserved_values():
    global _RESERVED_VALUES
    names = set(_HELPER_NAMES) | _PROTECTED_DATA_NAMES | {k for k in globals().keys() if _is_context_name(k)}
    _RESERVED_VALUES = {k: globals().get(k) for k in names if k in globals()}

def _restore_reserved_values():
    for k, v in _RESERVED_VALUES.items():
        globals()[k] = v

def _context_key(entry, index):
    if isinstance(entry, dict):
        key = entry.get("key")
        if isinstance(key, str) and key:
            return key
    return "context:" + str(index) + ":" + _safe_repr(entry, 200)

def _context_value(entry):
    if isinstance(entry, dict) and "value" in entry:
        return entry.get("value")
    return entry

def _clear_contexts():
    global context, context_0, _context_count
    for key in list(globals().keys()):
        if key == "context" or key.startswith("context_"):
            try:
                del globals()[key]
            except Exception:
                pass
    context = None
    context_0 = None
    _context_count = 0
    _context_keys.clear()


def _load_contexts(entries):
    global context, context_0, _context_count
    if entries is None:
        return
    _clear_contexts()
    if not isinstance(entries, list):
        entries = [{"key": "context", "value": entries}]
    for index, entry in enumerate(entries):
        key = _context_key(entry, index)
        if key in _context_keys:
            continue
        value = _context_value(entry)
        name = "context_" + str(_context_count)
        globals()[name] = value
        if _context_count == 0:
            context_0 = value
            context = value
            globals()["context_0"] = value
            globals()["context"] = value
        _context_count += 1
        _context_keys.add(key)
    _refresh_reserved_values()

def _inject_data(data):
    if not isinstance(data, dict):
        return
    for key, value in data.items():
        if not isinstance(key, str) or not key.isidentifier() or key.startswith("_") or _is_protected_name(key):
            raise ValueError("Invalid or reserved injected variable name: " + repr(key))
        globals()[key] = value

_refresh_reserved_values()

def _run_eval(msg):
    global _final_called, _final_value, _final_name, _last, history
    eval_id = msg.get("id")
    code = msg.get("code") or ""
    setup = msg.get("setup") or ""
    if msg.get("resetHistory"):
        history.clear()
    _load_contexts(msg.get("contexts"))
    _inject_data(msg.get("data"))
    _refresh_reserved_values()
    _logs.clear()
    _final_called = False
    _final_value = None
    _final_name = None
    try:
        if setup:
            exec(compile(setup, "<pi-rlm-repl-setup>", "exec"), globals(), globals())
            _restore_reserved_values()
        compiled, captures_expr = _compile_user(code)
        exec(compiled, globals(), globals())
        value = _final_value if _final_called else (_last if captures_expr else None)
        _send({
            "type": "result",
            "id": eval_id,
            "ok": True,
            "final": _final_called,
            "finalName": _final_name,
            "value": value,
            "logs": "".join(_logs),
            "stateKeys": sorted(str(k) for k in state.keys()),
            "varKeys": _visible_var_keys(),
            "historyLength": len(history),
            "contextKeys": sorted(k for k in globals().keys() if k == "context" or k.startswith("context_")),
        })
        _restore_reserved_values()
        _refresh_reserved_values()
    except Exception as exc:
        _send({
            "type": "result",
            "id": eval_id,
            "ok": False,
            "error": str(exc),
            "traceback": _traceback.format_exc(),
            "logs": "".join(_logs),
            "stateKeys": sorted(str(k) for k in state.keys()),
            "varKeys": _visible_var_keys(),
            "historyLength": len(history),
            "contextKeys": sorted(k for k in globals().keys() if k == "context" or k.startswith("context_")),
        })
        _restore_reserved_values()
        _refresh_reserved_values()

_send({"type": "ready"})

while True:
    _line = _ORIG_STDIN.readline()
    if not _line:
        break
    try:
        _msg = _json.loads(_line)
        if _msg.get("type") == "eval":
            _run_eval(_msg)
        elif _msg.get("type") == "shutdown":
            break
    except Exception:
        _send({"type": "worker_error", "error": _traceback.format_exc()})
`;

interface PythonEvalResult {
  ok: boolean;
  final?: boolean;
  finalName?: string;
  value?: unknown;
  logs?: string;
  error?: string;
  traceback?: string;
  stateKeys?: string[];
  varKeys?: string[];
  historyLength?: number;
  contextKeys?: string[];
}

interface BridgeContext {
  ctx: any;
  signal?: AbortSignal;
  onUpdate?: any;
  inherited?: RunState;
  parentDepth?: number;
  store?: ContextStore;
}

export type ReplStoreProvider = ContextStore | ((ctx: any) => ContextStore | undefined | Promise<ContextStore | undefined>);

type FinalOutputEmitter = (output: { text: string; variableName?: string; toolCallId?: string; timestamp: number }) => void | Promise<void>;

export async function resolveReplStore(provider: ReplStoreProvider | undefined, ctx: any): Promise<ContextStore | undefined> {
  if (!provider) return undefined;
  if (typeof provider === "function") return await provider(ctx);
  return provider;
}

export async function contextEntriesFromStore(store?: ContextStore): Promise<Array<{ key: string; value: unknown }> | undefined> {
  if (!store?.sources.length) return undefined;
  const entries: Array<{ key: string; value: unknown }> = [];
  for (const source of store.sources) {
    const key = `${source.id}:${source.path}:${source.sizeBytes ?? ""}:${source.entries ?? ""}`;
    if (source.kind === "inline" || source.kind === "file") {
      try {
        const text = await readFile(source.path, "utf8");
        source.sizeBytes = Buffer.byteLength(text, "utf8");
        entries.push({ key, value: text });
      } catch (e) {
        entries.push({ key, value: { path: source.path, relPath: source.relPath, kind: source.kind, error: errorText(e) } });
      }
      continue;
    }
    entries.push({
      key,
      value: {
        path: source.path,
        relPath: source.relPath,
        kind: source.kind,
        label: source.label,
        name: source.name,
        error: source.error,
      },
    });
  }
  return entries;
}

interface PendingEval {
  resolve: (value: PythonEvalResult) => void;
  reject: (err: Error) => void;
  timeoutMs: number;
  remainingMs: number;
  timerStartedAt?: number;
  timeout?: ReturnType<typeof setTimeout>;
  onAbort?: () => void;
  controller?: AbortController;
}

export function abortErrorText(signal?: AbortSignal): string {
  const reason = signal?.reason;
  if (reason instanceof Error) return reason.message;
  if (typeof reason === "string" && reason.trim()) return reason;
  return "Aborted.";
}

export function composeAbortSignal(a?: AbortSignal, b?: AbortSignal): AbortSignal | undefined {
  const signals = [a, b].filter(Boolean) as AbortSignal[];
  if (signals.length === 0) return undefined;
  if (signals.length === 1) return signals[0];
  const anyFn = (AbortSignal as any).any;
  if (typeof anyFn === "function") return anyFn(signals);

  const controller = new AbortController();
  const abort = (signal: AbortSignal) => {
    if (!controller.signal.aborted) controller.abort(signal.reason);
  };
  for (const signal of signals) {
    if (signal.aborted) abort(signal);
    else signal.addEventListener("abort", () => abort(signal), { once: true });
  }
  return controller.signal;
}

export function pythonCommand(): string {
  return process.env.PI_RLM_PYTHON?.trim() || "python3";
}


export function objectExtra(extra: unknown): Record<string, unknown> {
  return isRecord(extra) ? extra : {};
}

export function rejectUnknownReplParams(params: unknown): void {
  rejectUnknownKeys("repl params", params, REPL_PARAM_KEYS);
}

export function renderCodePreview(code: unknown): string {
  if (typeof code !== "string" || !code.trim()) return "...";
  const first = code.trim().split("\n").find((line) => line.trim().length > 0) ?? code.trim();
  return clip(first.replace(/\s+/g, " "), 100);
}

export function formatPythonValue(value: unknown): string {
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2) ?? String(value);
}

export function finalStoredMessage(variableName: unknown): string {
  const name = typeof variableName === "string" && variableName.trim() ? variableName.trim() : undefined;
  return name ? `[final stored in REPL variable: ${name}]` : "[final stored in REPL variable]";
}

export function splitFinalOutput(text: string): { preFinal: string; final?: string } {
  const match = text.match(/(?:^|\n)FINAL:\s*\n?([\s\S]*)$/);
  if (!match || match.index === undefined) return { preFinal: text.trim() };
  return {
    preFinal: text.slice(0, match.index).trim(),
    final: (match[1] ?? "").trim(),
  };
}

export class PythonReplWorker {
  private proc: any;
  private rl: any;
  private nextEvalId = 1;
  private pending = new Map<number, PendingEval>();
  private current?: BridgeContext;
  private currentEvalId?: number;
  private stderr = "";
  private exited = false;

  constructor(private cwd: string) {
    const cmd = pythonCommand();
    this.proc = spawn(cmd, ["-u", "-c", PYTHON_WORKER], { cwd, stdio: ["pipe", "pipe", "pipe"] });
    this.rl = createInterface({ input: this.proc.stdout });
    this.rl.on("line", (line: string) => this.handleLine(line));
    this.proc.stderr.on("data", (chunk: Buffer) => {
      this.stderr = clip(this.stderr + chunk.toString("utf8"), MAX_RESULT_CHARS);
    });
    this.proc.stdin?.on?.("error", (err: any) => {
      if (err?.code === "EPIPE" || err?.code === "ERR_STREAM_DESTROYED") {
        this.exited = true;
        this.failAll(new Error("Python REPL stdin closed."));
        return;
      }
      this.failAll(err instanceof Error ? err : new Error(errorText(err)));
    });
    this.proc.on("error", (err: Error) => this.failAll(err));
    this.proc.on("exit", (code: number | null, signal: string | null) => {
      this.exited = true;
      this.failAll(new Error(`Python REPL exited (${signal ?? code ?? "unknown"}).${this.stderr ? ` stderr: ${this.stderr}` : ""}`));
    });
  }

  isAlive(): boolean {
    return !this.exited && !this.proc.killed && !this.proc.stdin?.destroyed;
  }

  async eval(code: string, timeoutMs: number, bridge: BridgeContext, options: { data?: unknown; setup?: string; resetHistory?: boolean } = {}): Promise<PythonEvalResult> {
    if (!this.isAlive()) throw new Error("Python REPL is not running.");
    if (this.current) throw new Error("Python REPL is already evaluating code.");
    if (bridge.signal?.aborted) {
      this.kill();
      throw new Error(abortErrorText(bridge.signal));
    }

    const id = this.nextEvalId++;
    const evalController = new AbortController();
    const evalSignal = composeAbortSignal(bridge.signal, evalController.signal);
    const evalBridge: BridgeContext = { ...bridge, signal: evalSignal };
    this.current = evalBridge;
    this.currentEvalId = id;

    const contexts = await contextEntriesFromStore(bridge.store);

    let pendingForCleanup: PendingEval | undefined;
    return await new Promise<PythonEvalResult>((resolve, reject) => {
      const pending: PendingEval = {
        resolve,
        reject,
        timeoutMs,
        remainingMs: timeoutMs,
        controller: evalController,
      };
      pendingForCleanup = pending;
      const onAbort = () => {
        evalController.abort(bridge.signal?.reason ?? new Error("Aborted."));
        this.kill();
        reject(new Error(abortErrorText(bridge.signal)));
      };
      pending.onAbort = onAbort;

      bridge.signal?.addEventListener("abort", onAbort, { once: true });
      this.pending.set(id, pending);
      this.armEvalTimeout(id, pending);
      if (!this.write({ type: "eval", id, code, contexts, data: options.data, setup: options.setup, resetHistory: options.resetHistory })) {
        this.failAll(new Error("Python REPL stdin is closed."));
      }
    }).finally(() => {
      const pending = this.pending.get(id) ?? pendingForCleanup;
      if (pending?.timeout) clearTimeout(pending.timeout);
      if (pending) pending.timeout = undefined;
      if (pending?.onAbort) bridge.signal?.removeEventListener("abort", pending.onAbort);
      this.pending.delete(id);
      if (this.current === evalBridge) this.current = undefined;
      if (this.currentEvalId === id) this.currentEvalId = undefined;
    });
  }

  kill(): void {
    if (this.isAlive()) this.proc.kill("SIGKILL");
    this.exited = true;
  }

  shutdown(): void {
    if (!this.isAlive()) return;
    this.write({ type: "shutdown" });
    this.proc.kill();
  }

  private timeoutError(pending: PendingEval): Error {
    return new Error(
      `Python REPL local evaluation timed out after ${pending.timeoutMs}ms (time spent inside bridge helper calls is excluded).`,
    );
  }

  private armEvalTimeout(id: number, pending?: PendingEval): void {
    if (!pending || !this.pending.has(id)) return;
    if (pending.timeout) clearTimeout(pending.timeout);
    const delay = Math.max(1, pending.remainingMs);
    pending.timerStartedAt = Date.now();
    pending.timeout = setTimeout(() => {
      if (!this.pending.has(id)) return;
      pending.timeout = undefined;
      const err = this.timeoutError(pending);
      pending.controller?.abort(err);
      this.kill();
      pending.reject(err);
    }, delay);
  }

  private pauseEvalTimeout(pending?: PendingEval): void {
    if (!pending?.timeout) return;
    clearTimeout(pending.timeout);
    pending.timeout = undefined;
    if (pending.timerStartedAt !== undefined) {
      pending.remainingMs = Math.max(0, pending.remainingMs - (Date.now() - pending.timerStartedAt));
      pending.timerStartedAt = undefined;
    }
  }

  private write(obj: unknown): boolean {
    if (!this.isAlive()) return false;
    const stdin = this.proc.stdin;
    if (!stdin || stdin.destroyed || !stdin.writable) return false;
    try {
      stdin.write(`${JSON.stringify(obj)}\n`);
      return true;
    } catch (e: any) {
      if (e?.code === "EPIPE" || e?.code === "ERR_STREAM_DESTROYED") {
        this.exited = true;
        return false;
      }
      throw e;
    }
  }

  private handleLine(line: string): void {
    let msg: any;
    try {
      msg = JSON.parse(line);
    } catch {
      this.stderr = clip(`${this.stderr}\n[non-json stdout] ${line}`, MAX_RESULT_CHARS);
      return;
    }

    if (msg?.type === "ready") return;
    if (msg?.type === "call") {
      void this.handleBridgeCall(msg);
      return;
    }
    if (msg?.type === "result") {
      const pending = this.pending.get(Number(msg.id));
      if (!pending) return;
      if (pending.timeout) clearTimeout(pending.timeout);
      pending.timeout = undefined;
      pending.timerStartedAt = undefined;
      pending.resolve(msg as PythonEvalResult);
      return;
    }
    if (msg?.type === "worker_error") {
      this.stderr = clip(`${this.stderr}\n${msg.error ?? "worker_error"}`, MAX_RESULT_CHARS);
    }
  }

  private async handleBridgeCall(msg: any): Promise<void> {
    const bridge = this.current;
    const evalId = this.currentEvalId;
    const pending = evalId === undefined ? undefined : this.pending.get(evalId);
    if (!bridge || !this.isAlive()) return;

    this.pauseEvalTimeout(pending);
    let response: { ok: true; result: unknown } | { ok: false; error: string };
    try {
      response = { ok: true, result: await handleBridgeCall(msg.method, msg.params, bridge) };
    } catch (e) {
      response = { ok: false, error: errorText(e) };
    }

    if (this.current !== bridge || !this.isAlive() || bridge.signal?.aborted) return;
    if (!this.write({ type: "call_result", id: msg.id, ...response })) {
      this.failAll(new Error("Python REPL stdin is closed."));
      return;
    }
    if (evalId !== undefined) this.armEvalTimeout(evalId, pending);
  }


  private failAll(err: Error): void {
    for (const pending of this.pending.values()) {
      if (pending.timeout) clearTimeout(pending.timeout);
      pending.controller?.abort(err);
      pending.reject(err);
    }
    this.pending.clear();
    this.current = undefined;
    this.currentEvalId = undefined;
  }
}

async function handleBridgeCall(method: unknown, params: unknown, bridge: BridgeContext): Promise<unknown> {
  const p = objectExtra(params);
  const call = typeof method === "string" && RLM_CALLS.includes(method as any) ? method : undefined;
  if (call) {
    const paramsForDispatch = inheritSessionContextParams({ ...p, call }, bridge.store);
    const result = await dispatchRlmCall(bridge.ctx, paramsForDispatch, bridge.inherited, bridge.parentDepth, bridge.signal, bridge.onUpdate);
    return { text: textOf(result.content).trim(), content: result.content, details: result.details };
  }

  throw new Error(`Unknown Python REPL bridge method: ${String(method)}.`);
}
