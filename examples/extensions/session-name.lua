-- Translation of Pi v0.79.0 examples/extensions/session-name.ts.
-- Shows setSessionName/getSessionName to give sessions friendly names
-- that appear in the session selector instead of the first message.
local pi = ...

pi.register_command("session-name", {
  description = "Set or show session name (usage: /session-name [new name])",
  handler = function(args, ctx)
    local name = args:gsub("^%s+", ""):gsub("%s+$", "")

    if name ~= "" then
      pi.setSessionName(name)
      ctx.ui.notify("Session named: " .. name, "info")
    else
      local current = pi.getSessionName()
      ctx.ui.notify(current and ("Session: " .. current) or "No session name set", "info")
    end
  end,
})
