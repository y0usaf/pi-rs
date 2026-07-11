-- spawn-demo — exercises pi.spawn background coroutines (PLAN 4.2): a
-- handler starts work in the background, keeps responding while it runs,
-- and joins it for the result. The interactive frontend runs agent turns
-- this way while its event loop keeps rendering.
local pi = ...

pi.register_command("spawn-demo", {
	handler = function()
		local ticks = 0
		local task = pi.spawn(function()
			pi.sleep(20)
			return "background-done"
		end)
		-- The caller keeps running (an event loop shape): the spawned
		-- coroutine advances whenever the caller awaits.
		while not task:done() do
			ticks = ticks + 1
			pi.sleep(2)
		end
		return { value = task:join(), ticks = ticks, done = task:done() }
	end,
})
