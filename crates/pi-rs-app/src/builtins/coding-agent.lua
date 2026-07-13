-- The default one-shot coding agent pack. Policy stays in Lua; Rust only
-- provides the provider, tools, and JSONL persistence mechanisms.
-- Shares the messages.ts / system-prompt.ts fragments with the
-- interactive pack (src/builtins/mod.rs concatenates them ahead of this
-- file).
local pi = ...
pi.declare_package({ command_visibility = "internal" })

-- sdk.ts createAgentSession + agent-session.ts _buildRuntime: activation is
-- explicit declaration data on each registered tool.
local function active_tool_definitions()
  return EXTENSION_POLICY.active_tools()
end

pi.register_role({
  id = "coding-agent-print", role = "print", active = true, priority = 0,
  handler = function(args, ctx)
  local request = pi.json.decode(args)
  local events = {}
  -- main.ts createSessionManager → sdk.ts createAgentSession: open the
  -- CLI-selected session (--continue/--session) or create a fresh one.
  local session = construct_session(request)
  local cwd = session:get_cwd()
  local startup = session_startup_from_request(session, request)
  local active_tools, active_tool_names = active_tool_definitions()
  local model = startup.model or request.model
  local system_prompt_options = {
    cwd = cwd, agentDir = request.agentDir, toolNames = active_tool_names,
    readmePath = request.readmePath, docsPath = request.docsPath,
    examplesPath = request.examplesPath,
  }
  local extension_state = {
    request = request, cwd = cwd, session_manager = session, model = model,
    project_trusted = request.projectTrusted == true,
    extension_mode = request.mode or "print",
    extension_has_ui = request.mode == "rpc",
    extension_actions = {}, extension_context_generation = 0,
    system_prompt_options = system_prompt_options,
    registry = {
      get_available = pi.ai.available_models,
      find = pi.ai.find_model,
      has_configured_auth = pi.ai.has_configured_auth,
      is_using_oauth = pi.ai.is_using_oauth,
    },
  }
  local agent = pi.agent.new({
    initialState = {
      model = model, tools = active_tools,
      messages = startup.context.messages,
      thinkingLevel = startup.thinking_level,
      systemPrompt = build_session_system_prompt(system_prompt_options),
    },
    convertToLlm = convert_to_llm_with_block_images,
    apiKey = request.apiKey,
    getApiKey = function(provider) return pi.auth.get_api_key(provider) end,
    createToolContext = function(signal)
      return EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal })
    end,
    beforeToolCall = function(event, signal)
      return EXTENSION_POLICY.emit_tool_call(event,
        EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }))
    end,
    on_event = function(event) events[#events + 1] = event end,
  })
  extension_state.agent = agent
  extension_state.extension_is_idle = function()
    return agent:get_state().isStreaming ~= true
  end
  extension_state.extension_has_pending = function()
    return agent:has_queued_messages()
  end
  extension_state.extension_action_handlers = {
    abort = function() agent:abort() end,
    -- Pi print/JSON shutdown is a no-op; RPC defers exit until its command
    -- response, whose framing remains item 10.
    shutdown = function()
      if extension_state.extension_mode == "rpc" then
        extension_state.shutdown_requested = true
      end
    end,
    compact = function(action)
      local options = action.options or {}
      if options.onError then
        options.onError({ message = "Compaction is unavailable after print completion" })
      end
    end,
  }
  agent:subscribe(function(event)
    if event.type == "message_update" and event.assistantMessageEvent and
       event.assistantMessageEvent.type == "text_delta" then
      pi.output(event.assistantMessageEvent.delta or "")
    end
    persist_agent_event(session, event)
  end)

  -- Keep the queued-action applier live while the provider/tool coroutine
  -- runs. Extensions never mutate the one-shot state directly.
  local turn = pi.spawn(function() agent:prompt(request.prompt) end)
  while not turn:done() do
    EXTENSION_CONTEXT_POLICY.pump(extension_state)
    pi.sleep(1)
  end
  turn:join()
  EXTENSION_CONTEXT_POLICY.pump(extension_state)

  local state = agent:get_state()
  local last = state.messages[#state.messages]
  local text = ""
  if last and last.role == "assistant" then
    for _, part in ipairs(last.content or {}) do
      if part.type == "text" then text = text .. (part.text or "") end
    end
  end
  return {
    text = text, events = events, sessionPath = session:get_session_file(),
    model = startup.model and { provider = startup.model.provider, id = startup.model.id } or nil,
    thinkingLevel = startup.thinking_level,
    modelFallbackMessage = startup.fallback_message,
  }
end })

