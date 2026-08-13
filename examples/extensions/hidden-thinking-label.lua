-- Translation of Pi v0.79.0 examples/extensions/hidden-thinking-label.ts.
-- Customizes the label shown when thinking blocks are hidden.
local pi = ...

local DEFAULT_LABEL = "Pondering..."

local label = DEFAULT_LABEL

local function apply_label(ctx)
  ctx.ui.setHiddenThinkingLabel(label)
end

pi.on("session_start", function(_event, ctx)
  apply_label(ctx)
end)

pi.register_command("thinking-label", {
  description = "Set the hidden thinking label. Use without args to reset.",
  handler = function(args, ctx)
    local next_label = args:gsub("^%s+", ""):gsub("%s+$", "")

    if next_label == "" then
      label = DEFAULT_LABEL
      ctx.ui.setHiddenThinkingLabel()
      ctx.ui.notify("Hidden thinking label reset to: " .. DEFAULT_LABEL, nil)
      return
    end

    label = next_label
    ctx.ui.setHiddenThinkingLabel(label)
    ctx.ui.notify("Hidden thinking label set to: " .. label, nil)
  end,
})