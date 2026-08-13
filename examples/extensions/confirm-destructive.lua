-- Translation of Pi v0.79.0 examples/extensions/confirm-destructive.ts.
-- Prompts for confirmation before destructive session actions (clear,
-- switch, branch) using the session_before_* events.
local pi = ...

pi.on("session_before_switch", function(event, ctx)
  if not ctx.hasUI then return end

  if event.reason == "new" then
    local confirmed = ctx.ui.confirm("Clear session?", "This will delete all messages in the current session.")

    if not confirmed then
      ctx.ui.notify("Clear cancelled", "info")
      return { cancel = true }
    end
    return
  end

  -- reason === "resume" - check for unsaved changes (messages since last
  -- assistant response).
  local entries = ctx.sessionManager:get_entries()
  local has_unsaved_work = false
  for _, e in ipairs(entries) do
    if e.type == "message" and e.message.role == "user" then has_unsaved_work = true break end
  end

  if has_unsaved_work then
    local confirmed = ctx.ui.confirm("Switch session?", "You have messages in the current session. Switch anyway?")

    if not confirmed then
      ctx.ui.notify("Switch cancelled", "info")
      return { cancel = true }
    end
  end
end)

pi.on("session_before_fork", function(event, ctx)
  if not ctx.hasUI then return end

  local choice = ctx.ui.select("Fork from entry " .. event.entryId:sub(1, 8) .. "?", {
    "Yes, create fork",
    "No, stay in current session",
  })

  if choice ~= "Yes, create fork" then
    ctx.ui.notify("Fork cancelled", "info")
    return { cancel = true }
  end
end)