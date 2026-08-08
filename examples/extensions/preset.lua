-- preset: translation of the spec's preset.ts (non-UI subset) — named
-- presets configure model, thinking level, and active tools. Exercises
-- set_model, set_thinking_level, get_thinking_level, get_all_tools,
-- get_active_tools, set_active_tools (PLAN 9.4).
local pi = ...

local presets = {
  plan = {
    provider = "demo", model = "plan-model", thinkingLevel = "high",
    tools = { "read", "grep" },
  },
  fast = {
    thinkingLevel = "low",
    tools = { "read" },
  },
}

local active_preset_name

pi.register_flag("preset", {
  description = "Preset configuration to use",
  type = "string",
})

local function apply_preset(name, preset, ctx)
  local report = { name = name }
  if preset.provider and preset.model then
    local registry = ctx.modelRegistry or { find = function(provider, id) return pi.ai.find_model(provider, id) end }
    local model = registry.find(preset.provider, preset.model)
    if model then
      report.model = pi.set_model(model)
    else
      report.model_error = "not-found"
    end
  end
  if preset.thinkingLevel then
    pi.set_thinking_level(preset.thinkingLevel)
  end
  report.thinking = pi.get_thinking_level()
  if preset.tools and #preset.tools > 0 then
    local all = pi.get_all_tools()
    local known = {}
    for _, tool in ipairs(all) do known[tool.name] = true end
    local valid = {}
    for _, name in ipairs(preset.tools) do
      if known[name] then valid[#valid + 1] = name end
    end
    report.unknown = {}
    for _, name in ipairs(preset.tools) do
      if not known[name] then report.unknown[#report.unknown + 1] = name end
    end
    if #valid > 0 then pi.set_active_tools(valid) end
  end
  report.active = pi.get_active_tools()
  active_preset_name = name
  return report
end

pi.register_command("preset-apply", {
  description = "Apply a named preset: /preset-apply <name>",
  handler = function(args, ctx)
    local name = args:match("^%s*(.-)%s*$") or ""
    local preset = presets[name]
    if not preset then return { error = "unknown-preset" } end
    return apply_preset(name, preset, ctx)
  end,
})
