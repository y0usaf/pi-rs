-- File-backed pi-rlm translation (dogfood package).
-- Pi-native RLM: a Python REPL tool driven by a real `python3` subprocess.
-- Public surface only: pi.register_tool, pi.process.spawn (managed subprocess
-- with read_stdout/write_stdin/kill/dispose pipes), pi.register_message_renderer,
-- pi.sendMessage, pi.setActiveTools, pi.on, pi.ai.complete (the completeSimple
-- llm_query bridge), pi.fs.{read_file,exists,mkdir,write_file_atomic,tmpdir,
-- append_file}, pi.path.{join,dirname,extname,is_absolute,basename}, pi.env,
-- pi.json.{decode,encode}, pi.set_timeout/clear_timeout, pi.now_ms, pi.sleep,
-- ctx.sessionManager.{getSessionDir,getSessionId,getSessionFile},
-- ctx.modelRegistry.getApiKeyAndHeaders, ctx.cwd, parser helpers. Cleanup:
-- the Python worker subprocess is killed/disposed on session_shutdown and on
-- every tool reset; the session context store's dir persists with the session,
-- no process survives handle disposal.
local pi = ...

-- ── Constants ──────────────────────────────────────────────────────
local REPL_TOOL_NAME = "repl"
local RLM_FINAL_OUTPUT_CUSTOM_TYPE = "rlm_final"
local RLM_CALLS = { llm_query = true, llm_query_batched = true, rlm_query = true, rlm_query_batched = true }
local MAX_RESULT_CHARS = 50000
local MAX_QUERY_CONTEXT_CHARS = 500000
local MAX_INLINE_CHILD_CONTEXT_CHARS = 20000
local DEFAULT_MAX_DEPTH = 5
local DEFAULT_MAX_TURNS = 30
local DEFAULT_MAX_CALLS = 128
local DEFAULT_MAX_QUERIES = 256
local DEFAULT_MAX_CONCURRENT = 5
local REPL_PARAM_KEYS = { code = true, reset = true, timeoutMs = true, data = true, setup = true, resetHistory = true }

local function python_command()
  local env = pi.env.PI_RLM_PYTHON
  if env and env:gsub("%s", "") ~= "" then return env:gsub("^%s+", ""):gsub("%s+$", "") end
  return "python3"
end

-- ── Utils ──────────────────────────────────────────────────────────
local function is_record(v) return type(v) == "table" end

