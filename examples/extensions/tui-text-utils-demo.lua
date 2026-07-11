-- Public text-measurement/truncation mechanisms used by Lua-authored frontends.
local pi = ...
pi.register_command("tui-text-utils-demo", {
  description = "Exercise ANSI/Unicode visible width and truncation",
  handler = function()
    local colored = "\27[31m界abcdef\27[39m"
    return {
      width = pi.tui.visible_width(colored),
      clipped = pi.tui.truncate(colored, 6, "...", false),
      padded = pi.tui.truncate("pi", 5, "...", true),
    }
  end,
})
