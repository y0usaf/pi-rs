-- Deterministic TUI component/cell fixture exerciser (WS6.1).
local pi = ...
pi.register_command("tui-render-demo", {
    description = "Render a Text component through the public TUI seam",
    handler = function()
        local lines = pi.tui.text_render("hello\nworld", 10, 1, 0)
        local cells = pi.tui.differential_render({}, lines, true)
        return { lines = lines, cells = cells }
    end,
})
