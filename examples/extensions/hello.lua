-- hello — minimal custom tool example (translation of pi's
-- examples/extensions/hello.ts; exercises pi.register_tool).
--
-- The parameters table is the JSON schema pi's TypeBox call generates.
-- Spec execute signature is (tool_call_id, params, signal, on_update,
-- ctx); the host passes the first two today — the rest arrive with their
-- host mechanisms in later WS1 steps.
local pi = ...

pi.register_tool({
	name = "hello",
	label = "Hello",
	description = "A simple greeting tool",
	parameters = {
		type = "object",
		properties = {
			name = { type = "string", description = "Name to greet" },
		},
		required = { "name" },
	},

	execute = function(_tool_call_id, params)
		return {
			content = { { type = "text", text = "Hello, " .. params.name .. "!" } },
			details = { greeted = params.name },
		}
	end,
})
