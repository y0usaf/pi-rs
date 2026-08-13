-- Translation of Pi v0.79.0 examples/extensions/system-prompt-header.ts.
-- Displays a status widget showing the system prompt length.
local pi = ...

pi.on("agent_start", function(_event, ctx)
  local prompt = ctx.getSystemPrompt()
  ctx.ui.setStatus("system-prompt", "System: " .. #prompt .. " chars")
end)

pi.on("session_shutdown", function(_event, ctx)
  ctx.ui.setStatus("system-prompt", nil)
end)