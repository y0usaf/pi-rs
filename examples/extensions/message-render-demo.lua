-- Exerciser for per-customType message renderers on the public surface.
--
-- In pi, extensions register a custom TUI renderer for messages they send
-- (core/extensions/types.ts MessageRenderer + loader.ts
-- `registerMessageRenderer(customType, renderer)`); the interactive
-- transcript's CustomMessageComponent prefers the matched renderer over its
-- generic box. pi-rs keeps the same contract on `pi.register_message_renderer`:
-- a renderer receives `(message, { expanded }, theme)` — message is an
-- immutable snapshot carrying customType/content/details/display — and returns
-- a component (`function(width) -> lines`) or a lines array, or nil to fall
-- through to the default box. First registration per customType wins; a
-- failing renderer falls through to the default box without masking the row.
local pi = ...

pi.register_message_renderer("render-probe", function(message, options, theme)
  local marker = options.expanded and " (expanded)" or ""
  local level = (message.details and message.details.level) or "info"
  local text = theme:fg(level == "error" and "error" or "success",
    "[" .. (message.customType or "custom") .. marker .. "] "
    .. tostring(message.content or ""))
  if options.expanded and message.details and message.details.timestamp then
    text = text .. theme:fg("dim", " @ " .. tostring(message.details.timestamp))
  end
  return function(width)
    return pi.tui.text_render(text, width, 0, 0)
  end
end)

-- A broken renderer for another customType must not affect render-probe;
-- its own errors fall through to the default custom-message box.
pi.register_message_renderer("broken-probe", function()
  error("renderer failed")
end)

pi.register_command("message-render-probe", {
  description = "Probe custom-message renderers on the file-backed surface",
  handler = function()
    return { probed = true }
  end,
})