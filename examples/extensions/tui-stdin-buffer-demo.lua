-- Exercises deterministic terminal-input framing through the public Lua seam.
local pi = ...

pi.register_command("tui-stdin-buffer-demo", {
  description = "Buffer terminal input and bracketed paste",
  handler = function()
    local input = pi.tui.stdin_buffer()
    local first = input:feed("a\27[")
    local pending = input:buffer()
    local second = input:feed("A\27[200~hello")
    local paste = input:feed(" world\27[201~")
    input:feed("\27[")
    input:clear()
    local cleared = input:buffer()
    input:feed("\27[")
    local flushed = input:flush()
    return {
      first = first, pending = pending, second = second, paste = paste,
      cleared = cleared, flushed = flushed,
    }
  end,
})
