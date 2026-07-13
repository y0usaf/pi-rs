-- Public event-pipeline exerciser: ordered input/context/provider/tool middleware
-- plus ordinary lifecycle observation. Load with: pi -e ./event-pipeline-demo.lua
local pi = ...
local seen = {}

local function note(event)
  seen[#seen + 1] = event.type
end

for _, event in ipairs({
  "session_start", "agent_start", "turn_start", "message_start", "message_end",
  "tool_execution_start", "tool_execution_end", "turn_end", "agent_end",
  "session_shutdown",
}) do
  pi.on(event, note)
end

pi.on("input", function(event)
  if event.text == "demo:hello" then
    return { action = "transform", text = "Say hello through the event pipeline" }
  end
  return { action = "continue" }
end)

pi.on("context", function(event)
  return { messages = event.messages }
end)

pi.on("before_provider_request", function(event)
  local payload = {}
  for key, value in pairs(event.payload) do payload[key] = value end
  payload.metadata = payload.metadata or {}
  payload.metadata.pi_rs_event_demo = true
  return payload
end)

pi.on("after_provider_response", function(event)
  seen[#seen + 1] = "response:" .. tostring(event.status)
end)

pi.on("tool_result", function(event)
  return { content = event.content, details = event.details, isError = event.isError }
end)

pi.register_command("event-pipeline-seen", {
  description = "Show event types observed by the event pipeline demo",
  handler = function(_, ctx)
    ctx.ui.notify(table.concat(seen, ", "), "info")
  end,
})
