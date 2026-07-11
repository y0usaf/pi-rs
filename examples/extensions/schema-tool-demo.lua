-- Exercises prepare_arguments followed by schema coercion/validation (WS4.1).
local pi = ...
pi.register_tool({
  name = "schema-demo", description = "Coerce a count through JSON Schema",
  parameters = { type = "object", properties = { count = { type = "integer" } }, required = { "count" } },
  prepare_arguments = function(args) return { count = args.count } end,
  execute = function(_id, params)
    return { content = { { type = "text", text = tostring(params.count + 1) } }, details = params }
  end,
})
