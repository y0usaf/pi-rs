-- Translation of Pi v0.79.0 examples/extensions/git-checkpoint.ts.
-- Creates git stash checkpoints at each turn so /fork can restore code
-- state to that point in history.
local pi = ...

local checkpoints = {}
local current_entry_id = nil

-- Track the current entry ID when tool results are saved
pi.on("tool_result", function(_event, ctx)
  local leaf = ctx.sessionManager:get_leaf_entry()
  if leaf then current_entry_id = leaf.id end
end)

pi.on("turn_start", function()
  -- Create a git stash entry before the LLM makes changes
  local stash = pi.exec("git", { "stash", "create" })
  local ref = stash.stdout:gsub("%s+$", "")
  if ref ~= "" and current_entry_id then
    checkpoints[current_entry_id] = ref
  end
end)

pi.on("session_before_fork", function(event, ctx)
  local ref = checkpoints[event.entryId]
  if not ref then return end

  if not ctx.hasUI then
    return -- In non-interactive mode, don't restore automatically
  end

  local choice = ctx.ui.select("Restore code state?", {
    "Yes, restore code to that point",
    "No, keep current code",
  })

  if choice and choice:sub(1, 3) == "Yes" then
    pi.exec("git", { "stash", "apply", ref })
    ctx.ui.notify("Code restored to checkpoint", "info")
  end
end)

pi.on("agent_end", function()
  -- Clear checkpoints after agent completes
  checkpoints = {}
end)