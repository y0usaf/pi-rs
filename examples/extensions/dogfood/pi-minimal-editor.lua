-- File-backed pi-minimal-editor translation (dogfood package).
-- Public surface only: ctx.ui.setFooter + the pi.footer data-provider facade
-- (get_git_branch / extension_statuses / available_provider_count / on_branch_change),
-- ctx.ui.setEditorComponent + the pi.tui.editor mechanism (the "CustomEditor"
-- substrate: a wrapping editor that composes border chrome around the default
-- editor lines), ctx.sessionManager.{get_entries,getCwd,getSessionName},
-- ctx.getContextUsage, ctx.model, ctx.modelRegistry.isUsingOAuth,
-- pi.getThinkingLevel, pi.tui.text_render, pi.env.HOME/USERPROFILE,
-- pi.path.join, pi.now_ms. No privileged escape hatch; cleanup disposes the
-- footer and restores the default editor on session_shutdown.
local pi = ...

local ANSI = "\27%[`%z%a%s@-~]"
local LEVEL_COLOR = {
  off = "thinkingOff", minimal = "thinkingMinimal", low = "thinkingLow",
  medium = "thinkingMedium", high = "thinkingHigh", xhigh = "thinkingXhigh",
}

local function visible_width(s)
  local n = 0
  for _ in s:gmatch(".") do n = n + 1 end
  return n
end

local function truncate_to_width(s, width, ellipsis)
  ellipsis = ellipsis or "…"
  if visible_width(s) <= width then return s end
  local out = ""
  local w = 0
  for ch in s:gmatch(".") do
    if w + 1 > width then break end
    out = out .. ch
    w = w + 1
  end
  return out .. ellipsis
end

local function compact(n)
  local n = math.floor(n)
  if n < 1e3 then return tostring(n) end
  if n < 1e4 then return string.format("%.1fk", n / 1e3) end
  if n < 1e6 then return tostring(math.floor(n / 1e3)) .. "k" end
  if n < 1e7 then return string.format("%.1fM", n / 1e6) end
  return tostring(math.floor(n / 1e6)) .. "M"
end

local function has_ansi(text)
  return text:match("\27[%d;]*m") ~= nil
end

local function single_line(s)
  s = (s or ""):gsub("[\r\n\t]", " ")
  s = s:gsub("%s+", " ")
  return s:gsub("^%s+", ""):gsub("%s+$", "")
end

local function status_color(theme, text)
  text = single_line(text)
  if text == "" then return nil end
  if has_ansi(text) then return text end
  return theme:fg("dim", text)
end

