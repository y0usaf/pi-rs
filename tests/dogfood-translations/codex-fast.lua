-- pi-rs translation of the pinned dogfood package `pi-codex-fast` (v1.0.0,
-- `pi-flake` extensions/pi-codex-fast, src/index.ts). Preserves behavior with
-- the public Pi-compatible API + fs/settings mechanisms only: no privileged
-- escape hatch, no JS runtime. Read/configured-packages/bundled loading all
-- enter through the same `local pi = ...` chunk boundary.
local pi = ...

local DEFAULT_SETTINGS = {
  enabled = false,
  supportedModels = { "gpt-5.5" },
  showStatus = true,
}

local function is_record(value)
  return type(value) == "table"
end

local function parse_settings(raw)
  if type(raw) == "boolean" then return { enabled = raw } end
  if not is_record(raw) then return {} end
  local out = {}
  if type(raw.enabled) == "boolean" then out.enabled = raw.enabled end
  if type(raw.showStatus) == "boolean" then out.showStatus = raw.showStatus end
  if type(raw.supportedModels) == "table" then
    local models = {}
    for _, m in ipairs(raw.supportedModels) do
      if type(m) == "string" and m:gsub("^%s+", ""):gsub("%s+$", "") ~= "" then
        models[#models + 1] = m
      end
    end
    out.supportedModels = models
  end
  return out
end

local function pick_settings(parsed)
  local extension_settings = parsed and parsed.extensionSettings
  if not is_record(extension_settings) then return nil end
  return extension_settings["codex-fast"]
end

local function read_settings_file(path)
  local ok_exists, exists = pcall(pi.fs.exists, path)
  if not (ok_exists and exists) then return {} end
  local ok_read, content = pcall(pi.fs.read_file, path)
  if not ok_read then return {} end
  local ok_json, parsed = pcall(pi.json.decode, content)
  if not ok_json or not is_record(parsed) then return {} end
  return parse_settings(pick_settings(parsed))
end

-- getAgentDir(): the host agent dir; pi-rs exposes it via the environment
-- (`PI_CODING_AGENT_DIR` when set, else `$HOME/.pi/agent`), matching the
-- host's discover::agent_dir(). No privileged binding is used.
local function agent_dir()
  local env = pi.env.PI_CODING_AGENT_DIR
  if env and env ~= "" then return env end
  return (pi.env.HOME or "") .. "/.pi/agent"
end

local function load_settings(cwd)
  local merged = {}
  for key, value in pairs(DEFAULT_SETTINGS) do merged[key] = value end
  local function apply(partial)
    if partial.enabled ~= nil then merged.enabled = partial.enabled end
    if partial.showStatus ~= nil then merged.showStatus = partial.showStatus end
    if partial.supportedModels ~= nil then merged.supportedModels = partial.supportedModels end
  end
  apply(read_settings_file(pi.path.join(agent_dir(), "settings.json")))
  apply(read_settings_file(pi.path.join(cwd, ".pi", "settings.json")))
  return merged
end

local function is_codex_fast_active(ctx, settings)
  local model = ctx.model
  return model
    and model.provider == "openai-codex"
    and settings.enabled
    and contains(settings.supportedModels, model.id)
end

local function contains(list, value)
  for _, item in ipairs(list or {}) do if item == value then return true end end
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
    local supported = #settings.supportedModels > 0
      and table.concat(settings.supportedModels, ", ") or "(none)"
    local lines = {
      "codex-fast: " .. active,
      "model: " .. model,
      "enabled: " .. tostring(settings.enabled),
      "supportedModels: " .. supported,
      "config: ~/.pi/agent/settings.json#extensionSettings, .pi/settings.json#extensionSettings",
    }
    ctx.ui.notify(table.concat(lines, "\n"), "info")
  end,
})

pi.on("before_provider_request", function(event, ctx)
  local settings = load_settings(ctx.cwd)
  if not is_codex_fast_active(ctx, settings) then return end
  local payload = event.payload
  if not is_record(payload) or payload.service_tier ~= nil then return end

  local patched = {}
  for key, value in pairs(payload) do patched[key] = value end
  patched.service_tier = "priority"
  return patched
end)