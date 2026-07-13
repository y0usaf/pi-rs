-- Product-side extension composition. Rust exposes registration/handler
-- snapshots; this Lua policy chooses active tools and event fold semantics.
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

-- ExtensionRunner.emitToolCall: handlers run in extension/load order; the
-- latest non-nil result is retained and block short-circuits. Errors propagate
-- into the agent tool preflight, which settles them as failed tool results.
function EXTENSION_POLICY.emit_tool_call(core_event, context)
  local event = {
    type = "tool_call",
    toolCallId = core_event.toolCall.id,
    toolName = core_event.toolCall.name,
    input = core_event.args,
  }
  local result
  for _, entry in ipairs(EXTENSION_POLICY.api.extension_handlers("tool_call")) do
    local value = entry.handler(event, context)
    if value ~= nil then
      result = value
      if value.block then return value end
    end
  end
  return result
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
