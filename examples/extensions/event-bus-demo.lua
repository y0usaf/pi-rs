-- Exercises Pi's shared inter-extension event bus through the public Lua API.
-- Any extension can publish a channel; listeners receive the same data table.
local pi = ...

local received = {}
local unsubscribe = pi.events.on("demo:greeting", function(data)
  received[#received + 1] = (data.from or "unknown") .. ": " .. (data.message or "")
end)

pi.register_command("event-bus-demo", {
  description = "Emit a demo:greeting event",
  handler = function(args)
    received = {}
    pi.events.emit("demo:greeting", {
      from = "event-bus-demo",
      message = args ~= "" and args or "hello",
    })
    return { count = #received, messages = received }
  end,
})

pi.register_command("event-bus-unsubscribe", {
  description = "Unsubscribe the demo:greeting listener",
  handler = function()
    unsubscribe()
    unsubscribe() -- Unsubscription is idempotent.
  end,
})