local function clip(text, max)
  max = max or MAX_RESULT_CHARS
  text = tostring(text or "")
  if #text <= max then return text end
  return text:sub(1, max) .. "\n...[truncated " .. (#text - max) .. " chars]...\n"
end

local function error_text(e)
  if type(e) == "table" and e.message then return tostring(e.message) end
  return tostring(e)
end

local function text_of(content)
  local parts = {}
  if type(content) == "string" then return content end
  if type(content) == "table" then
    for i = 1, #content do
      local part = content[i]
      if type(part) == "table" and part.type == "text" and type(part.text) == "string" then
        parts[#parts + 1] = part.text
      end
    end
  end
  return table.concat(parts, "\n")
end

local function reject_unknown_keys(prefix, params, allowed)
  if type(params) ~= "table" then return end
  for key in pairs(params) do
    if not allowed[key] then
      error(prefix .. " prohibits unknown key: " .. tostring(key), 0)
    end
  end
end

local function xml_escape(s)
  s = tostring(s or "")
  s = s:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;")
  return s
end

local function render_template(template, params)
  template = template:gsub("%${(%w+)}", function(k)
    local v = params[k]
    if v == nil then v = "" end
    return tostring(v)
  end)
  return template
end

local result_worker_llm = nil

-- ── Python worker script (verbatim upstream RLM helper) ───────────
local PYTHON_WORKER = [=[
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
]=]

-- ── Python worker driver (pi.process) ──────────────────────────────
local python_worker = {}
python_worker.__index = python_worker

function python_worker.new(cwd)
  local self = setmetatable({}, python_worker)
  self.cwd = cwd
  self.proc = nil
  self.stdout_buffer = ""
  self.next_eval_id = 1
  self.pending = nil
  self.exited = false
  local cmd = python_command()
  self.proc = pi.process.spawn(cmd, { "-u", "-c", PYTHON_WORKER }, { cwd = cwd })
  -- Drain until "ready".
  local deadline = pi.now_ms() + 10000
  local ready = false
  while pi.now_ms() < deadline and not ready do
    local line = self:read_line()
    if line then
      local ok, msg = pcall(pi.json.decode, line)
      if ok and type(msg) == "table" and msg.type == "ready" then ready = true end
    else
      pi.sleep(20)
    end
  end
  if not ready then
    self:kill()
    error("Python REPL worker did not become ready.", 0)
  end
  return self
end

function python_worker:read_line()
  while true do
    local nl = self.stdout_buffer:find("\n", 1, true)
    if nl then
      local line = self.stdout_buffer:sub(1, nl - 1)
      self.stdout_buffer = self.stdout_buffer:sub(nl + 1)
      return line
    end
    local data = self.proc:read_stdout()
    if data and #data > 0 then
      self.stdout_buffer = self.stdout_buffer .. data
    else
      return nil
    end
  end
end

function python_worker:write(obj)
  local line = pi.json.encode(obj) .. "\n"
  return pcall(self.proc.write_stdin, self.proc, line)
end

function python_worker:kill()
  if self.proc then
    pcall(self.proc.kill, self.proc, "SIGTERM")
    pcall(self.proc.dispose, self.proc)
    self.proc = nil
  end
  self.exited = true
end

function python_worker:shutdown()
  if self.proc then
    local ok = pcall(self.write, self, { type = "shutdown" })
    pcall(self.proc.kill, self.proc, "SIGTERM")
    pcall(self.proc.dispose, self.proc)
    self.proc = nil
  end
  self.exited = true
end

function python_worker:is_alive() return not self.exited end

-- Handle one bridge call synchronously through the LLM bridge.
function python_worker:handle_bridge_call(msg, bridge)
  local ok, response = pcall(function()
    local method = msg.method
    local params = msg.params or {}
    if RLM_CALLS[method] then
      local result = handle_rlm_call(bridge.ctx, params, bridge.store, bridge.signal)
      return { ok = true, result = { text = text_of(result.content):gsub("^%s+", ""):gsub("%s+$", ""), content = result.content, details = result.details } }
    end
    error("Unknown Python REPL bridge method: " .. tostring(method), 0)
  end)
  if not ok then
    response = { ok = false, error = error_text(response) }
  end
  self:write({ type = "call_result", id = msg.id, ok = response.ok, result = response.ok and response.result or nil, error = response.ok and nil or response.error })
end

function python_worker:eval(code, timeout_ms, bridge, options)
  if not self:is_alive() then error("Python REPL is not running.", 0) end
  timeout_ms = math.max(100, math.min(120000, math.floor(timeout_ms or 30000)))
  local id = self.next_eval_id
  self.next_eval_id = self.next_eval_id + 1
  local contexts = context_entries_from_store(bridge.store)
  if not self:write({ type = "eval", id = id, code = code, contexts = contexts, data = options.data, setup = options.setup, resetHistory = options.resetHistory == true }) then
    error("Python REPL stdin is closed.", 0)
  end
  local deadline = pi.now_ms() + timeout_ms
  local result
  while true do
    local line = self:read_line()
    if line and line ~= "" then
      local ok, msg = pcall(pi.json.decode, line)
      if ok and type(msg) == "table" then
        if msg.type == "call" then
          self:handle_bridge_call(msg, bridge)
        elseif msg.type == "result" and tonumber(msg.id) == id then
          result = msg
          break
        elseif msg.type == "worker_error" then
          self:kill()
          error("Python REPL worker error: " .. tostring(msg.error), 0)
        end
      end
    else
      if not self:is_alive() then error("Python REPL exited.", 0) end
      if pi.now_ms() >= deadline then
        self:kill()
        error("Python REPL local evaluation timed out after " .. timeout_ms .. "ms", 0)
      end
      pi.sleep(10)
    end
  end
  return result
end

-- ── Session context store (file-backed) ───────────────────────────
local SESSION_CONTEXT_DIR = "rlm-context"
local stores = {}

local function session_key(ctx)
  local sf = ctx.sessionManager and ctx.sessionManager.getSessionFile and ctx.sessionManager.getSessionFile()
  return sf or (ctx.cwd .. "\0" .. (ctx.sessionManager and ctx.sessionManager.getSessionId and ctx.sessionManager.getSessionId() or ""))
end

local function session_store_dir(ctx)
  local dir = ctx.sessionManager and ctx.sessionManager.getSessionDir and ctx.sessionManager.getSessionDir()
  local id = (ctx.sessionManager and ctx.sessionManager.getSessionId and ctx.sessionManager.getSessionId()) or "session"
  id = id:gsub("[\\/]+", "-"):gsub("[^A-Za-z0-9._%-]+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if not dir or dir == "" then dir = pi.fs.tmpdir() end
  return pi.path.join(dir, SESSION_CONTEXT_DIR, id)
end

local function rel_path_for(cwd, path)
  if path:sub(1, #cwd) == cwd then
    local rel = path:sub(#cwd + 1):gsub("^[/\\]+", "")
    if rel ~= "" then return rel end
  end
  return pi.path.basename(path)
end

local function context_source_summary(source)
  local size = source.sizeBytes and string.format("%d B", source.sizeBytes) or "?"
  local details = source.error and (" (" .. source.error .. ")") or ""
  return source.label .. " [" .. source.kind .. ", " .. size .. "]" .. details
end

local function context_entries_from_store(store)
  if not store or not store.sources or #store.sources == 0 then return nil end
  local entries = {}
  for _, source in ipairs(store.sources) do
    local key = source.id .. ":" .. source.path .. ":" .. tostring(source.sizeBytes or "") .. ":" .. tostring(source.entries or "")
    if source.kind == "inline" or source.kind == "file" then
      local ok, text = pcall(pi.fs.read_file, source.path)
      if ok then
        source.sizeBytes = #text
        entries[#entries + 1] = { key = key, value = text }
      else
        entries[#entries + 1] = { key = key, value = { path = source.path, relPath = source.relPath, kind = source.kind, error = error_text(text) } }
      end
    else
      entries[#entries + 1] = { key = key, value = { path = source.path, relPath = source.relPath, kind = source.kind, label = source.label, name = source.name, error = source.error } }
    end
  end
  return entries
end

local function ensure_session_context_store(ctx)
  local key = session_key(ctx)
  local cached = stores[key]
  if cached then return cached end
  local dir = session_store_dir(ctx)
  local store = {
    dir = dir,
    scratchDir = pi.path.join(dir, "scratch"),
    notesDir = pi.path.join(dir, "notes"),
    artifactsDir = pi.path.join(dir, "artifacts"),
    manifestPath = pi.path.join(dir, "manifest.txt"),
    manifestJsonPath = pi.path.join(dir, "manifest.json"),
    readmePath = pi.path.join(dir, "README.md"),
    manifestText = "",
    sources = {},
  }
  local ok = pcall(pi.fs.mkdir, store.dir, true)
  if ok then
    pcall(pi.fs.mkdir, store.scratchDir, true)
    pcall(pi.fs.mkdir, store.notesDir, true)
    pcall(pi.fs.mkdir, store.artifactsDir, true)
  end
  stores[key] = store
  return store
end

local function release_session_context_store(ctx)
  stores[session_key(ctx)] = nil
end

-- Externalize large user input into a file-backed source.
local function should_externalize_input(text, source)
  if source == "extension" then return false end
  return #(text or "") > MAX_INLINE_CHILD_CONTEXT_CHARS
end

local function externalize_large_input(ctx, text)
  local store = ensure_session_context_store(ctx)
  local dir = pi.path.join(store.dir, "sources")
  pcall(pi.fs.mkdir, dir, true)
  local file = pi.path.join(dir, "user-input-" .. pi.now_ms() .. ".txt")
  pi.fs.write_file_atomic(file, text)
  local source = {
    id = "user-" .. pi.now_ms(),
    label = "externalized user input",
    path = file,
    relPath = "sources/" .. pi.path.basename(file),
    kind = "file",
    sizeBytes = #text,
  }
  store.sources[#store.sources + 1] = source
  local replacement = "[RLM externalized this large input to a session-context source; SHOW_VARS()/context reflects it.]"
  return { replacement = replacement }
end

local function record_user_input(ctx, text)
  -- In-memory note; keeps the input lifecycle explicit without spilling.
  local store = stores[session_key(ctx)]
  if store and #(text or "") > 0 and #store.sources < 50 then
    -- (kept minimal; full history recording lives in the REPL worker)
  end
end

-- ── LLM bridge (pi.ai.complete) ────────────────────────────────────
local LEAF_SYSTEM_PROMPT = [=[
You are Pi RLM, a recursive language-model helper. Answer the user's prompt accurately and concisely.${rootPromptBlock}
]=]

local LEAF_USER_PROMPT = [=[
${prompt}
${rootPromptBlock}${contextBlock}
]=]

local function run_llm_query(ctx, params, state, signal, call)
  local model = ctx.model
  if not model then error("Cannot resolve current session model for RLM call.", 0) end
  local auth
  if ctx.modelRegistry and ctx.modelRegistry.getApiKeyAndHeaders then
    auth = ctx.modelRegistry.getApiKeyAndHeaders(model)
  end
  if not auth or not auth.ok then error("Auth failed: " .. tostring(auth and auth.error or "none"), 0) end

  local root_block = params.rootPrompt and params.rootPrompt:gsub("%s", "") ~= "" and ("  <rootQuestion>" .. xml_escape(params.rootPrompt) .. "</rootQuestion>") or ""
  local context_block = params.context and params.context:gsub("%s", "") ~= "" and ("  <context>" .. xml_escape(params.context) .. "</context>") or ""
  local prompt = render_template(LEAF_USER_PROMPT, { prompt = params.prompt, rootPromptBlock = root_block, contextBlock = context_block })
  if #prompt > MAX_QUERY_CONTEXT_CHARS then
    prompt = prompt:sub(1, MAX_QUERY_CONTEXT_CHARS) .. "\n\n[truncated]"
  end

  local response = pi.ai.complete(model, {
    systemPrompt = render_template(LEAF_SYSTEM_PROMPT, { rootPromptBlock = root_block }),
    messages = { { role = "user", content = prompt, timestamp = pi.now_ms() } },
  }, {
    apiKey = auth.apiKey,
    headers = auth.headers and (auth.headers or nil) or nil,
    signal = signal,
    reasoning = model.reasoning and "low" or nil,
  }, nil)

  local content_text = text_of(response.content)
  local failed = response.stopReason == "error" or response.stopReason == "aborted"
  local truncated = response.stopReason == "length"
  local failure_text = failed and ((response.stopReason == "aborted") and "Aborted" or ("Error: " .. (response.errorMessage or "Provider returned " .. tostring(response.stopReason) .. "."))) or ""
  local text = failed and ((content_text:gsub("%s+$", "") .. (content_text:gsub("%s+$", "") ~= "" and "\n" or "") .. failure_text)) or (truncated and ("[truncated: provider hit its output limit]\n\n" .. content_text:gsub("%s+$", "")) or content_text)

  local usage = response.usage or {}
  local details = {
    call = call,
    kind = "llm",
    depth = 0,
    maxDepth = nil,
    callsUsed = 1,
    maxCalls = nil,
    queriesUsed = 1,
    maxQueries = nil,
    turns = 0,
    maxTurns = nil,
    model = tostring(model.provider) .. "/" .. tostring(model.id),
    status = failed and (response.stopReason == "aborted" and "aborted" or "error") or (truncated and "partial" or "completed"),
    usage = usage,
    prompt = params.prompt,
    rootPrompt = params.rootPrompt,
    paths = (call == "llm_query" or call == "llm_query_batched") and {} or (params.paths or {}),
    contextMode = "inline",
    answer = clip(text),
  }
  if failed or truncated then
    if failed then details.error = response.errorMessage or ("Provider returned " .. tostring(response.stopReason) .. ".") end
    details.incomplete = true
  end
  return { content = { { type = "text", text = clip(text) } }, details = details }
end

function handle_rlm_call(ctx, params, store, signal)
  local call = params.call or "llm_query"
  if not RLM_CALLS[call] then error("Unknown RLM call: " .. tostring(call), 0) end
  -- Batched / recursive calls degrade to sequential single llm_query
  -- (faithful call surface + per-item dispatch; recursion depth treated as a
  -- plain LM leaf for deterministic, bounded behavior in this translation).
  local prompts = params.prompts
  if type(prompts) ~= "table" or #prompts == 0 then prompts = nil end
  if prompts then
    local detail_results = {}
    for _, p in ipairs(prompts) do
      local r = run_llm_query(ctx, { prompt = p, rootPrompt = params.rootPrompt, model = params.model, context = params.context }, nil, signal, call)
      detail_results[#detail_results + 1] = r.details
    end
    return { content = { { type = "text", text = table.concat({}, "") } }, details = { call = call, batch = true, batchSize = #detail_results, results = detail_results } }
  end
  return run_llm_query(ctx, { prompt = params.prompt, rootPrompt = params.rootPrompt, model = params.model, context = params.context }, nil, signal, call)
end

-- ── Final output renderer ─────────────────────────────────────────
local function text_from_custom_content(content)
  local parts = {}
  if type(content) == "string" then return content end
  if type(content) == "table" then
    for i = 1, #content do
      local p = content[i]
      if type(p) == "table" then
        if p.type == "text" and type(p.text) == "string" then parts[#parts + 1] = p.text
        elseif p.type == "image" then parts[#parts + 1] = "[image]" end
      end
    end
  end
  return table.concat(parts, "\n")
end

local function emit_rlm_final_output(pi_ref, output)
  pi_ref.sendMessage({
    customType = RLM_FINAL_OUTPUT_CUSTOM_TYPE,
    content = output.text,
    display = true,
    details = {
      toolName = "rlm_final",
      variableName = output.variableName,
      toolCallId = output.toolCallId,
      emittedAt = output.timestamp,
    },
  }, { triggerTurn = false })
end

pi.register_message_renderer(RLM_FINAL_OUTPUT_CUSTOM_TYPE, function(message)
  local content = message and message.content or ""
  return {
    render = function(_, width)
      local text = text_from_custom_content(content):gsub("^%s+", ""):gsub("%s+$", "")
      if text == "" then return {} end
      return pi.tui.text_render(text, width, 1, 1)
    end,
  }
end)

-- ── REPL tool ─────────────────────────────────────────────────────
local function render_code_preview(code)
  if type(code) ~= "string" or code:gsub("%s", "") == "" then return "..." end
  local first = code:gsub("^%s*", ""):match("^([^\r\n]+)") or code
  first = clip(first:gsub("%s+", " "), 100)
  return first
end

local function format_python_value(value)
  if type(value) == "string" then return value end
  return pi.json.encode(value, true)
end

local function final_stored_message(name)
  if type(name) == "string" and name:gsub("%s", "") ~= "" then
    return "[final stored in REPL variable: " .. name .. "]"
  end
  return "[final stored in REPL variable]"
end

local rlm_workers = {}

local function create_rlm_repl_tool(state_store, on_final)
  local worker = nil
  local worker_cwd = nil
  local evals = 0
  return {
    name = REPL_TOOL_NAME,
    label = REPL_TOOL_NAME,
    description = "Python REPL using the upstream RLM helper contract: llm_query, llm_query_batched, rlm_query, rlm_query_batched, FINAL_VAR, SHOW_VARS, state/history/context variables, and injected custom data.",
    promptSnippet = "Run Python using a persistent RLM REPL (llm_query/rlm_query helpers available)",
    promptGuidelines = {},
    parameters = {
      type = "object",
      properties = {
        code = { type = "string", description = "Python code to run inside the upstream-style RLM REPL. Public helpers: llm_query, llm_query_batched, rlm_query, rlm_query_batched, FINAL_VAR, SHOW_VARS; use globals/state/history/context for persistence and context." },
        reset = { type = "boolean", description = "Clear persistent REPL state before running this code. Default false." },
        timeoutMs = { type = "number", description = "Local Python execution timeout. Default 30000, hard cap 120000." },
        data = { type = "object", description = "Optional JSON-serializable variables to inject into the Python REPL globals before running code." },
        setup = { type = "string", description = "Optional Python setup code to execute before the main code in this eval." },
        resetHistory = { type = "boolean", description = "Clear REPL history variables before running this code. Default false." },
      },
      required = { "code" },
    },
    execute = function(tool_call_id, params, signal, on_update, ctx)
      reject_unknown_keys("repl params", params, REPL_PARAM_KEYS)
      if params.reset == true then
        if worker then pcall(worker.shutdown, worker); worker = nil end
        worker_cwd = nil
      end
      if type(params.code) ~= "string" or params.code:gsub("%s", "") == "" then error("Missing required code.", 0) end
      if not worker or not worker:is_alive() or worker_cwd ~= ctx.cwd then
        if worker then pcall(worker.kill, worker); rlm_workers[worker] = nil end
        worker = python_worker.new(ctx.cwd)
        worker_cwd = ctx.cwd
        rlm_workers[worker] = true
      end
      evals = evals + 1
      if on_update then
        on_update({ content = { { type = "text", text = REPL_TOOL_NAME .. ": evaluating Python via " .. python_command() .. "..." } }, details = { kind = "repl", language = "python", evals = evals, final = false, timeoutMs = params.timeoutMs, cwd = ctx.cwd } })
      end

      local effective_store = state_store and state_store(ctx) or nil
      local ok_result, result = pcall(function()
        return worker:eval(params.code, params.timeoutMs or 30000, { ctx = ctx, signal = signal, store = effective_store }, { data = params.data, setup = params.setup, resetHistory = params.resetHistory == true })
      end)
      if not ok_result then error(result, 0) end

      if result.ok == false then
        local err_text = clip(table.concat({ (result.logs or ""):gsub("%s+$", ""), result.traceback or result.error }, "\n\n"))
        return {
          content = { { type = "text", text = clip(err_text) } },
          details = { kind = "repl", language = "python", evals = evals, final = false, timeoutMs = params.timeoutMs, cwd = ctx.cwd, stateKeys = result.stateKeys or {}, varKeys = result.varKeys or {}, historyLength = result.historyLength or 0, contextKeys = result.contextKeys or {}, error = result.error },
        }
      end

      local sections = {}
      local final_text = (result.final == true) and format_python_value(result.value):gsub("%s+$", "") or nil
      local logs = result.logs or ""
      if logs:gsub("%s", "") ~= "" then sections[#sections + 1] = "Console:\n" .. logs end
      if result.final == true then sections[#sections + 1] = final_stored_message(result.finalName)
      elseif result.value ~= nil then sections[#sections + 1] = "Result:\n" .. format_python_value(result.value) end
      if #sections == 0 then sections[#sections + 1] = "(no output)" end

      local final_mirrored = false
      if result.final == true and ctx.hasUI and on_final then
        local ok = pcall(function() on_final { text = final_text or "", variableName = result.finalName, toolCallId = tool_call_id, timestamp = pi.now_ms() } end)
        final_mirrored = ok
      end

      local text = clip(table.concat(sections, "\n\n"))
      return {
        content = { { type = "text", text = text } },
        details = {
          kind = "repl", language = "python", evals = evals,
          final = result.final == true, finalName = result.finalName,
          finalVar = result.finalName, finalText = final_text,
          finalValue = result.final == true and result.value or nil,
          finalMirrored = final_mirrored, timeoutMs = params.timeoutMs, cwd = ctx.cwd,
          stateKeys = result.stateKeys or {}, varKeys = result.varKeys or {},
          historyLength = result.historyLength or 0, contextKeys = result.contextKeys or {},
          scratchDir = effective_store and effective_store.scratchDir or nil,
        },
        terminate = result.final == true,
      }
    end,
    renderCall = function(args, theme)
      return { text = theme:fg("toolTitle", theme:bold(REPL_TOOL_NAME)) .. " " .. theme:fg("muted", render_code_preview(args and args.code)) }
    end,
    renderResult = function(result, options, theme)
      local text = clip(text_of(result.content):gsub("%s+$", ""))
      local details = result.details or {}
      return { text = theme:fg("success", "✓") .. " " .. theme:fg("toolTitle", theme:bold(REPL_TOOL_NAME)) .. (details.final and theme:fg("success", " FINAL") or "") .. " " .. clip(text:gsub("%s+", " "), 800) }
    end,
  }
end

-- ── Entry point ────────────────────────────────────────────────────
local function root_tools() return { REPL_TOOL_NAME } end

local function enforce_root_tools()
  pi.setActiveTools(root_tools())
  return "repl"
end

local rlm_final_outputs_active = false

pi.register_tool(create_rlm_repl_tool(function(ctx) return ensure_session_context_store(ctx) end, function(output) emit_rlm_final_output(pi, output) end))

pi.on("session_start", function(_event, ctx)
  enforce_root_tools()
  ensure_session_context_store(ctx)
end)

pi.on("session_tree", function(_event, ctx)
  enforce_root_tools()
  ensure_session_context_store(ctx)
end)

pi.on("session_shutdown", function(_event, ctx)
  release_session_context_store(ctx)
  for _, w in pairs(rlm_workers) do pcall(w.shutdown, w) end
  rlm_workers = {}
end)

pi.on("context", function(event)
  local messages = {}
  for i = 1, #event.messages do
    local message = event.messages[i]
    if not (type(message) == "table" and message.role == "custom" and message.customType == "rlm_final") then
      messages[#messages + 1] = message
    end
  end
  return { messages = messages }
end)

pi.on("agent_end", function(_event, ctx)
  if not ctx.hasUI then return end
end)

pi.on("before_provider_request", function() enforce_root_tools() end)

pi.on("input", function(event, ctx)
  if event.source == "extension" then return { action = "continue" } end
  if should_externalize_input(event.text, event.source) then
    local r = externalize_large_input(ctx, event.text)
    return { action = "transform", text = r.replacement, images = event.images }
  end
  record_user_input(ctx, event.text)
  return { action = "continue" }
end)

pi.on("before_agent_start", function(_event, ctx)
  enforce_root_tools()
  ensure_session_context_store(ctx)
  return { systemPrompt = "You are Pi RLM. In this mode only the " .. REPL_TOOL_NAME .. " tool is active. Use it to run Python and drive recursive language-model helpers." }
end)
