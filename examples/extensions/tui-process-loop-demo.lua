-- Exercises the terminal-independent process TUI lifecycle through the public Lua seam.
local pi = ...

pi.register_command("tui-process-loop-demo", {
  description = "Drive coalesced TUI input, rendering, resize, cursor, and teardown",
  handler = function()
    local tui = pi.tui.session(12, 4, true)
    tui:start()
    tui:request_render()
    local input = tui:feed("x")
    local coalesced = tui:render({ "heading", "ab_pi:ccd" })
    local idle = tui:render({ "ignored" })
    tui:resize(10, 3)
    local resized = tui:render({ "heading", "idle" })
    tui:stop()
    return {
      input = input,
      coalesced = coalesced,
      idle = idle,
      resized = resized,
      fullRedraws = tui:full_redraws(),
      output = tui:output(),
    }
  end,
})
