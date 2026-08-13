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
  -- Output guard (spec: output-guard.ts takeOverStdout): during non-interactive
  -- modes stray Lua stdlib `print`/`io.write` from extensions must not corrupt
  -- the stdout stream. pi.output stays the product protocol channel.
  pi.apply_stdout_guard(true)
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
  local extension_errors = {}
  EXTENSION_POLICY.on_error = function(error)
    extension_errors[#extension_errors + 1] = error
  end
  local extension_state = {
    request = request, cwd = cwd, session_manager = session, model = model,
    project_trusted = request.projectTrusted == true,
    extension_mode = request.mode or "print",
    extension_has_ui = request.mode == "rpc",
    extension_actions = {}, extension_context_generation = 0,
    extension_ui = EXTENSION_HEADLESS_UI,
    system_prompt_options = system_prompt_options,
    registry = {
      get_available = pi.ai.available_models,
      find = pi.ai.find_model,
      has_configured_auth = pi.ai.has_configured_auth,
      is_using_oauth = pi.ai.is_using_oauth,
    },
  }
  local system_prompt = build_session_system_prompt(system_prompt_options)
  local agent
  agent = pi.agent.new({
    initialState = {
      model = model, tools = active_tools,
      messages = startup.context.messages,
      thinkingLevel = startup.thinking_level,
      systemPrompt = system_prompt,
    },
    convertToLlm = convert_to_llm_with_block_images,
    transformContext = function(messages, signal)
      return EXTENSION_POLICY.emit_context(messages,
        EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }))
    end,
    apiKey = request.apiKey,
    getApiKey = function(provider) return pi.auth.get_api_key(provider) end,
    onPayload = function(payload)
      return EXTENSION_POLICY.emit_before_provider_request(payload,
        EXTENSION_CONTEXT_POLICY.snapshot(extension_state,
          { signal = agent and agent:get_state().signal or nil }))
    end,
    onResponse = function(response)
      EXTENSION_POLICY.emit_generic({ type = "after_provider_response",
        status = response.status, headers = response.headers },
        EXTENSION_CONTEXT_POLICY.snapshot(extension_state,
          { signal = agent and agent:get_state().signal or nil }))
    end,
    createToolContext = function(signal)
      return EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal })
    end,
    beforeToolCall = function(event, signal)
      return EXTENSION_POLICY.emit_tool_call(event,
        EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }))
    end,
    afterToolCall = function(event, signal)
      return EXTENSION_POLICY.emit_tool_result({
        type = "tool_result", toolCallId = event.toolCall.id,
        toolName = event.toolCall.name, input = event.args,
        content = event.result.content, details = event.result.details,
        isError = event.isError,
      }, EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }))
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
    -- PLAN 9.4: extension action methods for print mode
    send_message = function(action)
      local msg = action.message
      if msg then
        local custom = {
          role = "custom",
          customType = msg.customType or "custom",
          content = msg.content or { { type = "text", text = tostring(msg.text or "") } },
          display = msg.display,
          details = msg.details,
          timestamp = os.time() * 1000,
        }
        agent:steer(custom)
      end
    end,
    send_user_message = function(action)
      local content = action.content
      if content then
        agent:steer({
          role = "user",
          content = { { type = "text", text = content } },
          timestamp = os.time() * 1000,
        })
      end
    end,
    set_model = function(action)
      if action.model then
        agent:set_model(action.model)
      end
    end,
    set_thinking_level = function(action)
      if action.level then
        agent:set_thinking_level(action.level)
      end
    end,
    set_session_name = function(action)
      if action.name and session then
        session:append_session_info(action.name)
      end
    end,
    set_active_tools = function(action)
      if action.tools then
        local tools, _ = active_tool_definitions()
        local valid = {}
        for _, name in ipairs(action.tools) do
          for _, t in ipairs(tools) do
            if t.name == name then
              valid[#valid + 1] = t
            end
          end
        end
        if #valid > 0 or action.refresh then
          agent:set_tools(valid)
          if system_prompt_options then
            local names = {}
            for _, t in ipairs(valid) do names[#names + 1] = t.name end
            if #names > 0 then system_prompt_options.toolNames = names end
            agent:set_system_prompt(build_session_system_prompt(system_prompt_options))
          end
        end
      end
    end,
    set_label = function(action)
      -- Entry labels are interactive-only; print mode is a no-op.
    end,
    append_entry = function(action)
      -- Custom entries are interactive-only; print mode is a no-op.
    end,
  }
  -- PLAN 9.4: bind the ExtensionAPI runtime action/view methods onto the
  -- shared `pi` table for this live session (spec `bindCoreActions`).
  EXTENSION_POLICY.bind_pi_actions(extension_state)
  local turn_state = { index = 0 }
  -- PLAN 10: JSON mode writes the session header line (when present) then
  -- every agent event as JSONL, matching modes/print-mode.ts
  -- (`session.sessionManager.getHeader()` first, then each subscribe event).
  local json_mode = request.mode == "json"
  if json_mode then
    local header = session:get_header()
    if header then
      pi.output(pi.json.encode(header) .. "\n")
    end
  end
  agent:subscribe(function(event)
    local state = agent:get_state().signal
    EXTENSION_POLICY.emit_agent_event(event,
      EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = state }),
      pi.now_ms, turn_state)
    if json_mode then
      pi.output(pi.json.encode(event) .. "\n")
    end
    persist_agent_event(session, event)
  end)

  EXTENSION_POLICY.emit_generic({ type = "session_start", reason = "startup" },
    EXTENSION_CONTEXT_POLICY.snapshot(extension_state))
  local extension_resources = EXTENSION_POLICY.emit_resources_discover(
    cwd, "startup", EXTENSION_CONTEXT_POLICY.snapshot(extension_state))

  -- Keep the queued-action applier live while the provider/tool coroutine
  -- runs. Extensions never mutate the one-shot state directly.
  local turn = pi.spawn(function()
    local context = EXTENSION_CONTEXT_POLICY.snapshot(extension_state)
    local input = EXTENSION_POLICY.emit_input(request.prompt, nil, "interactive", nil, context)
    if input.action == "handled" then return end
    local prompt = input.action == "transform" and input.text or request.prompt
    local before = EXTENSION_POLICY.emit_before_agent_start(prompt, nil, system_prompt,
      system_prompt_options, EXTENSION_CONTEXT_POLICY.snapshot(extension_state))
    agent:set_system_prompt(before and before.systemPrompt or system_prompt)
    local initial_content = { { type = "text", text = prompt } }
    for _, img in ipairs(request.initialImages or {}) do
      initial_content[#initial_content + 1] = img
    end
    local prompts = { { role = "user", content = initial_content,
      timestamp = pi.now_ms() } }
    for _, message in ipairs(before and before.messages or {}) do
      prompts[#prompts + 1] = { role = "custom", customType = message.customType,
        content = message.content, display = message.display, details = message.details,
        timestamp = pi.now_ms() }
    end
    agent:prompt(prompts)
    -- Plan 10 (modes/print-mode.ts messages[]): after the initial message,
    -- send each remaining CLI message as a sequential follow-up prompt.
    -- session.prompt(message) appends a user message and runs the agent turn.
    for _, message in ipairs(request.followUpMessages or {}) do
      agent:prompt({ role = "user",
        content = { { type = "text", text = message } },
        timestamp = pi.now_ms() })
    end
  end)
  while not turn:done() do
    EXTENSION_CONTEXT_POLICY.pump(extension_state)
    pi.sleep(1)
  end
  turn:join()
  EXTENSION_CONTEXT_POLICY.pump(extension_state)
  local state = agent:get_state()
  local last = state.messages[#state.messages]
  local text = ""
  local stop_reason = last and last.role == "assistant" and last.stopReason
  local error_message = last and last.errorMessage
  if last and last.role == "assistant" then
    for _, part in ipairs(last.content or {}) do
      if part.type == "text" then text = text .. (part.text or "") end
    end
  end
  -- PLAN 10 (modes/print-mode.ts): text mode emits the final assistant
  -- message's text blocks to stdout, each followed by `\n` — it does not
  -- stream deltas. On `error`/`aborted` the run fails and the caller maps
  -- `exitCode` + `errorMessage` to a nonzero process exit with an error on
  -- stderr (main.ts handles this; print-mode.ts `console.error` + exit 1).
  local exit_code = 0
  if not json_mode then
    if stop_reason == "error" or stop_reason == "aborted" then
      exit_code = 1
    else
      for _, part in ipairs(last and last.content or {}) do
        if part.type == "text" then
          pi.output((part.text or "") .. "\n")
        end
      end
    end
  end
  return {
    text = text, events = events, sessionPath = session:get_session_file(),
    model = startup.model and { provider = startup.model.provider, id = startup.model.id } or nil,
    thinkingLevel = startup.thinking_level,
    modelFallbackMessage = startup.fallback_message,
    extensionErrors = extension_errors, extensionResources = extension_resources,
    exitCode = exit_code, stopReason = stop_reason, errorMessage = error_message,
  }
end })

