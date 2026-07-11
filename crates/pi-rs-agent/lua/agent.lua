-- packages/agent/src/agent.ts and agent-loop.ts, translated to Lua.
-- Rust remains the provider/cancellation mechanism; streamed-turn policy lives here.
local pi = ...

local UNKNOWN_MODEL = {
  id = "unknown", name = "unknown", api = "unknown", provider = "unknown",
  baseUrl = "", reasoning = false, input = {},
  cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
  contextWindow = 0, maxTokens = 0,
}

local EMPTY_USAGE = {
  input = 0, output = 0, cacheRead = 0, cacheWrite = 0, totalTokens = 0,
  cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, total = 0 },
}

local function copy_array(values)
  local result = {}
  for i, value in ipairs(values or {}) do result[i] = value end
  return result
end

local function shallow_copy(value)
  local result = {}
  for key, item in pairs(value) do result[key] = item end
  return result
end

local function default_convert_to_llm(messages)
  local result = {}
  for _, message in ipairs(messages) do
    if message.role == "user" or message.role == "assistant" or message.role == "toolResult" then
      result[#result + 1] = message
    end
  end
  return result
end

-- Port of streamAssistantResponse. stream_fn follows the public
-- pi.ai.stream_simple callback shape: (model, context, options, on_event).
local function stream_assistant_response(context, config, signal, emit, stream_fn)
  local messages = context.messages
  if config.transformContext then
    messages = config.transformContext(messages, signal)
  end
  local convert = config.convertToLlm or default_convert_to_llm
  local llm_context = {
    systemPrompt = context.systemPrompt,
    messages = convert(messages),
    tools = context.tools,
  }

  local api_key = config.apiKey
  if config.getApiKey then
    api_key = config.getApiKey(config.model.provider) or api_key
  end
  local stream = stream_fn or config.streamFn or pi.ai.stream_simple
  local options = shallow_copy(config)
  options.apiKey = api_key
  options.signal = signal

  local partial = nil
  local added_partial = false
  local final_message = stream(config.model, llm_context, options, function(event)
    if event.type == "start" then
      partial = event.partial
      context.messages[#context.messages + 1] = partial
      added_partial = true
      emit({ type = "message_start", message = shallow_copy(partial) })
    elseif event.type ~= "done" and event.type ~= "error" and partial then
      partial = event.partial
      context.messages[#context.messages] = partial
      emit({ type = "message_update", assistantMessageEvent = event,
             message = shallow_copy(partial) })
    end
  end)

  if added_partial then
    context.messages[#context.messages] = final_message
  else
    context.messages[#context.messages + 1] = final_message
    emit({ type = "message_start", message = shallow_copy(final_message) })
  end
  emit({ type = "message_end", message = final_message })
  return final_message
end

local function error_text(value)
  local text = tostring(value)
  text = text:match("^(.-)\nstack traceback:") or text
  text = text:gsub("^runtime error: ", "")
  return text:match("^.-:%d+: (.*)$") or text
end

local function error_tool_result(message)
  return { content = { { type = "text", text = message } }, details = {} }
end

local function find_tool(tools, name)
  for _, tool in ipairs(tools or {}) do
    if tool.name == name then return tool end
  end
  return nil
end

local function prepare_tool_call(context, assistant_message, original_call, config, signal, emit)
  emit({ type = "tool_execution_start", toolCallId = original_call.id,
         toolName = original_call.name, args = original_call.arguments })
  local tool = find_tool(context.tools, original_call.name)
  if not tool then
    return { call = original_call, immediate = true,
      result = error_tool_result("Tool " .. original_call.name .. " not found"), isError = true }
  end
  local args = original_call.arguments
  local ok, prepared_or_error = pcall(function()
    if tool.prepare_arguments then args = tool.prepare_arguments(args) end
    args = pi.validate_tool_arguments(tool.name, tool.parameters or {}, args)
    if config.beforeToolCall then
      local before = config.beforeToolCall({ assistantMessage = assistant_message,
        toolCall = original_call, args = args, context = context }, signal)
      if signal and signal:is_aborted() then error("Operation aborted", 0) end
      if before and before.block then error(before.reason or "Tool execution was blocked", 0) end
    end
    if signal and signal:is_aborted() then error("Operation aborted", 0) end
    return args
  end)
  if not ok then
    return { call = original_call, immediate = true,
      result = error_tool_result(error_text(prepared_or_error)), isError = true }
  end
  return { call = original_call, tool = tool, args = prepared_or_error }
end

local function execute_prepared(context, assistant_message, prepared, config, signal, emit)
  if prepared.immediate then return prepared end
  local call, tool, args = prepared.call, prepared.tool, prepared.args
  local updates = function(partial)
    emit({ type = "tool_execution_update", toolCallId = call.id,
           toolName = call.name, args = call.arguments, partialResult = partial })
  end
  local executed, value = pcall(tool.execute, call.id, args, signal, updates,
    -- Spec ExtensionContext slice: `model` is the current agent model
    -- (runner.ts `get model()`); the full ctx bridge lands with item 9.
    { cwd = pi.cwd(), signal = signal, isIdle = false, model = config.model })
  local result, is_error
  if executed then result, is_error = value, false
  else result, is_error = error_tool_result(error_text(value)), true end
  if config.afterToolCall then
    local finalized, after = pcall(config.afterToolCall,
      { assistantMessage = assistant_message, toolCall = call, args = args,
        result = result, isError = is_error, context = context }, signal)
    if not finalized then result, is_error = error_tool_result(error_text(after)), true
    elseif after then
      result = { content = after.content or result.content,
                 details = after.details or result.details,
                 terminate = after.terminate == nil and result.terminate or after.terminate }
      if after.isError ~= nil then is_error = after.isError end
    end
  end
  return { call = call, result = result, isError = is_error }
end

local function emit_tool_end(outcome, emit)
  emit({ type = "tool_execution_end", toolCallId = outcome.call.id,
         toolName = outcome.call.name, result = outcome.result, isError = outcome.isError })
end

local function emit_tool_message(outcome, emit)
  local message = { role = "toolResult", toolCallId = outcome.call.id,
    toolName = outcome.call.name, content = outcome.result.content, details = outcome.result.details,
    isError = outcome.isError, timestamp = os.time() * 1000 }
  emit({ type = "message_start", message = message })
  emit({ type = "message_end", message = message })
  return message
end

local function should_terminate(outcomes)
  if #outcomes == 0 then return false end
  for _, outcome in ipairs(outcomes) do
    if outcome.result.terminate ~= true then return false end
  end
  return true
end

local function execute_tool_calls(context, assistant_message, tool_calls, config, signal, emit)
  local sequential = config.toolExecution == "sequential"
  for _, call in ipairs(tool_calls) do
    local tool = find_tool(context.tools, call.name)
    if tool and tool.executionMode == "sequential" then sequential = true end
  end
  if sequential then
    -- executeToolCallsSequential: each call is prepared, executed, ended, and
    -- its tool-result message emitted before the next call starts.
    local outcomes, messages = {}, {}
    for _, call in ipairs(tool_calls) do
      local outcome = execute_prepared(context, assistant_message,
        prepare_tool_call(context, assistant_message, call, config, signal, emit),
        config, signal, emit)
      emit_tool_end(outcome, emit)
      messages[#messages + 1] = emit_tool_message(outcome, emit)
      outcomes[#outcomes + 1] = outcome
      if signal and signal:is_aborted() then break end
    end
    return messages, should_terminate(outcomes)
  end
  -- executeToolCallsParallel: calls are prepared in source order (immediate
  -- outcomes end right away); prepared calls execute concurrently, each
  -- emitting tool_execution_end as it finalizes (completion order), while
  -- tool-result messages follow later in assistant source order.
  local entries, tasks = {}, {}
  for index, call in ipairs(tool_calls) do
    local prepared = prepare_tool_call(context, assistant_message, call, config, signal, emit)
    if prepared.immediate then
      emit_tool_end(prepared, emit)
      entries[index] = prepared
    else
      tasks[#tasks + 1] = function()
        local outcome = execute_prepared(context, assistant_message, prepared, config, signal, emit)
        emit_tool_end(outcome, emit)
        return { sourceIndex = index, outcome = outcome }
      end
    end
    if signal and signal:is_aborted() then break end
  end
  for _, completed in ipairs(pi.parallel(tasks)) do
    if not completed.ok then error(completed.error, 0) end
    entries[completed.value.sourceIndex] = completed.value.outcome
  end
  local messages, ordered = {}, {}
  for index = 1, #tool_calls do
    local outcome = entries[index]
    if outcome then
      ordered[#ordered + 1] = outcome
      messages[#messages + 1] = emit_tool_message(outcome, emit)
    end
  end
  return messages, should_terminate(ordered)
end


-- Port of agent-loop.ts runLoop: tool turns, steering, follow-ups and hooks.
local function run_turn(prompts, context, config, emit, signal, stream_fn)
  prompts = prompts or {}
  context.messages = copy_array(context.messages)
  context.tools = context.tools or pi.registered_tools()
  emit({ type = "agent_start" })
  emit({ type = "turn_start" })
  local result = copy_array(prompts)
  for _, prompt in ipairs(prompts) do
    context.messages[#context.messages + 1] = prompt
    emit({ type = "message_start", message = prompt })
    emit({ type = "message_end", message = prompt })
  end
  local first_turn = true
  local pending = config.getSteeringMessages and config.getSteeringMessages() or {}
  while true do
    local has_more_tools = true
    while has_more_tools or #pending > 0 do
      if not first_turn then emit({ type = "turn_start" }) else first_turn = false end
      for _, message in ipairs(pending) do
        emit({ type = "message_start", message = message })
        emit({ type = "message_end", message = message })
        context.messages[#context.messages + 1] = message
        result[#result + 1] = message
      end
      pending = {}
      local message = stream_assistant_response(context, config, signal, emit, stream_fn)
      result[#result + 1] = message
      if message.stopReason == "error" or message.stopReason == "aborted" then
        emit({ type = "turn_end", message = message, toolResults = {} })
        emit({ type = "agent_end", messages = result })
        return result
      end
      local calls = {}
      for _, content in ipairs(message.content or {}) do
        if content.type == "toolCall" then calls[#calls + 1] = content end
      end
      local tool_results, terminate = {}, false
      has_more_tools = false
      if #calls > 0 then
        tool_results, terminate = execute_tool_calls(context, message, calls, config, signal, emit)
        has_more_tools = not terminate
        for _, tool_result in ipairs(tool_results) do
          context.messages[#context.messages + 1] = tool_result
          result[#result + 1] = tool_result
        end
      end
      emit({ type = "turn_end", message = message, toolResults = tool_results })
      local turn = { message = message, toolResults = tool_results,
        context = context, newMessages = result }
      local snapshot = config.prepareNextTurn and config.prepareNextTurn(turn) or nil
      if snapshot then
        context = snapshot.context or context
        config.model = snapshot.model or config.model
        if snapshot.thinkingLevel ~= nil then
          if snapshot.thinkingLevel == "off" then
            config.reasoning = nil
          else
            config.reasoning = snapshot.thinkingLevel
          end
        end
      end
      turn.context = context
      if config.shouldStopAfterTurn and config.shouldStopAfterTurn(turn) then
        emit({ type = "agent_end", messages = result })
        return result
      end
      pending = config.getSteeringMessages and config.getSteeringMessages() or {}
    end
    local follow_up = config.getFollowUpMessages and config.getFollowUpMessages() or {}
    if #follow_up == 0 then break end
    pending = follow_up
  end
  emit({ type = "agent_end", messages = result })
  return result
end

local function new_agent(options)
  options = options or {}
  local initial = options.initialState or {}
  local listeners = {}
  local tools = copy_array(initial.tools)
  local messages = copy_array(initial.messages)
  local steering = {}
  local follow_up = {}
  local active_run = nil
  local steering_mode = options.steeringMode or "one-at-a-time"
  local follow_up_mode = options.followUpMode or "one-at-a-time"
  local state = {
    systemPrompt = initial.systemPrompt or "",
    model = initial.model or UNKNOWN_MODEL,
    thinkingLevel = initial.thinkingLevel or "off",
    isStreaming = false,
    pendingToolCalls = {},
  }

  local function notify(event)
    if event.type == "message_start" or event.type == "message_update" then
      state.streamingMessage = event.message
    elseif event.type == "message_end" then
      state.streamingMessage = nil
      messages[#messages + 1] = event.message
    elseif event.type == "tool_execution_start" then
      local pending = shallow_copy(state.pendingToolCalls)
      pending[event.toolCallId] = true
      state.pendingToolCalls = pending
    elseif event.type == "tool_execution_end" then
      local pending = shallow_copy(state.pendingToolCalls)
      pending[event.toolCallId] = nil
      state.pendingToolCalls = pending
    elseif event.type == "turn_end" then
      if event.message and event.message.role == "assistant" and event.message.errorMessage then
        state.errorMessage = event.message.errorMessage
      end
    elseif event.type == "agent_end" then
      state.streamingMessage = nil
    end
    if not active_run then error("Agent listener invoked outside active run", 0) end
    -- Snapshot insertion order. Unsubscribing during dispatch affects the next event.
    local snapshot = copy_array(listeners)
    for _, listener in ipairs(snapshot) do listener(event, active_run.signal) end
  end

  local function failure_message(err, aborted)
    return {
      role = "assistant", content = { { type = "text", text = "" } },
      api = state.model.api, provider = state.model.provider, model = state.model.id,
      usage = shallow_copy(EMPTY_USAGE), stopReason = aborted and "aborted" or "error",
      errorMessage = error_text(err), timestamp = os.time() * 1000,
    }
  end

  local function context_snapshot()
    return { systemPrompt = state.systemPrompt, messages = copy_array(messages), tools = copy_array(tools) }
  end

  local function drain(queue, mode)
    if #queue == 0 then return {} end
    if mode == "all" then
      local values = copy_array(queue)
      for index = #queue, 1, -1 do queue[index] = nil end
      return values
    end
    return { table.remove(queue, 1) }
  end

  local function loop_config(skip_initial_steering)
    local config = shallow_copy(options)
    config.model = state.model
    if state.thinkingLevel == "off" then
      config.reasoning = nil
    else
      config.reasoning = state.thinkingLevel
    end
    if options.prepareNextTurn then
      config.prepareNextTurn = function() return options.prepareNextTurn(active_run.signal) end
    end
    local skip = skip_initial_steering == true
    config.getSteeringMessages = function()
      if skip then skip = false; return {} end
      return drain(steering, steering_mode)
    end
    config.getFollowUpMessages = function() return drain(follow_up, follow_up_mode) end
    return config
  end

  local function run_with_lifecycle(prompts, skip_initial_steering)
    if active_run then error("Agent is already processing.", 0) end
    local signal = pi.abort_signal()
    active_run = { signal = signal }
    state.isStreaming = true
    state.streamingMessage = nil
    state.errorMessage = nil
    local ok, err = pcall(run_turn, prompts, context_snapshot(),
                          loop_config(skip_initial_steering), notify, signal, options.streamFn)
    if not ok then
      local failed = failure_message(err, signal:is_aborted())
      notify({ type = "message_start", message = failed })
      notify({ type = "message_end", message = failed })
      notify({ type = "turn_end", message = failed, toolResults = {} })
      notify({ type = "agent_end", messages = { failed } })
    end
    state.isStreaming = false
    state.streamingMessage = nil
    state.pendingToolCalls = {}
    active_run = nil
  end


  local agent = {}
  function agent:get_state()
    state.tools = tools
    state.messages = messages
    state.signal = active_run and active_run.signal or nil
    return state
  end
  function agent:set_system_prompt(value) state.systemPrompt = value end
  function agent:set_model(value) state.model = value end
  function agent:set_thinking_level(value) state.thinkingLevel = value end
  function agent:set_tools(value) tools = copy_array(value) end
  function agent:set_messages(value) messages = copy_array(value) end
  function agent:append_message(value) messages[#messages + 1] = value end
  function agent:clear_messages() messages = {} end
  function agent:subscribe(listener)
    for _, candidate in ipairs(listeners) do
      if candidate == listener then return function() end end
    end
    listeners[#listeners + 1] = listener
    local active = true
    return function()
      if not active then return end
      active = false
      for i, candidate in ipairs(listeners) do
        if candidate == listener then table.remove(listeners, i); break end
      end
    end
  end
  function agent:steer(message) steering[#steering + 1] = message end
  function agent:set_steering_mode(mode) steering_mode = mode end
  function agent:get_steering_mode() return steering_mode end
  function agent:set_follow_up_mode(mode) follow_up_mode = mode end
  function agent:get_follow_up_mode() return follow_up_mode end
  function agent:set_transport(transport) options.transport = transport end
  function agent:get_transport() return options.transport end
  function agent:follow_up(message) follow_up[#follow_up + 1] = message end
  function agent:has_queued_messages() return #steering > 0 or #follow_up > 0 end
  function agent:clear_steering_queue() steering = {} end
  function agent:clear_follow_up_queue() follow_up = {} end
  function agent:clear_all_queues() steering = {}; follow_up = {} end
  function agent:abort() if active_run then active_run.signal:abort() end end
  -- Lua calls already await coroutine completion, so this is the settled equivalent
  -- of waitForIdle after prompt/continue returns.
  function agent:wait_for_idle() return not active_run end
  function agent:reset()
    messages = {}
    state.isStreaming = false
    state.streamingMessage = nil
    state.pendingToolCalls = {}
    state.errorMessage = nil
    steering = {}
    follow_up = {}
  end
  function agent:prompt(input, images)
    if active_run then
      error("Agent is already processing a prompt. Use steer() or followUp() to queue messages, or wait for completion.", 0)
    end
    local prompts
    if type(input) == "string" then
      local content = { { type = "text", text = input } }
      for _, image in ipairs(images or {}) do content[#content + 1] = image end
      prompts = { { role = "user", content = content, timestamp = os.time() * 1000 } }
    elseif input and input.role then prompts = { input }
    else prompts = input or {} end
    run_with_lifecycle(prompts)
  end
  function agent:continue()
    if active_run then error("Agent is already processing. Wait for completion before continuing.", 0) end
    local last = messages[#messages]
    if not last then error("No messages to continue from", 0) end
    if last.role == "assistant" then
      local queued = drain(steering, steering_mode)
      if #queued > 0 then run_with_lifecycle(queued, true); return end
      queued = drain(follow_up, follow_up_mode)
      if #queued == 0 then error("Cannot continue from message role: assistant", 0) end
      run_with_lifecycle(queued)
      return
    end
    run_with_lifecycle({})
  end
  return agent
end

pi.agent = {
  new = new_agent,
  run_turn = run_turn,
  stream_assistant_response = stream_assistant_response,
}
