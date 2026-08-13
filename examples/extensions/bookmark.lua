-- Translation of Pi v0.79.0 examples/extensions/bookmark.ts.
-- Shows setLabel to mark entries with labels for easy navigation in /tree.
-- Labels appear in the tree view and help you find important points.
local pi = ...

pi.register_command("bookmark", {
  description = "Bookmark last message (usage: /bookmark [label])",
  handler = function(args, ctx)
    local label = args:gsub("^%s+", ""):gsub("%s+$", "")
    if label == "" then label = "bookmark-" .. pi.now_ms() end

    -- Find the last assistant message entry
    local entries = ctx.sessionManager:get_entries()
    for i = #entries, 1, -1 do
      local entry = entries[i]
      if entry.type == "message" and entry.message.role == "assistant" then
        pi.setLabel(entry.id, label)
        ctx.ui.notify("Bookmarked as: " .. label, "info")
        return
      end
    end

    ctx.ui.notify("No assistant message to bookmark", "warning")
  end,
})

-- Remove bookmark
pi.register_command("unbookmark", {
  description = "Remove bookmark from last labeled entry",
  handler = function(_args, ctx)
    local entries = ctx.sessionManager:get_entries()
    for i = #entries, 1, -1 do
      local entry = entries[i]
      local label = ctx.sessionManager:get_label(entry.id)
      if label then
        pi.setLabel(entry.id, nil)
        ctx.ui.notify("Removed bookmark: " .. label, "info")
        return
      end
    end
    ctx.ui.notify("No bookmarked entry found", "warning")
  end,
})