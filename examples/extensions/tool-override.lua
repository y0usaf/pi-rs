-- tool-override: translation of the spec's tool-override.ts (subset) — an
-- extension registers a tool with the same name as a built-in ("read") to
-- replace it. No custom renderCall/renderResult: the built-in renderer is
-- used automatically. Exercises register_tool override, schema validation
-- (prepare_arguments + parameters), and the resolved first-registration
-- mirror (PLAN 9.4).
local pi = ...

local BLOCKED_PATTERNS = {
  "%.env$", "%.env%.", "secrets?%.", "credentials?%.",
}

local function is_blocked(path)
  for _, pattern in ipairs(BLOCKED_PATTERNS) do
    if path:match(pattern) then return true end
  end
  return false
end

pi.register_tool({
  name = "read", -- Same name as the built-in: this overrides it.
  label = "read (audited)",
  description = "Read a file with access logging. Sensitive paths are blocked.",
  parameters = {
    type = "object",
    properties = {
      path = { type = "string", description = "Path to read" },
      offset = { type = "number", description = "Line to start from (1-indexed)" },
      limit = { type = "number", description = "Maximum lines to read" },
    },
    required = { "path" },
  },
  prepare_arguments = function(params)
    if params.path and params.path:sub(1, 1) ~= "/" and not params.path:match("^%./") then
      params.path = "./" .. params.path
    end
    return params
  end,
  execute = function(tool_call_id, params, signal, on_update, ctx)
    if is_blocked(params.path) then
      return {
        content = { { type = "text",
          text = "Access denied: \"" .. params.path .. "\" matches a blocked pattern (sensitive file)." } },
        details = { blocked = true, cwd = ctx.cwd },
      }
    end
    -- The override owns the read policy; this exerciser reports the
    -- resolved path rather than performing IO so the accept test is
    -- hermetic. A real override would call pi.fs.read here.
    return {
      content = { { type = "text", text = "read " .. params.path .. " (offset="
        .. tostring(params.offset) .. ", limit=" .. tostring(params.limit) .. ")" } },
      details = { blocked = false, cwd = ctx.cwd },
    }
  end,
})

pi.register_command("tool-override-probe", {
  description = "Report the resolved read tool's metadata",
  handler = function()
    local all = pi.get_all_tools()
    for _, tool in ipairs(all) do
      if tool.name == "read" then
        return { label = "read (audited)", source = tool.sourceInfo.source,
          description = tool.description, hasParameters = tool.parameters ~= nil }
      end
    end
    return { missing = true }
  end,
})
