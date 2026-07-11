-- command-demo — exercises pi.register_command (WS1.2).
--
-- The registration half of pi's examples/extensions/commands.ts; the UI
-- half (ctx.ui.select and friends) arrives with the WS3 tui bindings.
-- Command handlers run on the coroutine path, so awaits work here too.
local pi = ...

pi.register_command("echo", {
	description = "Echo the arguments back",
	handler = function(args)
		pi.sleep(10)
		return { message = "echo: " .. args }
	end,
})
