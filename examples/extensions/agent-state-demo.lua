-- Exercises the embedded pi-rs-agent public Lua API (WS4.1).
local pi = ...

pi.register_command("agent-state-demo", {
  description = "Exercise agent defaults and state mutators",
  handler = function()
    local agent = pi.agent.new({ initialState = { systemPrompt = "demo" } })
    agent:set_thinking_level("low")
    agent:set_transport("websocket")
    agent:append_message({ role = "user", content = "hello", timestamp = 0 })
    local state = agent:get_state()
    return {
      systemPrompt = state.systemPrompt,
      thinkingLevel = state.thinkingLevel,
      messageCount = #state.messages,
      isStreaming = state.isStreaming,
      transport = agent:get_transport(),
    }
  end,
})
