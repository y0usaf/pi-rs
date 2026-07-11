-- Exercises the deterministic streamed-turn public API (WS4.2).
local pi = ...

local MODEL = {
  id = "fixture", name = "fixture", api = "anthropic-messages", provider = "fixture",
  baseUrl = "", reasoning = true, input = { "text" },
  cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
  contextWindow = 1000, maxTokens = 100,
}

local function message(content, stop_reason)
  return {
    role = "assistant", content = content, api = MODEL.api, provider = MODEL.provider,
    model = MODEL.id, usage = { input = 0, output = 0, cacheRead = 0,
      cacheWrite = 0, totalTokens = 0,
      cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, total = 0 } },
    stopReason = stop_reason or "stop", timestamp = 0,
  }
end

pi.register_command("agent-turn-demo", {
  description = "Replay a text fixture through the Lua agent turn",
  handler = function()
    local events = {}
    local context = { systemPrompt = "demo", messages = {}, tools = {} }
    local final = message({ { type = "text", text = "hello" } })
    local function fixture(_model, request, options, on_event)
      local partial = message({})
      on_event({ type = "start", partial = partial })
      partial = message({ { type = "text", text = "hel" } })
      on_event({ type = "text_delta", contentIndex = 0, delta = "hel", partial = partial })
      partial = message({ { type = "text", text = "hello" } })
      on_event({ type = "text_end", contentIndex = 0, content = "hello", partial = partial })
      on_event({ type = "done", reason = "stop", message = final })
      return final
    end
    local result = pi.agent.run_turn(
      { { role = "user", content = "hi", timestamp = 0 } }, context,
      { model = MODEL, apiKey = "fallback" },
      function(event) events[#events + 1] = event.type end,
      pi.abort_signal(), fixture)
    return { events = events, text = result[2].content[1].text,
             transcriptText = context.messages[2].content[1].text }
  end,
})
