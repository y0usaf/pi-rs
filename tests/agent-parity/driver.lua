-- Differential parity driver (tests/agent-parity): replays the oracle
-- cases through the public agent surface (pi.agent.new) with scripted
-- streams, scripted tools, scripted hooks, and event-count triggers that
-- mirror gen-oracle.ts 1:1. Loaded by crates/pi-rs-agent/tests/agent_parity.rs
-- like a user extension; nothing here reaches host internals.
local pi = ...

local EMPTY_USAGE = {
  input = 0, output = 0, cacheRead = 0, cacheWrite = 0, totalTokens = 0,
  cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, total = 0 },
}

local function deep_copy(value)
  if type(value) ~= "table" then return value end
  local out = {}
  for key, item in pairs(value) do out[key] = deep_copy(item) end
  return out
end

local function error_text(value)
  local text = tostring(value)
  text = text:match("^(.-)\nstack traceback:") or text
  text = text:gsub("^runtime error: ", "")
  return text:match("^.-:%d+: (.*)$") or text
end

local function base_message(model, content, stop_reason)
  return {
    role = "assistant", content = content, api = model.api,
    provider = model.provider, model = model.id, usage = deep_copy(EMPTY_USAGE),
    stopReason = stop_reason, timestamp = 0,
  }
end