local function footer_stats(ctx, theme)
  local input, output, read, write, cost = 0, 0, 0, 0, 0
  local entries = ctx.sessionManager and ctx.sessionManager.get_entries() or {}
  for _, entry in ipairs(entries) do
    if entry.type == "message" and entry.message and entry.message.role == "assistant" and entry.message.usage then
      local u = entry.message.usage
      input = input + (u.input or 0)
      output = output + (u.output or 0)
      read = read + (u.cacheRead or 0)
      write = write + (u.cacheWrite or 0)
      if u.cost and u.cost.total then cost = cost + u.cost.total end
    end
  end
  local usage = ctx.getContextUsage and ctx.getContextUsage() or nil
  local context_window = ctx.model and ctx.model.contextWindow or 0
  if usage and usage.contextWindow and usage.contextWindow > 0 then context_window = usage.contextWindow end
  local pct = usage and usage.percent or 0
  local pct_text
  if usage and usage.percent ~= nil then
    pct_text = string.format("%.1f%%/%s (auto)", pct, compact(context_window))
  else
    pct_text = "?/" .. compact(context_window) .. " (auto)"
  end
  local sub = false
  if ctx.model and ctx.modelRegistry and ctx.modelRegistry.isUsingOAuth then
    sub = ctx.modelRegistry.isUsingOAuth(ctx.model)
  end
  local parts = {}
  if input > 0 then parts[#parts + 1] = "↑" .. compact(input) end
  if output > 0 then parts[#parts + 1] = "↓" .. compact(output) end
  if read > 0 then parts[#parts + 1] = "R" .. compact(read) end
  if write > 0 then parts[#parts + 1] = "W" .. compact(write) end
  if (cost > 0 or sub) then
    parts[#parts + 1] = "$" .. string.format("%.3f", cost) .. (sub and " (sub)" or "")
  end
  if pct > 90 then parts[#parts + 1] = theme:fg("error", pct_text)
  elseif pct > 70 then parts[#parts + 1] = theme:fg("warning", pct_text)
  else parts[#parts + 1] = pct_text end
  return table.concat(parts, " ")
end

local function borders(pi_ref, ctx, theme, width)
  local value = ctx.model and ctx.model.reasoning and pi_ref.getThinkingLevel() or nil
  local level = "off"
  if value and LEVEL_COLOR[value] then level = value end
  local color = LEVEL_COLOR[level]
  local fill = theme:fg(color, "─")

  local function box(...)
    local parts = {}
    for _, p in ipairs({ ... }) do
      if p then parts[#parts + 1] = p end
    end
    if #parts == 0 then return nil end
    return table.concat(parts, theme:fg("dim", " • "))
  end

  local function line(boxes)
    local width_num = width
    local fixed = 0
    for _, part in ipairs(boxes) do fixed = fixed + visible_width(part) end
    local gap
    if #boxes < 2 then
      gap = width_num - fixed
    else
      gap = math.floor((width_num - fixed) / (#boxes - 1))
    end
    if gap < 1 then gap = 1 end
    local text
    if #boxes == 0 then
      text = fill:rep(width_num)
    else
      text = table.concat(boxes, fill:rep(gap))
      if #boxes == 1 then text = text .. fill:rep(gap) end
    end
    local clipped = truncate_to_width(text, width_num, "…")
    local pad = string.rep(" ", math.max(0, width_num - visible_width(clipped)))
    return clipped .. pad
  end

  local home = pi_ref.env and (pi_ref.env.HOME or pi_ref.env.USERPROFILE)
  local cwd = ctx.sessionManager and ctx.sessionManager.getCwd and ctx.sessionManager.getCwd() or ""
  if home and cwd and cwd:sub(1, #home) == home then cwd = "~" .. cwd:sub(#home + 1) end
  local branch = pi_ref.footer and pi_ref.footer.get_git_branch and pi_ref.footer.get_git_branch(cwd)
  if branch then cwd = cwd .. " (" .. branch .. ")" end
  local session = ctx.sessionManager and ctx.sessionManager.getSessionName and ctx.sessionManager.getSessionName()
  if session then cwd = cwd .. " • " .. session end
  if visible_width(cwd) > width then
    local half = math.floor(width / 2) - 1
    if half > 1 then
      cwd = cwd:sub(1, half) .. "…" .. cwd:sub(-(half - 1))
    else
      cwd = cwd:sub(1, math.max(1, width))
    end
  end

  local model = (ctx.model and ctx.model.id) or "no-model"
  local provider_count = 0
  if pi_ref.footer and pi_ref.footer.available_provider_count then
    -- async facade; on the sync footer factory fall back to a best-effort read.
    local ok, res = pcall(pi_ref.footer.available_provider_count)
    if ok then provider_count = res or 0 end
  end
  local model_text = (provider_count > 1 and ctx.model) and ("(" .. ctx.model.provider .. ") " .. model) or model
  local thinking = nil
  if ctx.model and ctx.model.reasoning then
    thinking = (level == "off") and "thinking off" or level
  end

  local stats = footer_stats(ctx, theme)
  local top_boxes = {}
  if cwd ~= "" then top_boxes[#top_boxes + 1] = theme:fg("dim", cwd) end
  if stats ~= "" then top_boxes[#top_boxes + 1] = theme:fg("dim", stats) end
  local top = line(top_boxes)

  local bottom_boxes = {}
  bottom_boxes[#bottom_boxes + 1] = box(theme:fg("dim", model_text), thinking and theme:fg(level == "off" and "dim" or color, thinking) or nil)
  local statuses = {}
  if pi_ref.footer and pi_ref.footer.extension_statuses then
    statuses = pi_ref.footer.extension_statuses() or {}
  end
  local status_entries = {}
  for i = 1, #statuses do status_entries[#status_entries + 1] = statuses[i] end
  table.sort(status_entries, function(a, b) return (a or "") < (b or "") end)
  for _, value in ipairs(status_entries) do
    local st = status_color(theme, value)
    if st then bottom_boxes[#bottom_boxes + 1] = box(st) end
  end
  local bottom = line(bottom_boxes)

  return { top = top, bottom = bottom }
end

local function minimal_editor_for(pi_ref, ctx, theme, inner)
  local self = {}
  self.editor = inner
  function self:render(width)
    local lines = inner:render(width)
    if width < 4 or #lines == 0 then return lines end
    local b = borders(pi_ref, ctx, self._theme or theme, width)
    local out = { b.top }
    -- Strip the default first (prompt) line and any separator dashed line,
    -- mirroring MinimalEditor's chrome remapping.
    for i = 2, #lines do out[#out + 1] = lines[i] end
    out[#out + 1] = b.bottom
    return out
  end
  function self:set_text(text) inner:set_text(text) end
  function self:get_text() return inner:get_text() end
  function self:set_focused(f) inner:set_focused(f) end
  function self:handle_input(data) return inner:input_effect(data) end
  function self:input_effect(data) return inner:input_effect(data) end
  function self:set_terminal_rows(rows) if inner.set_terminal_rows then inner:set_terminal_rows(rows) end end
  function self:dispose() end
  return self
end

pi.on("session_start", function(_event, ctx)
  local pi_ref = pi
  local captured_ctx = ctx
  ctx.ui.setFooter(function(_tui, theme)
    local disposed = false
    local unsubscribe = function() end
    local ok_unsub = nil
    if pi.footer and pi.footer.on_branch_change then
      ok_unsub = pi.footer.on_branch_change(function() _tui:requestRender() end)
      if ok_unsub then unsubscribe = ok_unsub end
    end
    local footer_component = {
      dispose = function()
        if disposed then return end
        disposed = true
        if unsubscribe then unsubscribe() end
        ctx.ui.setFooter(nil)
      end,
      invalidate = function() end,
      render = function(_width)
        local b = borders(pi_ref, captured_ctx, theme, _width)
        return { b.top, b.bottom }
      end,
    }
    ctx.ui.setEditorComponent(function(editor_tui, editor_theme, _keybindings)
      local editor = pi.tui.editor("")
      local comp = minimal_editor_for(pi_ref, captured_ctx, editor_theme, editor)
      comp._theme = editor_theme
      comp._tui = editor_tui
      return comp
    end)
    return footer_component
  end)
end)

pi.on("session_shutdown", function(_event, ctx)
  ctx.ui.setEditorComponent(nil)
  ctx.ui.setFooter(nil)
end)