-- PLAN 9.3 differential fold driver. Ordinary file-backed extensions register
-- through pi.on; this invokes the same policy used by real product seams.
pi.register_command("extension-event-fold-parity", { handler = function(args)
  local request = pi.json.decode(args)
  local errors = {}
  EXTENSION_POLICY.on_error = function(error) errors[#errors + 1] = error end
  local context = { cwd = request.cwd, mode = "tui", hasUI = false,
    getSystemPrompt = function() return "system" end }
  local generic_types = { "session_start", "session_compact", "session_shutdown",
    "session_tree", "after_provider_response", "agent_start", "agent_end",
    "turn_start", "turn_end", "message_start", "message_update",
    "tool_execution_start", "tool_execution_update", "tool_execution_end",
    "model_select", "thinking_level_select" }
  for _, kind in ipairs(generic_types) do
    EXTENSION_POLICY.emit_generic({ type = kind, status = 201, headers = { x = "y" },
      messages = {}, turnIndex = 2, timestamp = 123,
      message = { role = "assistant", content = {}, timestamp = 0 },
      toolResults = {}, toolCallId = "call", toolName = "bash",
      args = { command = "x" }, partialResult = { content = {} },
      result = { content = {} }, isError = false, source = "set",
      level = "low", previousLevel = "off", newLeafId = "leaf",
      oldLeafId = "old", fromExtension = false, compactionEntry = { id = "compact" },
    }, context)
  end
  local before_switch = EXTENSION_POLICY.emit_generic({ type = "session_before_switch",
    reason = "resume", targetSessionFile = "target.jsonl" }, context)
  local before_fork = EXTENSION_POLICY.emit_generic({ type = "session_before_fork",
    entryId = "entry", position = "before" }, context)
  local before_compact = EXTENSION_POLICY.emit_generic({ type = "session_before_compact",
    preparation = { firstKeptEntryId = "keep" }, branchEntries = {},
    signal = pi.abort_signal() }, context)
  local before_tree = EXTENSION_POLICY.emit_generic({ type = "session_before_tree",
    preparation = { targetId = "target" }, signal = pi.abort_signal() }, context)
  local messages = EXTENSION_POLICY.emit_context(
    { { role = "user", content = "base", timestamp = 0 } }, context)
  local payload = EXTENSION_POLICY.emit_before_provider_request({ base = true }, context)
  local before_agent = EXTENSION_POLICY.emit_before_agent_start(
    "prompt", nil, "system", { cwd = request.cwd }, context)
  local message = EXTENSION_POLICY.emit_message_end({ type = "message_end",
    message = { role = "assistant", content = { { type = "text", text = "base" } },
      api = "x", provider = "x", model = "x", usage = {},
      stopReason = "stop", timestamp = 0 } }, context)
  local tool_input = { command = "echo" }
  local tool_call = EXTENSION_POLICY.emit_tool_call({ type = "tool_call",
    toolCallId = "call", toolName = "bash", input = tool_input }, context)
  local tool_result = EXTENSION_POLICY.emit_tool_result({ type = "tool_result",
    toolCallId = "call", toolName = "bash", input = tool_input,
    content = { { type = "text", text = "base result" } },
    details = { base = true }, isError = false }, context)
  local user_bash = EXTENSION_POLICY.emit_user_bash({ type = "user_bash",
    command = "echo hi", excludeFromContext = false, cwd = request.cwd }, context)
  local input = EXTENSION_POLICY.emit_input("go", nil, "interactive", nil, context)
  local handled = EXTENSION_POLICY.emit_input("handle", nil, "interactive", "steer", context)
  local trust = EXTENSION_POLICY.emit_project_trust(
    { type = "project_trust", cwd = request.cwd }, context)
  local resources = EXTENSION_POLICY.emit_resources_discover(request.cwd, "startup", context)
  local trace
  for _, command in ipairs(pi.registered_extension_commands()) do
    if command.invocation_name == "event-trace" then trace = command.handler("", context) end
  end
  return { beforeSwitch = before_switch, beforeFork = before_fork,
    beforeCompact = before_compact, beforeTree = before_tree, context = messages,
    payload = payload, beforeAgent = before_agent, message = message,
    toolInput = tool_input, toolCall = tool_call, toolResult = tool_result,
    userBash = user_bash, input = input, handledInput = handled, trust = trust,
    resources = resources, errors = errors, trace = trace }
end })

-- PLAN 9.1 product-extension exerciser: the same active-tool composition and
-- tool_call fold used by pi-rs-run, without requiring a provider fixture.
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
            { cwd = pi.cwd(), mode = "print", hasUI = false, ui = EXTENSION_HEADLESS_UI }),
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
        cwd = pi.cwd(), mode = "print", hasUI = false, ui = EXTENSION_HEADLESS_UI,
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
      out.estimate = EXTENSION_POLICY.compaction.estimate_context_tokens(case.messages)
    end
    if case.usage then
      out.contextTokens = EXTENSION_POLICY.compaction.calculate_context_tokens(case.usage)
    end
    return out
  end
  if mode == "should" then
    return { shouldCompact = EXTENSION_POLICY.compaction.should_compact(
      case.contextTokens, case.contextWindow, settings) }
  end
  if mode == "overflow" then
    return { overflow = EXTENSION_POLICY.compaction.is_context_overflow(
      case.message, case.contextWindow) }
  end

  local preparation = EXTENSION_POLICY.compaction.prepare_compaction(case.entries, settings)
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
    local executed, value = pcall(EXTENSION_POLICY.compaction.compact, preparation, model, {
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


-- PLAN 10: RPC mode role handler. Stdin-based JSON-RPC protocol:
-- commands come one-per-line on stdin; responses and agent events
-- go to stdout as JSONL. Mirrors Pi's real `runRpcMode` (modes/rpc/
-- rpc-mode.ts): `type: "response"` objects with `command`/`success`/`data`,
-- `id` present only when the client supplied one, unknown commands and parse
-- errors shaped exactly like Pi, and extension events streamed verbatim.
--
-- The differential oracle in tests/rpc-parity/oracle.json is generated from
-- Pi's real runRpcMode driving a scripted session (scripts/rpc-oracle) and
-- pins this framing byte-for-byte for the synchronous command vocabulary:
-- get_state, get_available_models, set_steering_mode, set_follow_up_mode,
-- set_auto_compaction, set_auto_retry, abort_retry, get_messages,
-- get_last_assistant_text, set_thinking_level, cycle_thinking_level, set_model,
-- cycle_model, get_commands, set_session_name, get_session_stats, export_html,
-- plus unknown-command and JSON-parse-error responses. The async prompt/steer/
-- follow_up/abort/bash/compact/fork/clone/new_session/switch_session commands
-- still stream through the agent and remain PLAN 10 open rows.
pi.register_role({
  id = "coding-agent-rpc", role = "rpc", active = true, priority = 0,
  handler = function(args, ctx)
    local request = pi.json.decode(args)
    local events = {}
    -- Output guard (spec: rpc-mode.ts takeOverStdout): RPC stdout carries only
    -- JSONL protocol records; stray Lua stdlib print/io.write from extensions go
    -- to stderr so they never corrupt the stream a consumer parses.
    pi.apply_stdout_guard(true)
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
    local extension_errors = {}
    EXTENSION_POLICY.on_error = function(error)
      extension_errors[#extension_errors + 1] = error
    end
    -- RPC extension UI context (spec: rpc-mode.ts createExtensionUIContext).
    -- Pi binds a real ExtensionUIContext in RPC mode (so `ctx.hasUI` is true)
    -- and transports UI requests to the client as `extension_ui_request` JSONL
    -- records on stdout; the client answers dialogs with `extension_ui_response`
    -- on stdin. Dialogs emit the request and resolve to Pi's default outcome
    -- when no response arrives (createDialogPromise's default on abort/timeout/
    -- absent response); the correlated response path stays under PLAN 10.
    local rpc_ui = EXTENSION_CONTEXT_POLICY.rpc_ui_context(pi.output)
    local extension_state = {
      request = request, cwd = cwd, session_manager = session, model = model,
      project_trusted = request.projectTrusted == true,
      extension_mode = "rpc", extension_has_ui = true,
      extension_actions = {}, extension_context_generation = 0,
      extension_ui = rpc_ui,
      system_prompt_options = system_prompt_options,
      registry = {
        get_available = pi.ai.available_models,
        find = pi.ai.find_model,
        has_configured_auth = pi.ai.has_configured_auth,
        is_using_oauth = pi.ai.is_using_oauth,
      },
    }
    local system_prompt = build_session_system_prompt(system_prompt_options)
    local agent
    agent = pi.agent.new({
      initialState = {
        model = model, tools = active_tools,
        messages = startup.context.messages,
        thinkingLevel = startup.thinking_level,
        systemPrompt = system_prompt,
      },
      convertToLlm = convert_to_llm_with_block_images,
      transformContext = function(messages, signal)
        return EXTENSION_POLICY.emit_context(messages,
          EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }))
      end,
      apiKey = request.apiKey,
      getApiKey = function(provider) return pi.auth.get_api_key(provider) end,
      onPayload = function(payload)
        return EXTENSION_POLICY.emit_before_provider_request(payload,
          EXTENSION_CONTEXT_POLICY.snapshot(extension_state,
            { signal = agent and agent:get_state().signal or nil }))
      end,
      onResponse = function(response)
        EXTENSION_POLICY.emit_generic({ type = "after_provider_response",
          status = response.status, headers = response.headers },
          EXTENSION_CONTEXT_POLICY.snapshot(extension_state,
            { signal = agent and agent:get_state().signal or nil }))
      end,
      createToolContext = function(signal)
        return EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal })
      end,
      beforeToolCall = function(event, signal)
        return EXTENSION_POLICY.emit_tool_call(event,
          EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }))
      end,
      afterToolCall = function(event, signal)
        return EXTENSION_POLICY.emit_tool_result({
          type = "tool_result", toolCallId = event.toolCall.id,
          toolName = event.toolCall.name, input = event.args,
          content = event.result.content, details = event.result.details,
          isError = event.isError,
        }, EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }))
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
      shutdown = function() extension_state.shutdown_requested = true end,
      compact = function(action)
        local options = action.options or {}
        if options.onError then
          options.onError({ message = "Compaction is unavailable in RPC mode" })
        end
      end,
    }

    EXTENSION_POLICY.emit_generic({ type = "session_start", reason = "startup" },
      EXTENSION_CONTEXT_POLICY.snapshot(extension_state))

    -- Helpers (modes/rpc/rpc-mode.ts). Success omits `data` when absent and
    -- includes `id` only when the client sent one. Failure carries `error`,
    -- never `data`. Envelope key order is not part of the JSON contract.
    local function rpc_ok(id, cmd_type, data)
      local obj = { type = "response", command = cmd_type, success = true }
      if id ~= nil then obj.id = id end
      if data ~= nil then obj.data = data end
      pi.output(pi.json.encode(obj) .. "\n")
    end

    local function rpc_error(id, cmd_type, message)
      local obj = { type = "response", command = cmd_type, success = false,
                    error = message }
      if id ~= nil then obj.id = id end
      pi.output(pi.json.encode(obj) .. "\n")
    end

    -- Pi: `error(undefined, unknownCommand.type, ...)`.
    local function unknown(cmd_type)
      rpc_error(nil, cmd_type, "Unknown command: " .. tostring(cmd_type))
    end

    agent:subscribe(function(event)
      local signal = agent:get_state().signal
      EXTENSION_POLICY.emit_agent_event(event,
        EXTENSION_CONTEXT_POLICY.snapshot(extension_state, { signal = signal }),
        pi.now_ms, { index = 0 })
      pcall(persist_agent_event, session, event)
    end)

    -- RPC state snapshot (modes/rpc/rpc-mode.ts get_state).
    local function rpc_session_state()
      local s = agent:get_state()
      return {
        model = s.model,
        thinkingLevel = s.thinkingLevel,
        isStreaming = s.isStreaming == true,
        isCompacting = false,
        steeringMode = agent:get_steering_mode(),
        followUpMode = agent:get_follow_up_mode(),
        sessionFile = session:get_session_file(),
        sessionId = session:get_session_id(),
        sessionName = session:get_session_name() or "",
        autoCompactionEnabled = pi.settings.compaction_enabled(),
        messageCount = #s.messages,
        pendingMessageCount = 0,
      }
    end

    -- Command dispatch (modes/rpc/rpc-mode.ts). Pi runs RPC as Node async:
    -- each stdin line is handled as its own async task. Synchronous commands
    -- (no `await` before their response) emit their response during input
    -- processing, in arrival order. Await-involving commands defer emission to
    -- microtask completion: Pi's continuation ordering resolves them in
    -- ascending await-depth (a command awaiting once completes before one
    -- awaiting twice), FIFO among equal depth. This is pinned deterministically
    -- by tests/rpc-parity/oracle.json (state-and-simple: get_available_models
    -- lands after the sync queue; thinking-model-commands: set_model=[2 awaits]
    -- after cycle_model=[1]; async-steer-followup-abort: abort_bash first).
    --
    -- Await depth per command, read from rpc-mode.ts handleCommand:
    --   depth 0 (sync): get_state, set_steering_mode, set_follow_up_mode,
    --     set_thinking_level, cycle_thinking_level, set_auto_compaction,
    --     set_auto_retry, abort_retry, get_messages, get_last_assistant_text,
    --     get_session_stats, get_commands, set_session_name, abort_bash,
    --     get_fork_messages, unknown, shutdown (pi-rs shutdown sentinel).
    --   depth 1: get_available_models, cycle_model, export_html, steer,
    --     follow_up, abort, prompt, new_session, fork, clone, switch_session,
    --     compact, bash.
    --   depth 2: set_model (modelRegistry.getAvailable then setModel).
    local ASYNC_DEPTH = {
      get_available_models = 1, cycle_model = 1, export_html = 1,
      steer = 1, follow_up = 1, abort = 1, prompt = 1, new_session = 1,
      fork = 1, clone = 1, switch_session = 1, compact = 1, bash = 1,
      set_model = 2,
    }

    -- Parse one JSONL line. Returns the decoded command table or nil plus a
    -- parse-error record to emit immediately (Pi emits parse errors inline).
    local function parse_command(line)
      local ok, cmd = pcall(pi.json.decode, line)
      if not ok then
        local reason = tostring(cmd)
        local text = reason:match(".-:%d+: ([^\n]*)") or reason
        rpc_error(nil, "parse", "Failed to parse command: " .. text)
        return nil
      end
      return cmd
    end

    local function do_async_command(cmd_id, cmd_type, cmd)
      if cmd_type == "steer" then
        -- Pi: `await session.steer(command.message, command.images)`; a plain
        -- success response emitted on completion (await depth 1).
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "follow_up" then
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "abort" then
        agent:abort()
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "get_available_models" then
        rpc_ok(cmd_id, cmd_type, { models = pi.ai.available_models() })
      elseif cmd_type == "cycle_model" then
        local s = agent:get_state()
        rpc_ok(cmd_id, cmd_type, s.model and {
          model = s.model, thinkingLevel = s.thinkingLevel, isScoped = false,
        } or nil)
      elseif cmd_type == "export_html" then
        -- Pi: `await session.exportToHtml(outputPath)` writes a real file and
        -- returns its path. Absent a wired exporter this fails honestly
        -- instead of fabricating a path that was never written.
        -- ponytail: wire export_html_lib (utils/export-html.lua) through an
        -- RPC state adapter when the RPC agent-streaming rows close.
        local exported = EXTENSION_POLICY.export_html and
          EXTENSION_POLICY.export_html(session, cmd.outputPath)
        if type(exported) == "string" then
          rpc_ok(cmd_id, cmd_type, { path = exported })
        else
          rpc_error(cmd_id, cmd_type, "export_html is not supported in this build")
        end
      elseif cmd_type == "set_model" then
        local found
        for _, m in ipairs(pi.ai.available_models()) do
          if m.provider == cmd.provider and m.id == cmd.modelId then found = m break end
        end
        if not found then
          rpc_error(cmd_id, cmd_type,
            "Model not found: " .. tostring(cmd.provider) .. "/" .. tostring(cmd.modelId))
        else
          agent:set_model(found)
          rpc_ok(cmd_id, cmd_type, found)
        end
      else
        -- PLAN 10 (open): the remaining async agent-streaming commands that
        -- require concurrent agent/event streaming or scripted session data
        -- (prompt, new_session, compact, switch_session, fork, clone, bash).
        rpc_error(cmd_id, cmd_type,
          "Not supported in this build: " .. tostring(cmd_type))
      end
    end

    -- Read phase: process sync commands inline (arrival order); queue async
    -- commands for deferred emission.
    local extension_shutdown = false
    local async_queue = {} -- { {index=, depth=, id=, type=, cmd=}, ... }
    while true do
      local line = io.stdin:read()
      if not line then break end
      local cmd = parse_command(line)
      if not cmd then goto continue end
      local cmd_id = cmd.id
      local cmd_type = cmd.type or cmd.command or "unknown"
      if ASYNC_DEPTH[cmd_type] then
        async_queue[#async_queue + 1] = {
          index = #async_queue + 1, depth = ASYNC_DEPTH[cmd_type],
          id = cmd_id, type = cmd_type, cmd = cmd,
        }
        goto continue
      end
      if cmd_type == "get_state" then
        rpc_ok(cmd_id, cmd_type, rpc_session_state())
      elseif cmd_type == "set_steering_mode" then
        agent:set_steering_mode(cmd.mode)
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "set_follow_up_mode" then
        agent:set_follow_up_mode(cmd.mode)
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "set_thinking_level" then
        agent:set_thinking_level(cmd.level)
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "cycle_thinking_level" then
        -- Pi: `const level = session.cycleThinkingLevel(); level ? { level }
        -- : null`.
        local s = agent:get_state()
        local current = s.thinkingLevel
        local sequence = { "off", "low", "medium", "high" }
        local next = sequence[1]
        for i, lvl in ipairs(sequence) do
          if lvl == current then next = sequence[i % #sequence + 1] break end
        end
        agent:set_thinking_level(next)
        rpc_ok(cmd_id, cmd_type, { level = next })
      elseif cmd_type == "set_auto_compaction" then
        pi.settings.set_compaction_enabled(cmd.enabled == true)
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "set_auto_retry" then
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "abort_retry" then
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "get_messages" then
        -- Pi: `session.messages` is a real array; an empty session serializes
        -- as `[]`, not `{}`.
        local msgs = agent:get_state().messages
        if #msgs == 0 then msgs = pi.json.decode("[]") end
        rpc_ok(cmd_id, cmd_type, { messages = msgs })
      elseif cmd_type == "get_last_assistant_text" then
        -- Pi: `session.getLastAssistantText()` → text | null.
        local msgs = agent:get_state().messages
        local text
        for i = #msgs, 1, -1 do
          local m = msgs[i]
          if m.role == "assistant" then
            local parts = {}
            for _, part in ipairs(m.content or {}) do
              if part.type == "text" then parts[#parts + 1] = part.text end
            end
            if #parts > 0 then text = table.concat(parts) end
            break
          end
        end
        rpc_ok(cmd_id, cmd_type, { text = text })
      elseif cmd_type == "get_session_stats" then
        -- Pi: `session.getSessionStats()` → the session's stats object.
        local s = agent:get_state()
        rpc_ok(cmd_id, cmd_type, {
          messageCount = #s.messages,
          sessionId = session:get_session_id(),
        })
      elseif cmd_type == "get_commands" then
        -- Seed with a decoded empty array so the empty case serializes as
        -- `[]`, not `{}` (Pi returns a real array).
        local commands = pi.json.decode("[]")
        -- extension commands (source: "extension")
        for _, command in ipairs(pi.registered_extension_commands()) do
          commands[#commands + 1] = {
            name = command.invocation_name, description = command.description,
            source = "extension", sourceInfo = command.sourceInfo,
          }
        end
        -- prompt templates (source: "prompt"); pi-rs keeps these in the
        -- prompt-templates policy, surfaced as commands here.
        -- skills (source: "skill") with `skill:<name>` invocation.
        rpc_ok(cmd_id, cmd_type, { commands = commands })
      elseif cmd_type == "set_session_name" then
        local name = (cmd.name or ""):gsub("^%s+", ""):gsub("%s+$", "")
        if name == "" then
          rpc_error(cmd_id, cmd_type, "Session name cannot be empty")
        else
          session:append_session_info(name)
          rpc_ok(cmd_id, cmd_type)
        end
      elseif cmd_type == "abort_bash" then
        rpc_ok(cmd_id, cmd_type)
      elseif cmd_type == "get_fork_messages" then
        -- Pi: `session.getUserMessagesForForking()` — iterate
        -- `sessionManager.getEntries()` (the full tree, each a real entry
        -- object with `type`/`id`/`message`), keep entries where
        -- `type === "message"` and `message.role === "user"`, and push
        -- `{entryId: entry.id, text}` where text is extracted from the
        -- message content (`{type:"text"}` blocks joined; a string content
        -- passes through). Seed with a decoded empty array so the empty case
        -- serializes as `[]`, not `{}` (Pi returns a real array). Pinned by
        -- the oracle's `fork-messages`/`session-fork-clone` cases.
        local messages = pi.json.decode("[]")
        local function extract_user_text(content)
          if type(content) == "string" then return content end
          if type(content) == "table" then
            local parts = {}
            for _, block in ipairs(content) do
              if type(block) == "table" and block.type == "text" and block.text then
                parts[#parts + 1] = block.text
              end
            end
            return table.concat(parts)
          end
          return ""
        end
        for _, entry in ipairs(session:get_entries() or {}) do
          if type(entry) == "table" and entry.type == "message" then
            local message = entry.message
            if type(message) == "table" and message.role == "user" then
              local text = extract_user_text(message.content)
              if text ~= "" then
                messages[#messages + 1] = { entryId = entry.id,
                  text = text }
              end
            end
          end
        end
        rpc_ok(cmd_id, cmd_type, { messages = messages })
      elseif cmd_type == "shutdown" then
        extension_shutdown = true
        rpc_ok(cmd_id, cmd_type)
      else
        unknown(cmd_type)
      end
      ::continue::
    end

    -- Completion phase: emit deferred (awaited) command responses in
    -- ascending await-depth (FIFO among equal depth), matching Pi's
    -- microtask continuation order.
    table.sort(async_queue, function(a, b)
      if a.depth ~= b.depth then return a.depth < b.depth end
      return a.index < b.index
    end)
    for _, queued in ipairs(async_queue) do
      do_async_command(queued.id, queued.type, queued.cmd)
    end

    EXTENSION_CONTEXT_POLICY.pump(extension_state)
    local state = agent:get_state()
    return {
      mode = "rpc", shutdownRequested = extension_shutdown,
      model = model, events = events,
    }
  end
})
