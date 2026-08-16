-- File-backed pi-tool-management translation (dogfood package).
-- Public surface only: pi.register_command, pi.register_setting_item /
-- pi.registered_setting_items (push-model settings list), pi.getActiveTools /
-- pi.getAllTools / pi.setActiveTools / pi.appendEntry, ctx.ui.custom /
-- ctx.ui.notify / ctx.ui.theme.fg/bold, pi.tui.settings_list / text_render,
-- pi.fs.{exists,read_file,mkdir,write_file_atomic}, pi.path.join, pi.env,
-- pi.json.{decode,encode}. No privileged escape hatch; no long-lived host
-- resources (settings persist through the settings file; state is a Set).
local pi = ...

local SETTINGS_VERSION = 1
local ALLOWED = "allowed"
local BLOCKED = "blocked"
local BLOCKED_EXTERNALLY = "blocked (external)"

local function get_agent_dir()
  local home = pi.env.HOME or pi.env.USERPROFILE
  if home then return pi.path.join(home, ".pi", "agent") end
  return pi.path.join(".pi", "agent")
end

local SETTINGS_PATH = pi.path.join(get_agent_dir(), "tool-settings.json")

-- ── Helpers ────────────────────────────────────────────────────────
local function unique_sorted(arr)
  local seen = {}
  local out = {}
  for _, v in ipairs(arr or {}) do
    if type(v) == "string" and not seen[v] then
      seen[v] = true
      out[#out + 1] = v
    end
  end
  table.sort(out)
  return out
end

local function normalize_tool(tool)
  if type(tool) == "string" then
    local name = tool:gsub("^%s+", ""):gsub("%s+$", "")
    if name ~= "" then return { name = name } end
    return nil
  end
  if type(tool) ~= "table" or type(tool.name) ~= "string" then return nil end
  local name = tool.name:gsub("^%s+", ""):gsub("%s+$", "")
  if name == "" then return nil end
  local si = tool.sourceInfo
  if type(si) ~= "table" then return { name = name } end
  local rec = { name = name }
  if type(si.source) == "string" then rec.sourceInfo = { source = si.source } end
  if type(si.scope) == "string" then
    rec.sourceInfo = rec.sourceInfo or {}
    rec.sourceInfo.scope = si.scope
  end
  return rec
end

local function get_all_tool_records()
  local raw = pi.getAllTools()
  if type(raw) ~= "table" then return {} end
  local seen = {}
  local tools = {}
  for i = 1, #raw do
    local tool = normalize_tool(raw[i])
    if tool and not seen[tool.name] then
      seen[tool.name] = true
      tools[#tools + 1] = tool
    end
  end
  return tools
end

-- ── Settings I/O ───────────────────────────────────────────────────
local disabled_tools = {}
local last_warning
local last_save_error

local function parse_settings(raw)
  local ok, parsed = pcall(pi.json.decode, raw)
  if not ok or type(parsed) ~= "table" then
    return {}, "Ignoring invalid settings in " .. SETTINGS_PATH .. ": expected object"
  end
  if parsed.version ~= SETTINGS_VERSION then
    return {}, "Ignoring unsupported settings version in " .. SETTINGS_PATH .. ": " .. tostring(parsed.version)
  end
  local dt = {}
  if type(parsed.disabledTools) == "table" then
    for _, v in ipairs(parsed.disabledTools) do
      if type(v) == "string" then dt[#dt + 1] = v:gsub("^%s+", ""):gsub("%s+$", "") end
    end
  end
  return { disabledTools = unique_sorted(dt) }, nil
end

local function load_settings()
  if not pi.fs.exists(SETTINGS_PATH) then return end
  local ok, raw = pcall(pi.fs.read_file, SETTINGS_PATH)
  if not ok then
    last_warning = "Failed to load " .. SETTINGS_PATH .. ": " .. tostring(raw)
    return
  end
  local result, warning = parse_settings(raw)
  disabled_tools = {}
  for _, v in ipairs(result.disabledTools) do disabled_tools[v] = true end
  last_warning = warning
end

local function save_settings()
  local list = unique_sorted(disabled_tools)
  local file = { version = SETTINGS_VERSION, disabledTools = list }
  local body = pi.json.encode(file, true) .. "\n"
  local ok, err = pcall(function()
    pi.fs.mkdir(get_agent_dir(), true)
    pi.fs.write_file_atomic(SETTINGS_PATH, body)
  end)
  if not ok then
    last_save_error = "Failed to save " .. SETTINGS_PATH .. ": " .. tostring(err)
  else
    last_save_error = nil
  end
end

-- ── Tool sorting & enforcement ─────────────────────────────────────
local function get_tool_category(tool)
  local si = tool.sourceInfo
  if si and si.source == "builtin" then return "Built-in" end
  if si and si.source == "sdk" then return "SDK" end
  if si and si.scope == "project" then return "Project extension" end
  if si and si.scope == "user" then return "User extension" end
  if si then return "Extension" end
  return "Tool"
end

local function sort_tools(tools)
  local rank = function(t)
    local si = t.sourceInfo
    if si and si.source == "builtin" then return 0 end
    if si and si.source == "sdk" then return 1 end
    if si and si.scope == "project" then return 2 end
    if si and si.scope == "user" then return 3 end
    return 4
  end
  local out = {}
  for _, t in ipairs(tools) do out[#out + 1] = t end
  table.sort(out, function(a, b)
    local ra, rb = rank(a), rank(b)
    if ra ~= rb then return ra < rb end
    return (a.name or "") < (b.name or "")
  end)
  return out
end

local function get_tool_value(name, active)
  if disabled_tools[name] then return BLOCKED end
  if not active[name] then return BLOCKED_EXTERNALLY end
  return ALLOWED
end

local function enforce_disabled_tools()
  local all = {}
  for _, t in ipairs(get_all_tool_records()) do all[t.name] = true end
  local names = {}
  for n in pairs(all) do names[#names + 1] = n end
  if next(all) == nil then return end

  local active = pi.getActiveTools()
  if type(active) ~= "table" then return end
  local filtered = {}
  for _, n in ipairs(active) do
    if all[n] and not disabled_tools[n] then filtered[#filtered + 1] = n end
  end
  local changed = #active ~= #filtered
  if not changed then
    for i = 1, #active do
      if active[i] ~= filtered[i] then changed = true; break end
    end
  end
  if changed then pi.setActiveTools(filtered) end
end

local function reload_and_enforce()
  load_settings()
  enforce_disabled_tools()
end

-- ── /tools custom overlay ──────────────────────────────────────────
local function render_settings_overlay(ctx, all_tools)
  local done_called = false
  ctx.ui.custom(function(_, theme, _, done)
    local active = {}
    for _, n in ipairs(pi.getActiveTools()) do active[n] = true end
    local items = {}
    local blocked_externally = {}
    for _, tool in ipairs(all_tools) do
      local current = get_tool_value(tool.name, active)
      local t = {
        id = tool.name,
        label = tool.name .. " · " .. get_tool_category(tool),
        current_value = current,
        values = (current == BLOCKED) and { BLOCKED, ALLOWED }
                 or (current == BLOCKED_EXTERNALLY) and { BLOCKED_EXTERNALLY, BLOCKED }
                 or { ALLOWED, BLOCKED },
      }
      if current == BLOCKED_EXTERNALLY then
        t.description = "Blocked (external)."
        blocked_externally[#blocked_externally + 1] = tool.name
      end
      items[#items + 1] = t
    end
    table.sort(blocked_externally)

    local sl = pi.tui.settings_list(items, math.min(#items + 2, 15), false)

    local function render(width)
      local lines = {}
      local function push(text, style)
        if not text then return end
        local styled = style and style(text) or text
        local r = pi.tui.text_render(styled, width, 0, 0)
        for _, l in ipairs(r) do lines[#lines + 1] = l end
      end
      push("Tool Management", function(s) return theme:fg("accent", theme:bold(s)) end)
      push(SETTINGS_PATH, function(s) return theme:fg("dim", s) end)
      push("This menu edits this extension's global disabled-tools list.", function(s) return theme:fg("muted", s) end)
      if #blocked_externally > 0 then
        push("Blocked (external) now: " .. table.concat(blocked_externally, ", "), function(s) return theme:fg("warning", s) end)
      end
      push("Close + reopen to refresh tools added while this menu is open.", function(s) return theme:fg("muted", s) end)
      local rendered = sl:render(width)
      for _, l in ipairs(rendered) do lines[#lines + 1] = l end
      push("↑↓ navigate • ←/→ toggle • esc close", function(s) return theme:fg("dim", s) end)
      return lines
    end

    local function apply_toggle(id, new_value)
      if new_value == BLOCKED then disabled_tools[id] = true else disabled_tools[id] = nil end
      local ok = pcall(enforce_disabled_tools)
      if not ok then
        ctx.ui.notify("Failed to apply tool changes", "error")
      else
        local new_active = {}
        for _, n in ipairs(pi.getActiveTools()) do new_active[n] = true end
        sl:update_value(id, get_tool_value(id, new_active))
        pcall(save_settings)
        if last_save_error then
          ctx.ui.notify(last_save_error .. "\nChanges remain applied in this session.", "error")
        end
      end
    end

    return {
      render = render,
      handle_input = function(_, data)
        if data == "escape" or data == "\27" then
          if not done_called then done_called = true; done(nil) end
          return
        end
        local action = sl:input(data)
        if action then
          local kind = action.kind or action.kind_set
          if kind == "changed" and action.value then
            apply_toggle(action.id, action.value)
          elseif kind == "cancel" then
            if not done_called then done_called = true; done(nil) end
          end
        end
      end,
      dispose = function() end,
    }
  end)
end

local function reload_and_enforce()
  load_settings()
  enforce_disabled_tools()
end

-- ── Extension entry point ──────────────────────────────────────────
pi.register_command("tools", {
  description = "Manage this extension's global disabled-tools list (~/.pi/agent/tool-settings.json)",
  handler = function(_args, ctx)
    reload_and_enforce()
    local all_tools = sort_tools(get_all_tool_records())
    if #all_tools == 0 then
      ctx.ui.notify("No tools available", "info")
      return
    end
    render_settings_overlay(ctx, all_tools)
  end,
})

pi.register_command("tools-status", {
  description = "Show tool-settings.json status",
  handler = function(_args, ctx)
    reload_and_enforce()
    local all_tools = get_all_tool_records()
    local known = {}
    for _, t in ipairs(all_tools) do known[t.name] = true end
    local active = pi.getActiveTools()
    local active_known = 0
    for _, n in ipairs(active) do if known[n] then active_known = active_known + 1 end end
    local disabled = unique_sorted(disabled_tools)
    local unresolved = {}
    for _, n in ipairs(disabled) do if not known[n] then unresolved[#unresolved + 1] = n end end
    local blocked_externally = {}
    for _, t in ipairs(all_tools) do
      if not disabled_tools[t.name] then
        local is_active = false
        for _, n in ipairs(active) do if n == t.name then is_active = true end end
        if not is_active then blocked_externally[#blocked_externally + 1] = t.name end
      end
    end
    table.sort(blocked_externally)

    local lines = {
      "settings: " .. SETTINGS_PATH,
      "currentlyActiveAfterAllFilters: " .. active_known .. "/" .. #all_tools,
      "disabledTools: " .. (#disabled > 0 and table.concat(disabled, ", ") or "(none)"),
      "blockedExternally: " .. (#blocked_externally > 0 and table.concat(blocked_externally, ", ") or "(none)"),
      "note: blockedExternally means a known tool this extension allows is shown as blocked (external) when it is absent from the current runtime active-tool set (another extension or runtime mode may be hiding it)",
    }
    if #unresolved > 0 then lines[#lines + 1] = "unresolvedDisabledTools: " .. table.concat(unresolved, ", ") end
    if last_warning then lines[#lines + 1] = "loadWarning: " .. last_warning end
    if last_save_error then lines[#lines + 1] = "saveError: " .. last_save_error end

    ctx.ui.notify(table.concat(lines, "\n"), last_save_error and "error" or (last_warning and "warning" or "info"))
  end,
})

-- The push-model settings list: tool-management also contributes a custom
-- `/settings` row (an active-tools filter) through the host registry.
pi.register_setting_item({
  id = "activeToolsFilter",
  label = "Filter active tools",
  type = "text",
  settings_key = "toolManagement.activeFilter",
  default = "",
  source = "tool-management",
})

-- Enforce disabled tools on all 4 lifecycle hooks.
local hook_names = { "session_start", "session_tree", "before_agent_start", "before_provider_request" }
for _, event in ipairs(hook_names) do
  pi.on(event, function(_event, _ctx) reload_and_enforce() end)
end
