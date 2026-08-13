-- Translation of Pi v0.79.0 examples/extensions/mac-system-theme.ts.
-- Syncs pi theme with macOS system appearance (dark/light mode) by polling
-- the system appearance preferences every 2 seconds.
local pi = ...

local function is_dark_mode()
  local result = pi.exec("osascript", {
    "-e", 'tell application "System Events" to tell appearance preferences to return dark mode',
  })
  return result.stdout:gsub("%s+$", "") == "true"
end

local timer = nil

local function stop_polling()
  if timer then
    pi.clear_interval(timer)
    timer = nil
  end
end

pi.on("session_start", function(_event, ctx)
  local ok, current_theme = pcall(is_dark_mode)
  current_theme = (ok and current_theme) and "dark" or "light"
  ctx.ui.setTheme(current_theme)

  timer = pi.set_interval(2000, function()
    local next_ok, dark = pcall(is_dark_mode)
    local next_theme = (next_ok and dark) and "dark" or "light"
    if next_theme ~= current_theme then
      current_theme = next_theme
      ctx.ui.setTheme(current_theme)
    end
  end)
end)

pi.on("session_shutdown", function()
  stop_polling()
end)