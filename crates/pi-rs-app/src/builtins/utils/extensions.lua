-- Product-side extension composition. Rust exposes registration/handler
-- snapshots; this Lua policy chooses active tools and the complete Pi event
-- fold semantics. Every product seam dispatches through extension_handlers;
-- there is no embedded-only callback path.
EXTENSION_POLICY = EXTENSION_POLICY or {}
EXTENSION_POLICY.api = pi

function EXTENSION_POLICY.active_tools()
  local active, names = {}, {}
  for _, definition in ipairs(EXTENSION_POLICY.api.registered_active_tools()) do
    active[#active + 1] = definition
    names[#names + 1] = definition.name
  end
  return active, names
end

function EXTENSION_POLICY.copied(value)
  if value == nil then return nil end
  return pi.json.decode(pi.json.encode(value))
end

function EXTENSION_POLICY.error_text(value)
  local text = tostring(value)
  text = text:match("^(.-)\nstack traceback:") or text
  text = text:gsub("^runtime error: ", "")
  return text:match("^.-:%d+: (.*)$") or text
end

function EXTENSION_POLICY.report_error(context, entry, event_type, value)
  local error = {
    extensionPath = entry.source,
    event = event_type,
    error = EXTENSION_POLICY.error_text(value),
  }
  if EXTENSION_POLICY.on_error then EXTENSION_POLICY.on_error(error) end
  return error
end

function EXTENSION_POLICY.handlers(event_type)
  return EXTENSION_POLICY.api.extension_handlers(event_type)
end

-- ExtensionRunner.emit: ordinary events isolate handler errors. The four
-- session_before_* events retain the latest result and cancel immediately.
function EXTENSION_POLICY.emit_generic(event, context)
  local result
  local is_before = event.type == "session_before_switch"
    or event.type == "session_before_fork"
    or event.type == "session_before_compact"
    or event.type == "session_before_tree"
  for _, entry in ipairs(EXTENSION_POLICY.handlers(event.type)) do
    local ok, value = pcall(entry.handler, event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, event.type, value)
    elseif is_before and value ~= nil then
      result = value
      if value.cancel then return value end
    end
  end
  return result
end

-- ExtensionRunner.emitToolCall: mutable event.input is shared in order; latest
-- result wins and block short-circuits. This one hook is deliberately fail-safe:
-- errors propagate and settle the tool as blocked/failed in agent-core.
function EXTENSION_POLICY.emit_tool_call(core_event, context)
  local event
  if core_event.type == "tool_call" then
    event = core_event
  else
    event = {
      type = "tool_call",
      toolCallId = core_event.toolCall.id,
      toolName = core_event.toolCall.name,
      input = core_event.args,
    }
  end
  local result
  for _, entry in ipairs(EXTENSION_POLICY.handlers("tool_call")) do
    local value = entry.handler(event, context)
    if value ~= nil then
      result = value
      if value.block then return value end
    end
  end
  return result
end

-- ExtensionRunner.emitToolResult: each partial override updates the event seen
-- by later handlers; omitted keys retain the previous value.
function EXTENSION_POLICY.emit_tool_result(event, context)
  local current = {
    type = "tool_result", toolCallId = event.toolCallId,
    toolName = event.toolName, input = event.input,
    content = event.content, details = event.details, isError = event.isError,
  }
  local modified = false
  for _, entry in ipairs(EXTENSION_POLICY.handlers("tool_result")) do
    local ok, value = pcall(entry.handler, current, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "tool_result", value)
    elseif value ~= nil then
      if value.content ~= nil then current.content = value.content; modified = true end
      if value.details ~= nil then current.details = value.details; modified = true end
      if value.isError ~= nil then current.isError = value.isError; modified = true end
    end
  end
  if not modified then return nil end
  return { content = current.content, details = current.details, isError = current.isError }
end

-- ExtensionRunner.emitMessageEnd: replacements chain; role changes are rejected
-- as attributed errors and do not poison later handlers.
function EXTENSION_POLICY.emit_message_end(event, context)
  local current, modified = event.message, false
  for _, entry in ipairs(EXTENSION_POLICY.handlers("message_end")) do
    local current_event = { type = "message_end", message = current }
    local ok, value = pcall(entry.handler, current_event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "message_end", value)
    elseif value and value.message then
      if value.message.role ~= current.role then
        EXTENSION_POLICY.report_error(context, entry, "message_end",
          "message_end handlers must return a message with the same role")
      else
        current, modified = value.message, true
      end
    end
  end
  return modified and current or nil
end

function EXTENSION_POLICY.emit_context(messages, context)
  local current = EXTENSION_POLICY.copied(messages)
  for _, entry in ipairs(EXTENSION_POLICY.handlers("context")) do
    local event = { type = "context", messages = current }
    local ok, value = pcall(entry.handler, event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "context", value)
    elseif value and value.messages then
      current = value.messages
    end
  end
  return current
end

function EXTENSION_POLICY.emit_before_provider_request(payload, context)
  local current = payload
  for _, entry in ipairs(EXTENSION_POLICY.handlers("before_provider_request")) do
    local event = { type = "before_provider_request", payload = current }
    local ok, value = pcall(entry.handler, event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "before_provider_request", value)
    elseif value ~= nil then
      current = value
    end
  end
  return current
end

function EXTENSION_POLICY.emit_before_agent_start(prompt, images, system_prompt, options, context)
  local current_prompt = system_prompt
  local messages, modified = {}, false
  -- getSystemPrompt is turn-local while this middleware chains.
  local prior_get = context.getSystemPrompt
  context.getSystemPrompt = function() return current_prompt end
  for _, entry in ipairs(EXTENSION_POLICY.handlers("before_agent_start")) do
    local event = {
      type = "before_agent_start", prompt = prompt, images = images,
      systemPrompt = current_prompt, systemPromptOptions = options,
    }
    local ok, value = pcall(entry.handler, event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "before_agent_start", value)
    elseif value then
      if value.message then messages[#messages + 1] = value.message end
      if value.systemPrompt ~= nil then current_prompt, modified = value.systemPrompt, true end
    end
  end
  context.getSystemPrompt = prior_get
  if #messages == 0 and not modified then return nil end
  return {
    messages = #messages > 0 and messages or nil,
    systemPrompt = modified and current_prompt or nil,
  }
end

function EXTENSION_POLICY.emit_resources_discover(cwd, reason, context)
  local result = { skillPaths = {}, promptPaths = {}, themePaths = {} }
  for _, entry in ipairs(EXTENSION_POLICY.handlers("resources_discover")) do
    local ok, value = pcall(entry.handler,
      { type = "resources_discover", cwd = cwd, reason = reason }, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "resources_discover", value)
    elseif value then
      for _, kind in ipairs({ "skillPaths", "promptPaths", "themePaths" }) do
        for _, path in ipairs(value[kind] or {}) do
          result[kind][#result[kind] + 1] = { path = path, extensionPath = entry.source }
        end
      end
    end
  end
  return result
end

function EXTENSION_POLICY.emit_project_trust(event, context)
  for _, entry in ipairs(EXTENSION_POLICY.handlers("project_trust")) do
    local ok, value = pcall(entry.handler, event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "project_trust", value)
    elseif value and value.trusted ~= "undecided" then
      return value
    end
  end
  return nil
end

function EXTENSION_POLICY.emit_user_bash(event, context)
  for _, entry in ipairs(EXTENSION_POLICY.handlers("user_bash")) do
    local ok, value = pcall(entry.handler, event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "user_bash", value)
    elseif value ~= nil then
      return value
    end
  end
  return nil
end

function EXTENSION_POLICY.emit_input(text, images, source, streaming_behavior, context)
  local current_text, current_images = text, images
  for _, entry in ipairs(EXTENSION_POLICY.handlers("input")) do
    local event = {
      type = "input", text = current_text, images = current_images,
      source = source, streamingBehavior = streaming_behavior,
    }
    local ok, value = pcall(entry.handler, event, context)
    if not ok then
      EXTENSION_POLICY.report_error(context, entry, "input", value)
    elseif value and value.action == "handled" then
      return value
    elseif value and value.action == "transform" then
      current_text = value.text
      if value.images ~= nil then current_images = value.images end
    end
  end
  if current_text ~= text or current_images ~= images then
    return { action = "transform", text = current_text, images = current_images }
  end
  return { action = "continue" }
end

function EXTENSION_POLICY.replace_table(target, replacement)
  if target == replacement then return end
  for key in pairs(target) do target[key] = nil end
  for key, value in pairs(replacement) do target[key] = value end
end

function EXTENSION_POLICY.emit_agent_event(event, context, now_ms, turn_state)
  local extension_event
  if event.type == "agent_start" then
    turn_state.index = 0
    extension_event = { type = "agent_start" }
  elseif event.type == "agent_end" then
    extension_event = { type = "agent_end", messages = event.messages }
  elseif event.type == "turn_start" then
    extension_event = { type = "turn_start", turnIndex = turn_state.index,
      timestamp = now_ms() }
  elseif event.type == "turn_end" then
    extension_event = { type = "turn_end", turnIndex = turn_state.index,
      message = event.message, toolResults = event.toolResults }
  elseif event.type == "message_start" then
    extension_event = { type = "message_start", message = event.message }
  elseif event.type == "message_update" then
    extension_event = { type = "message_update", message = event.message,
      assistantMessageEvent = event.assistantMessageEvent }
  elseif event.type == "message_end" then
    local replacement = EXTENSION_POLICY.emit_message_end(
      { type = "message_end", message = event.message }, context)
    if replacement then EXTENSION_POLICY.replace_table(event.message, replacement) end
    return
  elseif event.type == "tool_execution_start" then
    extension_event = { type = event.type, toolCallId = event.toolCallId,
      toolName = event.toolName, args = event.args }
  elseif event.type == "tool_execution_update" then
    extension_event = { type = event.type, toolCallId = event.toolCallId,
      toolName = event.toolName, args = event.args, partialResult = event.partialResult }
  elseif event.type == "tool_execution_end" then
    extension_event = { type = event.type, toolCallId = event.toolCallId,
      toolName = event.toolName, result = event.result, isError = event.isError }
  end
  if extension_event then EXTENSION_POLICY.emit_generic(extension_event, context) end
  if event.type == "turn_end" then turn_state.index = turn_state.index + 1 end
end

function EXTENSION_POLICY.execute_command(text, context, options)
  if text:sub(1, 1) ~= "/" then return false end
  local body = text:sub(2)
  local command_name, args = body:match("^(%S+)%s?(.*)$")
  if not command_name then return false end
  for _, command in ipairs(EXTENSION_POLICY.api.registered_extension_commands()) do
    if command.invocation_name == command_name then
      local function execute()
        local ok, err = pcall(command.handler, args or "", context)
        if not ok and options and options.on_error then options.on_error(tostring(err)) end
        return ok and nil or tostring(err)
      end
      if options and options.background then
        EXTENSION_POLICY.api.spawn(execute)
        return true
      end
      return true, execute()
    end
  end
  return false
end



-- ExtensionContext policy shared by every product mode. Values are copied or
-- exposed through read-only facades; mutations are plain queued actions. The
-- mode loop is the only action applier.
EXTENSION_CONTEXT_POLICY = EXTENSION_CONTEXT_POLICY or {}

EXTENSION_CONTEXT_POLICY.stale_message = "This extension ctx is stale after session replacement or reload. Do not use a captured pi or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload(). For newSession, fork, and switchSession, move post-replacement work into withSession and use the ctx passed to withSession. For reload, do not use the old ctx after await ctx.reload()."

EXTENSION_CONTEXT_POLICY.NO_UI = {
  select = function() return nil end,
  confirm = function() return false end,
  input = function() return nil end,
  editor = function() return nil end,
  notify = function() end,
}

function EXTENSION_CONTEXT_POLICY.readonly_facade(methods)
  return setmetatable({}, {
    __index = methods,
    __newindex = function() error("extension context is read-only", 2) end,
    __metatable = false,
  })
end

function EXTENSION_CONTEXT_POLICY.copy_snapshot(value)
  if value == nil then return nil end
  return EXTENSION_POLICY.api.json.decode(EXTENSION_POLICY.api.json.encode(value))
end

function EXTENSION_CONTEXT_POLICY.assert_active(state, generation)
  if generation ~= state.extension_context_generation then
    error(EXTENSION_CONTEXT_POLICY.stale_message, 3)
  end
end

function EXTENSION_CONTEXT_POLICY.enqueue(state, generation, action)
  EXTENSION_CONTEXT_POLICY.assert_active(state, generation)
  action.generation = generation
  state.extension_actions[#state.extension_actions + 1] = action
  state.async_render = true
end

function EXTENSION_CONTEXT_POLICY.context_is_idle(state)
  if state.extension_is_idle then return state.extension_is_idle() end
  local agent_state = state.agent and state.agent:get_state() or {}
  return agent_state.isStreaming ~= true
end

function EXTENSION_CONTEXT_POLICY.context_has_pending(state)
  if state.extension_has_pending then return state.extension_has_pending() end
  if state.agent and state.agent.has_queued_messages and state.agent:has_queued_messages() then
    return true
  end
  return #(state.steering_texts or {}) + #(state.follow_up_texts or {})
    + #(state.compaction_queued or {}) > 0
end

function EXTENSION_CONTEXT_POLICY.context_usage(state, agent_state)
  if state.extension_context_usage then return state.extension_context_usage() end
  if state.model and (state.model.contextWindow or 0) > 0 then
    local estimate = compaction_lib.estimate_context_tokens(agent_state.messages or {})
    return {
      tokens = estimate.tokens,
      contextWindow = state.model.contextWindow,
      percent = estimate.tokens / state.model.contextWindow * 100,
    }
  end
end

function EXTENSION_CONTEXT_POLICY.await_action(state, generation, action)
  EXTENSION_CONTEXT_POLICY.enqueue(state, generation, action)
  while not action.settled do EXTENSION_POLICY.api.sleep(1) end
  if not action.ok then error(action.error, 2) end
  return action.result
end

function EXTENSION_CONTEXT_POLICY.snapshot(state, options)
  options = options or {}
  local generation = state.extension_context_generation
  local function assert_active()
    EXTENSION_CONTEXT_POLICY.assert_active(state, generation)
  end
  local manager = state.session_manager
  local registry = state.registry
  local agent_state = state.agent and state.agent:get_state() or {}
  local idle = EXTENSION_CONTEXT_POLICY.context_is_idle(state)
  local pending = EXTENSION_CONTEXT_POLICY.context_has_pending(state)
  local usage = EXTENSION_CONTEXT_POLICY.context_usage(state, agent_state)
  local session_snapshot = EXTENSION_CONTEXT_POLICY.readonly_facade({
    get_session_file = function() assert_active(); return manager:get_session_file() end,
    get_session_id = function() assert_active(); return manager:get_session_id() end,
    get_session_name = function() assert_active(); return manager:get_session_name() end,
    get_cwd = function() assert_active(); return manager:get_cwd() end,
    get_leaf_id = function() assert_active(); return manager:get_leaf_id() end,
    get_header = function()
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(manager:get_header())
    end,
    get_entries = function()
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(manager:get_entries())
    end,
    get_branch = function()
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(manager:get_branch())
    end,
    get_entry = function(id)
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(manager:get_entry(id))
    end,
    build_session_context = function()
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(manager:build_session_context())
    end,
    is_persisted = function() assert_active(); return manager:is_persisted() end,
  })
  local registry_snapshot = EXTENSION_CONTEXT_POLICY.readonly_facade({
    get_available = function()
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(registry.get_available())
    end,
    find = function(provider, id)
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(registry.find(provider, id))
    end,
    has_configured_auth = function(model)
      assert_active(); return registry.has_configured_auth(model)
    end,
    is_using_oauth = function(model)
      assert_active(); return registry.is_using_oauth(model)
    end,
  })
  local context = {
    ui = state.extension_ui or EXTENSION_CONTEXT_POLICY.NO_UI,
    mode = state.extension_mode or "print",
    hasUI = state.extension_has_ui == true,
    cwd = state.cwd,
    sessionManager = session_snapshot,
    modelRegistry = registry_snapshot,
    model = EXTENSION_CONTEXT_POLICY.copy_snapshot(state.model),
    signal = options.signal,
    isIdle = function() assert_active(); return idle end,
    isProjectTrusted = function() assert_active(); return state.project_trusted == true end,
    hasPendingMessages = function() assert_active(); return pending end,
    getContextUsage = function()
      assert_active(); return EXTENSION_CONTEXT_POLICY.copy_snapshot(usage)
    end,
    getSystemPrompt = function() assert_active(); return agent_state.systemPrompt or "" end,
    abort = function()
      EXTENSION_CONTEXT_POLICY.enqueue(state, generation, { kind = "abort" })
    end,
    shutdown = function()
      EXTENSION_CONTEXT_POLICY.enqueue(state, generation, { kind = "shutdown" })
    end,
    compact = function(compact_options)
      EXTENSION_CONTEXT_POLICY.enqueue(state, generation,
        { kind = "compact", options = compact_options or {} })
    end,
  }
  if options.command then
    context.getSystemPromptOptions = function()
      assert_active()
      return EXTENSION_CONTEXT_POLICY.copy_snapshot(
        state.system_prompt_options or { cwd = state.cwd })
    end
    context.waitForIdle = function()
      return EXTENSION_CONTEXT_POLICY.await_action(state, generation, { kind = "wait_idle" })
    end
    context.newSession = function(action_options)
      return EXTENSION_CONTEXT_POLICY.await_action(state, generation,
        { kind = "new_session", options = action_options or {} })
    end
    context.fork = function(entry_id, action_options)
      return EXTENSION_CONTEXT_POLICY.await_action(state, generation, {
        kind = "fork", entryId = entry_id, options = action_options or {},
      })
    end
    context.navigateTree = function(target_id, action_options)
      return EXTENSION_CONTEXT_POLICY.await_action(state, generation, {
        kind = "navigate_tree", targetId = target_id, options = action_options or {},
      })
    end
    context.switchSession = function(session_path, action_options)
      return EXTENSION_CONTEXT_POLICY.await_action(state, generation, {
        kind = "switch_session", sessionPath = session_path, options = action_options or {},
      })
    end
    context.reload = function()
      return EXTENSION_CONTEXT_POLICY.await_action(state, generation, { kind = "reload" })
    end
  end
  return context
end

function EXTENSION_CONTEXT_POLICY.settle_action(action, ok, value)
  action.ok = ok
  if ok then action.result = value else action.error = tostring(value) end
  action.settled = true
end

function EXTENSION_CONTEXT_POLICY.pump(state)
  local remaining = {}
  for _, action in ipairs(state.extension_actions) do
    if action.settled then
      -- The waiter owns the result; removing the queue entry is safe.
    elseif action.kind == "wait_idle" and not EXTENSION_CONTEXT_POLICY.context_is_idle(state) then
      remaining[#remaining + 1] = action
    elseif action.kind == "wait_idle" then
      EXTENSION_CONTEXT_POLICY.settle_action(action, true, nil)
    elseif not action.started and action.generation ~= state.extension_context_generation then
      EXTENSION_CONTEXT_POLICY.settle_action(action, false, EXTENSION_CONTEXT_POLICY.stale_message)
    elseif action.kind == "abort" then
      local handler = state.extension_action_handlers and state.extension_action_handlers.abort
      if handler then handler(action) end
      EXTENSION_CONTEXT_POLICY.settle_action(action, true, nil)
    elseif action.kind == "shutdown" then
      local handler = state.extension_action_handlers and state.extension_action_handlers.shutdown
      if handler then handler(action) end
      EXTENSION_CONTEXT_POLICY.settle_action(action, true, nil)
    elseif action.kind == "compact" then
      local handler = state.extension_action_handlers and state.extension_action_handlers.compact
      if handler then handler(action) end
      EXTENSION_CONTEXT_POLICY.settle_action(action, true, nil)
    elseif action.started then
      remaining[#remaining + 1] = action
    else
      local handler = state.extension_action_handlers
        and state.extension_action_handlers[action.kind]
      if not handler then
        EXTENSION_CONTEXT_POLICY.settle_action(action, false,
          "Extension action is not available in "
            .. tostring(state.extension_mode or "print") .. " mode: " .. action.kind)
      else
        action.started = true
        action.task = EXTENSION_POLICY.api.spawn(function()
          EXTENSION_CONTEXT_POLICY.settle_action(action, pcall(handler, action))
          state.async_render = true
        end)
        remaining[#remaining + 1] = action
      end
    end
  end
  state.extension_actions = remaining
  if state.extension_after_pump then state.extension_after_pump() end
end

-- Pi runner.ts noOpUIContext for print/json modes. Mutations are inert and
-- dialog calls return the pinned no-UI outcomes without touching frontend state.
EXTENSION_HEADLESS_UI = EXTENSION_HEADLESS_UI or (function()
  local theme = {
    fg = function(_, _, text) return text end, bg = function(_, _, text) return text end,
    bold = function(_, text) return text end, italic = function(_, text) return text end,
    underline = function(_, text) return text end, strikethrough = function(_, text) return text end,
  }
  return {
    select = function() return nil end, confirm = function() return false end,
    input = function() return nil end, notify = function() end,
    onTerminalInput = function() return function() end end,
    setStatus = function() end, setWorkingMessage = function() end,
    setWorkingVisible = function() end, setWorkingIndicator = function() end,
    setHiddenThinkingLabel = function() end, setWidget = function() end,
    setFooter = function() end, setHeader = function() end, setTitle = function() end,
    custom = function() return nil end, pasteToEditor = function() end,
    setEditorText = function() end, getEditorText = function() return "" end,
    editor = function() return nil end, addAutocompleteProvider = function() end,
    setEditorComponent = function() end, getEditorComponent = function() return nil end,
    theme = theme, getAllThemes = function() return {} end, getTheme = function() return nil end,
    setTheme = function() return { success = false, error = "UI not available" } end,
    getToolsExpanded = function() return false end, setToolsExpanded = function() end,
  }
end)()

