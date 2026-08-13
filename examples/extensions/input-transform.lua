-- Translation of Pi v0.79.0 examples/extensions/input-transform.ts.
-- Demonstrates the `input` event for intercepting/transforming user input.
local pi = ...

pi.on("input", function(event, ctx)
  -- Source-based logic: skip processing for extension-injected messages
  if event.source == "extension" then
    return { action = "continue" }
  end

  -- Transform: ?quick prefix for brief responses
  if event.text:sub(1, 7) == "?quick " then
    local query = event.text:sub(8):gsub("^%s+", ""):gsub("%s+$", "")
    if query == "" then
      ctx.ui.notify("Usage: ?quick <question>", "warning")
      return { action = "handled" }
    end
    return { action = "transform", text = "Respond briefly in 1-2 sentences: " .. query }
  end

  -- Handle: instant responses without LLM
  if event.text:lower() == "ping" then
    ctx.ui.notify("pong", "info")
    return { action = "handled" }
  end
  if event.text:lower() == "time" then
    ctx.ui.notify(os.date("%c"), "info")
    return { action = "handled" }
  end

  return { action = "continue" }
end)