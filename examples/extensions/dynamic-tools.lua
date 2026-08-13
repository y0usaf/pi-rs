-- Translation of Pi v0.79.0 examples/extensions/dynamic-tools.ts.
-- Registers tools after session initialization and at runtime via a
-- /add-echo-tool command.
local pi = ...

local function normalize_tool_name(input)
  local trimmed = input:gsub("^%s+", ""):gsub("%s+$", ""):lower()
  if trimmed == "" then return nil end
  if not trimmed:match("^[a-z0-9_]+$") then return nil end
  return trimmed
end

local registered_tool_names = {}

local function register_echo_tool(name, label, prefix)
  if registered_tool_names[name] then
    return false
  end

  registered_tool_names[name] = true
  pi.register_tool({
    name = name,
    label = label,
    description = "Echo a message with prefix: " .. prefix,
    promptSnippet = "Echo back user-provided text with " .. prefix:gsub("%s+$", "") .. " prefix",
    promptGuidelines = { "Use echo_session when the user asks for exact echo output." },
    parameters = {
      type = "object",
      properties = { message = { type = "string", description = "Message to echo" } },
      required = { "message" },
    },
    execute = function(_tool_call_id, params)
      return {
        content = { { type = "text", text = prefix .. params.message } },
        details = { tool = name, prefix = prefix },
      }
    end,
  })
  return true
end

pi.on("session_start", function(_event, ctx)
  register_echo_tool("echo_session", "Echo Session", "[session] ")
  ctx.ui.notify("Registered dynamic tool: echo_session", "info")
end)

pi.register_command("add-echo-tool", {
  description = "Register a new echo tool dynamically: /add-echo-tool <tool_name>",
  handler = function(args, ctx)
    local tool_name = normalize_tool_name(args)
    if not tool_name then
      ctx.ui.notify("Usage: /add-echo-tool <tool_name> (lowercase, numbers, underscores)", "warning")
      return
    end

    local created = register_echo_tool(tool_name, "Echo " .. tool_name, "[" .. tool_name .. "] ")
    if not created then
      ctx.ui.notify("Tool already registered: " .. tool_name, "warning")
      return
    end

    ctx.ui.notify("Registered dynamic tool: " .. tool_name, "info")
  end,
})