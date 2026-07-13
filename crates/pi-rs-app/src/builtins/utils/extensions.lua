-- Product-side extension composition. Rust exposes registration/handler
-- snapshots; this Lua policy chooses active tools and event fold semantics.
EXTENSION_POLICY = EXTENSION_POLICY or {}

function EXTENSION_POLICY.active_tools()
  local active, names = {}, {}
  for _, definition in ipairs(pi.registered_active_tools()) do
    active[#active + 1] = definition
    names[#names + 1] = definition.name
  end
  return active, names
end

-- ExtensionRunner.emitToolCall: handlers run in extension/load order; the
-- latest non-nil result is retained and block short-circuits. Errors propagate
-- into the agent tool preflight, which settles them as failed tool results.
function EXTENSION_POLICY.emit_tool_call(core_event, context)
  local event = {
    type = "tool_call",
    toolCallId = core_event.toolCall.id,
    toolName = core_event.toolCall.name,
    input = core_event.args,
  }
  local result
  for _, entry in ipairs(pi.extension_handlers("tool_call")) do
    local value = entry.handler(event, context)
    if value ~= nil then
      result = value
      if value.block then return value end
    end
  end
  return result
end

function EXTENSION_POLICY.execute_command(text, context, options)
  if text:sub(1, 1) ~= "/" then return false end
  local body = text:sub(2)
  local command_name, args = body:match("^(%S+)%s?(.*)$")
  if not command_name then return false end
  for _, command in ipairs(pi.registered_extension_commands()) do
    if command.invocation_name == command_name then
      local function execute()
        local ok, err = pcall(command.handler, args or "", context)
        if not ok and options and options.on_error then options.on_error(tostring(err)) end
        return ok and nil or tostring(err)
      end
      if options and options.background then
        pi.spawn(execute)
        return true
      end
      return true, execute()
    end
  end
  return false
end
