-- Translation of Pi v0.79.0 examples/extensions/dirty-repo-guard.ts.
-- Prevents session changes when there are uncommitted git changes.
local pi = ...

local function check_dirty_repo(ctx, action)
  local result = pi.exec("git", { "status", "--porcelain" })
  if result.code ~= 0 then
    return nil -- Not a git repo, allow the action
  end

  local has_changes = #(result.stdout:gsub("^%s+", ""):gsub("%s+$", "")) > 0
  if not has_changes then
    return nil
  end

  if not ctx.hasUI then
    return { cancel = true }
  end

  local changed_files = 0
  for line in result.stdout:gmatch("[^\n]+") do
    if line:gsub("%s+", "") ~= "" then changed_files = changed_files + 1 end
  end

  local choice = ctx.ui.select("You have " .. changed_files .. " uncommitted file(s). " .. action .. " anyway?", {
    "Yes, proceed anyway",
    "No, let me commit first",
  })

  if choice ~= "Yes, proceed anyway" then
    ctx.ui.notify("Commit your changes first", "warning")
    return { cancel = true }
  end
end

pi.on("session_before_switch", function(event, ctx)
  local action = event.reason == "new" and "new session" or "switch session"
  return check_dirty_repo(ctx, action)
end)

pi.on("session_before_fork", function(_event, ctx)
  return check_dirty_repo(ctx, "fork")
end)