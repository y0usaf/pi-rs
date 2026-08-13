-- Translation of Pi v0.79.0 examples/extensions/message-renderer.ts.
-- Registers a custom renderer for "status-update" messages and a /status
-- command that sends them with custom rendering.
local pi = ...

pi.register_message_renderer("status-update", function(message, options, theme)
  local details = message.details
  local level = (details and details.level) or "info"

  local color = level == "error" and "error" or (level == "warn" and "warning" or "success")
  local prefix = theme:fg(color, "[" .. level:upper() .. "]")
  local text = prefix .. " " .. tostring(message.content or "")

  -- Show timestamp when expanded
  if options.expanded and details and details.timestamp then
    text = text .. "\n" .. theme:fg("dim", "  at " .. tostring(details.timestamp))
  end

  return function(width)
    return pi.tui.text_render(text, width, 1, 1)
  end
end)

-- Command to send status messages
pi.register_command("status", {
  description = "Send a status message (usage: /status [warn|error] message)",
  handler = function(args, _ctx)
    local trimmed = args:gsub("^%s+", ""):gsub("%s+$", "")
    local level = "info"
    local content = trimmed

    -- Check for level prefix
    local first = trimmed:match("^(%S+)")
    if first == "warn" or first == "error" then
      level = first
      content = trimmed:sub(#first + 1):gsub("^%s+", "")
      if content == "" then content = "Status update" end
    end

    if content:gsub("%s+", "") == "" then content = "Status update" end

    pi.sendMessage({
      customType = "status-update",
      content = content,
      display = true,
      details = { level = level, timestamp = pi.now_ms() },
    })
  end,
})