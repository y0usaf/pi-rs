-- Doom-overlay mapping: temporary custom-component overlay composition.
-- Mirrors ref/pi doom-overlay/index.ts: ctx.ui.custom(factory, {
--   overlay = true, overlayOptions = { width, maxHeight, anchor, margin } })
-- with a component owning render/handle_input/dispose and an onHandle overlay
-- control handle (hide/show/focus/unfocus) that never touches frontend state.
local pi = ...

local function overlay_component(theme, done)
  local component = {
    wantsKeyRelease = true,
    render = function(_, width)
      local title = theme:fg("accent", "OVERLAY DEMO")
      local body = theme:fg("dim", "Enter to close · h hide · f focus")
      return pi.tui.text_render(title .. "\n" .. body, width, 1, 0)
    end,
    handle_input = function(_, data)
      if data == "\r" or data == "\n" then done(true) end
    end,
    dispose = function() end,
  }
  -- The doom overlay runs a render loop (setInterval); the translation uses a
  -- bounded spawned sleep so the composition is exercisable headlessly.
  pi.spawn(function() pi.sleep(1); done(true) end)
  return component
end

pi.register_command("ui-overlay-demo", {
  description = "Exercise temporary custom-component overlay composition (doom-overlay mapping)",
  handler = function(_, ctx)
    local handle = nil
    local closed = ctx.ui.custom(function(tui, theme, keybindings, done)
      return overlay_component(theme, done)
    end, {
      overlay = true,
      overlayOptions = { width = "75%", maxHeight = "95%", anchor = "center", margin = { top = 1 } },
      onHandle = function(value) handle = value end,
    })
    ctx.ui.notify(closed and "Overlay closed" or "Overlay dismissed", "info")
  end,
})
