-- Translation of Pi v0.79.0 examples/extensions/event-bus.ts.
-- Inter-extension event bus: one extension emits events other extensions
-- listen to, via pi.events.
local pi = ...

-- Store ctx for use in event handler
local current_ctx = nil

pi.on("session_start", function(_event, ctx)
  current_ctx = ctx
end)

-- Listen for events from other extensions
pi.events.on("my:notification", function(data)
  local message, from = data.message, data.from
  if current_ctx then
    current_ctx.ui.notify("Event from " .. from .. ": " .. message, "info")
  end
end)

-- Command to emit events (emits "my:notification" which the listener receives)
pi.register_command("emit", {
  description = "Emit my:notification event (usage: /emit message)",
  handler = function(args, _ctx)
    local message = args:gsub("^%s+", ""):gsub("%s+$", "")
    if message == "" then message = "hello" end
    pi.events.emit("my:notification", { message = message, from = "/emit command" })
  end,
})

-- Example: emit on session start
pi.on("session_start", function()
  pi.events.emit("my:notification", {
    message = "Session started",
    from = "event-bus-example",
  })
end)