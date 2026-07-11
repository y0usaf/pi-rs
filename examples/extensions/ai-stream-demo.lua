-- Exercises pi.ai.stream_simple and the shared abort signal (WS4.1).
local pi = ...

pi.register_command("ai-stream-demo", {
  description = "Stream a provider response and count protocol events",
  handler = function(_args, ctx)
    local signal = (ctx and ctx.signal) or pi.abort_signal()
    local count = 0
    local model = {
      id = "demo", name = "demo", api = "missing-demo-api", provider = "demo",
      baseUrl = "", reasoning = false, input = { "text" },
      cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
      contextWindow = 1, maxTokens = 1,
    }
    local final = pi.ai.stream_simple(model, { messages = {} }, { signal = signal }, function(_event)
      count = count + 1
    end)
    return { events = count, stopReason = final.stopReason, hasError = final.errorMessage ~= nil,
             aborted = signal:is_aborted() }
  end,
})