-- PLAN 9.1 product-extension exerciser: the print role's active-tool
-- composition and tool_call fold, without requiring a provider fixture.
pi.register_command("extension-vertical-slice", { handler = function(args)
  local request = pi.json.decode(args)
  local tools, names = active_tool_definitions()
  if request.commandCompletion then
    for _, command in ipairs(pi.registered_extension_commands()) do
      if command.invocation_name == request.commandCompletion.name and command.get_argument_completions then
        return { completions = command.get_argument_completions(request.commandCompletion.prefix or "") }
      end
    end
    return { completions = nil }
  end
  if request.tool then
    for _, tool in ipairs(tools) do
      if tool.name == request.tool then
        return {
          toolNames = names,
          result = tool.execute("extension-slice", request.arguments or {},
            pi.abort_signal(), nil,
            { cwd = pi.cwd(), mode = "print", hasUI = false }),
        }
      end
    end
  end
  if request.toolCall then
    return {
      toolNames = names,
      hookResult = EXTENSION_POLICY.emit_tool_call({
        toolCall = {
          id = request.toolCall.id or "extension-slice",
          name = request.toolCall.name,
        },
        args = request.toolCall.arguments or {},
      }, {
        cwd = pi.cwd(), mode = "print", hasUI = false,
        isProjectTrusted = function() return request.projectTrusted == true end,
      }),
    }
  end
  return { toolNames = names }
end })

-- Differential parity seam (tests/tool-parity): replays oracle cases
-- through the registered tool definitions with the agent loop's exact
-- invocation shape — prepare_arguments → validate → execute(id, args,
-- signal, on_update, ctx) — plus a controllable abort signal ("pre" or
-- abortAfterMs via pi.spawn) and an injectable ctx.model, so the oracle
-- can pin cancellation and the non-vision image note.
local function tool_error_text(value)
  -- agent.lua error_text: strip the traceback and Lua's source:line
  -- prefix so messages compare against pi's Error.message strings.
  local text = tostring(value)
  text = text:match("^(.-)\nstack traceback:") or text
  text = text:gsub("^runtime error: ", "")
  return text:match("^.-:%d+: (.*)$") or text
end

pi.register_command("tool-parity", { handler = function(args)
  local case = pi.json.decode(args)
  local tool
  for _, def in ipairs(pi.registered_tools()) do
    if def.name == case.tool then tool = def end
  end
  if not tool then
    return { ok = false, error = "tool not registered: " .. tostring(case.tool) }
  end
  local signal = pi.abort_signal()
  if case.abort == "pre" then signal:abort() end
  if type(case.abortAfterMs) == "number" then
    pi.spawn(function()
      pi.sleep(case.abortAfterMs)
      signal:abort()
    end)
  end
  local executed, value = pcall(function()
    local params = case.args
    if tool.prepare_arguments then params = tool.prepare_arguments(params) end
    params = pi.validate_tool_arguments(tool.name, tool.parameters or {}, params)
    return tool.execute("parity-call", params, signal, nil,
      { cwd = pi.cwd(), signal = signal, isIdle = false, model = case.model })
  end)
  if executed then return { ok = true, result = value } end
  return { ok = false, error = tool_error_text(value) }
end })

-- Differential parity seam (tests/system-prompt-parity): replays oracle
-- cases through the same chunk-local ports the product wiring uses —
-- "raw" cases hit buildSystemPrompt directly, "session" cases run the
-- loadProjectContextFiles + _rebuildSystemPrompt composition.
pi.register_command("system-prompt-parity", { handler = function(args)
  local case = pi.json.decode(args)
  if case.mode == "raw" then
    return { prompt = build_system_prompt({
      cwd = case.cwd,
      selectedTools = case.selectedTools,
      toolSnippets = case.toolSnippets,
      promptGuidelines = case.promptGuidelines,
      customPrompt = case.customPrompt,
      appendSystemPrompt = case.appendSystemPrompt,
      contextFiles = case.contextFiles,
      skills = case.skills,
      readmePath = case.readmePath,
      docsPath = case.docsPath,
      examplesPath = case.examplesPath,
      now = case.now,
    }) }
  end
  local context_files = load_project_context_files({
    cwd = case.cwd, agentDir = case.agentDir,
  })
  local prompt = build_session_system_prompt({
    cwd = case.cwd,
    agentDir = case.agentDir,
    toolNames = case.toolNames,
    customPrompt = case.customPrompt,
    appendSystemPrompt = case.appendSystemPrompt,
    skills = case.skills,
    contextFiles = context_files,
    readmePath = case.readmePath,
    docsPath = case.docsPath,
    examplesPath = case.examplesPath,
    now = case.now,
  })
  return { prompt = prompt, contextFiles = context_files }
end })

