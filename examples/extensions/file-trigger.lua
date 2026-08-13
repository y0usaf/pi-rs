-- Translation of Pi v0.79.0 examples/extensions/file-trigger.ts.
-- Watches a trigger file and injects its contents into the conversation.
-- Useful for external systems to send messages to the agent.
local pi = ...

pi.on("session_start", function(_event, ctx)
  local trigger_file = "/tmp/agent-trigger.txt"

  local watcher = pi.fs.watch_file(trigger_file, function()
    local ok, content = pcall(pi.fs.read_file, trigger_file)
    if ok then
      content = content:gsub("^%s+", ""):gsub("%s+$", "")
      if content ~= "" then
        pi.sendMessage(
          {
            customType = "file-trigger",
            content = "External trigger: " .. content,
            display = true,
          },
          { triggerTurn = true }
        )
        pi.fs.write_file(trigger_file, "") -- Clear after reading
      end
    end
  end)

  if ctx.hasUI then
    ctx.ui.notify("Watching " .. trigger_file, "info")
  end

  -- Keep the watcher alive for the session; it is disposed on reload/shutdown.
  pi.on("session_shutdown", function()
    if watcher and watcher.close then watcher:close() end
  end)
end)