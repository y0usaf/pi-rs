-- Translation of Pi v0.79.0 examples/extensions/widget-placement.ts.
-- Demonstrates ctx.ui.setWidget() with above/below editor placement.
local pi = ...

pi.on("session_start", function(_event, ctx)
  if not ctx.hasUI then return end
  ctx.ui.setWidget("widget-above", { "Above editor widget" })
  ctx.ui.setWidget("widget-below", { "Below editor widget" }, { placement = "belowEditor" })
end)