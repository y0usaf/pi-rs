-- await-demo — exercises the coroutine async seam (WS1.1).
--
-- Handlers run as coroutines and may await host futures; `pi.sleep(ms)`
-- suspends the handler without burning watchdog budget (the budget meters
-- Lua execution only). This is the Lua mirror of a pi extension doing
-- `await new Promise((r) => setTimeout(r, ms))`.
local pi = ...

pi.on("session_start", function(event)
	pi.sleep(50)
	return { message = "hello from await-demo (awaited 50ms)" }
end)
