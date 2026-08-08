-- stateful-tools: translation of the spec's tools.ts (non-UI subset) —
-- a /tools-enable command that persists the enabled-tool set to the
-- session via append_entry and restores it from the branch, using
-- get_all_tools / set_active_tools / get_active_tools (PLAN 9.4).
local pi = ...

-- tools.ts keeps the enabled set as a Set plus an ordered array for the
-- wire surface (Array.from(set)); the Lua port tracks both.
local enabled_tools = {}  -- set: name -> true
local enabled_order = {}  -- ordered array for set_active_tools
local all_tools = {}

local function persist_state()
  pi.append_entry("tools-config", { enabledTools = enabled_order })
end

local function apply_tools()
  pi.set_active_tools(enabled_order)
end

pi.register_command("tools-enable", {
  description = "Enable tools by name: /tools-enable <name1> <name2> ...",
  handler = function(args, ctx)
    all_tools = pi.get_all_tools()
    local known = {}
    for _, tool in ipairs(all_tools) do known[tool.name] = true end
    local requested = {}
    for name in args:gmatch("%S+") do
      if known[name] and not enabled_tools[name] then
        enabled_tools[name] = true
        enabled_order[#enabled_order + 1] = name
        requested[#requested + 1] = name
      end
    end
    persist_state()
    apply_tools()
    return { enabled = requested, active = pi.get_active_tools() }
  end,
})

pi.register_command("tools-state", {
  description = "Report the current tool set and any persisted config entries",
  handler = function(_, ctx)
    local entries = ctx.sessionManager and ctx.sessionManager.get_branch() or {}
    local saved = nil
    for _, entry in ipairs(entries) do
      if entry.type == "custom" and entry.customType == "tools-config" then
        saved = entry.data
      end
    end
    return { active = pi.get_active_tools(), saved = saved }
  end,
})
