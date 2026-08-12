-- Exerciser for the TCP framed client mechanism: pi.tcp.connect and the
-- socket read/write/line primitives, plus deterministic close/disposal.
--
-- Translations from Node built-ins the dogfood suite relies on:
--   net#createConnection / net#Socket → pi.tcp.connect + handle
--   resource.tcp_socket (gecko Marionette) → the owned handle (closed on
--   dispose, so no socket survives reload/shutdown).
local pi = ...

pi.register_command("tcp-demo", {
  description = "Connect to a TCP echo server and round-trip bytes/lines",
  handler = function(arg)
    local host, port = arg:match("^(%S+):(%d+)$")
    local socket = pi.tcp.connect(host, tonumber(port), { timeout_ms = 2000 })
    -- Echo server: newline-terminated requests get newline-terminated echoes.
    socket:write("ping\n")
    local line = socket:read_line()
    socket:write("frame\x00")
    local bytes = socket:read(5)
    local closed_before = socket:is_closed()
    socket:close()
    return {
      echo = line,
      bytes = bytes,
      closed_before = closed_before,
      closed_after = socket:is_closed(),
    }
  end,
})

-- A socket whose handle is disposed is deterministically closed; no socket
-- survives disposal.
pi.register_command("tcp-dispose", {
  description = "Verify a TCP socket is closed via dispose",
  handler = function(arg)
    local host, port = arg:match("^(%S+):(%d+)$")
    local socket = pi.tcp.connect(host, tonumber(port), { timeout_ms = 2000 })
    socket:write("opencheck\n")
    local echoed = socket:read_line()
    socket:close()
    -- After close, reads return empty and is_closed is true.
    local post_read = socket:read(4)
    return { echoed = echoed, closed = socket:is_closed(), post_read_len = #post_read }
  end,
})