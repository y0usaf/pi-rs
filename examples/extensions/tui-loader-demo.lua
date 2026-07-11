-- Exercises the WS6.7 loader and cancellation mechanisms through the public Lua seam.
local pi = ...
pi.register_command("tui-loader-demo", {
  description = "Render and advance a cancellable loader",
  handler = function()
    local loader = pi.tui.loader("Working...", { frames = { "a", "b" }, interval_ms = 20 })
    local before = loader:render(16)
    loader:advance(20)
    local after = loader:render(16)
    local cancellable = pi.tui.cancellable_loader("Cancel me")
    cancellable:input("\x1b")
    cancellable:dispose()
    return { before = before, after = after, aborted = cancellable:aborted(), running = loader:running() }
  end,
})
