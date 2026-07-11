-- Exercises the awaitable HTTP mechanism used by Lua-authored startup policy.
local pi = ...

pi.register_command("http-demo", {
  description = "GET a URL and return its response",
  handler = function(args)
    local response = pi.http.get(args, {
      headers = { accept = "text/plain", ["x-pi-demo"] = "1" },
      timeout_ms = 5000,
    })
    return response
  end,
})
