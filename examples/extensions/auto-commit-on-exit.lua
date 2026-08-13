-- Translation of Pi v0.79.0 examples/extensions/auto-commit-on-exit.ts.
-- Automatically commits changes when the agent exits, using the last
-- assistant message to generate a commit message.
local pi = ...

pi.on("session_shutdown", function(_event, ctx)
  -- Check for uncommitted changes
  local status = pi.exec("git", { "status", "--porcelain" })

  if status.code ~= 0 or #(status.stdout:gsub("%s+", "")) == 0 then
    return -- Not a git repo or no changes
  end

  -- Find the last assistant message for commit context
  local entries = ctx.sessionManager:get_entries()
  local last_assistant_text = ""
  for i = #entries, 1, -1 do
    local entry = entries[i]
    if entry.type == "message" and entry.message.role == "assistant" then
      local content = entry.message.content
      if type(content) == "table" then
        local parts = {}
        for _, c in ipairs(content) do
          if c.type == "text" and c.text then parts[#parts + 1] = c.text end
        end
        last_assistant_text = table.concat(parts, "\n")
      end
      break
    end
  end

  local first_line = last_assistant_text:match("^[^\n]*") or "Work in progress"
  if first_line == "" then first_line = "Work in progress" end
  local commit_message = "[pi] " .. first_line:sub(1, 50) .. (first_line:len() > 50 and "..." or "")

  -- Stage and commit
  pi.exec("git", { "add", "-A" })
  local commit = pi.exec("git", { "commit", "-m", commit_message })

  if commit.code == 0 and ctx.hasUI then
    ctx.ui.notify("Auto-committed: " .. commit_message, "info")
  end
end)