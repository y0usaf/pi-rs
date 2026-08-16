-- File-backed pi-codex-fast translation (dogfood package).
-- Public surface only: events (on), register_command, ctx.ui.{setStatus,notify,theme.fg},
-- pi.fs (exists/read_file), pi.path.join, pi.env, pi.json.decode.
-- No privileged escape hatch, no long-lived resources.
local pi = ...

local DEFAULT_SETTINGS = {
  enabled = false,
  supportedModels = { "gpt-5.5" },
  showStatus = true,
}

local function parse_settings(raw)
  if type(raw) == "boolean" then return { enabled = raw } end
  if type(raw) ~= "table" then return {} end
  local out = {}
  if type(raw.enabled) == "boolean" then out.enabled = raw.enabled end
  if type(raw.showStatus) == "boolean" then out.showStatus = raw.showStatus end
  if type(raw.supportedModels) == "table" then
    local filtered = {}
    local is_array = false
    for _ in ipairs(raw.supportedModels) do is_array = true break end
    if is_array then
      for _, s in ipairs(raw.supportedModels) do
        if type(s) == "string" and s:gsub("%s", "") ~= "" then
          filtered[#filtered + 1] = s
        end
      end
      out.supportedModels = filtered
    end
  end
  return out
end

local function pick_settings(parsed)
  local ext = parsed and parsed.extensionSettings
  if type(ext) ~= "table" then return nil end
  return ext["codex-fast"]
end

local function read_settings_file(path)
  if not pi.fs.exists(path) then return {} end
  local ok, parsed = pcall(pi.json.decode, pi.fs.read_file(path))
  if not ok then return {} end
  if type(parsed) ~= "table" then return {} end
  return parse_settings(pick_settings(parsed))
end

local function get_agent_dir()
  local home = pi.env.HOME or pi.env.USERPROFILE or "."
  return pi.path.join(home, ".pi", "agent")
end

local function merge_settings(base, extra)
  local out = {}
  for key, value in pairs(base) do out[key] = value end
  for key, value in pairs(extra) do out[key] = value end
  return out
end

local function load_settings(cwd)
  return merge_settings(
    merge_settings(DEFAULT_SETTINGS, read_settings_file(pi.path.join(get_agent_dir(), "settings.json"))),
    read_settings_file(pi.path.join(cwd, ".pi", "settings.json")))
end

local function is_codex_fast_active(ctx, settings)
  local model = ctx.model
  if not model then return false end
  if model.provider ~= "openai-codex" then return false end
  if not settings.enabled then return false end
  for _, id in ipairs(settings.supportedModels) do
    if id == model.id then return true end
  end
  return false
end

local function update_status(ctx)
  local settings = load_settings(ctx.cwd)
  local active = settings.showStatus and is_codex_fast_active(ctx, settings)
  ctx.ui.setStatus("codex-fast", active and ctx.ui.theme:fg("accent", "⚡") or nil)
end

pi.on("session_start", function(_event, ctx)
  update_status(ctx)
end)

pi.on("model_select", function(_event, ctx)
  update_status(ctx)
end)

pi.register_command("codex-fast", {
  description = "Show Codex fast-mode status",
  handler = function(_args, ctx)
    local settings = load_settings(ctx.cwd)
    local model = ctx.model and (ctx.model.provider .. "/" .. ctx.model.id) or "none"
    local active = is_codex_fast_active(ctx, settings) and "on" or "off"
    local supported = table.concat(settings.supportedModels, ", ")
    ctx.ui.notify(table.concat({
      "codex-fast: " .. active,
      "model: " .. model,
      "enabled: " .. tostring(settings.enabled),
      "supportedModels: " .. (supported ~= "" and supported or "(none)"),
      "config: ~/.pi/agent/settings.json#extensionSettings, .pi/settings.json#extensionSettings",
    }, "\n"), "info")
  end,
})

pi.on("before_provider_request", function(event, ctx)
  local settings = load_settings(ctx.cwd)
  if not is_codex_fast_active(ctx, settings) then return end
  local payload = event.payload
  if type(payload) ~= "table" then return end
  if payload.service_tier ~= nil then return end

  local out = {}
  for key, value in pairs(payload) do out[key] = value end
  out.service_tier = "priority"
  return out
end)
