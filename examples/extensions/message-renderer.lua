-- message-renderer: translation of the spec's message-renderer.ts —
-- register_message_renderer + send_message (PLAN 9.4). The renderer
-- snapshot is read back through pi.registered_message_renderers().
local pi = ...

pi.register_message_renderer("status-update", function(message, options, theme)
  local details = message.details or {}
  local level = details.level or "info"
  local text = "[" .. level:upper() .. "] " .. message.content
  if options.expanded and details.timestamp then
    text = text .. "\n  at " .. tostring(details.timestamp)
  end
  return { rendered = text, level = level }
end)

pi.register_command("status", {
  description = "Send a status message (usage: /status [warn|error] message)",
  handler = function(args)
    local parts = {}
    for part in args:gmatch("%S+") do parts[#parts + 1] = part end
    local level = "info"
    local content = args
    if parts[1] == "warn" or parts[1] == "error" then
      level = parts[1]
      content = #parts > 1 and table.concat(parts, " ", 2) or "Status update"
    end
    pi.send_message({
      customType = "status-update", content = content, display = true,
      details = { level = level, timestamp = 42 },
    })
    return { sent = { level = level, content = content } }
  end,
})

pi.register_command("message-renderer-probe", {
  description = "Read the registered renderers back and invoke the first",
  handler = function()
    local renderers = pi.registered_message_renderers()
    local rows = {}
    for i, entry in ipairs(renderers) do
      rows[i] = { customType = entry.customType, source = entry.source }
    end
    local invoked
    if renderers[1] then
      invoked = renderers[1].render(
        { customType = "status-update", content = "hello", display = true, details = {} },
        { expanded = true }, {})
    end
    return { renderers = rows, invoked = invoked }
  end,
})
