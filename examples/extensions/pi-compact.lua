-- File-backed pi-compact translation: public ordered rendering middleware only.
-- No frontend classes, globals, imports, or prototype patching.
local pi = ...

local config = {
  user = { mode = "borderless", gap = true },
  thinking = { mode = "compact" },
  tool = { mode = "compact", gap = false },
  custom = { mode = "compact", gap = false },
}

local function strip_ansi(text)
  text = (text or ""):gsub("\27%][^\7\27]*\7", "")
  text = text:gsub("\27%][^\27]-\27\\", "")
  return text:gsub("\27%[[%d;:?]*[%a~]", "")
end

local function squash(text)
  return (strip_ansi(text):gsub("%s+", " "):match("^%s*(.-)%s*$"))
end

local function clip(text, limit)
  if #text <= limit then return text end
  return text:sub(1, limit - 1) .. "…"
end

local function one_line(text, context, background)
  local line = pi.tui.truncate(text:gsub("\t", "    "), context.width, "…", false)
  line = line .. string.rep(" ", math.max(0, context.width - pi.tui.visible_width(line)))
  if background then line = context.theme:bg(background, line) end
  return { line }
end

local function with_gap(lines, enabled)
  if enabled and #lines > 0 then lines[#lines + 1] = "" end
  return lines
end

local function without_vertical_padding(lines)
  if #lines <= 2 then return lines end
  local out = {}
  for index = 2, #lines - 1 do out[#out + 1] = lines[index] end
  return out
end

pi.register_render_middleware("user", {
  name = "pi-compact-user", order = -100,
  render = function(row, context, next_render)
    local mode = config.user.mode
    if mode == "normal" then return next_render() end
    if mode == "hidden" then return {} end
    if mode == "borderless" then return with_gap(without_vertical_padding(next_render()), config.user.gap) end
    local text = clip(squash(row.text), 512)
    local lines = one_line("::: " .. (text ~= "" and text or "…"), context, "userMessageBg")
    lines[1] = "\27]133;A\7" .. lines[1] .. "\27]133;B\7\27]133;C\7"
    return with_gap(lines, config.user.gap)
  end,
})

local function assistant_without_thinking(row)
  local message = row.message or {}
  local copy = {}
  for key, value in pairs(message) do if key ~= "content" then copy[key] = value end end
  copy.content = {}
  for _, block in ipairs(message.content or {}) do
    if block.type ~= "thinking" then copy.content[#copy.content + 1] = block end
  end
  return { kind = "assistant", message = copy, streaming = row.streaming }
end

pi.register_render_middleware("assistant", {
  name = "pi-compact-thinking", order = -100,
  render = function(row, context, next_render)
    local blocks, chars = 0, 0
    for _, block in ipairs((row.message and row.message.content) or {}) do
      if block.type == "thinking" and squash(block.thinking) ~= "" then
        blocks, chars = blocks + 1, chars + #strip_ansi(block.thinking)
      end
    end
    if blocks == 0 or config.thinking.mode == "normal" then return next_render() end
    local rendered = next_render(assistant_without_thinking(row))
    if config.thinking.mode == "hidden" then return rendered end
    context.state.started = context.state.started or pi.now_ms()
    local elapsed = context.streaming and math.max(0, (pi.now_ms() - context.state.started) / 1000) or 0
    local label = string.format("🧠 %.1fs · %d %s", elapsed, chars, chars == 1 and "char" or "chars")
    local background = context.streaming and "toolPendingBg" or "toolSuccessBg"
    local compact = one_line(context.theme:fg("muted", label), context, background)
    for _, line in ipairs(rendered) do compact[#compact + 1] = line end
    return compact
  end,
})

local function tool_summary(row)
  local args = row.args or {}
  local value = args.path or args.command or args.pattern or args.query
  if value == nil then value = pi.json.encode(args) end
  value = clip(squash(tostring(value)), 120)
  local prefix = row.state == "pending" and "⠋" or (row.state == "error" and "✗" or "✓")
  return prefix .. " " .. (row.name or "tool") .. (value ~= "" and (" ╱ " .. value) or "")
end

pi.register_render_middleware("tool", {
  name = "pi-compact-tool", order = -100,
  render = function(row, context, next_render)
    local mode = config.tool.mode
    if mode == "normal" or context.expanded then return next_render() end
    if mode == "hidden" then return {} end
    if mode == "borderless" then return with_gap(without_vertical_padding(next_render()), config.tool.gap) end
    local background = row.state == "pending" and "toolPendingBg"
      or (row.state == "error" and "toolErrorBg" or "toolSuccessBg")
    return with_gap(one_line(tool_summary(row), context, background), config.tool.gap)
  end,
})

pi.register_render_middleware("custom", {
  name = "pi-compact-custom", order = -100,
  render = function(row, context, next_render)
    local mode = config.custom.mode
    if mode == "normal" or context.expanded then return next_render() end
    if mode == "hidden" then return {} end
    if mode == "borderless" then return with_gap(without_vertical_padding(next_render()), config.custom.gap) end
    local message = row.message or row
    local content = type(message.content) == "string" and message.content or pi.json.encode(message.content or {})
    local label = "[" .. (message.customType or "custom") .. "] " .. clip(squash(content), 120)
    return with_gap(one_line(context.theme:fg("muted", label), context, "toolSuccessBg"), config.custom.gap)
  end,
})

pi.register_command("compact-rendering", {
  description = "Set compact rendering: user|thinking|tool|custom MODE",
  handler = function(args, ctx)
    local kind, mode = args:match("^(%S+)%s+(%S+)$")
    local target = config[kind or ""]
    local valid = { normal = true, borderless = true, compact = true, hidden = true }
    if not target or not valid[mode or ""] or (kind == "thinking" and mode == "borderless") then
      ctx.ui.notify("Usage: /compact-rendering user|thinking|tool|custom normal|borderless|compact|hidden", "error")
      return
    end
    target.mode = mode
    ctx.ui.notify("pi-compact: " .. kind .. "=" .. mode, "info")
  end,
})