-- Differential parity seam (tests/compaction-parity): replays oracle
-- cases through the compaction policy fragment (utils/compaction.lua) —
-- prepareCompaction/compact with a scripted stream_fn recording every
-- summarization request (the spec's injectable streamFn), plus the
-- token-estimation, shouldCompact, and isContextOverflow slices. The
-- fixed now_ms mirrors gen-oracle.ts's pinned Date.now.
local CP_NOW_MS = 1750000000000

local function cp_settings(case)
  local settings = { enabled = true, reserveTokens = 16384, keepRecentTokens = 20000 }
  for key, value in pairs(case.settings or {}) do settings[key] = value end
  return settings
end

local function cp_sorted_paths(set)
  local paths = {}
  for path in pairs(set) do paths[#paths + 1] = path end
  table.sort(paths)
  return paths
end

local function cp_preparation(preparation)
  return {
    firstKeptEntryId = preparation.firstKeptEntryId,
    isSplitTurn = preparation.isSplitTurn,
    tokensBefore = preparation.tokensBefore,
    previousSummary = preparation.previousSummary,
    messagesToSummarize = preparation.messagesToSummarize,
    turnPrefixMessages = preparation.turnPrefixMessages,
    fileOps = {
      read = cp_sorted_paths(preparation.fileOps.read),
      written = cp_sorted_paths(preparation.fileOps.written),
      edited = cp_sorted_paths(preparation.fileOps.edited),
    },
  }
end

pi.register_command("compaction-parity", { handler = function(args)
  local request = pi.json.decode(args)
  local case = request.case
  local mode = case.mode or "prepare"
  local model = request.models[case.model or "default"]
  local settings = cp_settings(case)

  if mode == "tokens" then
    local out = {}
    if case.messages then
      out.estimate = compaction_lib.estimate_context_tokens(case.messages)
    end
    if case.usage then
      out.contextTokens = compaction_lib.calculate_context_tokens(case.usage)
    end
    return out
  end
  if mode == "should" then
    return { shouldCompact = compaction_lib.should_compact(
      case.contextTokens, case.contextWindow, settings) }
  end
  if mode == "overflow" then
    return { overflow = compaction_lib.is_context_overflow(
      case.message, case.contextWindow) }
  end

  local preparation = compaction_lib.prepare_compaction(case.entries, settings)
  if not preparation then return { prepared = false } end
  local out = { prepared = true, preparation = cp_preparation(preparation) }
  if mode == "compact" then
    local requests = {}
    local stream_fn = function(stream_model, context, options)
      requests[#requests + 1] = {
        systemPrompt = context.systemPrompt,
        messages = context.messages,
        maxTokens = options.maxTokens,
        reasoning = options.reasoning,
        apiKey = options.apiKey,
      }
      local scripted = (case.responses or {})[#requests] or { text = "" }
      if scripted.errorMessage then
        return { role = "assistant", content = {}, api = stream_model.api,
          provider = stream_model.provider, model = stream_model.id,
          stopReason = "error", errorMessage = scripted.errorMessage, timestamp = 0 }
      end
      return { role = "assistant",
        content = { { type = "text", text = scripted.text or "" } },
        api = stream_model.api, provider = stream_model.provider,
        model = stream_model.id, stopReason = "stop", timestamp = 0 }
    end
    local executed, value = pcall(compaction_lib.compact, preparation, model, {
      apiKey = case.apiKey or "oracle-key",
      customInstructions = case.customInstructions,
      thinkingLevel = case.thinkingLevel,
      stream_fn = stream_fn,
      now_ms = function() return CP_NOW_MS end,
    })
    if executed then out.result = value
    else out.error = tool_error_text(value) end
    out.requests = requests
  end
  return out
end })

-- Differential parity seam (tests/session-parity): replays oracle cases
-- through the product session-persistence policy — a real pi.session
-- handle fed by the same persist_agent_event / session_startup
-- fragments the product packs run (utils/agent-session.lua) — with the
-- scripted streams, scripted tools, and event-count triggers mirrored
-- 1:1 from tests/session-parity/gen-oracle.ts. The op set mirrors pi's
-- AgentSession surface: prompt, setSessionName (appendSessionInfo), and
-- setModel (swap the agent model, then appendModelChange — the same
-- ordering session_set_model uses in the interactive pack).
local SP_EMPTY_USAGE = {
  input = 0, output = 0, cacheRead = 0, cacheWrite = 0, totalTokens = 0,
  cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, total = 0 },
}

local function sp_deep_copy(value)
  if type(value) ~= "table" then return value end
  local out = {}
  for key, item in pairs(value) do out[key] = sp_deep_copy(item) end
  return out
end

local function sp_base_message(model, content, stop_reason)
  return {
    role = "assistant", content = content, api = model.api,
    provider = model.provider, model = model.id,
    usage = sp_deep_copy(SP_EMPTY_USAGE), stopReason = stop_reason, timestamp = 0,
  }
end

-- Mirror of gen-oracle.ts synthesize().
local function sp_synthesize(turn, model)
  local blocks = turn.blocks or {}
  local function snapshot(count, current)
    local content = {}
    for i = 1, count do content[i] = sp_deep_copy(blocks[i]) end
    if current ~= nil then content[count + 1] = current end
    return content
  end
  local events = { { type = "start", partial = sp_base_message(model, {}, "stop") } }
  for index, block in ipairs(blocks) do
    local ci = index - 1
    if block.type == "text" then
      events[#events + 1] = { type = "text_start", contentIndex = ci,
        partial = sp_base_message(model, snapshot(index - 1, { type = "text", text = "" }), "stop") }
      events[#events + 1] = { type = "text_delta", contentIndex = ci, delta = block.text,
        partial = sp_base_message(model, snapshot(index), "stop") }
      events[#events + 1] = { type = "text_end", contentIndex = ci, content = block.text,
        partial = sp_base_message(model, snapshot(index), "stop") }
    elseif block.type == "thinking" then
      events[#events + 1] = { type = "thinking_start", contentIndex = ci,
        partial = sp_base_message(model, snapshot(index - 1, { type = "thinking", thinking = "" }), "stop") }
      events[#events + 1] = { type = "thinking_delta", contentIndex = ci, delta = block.thinking,
        partial = sp_base_message(model, snapshot(index), "stop") }
      events[#events + 1] = { type = "thinking_end", contentIndex = ci, content = block.thinking,
        partial = sp_base_message(model, snapshot(index), "stop") }
    elseif block.type == "toolCall" then
      events[#events + 1] = { type = "toolcall_start", contentIndex = ci,
        partial = sp_base_message(model, snapshot(index - 1,
          { type = "toolCall", id = block.id, name = block.name, arguments = {} }), "stop") }
      events[#events + 1] = { type = "toolcall_delta", contentIndex = ci,
        delta = pi.json.encode(block.arguments),
        partial = sp_base_message(model, snapshot(index), "stop") }
      events[#events + 1] = { type = "toolcall_end", contentIndex = ci,
        toolCall = sp_deep_copy(block),
        partial = sp_base_message(model, snapshot(index), "stop") }
    else
      error("unknown block type " .. tostring(block.type), 0)
    end
  end
  local final = sp_base_message(model, snapshot(#blocks), turn.stopReason or "stop")
  if turn.errorMessage ~= nil then final.errorMessage = turn.errorMessage end
  if turn.stopReason == "error" or turn.stopReason == "aborted" then
    events[#events + 1] = { type = "error", reason = turn.stopReason, error = final }
  else
    events[#events + 1] = { type = "done", reason = turn.stopReason or "stop", message = final }
  end
  return events, final
end

local function sp_make_stream_fn(case)
  local turn_index = 0
  return function(model, _context, options, push)
    turn_index = turn_index + 1
    local turn = case.turns[math.min(turn_index, #case.turns)]
    if turn["throw"] then error(turn["throw"], 0) end
    local events, final = sp_synthesize(turn, model)
    local last_content = {}
    for _, event in ipairs(events) do
      local signal = options.signal
      if signal and signal:is_aborted() then
        local aborted = sp_base_message(model, last_content, "aborted")
        aborted.errorMessage = "Request was aborted"
        push({ type = "error", reason = "aborted", error = aborted })
        return aborted
      end
      push(event)
      local partial = event.partial or event.message or event.error
      if partial and partial.content then last_content = sp_deep_copy(partial.content) end
    end
    return final
  end
end

local function sp_build_tool(spec)
  local count = 0
  local invocations = spec.invocations or {}
  return {
    label = spec.name,
    name = spec.name,
    description = "scripted " .. spec.name,
    parameters = spec.parameters,
    executionMode = spec.executionMode,
    execute = function(_id, _args, signal, on_update)
      local inv = {}
      if #invocations > 0 then inv = invocations[math.min(count + 1, #invocations)] end
      count = count + 1
      local function check()
        if inv.abortCheck and signal and signal:is_aborted() then
          error(spec.name .. " aborted", 0)
        end
      end
      check()
      for _, update in ipairs(inv.updates or {}) do
        if update.sleepMs then pi.sleep(update.sleepMs) end
        check()
        if on_update then on_update(sp_deep_copy(update.partial)) end
      end
      if inv.sleepMs then pi.sleep(inv.sleepMs) end
      check()
      if inv["throw"] then error(inv["throw"], 0) end
      return sp_deep_copy(inv.result or
        { content = { { type = "text", text = spec.name .. " ok" } }, details = {} })
    end,
  }
end

pi.register_command("session-parity", { handler = function(args)
  local request = pi.json.decode(args)
  local case = request.case
  local models = request.models
  local options = case.options or {}
  local model = models[options.model or "default"]

  local session = pi.session.create({
    cwd = request.cwd, sessionDir = request.sessionDir, agentDir = request.agentDir,
  })

  local tools = {}
  for _, spec in ipairs(case.tools or {}) do tools[#tools + 1] = sp_build_tool(spec) end
  local agent = pi.agent.new({
    initialState = {
      systemPrompt = options.systemPrompt or "",
      model = model,
      thinkingLevel = options.thinkingLevel,
      tools = tools,
      messages = {},
    },
    streamFn = sp_make_stream_fn(case),
  })

  -- The product persistence policy under test (utils/agent-session.lua).
  session_startup(session, {
    cliModel = model, fallbackModel = model,
    cliThinking = options.thinkingLevel,
  })
  local counts = {}
  local fired = {}
  agent:subscribe(function(event)
    persist_agent_event(session, event)
    counts[event.type] = (counts[event.type] or 0) + 1
    for index, trigger in ipairs(case.triggers or {}) do
      if not fired[index] and trigger.on.event == event.type
        and counts[event.type] == trigger.on.count then
        fired[index] = true
        if trigger.action == "abort" then agent:abort()
        elseif trigger.action == "steer" then
          -- agent-session.ts _queueSteer message shape (the interactive
          -- pack's user_message equivalent).
          agent:steer({ role = "user",
            content = { { type = "text", text = trigger.text } },
            timestamp = os.time() * 1000 })
        elseif trigger.action == "followUp" then
          agent:follow_up({ role = "user",
            content = { { type = "text", text = trigger.text } },
            timestamp = os.time() * 1000 })
        else error("unknown trigger action " .. tostring(trigger.action), 0) end
      end
    end
  end)

  for _, op in ipairs(case.ops or {}) do
    if op.op == "prompt" then
      agent:prompt(op.text)
    elseif op.op == "setName" then
      -- agent-session.ts setSessionName.
      session:append_session_info(op.name)
    elseif op.op == "setModel" then
      -- agent-session.ts setModel: swap the agent model, then persist —
      -- the same ordering as the interactive pack's session_set_model.
      agent:set_model(models[op.model])
      session:append_model_change(models[op.model].provider, models[op.model].id)
    else
      error("unknown op " .. tostring(op.op), 0)
    end
  end

  return { sessionFile = session:get_session_file() }
end })
