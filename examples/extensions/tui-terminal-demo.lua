-- Exercises deterministic terminal state and output bytes through the public Lua seam.
local pi = ...

pi.register_command("tui-terminal-demo", {
  description = "Drive terminal protocol state without process I/O",
  handler = function()
    local terminal = pi.tui.terminal(100, 40)
    local dimensions = terminal:dimensions()
    terminal:start()
    local started = terminal:output()
    local negotiation = terminal:feed("\27[?0u")
    local modify_output = terminal:output()
    terminal:feed("\27[?7u")
    local kitty_output = terminal:output()
    local flags = terminal:protocol_flags()
    local input = terminal:feed("x")
    terminal:feed("\27[")
    local flushed = terminal:flush()

    terminal:write("ok")
    terminal:move(2)
    terminal:move(-1)
    terminal:cursor(false)
    terminal:cursor(true)
    terminal:clear()
    terminal:clear("below")
    terminal:clear("screen")
    terminal:title("pi-rs")
    terminal:progress(true)
    terminal:progress_keepalive()
    terminal:progress(false)
    local drawing = terminal:output()

    terminal:drain()
    local drained = terminal:output()
    local discarded = terminal:feed("ignored")
    terminal:stop()
    local stopped = terminal:output()
    return {
      dimensions = dimensions, started = started, negotiation = negotiation,
      modify_output = modify_output, kitty_output = kitty_output, flags = flags,
      input = input, flushed = flushed, drawing = drawing, drained = drained,
      discarded = discarded, stopped = stopped,
    }
  end,
})
