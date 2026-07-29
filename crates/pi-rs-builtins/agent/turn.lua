-- The shipped agent reducer: one coding turn over the public model, effect,
-- and action surface.
--
-- Everything here is product policy. Rust supplies the bounded stream
-- crossing, cancellation signals, and the action queue; the turn shape
-- (retry, tool settlement, steering, follow-up, cancellation, diagnostics)
-- is Lua and may be replaced wholesale by registering another agent root.

local pi = ...
local module = pi.kernel.v1.module
local models = pi.models.v1
local effects = pi.effects.v1
local roots = pi.roots.v1

module.define({
  name = "pi.agent.turn",
  version = "1",
  dependencies = {
    queues = { name = "pi.agent.queue", version = "1" },
    tools = { name = "pi.agent.tools", version = "1" },
  },
  factory = function(deps)
    local queues = deps.queues
    local tools = deps.tools

    local DEFAULT_LIMITS = {
      max_retries = 2,
      max_tool_iterations = 8,
      max_follow_ups = 4,
      max_requests = 16,
      max_events = 256,
      queue_limit = 64,
    }

    -- Snapshots are read-only views; copy anything kept across dispatches.
    local function clone(value)
      if type(value) ~= "table" then
        return value
      end
      local copy = {}
      for key, item in pairs(value) do
        copy[key] = clone(item)
      end
      return copy
    end

    local function user_message(text)
      return { role = "user", content = text, timestamp = 0 }
    end

    local function tool_result_message(call, output, is_error)
      return {
        role = "toolResult",
        toolCallId = call.id,
        toolName = call.name,
        content = { { type = "text", text = output } },
        isError = is_error and true or false,
        timestamp = 0,
      }
    end

    local function message_text(message)
      if type(message) ~= "table" or type(message.content) ~= "table" then
        return ""
      end
      local parts = {}
      for _, block in ipairs(message.content) do
        if type(block) == "table" and block.type == "text" and type(block.text) == "string" then
          parts[#parts + 1] = block.text
        end
      end
      return table.concat(parts)
    end

    -- Tool calls are read from the settled message: a malformed or partial
    -- stream cannot desynchronise settlement from the conversation.
    local function message_tool_calls(message)
      local calls = {}
      if type(message) ~= "table" or type(message.content) ~= "table" then
        return calls
      end
      for _, block in ipairs(message.content) do
        if type(block) == "table" and block.type == "toolCall" and type(block.id) == "string" then
          calls[#calls + 1] = {
            id = block.id,
            name = type(block.name) == "string" and block.name or "",
            arguments = type(block.arguments) == "table" and block.arguments or {},
          }
        end
      end
      return calls
    end

    -- The one call a streaming tool-call event is about, read straight out of
    -- the partial message the host already built. `contentIndex` is a 0-based
    -- index into that message's content, so parallel calls stay distinct
    -- without the agent keeping a scratch copy of the stream.
    local function streaming_call(event)
      local partial = event.partial
      if type(partial) ~= "table" or type(partial.content) ~= "table" then
        return nil
      end
      local index = event.contentIndex
      if type(index) ~= "number" then
        return nil
      end
      local block = partial.content[math.floor(index) + 1]
      if type(block) ~= "table" or block.type ~= "toolCall" or type(block.id) ~= "string" then
        return nil
      end
      return {
        id = block.id,
        name = type(block.name) == "string" and block.name or "",
        arguments = type(block.arguments) == "table" and block.arguments or {},
      }
    end

    -- Consecutive parallel-eligible calls settle as one bounded group; a
    -- serializing tool (or an unknown name) always settles alone, in call
    -- order, so file mutations cannot interleave.
    local function group_calls(calls)
      local groups = {}
      local current = nil
      for _, call in ipairs(calls) do
        local entry = tools.find(call.name)
        local serial = (entry == nil) or entry.serialize
        if serial or current == nil then
          current = { index = #groups + 1, mode = serial and "serial" or "parallel", calls = { call } }
          groups[#groups + 1] = current
          if serial then
            current = nil
          end
        else
          current.calls[#current.calls + 1] = call
        end
      end
      return groups
    end

    local Agent = {}
    Agent.__index = Agent

    function Agent.new(config)
      config = config or {}
      local limits = {}
      for key, value in pairs(DEFAULT_LIMITS) do
        limits[key] = value
      end
      for key, value in pairs(config.limits or {}) do
        if DEFAULT_LIMITS[key] ~= nil and tonumber(value) then
          limits[key] = tonumber(value)
        end
      end
      return setmetatable({
        conversation = {},
        model = clone(config.model),
        options = clone(config.options) or {},
        system_prompt = config.system_prompt,
        limits = limits,
        steering = queues.new(limits.queue_limit),
        follow_ups = queues.new(limits.queue_limit),
        interrupts = queues.new(limits.queue_limit),
        active_signal = nil,
      }, Agent)
    end

    function Agent:configure(settings)
      if type(settings) ~= "table" then
        return
      end
      if settings.model ~= nil then
        self.model = clone(settings.model)
      end
      if settings.options ~= nil then
        self.options = clone(settings.options)
      end
      if settings.system_prompt ~= nil then
        self.system_prompt = settings.system_prompt
      end
      for key, value in pairs(settings.limits or {}) do
        if DEFAULT_LIMITS[key] ~= nil and tonumber(value) then
          self.limits[key] = tonumber(value)
        end
      end
    end

    -- Cancellation has two sources: a queued product interrupt and the
    -- kernel's own dispatch cancellation.
    function Agent:should_cancel()
      if self.interrupts:len() > 0 then
        return true
      end
      local ok, handle = pcall(roots.cancellation)
      if ok and handle then
        local known, cancelled = pcall(handle.is_cancelled, handle)
        if known and cancelled then
          return true
        end
      end
      return false
    end

    function Agent:context()
      local messages = {}
      for index, message in ipairs(self.conversation) do
        messages[index] = message
      end
      local context = { messages = messages }
      if self.system_prompt ~= nil then
        context.systemPrompt = self.system_prompt
      end
      local declared = tools.declarations()
      if #declared > 0 then
        context.tools = declared
      end
      return context
    end

    function Agent:stream_options(signal)
      local options = {}
      for key, value in pairs(self.options) do
        if type(value) ~= "table" and type(value) ~= "function" then
          options[key] = value
        end
      end
      options.signal = signal
      options.max_events = self.limits.max_events
      return options
    end

    function Agent:drain_steering(emit)
      for _, text in ipairs(self.steering:drain()) do
        self.conversation[#self.conversation + 1] = user_message(text)
        emit("agent_steered", { text = text })
      end
    end

    -- One provider request. Deltas render incrementally; a queued interrupt
    -- aborts the signal and stops the crossing at the next event.
    function Agent:stream_once(emit)
      if type(self.model) ~= "table" then
        return { state = "error", retryable = false, reason = "missing_model", tool_calls = {} }
      end

      emit("agent_status", { state = "streaming", messages = #self.conversation })

      local signal = effects.cancellation.new()
      self.active_signal = signal
      local cancelled = false
      local streamed = ""

      local function on_event(event)
        if self:should_cancel() then
          cancelled = true
          signal:abort()
          error("pi.agent: cancelled", 0)
        end
        if type(event) ~= "table" then
          return
        end
        if event.type == "text_delta" then
          local delta = event.delta
          if type(delta) == "string" and #delta > 0 then
            streamed = streamed .. delta
            emit("agent_text_delta", { text = delta })
          end
        elseif event.type == "thinking_delta" then
          -- Reasoning is streamed and named separately from the answer: it
          -- is not part of `streamed`, which is the partial reply a
          -- cancellation reports.
          local delta = event.delta
          if type(delta) == "string" and #delta > 0 then
            emit("agent_thinking_delta", { text = delta })
          end
        elseif event.type == "thinking_end" then
          local content = event.content
          if type(content) == "string" and #content > 0 then
            emit("agent_thinking", { text = content })
          end
        elseif
          event.type == "toolcall_start"
          or event.type == "toolcall_delta"
          or event.type == "toolcall_end"
        then
          -- A provider names a call and then streams its arguments. Saying so
          -- while they arrive is what lets a frontend show the call before it
          -- is runnable. Settlement still reads the finished message, so a
          -- truncated stream can announce a call but never start one.
          local call = streaming_call(event)
          if call ~= nil then
            emit("agent_tool_delta", call)
          end
        elseif event.type == "error" then
          emit("agent_diagnostic", { reason = "stream_error_event" })
        end
      end

      local model = self.model
      local context = self:context()
      local options = self:stream_options(signal)
      local ok, result = pcall(function()
        return models.stream(model, context, options, on_event)
      end)
      self.active_signal = nil

      if cancelled then
        emit("agent_cancelled", { reason = "interrupt", partial = streamed })
        return { state = "cancelled", tool_calls = {} }
      end
      if not ok then
        return {
          state = "error",
          retryable = true,
          reason = tostring(result),
          tool_calls = {},
        }
      end

      local message = result
      if type(message) ~= "table" then
        return { state = "error", retryable = true, reason = "empty provider result", tool_calls = {} }
      end
      local stop = message.stopReason
      if stop == "aborted" then
        emit("agent_cancelled", { reason = "provider_aborted", partial = streamed })
        return { state = "cancelled", tool_calls = {} }
      end
      if stop == "error" then
        return {
          state = "error",
          retryable = true,
          reason = tostring(message.errorMessage or "provider error"),
          tool_calls = {},
        }
      end

      self.conversation[#self.conversation + 1] = message
      local calls = message_tool_calls(message)
      emit("agent_message", {
        text = message_text(message),
        streamed = streamed,
        stop_reason = tostring(stop),
        tool_calls = #calls,
      })
      if stop == "toolUse" and #calls == 0 then
        emit("agent_diagnostic", { reason = "tool_use_without_calls" })
      end
      return { state = "ok", stop_reason = stop, tool_calls = calls }
    end

    -- Bounded retry around one request; only transport/provider failures are
    -- retried, and the bound is a declared limit, never an open loop.
    function Agent:request(emit)
      local attempt = 0
      while true do
        local result = self:stream_once(emit)
        if result.state == "error" and result.retryable and attempt < self.limits.max_retries then
          attempt = attempt + 1
          emit("agent_retry", { attempt = attempt, reason = result.reason })
        else
          if result.state == "error" then
            emit("agent_error", { reason = result.reason, attempts = attempt + 1 })
          end
          return result
        end
      end
    end

    function Agent:settle_tools(calls, emit)
      for _, group in ipairs(group_calls(calls)) do
        emit("agent_tool_group", { index = group.index, mode = group.mode, calls = #group.calls })
        for _, call in ipairs(group.calls) do
          if self:should_cancel() then
            emit("agent_cancelled", { reason = "tool_settlement", partial = "" })
            return true
          end
          emit("agent_tool_start", {
            id = call.id,
            name = call.name,
            -- The call's own arguments travel with the start action so a
            -- frontend can say what is running without asking the agent.
            arguments = call.arguments,
            group = group.index,
            mode = group.mode,
          })
          local entry = tools.find(call.name)
          local ok, result
          if entry == nil then
            ok, result = false, "unknown tool: " .. tostring(call.name)
          else
            ok, result = pcall(entry.execute, {
              id = call.id,
              name = call.name,
              arguments = call.arguments,
            })
          end
          local output, failed
          if not ok then
            output, failed = tostring(result), true
          elseif type(result) == "table" then
            output, failed = tostring(result.output or ""), result.is_error == true
          else
            output, failed = tostring(result), false
          end
          self.conversation[#self.conversation + 1] = tool_result_message(call, output, failed)
          emit("agent_tool_result", {
            id = call.id,
            name = call.name,
            group = group.index,
            mode = group.mode,
            ok = not failed,
            output = output,
          })
        end
      end
      return false
    end

    -- One turn: requests until the model stops asking for tools and no
    -- steering remains, bounded by declared request/tool limits.
    function Agent:run_turn(text, emit)
      self.conversation[#self.conversation + 1] = user_message(text)
      emit("agent_turn_start", { prompt = text })
      local requests = 0
      local tool_iterations = 0

      while true do
        requests = requests + 1
        if requests > self.limits.max_requests then
          emit("agent_error", { reason = "request_limit", limit = self.limits.max_requests })
          return "error"
        end
        self:drain_steering(emit)

        local result = self:request(emit)
        if result.state == "cancelled" then
          return "cancelled"
        end
        if result.state == "error" then
          return "error"
        end

        if #result.tool_calls > 0 then
          if tool_iterations >= self.limits.max_tool_iterations then
            emit("agent_error", {
              reason = "tool_iteration_limit",
              limit = self.limits.max_tool_iterations,
            })
            return "error"
          end
          tool_iterations = tool_iterations + 1
          if self:settle_tools(result.tool_calls, emit) then
            return "cancelled"
          end
        elseif self.steering:len() == 0 then
          emit("agent_status", { state = "idle", messages = #self.conversation })
          return "complete"
        end
      end
    end

    function Agent:handle(event, emit)
      local kind = type(event) == "table" and event.kind or nil

      if kind == "configure" then
        self:configure(event)
        emit("agent_configured", {
          model = type(self.model) == "table" and tostring(self.model.id) or nil,
          tools = #tools.declarations(),
        })
        return
      end

      if kind == "prompt" then
        if event.model ~= nil then
          self.model = clone(event.model)
        end
        if event.options ~= nil then
          self.options = clone(event.options)
        end
        local outcome = self:run_turn(tostring(event.text or ""), emit)
        local drained = 0
        while outcome ~= "cancelled" and self.follow_ups:len() > 0 do
          if drained >= self.limits.max_follow_ups then
            emit("agent_diagnostic", {
              reason = "follow_up_limit",
              pending = self.follow_ups:len(),
            })
            break
          end
          drained = drained + 1
          local next_prompt = self.follow_ups:take()
          emit("agent_follow_up", { text = next_prompt, remaining = self.follow_ups:len() })
          outcome = self:run_turn(next_prompt, emit)
        end
        self.interrupts:clear()
        return
      end

      if kind == "steer" or kind == "follow_up" then
        local queue = kind == "steer" and self.steering or self.follow_ups
        local text = tostring(event.text or "")
        local accepted, reason = queue:push(text)
        emit("agent_queued", {
          queue = kind,
          -- The text travels with the acceptance so a frontend can show what
          -- is pending without asking the agent for its queue.
          text = text,
          accepted = accepted and true or false,
          reason = accepted and nil or tostring(reason),
          depth = queue:len(),
        })
        return
      end

      if kind == "interrupt" then
        self.interrupts:push(true)
        if self.active_signal ~= nil then
          self.active_signal:abort()
        end
        emit("agent_queued", { queue = "interrupt", accepted = true, depth = self.interrupts:len() })
        return
      end

      if kind == "status" then
        emit("agent_status", {
          state = "idle",
          messages = #self.conversation,
          steering = self.steering:len(),
          follow_ups = self.follow_ups:len(),
          interrupts = self.interrupts:len(),
          tools = #tools.declarations(),
        })
        return
      end

      if kind == "reset" then
        self.conversation = {}
        self.steering:clear()
        self.follow_ups:clear()
        self.interrupts:clear()
        emit("agent_reset", {})
        return
      end

      emit("agent_diagnostic", { reason = "unknown_event", kind = tostring(kind) })
    end

    return {
      new = Agent.new,
      defaults = DEFAULT_LIMITS,
    }
  end,
})
