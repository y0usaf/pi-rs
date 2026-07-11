-- Exercises the WS6.4 select-list mechanism through the public Lua seam.
local pi = ...
pi.register_command("tui-select-list-demo", {
  description = "Navigate a TUI select list",
  handler = function()
    local list = pi.tui.select_list({
      { value = "alpha", label = "alpha", description = "first" },
      { value = "beta", label = "beta" },
    }, 1)
    list:input("\x1b[B")
    return { selected = list:selected(), lines = list:render(40) }
  end,
})
