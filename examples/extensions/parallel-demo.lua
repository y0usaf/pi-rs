-- parallel-demo — exercises structured concurrent host futures (WS4.5).
local pi = ...

pi.register_command("parallel-demo", {
	handler = function()
		local started = pi.monotonic_ms()
		local completed = pi.parallel({
			function() pi.sleep(30); return "slow" end,
			function() pi.sleep(5); return "fast" end,
		})
		return { first = completed[1].value, second = completed[2].value,
			firstIndex = completed[1].index, elapsed = pi.monotonic_ms() - started }
	end,
})
