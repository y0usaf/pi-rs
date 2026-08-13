-- Translation of Pi v0.79.0 examples/extensions/status-line.ts.
-- Displays persistent status text in the footer using ctx.ui.setStatus()
-- with themed colors and turn progress.
local pi = ...

local turn_count = 0

pi.on("session_start", function(_event, ctx)
  local theme = ctx.ui.theme
  ctx.ui.setStatus("status-demo", theme:fg("dim", "Ready"))
end)

pi.on("turn_start", function(_event, ctx)
  turn_count = turn_count + 1
  local theme = ctx.ui.theme
  local spinner = theme:fg("accent", "●")
  local text = theme:fg("dim", " Turn " .. turn_count .. "...")
  ctx.ui.setStatus("status-demo", spinner .. text)
end)

pi.on("turn_end", function(_event, ctx)
  local theme = ctx.ui.theme
  local check = theme:fg("success", "✓")
  local text = theme:fg("dim", " Turn " .. turn_count .. " complete")
  ctx.ui.setStatus("status-demo", check .. text)
end)