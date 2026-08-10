-- prime/rlm.lua — RLM agent loop as Lua policy (P3 core).
--
-- Port of prime-agent's RLM loop shape onto the pi-rs public surface:
--   - the turn loop: prompt -> stream (pi.ai.stream_simple) -> tool calls
--     (python via the kernel bridge, bash, rlm.run subagent spawn) ->
--     tool results -> repeat until the model stops requesting tools;
--   - the persistent IPython kernel as the built-in model tool;
--   - recursive subagents: rlm.run spawns a child session at depth+1 with
--     its own kernel, running the same loop, admitted immediately with a
--     spawn handle (results arrive via the child's session records);
--   - the harness store over pi.records (memories/skills/prompts schema
--     is policy data; P4 adds /refine and the full projection).
--
-- Registered as a role so the launcher reaches it generically. The parity
-- product never loads this package; the .#prime flake app composes it.

local pi = ...

local RLM_MAX_DEPTH = 8          -- product limit (Lua spawn-admission policy)
local HARNESS_COLLECTION = "harness"

-- ---------------------------------------------------------------------------
-- per-session state registry: the python tool and host-request handlers need
-- the current session's kernel, model, and subagent registry. Keyed by the
-- session id the launcher passes.
-- ---------------------------------------------------------------------------
local sessions = {}

local function now_ms()
  return pi.now_ms()
end

local function deep_copy(value, seen)
  seen = seen or {}
  if type(value) ~= "table" then return value end
  if seen[value] then return seen[value] end
  local copy = {}
  seen[value] = copy
  for k, v in pairs(value) do copy[k] = deep_copy(v, seen) end
  return copy
end

-- ---------------------------------------------------------------------------
-- harness store: durable records over pi.records. Collection names are data;
-- scope (local/global) is a path prefix chosen here, in policy.
-- ---------------------------------------------------------------------------
local function harness_store(session_dir)
  local store = pi.records.open(session_dir .. "/harness.jsonl")
  return {
    put = function(kind, key, value)
      store:put(kind, key, value)
    end,
    get = function(kind, key)
      return store:get(kind, key)
    end,
    list = function(kind)
      return store:list(kind)
    end,
    delete = function(kind, key)
      store:delete(kind, key)
    end,
  }
end

-- ---------------------------------------------------------------------------
-- system prompt: base + a harness block listing durable memories/skills/
-- prompts (the RLM "continual harness" prompt projection, minimal for P3).
-- ---------------------------------------------------------------------------
local function harness_block(harness)
  local lines = {}
  for _, kind in ipairs({ "prompt", "memory", "skill" }) do
    for _, entry in ipairs(harness.list(kind)) do
      lines[#lines + 1] = string.format("- %s %s: %s", kind, entry.key,
        tostring(entry.value.text or entry.value.description or "—"))
    end
  end
  if #lines == 0 then return "" end
  return [[
# Persistent harness state (durable across turns; update via the harness tool)

]] .. table.concat(lines, "\n") .. "\n"
end

-- ---------------------------------------------------------------------------
-- stream one assistant response (pi.ai.stream_simple shape).
-- ---------------------------------------------------------------------------
local function stream_assistant(session, emit)
  local context = {
    systemPrompt = session.system_prompt .. harness_block(session.harness),
    messages = session.messages,
    tools = session.tools,
  }
  local partial = nil
  local added = false
  local final = pi.ai.stream_simple(session.model, context, session.options, function(event)
    if event.type == "start" then
      partial = event.partial
      session.messages[#session.messages + 1] = partial
      added = true
      if emit then emit({ type = "message_start", message = partial }) end
    elseif event.type ~= "done" and event.type ~= "error" and partial then
      partial = event.partial
      session.messages[#session.messages] = partial
      if emit then emit({ type = "message_update", message = partial }) end
    end
  end)
  if added then
    session.messages[#session.messages] = final
  else
    session.messages[#session.messages + 1] = final
  end
  return final
end

-- ---------------------------------------------------------------------------
-- tool execution: dispatch a tool call through the public tool registry.
-- ---------------------------------------------------------------------------
local function find_tool(name, session)
  for _, tool in ipairs(session.tools or {}) do
    if tool.name == name then return tool end
  end
  for _, tool in ipairs(pi.registered_tools() or {}) do
    if tool.name == name then return tool end
  end
  return nil
end

local function execute_call(session, call, emit)
  local tool = find_tool(call.name, session)
  if not tool then
    return { role = "toolResult", toolCallId = call.id, toolName = call.name,
             content = "Tool not found: " .. call.name, isError = true }
  end
  local tool_context = { session_id = session.session_id, rlm_depth = session.rlm_depth }
  if emit then emit({ type = "tool_execution_start", toolCallId = call.id, toolName = call.name }) end
  local ok, value = pcall(tool.execute, call.id, call.args or {}, nil, nil, tool_context)
  if emit then emit({ type = "tool_execution_end", toolCallId = call.id, toolName = call.name }) end
  local result, is_error
  if ok then
    result, is_error = value.content, value.isError == true
  else
    result, is_error = tostring(value), true
  end
  return { role = "toolResult", toolCallId = call.id, toolName = call.name,
           content = result, isError = is_error, timestamp = now_ms() }
end

-- ---------------------------------------------------------------------------
-- the RLM turn loop.
-- ---------------------------------------------------------------------------
local function run_rlm_turn(session, prompt, emit)
  local user_message = { role = "user", content = prompt, timestamp = now_ms() }
  session.messages[#session.messages + 1] = user_message
  local depth_guard = 0
  while true do
    depth_guard = depth_guard + 1
    if depth_guard > 64 then
      error("RLM loop exceeded 64 tool-turns in one prompt (runaway)")
    end
    local message = stream_assistant(session, emit)
    if message.stopReason == "error" or message.stopReason == "aborted" then
      return message
    end
    local calls = {}
    for _, content in ipairs(message.content or {}) do
      if content.type == "toolCall" then calls[#calls + 1] = content end
    end
    if #calls == 0 then
      return message  -- prose stop: the assistant answered without tools
    end
    for _, call in ipairs(calls) do
      local result = execute_call(session, call, emit)
      session.messages[#session.messages + 1] = result
    end
  end
end

-- ---------------------------------------------------------------------------
-- subagent registry (durable records) + rlm host-request handlers.
-- ---------------------------------------------------------------------------
local function registry_for(session)
  local records = pi.records.open(session.session_dir .. "/subagents.jsonl")
  return records
end

local function spawn_handle_payload(records, child)
  return {
    rlm_child_id = child.id,
    name = child.name,
    session_dir = child.session_dir,
    model = child.model,
  }
end

-- The python tool's host-request pump (pi.repl) routes rlm.* requests here.
-- The handlers run on the session's pump coroutine.
local create_session
local function make_host_handlers(session)
  return {
    ["rlm.run"] = function(payload)
      local prompt = payload.prompt
      local rlm_depth = (session.rlm_depth or 0) + 1
      if rlm_depth > RLM_MAX_DEPTH then
        return { status = "error", error = "rlm depth limit (" .. RLM_MAX_DEPTH .. ") reached" }
      end
      local child_id = session.session_id .. "-c" .. tostring(now_ms())
      local child_session_dir = session.session_dir .. "/" .. child_id
      local records = registry_for(session)
      records:put("subagents", child_id, {
        rlm_child_id = child_id, name = payload.name or "subagent",
        active_session_id = nil, session_id = child_id,
        session_name = payload.name or ("rlm-" .. child_id),
        session_dir = child_session_dir, status = "queued",
        parent = session.session_id, rlm_depth = rlm_depth,
      })
      -- Admit immediately (per plan: admission returns a handle; results
      -- arrive via records). The child runs the same loop in the
      -- background with its own kernel.
      pi.spawn(function()
        local child_session = create_session({
          session_id = child_id,
          session_dir = child_session_dir,
          model = payload.model or session.model,
          rlm_depth = rlm_depth,
          parent = session,
          system_prompt = session.system_prompt,
        })
        local ok, err = pcall(run_rlm_turn, child_session, prompt, nil)
        records:put("subagents", child_id, {
          rlm_child_id = child_id, name = payload.name or "subagent",
          active_session_id = nil, session_id = child_id,
          session_name = payload.name or ("rlm-" .. child_id),
          session_dir = child_session_dir, status = ok and "completed" or "error",
          error = not ok and tostring(err) or nil,
          parent = session.session_id, rlm_depth = rlm_depth,
        })
      end)
      return spawn_handle_payload(records, {
        id = child_id, name = payload.name or "subagent",
        session_dir = child_session_dir, model = payload.model or session.model,
      })
    end,
    ["rlm.find_models"] = function(payload)
      local query = (payload.query or ""):lower()
      local limit = payload.limit or 8
      local models = {}
      local candidates = { session.model, "sonnet", "opus", "haiku", "gpt-4o", "gpt-4o-mini" }
      for _, candidate in ipairs(candidates) do
        local id = tostring(candidate)
        if query == "" or id:lower():find(query, 1, true) then
          if #models >= limit then break end
          models[#models + 1] = { provider = "scripted", id = id, name = id, selector = "scripted/" .. id }
        end
      end
      return { status = "ok", models = models }
    end,
    ["rlm.list_subagents"] = function()
      local records = registry_for(session)
      local subagents = {}
      for _, entry in ipairs(records:list("subagents")) do
        subagents[#subagents + 1] = entry.value
      end
      return { status = "ok", subagents = subagents }
    end,
    ["rlm.delete_subagent"] = function(payload)
      local target = payload.target
      local records = registry_for(session)
      local entry = records:get("subagents", target)
      if not entry then
        return { status = "error", error = "no subagent with id " .. tostring(target) }
      end
      records:delete("subagents", target)
      return { status = "ok", subagent = entry }
    end,
  }
end

-- ---------------------------------------------------------------------------
-- the python tool: persistent IPython kernel via pi.repl.
-- ---------------------------------------------------------------------------
local function make_python_tool()
  return {
    name = "python",
    description = "Execute Python code in the persistent IPython kernel. "
      .. "State persists across calls. Use await rlm.run(prompt) to spawn "
      .. "a recursive subagent, await rlm.find_models() to search models, "
      .. "rlm.list_subagents()/rlm.delete_subagent() to manage children.",
    parameters = {
      type = "object",
      properties = { code = { type = "string", description = "Python code to execute" } },
      required = { "code" },
    },
    executionMode = "sequential",
    execute = function(tool_call_id, args, signal, updates, tool_context)
      local session = sessions[tool_context.session_id]
      if not session or not session.kernel then
        return { content = "error: no RLM session kernel (python tool called outside a session)", isError = true }
      end
      local code = args.code or ""
      local ok, result = pcall(function()
        return session.kernel:execute(code)
      end)
      if not ok then
        return { content = "kernel error: " .. tostring(result), isError = true }
      end
      local body = {}
      if result.stdout and #result.stdout > 0 then body[#body + 1] = result.stdout end
      if result.stderr and #result.stderr > 0 then body[#body + 1] = result.stderr end
      if result.result then body[#body + 1] = tostring(result.result) end
      if result.error then
        body[#body + 1] = result.error.ename .. ": " .. result.error.evalue
      end
      local text = table.concat(body, "")
      if #text == 0 then text = "(no output)" end
      return { content = text, isError = result.status == "error" }
    end,
  }
end

-- ---------------------------------------------------------------------------
-- session construction: kernel, harness, registry, tools, host-request pump.
-- ---------------------------------------------------------------------------
create_session = function(options)
  local session_id = options.session_id
  local session_dir = options.session_dir or (os.getenv("PI_CODING_AGENT_DIR") or "~/.pi/agent") .. "/rlm/" .. session_id
  local model = options.model
  local kernel, requests = pi.repl.spawn({ watchdog_ms = 300000 })
  local session = {
    session_id = session_id,
    session_dir = session_dir,
    model = model,
    rlm_depth = options.rlm_depth or 0,
    rlm_max_depth = options.rlm_max_depth or RLM_MAX_DEPTH,
    parent = options.parent,
    system_prompt = options.system_prompt or os.getenv("PI_SYSTEM_PROMPT") or
      "You are an autonomous coding and research agent. You have a persistent "
      .. "IPython kernel available as the python tool. Use rlm.run() inside "
      .. "python to spawn recursive subagents for parallel or background work. "
      .. "You operate autonomously toward the user's request; run tools, "
      .. "inspect results, and iterate until the task is complete.",
    messages = {},
    kernel = kernel,
    harness = harness_store(session_dir),
    options = {
      maxTokens = options.max_tokens,
      thinkingLevel = options.thinking_level,
    },
  }
  session.tools = {}
  for _, tool in ipairs(pi.registered_tools() or {}) do
    session.tools[#session.tools + 1] = tool
  end
  session.tools[#session.tools + 1] = make_python_tool()
  sessions[session_id] = session

  -- The host-request pump: cells await rlm.* host requests; this coroutine
  -- answers them (doctrine 02: events in as tables, actions out as tables).
  local handlers = make_host_handlers(session)
  pi.spawn(function()
    while true do
      local req = requests:receive()
      local handler = handlers[req:get_kind()]
      local ok, reply = pcall(handler, req:get_payload())
      if ok and reply then
        req:reply(reply)
      elseif ok then
        req:reply({ status = "error", error = "no handler for " .. req:get_kind() })
      else
        req:reply({ status = "error", error = tostring(reply) })
      end
    end
  end)
  return session
end

-- ---------------------------------------------------------------------------
-- the role the launcher calls: run one RLM session on a user prompt.
-- ---------------------------------------------------------------------------
pi.register_role({
  id = "prime-rlm",
  role = "prime-rlm",
  active = true,
  priority = 0,
  description = "Run the RLM agent loop on a prompt (Prime Agent)",
  handler = function(args)
    local request = pi.json.decode(args)
    local session_id = request.sessionId or ("prime-" .. tostring(now_ms()))
    local session = create_session({
      session_id = session_id,
      session_dir = request.sessionDir,
      model = request.model,
      rlm_depth = request.rlmDepth or 0,
      rlm_max_depth = request.rlmMaxDepth,
      thinking_level = request.thinkingLevel,
      system_prompt = request.systemPrompt,
    })
    local message = run_rlm_turn(session, request.prompt or "", function(event)
      if request.onEvent then request.onEvent(event) end
    end)
    -- The kernel is a managed resource; dispose it explicitly on return.
    session.kernel:shutdown()
    return {
      result = message,
      messages = session.messages,
      sessionId = session_id,
      sessionDir = session.session_dir,
    }
  end,
})
