-- The default one-shot coding agent pack. Policy stays in Lua; Rust only
-- provides the provider, tools, and JSONL persistence mechanisms.
-- Shares the messages.ts / system-prompt.ts fragments with the
-- interactive pack (src/builtins/mod.rs concatenates them ahead of this
-- file).
do
local pi = ...
pi.declare_package({ command_visibility = "internal" })

-- The shared policy fragments arrive through the same public exact-version
-- modules a file-backed package would import (de-inlined chunk-local layer).
local messages = pi.module.require("pi.utils.messages", "1")
local system_prompt_mod = pi.module.require("pi.utils.system-prompt", "1")
local agent_session = pi.module.require("pi.utils.agent-session", "1")
local extensions_mod = pi.module.require("pi.utils.extensions", "1")

-- sdk.ts createAgentSession + agent-session.ts _buildRuntime: activation is
-- explicit declaration data on each registered tool.
local function active_tool_definitions()
  return EXTENSION_POLICY.active_tools()
end

-- Shared one-shot runtime (print/json/rpc): session, agent, extension
-- context, and runtime bridge through the same public seams
-- (sdk.ts createAgentSession + agent-session.ts _buildRuntime). The
-- caller owns the turn loop and the mode-specific stdout/stderr policy.
local function build_one_shot_runtime(request, on_error, written_lines)
  local events = {}
  -- Every stdout line the mode writes (json header/events, rpc responses)
  -- is mirrored here for in-process tests; main.rs ignores it. RPC passes
  -- the shared rpc_written list so response/event/extension_error order
  -- survives across dispatches.
  written_lines = written_lines or {}
  local function write_line(text)
    pi.output(text)
    written_lines[#written_lines + 1] = text
  end
  -- main.ts createSessionManager → sdk.ts createAgentSession: open the
  -- CLI-selected session (--continue/--session) or create a fresh one.
  local session = agent_session.construct_session(request)
  local cwd = session:get_cwd()
  local startup = agent_session.session_startup_from_request(session, request)
  -- Spec (print-mode.ts): json mode writes the session header as the
  -- first stdout record, before any event.
  if request.mode == "json" then
    local header = session:get_header()
    if header then write_line(pi.json.encode(header) .. "\n") end
  end
  local active_tools, active_tool_names = active_tool_definitions()
  local model = startup.model or request.model
  -- main.ts: --api-key mirrors into the VM's auth storage so the
  -- per-request getApiKey seam resolves it (agent-session.ts bindCore).
  if request.runtimeApiKey and model then
    pi.auth.set_runtime_api_key(model.provider, request.runtimeApiKey)
  end
  local system_prompt_options = {
    cwd = cwd, agentDir = request.agentDir, toolNames = active_tool_names,
    readmePath = request.readmePath, docsPath = request.docsPath,
    examplesPath = request.examplesPath,
  }
  local extension_errors = {}
  EXTENSION_POLICY.on_error = on_error or function(error)
    extension_errors[#extension_errors + 1] = error
    -- Spec (print-mode.ts bindExtensions onError): print/json modes report
    -- extension errors on stderr; RPC emits extension_error records instead.
    if request.mode ~= "rpc" then
      io.stderr:write("Extension error (" .. tostring(error.extensionPath)
        .. "): " .. tostring(error.error) .. "\n")
    end
  end
  local extension_state = {
    request = request, cwd = cwd, session_manager = session, model = model,
    project_trusted = request.projectTrusted == true,
    extension_mode = request.mode or "print",
    extension_has_ui = request.mode == "rpc",
    extension_actions = {}, extension_context_generation = 0,
    extension_ui = extensions_mod.headless_ui,
    system_prompt_options = system_prompt_options,
    registry = {
      get_available = pi.ai.available_models,
      find = pi.ai.find_model,
      has_configured_auth = pi.ai.has_configured_auth,
      is_using_oauth = pi.ai.is_using_oauth,
    },
  }
  local system_prompt = system_prompt_mod.build_session_system_prompt(system_prompt_options)
  local agent
  agent = pi.agent.new({
    initialState = {
      model = model, tools = active_tools,
      messages = startup.context.messages,
      thinkingLevel = startup.thinking_level,
      systemPrompt = system_prompt,
    },
    convertToLlm = messages.convert_to_llm_with_block_images,
    transformContext = function(messages, signal)
      return EXTENSION_POLICY.emit_context(messages,
        extensions_mod.context_policy.snapshot(extension_state, { signal = signal }))
    end,
    apiKey = request.apiKey,
    getApiKey = function(provider) return pi.auth.get_api_key(provider) end,
    onPayload = function(payload)
      return EXTENSION_POLICY.emit_before_provider_request(payload,
        extensions_mod.context_policy.snapshot(extension_state,
          { signal = agent and agent:get_state().signal or nil }))
    end,
    onResponse = function(response)
      EXTENSION_POLICY.emit_generic({ type = "after_provider_response",
        status = response.status, headers = response.headers },
        extensions_mod.context_policy.snapshot(extension_state,
          { signal = agent and agent:get_state().signal or nil }))
    end,
    createToolContext = function(signal)
      return extensions_mod.context_policy.snapshot(extension_state, { signal = signal })
    end,
    beforeToolCall = function(event, signal)
      return EXTENSION_POLICY.emit_tool_call(event,
        extensions_mod.context_policy.snapshot(extension_state, { signal = signal }))
    end,
    afterToolCall = function(event, signal)
      return EXTENSION_POLICY.emit_tool_result({
        type = "tool_result", toolCallId = event.toolCall.id,
        toolName = event.toolCall.name, input = event.args,
        content = event.result.content, details = event.result.details,
        isError = event.isError,
      }, extensions_mod.context_policy.snapshot(extension_state, { signal = signal }))
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
    -- response (runRpcMode shutdownRequested).
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
  local turn_state = { index = 0 }
  agent:subscribe(function(event)
    local signal = agent:get_state().signal
    EXTENSION_POLICY.emit_agent_event(event,
      extensions_mod.context_policy.snapshot(extension_state, { signal = signal }),
      pi.now_ms, turn_state)
    -- Spec: json/rpc modes stream every session event as one JSONL record.
    if request.mode == "json" or request.mode == "rpc" then
      write_line(pi.json.encode(event) .. "\n")
    end
    agent_session.persist_agent_event(session, event)
  end)
  -- PLAN 9.4: the ExtensionAPI runtime bridge — product policy for the
  -- non-UI members the host api.rs delegates here (agent-session.ts
  -- bindCore). One-shot mode has no UI loop, so message delivery routes
  -- through the same agent steer/follow-up/prompt seams.
  local runtime_bridge = {}
  runtime_bridge.append_entry = function(custom_type, data)
    session:append_custom_entry(custom_type, data)
  end
  runtime_bridge.set_session_name = function(name)
    session:append_session_info(name)
  end
  runtime_bridge.get_session_name = function()
    return session:get_session_name()
  end
  runtime_bridge.set_label = function(entry_id, label)
    session:append_label_change(entry_id, label)
  end
  runtime_bridge.get_active_tools = function()
    local names = {}
    for i, tool in ipairs(agent:get_state().tools or {}) do
      names[i] = tool.name
    end
    return names
  end
  runtime_bridge.set_active_tools = function(tool_names)
    local known = {}
    for _, def in ipairs(pi.registered_tools()) do known[def.name] = def end
    local tools, valid = {}, {}
    for _, name in ipairs(tool_names) do
      local def = known[name]
      if def then tools[#tools + 1] = def; valid[#valid + 1] = name end
    end
    agent:set_tools(tools)
    local options = {
      cwd = cwd, agentDir = request.agentDir, toolNames = valid,
      readmePath = request.readmePath, docsPath = request.docsPath,
      examplesPath = request.examplesPath,
    }
    agent:set_system_prompt(system_prompt_mod.build_session_system_prompt(options))
  end
  runtime_bridge.set_model = function(model)
    if not extension_state.registry.has_configured_auth(model) then return false end
    extension_state.model = model
    agent:set_model(model)
    session:append_model_change(model.provider, model.id)
    return true
  end
  runtime_bridge.get_thinking_level = function()
    return extension_state.thinking_level
  end
  runtime_bridge.set_thinking_level = function(level)
    extension_state.thinking_level = level
    agent:set_thinking_level(level)
    session:append_thinking_level_change(level)
  end
  runtime_bridge.send_message = function(message, options)
    local app_message = {
      role = "custom", customType = message.customType, content = message.content,
      display = message.display, details = message.details, timestamp = pi.now_ms(),
    }
    options = options or {}
    if agent:get_state().isStreaming then
      if options.deliverAs == "followUp" then agent:follow_up(app_message)
      else agent:steer(app_message) end
    elseif options.triggerTurn then
      pi.spawn(function() agent:prompt({ app_message }) end)
    else
      local messages = agent:get_state().messages
      messages[#messages + 1] = app_message
      session:append_custom_message_entry(
        app_message.customType, app_message.content, app_message.display, app_message.details)
    end
  end
  runtime_bridge.send_user_message = function(content, options)
    local text
    if type(content) == "string" then
      text = content
    else
      local parts = {}
      for _, part in ipairs(content) do
        if part.type == "text" then parts[#parts + 1] = part.text end
      end
      text = table.concat(parts, "\n")
    end
    options = options or {}
    local message = { role = "user", content = { { type = "text", text = text } },
      timestamp = pi.now_ms() }
    if agent:get_state().isStreaming then
      if options.deliverAs == "followUp" then agent:follow_up(message)
      else agent:steer(message) end
    else
      pi.spawn(function() agent:prompt({ message }) end)
    end
  end
  pi.install_runtime_bridge(runtime_bridge)

  EXTENSION_POLICY.emit_generic({ type = "session_start", reason = "startup" },
    extensions_mod.context_policy.snapshot(extension_state))
  local extension_resources = EXTENSION_POLICY.emit_resources_discover(
    cwd, "startup", extensions_mod.context_policy.snapshot(extension_state))

  return {
    request = request, events = events, session = session, cwd = cwd,
    startup = startup, model = model, agent = agent,
    system_prompt = system_prompt, system_prompt_options = system_prompt_options,
    extension_state = extension_state, extension_errors = extension_errors,
    extension_resources = extension_resources, turn_state = turn_state,
    written_lines = written_lines,
  }
end

pi.register_role({
  id = "coding-agent-print", role = "print", active = true, priority = 0,
  handler = function(args, ctx)
    local request = pi.json.decode(args)
    local rt = build_one_shot_runtime(request)
    local turn = pi.spawn(function()
      local context = extensions_mod.context_policy.snapshot(rt.extension_state)
      local input = EXTENSION_POLICY.emit_input(request.prompt, nil, "interactive", nil, context)
      if input.action == "handled" then return end
      local prompt = input.action == "transform" and input.text or request.prompt
      local before = EXTENSION_POLICY.emit_before_agent_start(prompt, nil, rt.system_prompt,
        rt.system_prompt_options, extensions_mod.context_policy.snapshot(rt.extension_state))
      rt.agent:set_system_prompt(before and before.systemPrompt or rt.system_prompt)
      local prompts = { { role = "user", content = { { type = "text", text = prompt } },
        timestamp = pi.now_ms() } }
      for _, message in ipairs(before and before.messages or {}) do
        prompts[#prompts + 1] = { role = "custom", customType = message.customType,
          content = message.content, display = message.display, details = message.details,
          timestamp = pi.now_ms() }
      end
      rt.agent:prompt(prompts)
    end)
    while not turn:done() do
      extensions_mod.context_policy.pump(rt.extension_state)
      pi.sleep(1)
    end
    turn:join()
    extensions_mod.context_policy.pump(rt.extension_state)
    -- Spec (print-mode.ts): text mode prints the final assistant text
    -- parts, each followed by "\n"; error/aborted stop reasons print the
    -- error to stderr and exit 1 (main.rs maps them). Nothing streams.
    local state = rt.agent:get_state()
    local last = state.messages[#state.messages]
    local text_parts, text = {}, ""
    local stop_reason, error_message = nil, nil
    if last and last.role == "assistant" then
      stop_reason = last.stopReason
      error_message = last.errorMessage
      for _, part in ipairs(last.content or {}) do
        if part.type == "text" then
          text_parts[#text_parts + 1] = part.text or ""
          text = text .. (part.text or "")
        end
      end
    end
    return {
      text = text, textParts = text_parts, events = rt.events,
      sessionPath = rt.session:get_session_file(),
      model = rt.startup.model and { provider = rt.startup.model.provider, id = rt.startup.model.id } or nil,
      thinkingLevel = rt.startup.thinking_level,
      modelFallbackMessage = rt.startup.fallback_message,
      stopReason = stop_reason, errorMessage = error_message,
      extensionErrors = rt.extension_errors, extensionResources = rt.extension_resources,
      writtenLines = rt.written_lines,
    }
  end,
})

-- runRpcMode (modes/rpc/rpc-mode.ts): JSON-L command loop over stdin.
-- Rust owns the line reader (main.rs run_rpc_mode) and dispatches each
-- command through this generic role; Lua owns the session, command
-- semantics, and every JSON line written to stdout (responses, events,
-- extension_ui_request). The runtime outlives the dispatch: it lives in
-- the chunk-local rpc_runtime so subsequent commands (get_state, steer,
-- prompt, ...) see the same session/agent.
local rpc_runtime = nil
-- Every JSON line the rpc role writes, mirrored for in-process tests.
local rpc_written = {}

local function rpc_output(obj)
  local line = pi.json.encode(obj) .. "\n"
  pi.output(line)
  rpc_written[#rpc_written + 1] = line
end

local function rpc_success(id, command, data)
  local response = { id = id, type = "response", command = command, success = true }
  if data ~= nil then response.data = data end
  rpc_output(response)
end

local function rpc_error(id, command, message)
  rpc_output({ id = id, type = "response", command = command, success = false, error = message })
end

local function rpc_command_kind(command)
  if type(command) == "table" then
    return type(command.type) == "string" and command.type or "rpc"
  end
  return "rpc"
end

-- RPC extension UI: emit extension_ui_request records (fire-and-forget)
-- and return the pinned no-UI outcomes for dialogs (runRpcMode's
-- createExtensionUIContext defaults on abort/timeout; a synchronous
-- dispatch cannot await the client's extension_ui_response line).
local function rpc_ui()
  local ui = {}
  local serial = 0
  local function fire(method, fields)
    serial = serial + 1
    local request = { type = "extension_ui_request",
      id = "ui-" .. pi.monotonic_ms() .. "-" .. serial, method = method }
    for key, value in pairs(fields) do request[key] = value end
    rpc_output(request)
  end
  local theme = {
    fg = function(_, _, text) return text end, bg = function(_, _, text) return text end,
    bold = function(_, text) return text end, italic = function(_, text) return text end,
    underline = function(_, text) return text end, strikethrough = function(_, text) return text end,
  }
  ui.select = function(title, options, opts)
    fire("select", { title = title, options = options, timeout = opts and opts.timeout })
    return nil
  end
  ui.confirm = function(title, message, opts)
    fire("confirm", { title = title, message = message, timeout = opts and opts.timeout })
    return false
  end
  ui.input = function(title, placeholder, opts)
    fire("input", { title = title, placeholder = placeholder, timeout = opts and opts.timeout })
    return nil
  end
  ui.notify = function(message, notify_type)
    fire("notify", { message = message, notifyType = notify_type })
  end
  ui.onTerminalInput = function() return function() end end
  ui.setStatus = function(key, text)
    fire("setStatus", { statusKey = key, statusText = text })
  end
  ui.setWorkingMessage = function() end
  ui.setWorkingVisible = function() end
  ui.setWorkingIndicator = function() end
  ui.setHiddenThinkingLabel = function() end
  ui.setWidget = function(key, content, options)
    if content == nil or type(content) == "table" then
      fire("setWidget", { widgetKey = key, widgetLines = content,
        widgetPlacement = options and options.placement })
    end
  end
  ui.setFooter = function() end
  ui.setHeader = function() end
  ui.setTitle = function(title)
    fire("setTitle", { title = title })
  end
  ui.custom = function() return nil end
  ui.pasteToEditor = function(text) ui.setEditorText(text) end
  ui.setEditorText = function(text)
    fire("set_editor_text", { text = text })
  end
  ui.getEditorText = function() return "" end
  ui.editor = function(title, prefill)
    fire("editor", { title = title, prefill = prefill })
    return nil
  end
  ui.addAutocompleteProvider = function() end
  ui.setEditorComponent = function() end
  ui.getEditorComponent = function() return nil end
  ui.theme = theme
  ui.getAllThemes = function() return {} end
  ui.getTheme = function() return nil end
  ui.setTheme = function()
    return { success = false, error = "Theme switching not supported in RPC mode" }
  end
  ui.getToolsExpanded = function() return false end
  ui.setToolsExpanded = function() end
  return ui
end

local function rpc_session_state(rt)
  local agent_state = rt.agent:get_state()
  return {
    model = agent_state.model,
    thinkingLevel = agent_state.thinkingLevel,
    isStreaming = agent_state.isStreaming == true,
    isCompacting = false,
    steeringMode = rt.agent:get_steering_mode(),
    followUpMode = rt.agent:get_follow_up_mode(),
    sessionFile = rt.session:get_session_file(),
    sessionId = rt.session:get_session_id(),
    sessionName = rt.session:get_session_name(),
    autoCompactionEnabled = rt.auto_compaction ~= false,
    messageCount = #(agent_state.messages or {}),
    pendingMessageCount = rt.pending_count or 0,
  }
end

local function rpc_user_message(text, images)
  local content = { { type = "text", text = text } }
  for _, image in ipairs(images or {}) do content[#content + 1] = image end
  return { role = "user", content = content, timestamp = pi.now_ms() }
end

-- agent-session.ts _throwIfExtensionCommand: queued "/name" messages error
-- when name is a registered extension command.
local function rpc_extension_command_error(text)
  if type(text) ~= "string" or text:sub(1, 1) ~= "/" then return nil end
  local space = text:find(" ")
  local name = space and text:sub(2, space - 1) or text:sub(2)
  for _, command in ipairs(pi.registered_extension_commands()) do
    if command.invocation_name == name then
      return 'Extension command "/' .. name
        .. '" cannot be queued. Use prompt() or execute the command when not streaming.'
    end
  end
  return nil
end

local function rpc_find_tool(name)
  for _, def in ipairs(pi.registered_tools()) do
    if def.name == name then return def end
  end
  return nil
end

local function rpc_handle_command(rt, command)
  local id = command.id
  local kind = command.type
  if kind == "prompt" then
    -- session.prompt preflight: input interception then before_agent_start.
    -- The authoritative response is emitted only after preflight succeeds;
    -- queued/immediately handled prompts also count as success.
    if rt.agent:get_state().isStreaming then
      rpc_error(id, "prompt",
        "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message.")
      return
    end
    local ok_preflight, preflight = pcall(function()
      local context = extensions_mod.context_policy.snapshot(rt.extension_state)
      local input = EXTENSION_POLICY.emit_input(command.message, command.images, "rpc", nil, context)
      if input.action == "handled" then return "handled" end
      local prompt = input.action == "transform" and input.text or command.message
      local before = EXTENSION_POLICY.emit_before_agent_start(prompt, command.images,
        rt.system_prompt, rt.system_prompt_options,
        extensions_mod.context_policy.snapshot(rt.extension_state))
      return { prompt = prompt, before = before }
    end)
    if not ok_preflight then
      rpc_error(id, "prompt", EXTENSION_POLICY.error_text(preflight))
      return
    end
    rpc_success(id, "prompt")
    if preflight ~= "handled" then
      local prompt = preflight.prompt
      local before = preflight.before
      rt.agent:set_system_prompt(before and before.systemPrompt or rt.system_prompt)
      local turn = pi.spawn(function()
        local content = { { type = "text", text = prompt } }
        for _, image in ipairs(command.images or {}) do content[#content + 1] = image end
        local prompts = { { role = "user", content = content, timestamp = pi.now_ms() } }
        for _, message in ipairs(before and before.messages or {}) do
          prompts[#prompts + 1] = { role = "custom", customType = message.customType,
            content = message.content, display = message.display, details = message.details,
            timestamp = pi.now_ms() }
        end
        rt.agent:prompt(prompts)
      end)
      while not turn:done() do
        extensions_mod.context_policy.pump(rt.extension_state)
        pi.sleep(1)
      end
      turn:join()
      extensions_mod.context_policy.pump(rt.extension_state)
    end
    rt.pending_count = 0
    return
  end
  if kind == "steer" or kind == "follow_up" then
    local queued_error = rpc_extension_command_error(command.message)
    if queued_error then
      rpc_error(id, kind, queued_error)
      return
    end
    local message = rpc_user_message(command.message, command.images)
    if kind == "steer" then rt.agent:steer(message) else rt.agent:follow_up(message) end
    rt.pending_count = (rt.pending_count or 0) + 1
    rpc_success(id, kind)
    return
  end
  if kind == "abort" then
    rt.agent:abort()
    rpc_success(id, "abort")
    return
  end
  if kind == "get_state" then
    rpc_success(id, "get_state", rpc_session_state(rt))
    return
  end
  if kind == "new_session" then
    local ok, err = pcall(function()
      rt.session:new_session(command.parentSession and { parentSession = command.parentSession } or {})
    end)
    if not ok then
      rpc_error(id, "new_session", tostring(err))
    else
      rt.agent:reset()
      rt.agent:set_messages({})
      rt.pending_count = 0
      rpc_success(id, "new_session", { cancelled = false })
    end
    return
  end
  if kind == "get_available_models" then
    rpc_success(id, "get_available_models", { models = pi.ai.available_models() })
    return
  end
  if kind == "set_model" then
    local found = nil
    for _, candidate in ipairs(pi.ai.available_models()) do
      if candidate.provider == command.provider and candidate.id == command.modelId then
        found = candidate
        break
      end
    end
    if not found then
      rpc_error(id, "set_model", "Model not found: " .. command.provider .. "/" .. command.modelId)
    else
      rt.extension_state.model = found
      rt.agent:set_model(found)
      rt.session:append_model_change(found.provider, found.id)
      rpc_success(id, "set_model", found)
    end
    return
  end
  if kind == "cycle_model" then
    local candidates = {}
    for _, candidate in ipairs(pi.ai.available_models()) do
      if pi.ai.has_configured_auth(candidate) then candidates[#candidates + 1] = candidate end
    end
    if #candidates <= 1 then
      rpc_success(id, "cycle_model", nil)
    else
      local current = rt.extension_state.model
      local current_index = 0
      for index, candidate in ipairs(candidates) do
        if candidate.provider == current.provider and candidate.id == current.id then
          current_index = index - 1
          break
        end
      end
      local next_model = candidates[(current_index % #candidates) + 1]
      local thinking = rt.thinking_level or "off"
      rt.extension_state.model = next_model
      rt.agent:set_model(next_model)
      rt.session:append_model_change(next_model.provider, next_model.id)
      rpc_success(id, "cycle_model",
        { model = next_model, thinkingLevel = thinking, isScoped = false })
    end
    return
  end
  if kind == "set_thinking_level" then
    local model = rt.extension_state.model
    local available = pi.ai.supported_thinking_levels(model)
    local effective = command.level
    local known = false
    for _, candidate in ipairs(available) do
      if candidate == command.level then known = true break end
    end
    if not known then effective = pi.ai.clamp_thinking_level(model, command.level) end
    rt.thinking_level = effective
    rt.agent:set_thinking_level(effective)
    rt.session:append_thinking_level_change(effective)
    rpc_success(id, "set_thinking_level")
    return
  end
  if kind == "cycle_thinking_level" then
    local model = rt.extension_state.model
    if not (model and model.reasoning) then
      rpc_success(id, "cycle_thinking_level", nil)
    else
      local levels = pi.ai.supported_thinking_levels(model)
      local index = -1
      for i, candidate in ipairs(levels) do
        if candidate == rt.thinking_level then index = i - 1 break end
      end
      local next_level = levels[((index + 1) % #levels) + 1]
      rt.thinking_level = next_level
      rt.agent:set_thinking_level(next_level)
      rt.session:append_thinking_level_change(next_level)
      rpc_success(id, "cycle_thinking_level", { level = next_level })
    end
    return
  end
  if kind == "set_steering_mode" then
    rt.agent:set_steering_mode(command.mode)
    rpc_success(id, "set_steering_mode")
    return
  end
  if kind == "set_follow_up_mode" then
    rt.agent:set_follow_up_mode(command.mode)
    rpc_success(id, "set_follow_up_mode")
    return
  end
  if kind == "compact" then
    -- Compaction is agent-session policy (PLAN 6.5) wired in the
    -- interactive frontend; the one-shot RPC runtime does not run it.
    rpc_error(id, "compact", "Compaction is unavailable in one-shot mode")
    return
  end
  if kind == "set_auto_compaction" then
    rt.auto_compaction = command.enabled == true
    rpc_success(id, "set_auto_compaction")
    return
  end
  if kind == "set_auto_retry" then
    rt.auto_retry = command.enabled == true
    rpc_success(id, "set_auto_retry")
    return
  end
  if kind == "abort_retry" then
    rpc_success(id, "abort_retry")
    return
  end
  if kind == "bash" then
    local bash_tool = rpc_find_tool("bash")
    if not bash_tool then
      rpc_error(id, "bash", "Bash tool is not available")
      return
    end
    local ok, result = pcall(bash_tool.execute, "bash", { command = command.command },
      pi.abort_signal(), nil, extensions_mod.context_policy.snapshot(rt.extension_state))
    if not ok then
      -- The tool raises for abort/timeout/non-zero exit (spec fail()).
      local text = tostring(result)
      local exit_code = nil
      local cancelled = false
      local code = text:match("Command exited with code (%d+)")
      if code then exit_code = tonumber(code) end
      if text:find("Command aborted", 1, true) then cancelled = true end
      rpc_success(id, "bash", { output = text, exitCode = exit_code,
        cancelled = cancelled, truncated = false })
    else
      local text = ""
      for _, part in ipairs(result.content or {}) do
        if part.type == "text" then text = text .. (part.text or "") end
      end
      local details = result.details or {}
      rpc_success(id, "bash", { output = text, exitCode = 0, cancelled = false,
        truncated = details.truncation and details.truncation.truncated or false,
        fullOutputPath = details.fullOutputPath })
    end
    return
  end
  if kind == "abort_bash" then
    rpc_success(id, "abort_bash")
    return
  end
  if kind == "get_session_stats" then
    local messages = rt.agent:get_state().messages or {}
    local user, assistant, tool_results, tool_calls = 0, 0, 0, 0
    local total_input, total_output, total_cache_read, total_cache_write, total_cost = 0, 0, 0, 0, 0
    for _, message in ipairs(messages) do
      if message.role == "user" then
        user = user + 1
      elseif message.role == "assistant" then
        assistant = assistant + 1
        for _, part in ipairs(message.content or {}) do
          if part.type == "toolCall" then tool_calls = tool_calls + 1 end
        end
        local usage = message.usage or {}
        total_input = total_input + (usage.input or 0)
        total_output = total_output + (usage.output or 0)
        total_cache_read = total_cache_read + (usage.cacheRead or 0)
        total_cache_write = total_cache_write + (usage.cacheWrite or 0)
        total_cost = total_cost + ((usage.cost and usage.cost.total) or 0)
      elseif message.role == "toolResult" then
        tool_results = tool_results + 1
      end
    end
    rpc_success(id, "get_session_stats", {
      sessionFile = rt.session:get_session_file(),
      sessionId = rt.session:get_session_id(),
      userMessages = user, assistantMessages = assistant,
      toolCalls = tool_calls, toolResults = tool_results,
      totalMessages = #messages,
      tokens = { input = total_input, output = total_output,
        cacheRead = total_cache_read, cacheWrite = total_cache_write,
        total = total_input + total_output + total_cache_read + total_cache_write },
      cost = total_cost,
    })
    return
  end
  if kind == "export_html" then
    -- Reuse the shipped export-from-file command (interactive pack). The
    -- one-shot pack keeps no export policy of its own.
    local exported = nil
    for _, command in ipairs(pi.registered_extension_commands()) do
      if command.invocation_name == "export-from-file" then
        local ok, result = pcall(command.handler,
          pi.json.encode({ sessionFile = rt.session:get_session_file(),
            outputPath = command.outputPath }),
          extensions_mod.context_policy.snapshot(rt.extension_state))
        if not ok then
          rpc_error(id, "export_html", tostring(result))
        else
          rpc_success(id, "export_html", { path = result.outputPath })
        end
        exported = true
        break
      end
    end
    if not exported then
      rpc_error(id, "export_html", "Export is unavailable: interactive pack not loaded")
    end
    return
  end
  if kind == "switch_session" then
    local ok, err = pcall(function()
      local opened = pi.session.open({ path = command.sessionPath })
      rt.session = opened
      local context = opened:build_session_context()
      rt.agent:reset()
      rt.agent:set_messages(context.messages or {})
      rt.pending_count = 0
    end)
    if not ok then rpc_error(id, "switch_session", tostring(err))
    else rpc_success(id, "switch_session", { cancelled = false }) end
    return
  end
  if kind == "fork" then
    local ok, err = pcall(function()
      rt.session:create_branched_session(command.entryId)
      rt.agent:reset()
      rt.agent:set_messages({})
      rt.pending_count = 0
    end)
    if not ok then rpc_error(id, "fork", tostring(err))
    else rpc_success(id, "fork", { cancelled = false }) end
    return
  end
  if kind == "clone" then
    local leaf = rt.session:get_leaf_id()
    if not leaf then
      rpc_error(id, "clone", "Cannot clone session: no current entry selected")
    else
      rt.session:create_branched_session(leaf)
      rt.agent:reset()
      rt.agent:set_messages({})
      rt.pending_count = 0
      rpc_success(id, "clone", { cancelled = false })
    end
    return
  end
  if kind == "get_fork_messages" then
    local messages = {}
    for _, message in ipairs(rt.session:build_session_context().messages or {}) do
      if message.role == "user" then messages[#messages + 1] = message end
    end
    rpc_success(id, "get_fork_messages", { messages = messages })
    return
  end
  if kind == "get_last_assistant_text" then
    local text = ""
    local messages = rt.agent:get_state().messages or {}
    for i = #messages, 1, -1 do
      local message = messages[i]
      if message.role == "assistant" then
        for _, part in ipairs(message.content or {}) do
          if part.type == "text" then text = text .. (part.text or "") end
        end
        break
      end
    end
    rpc_success(id, "get_last_assistant_text", { text = text })
    return
  end
  if kind == "set_session_name" then
    local name = command.name and (command.name:gsub("^%s*(.-)%s*$", "%1")) or ""
    if name == "" then
      rpc_error(id, "set_session_name", "Session name cannot be empty")
    else
      rt.session:append_session_info(name)
      rpc_success(id, "set_session_name")
    end
    return
  end
  if kind == "get_messages" then
    rpc_success(id, "get_messages", { messages = rt.agent:get_state().messages or {} })
    return
  end
  if kind == "get_commands" then
    local commands = {}
    for _, command in ipairs(pi.registered_extension_commands()) do
      commands[#commands + 1] = {
        name = command.invocation_name, description = command.description,
        source = "extension", sourceInfo = command.source_info,
      }
    end
    rpc_success(id, "get_commands", { commands = commands })
    return
  end
  if kind == "extension_ui_response" then
    -- Dialog responses arrive on stdin after the emitting dispatch
    -- returned; nothing is pending in this synchronous runtime.
    return
  end
  rpc_error(id, kind, "Unknown command: " .. tostring(kind))
end

pi.register_role({
  id = "coding-agent-rpc", role = "rpc", active = true, priority = 0,
  description = "Run the JSON-L RPC protocol (--mode rpc)",
  handler = function(args, ctx)
    local request = pi.json.decode(args)
    local command = request.rpcCommand or {}
    if not rpc_runtime then
      rpc_written = {}
      local function rpc_on_error(error)
        if rpc_runtime then
          rpc_runtime.extension_errors[#rpc_runtime.extension_errors + 1] = error
        end
        rpc_output({ type = "extension_error", extensionPath = error.extensionPath,
          event = error.event, error = error.error })
      end
      -- The shared written_lines list keeps the build-time session_start
      -- extension errors before the first command response, matching
      -- runRpcMode's rebind-before-loop ordering.
      rpc_runtime = build_one_shot_runtime(request, rpc_on_error, rpc_written)
      rpc_runtime.pending_count = 0
      rpc_runtime.auto_compaction = true
      rpc_runtime.thinking_level = rpc_runtime.startup.thinking_level
      -- RPC-specific extension surface: the RPC UI context
      -- (runRpcMode createExtensionUIContext).
      rpc_runtime.extension_state.extension_ui = rpc_ui()
    end
    local ok, err = pcall(rpc_handle_command, rpc_runtime, command)
    if not ok then
      -- Spec: the per-command catch writes an error response carrying the
      -- command type (main.rs falls back to a generic record otherwise).
      rpc_error(command.id, rpc_command_kind(command), tostring(err))
    end
    return { writtenLines = rpc_written }
  end,
})

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
            { cwd = pi.cwd(), mode = "print", hasUI = false, ui = extensions_mod.headless_ui }),
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
        cwd = pi.cwd(), mode = "print", hasUI = false, ui = extensions_mod.headless_ui,
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
    return { prompt = system_prompt_mod.build_system_prompt({
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
  local context_files = system_prompt_mod.load_project_context_files({
    cwd = case.cwd, agentDir = case.agentDir,
  })
  local prompt = system_prompt_mod.build_session_system_prompt({
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
  agent_session.session_startup(session, {
    cliModel = model, fallbackModel = model,
    cliThinking = options.thinkingLevel,
  })
  local counts = {}
  local fired = {}
  agent:subscribe(function(event)
    agent_session.persist_agent_event(session, event)
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
end
