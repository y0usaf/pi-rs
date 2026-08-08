-- dynamic-tools: translation of the spec's dynamic-tools.ts — tools
-- registered after session initialization and at runtime via a command.
-- Exercises register_tool, get_all_tools, get_active_tools, and the
-- session_start event seam (PLAN 9.4).
local pi = ...

local registered = {}

local function register_echo_tool(name, label, prefix)
  if registered[name] then return false end
  registered[name] = true
  pi.register_tool({
    name = name,
    label = label,
    description = "Echo a message with prefix: " .. prefix,
    promptSnippet = "Echo back user-provided text with " .. prefix .. " prefix",
    promptGuidelines = { "Use " .. name .. " when the user asks for exact echo output." },
    parameters = {
      type = "object",
      properties = { message = { type = "string", description = "Message to echo" } },
      required = { "message" },
    },
    execute = function(tool_call_id, params)
      return {
        content = { { type = "text", text = prefix .. params.message } },
        details = { tool = name, prefix = prefix },
      }
    end,
  })
  return true
end

pi.on("session_start", function(event, ctx)
  register_echo_tool("echo_session", "Echo Session", "[session] ")
end)

pi.register_command("add-echo-tool", {
  description = "Register a new echo tool dynamically: /add-echo-tool <tool_name>",
  handler = function(args)
    local tool_name = args:match("^%s*(%S+)%s*$") or ""
    if tool_name == "" or not tool_name:match("^[a-z0-9_]+$") then
      return { created = false, reason = "invalid-name" }
    end
    local created = register_echo_tool(tool_name, "Echo " .. tool_name,
      "[" .. tool_name .. "] ")
    return { created = created }
  end,
})

pi.register_command("dynamic-tools-inspect", {
  description = "Report registered and active tool names",
  handler = function()
    local all = pi.get_all_tools()
    local names = {}
    for i, tool in ipairs(all) do names[i] = tool.name end
    return { names = names, active = pi.get_active_tools() }
  end,
})