-- Mirror of gen-oracle.ts synthesize(): the scripted stream event list for a
-- turn spec. Recorded message_update events pin any drift between the two.
local function synthesize(turn, model)
  local blocks = turn.blocks or {}
  local function snapshot(count, current)
    local content = {}
    for i = 1, count do content[i] = deep_copy(blocks[i]) end
    if current ~= nil then content[count + 1] = current end
    return content
  end
  local events = { { type = "start", partial = base_message(model, {}, "stop") } }
  for index, block in ipairs(blocks) do
    local ci = index - 1
    if block.type == "text" then
      events[#events + 1] = { type = "text_start", contentIndex = ci,
        partial = base_message(model, snapshot(index - 1, { type = "text", text = "" }), "stop") }
      events[#events + 1] = { type = "text_delta", contentIndex = ci, delta = block.text,
        partial = base_message(model, snapshot(index), "stop") }
      events[#events + 1] = { type = "text_end", contentIndex = ci, content = block.text,
        partial = base_message(model, snapshot(index), "stop") }
    elseif block.type == "thinking" then
      events[#events + 1] = { type = "thinking_start", contentIndex = ci,
        partial = base_message(model, snapshot(index - 1, { type = "thinking", thinking = "" }), "stop") }
      events[#events + 1] = { type = "thinking_delta", contentIndex = ci, delta = block.thinking,
        partial = base_message(model, snapshot(index), "stop") }
      events[#events + 1] = { type = "thinking_end", contentIndex = ci, content = block.thinking,
        partial = base_message(model, snapshot(index), "stop") }
    elseif block.type == "toolCall" then
      events[#events + 1] = { type = "toolcall_start", contentIndex = ci,
        partial = base_message(model, snapshot(index - 1,
          { type = "toolCall", id = block.id, name = block.name, arguments = {} }), "stop") }
      events[#events + 1] = { type = "toolcall_delta", contentIndex = ci,
        delta = pi.json.encode(block.arguments),
        partial = base_message(model, snapshot(index), "stop") }
      events[#events + 1] = { type = "toolcall_end", contentIndex = ci, toolCall = deep_copy(block),
        partial = base_message(model, snapshot(index), "stop") }
    else
      error("unknown block type " .. tostring(block.type), 0)
    end
  end
  local final = base_message(model, snapshot(#blocks), turn.stopReason or "stop")
  if turn.errorMessage ~= nil then final.errorMessage = turn.errorMessage end
  local terminal
  if turn.stopReason == "error" or turn.stopReason == "aborted" then
    terminal = { type = "error", reason = turn.stopReason, error = final }
  else
    terminal = { type = "done", reason = turn.stopReason or "stop", message = final }
  end
  events[#events + 1] = terminal
  return events, final
end

local function make_stream_fn(case, recorder)
  local turn_index = 0
  return function(model, context, options, push)
    turn_index = turn_index + 1
    local turn = case.turns[math.min(turn_index, #case.turns)]
    recorder.requests[#recorder.requests + 1] = deep_copy({
      model = model.id,
      reasoning = options.reasoning or "none",
      systemPrompt = context.systemPrompt or "",
      messages = context.messages,
    })
    if turn["throw"] then error(turn["throw"], 0) end
    local events, final = synthesize(turn, model)
    local last_content = {}
    for _, event in ipairs(events) do
      local signal = options.signal
      if signal and signal:is_aborted() then
        local aborted = base_message(model, last_content, "aborted")
        aborted.errorMessage = "Request was aborted"
        push({ type = "error", reason = "aborted", error = aborted })
        return aborted
      end
      push(event)
      local partial = event.partial or event.message or event.error
      if partial and partial.content then last_content = deep_copy(partial.content) end
    end
    return final
  end
end

local function build_tool(spec)
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
        if on_update then on_update(deep_copy(update.partial)) end
      end
      if inv.sleepMs then pi.sleep(inv.sleepMs) end
      check()
      if inv["throw"] then error(inv["throw"], 0) end
      return deep_copy(inv.result or
        { content = { { type = "text", text = spec.name .. " ok" } }, details = {} })
    end,
  }
end

local function scripted_hook(scripts, apply)
  if not scripts then return nil end
  local index = 0
  return function()
    index = index + 1
    local entry = scripts[math.min(index, #scripts)]
    if not entry or entry.skip then return nil end
    if entry["throw"] then error(entry["throw"], 0) end
    return apply(entry)
  end
end

local function run_case(case, models)
  local options = case.options or {}
  local model = models[options.model or "default"]
  local recorder = { events = {}, requests = {} }
  local hooks = case.hooks or {}
  local tools = {}
  for _, spec in ipairs(case.tools or {}) do tools[#tools + 1] = build_tool(spec) end
  local agent = pi.agent.new({
    initialState = {
      systemPrompt = options.systemPrompt or "",
      model = model,
      thinkingLevel = options.thinkingLevel,
      tools = tools,
      messages = deep_copy(options.initialMessages or {}),
    },
    streamFn = make_stream_fn(case, recorder),
    toolExecution = options.toolExecution,
    steeringMode = options.steeringMode,
    followUpMode = options.followUpMode,
    beforeToolCall = scripted_hook(hooks.beforeToolCall, function(entry)
      return { block = entry.block, reason = entry.reason }
    end),
    afterToolCall = scripted_hook(hooks.afterToolCall, function(entry)
      return { content = entry.content, details = entry.details,
               isError = entry.isError, terminate = entry.terminate }
    end),
    prepareNextTurn = scripted_hook(hooks.prepareNextTurn, function(entry)
      return { model = entry.model and models[entry.model] or nil,
               thinkingLevel = entry.thinkingLevel }
    end),
  })

  local counts = {}
  local fired = {}
  agent:subscribe(function(event)
    recorder.events[#recorder.events + 1] = deep_copy(event)
    counts[event.type] = (counts[event.type] or 0) + 1
    for index, trigger in ipairs(case.triggers or {}) do
      if not fired[index] and trigger.on.event == event.type
        and counts[event.type] == trigger.on.count then
        fired[index] = true
        if trigger.action == "steer" then agent:steer(deep_copy(trigger.message))
        elseif trigger.action == "followUp" then agent:follow_up(deep_copy(trigger.message))
        elseif trigger.action == "abort" then agent:abort()
        else error("unknown trigger action " .. tostring(trigger.action), 0) end
      end
    end
  end)

  local phases = {}
  for _, phase in ipairs(case.phases) do
    for _, message in ipairs(phase.steer or {}) do agent:steer(deep_copy(message)) end
    for _, message in ipairs(phase.followUp or {}) do agent:follow_up(deep_copy(message)) end
    local ok, err
    if phase["continue"] then
      ok, err = pcall(function() agent:continue() end)
    else
      ok, err = pcall(function() agent:prompt(deep_copy(phase.prompt)) end)
    end
    if ok then phases[#phases + 1] = { ok = true }
    else phases[#phases + 1] = { ok = false, error = error_text(err) } end
  end

  local final_state = agent:get_state()
  local state = { messages = deep_copy(final_state.messages) }
  if final_state.errorMessage ~= nil then state.errorMessage = final_state.errorMessage end
  return {
    name = case.name,
    events = recorder.events,
    requests = recorder.requests,
    phases = phases,
    state = state,
  }
end

pi.register_command("agent-parity", { handler = function(args)
  local payload = pi.json.decode(args)
  return run_case(payload.case, payload.models)
end })
