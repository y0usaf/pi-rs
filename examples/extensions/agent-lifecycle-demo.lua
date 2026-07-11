-- Exercises the stateful Agent lifecycle public API (WS4.4).
local pi = ...

local MODEL = {
  id = "fixture", name = "fixture", api = "anthropic-messages", provider = "fixture",
  baseUrl = "", reasoning = false, input = { "text" },
  cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
  contextWindow = 1000, maxTokens = 100,
}

local function assistant(text)
  return {
    role = "assistant", content = { { type = "text", text = text } },
    api = MODEL.api, provider = MODEL.provider, model = MODEL.id,
    usage = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, totalTokens = 0,
      cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, total = 0 } },
    stopReason = "stop", timestamp = 0,
  }
end

pi.register_command("agent-lifecycle-demo", {
  description = "Run two prompts through a stateful, subscribed Agent",
  handler = function()
    local request_sizes, events = {}, {}
    local agent = pi.agent.new({
      initialState = { model = MODEL },
      streamFn = function(_model, context, _options, push)
        request_sizes[#request_sizes + 1] = #context.messages
        local final = assistant(#request_sizes == 1 and "hello Alice" or "I remember Alice")
        push({ type = "done", reason = "stop", message = final })
        return final
      end,
    })
    local active_at_end, idle_at_end
    agent:subscribe(function(event, signal)
      events[#events + 1] = event.type
      if event.type == "agent_end" then
        active_at_end = agent:get_state().isStreaming
        idle_at_end = agent:wait_for_idle()
      end
      assert(not signal:is_aborted())
    end)
    agent:prompt("My name is Alice")
    agent:prompt("What is my name?")
    local state = agent:get_state()
    return { messageCount = #state.messages, requestSizes = request_sizes, events = events,
      activeAtEnd = active_at_end, idleAtEnd = idle_at_end, idleNow = agent:wait_for_idle(),
      finalText = state.messages[#state.messages].content[1].text }
  end,
})
