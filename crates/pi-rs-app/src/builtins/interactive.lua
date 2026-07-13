local pi = ...
pi.declare_package({ command_visibility = "internal" })

-- Product policy ported from interactive/theme/theme.ts. Theme assets are
-- injected ahead of this file by the embedded-pack declaration below.
local dark_json = pi.json.decode(DARK_THEME_JSON)
local light_json = pi.json.decode(LIGHT_THEME_JSON)

local CUBE = { 0, 95, 135, 175, 215, 255 }
local GRAY = {}
for i = 0, 23 do GRAY[i + 1] = 8 + i * 10 end

local function hex_rgb(hex)
  local cleaned = hex:gsub("#", "")
  if #cleaned ~= 6 then error("Invalid hex color: " .. hex) end
  local r, g, b = tonumber(cleaned:sub(1, 2), 16), tonumber(cleaned:sub(3, 4), 16), tonumber(cleaned:sub(5, 6), 16)
  if not r or not g or not b then error("Invalid hex color: " .. hex) end
  return r, g, b
end

local function closest(values, value)
  local best, distance = 1, math.huge
  for i, candidate in ipairs(values) do
    local next_distance = math.abs(value - candidate)
    if next_distance < distance then best, distance = i, next_distance end
  end
  return best
end

local function color_distance(r1, g1, b1, r2, g2, b2)
  local dr, dg, db = r1 - r2, g1 - g2, b1 - b2
  return dr * dr * 0.299 + dg * dg * 0.587 + db * db * 0.114
end

local function rgb_256(r, g, b)
  local ri, gi, bi = closest(CUBE, r), closest(CUBE, g), closest(CUBE, b)
  local cube_index = 16 + 36 * (ri - 1) + 6 * (gi - 1) + (bi - 1)
  local cube_distance = color_distance(r, g, b, CUBE[ri], CUBE[gi], CUBE[bi])
  -- JS Math.round(x) for non-negative x.
  local gray = math.floor(0.299 * r + 0.587 * g + 0.114 * b + 0.5)
  local gray_i = closest(GRAY, gray)
  local gray_distance = color_distance(r, g, b, GRAY[gray_i], GRAY[gray_i], GRAY[gray_i])
  if math.max(r, g, b) - math.min(r, g, b) < 10 and gray_distance < cube_distance then
    return 231 + gray_i
  end
  return cube_index
end

local BG_KEYS = {
  selectedBg = true, userMessageBg = true, customMessageBg = true,
  toolPendingBg = true, toolSuccessBg = true, toolErrorBg = true,
}

local function resolve(value, vars, visited)
  if type(value) == "number" or value == "" or value:sub(1, 1) == "#" then return value end
  visited = visited or {}
  if visited[value] then error("Circular variable reference detected: " .. value) end
  if vars[value] == nil then error("Variable reference not found: " .. value) end
  visited[value] = true
  return resolve(vars[value], vars, visited)
end

local function color_ansi(value, mode, background)
  local base = background and 48 or 38
  if value == "" then return background and "\27[49m" or "\27[39m" end
  if type(value) == "number" then return string.format("\27[%d;5;%dm", base, value) end
  local r, g, b = hex_rgb(value)
  if mode == "truecolor" then return string.format("\27[%d;2;%d;%d;%dm", base, r, g, b) end
  return string.format("\27[%d;5;%dm", base, rgb_256(r, g, b))
end

local function create_theme(data, mode)
  local theme = { name = data.name, mode = mode, fg_codes = {}, bg_codes = {} }
  local vars = data.vars or {}
  for key, value in pairs(data.colors) do
    local resolved = resolve(value, vars)
    if BG_KEYS[key] then theme.bg_codes[key] = color_ansi(resolved, mode, true)
    else theme.fg_codes[key] = color_ansi(resolved, mode, false) end
  end
  function theme:fg(key, text)
    local ansi = self.fg_codes[key]
    if not ansi then error("Unknown theme color: " .. key) end
    return ansi .. text .. "\27[39m"
  end
  function theme:bg(key, text)
    local ansi = self.bg_codes[key]
    if not ansi then error("Unknown theme background color: " .. key) end
    return ansi .. text .. "\27[49m"
  end
  -- chalk styles: an inner close sequence is replaced by the open sequence
  -- so the style resumes after nested resets (chalk `stringReplaceAll`),
  -- and every newline is encased close/open (chalk
  -- `stringEncaseCRLFWithFirstIndex`) so each line carries its own codes.
  local function chalk_style(open, close)
    local close_pattern = close:gsub("%[", "%%[")
    return function(text)
      local body = text:gsub(close_pattern, open)
      body = body:gsub("\n", close .. "\n" .. open)
      return open .. body .. close
    end
  end
  theme.bold = (function()
    local style = chalk_style("\27[1m", "\27[22m")
    return function(_, text) return style(text) end
  end)()
  theme.italic = (function()
    local style = chalk_style("\27[3m", "\27[23m")
    return function(_, text) return style(text) end
  end)()
  theme.underline = (function()
    local style = chalk_style("\27[4m", "\27[24m")
    return function(_, text) return style(text) end
  end)()
  theme.strikethrough = (function()
    local style = chalk_style("\27[9m", "\27[29m")
    return function(_, text) return style(text) end
  end)()
  theme.inverse = (function()
    local style = chalk_style("\27[7m", "\27[27m")
    return function(_, text) return style(text) end
  end)()
  return theme
end

-- Port of theme.ts getMarkdownTheme. Validated languages highlight through
-- the pi.hljs engine (shared utils/syntax-highlight.lua fragment).
local function get_markdown_theme(theme)
  return {
    heading = function(t) return theme:fg("mdHeading", t) end,
    link = function(t) return theme:fg("mdLink", t) end,
    link_url = function(t) return theme:fg("mdLinkUrl", t) end,
    code = function(t) return theme:fg("mdCode", t) end,
    code_block = function(t) return theme:fg("mdCodeBlock", t) end,
    code_block_border = function(t) return theme:fg("mdCodeBlockBorder", t) end,
    quote = function(t) return theme:fg("mdQuote", t) end,
    quote_border = function(t) return theme:fg("mdQuoteBorder", t) end,
    hr = function(t) return theme:fg("mdHr", t) end,
    list_bullet = function(t) return theme:fg("mdListBullet", t) end,
    bold = function(t) return theme:bold(t) end,
    italic = function(t) return theme:italic(t) end,
    underline = function(t) return theme:underline(t) end,
    strikethrough = function(t) return theme:strikethrough(t) end,
    highlight_code = function(code, lang)
      return markdown_highlight_code(code, lang, theme)
    end,
  }
end

local DEFAULT_KEYS = {
  ["app.interrupt"] = "escape", ["app.clear"] = "ctrl+c", ["app.exit"] = "ctrl+d",
  ["app.suspend"] = pi.platform() == "win32" and "" or "ctrl+z",
  ["app.thinking.cycle"] = "shift+tab",
  ["app.model.cycleForward"] = "ctrl+p", ["app.model.cycleBackward"] = "shift+ctrl+p",
  ["app.model.select"] = "ctrl+l", ["app.tools.expand"] = "ctrl+o",
  ["app.thinking.toggle"] = "ctrl+t", ["app.editor.external"] = "ctrl+g",
  ["app.message.followUp"] = "alt+enter", ["app.message.dequeue"] = "alt+up",
  ["app.clipboard.pasteImage"] = "ctrl+v", ["tui.editor.deleteToLineEnd"] = "ctrl+k",
  -- core/keybindings.ts app.session.* + app.models.* selector surfaces.
  ["app.session.toggleNamedFilter"] = "ctrl+n", ["app.session.togglePath"] = "ctrl+p",
  ["app.session.toggleSort"] = "ctrl+s", ["app.session.rename"] = "ctrl+r",
  ["app.session.delete"] = "ctrl+d", ["app.session.deleteNoninvasive"] = "ctrl+backspace",
  ["app.models.save"] = "ctrl+s", ["app.models.enableAll"] = "ctrl+a",
  ["app.models.clearAll"] = "ctrl+x", ["app.models.toggleProvider"] = "ctrl+p",
  ["app.models.reorderUp"] = "alt+up", ["app.models.reorderDown"] = "alt+down",
  ["tui.input.tab"] = "tab",
  -- core/keybindings.ts app.tree.* (the tree-selector surface; the
  -- app.session.tree/fork actions ship with empty defaultKeys, so they
  -- are reachable only through /tree, /fork, and double-escape until
  -- user keybinding config lands with item 7).
  ["app.tree.foldOrUp"] = { "ctrl+left", "alt+left" },
  ["app.tree.unfoldOrDown"] = { "ctrl+right", "alt+right" },
  ["app.tree.editLabel"] = "shift+l", ["app.tree.toggleLabelTimestamp"] = "shift+t",
  ["app.tree.filter.default"] = "ctrl+d", ["app.tree.filter.noTools"] = "ctrl+t",
  ["app.tree.filter.userOnly"] = "ctrl+u", ["app.tree.filter.labeledOnly"] = "ctrl+l",
  ["app.tree.filter.all"] = "ctrl+a", ["app.tree.filter.cycleForward"] = "ctrl+o",
  ["app.tree.filter.cycleBackward"] = "shift+ctrl+o",
  ["tui.editor.cursorLeft"] = "left", ["tui.editor.cursorRight"] = "right",
  ["tui.editor.deleteCharBackward"] = "backspace",
}

-- JS String.split semantics (empty pieces preserved), so "/" formats as
-- "/" like formatKeyText's split("/")/split("+") round trip.
local function split_all(text, sep)
  local out, pos = {}, 1
  while true do
    local s, e = text:find(sep, pos, true)
    if not s then
      out[#out + 1] = text:sub(pos)
      return out
    end
    out[#out + 1] = text:sub(pos, s - 1)
    pos = e + 1
  end
end

local function format_key(key, capitalize)
  local result = {}
  for _, slash_part in ipairs(split_all(key, "/")) do
    local pieces = {}
    for _, part in ipairs(split_all(slash_part, "+")) do
      if capitalize then part = part:sub(1, 1):upper() .. part:sub(2) end
      pieces[#pieces + 1] = part
    end
    result[#result + 1] = table.concat(pieces, "+")
  end
  return table.concat(result, "/")
end

local function header(options, theme)
  local keys = options.keys or DEFAULT_KEYS
  local function key_text(action) return format_key(keys[action] or "", false) end
  local function hint(action, description)
    return theme:fg("dim", key_text(action)) .. theme:fg("muted", " " .. description)
  end
  local function raw_hint(key, description)
    return theme:fg("dim", format_key(key, false)) .. theme:fg("muted", " " .. description)
  end
  local logo = theme:bold(theme:fg("accent", options.app_name or "pi")) .. theme:fg("dim", " v" .. (options.version or "0.1.0"))
  local onboarding = theme:fg("dim", "Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.")
  if options.expanded then
    return table.concat({ logo,
      hint("app.interrupt", "to interrupt"), hint("app.clear", "to clear"),
      raw_hint(key_text("app.clear") .. " twice", "to exit"), hint("app.exit", "to exit (empty)"),
      hint("app.suspend", "to suspend"), hint("tui.editor.deleteToLineEnd", "to delete to end"),
      hint("app.thinking.cycle", "to cycle thinking level"),
      raw_hint(key_text("app.model.cycleForward") .. "/" .. key_text("app.model.cycleBackward"), "to cycle models"),
      hint("app.model.select", "to select model"), hint("app.tools.expand", "to expand tools"),
      hint("app.thinking.toggle", "to expand thinking"), hint("app.editor.external", "for external editor"),
      raw_hint("/", "for commands"), raw_hint("!", "to run bash"), raw_hint("!!", "to run bash (no context)"),
      hint("app.message.followUp", "to queue follow-up"), hint("app.message.dequeue", "to edit all queued messages"),
      hint("app.clipboard.pasteImage", "to paste image"), raw_hint("drop files", "to attach"), "", onboarding,
    }, "\n")
  end
  local compact = table.concat({ hint("app.interrupt", "interrupt"),
    raw_hint(key_text("app.clear") .. "/" .. key_text("app.exit"), "clear/exit"),
    raw_hint("/", "commands"), raw_hint("!", "bash"), hint("app.tools.expand", "more") }, theme:fg("muted", " · "))
  local help = theme:fg("dim", "Press " .. key_text("app.tools.expand") .. " to show full startup help and loaded resources.")
  return logo .. "\n" .. compact .. "\n" .. help .. "\n\n" .. onboarding
end

local function format_tokens(count)
  if count < 1000 then return tostring(count) end
  if count < 10000 then return string.format("%.1fk", count / 1000) end
  if count < 1000000 then return string.format("%dk", math.floor(count / 1000 + 0.5)) end
  if count < 10000000 then return string.format("%.1fM", count / 1000000) end
  return string.format("%dM", math.floor(count / 1000000 + 0.5))
end

local function sanitize_status(text)
  return (text:gsub("[\r\n\t]", " "):gsub(" +", " "):match("^%s*(.-)%s*$"))
end

-- components/footer.ts formatCwdForFooter — paths arrive absolute from
-- the process, so the spec's resolve() is the identity here.
local function format_cwd_for_footer(cwd, home)
  if not home or home == "" then return cwd end
  if cwd == home then return "~" end
  if cwd:sub(1, #home + 1) == home .. "/" then return "~/" .. cwd:sub(#home + 2) end
  return cwd
end

local function footer(options, theme)
  local width = options.width
  local pwd = format_cwd_for_footer(options.cwd, options.home)
  if options.branch and options.branch ~= "" then pwd = pwd .. " (" .. options.branch .. ")" end
  if options.session_name and options.session_name ~= "" then pwd = pwd .. " • " .. options.session_name end
  local parts = {}
  local u = options.usage or {}
  if (u.input or 0) ~= 0 then parts[#parts + 1] = "↑" .. format_tokens(u.input) end
  if (u.output or 0) ~= 0 then parts[#parts + 1] = "↓" .. format_tokens(u.output) end
  if (u.cache_read or 0) ~= 0 then parts[#parts + 1] = "R" .. format_tokens(u.cache_read) end
  if (u.cache_write or 0) ~= 0 then parts[#parts + 1] = "W" .. format_tokens(u.cache_write) end
  -- footer.ts: totals gate the CH display, but the rate itself is the
  -- latest assistant entry's. `false` = known-absent (the latest entry
  -- had no prompt tokens — CH hides even with cached totals); nil =
  -- not provided — single-entry stubs compute from the totals
  -- (identical there).
  local cache_hit_rate = options.cache_hit_rate
  if cache_hit_rate == nil then
    local prompt = (u.input or 0) + (u.cache_read or 0) + (u.cache_write or 0)
    if prompt > 0 then cache_hit_rate = ((u.cache_read or 0) / prompt) * 100 end
  end
  if ((u.cache_read or 0) > 0 or (u.cache_write or 0) > 0)
     and cache_hit_rate ~= nil and cache_hit_rate ~= false then
    parts[#parts + 1] = string.format("CH%.1f%%", cache_hit_rate)
  end
  if (u.cost or 0) ~= 0 or options.subscription then
    parts[#parts + 1] = string.format("$%.3f%s", u.cost or 0, options.subscription and " (sub)" or "")
  end
  local context = options.context_percent == nil and "?" or string.format("%.1f", options.context_percent)
  local context_display = (context == "?" and "?/" or context .. "%/") .. format_tokens(options.context_window or 0) .. (options.auto_compact == false and "" or " (auto)")
  local context_number = options.context_percent or 0
  if context_number > 90 then context_display = theme:fg("error", context_display)
  elseif context_number > 70 then context_display = theme:fg("warning", context_display) end
  parts[#parts + 1] = context_display
  local left = table.concat(parts, " ")
  if pi.tui.visible_width(left) > width then left = pi.tui.truncate(left, width, "...", false) end
  local model = options.model_id or "no-model"
  if options.reasoning then model = model .. (options.thinking_level == "off" and " • thinking off" or " • " .. (options.thinking_level or "off")) end
  local right = model
  if (options.provider_count or 1) > 1 and options.provider then
    local with_provider = "(" .. options.provider .. ") " .. model
    if pi.tui.visible_width(left) + 2 + pi.tui.visible_width(with_provider) <= width then right = with_provider end
  end
  local lw, rw = pi.tui.visible_width(left), pi.tui.visible_width(right)
  local stats
  if lw + 2 + rw <= width then stats = left .. string.rep(" ", width - lw - rw) .. right
  else
    local available = width - lw - 2
    if available > 0 then
      right = pi.tui.truncate(right, available, "", false)
      stats = left .. string.rep(" ", math.max(0, width - lw - pi.tui.visible_width(right))) .. right
    else stats = left end
  end
  local lines = { pi.tui.truncate(theme:fg("dim", pwd), width, theme:fg("dim", "..."), false), theme:fg("dim", left) .. theme:fg("dim", stats:sub(#left + 1)) }
  if options.statuses and #options.statuses > 0 then
    table.sort(options.statuses, function(a, b) return a.key < b.key end)
    local values = {}; for _, item in ipairs(options.statuses) do values[#values + 1] = sanitize_status(item.text) end
    lines[#lines + 1] = pi.tui.truncate(table.concat(values, " "), width, theme:fg("dim", "..."), false)
  end
  return lines
end

-- Product-policy port of modes/interactive/components/custom-editor.ts. The
-- wrapped pi.tui editor remains the Rust terminal mechanism; all routing and
-- replaceable app handlers live here in Lua.
local function canonical_key(key)
  if key == "esc" then return "escape" end
  local modifiers, base = {}, nil
  for part in key:gmatch("[^+]+") do
    if part == "ctrl" or part == "shift" or part == "alt" or part == "super" then modifiers[part] = true
    else base = part end
  end
  if not base then return key end
  local out = {}
  for _, modifier in ipairs({ "ctrl", "shift", "alt", "super" }) do
    if modifiers[modifier] then out[#out + 1] = modifier end
  end
  out[#out + 1] = base
  return table.concat(out, "+")
end

local function binding_matches(data, binding)
  if binding == nil then return false end
  local function matches_one(key)
    -- keys.ts: `shift+letter` matches the legacy uppercase character.
    local shifted = key:match("^shift%+(%l)$")
    if shifted and data == shifted:upper() then return true end
    local decoded = pi.tui.decode_key(data)
    if not decoded then return false end
    return canonical_key(decoded) == canonical_key(key)
  end
  if type(binding) == "table" then
    for _, key in ipairs(binding) do
      if matches_one(key) then return true end
    end
    return false
  end
  return matches_one(binding)
end

-- theme.ts getSelectListTheme over the mechanism's open/close style slots:
-- accent selection, muted description/scroll-info/no-match.
local function get_select_list_theme(theme)
  local function fg(key) return { open = theme.fg_codes[key], close = "\27[39m" } end
  return {
    selected_text = fg("accent"),
    description = fg("muted"),
    scroll_info = fg("muted"),
    no_match = fg("muted"),
  }
end

local function custom_editor(options)
  options = options or {}
  local value = options.value or ""
  local self = {
    editor = pi.tui.editor(value),
    theme = options.theme,
    keys = options.keys or DEFAULT_KEYS,
    action_handlers = {},
    action_order = {},
    on_escape = options.on_escape,
    on_ctrl_d = options.on_ctrl_d,
    on_paste_image = options.on_paste_image,
    on_extension_shortcut = options.on_extension_shortcut,
  }
  -- theme.ts getEditorTheme: borderColor = theme.fg("borderMuted", …) and
  -- selectList = getSelectListTheme().
  if options.theme then
    self.editor:set_border_style(options.theme.fg_codes.borderMuted, "\27[39m")
    self.editor:set_select_list_theme(get_select_list_theme(options.theme))
  end

  function self:on_action(action, handler)
    if self.action_handlers[action] == nil then self.action_order[#self.action_order + 1] = action end
    self.action_handlers[action] = handler
  end
  for action, handler in pairs(options.action_handlers or {}) do self:on_action(action, handler) end
  function self:matches(data, action) return binding_matches(data, self.keys[action]) end
  function self:handle_input(data)
    if self.on_extension_shortcut and self.on_extension_shortcut(data) then return { kind = "extension" } end
    if self:matches(data, "app.clipboard.pasteImage") then
      if self.on_paste_image then self.on_paste_image() end
      return { kind = "app", action = "app.clipboard.pasteImage" }
    end
    if self:matches(data, "app.interrupt") then
      if not self.editor:autocomplete_showing() then
        local handler = self.on_escape or self.action_handlers["app.interrupt"]
        if handler then handler(); return { kind = "app", action = "app.interrupt" } end
      end
      return self.editor:input_effect(data)
    end
    if self:matches(data, "app.exit") and #self.editor:get_text() == 0 then
      local handler = self.on_ctrl_d or self.action_handlers["app.exit"]
      if handler then handler() end
      return { kind = "app", action = "app.exit" }
    end
    for _, action in ipairs(self.action_order) do
      local handler = self.action_handlers[action]
      if action ~= "app.interrupt" and action ~= "app.exit" and handler and self:matches(data, action) then
        handler()
        return { kind = "app", action = action }
      end
    end
    return self.editor:input_effect(data)
  end
  return self
end

-- Deterministic exerciser for the policy object until the process frontend
-- owns it. It deliberately drives the same factory the mounted UI will use.
pi.register_command("interactive-custom-editor", {
  description = "Exercise coding-agent CustomEditor input routing",
  handler = function(args)
    local options = pi.json.decode(args)
    local trace = {}
    local function record(action) trace[#trace + 1] = action end
    local editor = custom_editor({
      value = options.value,
      keys = options.keys,
      on_escape = function() record("escape") end,
      on_ctrl_d = function() record("exit") end,
      on_paste_image = function() record("pasteImage") end,
      on_extension_shortcut = function(data)
        if options.extension_shortcut and binding_matches(data, options.extension_shortcut) then
          record("extension"); return true
        end
        return false
      end,
    })
    for _, action in ipairs(options.actions or {}) do
      editor:on_action(action, function() record(action) end)
    end
    local effects = {}
    for _, data in ipairs(options.input or {}) do effects[#effects + 1] = editor:handle_input(data) end
    return { text = editor.editor:get_text(), trace = trace, effects = effects }
  end,
})

pi.register_command("interactive-startup-core", {
  description = "Render the exact landed startup/header/footer policy core",
  handler = function(args)
    local options = pi.json.decode(args)
    local data = options.theme == "light" and light_json or dark_json
    local theme = create_theme(data, options.color_mode or "truecolor")
    return { header = header(options, theme), footer = footer(options, theme), theme = theme.name }
  end,
})


-- ===========================================================================
-- Transcript presentation — ports of modes/interactive/components/*.ts.
-- Each component is a pure lines-producing function over the theme and the
-- pi.tui markdown/text mechanisms.
-- ===========================================================================

local OSC133_ZONE_START = "\27]133;A\7"
local OSC133_ZONE_END = "\27]133;B\7"
local OSC133_ZONE_FINAL = "\27]133;C\7"

local function osc133_zone(lines)
  if #lines == 0 then return lines end
  lines[1] = OSC133_ZONE_START .. lines[1]
  lines[#lines] = OSC133_ZONE_END .. OSC133_ZONE_FINAL .. lines[#lines]
  return lines
end

local function trim(text)
  return (text or ""):match("^%s*(.-)%s*$")
end

local function append(lines, more)
  for _, line in ipairs(more) do lines[#lines + 1] = line end
  return lines
end

-- Box(paddingX=1, paddingY=1, bg) around already-rendered child lines
-- (pi tui `Box.render`; children were rendered at width - 2).
local function box_lines(child_lines, width, bg)
  if #child_lines == 0 then return {} end
  local out = {}
  local function finish(line)
    local pad = width - pi.tui.visible_width(line)
    if pad > 0 then line = line .. string.rep(" ", pad) end
    return bg(line)
  end
  out[#out + 1] = finish("")
  for _, line in ipairs(child_lines) do out[#out + 1] = finish(" " .. line) end
  out[#out + 1] = finish("")
  return out
end

-- components/user-message.ts
local function user_message_lines(text, width, theme, md_theme)
  local content_width = math.max(1, width - 2)
  local markdown = pi.tui.markdown_render(text, content_width, 0, 0, {
    theme = md_theme,
    color = function(t) return theme:fg("userMessageText", t) end,
    preserve_ordered_list_markers = true,
  })
  local function bg(t) return theme:bg("userMessageBg", t) end
  return osc133_zone(box_lines(markdown, width, bg))
end

-- components/assistant-message.ts
local function assistant_message_lines(message, width, theme, md_theme, options)
  options = options or {}
  local lines = {}
  local content = message.content or {}
  local function visible(part)
    return (part.type == "text" and trim(part.text) ~= "")
      or (part.type == "thinking" and trim(part.thinking) ~= "")
  end
  local has_visible = false
  local has_tool_calls = false
  for _, part in ipairs(content) do
    if visible(part) then has_visible = true end
    if part.type == "toolCall" then has_tool_calls = true end
  end
  if has_visible then lines[#lines + 1] = "" end -- Spacer(1)
  for index, part in ipairs(content) do
    if part.type == "text" and trim(part.text) ~= "" then
      append(lines, pi.tui.markdown_render(trim(part.text), width, 1, 0, { theme = md_theme }))
    elseif part.type == "thinking" and trim(part.thinking) ~= "" then
      local visible_after = false
      for j = index + 1, #content do
        if visible(content[j]) then visible_after = true end
      end
      if options.hide_thinking then
        append(lines, pi.tui.text_render(
          theme:italic(theme:fg("thinkingText", options.hidden_thinking_label or "Thinking...")),
          width, 1, 0))
      else
        append(lines, pi.tui.markdown_render(trim(part.thinking), width, 1, 0, {
          theme = md_theme,
          color = function(t) return theme:fg("thinkingText", t) end,
          italic = true,
        }))
      end
      if visible_after then lines[#lines + 1] = "" end
    end
  end
  if not has_tool_calls then
    if message.stopReason == "aborted" then
      local abort_message = (message.errorMessage and message.errorMessage ~= "Request was aborted")
        and message.errorMessage or "Operation aborted"
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(theme:fg("error", abort_message), width, 1, 0))
    elseif message.stopReason == "error" then
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(
        theme:fg("error", "Error: " .. (message.errorMessage or "Unknown error")), width, 1, 0))
    end
  end
  if not has_tool_calls then return osc133_zone(lines) end
  return lines
end

-- components/branch-summary-message.ts — Box(1, 1, customMessageBg) with
-- a bold [branch] label, then the summary as Markdown when expanded or
-- the keybinding hint row when collapsed.
local function branch_summary_message_lines(message, width, theme, md_theme, expanded)
  local content_width = math.max(1, width - 2)
  local child_lines = {}
  -- Text(label, 0, 0): raw bold escapes inside the customMessageLabel fg.
  append(child_lines, pi.tui.text_render(
    theme:fg("customMessageLabel", "\27[1m[branch]\27[22m"), content_width, 0, 0))
  child_lines[#child_lines + 1] = "" -- Spacer(1)
  if expanded then
    append(child_lines, pi.tui.markdown_render(
      "**Branch Summary**\n\n" .. (message.summary or ""), content_width, 0, 0, {
        theme = md_theme,
        color = function(t) return theme:fg("customMessageText", t) end,
      }))
  else
    append(child_lines, pi.tui.text_render(
      theme:fg("customMessageText", "Branch summary (")
        .. theme:fg("dim", format_key(DEFAULT_KEYS["app.tools.expand"], false))
        .. theme:fg("customMessageText", " to expand)"),
      content_width, 0, 0))
  end
  local function bg(t) return theme:bg("customMessageBg", t) end
  return box_lines(child_lines, width, bg)
end

-- JS Number.prototype.toLocaleString() under node's default en-US locale.
local function to_locale_string(value)
  local formatted = string.format("%d", value)
  while true do
    local replaced
    formatted, replaced = formatted:gsub("^(-?%d+)(%d%d%d)", "%1,%2")
    if replaced == 0 then break end
  end
  return formatted
end

-- components/compaction-summary-message.ts — same customMessageBg box as
-- branch summaries; tokensBefore renders with toLocaleString grouping.
local function compaction_summary_message_lines(message, width, theme, md_theme, expanded)
  local content_width = math.max(1, width - 2)
  local child_lines = {}
  local token_str = to_locale_string(message.tokensBefore or 0)
  append(child_lines, pi.tui.text_render(
    theme:fg("customMessageLabel", "\27[1m[compaction]\27[22m"), content_width, 0, 0))
  child_lines[#child_lines + 1] = "" -- Spacer(1)
  if expanded then
    append(child_lines, pi.tui.markdown_render(
      "**Compacted from " .. token_str .. " tokens**\n\n" .. (message.summary or ""),
      content_width, 0, 0, {
        theme = md_theme,
        color = function(t) return theme:fg("customMessageText", t) end,
      }))
  else
    append(child_lines, pi.tui.text_render(
      theme:fg("customMessageText", "Compacted from " .. token_str .. " tokens (")
        .. theme:fg("dim", format_key(DEFAULT_KEYS["app.tools.expand"], false))
        .. theme:fg("customMessageText", " to expand)"),
      content_width, 0, 0))
  end
  local function bg(t) return theme:bg("customMessageBg", t) end
  return box_lines(child_lines, width, bg)
end

-- utils/ansi.ts stripAnsi + utils/shell.ts sanitizeBinaryOutput +
-- render-utils.ts getTextOutput, as consumed by tool-execution.ts. The
-- tools pack carries the same helpers for its renderers; the spec shares
-- one module, pi-rs shares per-pack copies until packs can share chunks.
local function strip_ansi(text)
  text = text:gsub("\27%][^\7\27]*\7", "")
  text = text:gsub("\27%][^\27]-\27\\", "")
  text = text:gsub("\27%[[%d;:?]*[%a~]", "")
  return text
end

local function sanitize_binary_output(s)
  if not utf8.len(s) then return s end
  local out = {}
  for _, code in utf8.codes(s) do
    local keep
    if code == 0x09 or code == 0x0a or code == 0x0d then
      keep = true
    elseif code <= 0x1f then
      keep = false
    elseif code >= 0xfff9 and code <= 0xfffb then
      keep = false
    else
      keep = true
    end
    if keep then out[#out + 1] = utf8.char(code) end
  end
  return table.concat(out)
end

local function tool_text_output(result, _show_images)
  if not result then return "" end
  local text_parts, image_parts = {}, {}
  for _, block in ipairs(result.content or {}) do
    if block.type == "text" then
      text_parts[#text_parts + 1] = sanitize_binary_output(strip_ansi(block.text or "")):gsub("\r", "")
    elseif block.type == "image" then
      -- imageFallback indicator; capability-less terminal default.
      image_parts[#image_parts + 1] = "[Image: [" .. (block.mimeType or "image/unknown") .. "]]"
    end
  end
  local output = table.concat(text_parts, "\n")
  if #image_parts > 0 then
    local indicators = table.concat(image_parts, "\n")
    output = output ~= "" and (output .. "\n" .. indicators) or indicators
  end
  return output
end

-- components/tool-execution.ts — the ToolExecutionComponent as a pure
-- lines-producing function. Tool rows carry the component's inputs
-- (args, result, state flags) plus a persistent render_state table (the
-- spec's ToolRenderContext.state). Renderers come from the resolved tool
-- registry (the spec's extension-over-builtin merge is the registry's
-- first-registration-wins view) and return `function(width) -> lines`.
local function find_tool_definition(name)
  for _, def in ipairs(pi.registered_tools()) do
    if def.name == name then return def end
  end
  return nil
end

local function tool_execution_lines(tool, width, theme, opts)
  opts = opts or {}
  local is_partial = tool.state == "pending"
  local is_error = tool.state == "error"
  local bg_key = is_partial and "toolPendingBg" or (is_error and "toolErrorBg" or "toolSuccessBg")
  local function bg(text) return theme:bg(bg_key, text) end
  local show_images = opts.show_images ~= false

  local def = find_tool_definition(tool.name)
  if def == nil then
    -- Generic fallback (formatToolExecution): Spacer(1) + Text(title +
    -- pretty-printed args + text output, 1, 1, statusBg). pi.json.encode
    -- replays the wire key order recorded at the JSON→Lua boundary, so
    -- args print exactly as pi's JSON.stringify(args, null, 2).
    local text = theme:fg("toolTitle", theme:bold(tool.name or "tool"))
    local args_json = tool.args ~= nil and pi.json.encode(tool.args, true) or ""
    if args_json ~= "" then text = text .. "\n\n" .. args_json end
    local output = tool_text_output(tool.result, show_images)
    if output ~= "" then text = text .. "\n" .. output end
    local lines = { "" } -- Spacer(1)
    for _, line in ipairs(pi.tui.text_render(text, width, 1, 1)) do
      lines[#lines + 1] = bg(line)
    end
    return lines
  end

  tool.render_state = tool.render_state or {}
  local context = {
    args = tool.args,
    toolCallId = tool.toolCallId,
    -- Frames re-render from state each pass; invalidate is a no-op seam.
    invalidate = function() end,
    state = tool.render_state,
    cwd = opts.cwd or pi.cwd(),
    executionStarted = tool.executionStarted or false,
    argsComplete = tool.argsComplete or false,
    isPartial = is_partial,
    expanded = opts.expanded or false,
    showImages = show_images,
    isError = is_error,
    now_ms = opts.now_ms or pi.monotonic_ms,
  }

  local components = {}
  if def.renderCall == nil then
    components[#components + 1] = function(w)
      return pi.tui.text_render(theme:fg("toolTitle", theme:bold(tool.name)), w, 0, 0)
    end
  else
    local ok, component = pcall(def.renderCall, tool.args, theme, context)
    if ok and component ~= nil then
      components[#components + 1] = component
    else
      components[#components + 1] = function(w)
        return pi.tui.text_render(theme:fg("toolTitle", theme:bold(tool.name)), w, 0, 0)
      end
    end
  end
  if tool.result then
    local function result_fallback()
      local output = tool_text_output(tool.result, show_images)
      if output == "" then return nil end
      return function(w)
        return pi.tui.text_render(theme:fg("toolOutput", output), w, 0, 0)
      end
    end
    if def.renderResult == nil then
      components[#components + 1] = result_fallback()
    else
      local ok, component = pcall(def.renderResult,
        { content = tool.result.content, details = tool.result.details },
        { expanded = context.expanded, isPartial = is_partial }, theme, context)
      if ok and component ~= nil then
        components[#components + 1] = component
      else
        components[#components + 1] = result_fallback()
      end
    end
  end

  local shell = def.renderShell or "default"
  if shell == "self" then
    -- Self-framing tools: [Spacer implied by leading ""] + content only
    -- when the container produced lines (image blocks: images milestone).
    local content = {}
    for _, component in ipairs(components) do append(content, component(width)) end
    if #content == 0 then return {} end
    local lines = { "" }
    append(lines, content)
    return lines
  end

  -- Default shell: Spacer(1) + Box(1, 1, statusBg) around the renderers.
  local content_width = math.max(1, width - 2)
  local child_lines = {}
  for _, component in ipairs(components) do
    for _, line in ipairs(component(content_width)) do
      child_lines[#child_lines + 1] = " " .. line
    end
  end
  local lines = { "" } -- Spacer(1)
  if #child_lines == 0 then return lines end
  local function apply(line)
    local pad = width - pi.tui.visible_width(line)
    if pad > 0 then line = line .. string.rep(" ", pad) end
    return bg(line)
  end
  lines[#lines + 1] = apply("")
  for _, line in ipairs(child_lines) do lines[#lines + 1] = apply(line) end
  lines[#lines + 1] = apply("")
  return lines
end

-- Easter-egg components: armin.ts, daxnuts.ts, and
-- earendil-announcement.ts. Animation state stays in transcript rows so the
-- regular frontend render path and timer pump use the same Lua policy.
ARMIN_WIDTH, ARMIN_HEIGHT = 31, 36
ARMIN_BITS = {
  0xff, 0xff, 0xff, 0x7f, 0xff, 0xf0, 0xff, 0x7f, 0xff, 0xed, 0xff, 0x7f, 0xff, 0xdb, 0xff, 0x7f,
  0xff, 0xb7, 0xff, 0x7f, 0xff, 0x77, 0xfe, 0x7f, 0x3f, 0xf8, 0xfe, 0x7f, 0xdf, 0xff, 0xfe, 0x7f,
  0xdf, 0x3f, 0xfc, 0x7f, 0x9f, 0xc3, 0xfb, 0x7f, 0x6f, 0xfc, 0xf4, 0x7f, 0xf7, 0x0f, 0xf7, 0x7f,
  0xf7, 0xff, 0xf7, 0x7f, 0xf7, 0xff, 0xe3, 0x7f, 0xf7, 0x07, 0xe8, 0x7f, 0xef, 0xf8, 0x67, 0x70,
  0x0f, 0xff, 0xbb, 0x6f, 0xf1, 0x00, 0xd0, 0x5b, 0xfd, 0x3f, 0xec, 0x53, 0xc1, 0xff, 0xef, 0x57,
  0x9f, 0xfd, 0xee, 0x5f, 0x9f, 0xfc, 0xae, 0x5f, 0x1f, 0x78, 0xac, 0x5f, 0x3f, 0x00, 0x50, 0x6c,
  0x7f, 0x00, 0xdc, 0x77, 0xff, 0xc0, 0x3f, 0x78, 0xff, 0x01, 0xf8, 0x7f, 0xff, 0x03, 0x9c, 0x78,
  0xff, 0x07, 0x8c, 0x7c, 0xff, 0x0f, 0xce, 0x78, 0xff, 0xff, 0xcf, 0x7f, 0xff, 0xff, 0xcf, 0x78,
  0xff, 0xff, 0xdf, 0x78, 0xff, 0xff, 0xdf, 0x7d, 0xff, 0xff, 0x3f, 0x7e, 0xff, 0xff, 0xff, 0x7f,
}
ARMIN_DISPLAY_HEIGHT = math.ceil(ARMIN_HEIGHT / 2)
ARMIN_EFFECTS = { "typewriter", "scanline", "rain", "fade", "crt", "glitch", "dissolve" }

function armin_empty_grid()
  local grid = {}
  for row = 1, ARMIN_DISPLAY_HEIGHT do
    grid[row] = {}
    for x = 1, ARMIN_WIDTH do grid[row][x] = " " end
  end
  return grid
end

function armin_final_grid()
  local grid = armin_empty_grid()
  local bytes_per_row = math.ceil(ARMIN_WIDTH / 8)
  local function pixel(x, y)
    if y >= ARMIN_HEIGHT then return false end
    local byte = ARMIN_BITS[y * bytes_per_row + math.floor(x / 8) + 1]
    return ((byte >> (x % 8)) & 1) == 0
  end
  for row = 0, ARMIN_DISPLAY_HEIGHT - 1 do
    for x = 0, ARMIN_WIDTH - 1 do
      local upper, lower = pixel(x, row * 2), pixel(x, row * 2 + 1)
      grid[row + 1][x + 1] = upper and (lower and "█" or "▀") or (lower and "▄" or " ")
    end
  end
  return grid
end

function armin_shuffle(positions, random)
  for i = #positions, 2, -1 do
    local j = math.floor(random() * i) + 1
    positions[i], positions[j] = positions[j], positions[i]
  end
end

function new_armin_row(effect, random)
  random = random or math.random
  effect = effect or ARMIN_EFFECTS[math.floor(random() * #ARMIN_EFFECTS) + 1]
  local row = { kind = "armin", effect = effect, final = armin_final_grid(),
    current = armin_empty_grid(), effect_state = {}, version = 0, random = random }
  if effect == "typewriter" then row.effect_state.pos = 0
  elseif effect == "scanline" then row.effect_state.row = 0
  elseif effect == "rain" then
    row.effect_state.drops = {}
    for x = 1, ARMIN_WIDTH do
      row.effect_state.drops[x] = {
        y = -math.floor(random() * ARMIN_DISPLAY_HEIGHT * 2), settled = 0 }
    end
  elseif effect == "fade" or effect == "dissolve" then
    local positions = {}
    for y = 1, ARMIN_DISPLAY_HEIGHT do
      for x = 1, ARMIN_WIDTH do positions[#positions + 1] = { y, x } end
    end
    armin_shuffle(positions, random)
    row.effect_state = { positions = positions, idx = 1 }
    if effect == "dissolve" then
      local chars = { " ", "░", "▒", "▓", "█", "▀", "▄" }
      for y = 1, ARMIN_DISPLAY_HEIGHT do
        for x = 1, ARMIN_WIDTH do
          row.current[y][x] = chars[math.floor(random() * #chars) + 1]
        end
      end
    end
  elseif effect == "crt" then row.effect_state.expansion = 0
  elseif effect == "glitch" then row.effect_state = { phase = 0, glitchFrames = 8 } end
  return row
end

function tick_armin(row)
  local state, done = row.effect_state, false
  if row.effect == "typewriter" then
    for _ = 1, 3 do
      local y, x = math.floor(state.pos / ARMIN_WIDTH), state.pos % ARMIN_WIDTH
      if y >= ARMIN_DISPLAY_HEIGHT then done = true break end
      row.current[y + 1][x + 1] = row.final[y + 1][x + 1]
      state.pos = state.pos + 1
    end
  elseif row.effect == "scanline" then
    if state.row >= ARMIN_DISPLAY_HEIGHT then done = true
    else
      for x = 1, ARMIN_WIDTH do row.current[state.row + 1][x] = row.final[state.row + 1][x] end
      state.row = state.row + 1
    end
  elseif row.effect == "rain" then
    done, row.current = true, armin_empty_grid()
    for x = 1, ARMIN_WIDTH do
      local drop = state.drops[x]
      for y = ARMIN_DISPLAY_HEIGHT, ARMIN_DISPLAY_HEIGHT - drop.settled + 1, -1 do
        if y >= 1 then row.current[y][x] = row.final[y][x] end
      end
      if drop.settled < ARMIN_DISPLAY_HEIGHT then
        done = false
        local target = -1
        for y = ARMIN_DISPLAY_HEIGHT - drop.settled, 1, -1 do
          if row.final[y][x] ~= " " then target = y - 1 break end
        end
        drop.y = drop.y + 1
        if drop.y >= 0 and drop.y < ARMIN_DISPLAY_HEIGHT then
          if target >= 0 and drop.y >= target then
            drop.settled = ARMIN_DISPLAY_HEIGHT - target
            drop.y = -math.floor(row.random() * 5) - 1
          else row.current[drop.y + 1][x] = "▓" end
        end
      end
    end
  elseif row.effect == "fade" or row.effect == "dissolve" then
    local count = row.effect == "fade" and 15 or 20
    for _ = 1, count do
      if state.idx > #state.positions then done = true break end
      local position = state.positions[state.idx]
      row.current[position[1]][position[2]] = row.final[position[1]][position[2]]
      state.idx = state.idx + 1
    end
  elseif row.effect == "crt" then
    row.current = armin_empty_grid()
    local middle = math.floor(ARMIN_DISPLAY_HEIGHT / 2)
    local top, bottom = middle - state.expansion, middle + state.expansion
    for y = math.max(0, top), math.min(ARMIN_DISPLAY_HEIGHT - 1, bottom) do
      for x = 1, ARMIN_WIDTH do row.current[y + 1][x] = row.final[y + 1][x] end
    end
    state.expansion = state.expansion + 1
    done = state.expansion > ARMIN_DISPLAY_HEIGHT
  elseif row.effect == "glitch" then
    if state.phase < state.glitchFrames then
      row.current = {}
      for y = 1, ARMIN_DISPLAY_HEIGHT do
        local source = row.final[y]
        local copy, offset = {}, math.floor(row.random() * 7) - 3
        for x = 1, ARMIN_WIDTH do copy[x] = source[x] end
        if row.random() < 0.3 then
          local shifted = {}
          -- JS slice(offset).concat(slice(0, offset)), including negative offsets.
          local split = offset >= 0 and (offset + 1) or (ARMIN_WIDTH + offset + 1)
          for x = split, ARMIN_WIDTH do shifted[#shifted + 1] = copy[x] end
          local finish = offset >= 0 and offset or (ARMIN_WIDTH + offset)
          for x = 1, finish do shifted[#shifted + 1] = copy[x] end
          copy = shifted
        end
        if row.random() < 0.2 then
          local swap = math.floor(row.random() * ARMIN_DISPLAY_HEIGHT) + 1
          copy = {}
          for x = 1, ARMIN_WIDTH do copy[x] = row.final[swap][x] end
        end
        row.current[y] = copy
      end
      state.phase = state.phase + 1
    else
      row.current = {}
      for y = 1, ARMIN_DISPLAY_HEIGHT do
        row.current[y] = {}
        for x = 1, ARMIN_WIDTH do row.current[y][x] = row.final[y][x] end
      end
      done = true
    end
  else done = true end
  row.version = row.version + 1
  return done
end

function armin_lines(row, width, theme)
  local lines, available = {}, width - 1
  for y = 1, ARMIN_DISPLAY_HEIGHT do
    local count = math.max(0, math.min(ARMIN_WIDTH, available))
    local clipped = table.concat(row.current[y], "", 1, count)
    lines[#lines + 1] = " " .. theme:fg("accent", clipped)
      .. string.rep(" ", math.max(0, width - 1 - count))
  end
  local message = "ARMIN SAYS HI"
  lines[#lines + 1] = " " .. theme:fg("accent", message)
    .. string.rep(" ", math.max(0, width - 1 - #message))
  return lines
end

function dax_rgb(r, g, b, background)
  return "\27[" .. (background and "48" or "38") .. ";2;" .. r .. ";" .. g .. ";" .. b .. "m"
end

function dax_image_lines()
  local lines = {}
  for y = 0, 30, 2 do
    local line = ""
    for x = 0, 31 do
      local top = (y * 32 + x) * 6 + 1
      local bottom = ((y + 1) * 32 + x) * 6 + 1
      local tr = tonumber(DAX_HEX:sub(top, top + 1), 16)
      local tg = tonumber(DAX_HEX:sub(top + 2, top + 3), 16)
      local tb = tonumber(DAX_HEX:sub(top + 4, top + 5), 16)
      local br = tonumber(DAX_HEX:sub(bottom, bottom + 1), 16)
      local bg = tonumber(DAX_HEX:sub(bottom + 2, bottom + 3), 16)
      local bb = tonumber(DAX_HEX:sub(bottom + 4, bottom + 5), 16)
      line = line .. dax_rgb(br, bg, bb, false) .. dax_rgb(tr, tg, tb, true) .. "▄"
    end
    lines[#lines + 1] = line .. "\27[0m"
  end
  return lines
end
DAX_IMAGE_LINES = dax_image_lines()

function center_ansi(text, width)
  return string.rep(" ", math.max(0, math.floor((width - pi.tui.visible_width(text)) / 2))) .. text
end

function daxnuts_lines(row, width, theme)
  local lines = { "" }
  local max_ticks = 25
  local revealed = math.min(#DAX_IMAGE_LINES, math.floor((row.tick / max_ticks) * (#DAX_IMAGE_LINES + 3)))
  for index, image_line in ipairs(DAX_IMAGE_LINES) do
    if index <= revealed then lines[#lines + 1] = center_ansi(image_line, width)
    elseif index == revealed + 1 then
      lines[#lines + 1] = center_ansi(dax_rgb(100, 200, 255, false)
        .. string.rep("▓", 32) .. "\27[0m", width)
    else lines[#lines + 1] = center_ansi(string.rep(" ", 32), width) end
  end
  lines[#lines + 1] = ""
  local text_phase = math.max(0, row.tick - max_ticks * 0.6)
  if text_phase > 0 or row.tick >= max_ticks then
    lines[#lines + 1] = center_ansi(theme:fg("accent", "Free Kimi K2.5 via OpenCode Zen"), width)
    lines[#lines + 1] = center_ansi(theme:fg("success", '"Powered by daxnuts"'), width)
    lines[#lines + 1] = center_ansi(theme:fg("muted", "— @thdxr"), width)
  else for _ = 1, 3 do lines[#lines + 1] = "" end end
  lines[#lines + 1] = ""
  if text_phase > 2 or row.tick >= max_ticks then
    lines[#lines + 1] = center_ansi(theme:fg("dim", "Try OpenCode"), width)
    lines[#lines + 1] = center_ansi(theme:fg("mdLink", "https://mistral.ai/news/mistral-vibe-2-0"), width)
  else
    lines[#lines + 1] = ""
    lines[#lines + 1] = ""
  end
  lines[#lines + 1] = ""
  return lines
end

function earendil_image_lines(width, theme)
  local caps = pi.tui.terminal_capabilities()
  if not caps.images then
    return { theme:fg("muted", pi.tui.image_fallback("image/png", 640, 537, "clankolas.png")) }
  end
  local result = pi.tui.image_render(caps.images, CLANKOLAS_BASE64,
    { width_px = 640, height_px = 537 },
    { max_width_cells = math.max(1, math.min(width - 2, 56)), move_cursor = false })
  local lines = {}
  if caps.images == "kitty" then
    lines[1] = result.sequence
    for _ = 2, result.rows do lines[#lines + 1] = "" end
  else
    for _ = 1, result.rows - 1 do lines[#lines + 1] = "" end
    local offset = result.rows - 1
    lines[#lines + 1] = (offset > 0 and ("\27[" .. offset .. "A") or "") .. result.sequence
  end
  return lines
end

function earendil_lines(width, theme)
  local border = theme:fg("accent", string.rep("─", math.max(1, width)))
  local lines = { border }
  append(lines, pi.tui.text_render(theme:bold(theme:fg("accent", "pi has joined Earendil")), width, 1, 0))
  lines[#lines + 1] = ""
  append(lines, pi.tui.text_render(theme:fg("muted", "Read the blog post:"), width, 1, 0))
  append(lines, pi.tui.text_render(theme:fg("mdLink",
    "https://mariozechner.at/posts/2026-04-08-ive-sold-out/"), width, 1, 0))
  lines[#lines + 1] = ""
  append(lines, earendil_image_lines(width, theme))
  lines[#lines + 1] = ""
  lines[#lines + 1] = border
  return lines
end

local function transcript_lines(state, width)
  local lines = {}

  local tool_opts = {
    cwd = state.cwd,
    expanded = state.tools_expanded or false,
    now_ms = state.now_ms,
  }
  for _, item in ipairs(state.transcript) do
    if item.kind == "armin" then
      append(lines, armin_lines(item, width, state.theme))
    elseif item.kind == "daxnuts" then
      append(lines, daxnuts_lines(item, width, state.theme))
    elseif item.kind == "earendil" then
      append(lines, earendil_lines(width, state.theme))
    elseif item.kind == "user" then
      -- interactive-mode.ts addMessageToChat "user": Spacer(1) before the
      -- message whenever the chat container already has content.
      if #lines > 0 then lines[#lines + 1] = "" end
      append(lines, user_message_lines(item.text, width, state.theme, state.md_theme))
    elseif item.kind == "assistant" then
      append(lines, assistant_message_lines(item.message, width, state.theme, state.md_theme,
        { hide_thinking = state.hide_thinking_block or false }))
    elseif item.kind == "tool" then
      append(lines, tool_execution_lines(item, width, state.theme, tool_opts))
    elseif item.kind == "bash" then
      -- interactive-mode.ts addMessageToChat "bashExecution" /
      -- handleBashCommand: the component frames itself (leading Spacer).
      append(lines, bash_execution_lines(item, width, state.theme))
    elseif item.kind == "branch_summary" then
      -- interactive-mode.ts "branchSummary": Spacer(1) + the component
      -- (expanded follows the shared toolOutputExpanded toggle).
      lines[#lines + 1] = ""
      append(lines, branch_summary_message_lines(
        item.message, width, state.theme, state.md_theme, state.tools_expanded or false))
    elseif item.kind == "compaction_summary" then
      -- interactive-mode.ts "compactionSummary": Spacer(1) + the component.
      lines[#lines + 1] = ""
      append(lines, compaction_summary_message_lines(
        item.message, width, state.theme, state.md_theme, state.tools_expanded or false))
    elseif item.kind == "spacer" then
      -- chatContainer.addChild(new Spacer(1)) rows mounted directly
      -- (the summarize-branch flow).
      lines[#lines + 1] = ""
    elseif item.kind == "status" then
      -- showStatus: Spacer(1) + Text(dim message, 1, 0)
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(state.theme:fg("dim", item.text), width, 1, 0))
    elseif item.kind == "info_block" then
      -- /changelog and /hotkeys: Spacer + DynamicBorder + title + Spacer +
      -- Markdown(1,1) + DynamicBorder.
      lines[#lines + 1] = ""
      lines[#lines + 1] = state.theme:fg("border", string.rep("─", math.max(1, width)))
      append(lines, pi.tui.text_render(
        state.theme:bold(state.theme:fg("accent", item.title)), width, 1, 0))
      lines[#lines + 1] = ""
      append(lines, pi.tui.markdown_render(item.markdown, width, 1, 1, { theme = state.md_theme }))
      lines[#lines + 1] = state.theme:fg("border", string.rep("─", math.max(1, width)))
    elseif item.kind == "startup_changelog" then
      -- showStartupNoticesIfNeeded: DynamicBorder + condensed Text or the
      -- What's New title/Spacer/Markdown/Spacer + DynamicBorder.
      lines[#lines + 1] = state.theme:fg("border", string.rep("─", math.max(1, width)))
      if item.collapsed then
        append(lines, pi.tui.text_render("Updated to v" .. item.latest_version .. ". Use "
          .. state.theme:bold("/changelog") .. " to view full changelog.", width, 1, 0))
      else
        append(lines, pi.tui.text_render(
          state.theme:bold(state.theme:fg("accent", "What's New")), width, 1, 0))
        lines[#lines + 1] = ""
        append(lines, pi.tui.markdown_render(item.markdown, width, 1, 0, { theme = state.md_theme }))
        lines[#lines + 1] = ""
      end
      lines[#lines + 1] = state.theme:fg("border", string.rep("─", math.max(1, width)))
    elseif item.kind == "update_available" then
      -- showNewVersionNotification: Spacer + warning border/title/action,
      -- optional muted Markdown note, changelog line, warning border.
      lines[#lines + 1] = ""
      lines[#lines + 1] = state.theme:fg("warning", string.rep("─", math.max(1, width)))
      local action = state.theme:fg("accent", state.app_name .. " update")
      local heading = state.theme:bold(state.theme:fg("warning", "Update Available"))
      local instruction = state.theme:fg("muted", "New version " .. item.version
        .. " is available. Run ") .. action
      append(lines, pi.tui.text_render(heading .. "\n" .. instruction, width, 1, 0))
      if item.note and trim(item.note) ~= "" then
        lines[#lines + 1] = ""
        append(lines, pi.tui.markdown_render(trim(item.note), width, 1, 0, {
          theme = state.md_theme,
          color = function(text) return state.theme:fg("muted", text) end,
        }))
        lines[#lines + 1] = ""
      end
      local changelog_url = "https://pi.dev/changelog"
      local changelog_link = state.theme:fg("accent", changelog_url)
      if pi.tui.terminal_capabilities().hyperlinks then
        changelog_link = pi.tui.hyperlink(state.theme:fg("accent", "open changelog"), changelog_url)
      end
      append(lines, pi.tui.text_render(
        state.theme:fg("muted", "Changelog: ") .. changelog_link, width, 1, 0))
      lines[#lines + 1] = state.theme:fg("warning", string.rep("─", math.max(1, width)))
    elseif item.kind == "text" then
      -- Pre-styled chat rows (interactive-mode.ts chatContainer.addChild
      -- of Spacer(1) + Text(content, 1, paddingY)): /name, /session, and
      -- the /new confirmation.
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(item.text, width, 1, item.padding_y or 0))
    elseif item.kind == "error" then
      -- showError: Spacer(1) + Text(error "Error: ...", 1, 0) + Spacer(1)
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(state.theme:fg("error", item.text), width, 1, 0))
      lines[#lines + 1] = ""
    elseif item.kind == "error_text" then
      -- Raw error rows (auto-compaction failures): Spacer(1) +
      -- Text(error message, 1, 0) — no "Error: " prefix, no trailing
      -- spacer (interactive-mode.ts compaction_end).
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(state.theme:fg("error", item.text), width, 1, 0))
    elseif item.kind == "warning" then
      -- showWarning: Spacer(1) + Text(warning "Warning: ...", 1, 0)
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(state.theme:fg("warning", item.text), width, 1, 0))
    elseif item.kind == "warning_raw" then
      append(lines, pi.tui.text_render(state.theme:fg("warning", item.text), width, 1, 0))
    end
  end
  if state.streaming_message then
    append(lines, assistant_message_lines(state.streaming_message, width, state.theme, state.md_theme,
      { hide_thinking = state.hide_thinking_block or false }))
  end
  return lines
end

-- JS String.length (UTF-16 code units) for the chars/4 token heuristic.
-- components/footer.ts render(): cumulative usage across ALL session
-- entries — not the context path, so abandoned tree branches still
-- count — plus the latest entry's cache hit rate, and agent-session.ts
-- getContextUsage() over the live context (no compaction boundary yet —
-- item 6).
local function footer_agent_data(state)
  local agent_state = state.agent:get_state()
  local totals = { input = 0, output = 0, cache_read = 0, cache_write = 0, cost = 0 }
  local latest_cache_hit_rate = nil
  for _, entry in ipairs(state.session_manager:get_entries()) do
    if entry.type == "message" and entry.message.role == "assistant" then
      local u = entry.message.usage or {}
      totals.input = totals.input + (u.input or 0)
      totals.output = totals.output + (u.output or 0)
      totals.cache_read = totals.cache_read + (u.cacheRead or 0)
      totals.cache_write = totals.cache_write + (u.cacheWrite or 0)
      totals.cost = totals.cost + ((u.cost and u.cost.total) or 0)
      local prompt = (u.input or 0) + (u.cacheRead or 0) + (u.cacheWrite or 0)
      latest_cache_hit_rate = prompt > 0 and ((u.cacheRead or 0) / prompt) * 100 or false
    end
  end
  local context_percent = nil
  local context_window = state.model.contextWindow or 0
  if context_window > 0 then
    -- agent-session.ts getContextUsage over the compaction.ts
    -- token-estimation port (utils/compaction.lua, written once).
    context_percent = (compaction_lib.estimate_context_tokens(agent_state.messages).tokens
      / context_window) * 100
  else
    -- getContextUsage() undefined: the footer falls back to 0 (footer.ts
    -- `contextUsage?.percent ?? 0` with undefined !== null).
    context_percent = 0
  end
  return totals, latest_cache_hit_rate, context_percent
end

local function content_text(message, wanted_type)
  local out = {}
  for _, part in ipairs((message and message.content) or {}) do
    if part.type == wanted_type then
      out[#out + 1] = part.text or part.thinking or ""
    end
  end
  return table.concat(out)
end
local function message_text(message) return content_text(message, "text") end

-- ===========================================================================
-- Selector overlay machinery — interactive-mode.ts showSelector plus the
-- pi-tui pieces the selectors compose (DynamicBorder, TruncatedText,
-- fuzzyFilter, Input). components/oauth-selector.ts is the template; its
-- auth bridge and /login wiring land with PLAN 3a.2.
-- ===========================================================================

-- pi-tui keybindings.ts tui.select.* defaults.
local SELECT_KEYS = {
  up = "up",
  down = "down",
  pageUp = "pageUp",
  pageDown = "pageDown",
  confirm = "enter",
  cancel = { "escape", "ctrl+c" },
}

-- components/dynamic-border.ts
local function dynamic_border_line(theme, width, color)
  color = color or function(text) return theme:fg("border", text) end
  return color(string.rep("─", math.max(1, width)))
end

-- pi-tui TruncatedText(text, 1, 0) rendered at width.
local function truncated_line(text, width)
  return pi.tui.truncated_text(text, 1, 0):render(width)
end

-- components/oauth-selector.ts — presentation and input policy. Data
-- callbacks are injected: get_credential(id) -> {type=...}|nil mirrors
-- authStorage.get, get_auth_status(id) -> {source=..., label=...}|nil
-- mirrors authStorage.getAuthStatus.
local function oauth_selector(opts)
  local theme = opts.theme
  local self = {
    mode = opts.mode,
    all = opts.providers or {},
    selected = 0, -- 0-based: the spec's windowing/clamping math copied 1:1
    search = pi.tui.input(),
    focused = false,
  }
  self.filtered = self.all

  -- formatStatusIndicator
  local function status_indicator(provider)
    local credential = opts.get_credential(provider.id)
    if credential and credential.type == provider.authType then
      return theme:fg("success", " ✓ configured")
    end
    if credential then
      local label = credential.type == "oauth" and "subscription configured" or "API key configured"
      return theme:fg("muted", " • ") .. theme:fg("warning", label)
    end
    if provider.authType ~= "api_key" then return theme:fg("muted", " • unconfigured") end
    local status = opts.get_auth_status(provider.id) or {}
    if status.source == "environment" then
      return theme:fg("success", " ✓ env: " .. (status.label or "API key"))
    elseif status.source == "runtime" then
      return theme:fg("success", " ✓ runtime API key")
    elseif status.source == "fallback" then
      return theme:fg("success", " ✓ custom API key")
    elseif status.source == "models_json_key" then
      return theme:fg("success", " ✓ key in models.json")
    elseif status.source == "models_json_command" then
      return theme:fg("success", " ✓ command in models.json")
    end
    return theme:fg("muted", " • unconfigured")
  end

  -- filterProviders
  local function filter(query)
    if query ~= "" then
      self.filtered = pi.tui.fuzzy_filter(self.all, query, function(provider)
        return provider.name .. " " .. provider.id .. " " .. provider.authType
      end)
    else
      self.filtered = self.all
    end
    self.selected = math.max(0, math.min(self.selected, math.max(0, #self.filtered - 1)))
  end

  function self:set_focused(focused)
    self.focused = focused
    self.search:set_focused(focused)
  end

  function self:handle_input(data)
    if binding_matches(data, SELECT_KEYS.up) then
      if #self.filtered == 0 then return end
      self.selected = math.max(0, self.selected - 1)
    elseif binding_matches(data, SELECT_KEYS.down) then
      if #self.filtered == 0 then return end
      self.selected = math.min(#self.filtered - 1, self.selected + 1)
    elseif binding_matches(data, SELECT_KEYS.confirm) then
      local provider = self.filtered[self.selected + 1]
      if provider then opts.on_select(provider.id) end
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      opts.on_cancel()
    else
      self.search:input(data)
      filter(self.search:value())
    end
  end

  -- updateList, produced at render time from the same state.
  local function list_lines(width)
    local lines = {}
    local count = #self.filtered
    local max_visible = 8
    local start_index = math.max(0,
      math.min(self.selected - math.floor(max_visible / 2), count - max_visible))
    local end_index = math.min(start_index + max_visible, count)
    for i = start_index, end_index - 1 do
      local provider = self.filtered[i + 1]
      local line
      if i == self.selected then
        line = theme:fg("accent", "→ ") .. theme:fg("accent", provider.name) .. status_indicator(provider)
      else
        line = "  " .. theme:fg("text", provider.name) .. status_indicator(provider)
      end
      append(lines, truncated_line(line, width))
    end
    if start_index > 0 or end_index < count then
      append(lines, truncated_line(
        theme:fg("muted", "  (" .. (self.selected + 1) .. "/" .. count .. ")"), width))
    end
    if count == 0 then
      local message
      if #self.all == 0 then
        message = self.mode == "login" and "No providers available"
          or "No providers logged in. Use /login first."
      else
        message = "No matching providers"
      end
      append(lines, truncated_line(theme:fg("muted", "  " .. message), width))
    end
    return lines
  end

  function self:render(width)
    local title = self.mode == "login" and "Select provider to configure:"
      or "Select provider to logout:"
    local border = dynamic_border_line(theme, width)
    local lines = { border, "" }
    append(lines, truncated_line(theme:fg("accent", theme:bold(title)), width))
    lines[#lines + 1] = ""
    append(lines, self.search:render(width))
    lines[#lines + 1] = ""
    append(lines, list_lines(width))
    lines[#lines + 1] = ""
    lines[#lines + 1] = border
    return lines
  end

  filter("")
  return self
end

-- interactive-mode.ts showSelector: the selector replaces the editor in the
-- editor slot and takes focus; done() restores the editor and its focus.
-- create(done) may return (component) or (component, focus).
local function show_selector(state, create)
  local component, focus = create(function()
    state.selector = nil
    state.set_editor_focus(true)
  end)
  focus = focus or component
  state.set_editor_focus(false)
  focus:set_focused(true)
  state.selector = component
end

-- The editor-slot swap outside the done() protocol (interactive-mode.ts
-- mounts login dialogs with editorContainer.clear()/addChild + setFocus).
local function mount_in_editor_slot(state, component)
  state.set_editor_focus(false)
  component:set_focused(true)
  state.selector = component
end

local function restore_editor(state)
  state.selector = nil
  state.set_editor_focus(true)
end

-- ===========================================================================
-- /login and /logout — ports of interactive-mode.ts's auth surface over the
-- pi.auth mechanism bindings: login-dialog.ts, extension-selector.ts, the
-- selector wiring, and completeProviderAuthentication's reachable slice.
-- ===========================================================================

-- components/keybinding-hints.ts keyText/keyHint over the tui.select.*
-- defaults (user-configured keybindings arrive with the interaction-shell
-- milestone; the darwin alt→option rename has no alt keys here).
local function select_key_text(action)
  local binding = SELECT_KEYS[action]
  if type(binding) == "table" then binding = table.concat(binding, "/") end
  return format_key(binding or "", false)
end

local function select_key_hint(theme, action, description)
  return theme:fg("dim", select_key_text(action)) .. theme:fg("muted", " " .. description)
end

local function raw_key_hint(theme, key, description)
  return theme:fg("dim", format_key(key, false)) .. theme:fg("muted", " " .. description)
end

-- interactive-mode.ts showStatus: Spacer + dim Text appended to the chat;
-- a status that is still the last chat entry is updated in place.
local function show_status(state, message)
  local transcript = state.transcript
  local last = transcript[#transcript]
  if last ~= nil and last == state.last_status then
    last.text = message
    return
  end
  local row = { kind = "status", text = message }
  transcript[#transcript + 1] = row
  state.last_status = row
end

local function show_error(state, message)
  state.transcript[#state.transcript + 1] = { kind = "error", text = "Error: " .. message }
end

local function show_warning(state, message)
  state.transcript[#state.transcript + 1] = { kind = "warning", text = "Warning: " .. message }
end

-- ===========================================================================
-- Bash mode — ports of components/bash-execution.ts and the
-- interactive-mode.ts `!`/`!!` surface (isBashMode border, handleBash-
-- Command, flushPendingBashComponents) over the utils/bash-executor.lua
-- fragment (core/bash-executor.ts) and the tools pack's shared truncation
-- exports (PLAN 7.1). The session half (AgentSession.executeBash /
-- recordBashResult / abortBash) binds in bind_session_runtime.
-- ===========================================================================

-- theme.ts getThinkingBorderColor's color keys (getBashModeBorderColor is
-- the bashMode key).
THINKING_BORDER_COLOR_KEYS = {
  off = "thinkingOff", minimal = "thinkingMinimal", low = "thinkingLow",
  medium = "thinkingMedium", high = "thinkingHigh", xhigh = "thinkingXhigh",
}

-- interactive-mode.ts isBashMode: maintained by onChange as
-- `text.trimStart().startsWith("!")`, with one stateful exception — the
-- submit handler forces it false after an awaited bash command settles,
-- and that override holds until the next text change (bug-for-bug: a
-- warning-restored "!…" editor keeps a non-bash border until typing
-- resumes). pi-rs recomputes on text change at the observation points and
-- carries the override through the unchanged-text window.
function sync_bash_mode(state)
  if not state.editor then return false end
  local text = state.editor.editor:get_text()
  if text ~= state.bash_mode_text then
    state.bash_mode_text = text
    state.is_bash_mode = (text:gsub("^%s+", "")):sub(1, 1) == "!"
  end
  return state.is_bash_mode or false
end

-- interactive-mode.ts updateEditorBorderColor, applied before each frame
-- (the spec calls it from onChange; the border is only observable at
-- render time).
function sync_editor_border(state)
  if not state.editor then return end
  local key
  if sync_bash_mode(state) then
    key = "bashMode"
  else
    key = THINKING_BORDER_COLOR_KEYS[state.thinking_level or "off"] or "thinkingOff"
  end
  if state.editor_border_key ~= key then
    state.editor_border_key = key
    state.editor.editor:set_border_style(state.theme.fg_codes[key], "\27[39m")
  end
end

-- components/bash-execution.ts BashExecutionComponent — rows carry the
-- component state; bash_execution_lines is the render.
function new_bash_execution_row(command, excluded)
  return {
    kind = "bash", command = command, excluded = excluded or false,
    output_lines = {}, status = "running", exit_code = nil,
    -- Loader(ui, colorKey spinner, muted text, "Running... (…)").
    loader = pi.tui.loader("Running... (" .. select_key_text("cancel") .. " to cancel)"),
    truncation_result = nil, full_output_path = nil, expanded = false,
    -- Constructor content shows until the first updateDisplay
    -- (appendOutput / setComplete / invalidate).
    display_updated = false,
  }
end

function bash_row_append_output(row, chunk)
  -- Strip ANSI codes and normalize line endings.
  local clean = strip_ansi(chunk):gsub("\r\n", "\n"):gsub("\r", "\n")
  local new_lines = split_all(clean, "\n")
  if #row.output_lines > 0 and #new_lines > 0 then
    -- Append first chunk to last line (incomplete line continuation).
    row.output_lines[#row.output_lines] = row.output_lines[#row.output_lines] .. new_lines[1]
    for i = 2, #new_lines do row.output_lines[#row.output_lines + 1] = new_lines[i] end
  else
    for i = 1, #new_lines do row.output_lines[#row.output_lines + 1] = new_lines[i] end
  end
  row.display_updated = true
end

function bash_row_set_complete(row, exit_code, cancelled, truncation_result, full_output_path)
  row.exit_code = exit_code
  row.status = cancelled and "cancelled"
    or ((exit_code ~= nil and exit_code ~= 0) and "error" or "complete")
  row.truncation_result = truncation_result
  row.full_output_path = full_output_path
  row.loader = nil -- loader.stop()
  row.display_updated = true
end

function bash_execution_lines(row, width, theme)
  -- Dim border for excluded-from-context commands (!! prefix).
  local color_key = row.excluded and "dim" or "bashMode"
  local function border_color(text) return theme:fg(color_key, text) end
  local function loader_lines()
    if not row.loader then return {} end
    local frame = row.loader:frame()
    local text = theme:fg("muted", "Running... (" .. select_key_text("cancel") .. " to cancel)")
    if frame ~= "" then text = theme:fg(color_key, frame) .. " " .. text end
    local lines = { "" }
    append(lines, pi.tui.text_render(text, width, 1, 0))
    return lines
  end

  local lines = { "" } -- Spacer(1)
  lines[#lines + 1] = dynamic_border_line(theme, width, border_color)

  if not row.display_updated then
    -- Constructor content: colorKey header + loader.
    append(lines, pi.tui.text_render(
      theme:fg(color_key, theme:bold("$ " .. row.command)), width, 1, 0))
    append(lines, loader_lines())
    lines[#lines + 1] = dynamic_border_line(theme, width, border_color)
    return lines
  end

  -- updateDisplay: context truncation with the bash tool's limits.
  local full_output = table.concat(row.output_lines, "\n")
  local context_truncation = truncate_lib.truncate_tail(full_output, {})
  local available_lines = {}
  if context_truncation.content ~= nil and context_truncation.content ~= "" then
    available_lines = split_all(context_truncation.content, "\n")
  end
  local hidden_line_count = math.max(0, #available_lines - 20) -- PREVIEW_LINES

  -- Command header (updateDisplay always styles it bashMode — spec
  -- behavior, excluded commands included).
  append(lines, pi.tui.text_render(
    theme:fg("bashMode", theme:bold("$ " .. row.command)), width, 1, 0))

  if #available_lines > 0 then
    if row.expanded then
      local styled = {}
      for i, line in ipairs(available_lines) do styled[i] = theme:fg("muted", line) end
      append(lines, pi.tui.text_render("\n" .. table.concat(styled, "\n"), width, 1, 0))
    else
      local styled = {}
      for i = hidden_line_count + 1, #available_lines do
        styled[#styled + 1] = theme:fg("muted", available_lines[i])
      end
      local result = visual_truncate_lib.truncate_to_visual_lines(
        "\n" .. table.concat(styled, "\n"), 20, width, 1)
      append(lines, result.visualLines)
    end
  end

  if row.status == "running" then
    append(lines, loader_lines())
  else
    local status_parts = {}
    if hidden_line_count > 0 then
      if row.expanded then
        status_parts[#status_parts + 1] = theme:fg("muted", "(")
          .. raw_key_hint(theme, DEFAULT_KEYS["app.tools.expand"], "to collapse")
          .. theme:fg("muted", ")")
      else
        status_parts[#status_parts + 1] =
          theme:fg("muted", "... " .. hidden_line_count .. " more lines (")
          .. raw_key_hint(theme, DEFAULT_KEYS["app.tools.expand"], "to expand")
          .. theme:fg("muted", ")")
      end
    end
    if row.status == "cancelled" then
      status_parts[#status_parts + 1] = theme:fg("warning", "(cancelled)")
    elseif row.status == "error" then
      status_parts[#status_parts + 1] = theme:fg("error", "(exit " .. tostring(row.exit_code) .. ")")
    end
    -- Context truncation warning (not preview truncation).
    local was_truncated = (row.truncation_result and row.truncation_result.truncated)
      or context_truncation.truncated
    if was_truncated and row.full_output_path then
      status_parts[#status_parts + 1] =
        theme:fg("warning", "Output truncated. Full output: " .. row.full_output_path)
    end
    if #status_parts > 0 then
      append(lines, pi.tui.text_render("\n" .. table.concat(status_parts, "\n"), width, 1, 0))
    end
  end

  lines[#lines + 1] = dynamic_border_line(theme, width, border_color)
  return lines
end

-- interactive-mode.ts updatePendingMessagesDisplay clears the pending
-- container, dropping deferred bash components from the display (they
-- stay in the flush list); pi-rs's queue rows render statelessly, so the
-- queue-change call sites clear the display list instead.
function clear_pending_bash_display(state)
  if state.pending_bash_rows and #state.pending_bash_rows > 0 then
    state.pending_bash_rows = {}
  end
end

-- interactive-mode.ts flushPendingBashComponents: move deferred
-- components from the pending container to the chat.
function flush_pending_bash_components(state)
  for _, row in ipairs(state.pending_bash_components or {}) do
    for index, pending in ipairs(state.pending_bash_rows or {}) do
      if pending == row then table.remove(state.pending_bash_rows, index) break end
    end
    state.transcript[#state.transcript + 1] = row
  end
  state.pending_bash_components = {}
end

-- interactive-mode.ts handleBashCommand. `user_bash` handlers run before any
-- row/process work; the first result/operations override wins and failures are
-- isolated by the shared extension event policy.
function handle_bash_command(state, command, exclude_from_context)
  local event_result = EXTENSION_POLICY.emit_user_bash({
    type = "user_bash", command = command,
    excludeFromContext = exclude_from_context,
    cwd = state.session_manager:get_cwd(),
  }, EXTENSION_CONTEXT_POLICY.snapshot(state))
  local row = new_bash_execution_row(command, exclude_from_context)
  local is_deferred = state.session.is_streaming()
  if is_deferred then
    state.pending_bash_rows = state.pending_bash_rows or {}
    state.pending_bash_components = state.pending_bash_components or {}
    state.pending_bash_rows[#state.pending_bash_rows + 1] = row
    state.pending_bash_components[#state.pending_bash_components + 1] = row
  else
    state.transcript[#state.transcript + 1] = row
  end
  state.bash_row = row

  if event_result and event_result.result then
    local result = event_result.result
    if result.output then bash_row_append_output(row, result.output) end
    bash_row_set_complete(row, result.exitCode, result.cancelled,
      result.truncated and { truncated = true, content = result.output } or nil,
      result.fullOutputPath)
    state.bash_slice.record(command, result, { excludeFromContext = exclude_from_context })
    state.bash_row = nil
    state.is_bash_mode = false
    state.bash_mode_text = state.editor and state.editor.editor:get_text() or ""
    state.async_render = true
    return
  end

  local signal = state.session.begin_bash()
  state.bash_task = pi.spawn(function()
    local executed, result = pcall(state.session.execute_bash, command, signal,
      function(chunk)
        if state.bash_row then
          bash_row_append_output(state.bash_row, chunk)
          if state.bash_chunk_hook then state.bash_chunk_hook() end
        end
      end,
      { excludeFromContext = exclude_from_context,
        operations = event_result and event_result.operations or nil })
    if executed then
      if state.bash_row then
        bash_row_set_complete(state.bash_row, result.exitCode, result.cancelled,
          result.truncated and { truncated = true, content = result.output } or nil,
          result.fullOutputPath)
      end
    else
      if state.bash_row then bash_row_set_complete(state.bash_row, nil, false) end
      local message = type(result) == "string" and result or "Unknown error"
      show_error(state, "Bash command failed: " .. message)
    end
    state.bash_row = nil
    state.is_bash_mode = false
    state.bash_mode_text = state.editor and state.editor.editor:get_text() or ""
  end)
end

-- core/provider-display-names.ts — presentation data.
local BUILT_IN_PROVIDER_DISPLAY_NAMES = {
  anthropic = "Anthropic",
  ["amazon-bedrock"] = "Amazon Bedrock",
  ["ant-ling"] = "Ant Ling",
  ["azure-openai-responses"] = "Azure OpenAI Responses",
  cerebras = "Cerebras",
  ["cloudflare-ai-gateway"] = "Cloudflare AI Gateway",
  ["cloudflare-workers-ai"] = "Cloudflare Workers AI",
  deepseek = "DeepSeek",
  fireworks = "Fireworks",
  google = "Google Gemini",
  ["google-vertex"] = "Google Vertex AI",
  groq = "Groq",
  huggingface = "Hugging Face",
  ["kimi-coding"] = "Kimi For Coding",
  mistral = "Mistral",
  minimax = "MiniMax",
  ["minimax-cn"] = "MiniMax (China)",
  moonshotai = "Moonshot AI",
  ["moonshotai-cn"] = "Moonshot AI (China)",
  nvidia = "NVIDIA NIM",
  opencode = "OpenCode Zen",
  ["opencode-go"] = "OpenCode Go",
  openai = "OpenAI",
  openrouter = "OpenRouter",
  together = "Together AI",
  ["vercel-ai-gateway"] = "Vercel AI Gateway",
  xai = "xAI",
  zai = "ZAI",
  ["zai-coding-cn"] = "ZAI Coding Plan (China)",
  xiaomi = "Xiaomi MiMo",
  ["xiaomi-token-plan-cn"] = "Xiaomi MiMo Token Plan (China)",
  ["xiaomi-token-plan-ams"] = "Xiaomi MiMo Token Plan (Amsterdam)",
  ["xiaomi-token-plan-sgp"] = "Xiaomi MiMo Token Plan (Singapore)",
}

-- model-registry.ts getProviderDisplayName, minus the registeredProviders
-- half (models.json / registerProvider glue lands with its milestone).
local function provider_display_name(auth, provider)
  for _, oauth_provider in ipairs(auth.oauth_providers()) do
    if oauth_provider.id == provider then return oauth_provider.name end
  end
  return BUILT_IN_PROVIDER_DISPLAY_NAMES[provider] or provider
end

-- interactive-mode.ts isApiKeyLoginProvider.
local function is_api_key_login_provider(provider_id, oauth_ids, built_in_ids)
  if BUILT_IN_PROVIDER_DISPLAY_NAMES[provider_id] then return true end
  if built_in_ids[provider_id] then return false end
  return not oauth_ids[provider_id]
end

-- interactive-mode.ts getLoginProviderOptions. Model providers come from
-- the catalog (spec: modelRegistry.getAll() — identical until the
-- models.json half of the registry lands).
local function get_login_provider_options(auth, auth_type)
  local oauth_ids, options = {}, {}
  for _, provider in ipairs(auth.oauth_providers()) do
    oauth_ids[provider.id] = true
    options[#options + 1] = { id = provider.id, name = provider.name, authType = "oauth" }
  end
  local built_in_ids = {}
  for _, provider in ipairs(pi.ai.providers()) do built_in_ids[provider] = true end
  for _, provider_id in ipairs(pi.ai.providers()) do
    if is_api_key_login_provider(provider_id, oauth_ids, built_in_ids) then
      options[#options + 1] = {
        id = provider_id,
        name = provider_display_name(auth, provider_id),
        authType = "api_key",
      }
    end
  end
  local filtered = {}
  for _, option in ipairs(options) do
    if auth_type == nil or option.authType == auth_type then
      filtered[#filtered + 1] = option
    end
  end
  table.sort(filtered, function(a, b) return a.name < b.name end)
  return filtered
end

-- interactive-mode.ts getLogoutProviderOptions.
local function get_logout_provider_options(auth)
  local options = {}
  for _, provider_id in ipairs(auth.list()) do
    local credential = auth.get(provider_id)
    if credential then
      options[#options + 1] = {
        id = provider_id,
        name = provider_display_name(auth, provider_id),
        authType = credential.type,
      }
    end
  end
  table.sort(options, function(a, b) return a.name < b.name end)
  return options
end

-- components/extension-selector.ts (the countdown-timer variant has no
-- caller on this surface — the auth selectors pass no timeout).
local function extension_selector(opts)
  local theme = opts.theme
  local self = { selected = 0, focused = false }
  function self:set_focused(focused) self.focused = focused end
  function self:handle_input(data)
    if opts.on_toggle_tools_expanded and binding_matches(data, DEFAULT_KEYS["app.tools.expand"]) then
      opts.on_toggle_tools_expanded()
    elseif binding_matches(data, SELECT_KEYS.up) or data == "k" then
      self.selected = math.max(0, self.selected - 1)
    elseif binding_matches(data, SELECT_KEYS.down) or data == "j" then
      self.selected = math.min(#opts.options - 1, self.selected + 1)
    elseif binding_matches(data, SELECT_KEYS.confirm) or data == "\n" then
      local selected = opts.options[self.selected + 1]
      if selected then opts.on_select(selected) end
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      opts.on_cancel()
    end
  end
  function self:render(width)
    local border = dynamic_border_line(theme, width)
    local lines = { border, "" }
    append(lines, pi.tui.text_render(theme:fg("accent", theme:bold(opts.title)), width, 1, 0))
    lines[#lines + 1] = ""
    for index, option in ipairs(opts.options) do
      local text
      if index - 1 == self.selected then
        text = theme:fg("accent", "→ ") .. theme:fg("accent", option)
      else
        text = "  " .. theme:fg("text", option)
      end
      append(lines, pi.tui.text_render(text, width, 1, 0))
    end
    lines[#lines + 1] = ""
    append(lines, pi.tui.text_render(
      raw_key_hint(theme, "↑↓", "navigate") .. "  "
        .. select_key_hint(theme, "confirm", "select") .. "  "
        .. select_key_hint(theme, "cancel", "cancel"),
      width, 1, 0))
    lines[#lines + 1] = ""
    lines[#lines + 1] = border
    return lines
  end
  return self
end


-- components/login-dialog.ts. Content children mirror the spec's
-- contentContainer (Spacer / Text / the Input); the pending input promise
-- becomes a resolve/reject pair.
local function login_dialog(opts)
  local theme = opts.theme
  local self = {
    content = {},
    input = pi.tui.input(),
    focused = false,
    input_pending = nil,
  }
  local provider_name = opts.provider_name
  if not provider_name then
    for _, provider in ipairs(opts.auth.oauth_providers()) do
      if provider.id == opts.provider_id then provider_name = provider.name end
    end
    provider_name = provider_name or opts.provider_id
  end
  local title = opts.title or ("Login to " .. provider_name)
  local open_browser = opts.open_browser or pi.open_browser

  function self:set_focused(focused)
    self.focused = focused
    self.input:set_focused(focused)
  end

  local function add(child) self.content[#self.content + 1] = child end

  local function replace_input_with_submitted_text(value)
    for index, child in ipairs(self.content) do
      if child.kind == "input" then
        self.content[index] = { kind = "text", text = "> " .. value, px = 0 }
      end
    end
  end

  function self:cancel()
    local pending = self.input_pending
    self.input_pending = nil
    if pending and pending.reject then pending.reject("Login cancelled") end
    opts.on_complete(false, "Login cancelled")
  end

  -- onAuth — authorization URL plus click hint, then open the browser.
  function self:show_auth(url, instructions)
    self.content = {}
    add({ kind = "spacer" })
    local linked_url = "\27]8;;" .. url .. "\7" .. url .. "\27]8;;\7"
    add({ kind = "text", text = theme:fg("accent", linked_url) })
    local click_hint = pi.platform() == "darwin" and "Cmd+click to open" or "Ctrl+click to open"
    local hyperlink = "\27]8;;" .. url .. "\7" .. click_hint .. "\27]8;;\7"
    add({ kind = "text", text = theme:fg("dim", hyperlink) })
    if instructions then
      add({ kind = "spacer" })
      add({ kind = "text", text = theme:fg("warning", instructions) })
    end
    open_browser(url)
  end

  -- onDeviceCode — URL and user code.
  function self:show_device_code(info)
    self.content = {}
    add({ kind = "spacer" })
    local uri = info.verificationUri
    add({ kind = "text", text = theme:fg("accent", "\27]8;;" .. uri .. "\7" .. uri .. "\27]8;;\7") })
    local click_hint = pi.platform() == "darwin" and "Cmd+click to open" or "Ctrl+click to open"
    add({ kind = "text", text = theme:fg("dim", "\27]8;;" .. uri .. "\7" .. click_hint .. "\27]8;;\7") })
    add({ kind = "spacer" })
    add({ kind = "text", text = theme:fg("warning", "Enter code: " .. info.userCode) })
    open_browser(uri)
  end

  -- Manual code/URL entry for callback-server providers.
  function self:show_manual_input(prompt_text, resolve, reject)
    self.input:set_value("")
    add({ kind = "spacer" })
    add({ kind = "text", text = theme:fg("dim", prompt_text) })
    add({ kind = "input" })
    add({ kind = "text", text = "(" .. select_key_hint(theme, "cancel", "to cancel") .. ")" })
    self.input_pending = { resolve = resolve, reject = reject }
  end

  -- onPrompt — appends (preserves the URL from show_auth).
  function self:show_prompt(message, placeholder, resolve, reject)
    add({ kind = "spacer" })
    add({ kind = "text", text = theme:fg("text", message) })
    if placeholder then
      add({ kind = "text", text = theme:fg("dim", "e.g., " .. placeholder) })
    end
    add({ kind = "input" })
    add({ kind = "text", text = "(" .. select_key_hint(theme, "cancel", "to cancel,") .. " "
      .. select_key_hint(theme, "confirm", "to submit") .. ")" })
    self.input:set_value("")
    self.input_pending = { resolve = resolve, reject = reject }
  end

  function self:show_info(lines)
    self.content = {}
    add({ kind = "spacer" })
    for _, line in ipairs(lines) do add({ kind = "text", text = line }) end
    add({ kind = "spacer" })
    add({ kind = "text", text = "(" .. select_key_hint(theme, "cancel", "to close") .. ")" })
  end

  function self:show_waiting(message)
    add({ kind = "spacer" })
    add({ kind = "text", text = theme:fg("dim", message) })
    add({ kind = "text", text = "(" .. select_key_hint(theme, "cancel", "to cancel") .. ")" })
  end

  function self:show_progress(message)
    add({ kind = "text", text = theme:fg("dim", message) })
  end

  function self:handle_input(data)
    if binding_matches(data, SELECT_KEYS.cancel) then
      self:cancel()
      return
    end
    local event = self.input:input(data)
    if event.kind == "submit" and self.input_pending then
      local value = event.value or self.input:value()
      replace_input_with_submitted_text(value)
      local pending = self.input_pending
      self.input_pending = nil
      pending.resolve(value)
    end
  end

  function self:render(width)
    local border = dynamic_border_line(theme, width)
    local lines = { border }
    append(lines, pi.tui.text_render(theme:fg("accent", theme:bold(title)), width, 1, 0))
    for _, child in ipairs(self.content) do
      if child.kind == "spacer" then
        lines[#lines + 1] = ""
      elseif child.kind == "input" then
        append(lines, self.input:render(width))
      else
        append(lines, pi.tui.text_render(child.text, width, child.px or 1, 0))
      end
    end
    lines[#lines + 1] = border
    return lines
  end

  return self
end

local ANTHROPIC_SUBSCRIPTION_AUTH_WARNING =
  "Anthropic subscription auth is active. Third-party harness usage draws from extra usage and is billed per token, not your Claude plan limits. Manage extra usage at https://claude.ai/settings/usage."

-- interactive-mode.ts maybeWarnAboutAnthropicSubscriptionAuth.
local function maybe_warn_about_anthropic_subscription_auth(state, model)
  local warnings = pi.settings.warnings()
  if warnings.anthropicExtraUsage == false then return end
  model = model or state.model
  if state.anthropic_subscription_warning_shown then return end
  if not model or model.provider ~= "anthropic" then return end
  local stored = state.auth.get("anthropic")
  if stored and stored.type == "oauth" then
    state.anthropic_subscription_warning_shown = true
    show_warning(state, ANTHROPIC_SUBSCRIPTION_AUTH_WARNING)
    return
  end
  local api_key
  if stored and stored.type == "api_key" then
    api_key = state.auth.resolve_config_value(stored.key)
  else
    api_key = state.auth.env_api_key(model.provider)
  end
  if type(api_key) == "string" and api_key:sub(1, 10) == "sk-ant-oat" then
    state.anthropic_subscription_warning_shown = true
    show_warning(state, ANTHROPIC_SUBSCRIPTION_AUTH_WARNING)
  end
end

-- interactive-mode.ts completeProviderAuthentication. The
-- isUnknownModel(previousModel) default-model selection branch is
-- unreachable here: this frontend always starts from a resolved model
-- (the unknown-model placeholder arrives with session restore).
local update_available_provider_count

local function complete_provider_authentication(state, provider_id, provider_name, auth_type)
  state.registry.refresh()
  local action_label = auth_type == "oauth" and ("Logged in to " .. provider_name)
    or ("Saved API key for " .. provider_name)
  update_available_provider_count(state)
  show_status(state, action_label .. ". Credentials saved to " .. state.auth.auth_path)
  maybe_warn_about_anthropic_subscription_auth(state)
end

local show_login_auth_type_selector
local show_login_provider_selector
local show_login_dialog
local show_api_key_login_dialog
local show_bedrock_setup_dialog

local BEDROCK_PROVIDER_ID = "amazon-bedrock"

-- interactive-mode.ts showLoginDialog — the OAuth flow half runs behind
-- the pi.auth.login_start handle; pump_login drains its callback events
-- into the dialog exactly as the spec's OAuthLoginCallbacks do.
function show_login_dialog(state, provider_id, provider_name)
  local uses_callback_server = false
  for _, provider in ipairs(state.auth.oauth_providers()) do
    if provider.id == provider_id then
      uses_callback_server = provider.usesCallbackServer or false
    end
  end
  local handle = state.auth.login_start(provider_id)
  local dialog = login_dialog({
    theme = state.theme,
    auth = state.auth,
    provider_id = provider_id,
    provider_name = provider_name,
    open_browser = state.open_browser,
    on_complete = function(_success, _message) end, -- completion handled by pump_login
  })
  -- Dialog cancel rejects the pending input (login-dialog.ts cancel());
  -- the flow then settles with "Login cancelled" through the handle.
  local dialog_cancel = dialog.cancel
  function dialog:cancel()
    dialog_cancel(self)
    handle:cancel()
  end
  mount_in_editor_slot(state, dialog)
  state.login = {
    handle = handle,
    dialog = dialog,
    provider_id = provider_id,
    provider_name = provider_name,
    uses_callback_server = uses_callback_server,
  }
end

-- Drain login-flow callback events. timeout_ms = 0 polls (the frontend
-- pumps every process event); a positive timeout awaits (scripted flows).
local function pump_login(state, timeout_ms)
  while state.login do
    local login = state.login
    local event = login.handle:next_event(timeout_ms or 0)
    if not event then return end
    local dialog = login.dialog
    if event.type == "auth" then
      dialog:show_auth(event.url, event.instructions)
      if login.uses_callback_server then
        dialog:show_manual_input("Paste redirect URL below, or complete login in browser:",
          function(value)
            if value and value ~= "" then login.handle:respond(value) end
          end,
          function(_message) login.handle:cancel() end)
      end
    elseif event.type == "deviceCode" then
      dialog:show_device_code(event)
      dialog:show_waiting("Waiting for authentication...")
    elseif event.type == "prompt" then
      dialog:show_prompt(event.message, event.placeholder,
        function(value) login.handle:respond(value) end,
        function(_message) login.handle:cancel() end)
    elseif event.type == "progress" then
      dialog:show_progress(event.message)
    elseif event.type == "select" then
      -- showOAuthLoginSelect: an ExtensionSelector replaces the dialog
      -- and restores it with the chosen option id (nil on cancel).
      local options = event.options or {}
      local labels = {}
      for _, option in ipairs(options) do labels[#labels + 1] = option.label end
      local selector = extension_selector({
        theme = state.theme,
        title = event.message,
        options = labels,
        on_select = function(label)
          mount_in_editor_slot(state, dialog)
          local id
          for _, option in ipairs(options) do
            if option.label == label then id = option.id end
          end
          login.handle:respond(id)
        end,
        on_cancel = function()
          mount_in_editor_slot(state, dialog)
          login.handle:respond(nil)
        end,
      })
      mount_in_editor_slot(state, selector)
    elseif event.type == "done" then
      state.login = nil
      restore_editor(state)
      complete_provider_authentication(state, login.provider_id, login.provider_name, "oauth")
    elseif event.type == "error" then
      state.login = nil
      restore_editor(state)
      if event.message ~= "Login cancelled" then
        show_error(state, "Failed to login to " .. login.provider_name .. ": " .. event.message)
      end
    end
  end
end

-- interactive-mode.ts showApiKeyLoginDialog.
function show_api_key_login_dialog(state, provider_id, provider_name)
  local function fail(message)
    restore_editor(state)
    if message ~= "Login cancelled" then
      show_error(state, "Failed to save API key for " .. provider_name .. ": " .. message)
    end
  end
  local dialog = login_dialog({
    theme = state.theme,
    auth = state.auth,
    provider_id = provider_id,
    provider_name = provider_name,
    open_browser = state.open_browser,
    on_complete = function(_success, _message) end, -- completion handled below
  })
  mount_in_editor_slot(state, dialog)
  dialog:show_prompt("Enter API key:", nil,
    function(value)
      local api_key = trim(value)
      if api_key == "" then
        fail("API key cannot be empty.")
        return
      end
      state.auth.set(provider_id, { type = "api_key", key = api_key })
      restore_editor(state)
      complete_provider_authentication(state, provider_id, provider_name, "api_key")
    end,
    function(message) fail(message) end)
end

-- interactive-mode.ts showBedrockSetupDialog.
function show_bedrock_setup_dialog(state, provider_id, provider_name)
  local theme = state.theme
  local dialog = login_dialog({
    theme = theme,
    auth = state.auth,
    provider_id = provider_id,
    provider_name = provider_name,
    title = "Amazon Bedrock setup",
    open_browser = state.open_browser,
    on_complete = function(_success, _message) restore_editor(state) end,
  })
  dialog:show_info({
    theme:fg("text", "Amazon Bedrock uses AWS credentials instead of a single API key."),
    theme:fg("text", "Configure an AWS profile, IAM keys, bearer token, or role-based credentials."),
    theme:fg("muted", "See:"),
    theme:fg("accent", "  " .. pi.path.join(state.docs_path or "", "providers.md")),
  })
  mount_in_editor_slot(state, dialog)
end

-- interactive-mode.ts showLoginAuthTypeSelector.
function show_login_auth_type_selector(state)
  local subscription_label = "Use a subscription"
  local api_key_label = "Use an API key"
  show_selector(state, function(done)
    return extension_selector({
      theme = state.theme,
      title = "Select authentication method:",
      options = { subscription_label, api_key_label },
      on_select = function(option)
        done()
        local auth_type = option == subscription_label and "oauth" or "api_key"
        show_login_provider_selector(state, auth_type)
      end,
      on_cancel = function() done() end,
    })
  end)
end

-- interactive-mode.ts showLoginProviderSelector. Exercisers may script
-- the option lists (pi computes them from its full provider registry;
-- provider breadth is the auth-compatibility milestone).
function show_login_provider_selector(state, auth_type)
  local provider_options = state.login_provider_options
    and state.login_provider_options(auth_type)
    or get_login_provider_options(state.auth, auth_type)
  if #provider_options == 0 then
    show_status(state, auth_type == "oauth" and "No subscription providers available."
      or "No API key providers available.")
    return
  end
  show_selector(state, function(done)
    return oauth_selector({
      mode = "login",
      providers = provider_options,
      theme = state.theme,
      get_credential = function(id) return state.auth.get(id) end,
      get_auth_status = function(id) return state.auth.get_auth_status(id) end,
      on_select = function(provider_id)
        done()
        local option
        for _, candidate in ipairs(provider_options) do
          if candidate.id == provider_id then option = candidate end
        end
        if not option then return end
        if option.authType == "oauth" then
          show_login_dialog(state, option.id, option.name)
        elseif option.id == BEDROCK_PROVIDER_ID then
          show_bedrock_setup_dialog(state, option.id, option.name)
        else
          show_api_key_login_dialog(state, option.id, option.name)
        end
      end,
      on_cancel = function()
        done()
        show_login_auth_type_selector(state)
      end,
    })
  end)
end

-- interactive-mode.ts showOAuthSelector.
local function show_oauth_selector(state, mode)
  if mode == "login" then
    show_login_auth_type_selector(state)
    return
  end

  local provider_options = state.logout_provider_options
    and state.logout_provider_options()
    or get_logout_provider_options(state.auth)
  if #provider_options == 0 then
    show_status(state,
      "No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged.")
    return
  end

  show_selector(state, function(done)
    return oauth_selector({
      mode = mode,
      providers = provider_options,
      theme = state.theme,
      get_credential = function(id) return state.auth.get(id) end,
      get_auth_status = function(id) return state.auth.get_auth_status(id) end,
      on_select = function(provider_id)
        done()
        local option
        for _, candidate in ipairs(provider_options) do
          if candidate.id == provider_id then option = candidate end
        end
        if not option then return end
        local ok, err = pcall(function()
          -- authStorage.logout, then the registry and the footer's
          -- provider count pick up the change.
          state.auth.remove(option.id)
          state.registry.refresh()
          update_available_provider_count(state)
        end)
        if ok then
          local message
          if option.authType == "oauth" then
            message = "Logged out of " .. option.name
          else
            message = "Removed stored API key for " .. option.name
              .. ". Environment variables and models.json config are unchanged."
          end
          show_status(state, message)
        else
          show_error(state, "Logout failed: " .. tostring(err))
        end
      end,
      on_cancel = function() done() end,
    })
  end)
end

-- The pi.auth-backed seam the product frontend uses; exercisers inject
-- scripted implementations of the same shape.
local function default_auth_seam()
  return {
    get = function(id) return pi.auth.get(id) end,
    get_auth_status = function(id) return pi.auth.get_auth_status(id) end,
    list = function() return pi.auth.list() end,
    set = function(id, credential) pi.auth.set(id, credential) end,
    remove = function(id) pi.auth.remove(id) end,
    oauth_providers = function() return pi.auth.oauth_providers() end,
    login_start = function(id) return pi.auth.login_start(id) end,
    env_api_key = function(id) return pi.auth.env_api_key(id) end,
    resolve_config_value = function(value) return pi.auth.resolve_config_value(value) end,
    auth_path = pi.auth.auth_path(),
  }
end

-- ===========================================================================
-- /model and the model selector — ports of components/model-selector.ts,
-- core/model-resolver.ts findExactModelReferenceMatch, and the
-- interactive-mode.ts model wiring (handleModelCommand, showModelSelector,
-- cycleModel, updateAvailableProviderCount) over the pi.ai registry bridge.
-- ===========================================================================

-- pi-ai models.ts modelsAreEqual.
local function models_are_equal(a, b)
  if not a or not b then return false end
  return a.id == b.id and a.provider == b.provider
end

-- The pi.ai-backed registry seam the product frontend uses; exercisers
-- inject scripted implementations of the same shape.
local function default_registry_seam()
  return {
    refresh = function() pi.ai.registry_refresh() end,
    get_error = function() return pi.ai.registry_error() end,
    get_available = function() return pi.ai.available_models() end,
    find = function(provider, id) return pi.ai.find_model(provider, id) end,
    has_configured_auth = function(model) return pi.ai.has_configured_auth(model) end,
    is_using_oauth = function(model) return pi.ai.is_using_oauth(model) end,
  }
end

-- core/model-resolver.ts findExactModelReferenceMatch.
local function find_exact_model_reference_match(reference, models)
  local trimmed = trim(reference or "")
  if trimmed == "" then return nil end
  local normalized = trimmed:lower()
  local canonical = {}
  for _, model in ipairs(models) do
    if (model.provider .. "/" .. model.id):lower() == normalized then
      canonical[#canonical + 1] = model
    end
  end
  if #canonical == 1 then return canonical[1] end
  if #canonical > 1 then return nil end
  local slash = trimmed:find("/", 1, true)
  if slash then
    local provider = trim(trimmed:sub(1, slash - 1))
    local model_id = trim(trimmed:sub(slash + 1))
    if provider ~= "" and model_id ~= "" then
      local matches = {}
      for _, model in ipairs(models) do
        if model.provider:lower() == provider:lower() and model.id:lower() == model_id:lower() then
          matches[#matches + 1] = model
        end
      end
      if #matches == 1 then return matches[1] end
      if #matches > 1 then return nil end
    end
  end
  local id_matches = {}
  for _, model in ipairs(models) do
    if model.id:lower() == normalized then id_matches[#id_matches + 1] = model end
  end
  if #id_matches == 1 then return id_matches[1] end
  return nil
end

-- components/model-selector.ts — presentation and input policy. The
-- registry and settings seams are injected; the spec's async loadModels
-- resolves synchronously through the seam.
local function model_selector(opts)
  local theme = opts.theme
  local registry = opts.registry
  local self = {
    search = pi.tui.input(),
    focused = false,
    all_models = {},
    scoped_model_items = {},
    active_models = {},
    filtered = {},
    selected = 0, -- 0-based: the spec's windowing math copied 1:1
    current_model = opts.current_model,
    scoped_models = opts.scoped_models or {},
    error_message = nil,
  }
  self.scope = #self.scoped_models > 0 and "scoped" or "all"

  -- sortModels — current model first, then provider; JS sort is stable,
  -- so ties keep registry order (decorate with the original index).
  local function sort_models(models)
    local decorated = {}
    for index, item in ipairs(models) do decorated[index] = { item = item, index = index } end
    table.sort(decorated, function(a, b)
      local a_current = models_are_equal(self.current_model, a.item.model)
      local b_current = models_are_equal(self.current_model, b.item.model)
      if a_current ~= b_current then return a_current end
      if a.item.provider ~= b.item.provider then return a.item.provider < b.item.provider end
      return a.index < b.index
    end)
    local sorted = {}
    for index, entry in ipairs(decorated) do sorted[index] = entry.item end
    return sorted
  end

  local function get_scope_text()
    local all_text = self.scope == "all" and theme:fg("accent", "all") or theme:fg("muted", "all")
    local scoped_text = self.scope == "scoped" and theme:fg("accent", "scoped")
      or theme:fg("muted", "scoped")
    return theme:fg("muted", "Scope: ") .. all_text .. theme:fg("muted", " | ") .. scoped_text
  end

  local function get_scope_hint_text()
    -- keyHint("tui.input.tab", "scope") over the default bindings.
    return theme:fg("dim", "tab") .. theme:fg("muted", " scope")
      .. theme:fg("muted", " (all/scoped)")
  end

  local function filter_models(query)
    if query ~= "" then
      self.filtered = pi.tui.fuzzy_filter(self.active_models, query, function(item)
        return item.id .. " " .. item.provider .. " " .. item.provider .. "/" .. item.id
          .. " " .. item.provider .. " " .. item.id
      end)
    else
      self.filtered = self.active_models
    end
    self.selected = math.min(self.selected, math.max(0, #self.filtered - 1))
  end

  local function set_scope(scope)
    if self.scope == scope then return end
    self.scope = scope
    self.active_models = scope == "scoped" and self.scoped_model_items or self.all_models
    local current_index = -1
    for index, item in ipairs(self.active_models) do
      if models_are_equal(self.current_model, item.model) then
        current_index = index - 1
        break
      end
    end
    self.selected = current_index >= 0 and current_index or 0
    filter_models(self.search:value())
  end

  local function load_models()
    registry.refresh()
    local load_error = registry.get_error()
    if load_error then self.error_message = load_error end
    local ok, available = pcall(registry.get_available)
    if not ok then
      self.all_models, self.scoped_model_items = {}, {}
      self.active_models, self.filtered = {}, {}
      self.error_message = tostring(available)
      return
    end
    local models = {}
    for _, model in ipairs(available) do
      models[#models + 1] = { provider = model.provider, id = model.id, model = model }
    end
    self.all_models = sort_models(models)
    local refreshed_scoped = {}
    for index, scoped in ipairs(self.scoped_models) do
      local refreshed = registry.find(scoped.model.provider, scoped.model.id)
      refreshed_scoped[index] = refreshed
        and { model = refreshed, thinkingLevel = scoped.thinkingLevel } or scoped
    end
    self.scoped_models = refreshed_scoped
    self.scoped_model_items = {}
    for index, scoped in ipairs(self.scoped_models) do
      self.scoped_model_items[index] =
        { provider = scoped.model.provider, id = scoped.model.id, model = scoped.model }
    end
    self.active_models = self.scope == "scoped" and self.scoped_model_items or self.all_models
    self.filtered = self.active_models
    local current_index = -1
    for index, item in ipairs(self.filtered) do
      if models_are_equal(self.current_model, item.model) then
        current_index = index - 1
        break
      end
    end
    if current_index >= 0 then
      self.selected = current_index
    else
      self.selected = math.min(self.selected, math.max(0, #self.filtered - 1))
    end
  end

  local function handle_select(model)
    -- Save as new default (settingsManager.setDefaultModelAndProvider);
    -- the product seam lands with the settings bridge (PLAN items 5/9).
    if opts.set_default_model_and_provider then
      opts.set_default_model_and_provider(model.provider, model.id)
    end
    opts.on_select(model)
  end

  function self:set_focused(focused)
    self.focused = focused
    self.search:set_focused(focused)
  end

  function self:handle_input(data)
    if binding_matches(data, "tab") then
      if #self.scoped_model_items > 0 then
        set_scope(self.scope == "all" and "scoped" or "all")
      end
      return
    end
    if binding_matches(data, SELECT_KEYS.up) then
      if #self.filtered == 0 then return end
      self.selected = self.selected == 0 and #self.filtered - 1 or self.selected - 1
    elseif binding_matches(data, SELECT_KEYS.down) then
      if #self.filtered == 0 then return end
      self.selected = self.selected == #self.filtered - 1 and 0 or self.selected + 1
    elseif binding_matches(data, SELECT_KEYS.confirm) then
      local item = self.filtered[self.selected + 1]
      if item then handle_select(item.model) end
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      opts.on_cancel()
    else
      self.search:input(data)
      filter_models(self.search:value())
    end
  end

  -- updateList, produced at render time from the same state.
  local function list_lines(width)
    local lines = {}
    local count = #self.filtered
    local max_visible = 10
    local start_index = math.max(0,
      math.min(self.selected - math.floor(max_visible / 2), count - max_visible))
    local end_index = math.min(start_index + max_visible, count)
    for i = start_index, end_index - 1 do
      local item = self.filtered[i + 1]
      local is_current = models_are_equal(self.current_model, item.model)
      local checkmark = is_current and theme:fg("success", " ✓") or ""
      local provider_badge = theme:fg("muted", "[" .. item.provider .. "]")
      local line
      if i == self.selected then
        line = theme:fg("accent", "→ ") .. theme:fg("accent", item.id)
          .. " " .. provider_badge .. checkmark
      else
        line = "  " .. item.id .. " " .. provider_badge .. checkmark
      end
      append(lines, pi.tui.text_render(line, width, 0, 0))
    end
    if start_index > 0 or end_index < count then
      append(lines, pi.tui.text_render(
        theme:fg("muted", "  (" .. (self.selected + 1) .. "/" .. count .. ")"), width, 0, 0))
    end
    if self.error_message then
      for error_line in (self.error_message .. "\n"):gmatch("(.-)\n") do
        append(lines, pi.tui.text_render(theme:fg("error", error_line), width, 0, 0))
      end
    elseif count == 0 then
      append(lines, pi.tui.text_render(theme:fg("muted", "  No matching models"), width, 0, 0))
    else
      local item = self.filtered[self.selected + 1]
      lines[#lines + 1] = "" -- Spacer(1)
      append(lines, pi.tui.text_render(
        theme:fg("muted", "  Model Name: " .. tostring(item.model.name)), width, 0, 0))
    end
    return lines
  end

  function self:render(width)
    local border = dynamic_border_line(theme, width)
    local lines = { border, "" }
    if #self.scoped_model_items > 0 then
      append(lines, pi.tui.text_render(get_scope_text(), width, 0, 0))
      append(lines, pi.tui.text_render(get_scope_hint_text(), width, 0, 0))
    else
      append(lines, pi.tui.text_render(theme:fg("warning",
        "Only showing models from configured providers. Use /login to add providers."),
        width, 0, 0))
    end
    lines[#lines + 1] = ""
    append(lines, self.search:render(width))
    lines[#lines + 1] = ""
    append(lines, list_lines(width))
    lines[#lines + 1] = ""
    lines[#lines + 1] = border
    return lines
  end

  load_models()
  if opts.initial_search and opts.initial_search ~= "" then
    self.search:set_value(opts.initial_search)
    filter_models(opts.initial_search)
  end
  return self
end

-- interactive-mode.ts getModelCandidates.
local function get_model_candidates(state)
  local scoped = state.scoped_models or {}
  if #scoped > 0 then
    local models = {}
    for index, item in ipairs(scoped) do models[index] = item.model end
    return models
  end
  state.registry.refresh()
  local ok, available = pcall(state.registry.get_available)
  if not ok then return {} end
  return available
end

-- interactive-mode.ts updateAvailableProviderCount (declared above the
-- login wiring, which also calls it).
function update_available_provider_count(state)
  local seen, count = {}, 0
  for _, model in ipairs(get_model_candidates(state)) do
    if not seen[model.provider] then
      seen[model.provider] = true
      count = count + 1
    end
  end
  state.provider_count = count
end

-- ===========================================================================
-- Thinking levels — the agent-session.ts thinking slice (PLAN 7.2):
-- setThinkingLevel / cycleThinkingLevel / getAvailableThinkingLevels /
-- supportsThinking over the pi.ai clamp mechanism, plus the interactive-
-- mode.ts cycleThinkingLevel status rows. The footer invalidate and
-- updateEditorBorderColor effects of thinking_level_changed are render-time
-- in pi-rs (frontend_frame reads state.thinking_level; sync_editor_border
-- keys the border off it).
-- ===========================================================================

-- agent-session.ts getAvailableThinkingLevels (the no-model fallback is
-- agent-loop.ts THINKING_LEVELS). Globals, not locals: the main chunk
-- rides Lua's 200-local limit.
function get_available_thinking_levels(state)
  if not state.model then
    return { "off", "minimal", "low", "medium", "high", "xhigh" }
  end
  return pi.ai.supported_thinking_levels(state.model)
end

-- agent-session.ts supportsThinking.
function supports_thinking(state)
  return state.model ~= nil and state.model.reasoning == true
end

-- agent-session.ts setThinkingLevel: clamp, persist, then emit the public
-- thinking_level_select event after the effective state changes.
function session_set_thinking_level(state, level)
  local available = get_available_thinking_levels(state)
  local effective = nil
  for _, candidate in ipairs(available) do
    if candidate == level then effective = level break end
  end
  if effective == nil then
    effective = state.model and pi.ai.clamp_thinking_level(state.model, level) or "off"
  end
  local previous = state.thinking_level
  local changing = effective ~= previous
  state.thinking_level = effective
  if state.agent then state.agent:set_thinking_level(effective) end
  if changing then
    if state.session_manager then
      state.session_manager:append_thinking_level_change(effective)
    end
    if supports_thinking(state) or effective ~= "off" then
      pi.settings.set_default_thinking_level(effective)
    end
    if EXTENSION_CONTEXT_POLICY and state.session_manager then
      EXTENSION_POLICY.emit_generic({ type = "thinking_level_select",
        level = effective, previousLevel = previous },
        EXTENSION_CONTEXT_POLICY.snapshot(state))
    end
  end
end

-- agent-session.ts cycleThinkingLevel.
function session_cycle_thinking_level(state)
  if not supports_thinking(state) then return nil end
  local levels = get_available_thinking_levels(state)
  local index = -1 -- 0-based; spec: indexOf === -1 -> next is levels[0]
  for i, candidate in ipairs(levels) do
    if candidate == state.thinking_level then index = i - 1 break end
  end
  local next_level = levels[((index + 1) % #levels) + 1]
  session_set_thinking_level(state, next_level)
  return next_level
end

-- interactive-mode.ts cycleThinkingLevel (footer.invalidate and
-- updateEditorBorderColor are render-time here).
function cycle_thinking_level(state)
  local new_level = session_cycle_thinking_level(state)
  if new_level == nil then
    show_status(state, "Current model does not support thinking")
  else
    show_status(state, "Thinking level: " .. new_level)
  end
end

-- agent-session.ts _getThinkingLevelForModelSwitch: an explicit scoped
-- level overrides; a non-thinking current model reads the settings
-- default (core/defaults.ts DEFAULT_THINKING_LEVEL "medium"); otherwise
-- the session level carries over.
function thinking_level_for_model_switch(state, explicit_level)
  if explicit_level ~= nil then return explicit_level end
  if not supports_thinking(state) then
    return pi.settings.default_thinking_level() or "medium"
  end
  return state.thinking_level
end

-- agent-session.ts setModel: mutate/persist/re-clamp, then model_select.
local function session_set_model(state, model)
  if not state.registry.has_configured_auth(model) then
    error("No API key for " .. model.provider .. "/" .. model.id, 0)
  end
  local previous_model = state.model
  pi.settings.set_default_model_and_provider(model.provider, model.id)
  local thinking_level = thinking_level_for_model_switch(state)
  if state.agent then state.agent:set_model(model) end
  state.model = model
  if state.session_manager then
    state.session_manager:append_model_change(model.provider, model.id)
  end
  session_set_thinking_level(state, thinking_level)
  if not models_are_equal(previous_model, model) and EXTENSION_CONTEXT_POLICY then
    EXTENSION_POLICY.emit_generic({ type = "model_select", model = model,
      previousModel = previous_model, source = "set" },
      EXTENSION_CONTEXT_POLICY.snapshot(state))
  end
end

-- interactive-mode.ts showModelSelector; successful explicit selection also
-- checks the OpenCode/Kimi K2.5 easter-egg predicate.
local function show_model_selector(state, initial_search)
  show_selector(state, function(done)
    return model_selector({
      theme = state.theme,
      current_model = state.model,
      scoped_models = state.scoped_models or {},
      registry = state.registry,
      set_default_model_and_provider = state.set_default_model_and_provider,
      initial_search = initial_search,
      on_select = function(model)
        local ok, err = pcall(session_set_model, state, model)
        if ok then
          done()
          show_status(state, "Model: " .. model.id)
          maybe_warn_about_anthropic_subscription_auth(state, model)
          check_daxnuts_easter_egg(state, model)
        else
          done()
          show_error(state, tostring(err))
        end
      end,
      on_cancel = function() done() end,
    })
  end)
end

-- interactive-mode.ts handleModelCommand.
local function handle_model_command(state, search_term)
  if not search_term or search_term == "" then
    show_model_selector(state)
    return
  end
  local model = find_exact_model_reference_match(search_term, get_model_candidates(state))
  if model then
    local ok, err = pcall(session_set_model, state, model)
    if ok then
      show_status(state, "Model: " .. model.id)
      maybe_warn_about_anthropic_subscription_auth(state, model)
      check_daxnuts_easter_egg(state, model)
    else
      show_error(state, tostring(err))
    end
    return
  end
  show_model_selector(state, search_term)
end

-- agent-session.ts cycleModel (scoped filtered by configured auth, else
-- all available; wrap-around) + the interactive-mode.ts status rows.
local function session_cycle_model(state, direction)
  local scoped = state.scoped_models or {}
  local candidates, is_scoped
  local explicit_levels = {}
  if #scoped > 0 then
    candidates, is_scoped = {}, true
    for _, item in ipairs(scoped) do
      if state.registry.has_configured_auth(item.model) then
        candidates[#candidates + 1] = item.model
        explicit_levels[#candidates] = item.thinkingLevel
      end
    end
  else
    is_scoped = false
    local ok, available = pcall(state.registry.get_available)
    candidates = ok and available or {}
  end
  if #candidates <= 1 then return nil end
  local current_index = 0 -- 0-based; spec: findIndex === -1 -> 0
  for index, model in ipairs(candidates) do
    if models_are_equal(model, state.model) then
      current_index = index - 1
      break
    end
  end
  local len = #candidates
  local next_index = direction == "forward" and (current_index + 1) % len
    or (current_index - 1 + len) % len
  local next_model = candidates[next_index + 1]
  local previous_model = state.model
  -- _cycleScopedModel: an explicit scoped thinking level overrides the
  -- session preference; _cycleAvailableModel re-clamps the current one.
  local explicit_level = is_scoped and explicit_levels[next_index + 1] or nil
  local thinking_level = thinking_level_for_model_switch(state, explicit_level)
  pi.settings.set_default_model_and_provider(next_model.provider, next_model.id)
  if state.agent then state.agent:set_model(next_model) end
  state.model = next_model
  if state.session_manager then
    state.session_manager:append_model_change(next_model.provider, next_model.id)
  end
  session_set_thinking_level(state, thinking_level)
  if EXTENSION_CONTEXT_POLICY then
    EXTENSION_POLICY.emit_generic({ type = "model_select", model = next_model,
      previousModel = previous_model, source = "cycle" },
      EXTENSION_CONTEXT_POLICY.snapshot(state))
  end
  return { model = next_model, thinkingLevel = state.thinking_level or "off", isScoped = is_scoped }
end

local function cycle_model(state, direction)
  local ok, result = pcall(session_cycle_model, state, direction)
  if not ok then
    show_error(state, tostring(result))
    return
  end
  if result == nil then
    local msg = #(state.scoped_models or {}) > 0 and "Only one model in scope"
      or "Only one model available"
    show_status(state, msg)
  else
    local thinking_str = (result.model.reasoning and result.thinkingLevel ~= "off")
      and (" (thinking: " .. result.thinkingLevel .. ")") or ""
    show_status(state, "Switched to " .. (result.model.name or result.model.id) .. thinking_str)
    maybe_warn_about_anthropic_subscription_auth(state, result.model)
  end
end

DEFAULT_KEYS.__scoped_models_policy = (function()
-- scoped-models-selector.ts + interactive-mode.ts showModelsSelector. Model
-- selection/order is session policy; only the settings store is mechanism.
local function full_model_id(model) return model.provider .. "/" .. model.id end

local function copy_list(values)
  local result = {}
  for index, value in ipairs(values or {}) do result[index] = value end
  return result
end

local function includes(values, target)
  if values == nil then return true end
  for _, value in ipairs(values) do if value == target then return true end end
  return false
end

local function resolve_exact_ids(ids, models)
  local by_id, result = {}, {}
  for _, model in ipairs(models) do by_id[full_model_id(model):lower()] = model end
  for _, id in ipairs(ids or {}) do
    local model = by_id[id:lower()]
    if model then result[#result + 1] = { model = model } end
  end
  return result
end

local function scoped_models_selector(opts)
  local theme = opts.theme
  local self = { focused = false, search = pi.tui.input(), selected = 0, dirty = false,
    models_by_id = {}, all_ids = {}, enabled_ids = opts.enabled_model_ids
      and copy_list(opts.enabled_model_ids) or nil, filtered = {} }
  for _, model in ipairs(opts.all_models) do
    local id = full_model_id(model)
    self.models_by_id[id] = model
    self.all_ids[#self.all_ids + 1] = id
  end

  local function sorted_ids()
    if self.enabled_ids == nil then return self.all_ids end
    local result, enabled = copy_list(self.enabled_ids), {}
    for _, id in ipairs(self.enabled_ids) do enabled[id] = true end
    for _, id in ipairs(self.all_ids) do if not enabled[id] then result[#result + 1] = id end end
    return result
  end

  local function build_items()
    local items = {}
    for _, id in ipairs(sorted_ids()) do
      local model = self.models_by_id[id]
      if model then items[#items + 1] = { fullId = id, model = model, enabled = includes(self.enabled_ids, id) } end
    end
    return items
  end

  local function refresh()
    local items, query = build_items(), self.search:value()
    self.filtered = query ~= "" and pi.tui.fuzzy_filter(items, query, function(item)
      return item.model.id .. " " .. item.model.provider
    end) or items
    self.selected = math.min(self.selected, math.max(0, #self.filtered - 1))
  end

  local function notify_change()
    opts.on_change(self.enabled_ids and copy_list(self.enabled_ids) or nil)
  end

  local function toggle(id)
    if self.enabled_ids == nil then return { id } end
    local result, found = {}, false
    for _, value in ipairs(self.enabled_ids) do
      if value == id then found = true else result[#result + 1] = value end
    end
    if not found then result[#result + 1] = id end
    return result
  end

  local function enable_all(target_ids)
    if self.enabled_ids == nil then return nil end
    local result = copy_list(self.enabled_ids)
    for _, id in ipairs(target_ids or self.all_ids) do
      if not includes(result, id) then result[#result + 1] = id end
    end
    return #result == #self.all_ids and nil or result
  end

  local function clear_all(target_ids)
    if self.enabled_ids == nil then
      if target_ids == nil then return {} end
      local targets, result = {}, {}
      for _, id in ipairs(target_ids) do targets[id] = true end
      for _, id in ipairs(self.all_ids) do if not targets[id] then result[#result + 1] = id end end
      return result
    end
    local targets, result = {}, {}
    for _, id in ipairs(target_ids or self.enabled_ids) do targets[id] = true end
    for _, id in ipairs(self.enabled_ids) do if not targets[id] then result[#result + 1] = id end end
    return result
  end

  local function provider_ids(provider)
    local result = {}
    for _, id in ipairs(self.all_ids) do
      if self.models_by_id[id].provider == provider then result[#result + 1] = id end
    end
    return result
  end

  local function footer_text()
    local enabled_count = self.enabled_ids and #self.enabled_ids or #self.all_ids
    local count = self.enabled_ids == nil and "all enabled"
      or (enabled_count .. "/" .. #self.all_ids .. " enabled")
    local parts = {
      select_key_text("confirm") .. " toggle",
      format_key(DEFAULT_KEYS["app.models.enableAll"], false) .. " all",
      format_key(DEFAULT_KEYS["app.models.clearAll"], false) .. " clear",
      format_key(DEFAULT_KEYS["app.models.toggleProvider"], false) .. " provider",
      format_key(DEFAULT_KEYS["app.models.reorderUp"], false) .. "/"
        .. format_key(DEFAULT_KEYS["app.models.reorderDown"], false) .. " reorder",
      format_key(DEFAULT_KEYS["app.models.save"], false) .. " save", count,
    }
    local text = "  " .. table.concat(parts, " · ")
    return self.dirty and theme:fg("dim", text .. " ") .. theme:fg("warning", "(unsaved)")
      or theme:fg("dim", text)
  end

  local function set_changed(enabled_ids)
    self.enabled_ids = enabled_ids
    self.dirty = true
    refresh()
    notify_change()
  end

  function self:set_focused(focused)
    self.focused = focused
    self.search:set_focused(focused)
  end

  function self:handle_input(data)
    if binding_matches(data, SELECT_KEYS.up) then
      if #self.filtered > 0 then self.selected = self.selected == 0 and #self.filtered - 1 or self.selected - 1 end
      return
    elseif binding_matches(data, SELECT_KEYS.down) then
      if #self.filtered > 0 then self.selected = self.selected == #self.filtered - 1 and 0 or self.selected + 1 end
      return
    end
    local reorder_up = binding_matches(data, DEFAULT_KEYS["app.models.reorderUp"])
    local reorder_down = binding_matches(data, DEFAULT_KEYS["app.models.reorderDown"])
    if reorder_up or reorder_down then
      if self.enabled_ids == nil then return end
      local item = self.filtered[self.selected + 1]
      if item and includes(self.enabled_ids, item.fullId) then
        local current
        for index, id in ipairs(self.enabled_ids) do if id == item.fullId then current = index end end
        local delta = reorder_up and -1 or 1
        local next_index = current and current + delta or 0
        if current and next_index >= 1 and next_index <= #self.enabled_ids then
          self.enabled_ids[current], self.enabled_ids[next_index] = self.enabled_ids[next_index], self.enabled_ids[current]
          self.dirty = true
          self.selected = self.selected + delta
          refresh()
          notify_change()
        end
      end
      return
    elseif binding_matches(data, SELECT_KEYS.confirm) then
      local item = self.filtered[self.selected + 1]
      if item then set_changed(toggle(item.fullId)) end
      return
    elseif binding_matches(data, DEFAULT_KEYS["app.models.enableAll"]) then
      local targets = self.search:value() ~= "" and (function()
        local ids = {}; for _, item in ipairs(self.filtered) do ids[#ids + 1] = item.fullId end; return ids
      end)() or nil
      set_changed(enable_all(targets))
      return
    elseif binding_matches(data, DEFAULT_KEYS["app.models.clearAll"]) then
      local targets = self.search:value() ~= "" and (function()
        local ids = {}; for _, item in ipairs(self.filtered) do ids[#ids + 1] = item.fullId end; return ids
      end)() or nil
      set_changed(clear_all(targets))
      return
    elseif binding_matches(data, DEFAULT_KEYS["app.models.toggleProvider"]) then
      local item = self.filtered[self.selected + 1]
      if item then
        local ids = provider_ids(item.model.provider)
        local all_enabled = true
        for _, id in ipairs(ids) do if not includes(self.enabled_ids, id) then all_enabled = false end end
        set_changed(all_enabled and clear_all(ids) or enable_all(ids))
      end
      return
    elseif binding_matches(data, DEFAULT_KEYS["app.models.save"]) then
      opts.on_persist(self.enabled_ids and copy_list(self.enabled_ids) or nil)
      self.dirty = false
      return
    elseif binding_matches(data, "ctrl+c") then
      if self.search:value() ~= "" then self.search:set_value(""); refresh() else opts.on_cancel() end
      return
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      opts.on_cancel()
      return
    end
    self.search:input(data)
    refresh()
  end

  function self:render(width)
    local lines = { dynamic_border_line(theme, width), "" }
    append(lines, pi.tui.text_render(theme:fg("accent", theme:bold("Model Configuration")), width, 0, 0))
    append(lines, pi.tui.text_render(theme:fg("muted", "Session-only. "
      .. format_key(DEFAULT_KEYS["app.models.save"], false) .. " to save to settings."), width, 0, 0))
    lines[#lines + 1] = ""
    append(lines, self.search:render(width))
    lines[#lines + 1] = ""
    if #self.filtered == 0 then
      append(lines, pi.tui.text_render(theme:fg("muted", "  No matching models"), width, 0, 0))
    else
      local max_visible = 8
      local start_index = math.max(0, math.min(self.selected - math.floor(max_visible / 2), #self.filtered - max_visible))
      local end_index = math.min(start_index + max_visible, #self.filtered)
      for i = start_index, end_index - 1 do
        local item = self.filtered[i + 1]
        local prefix = i == self.selected and theme:fg("accent", "→ ") or "  "
        local model_text = i == self.selected and theme:fg("accent", item.model.id) or item.model.id
        local badge = theme:fg("muted", " [" .. item.model.provider .. "]")
        local status = self.enabled_ids == nil and "" or (item.enabled
          and theme:fg("success", " ✓") or theme:fg("dim", " ✗"))
        append(lines, pi.tui.text_render(prefix .. model_text .. badge .. status, width, 0, 0))
      end
      if start_index > 0 or end_index < #self.filtered then
        append(lines, pi.tui.text_render(theme:fg("muted", "  ("
          .. (self.selected + 1) .. "/" .. #self.filtered .. ")"), width, 0, 0))
      end
      local selected = self.filtered[self.selected + 1]
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(theme:fg("muted", "  Model Name: "
        .. tostring(selected.model.name)), width, 0, 0))
    end
    lines[#lines + 1] = ""
    append(lines, pi.tui.text_render(footer_text(), width, 0, 0))
    lines[#lines + 1] = dynamic_border_line(theme, width)
    return lines
  end

  refresh()
  return self
end

local function show_scoped_models_selector(state)
  state.registry.refresh()
  local ok, all_models = pcall(state.registry.get_available)
  if not ok then all_models = {} end
  if #all_models == 0 then show_status(state, "No models available"); return end

  local enabled_ids = nil
  if #(state.scoped_models or {}) > 0 then
    enabled_ids = {}
    for _, scoped in ipairs(state.scoped_models) do enabled_ids[#enabled_ids + 1] = full_model_id(scoped.model) end
  else
    local patterns = pi.settings.enabled_models()
    if patterns and #patterns > 0 then
      local resolved = resolve_exact_ids(patterns, all_models)
      enabled_ids = {}
      for _, scoped in ipairs(resolved) do enabled_ids[#enabled_ids + 1] = full_model_id(scoped.model) end
    end
  end

  local function update_session_models(ids)
    if ids and #ids > 0 and #ids < #all_models then
      state.scoped_models = resolve_exact_ids(ids, all_models)
    else
      state.scoped_models = {}
    end
    update_available_provider_count(state)
  end

  show_selector(state, function(done)
    return scoped_models_selector({ theme = state.theme, all_models = all_models,
      enabled_model_ids = enabled_ids,
      on_change = update_session_models,
      on_persist = function(ids)
        local patterns = (ids == nil or #ids == #all_models) and nil or ids
        pi.settings.set_enabled_models(patterns)
        show_status(state, "Model selection saved to settings")
      end,
      on_cancel = function() done() end,
    })
  end)
end

local function initialize_scoped_models(state)
  local patterns = pi.settings.enabled_models()
  if not patterns or #patterns == 0 then return end
  local ok, models = pcall(state.registry.get_available)
  if not ok then return end
  local resolved = resolve_exact_ids(patterns, models)
  if #resolved > 0 and #resolved < #models then
    state.scoped_models = resolved
    -- main.ts/buildSessionOptions: a new non-CLI session starts on the
    -- first scoped model; resumed sessions restore their own model.
    if state.request and not state.request.modelFromCli and not state.request.sessionFile then
      state.model = resolved[1].model
    end
  end
end

return { show = show_scoped_models_selector, initialize = initialize_scoped_models,
  component = scoped_models_selector }
end)()

DEFAULT_KEYS.__settings_policy = (function()
-- components/settings-selector.ts. SettingsList/SelectList layout and input
-- are terminal mechanisms; setting meanings, submenu composition, and live
-- callbacks remain first-party Lua policy.
local HTTP_IDLE_TIMEOUT_CHOICES = {
  { label = "30 sec", timeoutMs = 30000 },
  { label = "1 min", timeoutMs = 60000 },
  { label = "2 min", timeoutMs = 120000 },
  { label = "5 min", timeoutMs = 300000 },
  { label = "disabled", timeoutMs = 0 },
}

local THINKING_DESCRIPTIONS = {
  off = "No reasoning",
  minimal = "Very brief reasoning (~1k tokens)",
  low = "Light reasoning (~2k tokens)",
  medium = "Moderate reasoning (~8k tokens)",
  high = "Deep reasoning (~16k tokens)",
  xhigh = "Maximum reasoning (~32k tokens)",
}

local DEFAULT_PROJECT_TRUST_LABELS = {
  ask = "Ask", always = "Always trust", never = "Never trust",
}

local function format_http_idle_timeout_ms(timeout)
  for _, choice in ipairs(HTTP_IDLE_TIMEOUT_CHOICES) do
    if choice.timeoutMs == timeout then return choice.label end
  end
  return tostring(timeout / 1000) .. " sec"
end

local function settings_list_theme(theme)
  return {
    label_selected_open = theme.fg_codes.accent, label_selected_close = "\27[39m",
    value_open = theme.fg_codes.muted, value_close = "\27[39m",
    value_selected_open = theme.fg_codes.accent, value_selected_close = "\27[39m",
    description_open = theme.fg_codes.dim, description_close = "\27[39m",
    hint_open = theme.fg_codes.dim, hint_close = "\27[39m",
    cursor = theme:fg("accent", "→ "),
  }
end

local function submenu_select_opts(theme)
  return {
    selected_open = theme.fg_codes.accent, selected_close = "\27[39m",
    description_open = theme.fg_codes.muted, description_close = "\27[39m",
    scroll_open = theme.fg_codes.muted, scroll_close = "\27[39m",
    no_match_open = theme.fg_codes.muted, no_match_close = "\27[39m",
    min_primary_column_width = 12, max_primary_column_width = 32,
  }
end

local function select_submenu(opts)
  -- Text children style their strings at construction; SelectList's theme
  -- callbacks read the global theme at render time. Rebuilding only the
  -- mechanism list after a preview preserves that distinction in Lua.
  local self = { focused = false }
  local title = opts.theme:bold(opts.theme:fg("accent", opts.title))
  local description = opts.theme:fg("muted", opts.description)
  local hint = opts.theme:fg("dim", "  Enter to select · Esc to go back")
  local function make_list(selected_value)
    local list = pi.tui.select_list(opts.options, math.min(#opts.options, 10),
      submenu_select_opts(opts.theme))
    for index, option in ipairs(opts.options) do
      if option.value == selected_value then list:set_selected_index(index - 1) end
    end
    return list
  end
  self.list = make_list(opts.current_value)
  function self:set_focused(focused) self.focused = focused end
  function self:handle_input(data)
    local action = self.list:input(data)
    if action == "confirm" then
      local item = self.list:selected()
      if item then opts.on_select(item.value) end
    elseif action == "cancel" then
      opts.on_cancel()
    elseif action == "changed" and opts.on_selection_change then
      local item = self.list:selected()
      if item then
        opts.on_selection_change(item.value)
        self.list = make_list(item.value)
      end
    end
  end
  function self:render(width)
    local lines = {}
    append(lines, pi.tui.text_render(title, width, 0, 0))
    if opts.description ~= "" then
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(description, width, 0, 0))
    end
    lines[#lines + 1] = ""
    append(lines, self.list:render(width))
    lines[#lines + 1] = ""
    append(lines, pi.tui.text_render(hint, width, 0, 0))
    return lines
  end
  return self
end

local function warning_settings_submenu(opts)
  local state = { anthropicExtraUsage = opts.warnings.anthropicExtraUsage }
  if state.anthropicExtraUsage == nil then state.anthropicExtraUsage = true end
  local list = pi.tui.settings_list({
    { id = "anthropic-extra-usage", label = "Anthropic extra usage",
      description = "Warn when Anthropic subscription auth may use paid extra usage",
      current_value = state.anthropicExtraUsage and "true" or "false",
      values = { "true", "false" } },
  }, 1, false, settings_list_theme(opts.theme))
  local self = { focused = false }
  function self:set_focused(focused) self.focused = focused end
  function self:handle_input(data)
    local action = list:input(data)
    if action.kind == "changed" then
      state.anthropicExtraUsage = action.value == "true"
      opts.on_change({ anthropicExtraUsage = state.anthropicExtraUsage })
    elseif action.kind == "cancel" then opts.on_cancel() end
  end
  function self:render(width) return list:render(width) end
  return self
end

local function settings_selector(opts)
  local config, callbacks, theme = opts.config, opts.callbacks, opts.theme
  local warnings = config.warnings or {}
  local items = {
    { id = "autocompact", label = "Auto-compact",
      description = "Automatically compact context when it gets too large",
      current_value = config.autoCompact and "true" or "false", values = { "true", "false" } },
  }
  local function add(item) items[#items + 1] = item end
  if config.supportsImages then
    add({ id = "show-images", label = "Show images", description = "Render images inline in terminal",
      current_value = config.showImages and "true" or "false", values = { "true", "false" } })
    add({ id = "image-width-cells", label = "Image width",
      description = "Preferred inline image width in terminal cells",
      current_value = tostring(config.imageWidthCells), values = { "60", "80", "120" } })
  end
  add({ id = "auto-resize-images", label = "Auto-resize images",
    description = "Resize large images to 2000x2000 max for better model compatibility",
    current_value = config.autoResizeImages and "true" or "false", values = { "true", "false" } })
  add({ id = "block-images", label = "Block images", description = "Prevent images from being sent to LLM providers",
    current_value = config.blockImages and "true" or "false", values = { "true", "false" } })
  add({ id = "skill-commands", label = "Skill commands", description = "Register skills as /skill:name commands",
    current_value = config.enableSkillCommands and "true" or "false", values = { "true", "false" } })
  add({ id = "show-hardware-cursor", label = "Show hardware cursor",
    description = "Show the terminal cursor while still positioning it for IME support",
    current_value = config.showHardwareCursor and "true" or "false", values = { "true", "false" } })
  add({ id = "editor-padding", label = "Editor padding", description = "Horizontal padding for input editor (0-3)",
    current_value = tostring(config.editorPaddingX), values = { "0", "1", "2", "3" } })
  add({ id = "autocomplete-max-visible", label = "Autocomplete max items",
    description = "Max visible items in autocomplete dropdown (3-20)",
    current_value = tostring(config.autocompleteMaxVisible), values = { "3", "5", "7", "10", "15", "20" } })
  add({ id = "clear-on-shrink", label = "Clear on shrink",
    description = "Clear empty rows when content shrinks (may cause flicker)",
    current_value = config.clearOnShrink and "true" or "false", values = { "true", "false" } })
  add({ id = "terminal-progress", label = "Terminal progress",
    description = "Show OSC 9;4 progress indicators in the terminal tab bar",
    current_value = config.showTerminalProgress and "true" or "false", values = { "true", "false" } })
  add({ id = "steering-mode", label = "Steering mode",
    description = "Enter while streaming queues steering messages. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once.",
    current_value = config.steeringMode, values = { "one-at-a-time", "all" } })
  add({ id = "follow-up-mode", label = "Follow-up mode",
    description = format_key(DEFAULT_KEYS["app.message.followUp"], false) .. " queues follow-up messages until agent stops. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once.",
    current_value = config.followUpMode, values = { "one-at-a-time", "all" } })
  add({ id = "transport", label = "Transport",
    description = "Preferred transport for providers that support multiple transports",
    current_value = config.transport, values = { "sse", "websocket", "websocket-cached", "auto" } })
  local timeout_values = {}
  for _, choice in ipairs(HTTP_IDLE_TIMEOUT_CHOICES) do timeout_values[#timeout_values + 1] = choice.label end
  add({ id = "http-idle-timeout", label = "HTTP idle timeout",
    description = "Maximum idle gap while waiting for HTTP headers or body chunks. Disable for local models that pause longer than five minutes.",
    current_value = format_http_idle_timeout_ms(config.httpIdleTimeoutMs), values = timeout_values })
  add({ id = "hide-thinking", label = "Hide thinking", description = "Hide thinking blocks in assistant responses",
    current_value = config.hideThinkingBlock and "true" or "false", values = { "true", "false" } })
  add({ id = "collapse-changelog", label = "Collapse changelog", description = "Show condensed changelog after updates",
    current_value = config.collapseChangelog and "true" or "false", values = { "true", "false" } })
  add({ id = "quiet-startup", label = "Quiet startup", description = "Disable verbose printing at startup",
    current_value = config.quietStartup and "true" or "false", values = { "true", "false" } })
  add({ id = "install-telemetry", label = "Install telemetry",
    description = "Send an anonymous version/update ping after changelog-detected updates",
    current_value = config.enableInstallTelemetry and "true" or "false", values = { "true", "false" } })
  add({ id = "default-project-trust", label = "Default project trust",
    description = "Fallback behavior when no extension or saved trust decision decides project trust",
    current_value = DEFAULT_PROJECT_TRUST_LABELS[config.defaultProjectTrust],
    values = { "Ask", "Always trust", "Never trust" } })
  add({ id = "double-escape-action", label = "Double-escape action",
    description = "Action when pressing Escape twice with empty editor", current_value = config.doubleEscapeAction,
    values = { "tree", "fork", "none" } })
  add({ id = "tree-filter-mode", label = "Tree filter mode", description = "Default filter when opening /tree",
    current_value = config.treeFilterMode, values = { "default", "no-tools", "user-only", "labeled-only", "all" } })
  add({ id = "warnings", label = "Warnings", description = "Enable or disable individual warnings",
    current_value = "configure", submenu = true })
  add({ id = "thinking", label = "Thinking level", description = "Reasoning depth for thinking-capable models",
    current_value = config.thinkingLevel, submenu = true })
  add({ id = "theme", label = "Theme", description = "Color theme for the interface",
    current_value = config.currentTheme, submenu = true })

  local settings_cursor = theme:fg("accent", "→ ")
  local function make_settings_list(query, selected_id)
    local list_theme = settings_list_theme(theme)
    list_theme.cursor = settings_cursor
    local result = pi.tui.settings_list(items, 10, true, list_theme)
    if query and query ~= "" then result:input(query) end
    if selected_id then result:select_id(selected_id) end
    return result
  end
  local list = make_settings_list()
  local self = { focused = false, submenu = nil }
  function self:set_focused(focused)
    self.focused = focused
    if self.submenu then self.submenu:set_focused(focused) end
  end
  local function close_submenu(value, id)
    self.submenu = nil
    if value ~= nil and id then
      list:update_value(id, value)
      for _, item in ipairs(items) do if item.id == id then item.current_value = value end end
    end
    if id == "theme" then list = make_settings_list(list:query(), id) end
  end
  local function open_submenu(id, current)
    if id == "warnings" then
      self.submenu = warning_settings_submenu({ theme = theme, warnings = warnings,
        on_change = function(next_warnings) warnings = next_warnings; callbacks.onWarningsChange(next_warnings) end,
        on_cancel = function() close_submenu(nil) end })
    elseif id == "thinking" then
      local options = {}
      for _, level in ipairs(config.availableThinkingLevels) do
        options[#options + 1] = { value = level, label = level, description = THINKING_DESCRIPTIONS[level] }
      end
      self.submenu = select_submenu({ theme = theme, title = "Thinking Level",
        description = "Select reasoning depth for thinking-capable models", options = options,
        current_value = current, on_select = function(value)
          callbacks.onThinkingLevelChange(value); close_submenu(value, id)
        end, on_cancel = function() close_submenu(nil) end })
    elseif id == "theme" then
      local options = {}
      for _, name in ipairs(config.availableThemes) do options[#options + 1] = { value = name, label = name } end
      self.submenu = select_submenu({ theme = theme, title = "Theme", description = "Select color theme",
        options = options, current_value = current, on_select = function(value)
          callbacks.onThemeChange(value); close_submenu(value, id)
        end, on_cancel = function() callbacks.onThemePreview(current); close_submenu(nil) end,
        on_selection_change = callbacks.onThemePreview })
    end
    if self.submenu then self.submenu:set_focused(self.focused) end
  end
  function self:handle_input(data)
    if self.submenu then self.submenu:handle_input(data); return end
    local action = list:input(data)
    if action.kind == "changed" then callbacks.onChange(action.id, action.value)
    elseif action.kind == "submenu" then open_submenu(action.id, action.value)
    elseif action.kind == "cancel" then callbacks.onCancel() end
  end
  function self:render(width)
    local lines = { dynamic_border_line(theme, width) }
    append(lines, self.submenu and self.submenu:render(width) or list:render(width))
    lines[#lines + 1] = dynamic_border_line(theme, width)
    return lines
  end
  return self
end

local function apply_theme(state, name)
  local data
  if name == "dark" then data = dark_json elseif name == "light" then data = light_json
  else return false, "Theme not found: " .. name end
  local replacement = create_theme(data, state.theme.mode)
  for key in pairs(state.theme) do state.theme[key] = nil end
  for key, value in pairs(replacement) do state.theme[key] = value end
  state.md_theme = get_markdown_theme(state.theme)
  state.theme_data = data
  if state.editor then
    state.editor.theme = state.theme
    state.editor.editor:set_select_list_theme(get_select_list_theme(state.theme))
    state.editor_border_key = nil
  end
  return true
end

local function show_settings_selector(state)
  show_selector(state, function(done)
    local caps = pi.tui.terminal_capabilities()
    local current_theme = pi.settings.theme() or "dark"
    local selector = settings_selector({ theme = state.theme,
      config = {
        autoCompact = pi.settings.compaction_enabled(), supportsImages = caps.images ~= nil,
        showImages = pi.settings.show_images(), imageWidthCells = pi.settings.image_width_cells(),
        autoResizeImages = pi.settings.image_auto_resize(), blockImages = pi.settings.block_images(),
        enableSkillCommands = pi.settings.enable_skill_commands(),
        steeringMode = state.agent and state.agent:get_steering_mode() or pi.settings.steering_mode(),
        followUpMode = state.agent and state.agent:get_follow_up_mode() or pi.settings.follow_up_mode(),
        transport = pi.settings.transport(), httpIdleTimeoutMs = pi.settings.http_idle_timeout_ms(),
        thinkingLevel = state.thinking_level,
        availableThinkingLevels = get_available_thinking_levels(state),
        currentTheme = current_theme, availableThemes = { "dark", "light" },
        hideThinkingBlock = state.hide_thinking_block or pi.settings.hide_thinking_block(),
        collapseChangelog = pi.settings.collapse_changelog(),
        enableInstallTelemetry = pi.settings.enable_install_telemetry(),
        doubleEscapeAction = state.double_escape_action or pi.settings.double_escape_action(),
        treeFilterMode = pi.settings.tree_filter_mode(),
        showHardwareCursor = pi.settings.show_hardware_cursor(),
        editorPaddingX = pi.settings.editor_padding_x(),
        autocompleteMaxVisible = pi.settings.autocomplete_max_visible(),
        quietStartup = pi.settings.quiet_startup(), clearOnShrink = pi.settings.clear_on_shrink(),
        showTerminalProgress = pi.settings.show_terminal_progress(),
        defaultProjectTrust = pi.settings.default_project_trust(), warnings = pi.settings.warnings(),
      },
      callbacks = {
        onChange = function(id, value)
          local enabled = value == "true"
          if id == "autocompact" then pi.settings.set_compaction_enabled(enabled)
          elseif id == "show-images" then pi.settings.set_show_images(enabled)
          elseif id == "image-width-cells" then pi.settings.set_image_width_cells(tonumber(value))
          elseif id == "auto-resize-images" then pi.settings.set_image_auto_resize(enabled)
          elseif id == "block-images" then pi.settings.set_block_images(enabled)
          elseif id == "skill-commands" then pi.settings.set_enable_skill_commands(enabled)
          elseif id == "steering-mode" then
            pi.settings.set_steering_mode(value); if state.agent then state.agent:set_steering_mode(value) end
          elseif id == "follow-up-mode" then
            pi.settings.set_follow_up_mode(value); if state.agent then state.agent:set_follow_up_mode(value) end
          elseif id == "transport" then
            pi.settings.set_transport(value); if state.agent then state.agent:set_transport(value) end
          elseif id == "http-idle-timeout" then
            for _, choice in ipairs(HTTP_IDLE_TIMEOUT_CHOICES) do
              if choice.label == value then
                pi.settings.set_http_idle_timeout_ms(choice.timeoutMs)
                show_status(state, "HTTP idle timeout: " .. format_http_idle_timeout_ms(choice.timeoutMs))
              end
            end
          elseif id == "hide-thinking" then state.hide_thinking_block = enabled; pi.settings.set_hide_thinking_block(enabled)
          elseif id == "collapse-changelog" then pi.settings.set_collapse_changelog(enabled)
          elseif id == "quiet-startup" then pi.settings.set_quiet_startup(enabled)
          elseif id == "install-telemetry" then pi.settings.set_enable_install_telemetry(enabled)
          elseif id == "default-project-trust" then
            local by_label = { Ask = "ask", ["Always trust"] = "always", ["Never trust"] = "never" }
            pi.settings.set_default_project_trust(by_label[value])
          elseif id == "double-escape-action" then state.double_escape_action = value; pi.settings.set_double_escape_action(value)
          elseif id == "tree-filter-mode" then pi.settings.set_tree_filter_mode(value)
          elseif id == "show-hardware-cursor" then pi.settings.set_show_hardware_cursor(enabled)
          elseif id == "editor-padding" then
            local padding = tonumber(value); pi.settings.set_editor_padding_x(padding); state.editor.editor:set_padding_x(padding)
          elseif id == "autocomplete-max-visible" then
            local visible = tonumber(value); pi.settings.set_autocomplete_max_visible(visible)
            state.editor.editor:set_autocomplete_max_visible(visible)
          elseif id == "clear-on-shrink" then pi.settings.set_clear_on_shrink(enabled)
          elseif id == "terminal-progress" then pi.settings.set_show_terminal_progress(enabled) end
        end,
        onThinkingLevelChange = function(level) session_set_thinking_level(state, level) end,
        onThemeChange = function(name)
          local ok, err = apply_theme(state, name); pi.settings.set_theme(name)
          if not ok then show_error(state, 'Failed to load theme "' .. name .. '": ' .. err .. '\nFell back to dark theme.') end
        end,
        onThemePreview = function(name) apply_theme(state, name) end,
        onWarningsChange = function(next_warnings) pi.settings.set_warnings(next_warnings) end,
        onCancel = function() done() end,
      },
    })
    return selector, selector
  end)
end
  return { show = show_settings_selector }
end)()

DEFAULT_KEYS.__trust_policy = (function()
-- components/trust-selector.ts + interactive-mode.ts showTrustSelector.
-- Persistence/discovery is the public pi.trust mechanism; this component and
-- all command/status behavior remain replaceable Lua policy.
local function trust_selector(options)
  local self = { focused = false, selected = 1 }
  local trust_options = pi.trust.options(options.cwd, false)
  local saved = options.saved_decision
  local function is_saved(option)
    return option.savedPath ~= nil and saved ~= nil
      and saved.decision == option.trusted and saved.path == option.savedPath
  end
  for index, option in ipairs(trust_options) do
    if is_saved(option) then self.selected = index; break end
  end
  function self:set_focused(focused) self.focused = focused end
  function self:handle_input(data)
    if binding_matches(data, SELECT_KEYS.up) or data == "k" then
      self.selected = math.max(1, self.selected - 1)
    elseif binding_matches(data, SELECT_KEYS.down) or data == "j" then
      self.selected = math.min(#trust_options, self.selected + 1)
    elseif binding_matches(data, SELECT_KEYS.confirm) or data == "\n" then
      local selected = trust_options[self.selected]
      if selected then options.on_select({ trusted = selected.trusted, updates = selected.updates }) end
    elseif binding_matches(data, SELECT_KEYS.cancel) then options.on_cancel() end
  end
  function self:render(width)
    local theme = options.theme
    local lines = { dynamic_border_line(theme, width), "" }
    append(lines, pi.tui.text_render(theme:fg("accent", theme:bold("Project trust")), width, 1, 0))
    append(lines, pi.tui.text_render(theme:fg("muted", options.cwd), width, 1, 0))
    lines[#lines + 1] = ""
    local saved_text = "none"
    if saved then
      saved_text = saved.decision and "trusted" or "untrusted"
      if saved.path ~= pi.trust.path(options.cwd) then
        saved_text = saved_text .. " (inherited from " .. saved.path .. ")"
      else saved_text = saved_text .. " (" .. saved.path .. ")" end
    end
    append(lines, pi.tui.text_render(theme:fg("muted", "Saved decision: " .. saved_text), width, 1, 0))
    append(lines, pi.tui.text_render(theme:fg("muted", "Current session: "
      .. (options.project_trusted and "trusted" or "untrusted")), width, 1, 0))
    lines[#lines + 1] = ""
    for index, option in ipairs(trust_options) do
      local selected = index == self.selected
      local prefix = selected and theme:fg("accent", "→ ") or "  "
      local label = theme:fg(selected and "accent" or "text", option.label)
      local check = is_saved(option) and theme:fg("success", " ✓") or ""
      append(lines, pi.tui.text_render(prefix .. label .. check, width, 1, 0))
    end
    lines[#lines + 1] = ""
    local hints = raw_key_hint(theme, "↑↓", "navigate") .. "  "
      .. select_key_hint(theme, "confirm", "save") .. "  "
      .. select_key_hint(theme, "cancel", "cancel")
    append(lines, pi.tui.text_render(hints, width, 1, 0))
    lines[#lines + 1] = ""
    lines[#lines + 1] = dynamic_border_line(theme, width)
    return lines
  end
  return self
end

local function show_trust_selector(state)
  local cwd = state.session_manager and state.session_manager:get_cwd() or state.cwd
  local saved = pi.trust.get_entry(cwd)
  show_selector(state, function(done)
    local selector = trust_selector({ theme = state.theme, cwd = cwd,
      saved_decision = saved, project_trusted = state.project_trusted,
      on_select = function(selection)
        pi.trust.set_many(selection.updates)
        done()
        show_status(state, "Saved trust decision: "
          .. (selection.trusted and "trusted" or "untrusted")
          .. ". Restart pi for this to take effect.")
      end,
      on_cancel = function() done() end,
    })
    return selector, selector
  end)
end
  return { show = show_trust_selector, selector = trust_selector }
end)()

-- interactive-mode.ts setupEditorSubmitHandler — the "/" command routing
-- skeleton. Builtin commands are routed here only once their dialogs land
-- (item 7 the rest); pi has no pre-dialog behavior for them, so until
-- then they fall through with extension and unknown "/" commands to the
-- prompt path, exactly as pi's fallthrough does.
local function handle_submit(text, actions)
  text = trim(text)
  if text == "" then return end
  if text == "/scoped-models" then
    actions.scoped_models_command()
    actions.set_text("")
    return
  end
  if text == "/settings" then
    actions.settings_command()
    actions.set_text("")
    return
  end
  if text == "/model" or text:sub(1, 7) == "/model " then
    local search_term = text:sub(1, 7) == "/model " and trim(text:sub(8)) or nil
    if search_term == "" then search_term = nil end
    actions.set_text("")
    actions.model_command(search_term)
    return
  end
  if text == "/export" or text:sub(1, 8) == "/export " then
    actions.export_command(text)
    actions.set_text("")
    return
  end
  if text == "/import" or text:sub(1, 8) == "/import " then
    actions.import_command(text)
    actions.set_text("")
    return
  end

  if text == "/share" then
    actions.share_command()
    actions.set_text("")
    return
  end

  if text == "/copy" then
    actions.copy_command()
    actions.set_text("")
    return
  end

  if text == "/login" then
    actions.show_oauth_selector("login")
    actions.set_text("")
    return
  end
  if text == "/logout" then
    actions.show_oauth_selector("logout")
    actions.set_text("")
    return
  end
  if text == "/name" or text:sub(1, 6) == "/name " then
    actions.name_command(text)
    actions.set_text("")
    return
  end
  if text == "/session" then
    actions.session_command()
    actions.set_text("")
    return
  end
  if text == "/changelog" then
    actions.changelog_command()
    actions.set_text("")
    return
  end
  if text == "/hotkeys" then
    actions.hotkeys_command()
    actions.set_text("")
    return
  end
  if text == "/reload" then
    actions.set_text("")
    actions.reload_command()
    return
  end
  if text == "/debug" then
    actions.debug_command()
    actions.set_text("")
    return
  end
  if text == "/arminsayshi" then
    actions.armin_command()
    actions.set_text("")
    return
  end
  if text == "/dementedelves" then
    actions.earendil_command()
    actions.set_text("")
    return
  end
  if text == "/new" then
    actions.set_text("")
    actions.clear_command()
    return
  end
  if text == "/compact" or text:sub(1, 9) == "/compact " then
    local custom_instructions = text:sub(1, 9) == "/compact " and trim(text:sub(10)) or nil
    actions.set_text("")
    actions.compact_command(custom_instructions)
    return
  end
  if text == "/resume" then
    actions.resume_command()
    actions.set_text("")
    return
  end
  if text == "/fork" then
    actions.fork_command()
    actions.set_text("")
    return
  end
  if text == "/clone" then
    actions.set_text("")
    actions.clone_command()
    return
  end
  if text == "/tree" then
    actions.tree_command()
    actions.set_text("")
    return
  end
  if text == "/trust" then
    actions.trust_command()
    actions.set_text("")
    return
  end

  if text == "/quit" then
    actions.set_text("")
    actions.quit()
    return
  end
  -- Handle bash command (! for normal, !! for excluded from context).
  if text:sub(1, 1) == "!" then
    local is_excluded = text:sub(1, 2) == "!!"
    local command = trim(text:sub(is_excluded and 3 or 2))
    if command ~= "" then
      if actions.is_bash_running() then
        actions.show_warning("A bash command is already running. Press Esc to cancel it first.")
        actions.set_text(text)
        return
      end
      actions.add_to_history(text)
      actions.bash_command(command, is_excluded)
      return
    end
  end
  if text:sub(1, 1) == "/" and actions.extension_command then
    local handled = actions.extension_command(text)
    if handled then
      actions.add_to_history(text)
      actions.set_text("")
      return
    end
  end
  actions.prompt(text)
end

pi.register_command("interactive-submit-route", {
  description = "Exercise the handleSubmit command-routing skeleton",
  handler = function(args)
    local request = pi.json.decode(args)
    local trace = {}
    for _, text in ipairs(request.texts or {}) do
      handle_submit(text, {
        set_text = function(value) trace[#trace + 1] = { action = "set_text", value = value } end,
        quit = function() trace[#trace + 1] = { action = "quit" } end,
        extension_command = function(value)
          local handled, err = EXTENSION_POLICY.execute_command(value, {
            cwd = request.cwd or pi.cwd(), mode = "interactive", hasUI = true,
          })
          if handled or err then
            trace[#trace + 1] = { action = "extension_command", value = value,
              handled = handled, error = err }
          end
          return handled
        end,
        prompt = function(value) trace[#trace + 1] = { action = "prompt", value = value } end,
        show_oauth_selector = function(mode)
          trace[#trace + 1] = { action = "show_oauth_selector", value = mode }
        end,
        model_command = function(search)
          trace[#trace + 1] = { action = "model_command", value = search }
        end,
        compact_command = function(custom)
          trace[#trace + 1] = { action = "compact_command", value = custom }
        end,
        export_command = function(value)
          trace[#trace + 1] = { action = "export_command", value = value }
        end,
        import_command = function(value)
          trace[#trace + 1] = { action = "import_command", value = value }
        end,

        changelog_command = function() trace[#trace + 1] = { action = "changelog_command" } end,
        hotkeys_command = function() trace[#trace + 1] = { action = "hotkeys_command" } end,
        debug_command = function() trace[#trace + 1] = { action = "debug_command" } end,
        reload_command = function() trace[#trace + 1] = { action = "reload_command" } end,
        armin_command = function() trace[#trace + 1] = { action = "armin_command" } end,
        earendil_command = function() trace[#trace + 1] = { action = "earendil_command" } end,
        share_command = function() trace[#trace + 1] = { action = "share_command" } end,
        copy_command = function() trace[#trace + 1] = { action = "copy_command" } end,
        is_bash_running = function() return request.bashRunning or false end,
        show_warning = function(message)
          trace[#trace + 1] = { action = "show_warning", value = message }
        end,
        add_to_history = function(value)
          trace[#trace + 1] = { action = "add_to_history", value = value }
        end,
        bash_command = function(command, excluded)
          trace[#trace + 1] = { action = "bash_command", value = command, excluded = excluded }
        end,
        fork_command = function() trace[#trace + 1] = { action = "fork_command" } end,
        clone_command = function() trace[#trace + 1] = { action = "clone_command" } end,
        tree_command = function() trace[#trace + 1] = { action = "tree_command" } end,
        trust_command = function() trace[#trace + 1] = { action = "trust_command" } end,
      })
    end
    return { trace = trace }
  end,
})

-- ===========================================================================
-- Autocomplete — core/slash-commands.ts BUILTIN_SLASH_COMMANDS plus
-- interactive-mode.ts createBaseAutocompleteProvider over the
-- pi.tui.autocomplete_provider mechanism (CombinedAutocompleteProvider).
-- ===========================================================================

local BUILTIN_SLASH_COMMANDS = {
  { name = "settings", description = "Open settings menu" },
  { name = "model", description = "Select model (opens selector UI)" },
  { name = "scoped-models", description = "Enable/disable models for Ctrl+P cycling" },
  { name = "export", description = "Export session (HTML default, or specify path: .html/.jsonl)" },
  { name = "import", description = "Import and resume a session from a JSONL file" },
  { name = "share", description = "Share session as a secret GitHub gist" },
  { name = "copy", description = "Copy last agent message to clipboard" },
  { name = "name", description = "Set session display name" },
  { name = "session", description = "Show session info and stats" },
  { name = "changelog", description = "Show changelog entries" },
  { name = "hotkeys", description = "Show all keyboard shortcuts" },
  { name = "fork", description = "Create a new fork from a previous user message" },
  { name = "clone", description = "Duplicate the current session at the current position" },
  { name = "tree", description = "Navigate session tree (switch branches)" },
  { name = "trust", description = "Save project trust decision for future sessions" },
  { name = "login", description = "Configure provider authentication" },
  { name = "logout", description = "Remove provider authentication" },
  { name = "new", description = "Start a new session" },
  { name = "compact", description = "Manually compact the session context" },
  { name = "resume", description = "Resume a different session" },
  { name = "reload", description = "Reload keybindings, extensions, skills, prompts, and themes" },
  { name = "quit", description = "Quit pi" },
}

-- utils/tools-manager.ts ensureTool, system-binary slice: probe `fd` then
-- `fdfind` on PATH. The GitHub-release download fallback lands with the
-- tools milestone (items 5/7).
local function resolve_fd_path()
  for _, name in ipairs({ "fd", "fdfind" }) do
    local probe = pi.exec("which", { name })
    if probe.code == 0 then
      local path = (probe.stdout or ""):match("[^\r\n]+")
      if path and path ~= "" then return path end
    end
  end
  return nil
end

-- interactive-mode.ts createBaseAutocompleteProvider: builtin commands plus
-- /model argument completions (fuzzy over "<id> <provider>"). Prompt
-- templates, extension commands, and skill commands join with their
-- milestones (items 7/9).
local function create_base_autocomplete_provider(state)
  local commands = {}
  for index, command in ipairs(BUILTIN_SLASH_COMMANDS) do
    commands[index] = { name = command.name, description = command.description }
    if command.name == "model" then
      commands[index].get_argument_completions = function(prefix)
        local models
        if #(state.scoped_models or {}) > 0 then
          models = {}
          for position, scoped in ipairs(state.scoped_models) do models[position] = scoped.model end
        else
          local ok, available = pcall(state.registry.get_available)
          models = ok and available or {}
        end
        if #models == 0 then return nil end
        local items = {}
        for position, model in ipairs(models) do
          items[position] = {
            id = model.id,
            provider = model.provider,
            label = model.provider .. "/" .. model.id,
          }
        end
        local filtered = pi.tui.fuzzy_filter(items, prefix, function(item)
          return item.id .. " " .. item.provider
        end)
        if #filtered == 0 then return nil end
        local results = {}
        for position, item in ipairs(filtered) do
          results[position] = { value = item.label, label = item.id, description = item.provider }
        end
        return results
      end
    end
  end
  local builtin_names = {}
  for _, command in ipairs(BUILTIN_SLASH_COMMANDS) do builtin_names[command.name] = true end
  for _, command in ipairs(pi.registered_extension_commands()) do
    if not builtin_names[command.name] then
      commands[#commands + 1] = {
        name = command.invocation_name,
        description = command.description,
        get_argument_completions = command.get_argument_completions,
      }
    end
  end

  return pi.tui.autocomplete_provider({
    commands = commands,
    base_path = state.cwd,
    fd_path = state.fd_path,
  })
end

-- Resolve one editor autocomplete request against the provider, mirroring
-- editor.ts requestAutocomplete → runAutocompleteRequest (the mechanism
-- validates staleness inside apply_autocomplete). Forced (file) completion
-- consults shouldTriggerFileCompletion and bails without touching the UI.
local function resolve_editor_autocomplete(editor, provider, request)
  if request.force
    and not provider:should_trigger_file_completion(request.lines, request.cursor_line, request.cursor_col)
  then
    return false
  end
  local suggestions = provider:get_suggestions(
    request.lines, request.cursor_line, request.cursor_col, { force = request.force })
  local response = editor:apply_autocomplete(request.id, suggestions)
  return response.accepted
end

-- The interactive pump: stash the newest request and resolve it once the
-- spec's debounce deadline (editor.ts getAutocompleteDebounceMs) passes.
local function pump_editor_autocomplete(state, now_ms)
  local editor = state.editor.editor
  local request = editor:take_autocomplete_request()
  if request then
    state.pending_autocomplete = { request = request, due = now_ms + (request.debounce_ms or 0) }
  end
  local pending = state.pending_autocomplete
  if not pending or now_ms < pending.due then return false end
  state.pending_autocomplete = nil
  return resolve_editor_autocomplete(editor, state.autocomplete_provider, pending.request)
end

-- ===========================================================================
-- Interaction shell — interactive-mode.ts setupKeyHandlers, handleCtrlC/D,
-- queue display/restore, the working loader, and init()'s container
-- composition (headerContainer, chatContainer, pendingMessagesContainer,
-- statusContainer, widget containers, editorContainer, footer).
-- ===========================================================================

local function user_message(text)
  return { role = "user", content = { { type = "text", text = text } }, timestamp = os.time() * 1000 }
end

-- interactive-mode.ts getAllQueuedMessages: the session queues plus the
-- messages queued while compaction ran.
local function all_queued_messages(state)
  local steering, follow_up = {}, {}
  for _, text in ipairs(state.steering_texts) do steering[#steering + 1] = text end
  for _, text in ipairs(state.follow_up_texts) do follow_up[#follow_up + 1] = text end
  for _, queued in ipairs(state.compaction_queued or {}) do
    if queued.mode == "steer" then steering[#steering + 1] = queued.text
    else follow_up[#follow_up + 1] = queued.text end
  end
  return steering, follow_up
end

-- agent-session.ts clearQueue + interactive-mode.ts clearAllQueues
-- (session queues plus the compaction queue).
local function clear_all_queues(state)
  local steering, follow_up = state.steering_texts, state.follow_up_texts
  state.steering_texts, state.follow_up_texts = {}, {}
  state.session.clear_queues()
  for _, queued in ipairs(state.compaction_queued or {}) do
    if queued.mode == "steer" then steering[#steering + 1] = queued.text
    else follow_up[#follow_up + 1] = queued.text end
  end
  state.compaction_queued = {}
  return steering, follow_up
end

-- interactive-mode.ts restoreQueuedMessagesToEditor.
local function restore_queued_messages_to_editor(state, options)
  options = options or {}
  clear_pending_bash_display(state) -- updatePendingMessagesDisplay
  local steering, follow_up = clear_all_queues(state)
  local all = {}
  for _, text in ipairs(steering) do all[#all + 1] = text end
  for _, text in ipairs(follow_up) do all[#all + 1] = text end
  if #all == 0 then
    if options.abort then state.session.abort() end
    return 0
  end
  local queued_text = table.concat(all, "\n\n")
  local current = options.current_text or state.editor.editor:get_text()
  local parts = {}
  for _, part in ipairs({ queued_text, current }) do
    if trim(part) ~= "" then parts[#parts + 1] = part end
  end
  state.editor.editor:set_text(table.concat(parts, "\n\n"))
  if options.abort then state.session.abort() end
  return #all
end

-- interactive-mode.ts shutdown()'s reachable slice: stop the loop. Input
-- draining and the resume-command line join with the sessions milestone
-- (item 6).
local function shutdown(state)
  if state.session_manager and not state.extension_shutdown_emitted
    and EXTENSION_CONTEXT_POLICY then
    state.extension_shutdown_emitted = true
    EXTENSION_POLICY.emit_generic({ type = "session_shutdown", reason = "quit" },
      EXTENSION_CONTEXT_POLICY.snapshot(state))
  end
  state.exit = true
end

-- interactive-mode.ts handleCtrlC: a second ctrl+c within 500ms exits;
-- otherwise clear the editor (no status chrome).
local function handle_ctrl_c(state)
  local now = pi.monotonic_ms()
  if state.last_sigint_time and now - state.last_sigint_time < 500 then
    shutdown(state)
  else
    state.editor.editor:set_text("")
    state.last_sigint_time = now
  end
end

-- interactive-mode.ts setToolsExpanded: the active header (ExpandableText)
-- and every expandable chat child re-render from state each frame.
local function set_tools_expanded(state, expanded)
  state.tools_expanded = expanded
  state.header_expanded = expanded
  -- setToolsExpanded walks chatContainer children only: bash rows in the
  -- transcript track the toggle; deferred rows in the pending container
  -- keep their expansion until flushed and toggled again.
  for _, item in ipairs(state.transcript) do
    if item.kind == "bash" then item.expanded = expanded end
  end
end

-- Extension UI is snapshot-in/actions-out: handlers enqueue plain Lua action
-- tables and may await their settlement; only the frontend pump mutates state.
EXTENSION_UI_POLICY = {}

function EXTENSION_UI_POLICY.enqueue(state, action)
  state.extension_ui_actions[#state.extension_ui_actions + 1] = action
  if state.extension_ui_trace then
    local trace = { type = action.kind }
    if action.kind == "notify" then
      trace.message, trace.level = action.message, action.level
    else
      trace.title, trace.options = action.title, action.options
    end
    state.extension_ui_trace[#state.extension_ui_trace + 1] = trace
  end
  state.async_render = true
end

function EXTENSION_UI_POLICY.context(state)
  local function await_dialog(kind, title, options)
    local action = { kind = kind, title = title, options = options, settled = false }
    EXTENSION_UI_POLICY.enqueue(state, action)
    while not action.settled do pi.sleep(10) end
    return action.value
  end
  return {
    select = function(title, options)
      return await_dialog("select", title, options)
    end,
    confirm = function(title, message)
      return await_dialog("confirm", title .. "\n" .. message, { "Yes", "No" }) == "Yes"
    end,
    notify = function(message, kind)
      EXTENSION_UI_POLICY.enqueue(state, { kind = "notify", message = message, level = kind })
    end,
  }
end

function EXTENSION_UI_POLICY.settle(state, action, value)
  if action.settled then return end
  action.value = value
  action.settled = true
  if state.extension_ui_trace then
    state.extension_ui_trace[#state.extension_ui_trace + 1] = {
      type = action.kind .. "_result", value = value,
    }
  end
  state.extension_ui_active = nil
  state.async_render = true
end

function EXTENSION_UI_POLICY.pump(state)
  if state.extension_ui_active then return false end
  local changed = false
  while #state.extension_ui_actions > 0 and not state.extension_ui_active do
    local action = table.remove(state.extension_ui_actions, 1)
    if action.kind == "notify" then
      if action.level == "error" then show_error(state, action.message)
      elseif action.level == "warning" then show_warning(state, action.message)
      else show_status(state, action.message) end
      changed = true
    else
      state.extension_ui_active = action
      show_selector(state, function(done)
        return extension_selector({
          theme = state.theme,
          title = action.title,
          options = action.options,
          on_toggle_tools_expanded = function()
            set_tools_expanded(state, not state.tools_expanded)
          end,
          on_select = function(value)
            done()
            EXTENSION_UI_POLICY.settle(state, action, value)
          end,
          on_cancel = function()
            done()
            EXTENSION_UI_POLICY.settle(state, action, nil)
          end,
        })
      end)
      changed = true
    end
  end
  return changed
end



local function handle_dequeue(state)
  local restored = restore_queued_messages_to_editor(state)
  if restored == 0 then
    show_status(state, "No queued messages to restore")
  else
    show_status(state, "Restored " .. restored .. " queued message"
      .. (restored > 1 and "s" or "") .. " to editor")
  end
end

-- interactive-mode.ts handleClipboardImagePaste: read the OS clipboard
-- image (pi.clipboard.read_image — the readClipboardImage mechanism),
-- write it to the spec's `pi-clipboard-<uuid>.<ext>` temp path, and insert
-- the path at the cursor; clipboard errors stay silent. Parity drivers stub
-- the read (state.read_clipboard_image) exactly as pi-shell-turn.ts stubs
-- readClipboardImage with the scenario's pre-written path.
local function handle_clipboard_image_paste(state)
  local ok, path = pcall(function()
    if state.read_clipboard_image then return state.read_clipboard_image() end
    local image = pi.clipboard.read_image()
    if not image then return nil end
    local ext = pi.clipboard.extension_for_mime_type(image.mimeType) or "png"
    local file_path = pi.path.join(pi.fs.tmpdir(), "pi-clipboard-" .. pi.random_uuid() .. "." .. ext)
    pi.fs.write_file(file_path, image.bytes)
    return file_path
  end)
  if not ok or not path or path == "" then return end
  state.editor.editor:insert_text_at_cursor(path)
end

-- interactive-mode.ts queueCompactionMessage.
local function queue_compaction_message(state, text, mode)
  state.compaction_queued[#state.compaction_queued + 1] = { text = text, mode = mode }
  state.editor.editor:add_to_history(text)
  state.editor.editor:set_text("")
  clear_pending_bash_display(state) -- updatePendingMessagesDisplay
  show_status(state, "Queued message for after compaction")
end

-- interactive-mode.ts flushCompactionQueue. Extension commands are intercepted
-- before queueing by handle_submit; queued prompt/steer/follow-up settlement
-- follows the same session path. Failures surface through the transcript.

local function flush_compaction_queue(state, options)
  if #state.compaction_queued == 0 then return end
  local queued = state.compaction_queued
  state.compaction_queued = {}

  if options and options.willRetry then
    -- When retry is pending, queue messages for the retry turn.
    for _, message in ipairs(queued) do
      if message.mode == "followUp" then
        local transformed = state.session.follow_up(message.text)
        if transformed ~= false then
          state.follow_up_texts[#state.follow_up_texts + 1] = transformed or message.text
        end
      else
        local transformed = state.session.steer(message.text)
        if transformed ~= false then
          state.steering_texts[#state.steering_texts + 1] = transformed or message.text
        end
      end
    end
    return
  end


  -- First message becomes the prompt (starts streaming); the rest queue.
  local first = queued[1]
  state.session.prompt(first.text)
  for i = 2, #queued do
    local message = queued[i]
    if message.mode == "followUp" then
      local transformed = state.session.follow_up(message.text)
      if transformed ~= false then
        state.follow_up_texts[#state.follow_up_texts + 1] = transformed or message.text
      end
    else
      local transformed = state.session.steer(message.text)
      if transformed ~= false then
        state.steering_texts[#state.steering_texts + 1] = transformed or message.text
      end
    end
  end
end

-- Session-tree navigation (double-escape "tree"/"fork" targets) lands with
-- the sessions milestone (item 6); until then the timing gate is armed but
-- the selectors are unreachable.
-- Forward declarations: the tree/fork selector bodies live with the
-- session-UI surface below (PLAN 6.4); handle_escape closes over them.
-- render_session_context is declared here so the compaction_end rebuild
-- (handle_agent_event, PLAN 6.5) can call the definition below.
local show_tree_selector, show_user_message_selector
local render_session_context

-- interactive-mode.ts setupKeyHandlers onEscape.
local function handle_escape(state)
  if state.escape_override then
    -- The summarize-branch flow swaps defaultEditor.onEscape for an
    -- abortBranchSummary trigger and restores it in its finally block.
    state.escape_override()
    return
  end
  if state.session.is_streaming() then
    restore_queued_messages_to_editor(state, { abort = true })
  elseif state.session.is_bash_running and state.session.is_bash_running() then
    state.session.abort_bash()
  elseif sync_bash_mode(state) then
    -- setText("") + isBashMode=false + updateEditorBorderColor; the
    -- border resyncs from the cleared text at the next frame.
    state.editor.editor:set_text("")
  elseif trim(state.editor.editor:get_text()) == "" then
    -- Double-escape with an empty editor (settingsManager
    -- getDoubleEscapeAction, default "tree"). The gate compares
    -- Date.now() values (lastEscapeTime starts at 0), so wall-clock ms.
    local action = state.double_escape_action or "tree"
    if action ~= "none" then
      local now = state.wall_now_ms and state.wall_now_ms() or pi.now_ms()
      if now - (state.last_escape_time or 0) < 500 then
        if action == "tree" then show_tree_selector(state)
        else show_user_message_selector(state) end
        state.last_escape_time = 0
      else
        state.last_escape_time = now
      end
    end
  end
end

-- interactive-mode.ts handleFollowUp. Extension commands submitted while idle
-- route through state.submit; streaming follow-ups remain queued messages.
local function handle_follow_up(state)
  local editor = state.editor.editor
  local text = trim(editor:get_expanded_text())
  if text == "" then return end
  -- Queue input during compaction (extension commands join item 9;
  -- scripted parity sessions may omit the compaction surface).
  if state.session.is_compacting and state.session.is_compacting() then
    queue_compaction_message(state, text, "followUp")
    return
  end
  if state.session.is_streaming() then
    editor:add_to_history(text)
    editor:set_text("")
    local transformed = state.session.follow_up(text)
    if transformed ~= false then
      state.follow_up_texts[#state.follow_up_texts + 1] = transformed or text
    end
    clear_pending_bash_display(state) -- updatePendingMessagesDisplay
  else
    -- Not streaming: alt+enter acts like regular Enter.
    editor:set_text("")
    state.submit(text)
  end
end

-- runner.ts getShortcuts → CustomEditor.onExtensionShortcut. Shortcut handlers
-- receive the same complete base context as event and tool handlers and run
-- asynchronously so terminal input is never held by extension work.
local function extension_shortcut_router(state, shortcuts)
  return function(data)
    for _, shortcut in ipairs(shortcuts) do
      if binding_matches(data, shortcut.shortcut) then
        pi.spawn(function()
          local ok, err = pcall(shortcut.handler,
            EXTENSION_CONTEXT_POLICY.snapshot(state))
          if not ok then
            show_error(state, "Shortcut handler error: " .. tostring(err))
          end
        end)
        return true
      end
    end
    return false
  end
end

-- interactive-mode.ts updatePendingMessagesDisplay (over
-- getAllQueuedMessages, so compaction-queued texts show too).
local function pending_messages_lines(state, width)
  local lines = {}
  -- Deferred bash components mount ahead of the queue rows; any queue
  -- change clears them from this container (updatePendingMessagesDisplay
  -- clear semantics — clear_pending_bash_display at the change sites).
  for _, row in ipairs(state.pending_bash_rows or {}) do
    append(lines, bash_execution_lines(row, width, state.theme))
  end
  local steering, follow_up = all_queued_messages(state)
  if #steering == 0 and #follow_up == 0 then return lines end
  local theme = state.theme
  lines[#lines + 1] = "" -- Spacer(1)
  for _, message in ipairs(steering) do
    append(lines, truncated_line(theme:fg("dim", "Steering: " .. message), width))
  end
  for _, message in ipairs(follow_up) do
    append(lines, truncated_line(theme:fg("dim", "Follow-up: " .. message), width))
  end
  -- getAppKeyDisplay: keyDisplayText capitalizes each key part.
  local hint = "↳ " .. format_key(DEFAULT_KEYS["app.message.dequeue"], true)
    .. " to edit all queued messages"
  append(lines, truncated_line(theme:fg("dim", hint), width))
  return lines
end

-- The statusContainer's Loader.render(): a leading blank line plus the
-- padded spinner/message text. Working/compaction use accent; retry uses warning.
local function status_container_lines(state, width)
  local loader = state.loader
  if not loader then return {} end
  local theme = state.theme
  local frame = loader.loader:frame()
  local text = theme:fg("muted", loader.message)
  if frame ~= "" then text = theme:fg(loader.color or "accent", frame) .. " " .. text end
  local lines = { "" }
  append(lines, pi.tui.text_render(text, width, 1, 0))
  return lines
end

function clear_retry_ui(state)
  if state.retry_escape_restore ~= nil then
    state.escape_override = state.retry_escape_restore or nil
    state.retry_escape_restore = nil
  end
  state.retry_countdown = nil
  if state.loader and state.loader.kind == "retry" then state.loader = nil end
end

local function start_working_loader(state)
  clear_retry_ui(state)
  local message = state.working_message or "Working..."
  state.loader = { loader = pi.tui.loader(message), message = message,
    last_ms = pi.monotonic_ms() }
end

-- interactive-mode.ts constructor + setupKeyHandlers: the CustomEditor with
-- the app-level handler wiring. setupExtensionShortcuts wires only when
-- extensions registered any.
function toggle_thinking_block_visibility(state)
  state.hide_thinking_block = not state.hide_thinking_block
  pi.settings.set_hide_thinking_block(state.hide_thinking_block)
  -- transcript_lines reads the setting for restored and streaming assistant
  -- rows, equivalent to rebuildChatFromMessages + streamingComponent refresh.
  show_status(state, "Thinking blocks: " .. (state.hide_thinking_block and "hidden" or "visible"))
end

function handle_suspend(state)
  if (state.platform or pi.platform()) == "win32" then
    show_status(state, "Suspend to background is not supported on Windows")
    return
  end
  state.pending_suspend = true
end

function take_suspend(state)
  local suspend = state.pending_suspend == true
  state.pending_suspend = false
  return suspend
end

local function setup_shell_editor(state)
  local shortcuts = pi.registered_shortcuts()
  state.editor = custom_editor({
    theme = state.theme,
    on_escape = function() handle_escape(state) end,
    on_ctrl_d = function() shutdown(state) end,
    on_paste_image = function() handle_clipboard_image_paste(state) end,
    on_extension_shortcut = #shortcuts > 0
      and extension_shortcut_router(state, shortcuts) or nil,
  })
  state.editor.editor:set_padding_x(pi.settings.editor_padding_x())
  state.editor.editor:set_autocomplete_max_visible(pi.settings.autocomplete_max_visible())
  state.editor.editor:set_focused(true)
  state.editor:on_action("app.clear", function() handle_ctrl_c(state) end)
  state.editor:on_action("app.suspend", function() handle_suspend(state) end)
  state.editor:on_action("app.thinking.cycle", function() cycle_thinking_level(state) end)
  state.editor:on_action("app.tools.expand", function()
    set_tools_expanded(state, not state.tools_expanded)
  end)
  state.editor:on_action("app.thinking.toggle", function() toggle_thinking_block_visibility(state) end)
  state.editor:on_action("app.editor.external", function()
    open_external_editor(state, state.editor.editor, "pi-editor-", ".pi.md", true, true)
  end)
  state.editor:on_action("app.message.followUp", function() handle_follow_up(state) end)
  state.editor:on_action("app.message.dequeue", function() handle_dequeue(state) end)
end

-- interactive-mode.ts setupEditorSubmitHandler's routed actions.
local function shell_submit_actions(state)
  return {
    set_text = function(value) state.editor.editor:set_text(value) end,
    quit = function() shutdown(state) end,
    show_oauth_selector = function(mode) show_oauth_selector(state, mode) end,
    settings_command = function() DEFAULT_KEYS.__settings_policy.show(state) end,
    scoped_models_command = function() DEFAULT_KEYS.__scoped_models_policy.show(state) end,
    changelog_command = function() handle_changelog_command(state) end,
    hotkeys_command = function() handle_hotkeys_command(state) end,
    debug_command = function() handle_debug_command(state) end,
    reload_command = function() handle_reload_command(state) end,
    armin_command = function() handle_armin_says_hi(state) end,
    earendil_command = function() handle_demented_delves(state) end,

    model_command = function(search) handle_model_command(state, search) end,
    export_command = function(text) handle_export_command(state, text) end,
    import_command = function(text) handle_import_command(state, text) end,

    share_command = function() handle_share_command(state) end,
    copy_command = function() handle_copy_command(state) end,
    name_command = function(text) handle_name_command(state, text) end,
    session_command = function() handle_session_command(state) end,
    clear_command = function() handle_clear_command(state) end,
    compact_command = function(custom) handle_compact_command(state, custom) end,
    resume_command = function() show_session_selector(state) end,
    fork_command = function() show_user_message_selector(state) end,
    clone_command = function() handle_clone_command(state) end,
    tree_command = function() show_tree_selector(state) end,
    trust_command = function() DEFAULT_KEYS.__trust_policy.show(state) end,
    is_bash_running = function()
      return state.session.is_bash_running ~= nil and state.session.is_bash_running() or false
    end,
    show_warning = function(message) show_warning(state, message) end,
    add_to_history = function(text) state.editor.editor:add_to_history(text) end,
    bash_command = function(command, excluded) handle_bash_command(state, command, excluded) end,
    extension_command = function(text)
      local context = EXTENSION_CONTEXT_POLICY.snapshot(state, { command = true })
      return EXTENSION_POLICY.execute_command(text, context, {
        background = true,
        on_error = function(err)
          EXTENSION_UI_POLICY.enqueue(state,
            { kind = "notify", message = err, level = "error" })
        end,
      })
    end,
    prompt = function(prompt)
      -- Queue input during compaction (extension commands join item 9).
      if state.session.is_compacting and state.session.is_compacting() then
        queue_compaction_message(state, prompt, "steer")
        return
      end
      -- interactive-mode.ts adds prompt-path submissions (steer and
      -- normal) to editor history; command routes are not added.
      state.editor.editor:add_to_history(prompt)
      if state.session.is_streaming() then
        -- session.prompt(text, { streamingBehavior: "steer" }) +
        -- updatePendingMessagesDisplay (clears deferred bash rows).
        local transformed = state.session.steer(prompt)
        if transformed ~= false then
          state.steering_texts[#state.steering_texts + 1] = transformed or prompt
        end
        clear_pending_bash_display(state)
      else
        -- Normal message submission: move pending bash components to
        -- chat first (flushPendingBashComponents).
        flush_pending_bash_components(state)
        state.session.prompt(prompt)
      end
    end,
  }
end

local function frontend_frame(state, width)
  width = math.max(20, width or 80)
  local lines = {}
  -- headerContainer: Spacer(1) + ExpandableText(logo/hints, 1, 0) + Spacer(1).
  lines[#lines + 1] = ""
  append(lines, pi.tui.text_render(header({ app_name = state.app_name,
    version = state.version, expanded = state.header_expanded }, state.theme), width, 1, 0))
  lines[#lines + 1] = ""
  -- chatContainer.
  append(lines, transcript_lines(state, width))
  -- pendingMessagesContainer.
  append(lines, pending_messages_lines(state, width))
  -- statusContainer (working loader).
  append(lines, status_container_lines(state, width))
  -- widgetContainerAbove: renderWidgets' spacer-when-empty default.
  lines[#lines + 1] = ""
  -- editorContainer: the editor, or the selector/dialog/reload box swapped into it.
  if state.reload_box then
    append(lines, state.reload_box:render(width))
  elseif state.selector then
    append(lines, state.selector:render(width))
  else
    -- updateEditorBorderColor (bash mode / thinking level), observable
    -- only at render time.
    sync_editor_border(state)
    for _, line in ipairs(state.editor.editor:render(width)) do lines[#lines + 1] = line end
  end
  -- widgetContainerBelow renders nothing by default.
  -- FooterComponent.render(): with a live agent, usage totals, the latest
  -- cache hit rate, and the context estimate come from the real message
  -- list; scripted states keep their stubbed values.
  local usage, cache_hit_rate, context_percent =
    state.usage, nil, state.context_percent
  if state.agent then
    usage, cache_hit_rate, context_percent = footer_agent_data(state)
    if cache_hit_rate == nil then cache_hit_rate = false end
  end
  local footer_lines = footer({
    width = width, cwd = state.cwd, home = state.home, branch = state.branch,
    -- footer.ts: "• sessionName" when the session has a display name.
    session_name = state.session_manager and state.session_manager:get_session_name() or "",
    usage = usage, cache_hit_rate = cache_hit_rate, context_percent = context_percent,
    context_window = state.model.contextWindow or 0,
    auto_compact = true, model_id = state.model.id, provider = state.model.provider,
    provider_count = state.provider_count or 1, reasoning = state.model.reasoning,
    thinking_level = state.thinking_level,
    subscription = state.registry and state.registry.is_using_oauth
      and state.registry.is_using_oauth(state.model) or false,
  }, state.theme)
  for _, line in ipairs(footer_lines) do lines[#lines + 1] = line end
  return lines
end

-- interactive-mode.ts handleEvent — the agent-event policy over transcript
-- rows. Assistant messages stream in place (the chatContainer's
-- streamingComponent — a transcript row updated by reference); toolCall
-- blocks in the partial mount pending tool rows during streaming;
-- message_end settles errors/aborts into the pending tools.
local function handle_agent_event(state, event)
  if event.type == "agent_start" then
    state.pending_tools = {}
    start_working_loader(state)
  elseif event.type == "agent_end" then
    state.loader = nil
    if state.streaming_row then
      -- A live streaming component at agent_end is removed (the turn
      -- settled without its message_end).
      for index, item in ipairs(state.transcript) do
        if item == state.streaming_row then table.remove(state.transcript, index) break end
      end
      state.streaming_row = nil
    end
    state.pending_tools = {}
    -- checkShutdownRequested (extension ctx.shutdown()).
    if state.shutdown_requested then state.exit = true end
  elseif event.type == "message_start" and event.message then
    local message = event.message
    if message.role == "user" then
      -- agent-session.ts: a starting user message is consumed from
      -- whichever queue carried it before the UI sees the event.
      local text = message_text(message)
      local removed = false
      for index, queued in ipairs(state.steering_texts) do
        if queued == text then table.remove(state.steering_texts, index); removed = true; break end
      end
      if not removed then
        for index, queued in ipairs(state.follow_up_texts) do
          if queued == text then
            table.remove(state.follow_up_texts, index)
            removed = true
            break
          end
        end
      end
      -- queue_update → updatePendingMessagesDisplay (clears deferred
      -- bash components from the pending display).
      if removed then clear_pending_bash_display(state) end
      state.transcript[#state.transcript + 1] = { kind = "user", text = text }
    elseif message.role == "assistant" then
      local row = { kind = "assistant", message = message, streaming = true }
      state.streaming_row = row
      state.transcript[#state.transcript + 1] = row
    end
  elseif event.type == "message_update" and event.message and event.message.role == "assistant" then
    if state.streaming_row then
      state.streaming_row.message = event.message
      -- Tool calls appearing in the partial mount pending components.
      for _, content in ipairs(event.message.content or {}) do
        if content.type == "toolCall" and content.id then
          local row = state.pending_tools[content.id]
          if row == nil then
            row = { kind = "tool", toolCallId = content.id, name = content.name,
              args = content.arguments, state = "pending", executionStarted = false,
              argsComplete = false, render_state = {} }
            state.pending_tools[content.id] = row
            state.transcript[#state.transcript + 1] = row
          else
            row.args = content.arguments
          end
        end
      end
    end
  elseif event.type == "message_end" and event.message then
    local message = event.message
    if message.role == "assistant" and state.streaming_row then
      local error_message
      if message.stopReason == "aborted" then
        local retry_attempt = state.session.retry_attempt and state.session.retry_attempt() or 0
        error_message = retry_attempt > 0
          and ("Aborted after " .. retry_attempt .. " retry attempt"
            .. (retry_attempt > 1 and "s" or ""))
          or "Operation aborted"
        message.errorMessage = error_message
      end
      state.streaming_row.message = message
      state.streaming_row.streaming = nil
      if message.stopReason == "aborted" or message.stopReason == "error" then
        if not error_message then error_message = message.errorMessage or "Error" end
        for _, row in pairs(state.pending_tools) do
          row.result = { content = { { type = "text", text = error_message } } }
          row.state = "error"
        end
        state.pending_tools = {}
      else
        -- Args are now complete — edit-tool diffs can compute.
        for _, row in pairs(state.pending_tools) do row.argsComplete = true end
      end
      state.streaming_row = nil
    end
    -- agent-session.ts _handleAgentEvent: session persistence.
    persist_agent_event(state.session_manager, event)

  elseif event.type == "tool_execution_start" then
    local row = state.pending_tools[event.toolCallId]
    if row == nil then
      row = { kind = "tool", toolCallId = event.toolCallId, name = event.toolName,
        args = event.args, state = "pending", executionStarted = false,
        argsComplete = false, render_state = {} }
      state.pending_tools[event.toolCallId] = row
      state.transcript[#state.transcript + 1] = row
    end
    row.executionStarted = true
  elseif event.type == "tool_execution_update" then
    local row = state.pending_tools[event.toolCallId]
    if row and event.partialResult then
      -- updateResult(partial): content/details refresh, state stays pending.
      row.result = { content = event.partialResult.content,
        details = event.partialResult.details }
    end
  elseif event.type == "tool_execution_end" then
    local row = state.pending_tools[event.toolCallId]
    if row then
      row.result = event.result
      row.state = event.isError and "error" or "success"
      state.pending_tools[event.toolCallId] = nil
    end
  elseif event.type == "compaction_start" then
    -- interactive-mode.ts compaction_start: keep the editor active
    -- (submissions queue during compaction); escape aborts compaction.
    state.compaction_escape_restore = { previous = state.escape_override }
    state.escape_override = function() state.session.abort_compaction() end
    local cancel_hint = "(" .. format_key(DEFAULT_KEYS["app.interrupt"], false) .. " to cancel)"
    local label
    if event.reason == "manual" then
      label = "Compacting context... " .. cancel_hint
    else
      label = (event.reason == "overflow" and "Context overflow detected, " or "")
        .. "Auto-compacting... " .. cancel_hint
    end
    state.loader = { loader = pi.tui.loader(label), message = label,
      last_ms = pi.monotonic_ms() }
  elseif event.type == "compaction_end" then
    if state.compaction_escape_restore then
      state.escape_override = state.compaction_escape_restore.previous
      state.compaction_escape_restore = nil
    end
    state.loader = nil
    if event.aborted then
      if event.reason == "manual" then
        show_error(state, "Compaction cancelled")
      else
        show_status(state, "Auto-compaction cancelled")
      end
    elseif event.result then
      -- chatContainer.clear() + rebuildChatFromMessages() + the
      -- compaction summary row (createCompactionSummaryMessage with the
      -- wall clock).
      state.transcript = {}
      state.last_status = nil
      state.pending_tools = {}
      state.session_context = nil
      render_session_context(state)
      state.transcript[#state.transcript + 1] = {
        kind = "compaction_summary",
        message = { role = "compactionSummary", summary = event.result.summary,
          tokensBefore = event.result.tokensBefore,
          timestamp = state.wall_now_ms() },
      }
    elseif event.errorMessage then
      if event.reason == "manual" then
        show_error(state, event.errorMessage)
      else
        state.transcript[#state.transcript + 1] =
          { kind = "error_text", text = event.errorMessage }
      end
    end
    flush_compaction_queue(state, { willRetry = event.willRetry })
  elseif event.type == "auto_retry_start" then
    -- Save the normal editor Escape path and temporarily make Escape abort
    -- the backoff sleep.
    state.retry_escape_restore = state.escape_override or false
    state.escape_override = function() state.session.abort_retry() end
    local function retry_message(seconds)
      local interrupt = format_key(DEFAULT_KEYS["app.interrupt"] or "", false)
      return "Retrying (" .. event.attempt .. "/" .. event.maxAttempts .. ") in "
        .. seconds .. "s... (" .. interrupt .. " to cancel)"
    end
    local seconds = math.ceil(event.delayMs / 1000)
    local message = retry_message(seconds)
    state.loader = { kind = "retry", loader = pi.tui.loader(message), message = message,
      color = "warning", last_ms = pi.monotonic_ms() }
    state.retry_countdown = { remaining = seconds,
      next_ms = pi.monotonic_ms() + 1000, message = retry_message }
  elseif event.type == "auto_retry_end" then
    clear_retry_ui(state)
    if not event.success then
      show_error(state, "Retry failed after " .. event.attempt .. " attempts: "
        .. (event.finalError or "Unknown error"))
    end
  end
  -- Agent events mutate rows outside the process callback. Always leave one
  -- coalesced frame pending: relying only on is_streaming() loses the final
  -- update when a fast message_end/agent_end settles between render ticks.
  state.async_render = true

end


-- interactive-mode.ts getUserMessageText.
local function get_user_message_text(message)
  if message.role ~= "user" then return "" end
  if type(message.content) == "string" then return message.content end
  return content_text(message, "text")
end

-- interactive-mode.ts renderSessionContext — the restored-transcript
-- slice reachable from pi-rs's persisted sessions: user/assistant/
-- toolResult rows with aborted/error settlement, branch-summary rows
-- (PLAN 6.4), compaction-summary rows (PLAN 6.5), and bashExecution
-- rows (PLAN 7.1). Custom messages and skill-invocation blocks join
-- their rungs (item 9).
function render_session_context(state, options)
  options = options or {}
  state.pending_tools = {}
  local rendered_pending = {}
  local context = state.session_context or state.session_manager:build_session_context()
  for _, message in ipairs(context.messages) do
    if message.role == "assistant" then
      state.transcript[#state.transcript + 1] = { kind = "assistant", message = message }
      for _, content in ipairs(message.content or {}) do
        if content.type == "toolCall" then
          local row = { kind = "tool", toolCallId = content.id, name = content.name,
            args = content.arguments, state = "pending", executionStarted = false,
            argsComplete = false, render_state = {} }
          state.transcript[#state.transcript + 1] = row
          if message.stopReason == "aborted" or message.stopReason == "error" then
            local error_message
            if message.stopReason == "aborted" then
              -- session.retryAttempt is 0 outside a live retry loop
              -- (the retry-attempt variant joins the retry surface).
              error_message = "Operation aborted"
            else
              error_message = message.errorMessage or "Error"
            end
            row.result = { content = { { type = "text", text = error_message } } }
            row.state = "error"
          else
            rendered_pending[content.id] = row
          end
        end
      end
    elseif message.role == "toolResult" then
      -- Match tool results to pending tool components.
      local row = rendered_pending[message.toolCallId]
      if row then
        row.result = { content = message.content, details = message.details }
        row.state = message.isError and "error" or "success"
        rendered_pending[message.toolCallId] = nil
      end
    elseif message.role == "user" then
      local text = get_user_message_text(message)
      if text ~= "" then
        state.transcript[#state.transcript + 1] = { kind = "user", text = text }
        if options.populate_history and state.editor then
          state.editor.editor:add_to_history(text)
        end
      end
    elseif message.role == "branchSummary" then
      state.transcript[#state.transcript + 1] = { kind = "branch_summary", message = message }
    elseif message.role == "compactionSummary" then
      state.transcript[#state.transcript + 1] = { kind = "compaction_summary", message = message }
    elseif message.role == "bashExecution" then
      -- addMessageToChat "bashExecution": rebuild the completed component.
      local row = new_bash_execution_row(message.command, message.excludeFromContext)
      if message.output ~= nil and message.output ~= "" then
        bash_row_append_output(row, message.output)
      end
      bash_row_set_complete(row, message.exitCode, message.cancelled or false,
        message.truncated and { truncated = true } or nil, message.fullOutputPath)
      state.transcript[#state.transcript + 1] = row
    end
  end
  for id, row in pairs(rendered_pending) do state.pending_tools[id] = row end
end

-- interactive-mode.ts renderInitialMessages, including the warning shown when
-- startup trust resolution leaves this project untrusted.
local function render_initial_messages(state)
  render_session_context(state, { populate_history = true })
  local compactions = 0
  for _, entry in ipairs(state.session_manager:get_entries()) do
    if entry.type == "compaction" then compactions = compactions + 1 end
  end
  if compactions > 0 then
    local times = compactions == 1 and "1 time" or (compactions .. " times")
    show_status(state, "Session compacted " .. times)
  end
  if state.project_trusted == false then
    if #state.transcript > 0 then state.transcript[#state.transcript + 1] = { kind = "spacer" } end
    state.transcript[#state.transcript + 1] = { kind = "warning_raw",
      text = "This project is not trusted. Project .pi resources and packages are ignored. Use /trust to save a trust decision, then restart pi." }
  end
end

-- ===========================================================================
-- Session UI — ports of components/session-selector.ts,
-- components/session-selector-search.ts, core/session-cwd.ts, and the
-- interactive-mode.ts /resume, /new, /name, and /session handlers over the
-- pi.session listing/persistence mechanism (PLAN 6.3).
-- ===========================================================================

-- keybinding-hints.ts keyText/keyHint over the app.session.* defaults
-- (formatKeys joins multi-key bindings with "/").
local function app_key_text(action)
  local binding = DEFAULT_KEYS[action]
  if type(binding) == "table" then binding = table.concat(binding, "/") end
  return format_key(binding or "", false)
end

local function app_key_hint(theme, action, description)
  return raw_key_hint(theme, DEFAULT_KEYS[action] or "", description)
end

-- session-selector.ts shortenPath — `~` for the home prefix.
local function shorten_session_path(path, home)
  if not path or path == "" then return path end
  if home and home ~= "" and path:sub(1, #home) == home then
    return "~" .. path:sub(#home + 1)
  end
  return path
end

-- session-selector.ts formatSessionDate.
local function format_session_date(modified_ms, now_ms)
  local diff_ms = now_ms - modified_ms
  local diff_mins = math.floor(diff_ms / 60000)
  local diff_hours = math.floor(diff_ms / 3600000)
  local diff_days = math.floor(diff_ms / 86400000)
  if diff_mins < 1 then return "now" end
  if diff_mins < 60 then return diff_mins .. "m" end
  if diff_hours < 24 then return diff_hours .. "h" end
  if diff_days < 7 then return diff_days .. "d" end
  if diff_days < 30 then return math.floor(diff_days / 7) .. "w" end
  if diff_days < 365 then return math.floor(diff_days / 30) .. "mo" end
  return math.floor(diff_days / 365) .. "y"
end

-- utils/paths.ts canonicalizePath — realpath with a fall-back to the raw
-- path when resolution fails.
local function canonicalize_session_path(path)
  if not path or path == "" then return path end
  local ok, resolved = pcall(pi.fs.realpath, path)
  if ok and resolved then return resolved end
  return path
end

-- session-selector-search.ts. Tokenization treats ASCII whitespace as the
-- spec's `/\s/` (exotic Unicode whitespace inside a search query is not a
-- reachable difference for session text).
local function normalize_whitespace_lower(text)
  return trim(text:lower():gsub("%s+", " "))
end

local function get_session_search_text(session)
  return session.id .. " " .. (session.name or "") .. " "
    .. session.allMessagesText .. " " .. session.cwd
end

local function has_session_name(session)
  return session.name ~= nil and trim(session.name) ~= ""
end

local function parse_search_query(query)
  local trimmed = trim(query)
  if trimmed == "" then return { mode = "tokens", tokens = {}, regex = nil } end

  -- Regex mode: re:<pattern> (JS `new RegExp(pattern, "i")` semantics via
  -- the pi.tui.js_regex_search mechanism).
  if trimmed:sub(1, 3) == "re:" then
    local pattern = trim(trimmed:sub(4))
    if pattern == "" then
      return { mode = "regex", tokens = {}, regex = nil, error = "Empty regex" }
    end
    local _, err = pi.tui.js_regex_search(pattern, "")
    if err then return { mode = "regex", tokens = {}, regex = nil, error = err } end
    return { mode = "regex", tokens = {}, regex = pattern }
  end

  -- Token mode with quote support.
  local tokens = {}
  local buf = ""
  local in_quote = false
  local had_unclosed_quote = false
  local function flush(kind)
    local value = trim(buf)
    buf = ""
    if value == "" then return end
    tokens[#tokens + 1] = { kind = kind, value = value }
  end
  for i = 1, #trimmed do
    local ch = trimmed:sub(i, i)
    if ch == '"' then
      if in_quote then
        flush("phrase")
        in_quote = false
      else
        flush("fuzzy")
        in_quote = true
      end
    elseif not in_quote and ch:match("%s") then
      flush("fuzzy")
    else
      buf = buf .. ch
    end
  end
  if in_quote then had_unclosed_quote = true end
  if had_unclosed_quote then
    local fallback = {}
    for token in trimmed:gmatch("%S+") do
      fallback[#fallback + 1] = { kind = "fuzzy", value = token }
    end
    return { mode = "tokens", tokens = fallback, regex = nil }
  end
  flush(in_quote and "phrase" or "fuzzy")
  return { mode = "tokens", tokens = tokens, regex = nil }
end

local function match_session(session, parsed)
  local text = get_session_search_text(session)
  if parsed.mode == "regex" then
    if not parsed.regex then return { matches = false, score = 0 } end
    local idx = pi.tui.js_regex_search(parsed.regex, text)
    if idx == nil then return { matches = false, score = 0 } end
    return { matches = true, score = idx * 0.1 }
  end
  if #parsed.tokens == 0 then return { matches = true, score = 0 } end
  local total_score = 0
  local normalized_text = nil
  for _, token in ipairs(parsed.tokens) do
    if token.kind == "phrase" then
      if normalized_text == nil then normalized_text = normalize_whitespace_lower(text) end
      local phrase = normalize_whitespace_lower(token.value)
      if phrase ~= "" then
        local idx = normalized_text:find(phrase, 1, true)
        if idx == nil then return { matches = false, score = 0 } end
        total_score = total_score + (idx - 1) * 0.1
      end
    else
      local m = pi.tui.fuzzy_match(token.value, text)
      if not m.matches then return { matches = false, score = 0 } end
      total_score = total_score + m.score
    end
  end
  return { matches = true, score = total_score }
end

local function filter_and_sort_sessions(sessions, query, sort_mode, name_filter)
  local name_filtered = {}
  for _, session in ipairs(sessions) do
    if name_filter ~= "named" or has_session_name(session) then
      name_filtered[#name_filtered + 1] = session
    end
  end
  if trim(query) == "" then return name_filtered end
  local parsed = parse_search_query(query)
  if parsed.error then return {} end

  if sort_mode == "recent" then
    local filtered = {}
    for _, session in ipairs(name_filtered) do
      if match_session(session, parsed).matches then filtered[#filtered + 1] = session end
    end
    return filtered
  end

  -- Relevance: score ascending, modified descending; JS sort is stable, so
  -- remaining ties keep incoming order (index tiebreak).
  local scored = {}
  for index, session in ipairs(name_filtered) do
    local result = match_session(session, parsed)
    if result.matches then
      scored[#scored + 1] = { session = session, score = result.score, index = index }
    end
  end
  table.sort(scored, function(a, b)
    if a.score ~= b.score then return a.score < b.score end
    if a.session.modified ~= b.session.modified then return a.session.modified > b.session.modified end
    return a.index < b.index
  end)
  local result = {}
  for _, entry in ipairs(scored) do result[#result + 1] = entry.session end
  return result
end

-- session-selector.ts buildSessionTree/flattenSessionTree.
local function build_session_tree(sessions)
  local by_path, order = {}, {}
  for index, session in ipairs(sessions) do
    local session_path = canonicalize_session_path(session.path) or session.path
    local node = { session = session, children = {}, index = index }
    by_path[session_path] = node
    order[#order + 1] = node
  end
  local roots = {}
  for _, node in ipairs(order) do
    local parent_path = canonicalize_session_path(node.session.parentSessionPath)
    local parent = parent_path and by_path[parent_path]
    if parent then
      parent.children[#parent.children + 1] = node
    else
      roots[#roots + 1] = node
    end
  end
  local function sort_nodes(nodes)
    table.sort(nodes, function(a, b)
      if a.session.modified ~= b.session.modified then
        return a.session.modified > b.session.modified
      end
      return a.index < b.index
    end)
    for _, node in ipairs(nodes) do sort_nodes(node.children) end
  end
  sort_nodes(roots)
  return roots
end

local function flatten_session_tree(roots)
  local result = {}
  local function walk(node, depth, ancestor_continues, is_last)
    result[#result + 1] = { session = node.session, depth = depth,
      is_last = is_last, ancestor_continues = ancestor_continues }
    for i, child in ipairs(node.children) do
      local child_is_last = i == #node.children
      local continues = depth > 0 and not is_last or false
      local next_continues = {}
      for _, value in ipairs(ancestor_continues) do next_continues[#next_continues + 1] = value end
      next_continues[#next_continues + 1] = continues
      walk(child, depth + 1, next_continues, child_is_last)
    end
  end
  for i, root in ipairs(roots) do walk(root, 0, {}, i == #roots) end
  return result
end

-- session-selector.ts deleteSessionFile — `trash` first, unlink fallback.
-- (pi.exec surfaces no spawn-error message, so the trash hint is the first
-- stderr line when present; Lua's os.remove message stands in for node's
-- unlink error string — neither reaches a stable frame.)
local function delete_session_file(session_path)
  local trash_args = session_path:sub(1, 1) == "-" and { "--", session_path } or { session_path }
  local ok_exec, trash = pcall(pi.exec, "trash", trash_args)
  if not ok_exec then trash = nil end
  if (trash and trash.code == 0) or not pi.fs.exists(session_path) then
    return { ok = true, method = "trash" }
  end
  local removed, unlink_error = os.remove(session_path)
  if removed then return { ok = true, method = "unlink" } end
  local hint = nil
  if trash and trash.stderr and trim(trash.stderr) ~= "" then
    hint = "trash: " .. (trim(trash.stderr):match("[^\n]*") or trim(trash.stderr)):sub(1, 200)
  end
  local message = tostring(unlink_error or "Unknown error")
  if hint then message = message .. " (" .. hint .. ")" end
  return { ok = false, method = "unlink", error = message }
end

-- components/session-selector.ts — the SessionSelectorComponent (header,
-- session list, rename mode, delete confirmation) as one lines-producing
-- state machine. Loaders are synchronous (pi.session.list/list_all), so
-- the spec's transient "Loading…" header state resolves within the same
-- dispatch and never reaches a stable frame.
local function session_selector(opts)
  local theme = opts.theme
  local now_ms = opts.now_ms or function() return os.time() * 1000 end
  local delete_file = opts.delete_file or delete_session_file
  local self = {
    scope = "current", sort_mode = "threaded", name_filter = "all",
    sessions = {}, filtered = {}, selected = 0,
    search = pi.tui.input(),
    show_cwd = false, show_path = false,
    confirming_delete_path = nil, status = nil,
    mode = "list", rename_input = pi.tui.input(), rename_target = nil,
    current_sessions = nil, all_sessions = nil,
    focused = false,
  }
  local can_rename = opts.rename_session ~= nil
  local show_rename_hint = opts.show_rename_hint
  if show_rename_hint == nil then show_rename_hint = can_rename end
  local current_session_canonical = opts.current_session_file
    and canonicalize_session_path(opts.current_session_file) or nil

  local function set_status(status, auto_hide_ms)
    if status and auto_hide_ms then
      status.deadline = now_ms() + auto_hide_ms
    end
    self.status = status
  end

  -- SessionList.filterSessions.
  local function filter_sessions(query)
    local trimmed = trim(query)
    local name_filtered = {}
    for _, session in ipairs(self.sessions) do
      if self.name_filter ~= "named" or has_session_name(session) then
        name_filtered[#name_filtered + 1] = session
      end
    end
    if self.sort_mode == "threaded" and trimmed == "" then
      self.filtered = flatten_session_tree(build_session_tree(name_filtered))
    else
      local filtered = filter_and_sort_sessions(name_filtered, query, self.sort_mode, "all")
      self.filtered = {}
      for _, session in ipairs(filtered) do
        self.filtered[#self.filtered + 1] = { session = session, depth = 0,
          is_last = true, ancestor_continues = {} }
      end
    end
    self.selected = math.min(self.selected, math.max(0, #self.filtered - 1))
  end

  local function set_sessions(sessions, show_cwd)
    self.sessions = sessions
    self.show_cwd = show_cwd
    filter_sessions(self.search:value())
  end

  local function is_current_session_path(path)
    if not current_session_canonical then return false end
    return (canonicalize_session_path(path) or path) == current_session_canonical
  end

  -- SessionSelectorComponent.loadScope, synchronous.
  local function load_scope(scope, reason)
    local show_cwd = scope == "all"
    local ok, sessions = pcall(scope == "current" and opts.load_current or opts.load_all)
    if ok then
      if scope == "current" then self.current_sessions = sessions
      else self.all_sessions = sessions end
      if scope ~= self.scope then return end
      set_sessions(sessions, show_cwd)
      if scope == "all" and #sessions == 0 and #(self.current_sessions or {}) == 0 then
        opts.on_cancel()
      end
    else
      if scope ~= self.scope then return end
      set_status({ type = "error",
        message = "Failed to load sessions: " .. tostring(sessions) }, 4000)
      if reason == "initial" then set_sessions({}, show_cwd) end
    end
  end

  local function toggle_scope()
    if self.scope == "current" then
      self.scope = "all"
      if self.all_sessions ~= nil then
        set_sessions(self.all_sessions, true)
        return
      end
      load_scope("all", "toggle")
      return
    end
    self.scope = "current"
    set_sessions(self.current_sessions or {}, false)
  end

  local function toggle_sort_mode()
    self.sort_mode = self.sort_mode == "threaded" and "recent"
      or self.sort_mode == "recent" and "relevance" or "threaded"
    filter_sessions(self.search:value())
  end

  local function toggle_name_filter()
    self.name_filter = self.name_filter == "all" and "named" or "all"
    filter_sessions(self.search:value())
  end

  local function refresh_after_mutation()
    load_scope(self.scope, "refresh")
  end

  local function selected_node()
    return self.filtered[self.selected + 1]
  end

  local function start_delete_confirmation()
    local node = selected_node()
    if not node then return end
    if is_current_session_path(node.session.path) then
      set_status({ type = "error", message = "Cannot delete the currently active session" }, 3000)
      return
    end
    self.confirming_delete_path = node.session.path
  end

  local function delete_session(session_path)
    local result = delete_file(session_path)
    if result.ok then
      local function without(sessions)
        if not sessions then return sessions end
        local kept = {}
        for _, session in ipairs(sessions) do
          if session.path ~= session_path then kept[#kept + 1] = session end
        end
        return kept
      end
      self.current_sessions = without(self.current_sessions)
      self.all_sessions = without(self.all_sessions)
      local sessions = self.scope == "all" and (self.all_sessions or {})
        or (self.current_sessions or {})
      set_sessions(sessions, self.scope == "all")
      set_status({ type = "info", message = result.method == "trash"
        and "Session moved to trash" or "Session deleted" }, 2000)
      refresh_after_mutation()
    else
      set_status({ type = "error",
        message = "Failed to delete: " .. (result.error or "Unknown error") }, 3000)
    end
  end

  local function exit_rename_mode()
    self.mode = "list"
    self.rename_target = nil
  end

  local function enter_rename_mode(session_path, current_name)
    self.mode = "rename"
    self.rename_target = session_path
    self.rename_input:set_value(current_name or "")
    self.rename_input:set_focused(self.focused)
  end

  local function confirm_rename(value)
    local next_name = trim(value)
    if next_name == "" then return end
    local target = self.rename_target
    if not target or not opts.rename_session then
      exit_rename_mode()
      return
    end
    local ok = pcall(opts.rename_session, target, next_name)
    if ok then refresh_after_mutation() end
    exit_rename_mode()
  end

  local function rename_selected()
    if not can_rename then return end
    local node = selected_node()
    if not node then return end
    local sessions = self.scope == "all" and (self.all_sessions or {})
      or (self.current_sessions or {})
    local current_name = nil
    for _, session in ipairs(sessions) do
      if session.path == node.session.path then current_name = session.name break end
    end
    enter_rename_mode(node.session.path, current_name)
  end

  function self:set_focused(focused)
    self.focused = focused
    self.search:set_focused(focused and self.mode == "list")
    self.rename_input:set_focused(focused and self.mode == "rename")
  end

  -- Status auto-hide (the spec's setTimeout): report when a deadline has
  -- passed so the frame loop re-renders, clearing lazily.
  function self:needs_render(now)
    if self.status and self.status.deadline and now >= self.status.deadline then
      self.status = nil
      return true
    end
    return false
  end

  function self:handle_input(data)
    if self.mode == "rename" then
      if binding_matches(data, SELECT_KEYS.cancel) then
        exit_rename_mode()
        return
      end
      local event = self.rename_input:input(data)
      if event.kind == "submit" then confirm_rename(event.value) end
      return
    end

    -- Delete confirmation intercepts all keys.
    if self.confirming_delete_path ~= nil then
      if binding_matches(data, SELECT_KEYS.confirm) then
        local path = self.confirming_delete_path
        self.confirming_delete_path = nil
        delete_session(path)
      elseif binding_matches(data, SELECT_KEYS.cancel) then
        self.confirming_delete_path = nil
      end
      return
    end

    if binding_matches(data, DEFAULT_KEYS["tui.input.tab"]) then
      toggle_scope()
    elseif binding_matches(data, DEFAULT_KEYS["app.session.toggleSort"]) then
      toggle_sort_mode()
    elseif binding_matches(data, DEFAULT_KEYS["app.session.toggleNamedFilter"]) then
      toggle_name_filter()
    elseif binding_matches(data, DEFAULT_KEYS["app.session.togglePath"]) then
      self.show_path = not self.show_path
    elseif binding_matches(data, DEFAULT_KEYS["app.session.delete"]) then
      start_delete_confirmation()
    elseif binding_matches(data, DEFAULT_KEYS["app.session.rename"]) then
      rename_selected()
    elseif binding_matches(data, DEFAULT_KEYS["app.session.deleteNoninvasive"]) then
      if #self.search:value() > 0 then
        self.search:input(data)
        filter_sessions(self.search:value())
        return
      end
      start_delete_confirmation()
    elseif binding_matches(data, SELECT_KEYS.up) then
      self.selected = math.max(0, self.selected - 1)
    elseif binding_matches(data, SELECT_KEYS.down) then
      self.selected = math.min(#self.filtered - 1, self.selected + 1)
    elseif binding_matches(data, SELECT_KEYS.pageUp) then
      self.selected = math.max(0, self.selected - 10)
    elseif binding_matches(data, SELECT_KEYS.pageDown) then
      self.selected = math.min(#self.filtered - 1, self.selected + 10)
    elseif binding_matches(data, SELECT_KEYS.confirm) then
      local node = selected_node()
      if node then opts.on_select(node.session.path) end
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      opts.on_cancel()
    else
      self.search:input(data)
      filter_sessions(self.search:value())
    end
  end

  -- SessionSelectorHeader.render.
  local function header_lines(width)
    local title = self.scope == "current" and "Resume Session (Current Folder)"
      or "Resume Session (All)"
    local left_text = theme:bold(title)
    local sort_label = self.sort_mode == "threaded" and "Threaded"
      or self.sort_mode == "recent" and "Recent" or "Fuzzy"
    local sort_text = theme:fg("muted", "Sort: ") .. theme:fg("accent", sort_label)
    local name_label = self.name_filter == "all" and "All" or "Named"
    local name_text = theme:fg("muted", "Name: ") .. theme:fg("accent", name_label)
    local scope_text
    if self.scope == "current" then
      scope_text = theme:fg("accent", "◉ Current Folder") .. theme:fg("muted", " | ○ All")
    else
      scope_text = theme:fg("muted", "○ Current Folder | ") .. theme:fg("accent", "◉ All")
    end
    local right_text = pi.tui.truncate(scope_text .. "  " .. name_text .. "  " .. sort_text, width, "", false)
    local available_left = math.max(0, width - pi.tui.visible_width(right_text) - 1)
    local left = pi.tui.truncate(left_text, available_left, "", false)
    local spacing = math.max(0, width - pi.tui.visible_width(left) - pi.tui.visible_width(right_text))

    local hint_line1, hint_line2
    if self.confirming_delete_path ~= nil then
      local confirm_hint = "Delete session? " .. select_key_hint(theme, "confirm", "confirm")
        .. " · " .. select_key_hint(theme, "cancel", "cancel")
      hint_line1 = theme:fg("error", pi.tui.truncate(confirm_hint, width, "…", false))
      hint_line2 = ""
    elseif self.status then
      local color = self.status.type == "error" and "error" or "accent"
      hint_line1 = theme:fg(color, pi.tui.truncate(self.status.message, width, "…", false))
      hint_line2 = ""
    else
      local path_state = self.show_path and "(on)" or "(off)"
      local sep = theme:fg("muted", " · ")
      local hint1 = app_key_hint(theme, "tui.input.tab", "scope") .. sep
        .. theme:fg("muted", 're:<pattern> regex · "phrase" exact')
      local hint2_parts = {
        app_key_hint(theme, "app.session.toggleSort", "sort"),
        app_key_hint(theme, "app.session.toggleNamedFilter", "named"),
        app_key_hint(theme, "app.session.delete", "delete"),
        app_key_hint(theme, "app.session.togglePath", "path " .. path_state),
      }
      if show_rename_hint then
        hint2_parts[#hint2_parts + 1] = app_key_hint(theme, "app.session.rename", "rename")
      end
      hint_line1 = pi.tui.truncate(hint1, width, "…", false)
      hint_line2 = pi.tui.truncate(table.concat(hint2_parts, sep), width, "…", false)
    end
    return { left .. string.rep(" ", spacing) .. right_text, hint_line1, hint_line2 }
  end

  local function build_tree_prefix(node)
    if node.depth == 0 then return "" end
    local parts = {}
    for _, continues in ipairs(node.ancestor_continues) do
      parts[#parts + 1] = continues and "│  " or "   "
    end
    parts[#parts + 1] = node.is_last and "└─ " or "├─ "
    return table.concat(parts)
  end

  -- SessionList.render.
  local function list_lines(width)
    local lines = {}
    append(lines, self.search:render(width))
    lines[#lines + 1] = ""

    if #self.filtered == 0 then
      local empty_message
      if self.name_filter == "named" then
        local toggle_key = app_key_text("app.session.toggleNamedFilter")
        if self.show_cwd then
          empty_message = "  No named sessions found. Press " .. toggle_key .. " to show all."
        else
          empty_message = "  No named sessions in current folder. Press " .. toggle_key
            .. " to show all, or Tab to view all."
        end
      elseif self.show_cwd then
        empty_message = "  No sessions found"
      else
        empty_message = "  No sessions in current folder. Press Tab to view all."
      end
      lines[#lines + 1] = theme:fg("muted", pi.tui.truncate(empty_message, width, "…", false))
      return lines
    end

    local count = #self.filtered
    local max_visible = 10
    local start_index = math.max(0,
      math.min(self.selected - math.floor(max_visible / 2), count - max_visible))
    local end_index = math.min(start_index + max_visible, count)
    local now = now_ms()

    for i = start_index, end_index - 1 do
      local node = self.filtered[i + 1]
      local session = node.session
      local is_selected = i == self.selected
      local is_confirming_delete = session.path == self.confirming_delete_path
      local is_current = is_current_session_path(session.path)

      local prefix = build_tree_prefix(node)
      local has_name = session.name ~= nil and session.name ~= ""
      local display_text = session.name or session.firstMessage
      local normalized = trim(display_text:gsub("[%z\1-\31\127]", " "))

      local age = format_session_date(session.modified, now)
      local right_part = session.messageCount .. " " .. age
      if self.show_cwd and session.cwd ~= "" then
        right_part = shorten_session_path(session.cwd, opts.home) .. " " .. right_part
      end
      if self.show_path then
        right_part = shorten_session_path(session.path, opts.home) .. " " .. right_part
      end

      local cursor = is_selected and theme:fg("accent", "› ") or "  "
      local prefix_width = pi.tui.visible_width(prefix)
      local right_width = pi.tui.visible_width(right_part) + 2
      local available_for_msg = width - 2 - prefix_width - right_width
      local truncated_msg = pi.tui.truncate(normalized, math.max(10, available_for_msg), "…", false)

      local message_color = nil
      if is_confirming_delete then message_color = "error"
      elseif is_current then message_color = "accent"
      elseif has_name then message_color = "warning" end
      local styled_msg = message_color and theme:fg(message_color, truncated_msg) or truncated_msg
      if is_selected then styled_msg = theme:bold(styled_msg) end

      local left_part = cursor .. theme:fg("dim", prefix) .. styled_msg
      local left_width = pi.tui.visible_width(left_part)
      local spacing = math.max(1, width - left_width - pi.tui.visible_width(right_part))
      local styled_right = theme:fg(is_confirming_delete and "error" or "dim", right_part)

      local line = left_part .. string.rep(" ", spacing) .. styled_right
      if is_selected then line = theme:bg("selectedBg", line) end
      lines[#lines + 1] = pi.tui.truncate(line, width, "...", false)
    end

    if start_index > 0 or end_index < count then
      local scroll_text = "  (" .. (self.selected + 1) .. "/" .. count .. ")"
      lines[#lines + 1] = theme:fg("muted", pi.tui.truncate(scroll_text, width, "", false))
    end
    return lines
  end

  local accent_border = function(width)
    return dynamic_border_line(theme, width, function(text) return theme:fg("accent", text) end)
  end

  function self:render(width)
    local lines = { "", accent_border(width), "" }
    if self.mode == "rename" then
      -- buildBaseLayout(panel, { showHeader = false }).
      append(lines, pi.tui.text_render(theme:bold("Rename Session"), width, 1, 0))
      lines[#lines + 1] = ""
      append(lines, self.rename_input:render(width))
      lines[#lines + 1] = ""
      append(lines, pi.tui.text_render(theme:fg("muted",
        select_key_text("confirm") .. " to save · " .. select_key_text("cancel") .. " to cancel"),
        width, 1, 0))
    else
      append(lines, header_lines(width))
      lines[#lines + 1] = ""
      append(lines, list_lines(width))
    end
    lines[#lines + 1] = ""
    lines[#lines + 1] = accent_border(width)
    return lines
  end

  -- The constructor's immediate current-scope load.
  load_scope("current", "initial")
  return self
end

-- sdk.ts createAgentSession + agent-session.ts _buildRuntime — the
-- session-bound runtime slice, re-run by /new and /resume (the spec's
-- AgentSessionRuntime.createRuntime + rebindCurrentSession): restore
-- model/thinking/messages from the manager, build the agent over the
-- active tool set with the cwd-bound system prompt, resubscribe, and
-- refresh the footer's provider count. session_before_switch /
-- session_shutdown / session_start extension events join the extension
-- surface (item 9).
-- ===========================================================================
-- Compaction — agent-session.ts compact() / _checkCompaction /
-- _runAutoCompaction over the utils/compaction.lua port (PLAN 6.5). The
-- LLM work is compaction_lib; this section decides when to run it, what
-- persists, and which events the UI sees.
-- ===========================================================================

-- agent.lua error_text: strip Lua's source:line prefix so surfaced
-- messages compare against pi's Error.message strings.
local function compaction_error_text(value)
  local text = tostring(value)
  text = text:match("^(.-)\nstack traceback:") or text
  text = text:gsub("^runtime error: ", "")
  return text:match("^.-:%d+: (.*)$") or text
end

-- core/auth-guidance.ts — the interactive slice (the CLI carries the
-- Rust port for headless paths).
local function provider_login_help(state)
  local docs = state.docs_path or ""
  return "Use /login to log into a provider via OAuth or API key. See:\n  "
    .. pi.path.join(docs, "providers.md") .. "\n  " .. pi.path.join(docs, "models.md")
end

local function format_no_model_selected_message(state)
  return "No model selected.\n\n" .. provider_login_help(state)
    .. "\n\nThen use /model to select a model."
end

local function format_no_api_key_found_message(state, provider)
  local display = provider == "unknown" and "the selected model" or provider
  return "No API key found for " .. display .. ".\n\n" .. provider_login_help(state)
end

-- session-manager.ts getLatestCompactionEntry.
local function get_latest_compaction_entry(entries)
  for i = #entries, 1, -1 do
    if entries[i].type == "compaction" then return entries[i] end
  end
  return nil
end

-- agent-session.ts provider-retry classification. Kept outside the runtime
-- slice so the Pi-derived retry oracle can exercise the shipped policy without
-- duplicating its regular expressions.
RETRY_POLICY_PATTERNS = {
  non_retryable = "GoUsageLimitError|FreeUsageLimitError|Monthly usage limit reached|available balance|insufficient_quota|out of budget|quota exceeded|billing",
  retryable = "overloaded|provider.?returned.?error|rate.?limit|too many requests|429|500|502|503|504|service.?unavailable|server.?error|internal.?error|network.?error|connection.?error|connection.?refused|connection.?lost|websocket.?closed|websocket.?error|other side closed|fetch failed|upstream.?connect|reset before headers|socket hang up|ended without|stream ended before message_stop|http2 request did not get a response|timed? out|timeout|terminated|retry delay",
}
function retry_policy_is_retryable(model, message)
  if not message or message.stopReason ~= "error"
     or not message.errorMessage or message.errorMessage == "" then return false end
  local context_window = model and model.contextWindow or 0
  if compaction_lib.is_context_overflow(message, context_window) then return false end
  if pi.tui.js_regex_search(RETRY_POLICY_PATTERNS.non_retryable, message.errorMessage) ~= nil then
    return false
  end
  return pi.tui.js_regex_search(RETRY_POLICY_PATTERNS.retryable, message.errorMessage) ~= nil
end
-- The compaction/retry slice bound to one runtime (recreated on every
-- bind_session_runtime, like the AgentSession fields it ports).
local function create_compaction_slice(state, agent, session_manager)
  local slice = {
    last_assistant = nil,
    overflow_recovery_attempted = false,
    manual_signal = nil,
    auto_signal = nil,
    retry_signal = nil,
    retry_attempt = 0,
  }

  local function emit(event)
    if state.agent ~= agent then return end
    handle_agent_event(state, event)
    if state.event_hook then state.event_hook(event) end
  end

  function slice.is_retryable_error(message)
    return retry_policy_is_retryable(state.model, message)
  end

  function slice.will_retry_after_agent_end(event)
    local settings = pi.settings.retry_settings()
    if not settings.enabled or slice.retry_attempt >= settings.maxRetries then return false end
    for i = #(event.messages or {}), 1, -1 do
      if event.messages[i].role == "assistant" then
        return slice.is_retryable_error(event.messages[i])
      end
    end
    return false
  end

  -- agent-session.ts _handleAgentEvent bookkeeping. This runs after the UI
  -- listener, matching AgentSession's listener-before-persistence ordering.
  function slice.note_event(event)
    if event.type == "message_start" and event.message
       and event.message.role == "user" then
      slice.overflow_recovery_attempted = false
    elseif event.type == "message_end" and event.message
       and event.message.role == "assistant" then
      slice.last_assistant = event.message
      if event.message.stopReason ~= "error" then
        slice.overflow_recovery_attempted = false
        if slice.retry_attempt > 0 then
          emit({ type = "auto_retry_end", success = true, attempt = slice.retry_attempt })
          slice.retry_attempt = 0
        end
      end
    end
  end

  -- agent-session.ts isCompacting (branch summarization counts).
  function slice.is_compacting()
    return slice.manual_signal ~= nil or slice.auto_signal ~= nil
      or state.branch_summary_signal ~= nil
  end

  -- agent-session.ts abortCompaction.
  function slice.abort_compaction()
    if slice.manual_signal then slice.manual_signal:abort() end
    if slice.auto_signal then slice.auto_signal:abort() end
  end

  local function run_compaction(preparation, signal, custom_instructions, api_key)
    return compaction_lib.compact(preparation, state.model, {
      apiKey = api_key,
      customInstructions = custom_instructions,
      signal = signal,
      thinkingLevel = state.thinking_level,
      now_ms = state.wall_now_ms,
    })
  end

  local function apply_compaction(result, from_extension)
    local id = session_manager:append_compaction(result.summary, result.firstKeptEntryId,
      result.tokensBefore, result.details, from_extension == true)
    agent:set_messages(session_manager:build_session_context().messages)
    local entry = session_manager:get_entry(id)
    if entry then
      EXTENSION_POLICY.emit_generic({ type = "session_compact",
        compactionEntry = entry, fromExtension = from_extension == true },
        EXTENSION_CONTEXT_POLICY.snapshot(state))
    end
  end

  local function extension_compaction(preparation, path_entries, instructions, signal)
    local result = EXTENSION_POLICY.emit_generic({
      type = "session_before_compact", preparation = preparation,
      branchEntries = path_entries, customInstructions = instructions, signal = signal,
    }, EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = signal }))
    if result and result.cancel then return nil, true, false end
    if result and result.compaction then return result.compaction, false, true end
    return nil, false, false
  end

  -- agent-session.ts compact() — manual compaction. Callers abort and
  -- join the in-flight turn first (the spec's `await this.abort()`).
  -- Auth resolves through the getApiKey seam (pi.auth.get_api_key —
  -- stored key or OAuth refresh); a missing key surfaces the spec's
  -- formatNoApiKeyFoundMessage.
  function slice.compact(custom_instructions)
    local signal = pi.abort_signal()
    slice.manual_signal = signal
    emit({ type = "compaction_start", reason = "manual" })
    local executed, result = pcall(function()
      if not state.model then error(format_no_model_selected_message(state), 0) end
      local api_key = pi.auth.get_api_key(state.model.provider)
      if not api_key then
        error(format_no_api_key_found_message(state, state.model.provider), 0)
      end
      local path_entries = session_manager:get_branch()
      local settings = pi.settings.compaction_settings()
      local preparation = compaction_lib.prepare_compaction(path_entries, settings)
      if not preparation then
        local last = path_entries[#path_entries]
        if last and last.type == "compaction" then error("Already compacted", 0) end
        error("Nothing to compact (session too small)", 0)
      end
      local extension_result, cancelled, from_extension = extension_compaction(
        preparation, path_entries, custom_instructions, signal)
      if cancelled then error("Compaction cancelled", 0) end
      local compact_result = extension_result
        or run_compaction(preparation, signal, custom_instructions, api_key)
      if signal:is_aborted() then error("Compaction cancelled", 0) end
      apply_compaction(compact_result, from_extension)
      return compact_result
    end)
    slice.manual_signal = nil
    if executed then
      emit({ type = "compaction_end", reason = "manual", result = result,
        aborted = false, willRetry = false })
      return result
    end
    local message = compaction_error_text(result)
    local aborted = message == "Compaction cancelled"
    emit({ type = "compaction_end", reason = "manual", aborted = aborted,
      willRetry = false,
      errorMessage = (not aborted) and ("Compaction failed: " .. message) or nil })
    error(message, 0)
  end

  -- agent-session.ts _runAutoCompaction.
  local function run_auto_compaction(reason, will_retry)
    local settings = pi.settings.compaction_settings()
    emit({ type = "compaction_start", reason = reason })
    local signal = pi.abort_signal()
    slice.auto_signal = signal
    local executed, outcome = pcall(function()
      if not state.model then return { silent = true } end
      local api_key = pi.auth.get_api_key(state.model.provider)
      if not api_key then return { silent = true } end
      local path_entries = session_manager:get_branch()
      local preparation = compaction_lib.prepare_compaction(path_entries, settings)
      if not preparation then return { silent = true } end
      local extension_result, cancelled, from_extension = extension_compaction(
        preparation, path_entries, nil, signal)
      if cancelled then return { aborted = true } end
      local result = extension_result or run_compaction(preparation, signal, nil, api_key)
      if signal:is_aborted() then return { aborted = true } end
      apply_compaction(result, from_extension)
      return { result = result }
    end)
    slice.auto_signal = nil
    if not executed then
      local message = compaction_error_text(outcome)
      emit({ type = "compaction_end", reason = reason, aborted = false,
        willRetry = false,
        errorMessage = reason == "overflow"
          and ("Context overflow recovery failed: " .. message)
          or ("Auto-compaction failed: " .. message) })
      return false
    end
    if outcome.silent then
      emit({ type = "compaction_end", reason = reason, aborted = false,
        willRetry = false })
      return false
    end
    if outcome.aborted then
      emit({ type = "compaction_end", reason = reason, aborted = true,
        willRetry = false })
      return false
    end
    emit({ type = "compaction_end", reason = reason, result = outcome.result,
      aborted = false, willRetry = will_retry })
    if will_retry then
      -- Remove a trailing error message from context for the retry (it
      -- stays persisted in the session for history).
      local messages = agent:get_state().messages
      local last = messages[#messages]
      if last and last.role == "assistant" and last.stopReason == "error" then
        local trimmed = {}
        for i = 1, #messages - 1 do trimmed[i] = messages[i] end
        agent:set_messages(trimmed)
      end
      return true
    end
    -- Queued follow-up/steering messages need one continuation.
    return agent:has_queued_messages()
  end

  -- agent-session.ts _checkCompaction.
  function slice.check_compaction(assistant_message, skip_aborted_check)
    local settings = pi.settings.compaction_settings()
    if not settings.enabled then return false end
    if skip_aborted_check ~= false and assistant_message.stopReason == "aborted" then
      return false
    end
    local context_window = state.model and state.model.contextWindow or 0
    local same_model = state.model ~= nil
      and assistant_message.provider == state.model.provider
      and assistant_message.model == state.model.id
    -- Skip checks when the message predates the latest compaction
    -- boundary (stale pre-compaction usage must not retrigger).
    local compaction_entry = get_latest_compaction_entry(session_manager:get_branch())
    if compaction_entry then
      local boundary_ms = pi.session.parse_iso_ms(compaction_entry.timestamp or "")
      if boundary_ms and (assistant_message.timestamp or 0) <= boundary_ms then
        return false
      end
    end
    -- Case 1: overflow — compact and auto-retry once.
    if same_model and compaction_lib.is_context_overflow(assistant_message, context_window) then
      if slice.overflow_recovery_attempted then
        emit({ type = "compaction_end", reason = "overflow", aborted = false,
          willRetry = false,
          errorMessage = "Context overflow recovery failed after one compact-and-retry attempt. Try reducing context or switching to a larger-context model." })
        return false
      end
      slice.overflow_recovery_attempted = true
      -- Remove the error message from agent state (it IS saved to the
      -- session) so the retry context excludes it.
      local messages = agent:get_state().messages
      if #messages > 0 and messages[#messages].role == "assistant" then
        local trimmed = {}
        for i = 1, #messages - 1 do trimmed[i] = messages[i] end
        agent:set_messages(trimmed)
      end
      return run_auto_compaction("overflow", true)
    end
    -- Case 2: threshold — error messages estimate from the last usage,
    -- gated to post-compaction usage sources.
    local context_tokens
    if assistant_message.stopReason == "error" then
      local messages = agent:get_state().messages
      local estimate = compaction_lib.estimate_context_tokens(messages)
      if estimate.lastUsageIndex == nil then return false end
      local usage_msg = messages[estimate.lastUsageIndex + 1]
      if compaction_entry and usage_msg and usage_msg.role == "assistant" then
        local boundary_ms = pi.session.parse_iso_ms(compaction_entry.timestamp or "")
        if boundary_ms and (usage_msg.timestamp or 0) <= boundary_ms then
          return false
        end
      end
      context_tokens = estimate.tokens
    else
      context_tokens = compaction_lib.calculate_context_tokens(assistant_message.usage)
    end
    if compaction_lib.should_compact(context_tokens, context_window, settings) then
      return run_auto_compaction("threshold", false)
    end
    return false
  end

  -- agent-session.ts _prepareRetry: remove the persisted error from live
  -- context, expose the retry event, and wait with abortable exponential backoff.
  function slice.prepare_retry(message)
    local settings = pi.settings.retry_settings()
    if not settings.enabled then return false end
    slice.retry_attempt = slice.retry_attempt + 1
    if slice.retry_attempt > settings.maxRetries then
      slice.retry_attempt = slice.retry_attempt - 1
      return false
    end
    local delay_ms = settings.baseDelayMs * (2 ^ (slice.retry_attempt - 1))
    -- Arm cancellation before notifying listeners. Pi assigns its controller
    -- immediately after emit; Lua listeners are synchronous, so this preserves
    -- the same user-visible ordering without an unabortable callback window.
    local signal = pi.abort_signal()
    slice.retry_signal = signal
    emit({ type = "auto_retry_start", attempt = slice.retry_attempt,
      maxAttempts = settings.maxRetries, delayMs = delay_ms,
      errorMessage = message.errorMessage or "Unknown error" })
    local messages = agent:get_state().messages
    if #messages > 0 and messages[#messages].role == "assistant" then
      local trimmed = {}
      for i = 1, #messages - 1 do trimmed[i] = messages[i] end
      agent:set_messages(trimmed)
    end
    local completed = pcall(pi.sleep, delay_ms, signal)
    slice.retry_signal = nil
    if not completed then
      local attempt = slice.retry_attempt
      slice.retry_attempt = 0
      emit({ type = "auto_retry_end", success = false, attempt = attempt,
        finalError = "Retry cancelled" })
      return false
    end
    return true
  end

  function slice.abort_retry()
    if slice.retry_signal then slice.retry_signal:abort() end
  end

  -- agent-session.ts _handlePostAgentRun: provider retry precedes compaction.
  function slice.handle_post_agent_run()
    local message = slice.last_assistant
    slice.last_assistant = nil
    if not message then return false end
    if slice.is_retryable_error(message) and slice.prepare_retry(message) then return true end
    if message.stopReason == "error" and slice.retry_attempt > 0 then
      emit({ type = "auto_retry_end", success = false, attempt = slice.retry_attempt,
        finalError = message.errorMessage })
      slice.retry_attempt = 0
    end
    if slice.check_compaction(message, true) then return true end
    -- Messages queued by agent_end handlers need a continuation.
    return agent:has_queued_messages()
  end

  -- agent-session.ts _runAgentPrompt — the finally block flushes bash
  -- messages queued while the turn streamed (PLAN 7.1).
  function slice.run_agent_prompt(text)
    local executed, err = pcall(function()
      agent:prompt(text)
      while slice.handle_post_agent_run() do agent:continue() end
    end)
    if state.bash_slice then state.bash_slice.flush_pending() end
    if not executed then error(err, 0) end
  end

  -- prompt()'s pre-send compaction check (catches aborted responses:
  -- skipAbortedCheck false). The continue loop flushes pending bash
  -- messages in its finally block, like _runAgentPrompt.
  function slice.pre_prompt_compaction()
    local messages = agent:get_state().messages
    local last = nil
    for i = #messages, 1, -1 do
      if messages[i].role == "assistant" then last = messages[i] break end
    end
    if last and slice.check_compaction(last, false) then
      local executed, err = pcall(function()
        agent:continue()
        while slice.handle_post_agent_run() do agent:continue() end
      end)
      if state.bash_slice then state.bash_slice.flush_pending() end
      if not executed then error(err, 0) end
    end
  end

  return slice
end

-- Scripted stream used only by the retry differential command below. It still
-- enters through pi.agent.new's public streamFn seam and therefore exercises
-- the shipped AgentSession retry loop rather than a test-only reimplementation.
function retry_parity_stream(request)
  local turns = request.retryParityTurns
  if not turns then return nil end
  local recorder = { requests = {} }
  request.__retry_recorder = recorder
  local index = 0
  local function copy(value) return pi.json.decode(pi.json.encode(value)) end
  local function message(model, turn)
    local failed = turn.stopReason == "error"
    return {
      role = "assistant",
      content = failed and {} or { { type = "text", text = turn.text or "ok" } },
      api = model.api, provider = model.provider, model = model.id,
      usage = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0,
        totalTokens = 0, cost = { input = 0, output = 0, cacheRead = 0,
          cacheWrite = 0, total = 0 } },
      stopReason = turn.stopReason or "stop", errorMessage = turn.errorMessage,
      timestamp = 0,
    }
  end
  return function(model, context, _options, push)
    index = index + 1
    local turn = turns[math.min(index, #turns)]
    recorder.requests[#recorder.requests + 1] = copy(context.messages or {})
    local final = message(model, turn)
    push({ type = "start", partial = message(model, { stopReason = "stop", text = "" }) })
    if final.stopReason == "error" then
      push({ type = "error", reason = "error", error = final })
    else
      push({ type = "done", reason = final.stopReason, message = final })
    end
    return final
  end
end

function bind_session_runtime(state, session_manager)
  local request = state.request
  state.extension_context_generation = (state.extension_context_generation or 0) + 1
  state.extension_errors = state.extension_errors or {}
  EXTENSION_POLICY.on_error = function(error)
    state.extension_errors[#state.extension_errors + 1] = error
    if state.theme and state.transcript then
      state.transcript[#state.transcript + 1] = { kind = "text", padding_y = 1,
        text = state.theme:fg("error", 'Extension "' .. error.extensionPath
          .. '" error: ' .. error.error) }
    end
    state.async_render = true
  end
  state.session_manager = session_manager
  state.cwd = session_manager:get_cwd()
  local startup = session_startup_from_request(session_manager, request)
  state.model = startup.model or request.model
  state.thinking_level = startup.thinking_level
  state.session_context = startup.context
  state.extension_turn_state = { index = 0 }
  state.extension_shutdown_emitted = false

  -- Tool activation is public declaration data shared by embedded and
  -- file-backed packages; source identity does not participate.
  local active_tools, active_tool_names = EXTENSION_POLICY.active_tools()
  local stream_fn = retry_parity_stream(request)
  state.system_prompt_options = {
    cwd = state.cwd, agentDir = request.agentDir,
    toolNames = active_tool_names,
    readmePath = request.readmePath, docsPath = request.docsPath,
    examplesPath = request.examplesPath,
  }
  local system_prompt = build_session_system_prompt(state.system_prompt_options)
  local agent
  agent = pi.agent.new({
    initialState = {
      model = state.model, tools = active_tools,
      -- sdk.ts: `agent.state.messages = existingSession.messages`.
      messages = startup.context.messages,
      thinkingLevel = state.thinking_level,
      systemPrompt = system_prompt,
    },
    steeringMode = pi.settings.steering_mode(),
    followUpMode = pi.settings.follow_up_mode(),
    transport = pi.settings.transport(),
    -- sdk.ts convertToLlmWithBlockImages over messages.ts convertToLlm.
    convertToLlm = convert_to_llm_with_block_images,
    streamFn = stream_fn,
    -- sdk.ts also passes settings thinkingBudgets / maxRetryDelayMs;
    -- those settings join the extension/configuration surface (item 9).
    -- configuration surface (item 9).
    apiKey = request.apiKey,
    -- Spec: each request resolves auth for the current model's provider
    -- (modelRegistry.getApiKeyForProvider) so /model can cross providers.
    getApiKey = function(provider) return pi.auth.get_api_key(provider) end,
    transformContext = function(messages, signal)
      return EXTENSION_POLICY.emit_context(messages,
        EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = signal }))
    end,
    onPayload = function(payload)
      return EXTENSION_POLICY.emit_before_provider_request(payload,
        EXTENSION_CONTEXT_POLICY.snapshot(state,
          { signal = agent and agent:get_state().signal or nil }))
    end,
    onResponse = function(response)
      EXTENSION_POLICY.emit_generic({ type = "after_provider_response",
        status = response.status, headers = response.headers },
        EXTENSION_CONTEXT_POLICY.snapshot(state,
          { signal = agent and agent:get_state().signal or nil }))
    end,
    createToolContext = function(signal)
      return EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = signal })
    end,
    beforeToolCall = function(event, signal)
      return EXTENSION_POLICY.emit_tool_call(event,
        EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = signal }))
    end,
    afterToolCall = function(event, signal)
      return EXTENSION_POLICY.emit_tool_result({
        type = "tool_result", toolCallId = event.toolCall.id,
        toolName = event.toolCall.name, input = event.args,
        content = event.result.content, details = event.result.details,
        isError = event.isError,
      }, EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = signal }))
    end,
  })
  state.agent = agent
  -- rebindCurrentSession clears any messages queued during compaction.
  state.compaction_queued = {}
  -- The bash half of agent-session.ts (PLAN 7.1): executeBash /
  -- recordBashResult / abortBash / _pendingBashMessages over the
  -- utils/bash-executor.lua fragment. A runtime replacement drops the
  -- old slice with its pending display rows.
  local bash_slice = { signal = nil, pending_messages = {} }
  state.bash_slice = bash_slice
  state.pending_bash_rows = {}
  state.pending_bash_components = {}
  state.bash_row = nil

  -- _flushPendingBashMessages: append to agent state and the session
  -- after the turn completes, preserving tool_use/tool_result ordering.
  function bash_slice.flush_pending()
    if #bash_slice.pending_messages == 0 then return end
    for _, message in ipairs(bash_slice.pending_messages) do
      agent:append_message(message)
      session_manager:append_message(message)
    end
    bash_slice.pending_messages = {}
  end

  -- recordBashResult.
  function bash_slice.record(command, result, options)
    local message = {
      role = "bashExecution", command = command,
      output = result.output, exitCode = result.exitCode,
      cancelled = result.cancelled or false,
      truncated = result.truncated or false,
      fullOutputPath = result.fullOutputPath,
      timestamp = state.wall_now_ms(),
      excludeFromContext = options and options.excludeFromContext or nil,
    }
    -- If the agent is streaming, defer to keep message ordering.
    if agent:get_state().isStreaming then
      bash_slice.pending_messages[#bash_slice.pending_messages + 1] = message
    else
      agent:append_message(message)
      session_manager:append_message(message)
    end
  end

  local compaction = create_compaction_slice(state, agent, session_manager)
  -- agent-session.ts's session surface, as consumed by the shell handlers.
  state.session = {
    is_streaming = function() return agent:get_state().isStreaming end,
    is_compacting = compaction.is_compacting,
    retry_attempt = function() return compaction.retry_attempt end,
    abort_retry = compaction.abort_retry,
    abort = function() agent:abort() end,
    abort_compaction = compaction.abort_compaction,
    compact = compaction.compact,
    clear_queues = function() agent:clear_all_queues() end,
    steer = function(text)
      local result = EXTENSION_POLICY.emit_input(text, nil, "interactive", "steer",
        EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = agent:get_state().signal }))
      if result.action == "handled" then return false end
      local transformed = result.action == "transform" and result.text or text
      agent:steer(user_message(transformed))
      return transformed
    end,
    follow_up = function(text)
      local result = EXTENSION_POLICY.emit_input(text, nil, "interactive", "followUp",
        EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = agent:get_state().signal }))
      if result.action == "handled" then return false end
      local transformed = result.action == "transform" and result.text or text
      agent:follow_up(user_message(transformed))
      return transformed
    end,
    -- executeBash's synchronous prefix (the spec assigns the abort
    -- controller on call, before any await): begin_bash arms the signal
    -- from the submit handler; the executor body then runs on the bash
    -- coroutine.
    begin_bash = function()
      bash_slice.signal = pi.abort_signal()
      return bash_slice.signal
    end,
    is_bash_running = function() return bash_slice.signal ~= nil end,
    abort_bash = function()
      if bash_slice.signal then bash_slice.signal:abort() end
    end,
    -- AgentSession.executeBash from the settings reads onward (the
    -- controller was armed by begin_bash).
    execute_bash = function(command, signal, on_chunk, options)
      local prefix = pi.settings.shell_command_prefix()
      local shell_path = pi.settings.shell_path()
      local resolved = prefix and (prefix .. "\n" .. command) or command
      local executed, result = pcall(bash_executor_lib.execute_bash_with_operations,
        resolved, session_manager:get_cwd(),
        (options and options.operations)
          or bash_executor_lib.create_local_bash_operations({ shellPath = shell_path }),
        { onChunk = on_chunk, signal = signal })
      bash_slice.signal = nil
      if not executed then error(result, 0) end
      bash_slice.record(command, result, options)
      return result
    end,
    -- AgentSession.prompt: input middleware precedes pre-send compaction; the
    -- before_agent_start fold then injects custom messages and a turn-local
    -- system prompt before agent-core starts.
    prompt = function(text)
      state.turn = pi.spawn(function()
        local input = EXTENSION_POLICY.emit_input(text, nil, "interactive", nil,
          EXTENSION_CONTEXT_POLICY.snapshot(state))
        if input.action == "handled" then return end
        local transformed = input.action == "transform" and input.text or text
        bash_slice.flush_pending()
        compaction.pre_prompt_compaction()
        local before = EXTENSION_POLICY.emit_before_agent_start(transformed, nil,
          system_prompt, state.system_prompt_options,
          EXTENSION_CONTEXT_POLICY.snapshot(state))
        agent:set_system_prompt(before and before.systemPrompt or system_prompt)
        local prompts = { user_message(transformed) }
        for _, message in ipairs(before and before.messages or {}) do
          prompts[#prompts + 1] = {
            role = "custom", customType = message.customType,
            content = message.content, display = message.display,
            details = message.details, timestamp = state.wall_now_ms(),
          }
        end
        compaction.run_agent_prompt(prompts)
      end)
    end,
  }
  agent:subscribe(function(event)
    -- A disposed runtime's late events must not reach the current UI.
    if state.agent ~= agent then return end
    local signal = agent:get_state().signal
    EXTENSION_POLICY.emit_agent_event(event,
      EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = signal }),
      state.wall_now_ms, state.extension_turn_state)
    if event.type == "agent_end" then
      event.willRetry = compaction.will_retry_after_agent_end(event)
    end
    handle_agent_event(state, event)
    if state.event_hook then state.event_hook(event) end
    compaction.note_event(event)
  end)
  local start_event = state.next_session_start_event
    or { type = "session_start", reason = "startup" }
  state.next_session_start_event = nil
  EXTENSION_POLICY.emit_generic(start_event, EXTENSION_CONTEXT_POLICY.snapshot(state))
  -- Resource registration itself belongs to 9.7; discovery callbacks already
  -- run at Pi's real post-session_start seam and retain attributed results.
  state.extension_resources = EXTENSION_POLICY.emit_resources_discover(state.cwd,
    start_event.reason == "reload" and "reload" or "startup",
    EXTENSION_CONTEXT_POLICY.snapshot(state))
  update_available_provider_count(state)
  return startup
end

-- interactive-mode.ts renderCurrentSessionState.
local function render_current_session_state(state)
  state.transcript = {}
  state.last_status = nil
  state.steering_texts, state.follow_up_texts = {}, {}
  state.streaming_row = nil
  state.pending_tools = {}
  render_initial_messages(state)
end

-- interactive-mode.ts handleCompactCommand. The compact call runs on a
-- background coroutine (the spec's `await this.session.compact(...)`
-- aborts and settles the in-flight turn first); failures surface
-- through the compaction_end event, so the pcall result is dropped.
function handle_compact_command(state, custom_instructions)
  local entries = state.session_manager:get_entries()
  local message_count = 0
  for _, entry in ipairs(entries) do
    if entry.type == "message" then message_count = message_count + 1 end
  end
  if message_count < 2 then
    show_warning(state, "Nothing to compact (no messages yet)")
    return
  end
  -- loadingAnimation.stop() + statusContainer.clear().
  state.loader = nil
  local prior_turn = state.turn
  state.turn = pi.spawn(function()
    if prior_turn then
      state.session.abort()
      pcall(function() prior_turn:join() end)
    end
    pcall(state.session.compact, custom_instructions)
  end)
end

-- interactive-mode.ts handleFatalRuntimeError: surface the error and stop
-- with exit status 1.
local function handle_fatal_runtime_error(state, prefix, err)
  show_error(state, prefix .. ": " .. tostring(err))
  state.exit = true
  state.exit_code = 1
end

-- core/session-cwd.ts getMissingSessionCwdIssue.
local function missing_session_cwd_issue(session_manager, fallback_cwd)
  local session_file = session_manager:get_session_file()
  if not session_file then return nil end
  local session_cwd = session_manager:get_cwd()
  if not session_cwd or session_cwd == "" or pi.fs.exists(session_cwd) then return nil end
  return { sessionFile = session_file, sessionCwd = session_cwd, fallbackCwd = fallback_cwd }
end

-- core/session-cwd.ts formatMissingSessionCwdPrompt.
local function format_missing_session_cwd_prompt(issue)
  return "cwd from session file does not exist\n" .. issue.sessionCwd
    .. "\n\ncontinue in current cwd\n" .. issue.fallbackCwd
end

-- AgentSessionRuntime.switchSession's pi-rs slice: open, assert the stored
-- cwd, tear down the current runtime, and bind the new one. Returns
-- (true) on success, (nil, issue) when the stored cwd is missing.
local function attempt_switch_session(state, session_path, cwd_override)
  local before = EXTENSION_POLICY.emit_generic({ type = "session_before_switch",
    reason = "resume", targetSessionFile = session_path },
    EXTENSION_CONTEXT_POLICY.snapshot(state))
  if before and before.cancel then return false, { cancelled = true } end
  local session_manager = pi.session.open({
    path = session_path, agentDir = state.request.agentDir, cwd = cwd_override,
  })
  local issue = missing_session_cwd_issue(session_manager, state.cwd)
  if issue then return nil, issue end
  local previous_file = state.session_manager:get_session_file()
  EXTENSION_POLICY.emit_generic({ type = "session_shutdown", reason = "resume",
    targetSessionFile = session_manager:get_session_file() },
    EXTENSION_CONTEXT_POLICY.snapshot(state))
  state.extension_shutdown_emitted = true
  state.agent:abort()
  state.next_session_start_event = { type = "session_start", reason = "resume",
    previousSessionFile = previous_file }
  bind_session_runtime(state, session_manager)
  return true
end

-- interactive-mode.ts promptForMissingSessionCwd over showExtensionConfirm
-- (an extension selector titled `${title}\n${message}` with Yes/No rows).
local function prompt_for_missing_session_cwd(state, issue, on_done)
  show_selector(state, function(done)
    return extension_selector({
      theme = state.theme,
      title = "Session cwd not found\n" .. format_missing_session_cwd_prompt(issue),
      options = { "Yes", "No" },
      on_select = function(option)
        done()
        on_done(option == "Yes")
      end,
      on_cancel = function()
        done()
        on_done(false)
      end,
    })
  end)
end

function path_command_argument(text, command)
  if text == command or text:sub(1, #command + 1) ~= command .. " " then return nil end
  local args = text:sub(#command + 2):gsub("^%s+", "")
  if args == "" then return nil end
  local first = args:sub(1, 1)
  if first == '"' or first == "'" then
    local close = args:find(first, 2, true)
    return close and args:sub(2, close - 1) or nil
  end
  return args:match("^%S+")
end

function iso_from_epoch_ms(ms)
  local seconds = math.floor(ms / 1000)
  return os.date("!%Y-%m-%dT%H:%M:%S", seconds) .. (".%03dZ"):format(ms % 1000)
end

function handle_export_command(state, text)
  local output_path = path_command_argument(text, "/export")
  local ok, result = pcall(function()
    if output_path and output_path:sub(-6) == ".jsonl" then
      local resolved = pi.path.resolve(output_path)
      return state.session_manager:export_branch_jsonl(resolved, iso_from_epoch_ms(state.wall_now_ms()))
    end
    return export_html_lib.generate(state, output_path)
  end)
  if ok then show_status(state, "Session exported to: " .. result)
  else show_error(state, "Failed to export session: " .. tostring(result)) end
end

function finish_import(state, input_path, destination, cwd_override)
  local ok, switched, issue = pcall(attempt_switch_session, state, destination, cwd_override)
  if not ok then return handle_fatal_runtime_error(state, "Failed to import session", switched) end
  if not switched then
    prompt_for_missing_session_cwd(state, issue, function(confirmed)
      if not confirmed then show_status(state, "Import cancelled"); return end
      finish_import(state, input_path, destination, issue.fallbackCwd)
    end)
    return
  end
  render_current_session_state(state)
  show_status(state, "Session imported from: " .. input_path)
end

function handle_import_command(state, text)
  local input_path = path_command_argument(text, "/import")
  if not input_path then show_error(state, "Usage: /import <path.jsonl>"); return end
  show_selector(state, function(done)
    return extension_selector({
      theme = state.theme,
      title = "Import session\nReplace current session with " .. input_path .. "?",
      options = { "Yes", "No" },
      on_select = function(option)
        done()
        if option ~= "Yes" then show_status(state, "Import cancelled"); return end
        local resolved = pi.path.resolve(input_path)
        if not pi.fs.exists(resolved) then
          show_error(state, "Failed to import session: File not found: " .. resolved)
          return
        end
        local destination = pi.path.join(state.session_manager:get_session_dir(), pi.path.basename(resolved))
        if destination ~= resolved then
          pi.fs.mkdir(state.session_manager:get_session_dir())
          pi.fs.write_file(destination, pi.fs.read_bytes(resolved))
        end
        finish_import(state, input_path, destination, nil)
      end,
      on_cancel = function() done(); show_status(state, "Import cancelled") end,
    })
  end)
end



function handle_share_command(state)
  local auth_ok, auth = pcall(pi.exec, "gh", { "auth", "status" })
  if not auth_ok then
    show_error(state, "GitHub CLI (gh) is not installed. Install it from https://cli.github.com/")
    return
  end
  if auth.code ~= 0 then
    show_error(state, "GitHub CLI is not logged in. Run 'gh auth login' first.")
    return
  end

  local tmp_file = pi.path.join(pi.fs.tmpdir(), "session.html")
  local exported, export_error = pcall(export_html_lib.generate, state, tmp_file)
  if not exported then
    show_error(state, "Failed to export session: " .. tostring(export_error))
    return
  end

  local signal, loader = pi.abort_signal(), pi.tui.cancellable_loader("Creating gist...")
  local component = { focused = false, last_ms = state.wall_now_ms(), disposed = false }
  function component:set_focused(focused) self.focused = focused end
  function component:render(width)
    local border = dynamic_border_line(state.theme, width)
    return { border, state.theme:fg("accent", loader:frame()) .. " " .. state.theme:fg("muted", "Creating gist..."),
      "", " " .. select_key_hint(state.theme, "cancel", "cancel"), "", border }
  end
  local function restore()
    if component.disposed then return end
    component.disposed = true
    loader:dispose()
    restore_editor(state)
    pcall(pi.fs.remove_file, tmp_file)
  end
  function component:handle_input(data)
    loader:input(data)
    if loader:aborted() then
      signal:abort()
      restore()
      show_status(state, "Share cancelled")
    end
  end
  function component:needs_render(now_ms)
    local elapsed = math.max(0, now_ms - self.last_ms)
    self.last_ms = now_ms
    return loader:advance(elapsed)
  end
  mount_in_editor_slot(state, component)

  pi.spawn(function()
    local ok, result = pcall(pi.exec, "gh", { "gist", "create", "--public=false", tmp_file }, { signal = signal })
    if signal:is_aborted() then return end
    restore()
    if not ok then
      show_error(state, "Failed to create gist: " .. tostring(result))
      return
    end
    if result.code ~= 0 then
      local message = trim(result.stderr or "")
      show_error(state, "Failed to create gist: " .. (message ~= "" and message or "Unknown error"))
      return
    end
    local gist_url = trim(result.stdout or "")
    local gist_id = gist_url:match("([^/]+)$")
    if not gist_id or gist_id == "" then
      show_error(state, "Failed to parse gist ID from gh output")
      return
    end
    local base = os.getenv("PI_SHARE_VIEWER_URL") or "https://pi.dev/session/"
    show_status(state, "Share URL: " .. base .. "#" .. gist_id .. "\nGist: " .. gist_url)
  end)
end


function handle_copy_command(state)
  local messages = state.agent:get_state().messages
  local text = nil
  for index = #messages, 1, -1 do
    local message = messages[index]
    if message.role == "assistant"
       and not (message.stopReason == "aborted" and #(message.content or {}) == 0) then
      local parts = {}
      for _, content in ipairs(message.content or {}) do
        if content.type == "text" then parts[#parts + 1] = content.text end
      end
      local joined = trim(table.concat(parts))
      if joined ~= "" then text = joined end
      break
    end
  end
  if not text then show_error(state, "No agent messages to copy yet."); return end
  local ok, err = pcall(function()
    if state.copy_text then state.copy_text(text) else pi.clipboard.write_text(text) end
  end)
  if ok then show_status(state, "Copied last agent message to clipboard")
  else show_error(state, tostring(err)) end
end

-- utils/changelog.ts parseChangelog. The checked-in markdown is the pinned
-- package asset; parity drivers may inject a smaller changelogText fixture.
function parse_changelog(markdown)
  local entries, current, lines = {}, nil, {}
  local function finish()
    if current and #lines > 0 then
      current.content = trim(table.concat(lines, "\n"))
      entries[#entries + 1] = current
    end
  end
  for line in (markdown .. "\n"):gmatch("([^\n]*)\n") do
    if line:sub(1, 3) == "## " then
      finish()
      local major, minor, patch = line:match("^##%s+%[?(%d+)%.(%d+)%.(%d+)%]?")
      if major then
        current = { major = tonumber(major), minor = tonumber(minor), patch = tonumber(patch) }
        lines = { line }
      else
        current, lines = nil, {}
      end
    elseif current then
      lines[#lines + 1] = line
    end
  end
  finish()
  return entries
end

function normalize_changelog_links(markdown, entry)
  local tag = "v" .. entry.major .. "." .. entry.minor .. "." .. entry.patch
  local repo = "https://github.com/earendil-works/pi"
  local function target(value)
    value = value:gsub("^https://github%.com/badlogic/pi%-mono", repo)
      :gsub("^https://github%.com/earendil%-works/pi%-mono", repo)
    for _, route in ipairs({ "blob", "tree" }) do
      for _, branch in ipairs({ "main", "master" }) do
        local prefix = repo .. "/" .. route .. "/" .. branch .. "/"
        if value:sub(1, #prefix) == prefix then
          value = repo .. "/" .. route .. "/" .. tag .. "/" .. value:sub(#prefix + 1)
        end
      end
    end
    if value:sub(1, 1) == "#" or value:sub(1, 2) == "//"
       or value:match("^[a-zA-Z][a-zA-Z0-9+%.%-]*:") then return value end
    local before_fragment, fragment = value:match("^([^#]*)(#.*)$")
    if not before_fragment then before_fragment, fragment = value, "" end
    local path_part, query = before_fragment:match("^([^?]*)(%?.*)$")
    if not path_part then path_part, query = before_fragment, "" end
    if path_part == "" then return value end
    path_part = path_part:gsub("\\", "/")
    local repository_path
    if path_part:sub(1, 1) == "/" then repository_path = path_part:gsub("^/+", "")
    else repository_path = "packages/coding-agent/" .. path_part end
    local parts = {}
    for part in repository_path:gmatch("[^/]+") do
      if part == ".." then
        if #parts == 0 then return value end
        parts[#parts] = nil
      elseif part ~= "." then parts[#parts + 1] = part end
    end
    repository_path = table.concat(parts, "/")
    if repository_path == "" then return value end
    local basename = repository_path:match("([^/]+)$") or repository_path
    local route = (path_part:sub(-1) == "/" or not basename:find("%.", 1, false)) and "tree" or "blob"
    repository_path = repository_path:gsub(" ", "%%20")
    return repo .. "/" .. route .. "/" .. tag .. "/" .. repository_path .. query .. fragment
  end
  return (markdown:gsub("(!?%[[^%]\n]+%]%()([^%s%)]+)([^%)]*%))",
    function(prefix, link, suffix) return prefix .. target(link) .. suffix end))
end

function changelog_entry_newer_than(entry, version)
  local parts = {}
  for value in tostring(version or ""):gmatch("[^%.]+") do
    parts[#parts + 1] = tonumber(value) or 0
  end
  local major, minor, patch = parts[1] or 0, parts[2] or 0, parts[3] or 0
  if entry.major ~= major then return entry.major > major end
  if entry.minor ~= minor then return entry.minor > minor end
  return entry.patch > patch
end

function truthy_env_flag(value)
  if value == nil or value == "" then return false end
  value = tostring(value):lower()
  return value == "1" or value == "true" or value == "yes"
end

function pi_user_agent(state, version)
  if state.request.userAgent then return state.request.userAgent end
  -- Runtime identity is implementation-specific; retain Pi's stable product
  -- prefix and platform shape for startup endpoints.
  return "pi/" .. version .. " (" .. pi.platform() .. "; rust; unknown)"
end

function report_install_telemetry(state, version)
  if pi.env.PI_OFFLINE ~= nil and pi.env.PI_OFFLINE ~= ""
     and not state.request.forceStartupNetwork then return false end
  local enabled
  if state.request.telemetryEnabled ~= nil then enabled = state.request.telemetryEnabled
  elseif pi.env.PI_TELEMETRY ~= nil then enabled = truthy_env_flag(pi.env.PI_TELEMETRY)
  else enabled = pi.settings.enable_install_telemetry() end
  if not enabled then return false end
  local url = state.request.telemetryUrl
    or ("https://pi.dev/api/report-install?version=" .. version)
  return pi.spawn(function()
    pcall(pi.http.get, url, {
      headers = { ["User-Agent"] = pi_user_agent(state, version) },
      timeout_ms = 5000,
    })
  end)
end

function parse_package_version(value)
  local cleaned = trim(tostring(value or "")):gsub("%+.*$", "")
  local major, minor, patch, prerelease = cleaned
    :match("^v?(%d+)%.(%d+)%.(%d+)%-([0-9A-Za-z%.%-]+)$")
  if not major then major, minor, patch = cleaned:match("^v?(%d+)%.(%d+)%.(%d+)$") end
  if not major then return nil end
  return { tonumber(major), tonumber(minor), tonumber(patch), prerelease }
end

function is_newer_package_version(candidate, current)
  local left, right = parse_package_version(candidate), parse_package_version(current)
  if not left or not right then return trim(candidate) ~= trim(current) end
  for index = 1, 3 do
    if left[index] ~= right[index] then return left[index] > right[index] end
  end
  if left[4] == right[4] then return false end
  if left[4] == nil then return true end
  if right[4] == nil then return false end
  return left[4] > right[4]
end

function mount_update_notification(state, release)
  state.transcript[#state.transcript + 1] = {
    kind = "update_available", version = trim(release.version),
    package_name = release.packageName, note = release.note and trim(release.note) or nil,
  }
end

function start_version_check(state)
  local skip = (pi.env.PI_SKIP_VERSION_CHECK ~= nil and pi.env.PI_SKIP_VERSION_CHECK ~= "")
    or (pi.env.PI_OFFLINE ~= nil and pi.env.PI_OFFLINE ~= "")
  if skip and not state.request.forceStartupNetwork then return nil end
  local task = pi.spawn(function()
    local ok, response = pcall(pi.http.get,
      state.request.versionCheckUrl or "https://pi.dev/api/latest-version", {
        headers = {
          ["User-Agent"] = pi_user_agent(state, state.version),
          accept = "application/json",
        },
        timeout_ms = state.request.versionCheckTimeoutMs or 10000,
      })
    if not ok or not response.ok then return nil end
    local decoded, release = pcall(pi.json.decode, response.body)
    if not decoded or type(release) ~= "table" or type(release.version) ~= "string"
       or trim(release.version) == "" then return nil end
    release.version = trim(release.version)
    if type(release.packageName) ~= "string" or trim(release.packageName) == "" then
      release.packageName = nil
    else release.packageName = trim(release.packageName) end
    if type(release.note) ~= "string" or trim(release.note) == "" then
      release.note = nil
    else release.note = trim(release.note) end
    if is_newer_package_version(release.version, state.version) then
      mount_update_notification(state, release)
      state.async_render = true
      return release
    end
    return nil
  end)
  state.version_check_task = task
  return task
end

-- interactive-mode.ts getChangelogForDisplay + showStartupNoticesIfNeeded.
-- Telemetry is launched by create_interactive_state after this policy reports
-- that it recorded a fresh/new version.
function mount_startup_changelog(state, options)
  options = options or {}
  if #(options.messages or {}) > 0 then return nil end
  local last_version = options.last_version
  if options.fresh then last_version = nil
  elseif last_version == nil then last_version = pi.settings.last_changelog_version() end
  local version = options.version or state.version
  if version == nil then return { displayed = false } end
  if last_version == nil then
    if not options.no_persist then pi.settings.set_last_changelog_version(version) end
    return { displayed = false, recorded = version, telemetry_due = true }
  end
  local chunks, latest = {}, nil
  for _, entry in ipairs(parse_changelog(options.markdown or CHANGELOG_MD)) do
    if changelog_entry_newer_than(entry, last_version) then
      chunks[#chunks + 1] = normalize_changelog_links(entry.content, entry)
      if latest == nil then latest = entry end
    end
  end
  if #chunks == 0 then return { displayed = false } end
  if not options.no_persist then pi.settings.set_last_changelog_version(version) end
  local latest_version = latest.major .. "." .. latest.minor .. "." .. latest.patch
  local collapsed = options.collapsed
  if collapsed == nil then collapsed = pi.settings.collapse_changelog() end
  state.transcript[#state.transcript + 1] = {
    kind = "startup_changelog", markdown = table.concat(chunks, "\n\n"),
    collapsed = collapsed, latest_version = latest_version,
  }
  return { displayed = true, recorded = version, telemetry_due = true, latest_version = latest_version }
end

function handle_changelog_command(state)
  local all = parse_changelog(state.request.changelogText or CHANGELOG_MD)
  local chunks = {}
  for index = #all, 1, -1 do
    chunks[#chunks + 1] = normalize_changelog_links(all[index].content, all[index])
  end
  state.transcript[#state.transcript + 1] = {
    kind = "info_block", title = "What's New",
    markdown = #chunks > 0 and table.concat(chunks, "\n\n") or "No changelog entries found.",
  }
end

HOTKEY_KEYS = {
  ["tui.editor.cursorUp"] = "up", ["tui.editor.cursorDown"] = "down",
  ["tui.editor.cursorLeft"] = { "left", "ctrl+b" },
  ["tui.editor.cursorRight"] = { "right", "ctrl+f" },
  ["tui.editor.cursorWordLeft"] = { "alt+left", "ctrl+left", "alt+b" },
  ["tui.editor.cursorWordRight"] = { "alt+right", "ctrl+right", "alt+f" },
  ["tui.editor.cursorLineStart"] = { "home", "ctrl+a" },
  ["tui.editor.cursorLineEnd"] = { "end", "ctrl+e" },
  ["tui.editor.jumpForward"] = "ctrl+]", ["tui.editor.jumpBackward"] = "ctrl+alt+]",
  ["tui.editor.pageUp"] = "pageUp", ["tui.editor.pageDown"] = "pageDown",
  ["tui.input.submit"] = "enter", ["tui.input.newLine"] = "shift+enter",
  ["tui.editor.deleteWordBackward"] = { "ctrl+w", "alt+backspace" },
  ["tui.editor.deleteWordForward"] = { "alt+d", "alt+delete" },
  ["tui.editor.deleteToLineStart"] = "ctrl+u", ["tui.editor.deleteToLineEnd"] = "ctrl+k",
  ["tui.editor.yank"] = "ctrl+y", ["tui.editor.yankPop"] = "alt+y",
  ["tui.editor.undo"] = "ctrl+-", ["tui.input.tab"] = "tab",
}

function display_key(action)
  local value = DEFAULT_KEYS[action] or HOTKEY_KEYS[action] or ""
  if type(value) == "table" then value = table.concat(value, "/") end
  return format_key(value, true)
end

function handle_hotkeys_command(state)
  local k = display_key
  local hotkeys = ([=[
**Navigation**
| Key | Action |
|-----|--------|
| `%s` / `%s` / `%s` / `%s` | Move cursor / browse history |
| `%s` / `%s` | Move by word |
| `%s` | Start of line |
| `%s` | End of line |
| `%s` | Jump forward to character |
| `%s` | Jump backward to character |
| `%s` / `%s` | Scroll by page |

**Editing**
| Key | Action |
|-----|--------|
| `%s` | Send message |
| `%s` | New line |
| `%s` | Delete word backwards |
| `%s` | Delete word forwards |
| `%s` | Delete to start of line |
| `%s` | Delete to end of line |
| `%s` | Paste the most-recently-deleted text |
| `%s` | Cycle through the deleted text after pasting |
| `%s` | Undo |

**Other**
| Key | Action |
|-----|--------|
| `%s` | Path completion / accept autocomplete |
| `%s` | Cancel autocomplete / abort streaming |
| `%s` | Clear editor (first) / exit (second) |
| `%s` | Exit (when editor is empty) |
| `%s` | Suspend to background |
| `%s` | Cycle thinking level |
| `%s` / `%s` | Cycle models |
| `%s` | Open model selector |
| `%s` | Toggle tool output expansion |
| `%s` | Toggle thinking block visibility |
| `%s` | Edit message in external editor |
| `%s` | Queue follow-up message |
| `%s` | Restore queued messages |
| `%s` | Paste image from clipboard |
| `/` | Slash commands |
| `!` | Run bash command |
| `!!` | Run bash command (excluded from context) |
 ]=]):format(
    k("tui.editor.cursorUp"), k("tui.editor.cursorDown"), k("tui.editor.cursorLeft"), k("tui.editor.cursorRight"),
    k("tui.editor.cursorWordLeft"), k("tui.editor.cursorWordRight"),
    k("tui.editor.cursorLineStart"), k("tui.editor.cursorLineEnd"),
    k("tui.editor.jumpForward"), k("tui.editor.jumpBackward"),
    k("tui.editor.pageUp"), k("tui.editor.pageDown"),
    k("tui.input.submit"), k("tui.input.newLine"), k("tui.editor.deleteWordBackward"),
    k("tui.editor.deleteWordForward"), k("tui.editor.deleteToLineStart"),
    k("tui.editor.deleteToLineEnd"), k("tui.editor.yank"), k("tui.editor.yankPop"),
    k("tui.editor.undo"), k("tui.input.tab"), k("app.interrupt"), k("app.clear"),
    k("app.exit"), k("app.suspend"), k("app.thinking.cycle"),
    k("app.model.cycleForward"), k("app.model.cycleBackward"), k("app.model.select"),
    k("app.tools.expand"), k("app.thinking.toggle"), k("app.editor.external"),
    k("app.message.followUp"), k("app.message.dequeue"), k("app.clipboard.pasteImage"))
  local shortcuts = pi.registered_shortcuts()
  if #shortcuts > 0 then
    hotkeys = hotkeys .. "\n\n**Extensions**\n| Key | Action |\n|-----|--------|\n"
    for _, shortcut in ipairs(shortcuts) do
      hotkeys = hotkeys .. "| `" .. format_key(shortcut.shortcut, true) .. "` | "
        .. (shortcut.description or shortcut.source or "") .. " |\n"
    end
  end
  state.transcript[#state.transcript + 1] = {
    kind = "info_block", title = "Keyboard Shortcuts", markdown = trim(hotkeys),
  }
end

function iso_timestamp(ms)
  ms = ms or pi.now_ms()
  return os.date("!%Y-%m-%dT%H:%M:%S", math.floor(ms / 1000))
    .. string.format(".%03dZ", math.floor(ms % 1000))
end

function handle_debug_command(state)
  local width, height = state.columns or 80, state.rows or 24
  local all_lines = frontend_frame(state, width)
  local debug_path = pi.path.join(state.request.agentDir or pi.path.join(state.home or "", ".pi/agent"),
    state.app_name .. "-debug.log")
  local out = {
    "Debug output at " .. iso_timestamp(state.request.nowMs),
    "Terminal: " .. width .. "x" .. height,
    "Total lines: " .. #all_lines, "",
    "=== All rendered lines with visible widths ===",
  }
  for index, line in ipairs(all_lines) do
    out[#out + 1] = "[" .. (index - 1) .. "] (w=" .. pi.tui.visible_width(line) .. ") "
      .. pi.json.encode(line)
  end
  out[#out + 1] = ""
  out[#out + 1] = "=== Agent messages (JSONL) ==="
  for _, message in ipairs(state.agent:get_state().messages or {}) do
    out[#out + 1] = pi.json.encode(message)
  end
  out[#out + 1] = ""
  pi.fs.mkdir(pi.path.dirname(debug_path))
  pi.fs.write_file(debug_path, table.concat(out, "\n"))
  state.transcript[#state.transcript + 1] = {
    kind = "text", padding_y = 1,
    text = state.theme:fg("accent", "✓ Debug log written") .. "\n"
      .. state.theme:fg("muted", debug_path),
  }
end

-- Hidden interactive-mode easter eggs. Components are transcript policy; the
-- timer coroutines only advance their state and request the next product frame.
function animate_armin(state, row)
  return pi.spawn(function()
    -- Node's setInterval truncates 1000/fps to integer milliseconds.
    local delay = row.effect == "glitch" and 16 or 33
    while true do
      pi.sleep(delay)
      local done = tick_armin(row)
      state.async_render = true
      if done then return end
    end
  end)
end

function handle_armin_says_hi(state)
  state.transcript[#state.transcript + 1] = { kind = "spacer" }
  local row = new_armin_row()
  state.transcript[#state.transcript + 1] = row
  row.task = animate_armin(state, row)
  state.async_render = true
end

function handle_demented_delves(state)
  state.transcript[#state.transcript + 1] = { kind = "spacer" }
  state.transcript[#state.transcript + 1] = { kind = "earendil" }
  state.async_render = true
end

function handle_daxnuts(state)
  state.transcript[#state.transcript + 1] = { kind = "spacer" }
  local row = { kind = "daxnuts", tick = 0 }
  state.transcript[#state.transcript + 1] = row
  row.task = pi.spawn(function()
    while row.tick < 25 do
      pi.sleep(80)
      row.tick = row.tick + 1
      state.async_render = true
    end
  end)
  state.async_render = true
end

function check_daxnuts_easter_egg(state, model)
  if model and model.provider == "opencode"
    and string.find(string.lower(model.id or ""), "kimi%-k2%.5")
  then handle_daxnuts(state) end
end

-- interactive-mode.ts handleReloadCommand. The static bordered box is mounted
-- before the asynchronous reload, then the current session runtime is rebuilt
-- over freshly loaded settings/context files and the transcript is re-rendered.
function reload_box_component(theme)
  local box = {}
  function box:handle_input(_) end
  function box:render(width)
    local lines = { dynamic_border_line(theme, width), "" }
    append(lines, pi.tui.text_render(theme:fg("muted",
      "Reloading keybindings, extensions, skills, prompts, themes..."), width, 1, 0))
    lines[#lines + 1] = ""
    lines[#lines + 1] = dynamic_border_line(theme, width)
    return lines
  end
  return box
end

function handle_reload_command(state)
  if state.session.is_streaming() then
    show_warning(state, "Wait for the current response to finish before reloading.")
    return nil
  end
  if state.session.is_compacting and state.session.is_compacting() then
    show_warning(state, "Wait for compaction to finish before reloading.")
    return nil
  end

  state.reload_box = reload_box_component(state.theme)
  state.set_editor_focus(false)
  state.async_render = true
  local previous_editor = state.editor
  local task = pi.spawn(function()
    -- process.nextTick: guarantee the static box gets a render opportunity.
    pi.sleep(state.request.reloadYieldMs or 1)
    local ok, err = pcall(function()
      if state.reload_impl then return state.reload_impl(state) end
      EXTENSION_POLICY.emit_generic({ type = "session_shutdown", reason = "reload" },
        EXTENSION_CONTEXT_POLICY.snapshot(state))
      state.extension_shutdown_emitted = true
      state.next_session_start_event = { type = "session_start", reason = "reload" }
      pi.settings.reload()
      local theme_name = pi.settings.theme()
      local data = theme_name == "light" and light_json or dark_json
      state.theme_data = data
      state.theme = create_theme(data, state.request.colorMode or "truecolor")
      state.md_theme = get_markdown_theme(state.theme)
      state.hide_thinking_block = pi.settings.hide_thinking_block()
      state.double_escape_action = pi.settings.double_escape_action()

      -- AgentSession.reload rebuilds the runtime over the existing manager;
      -- build_session_system_prompt rereads AGENTS.md/context files here.
      bind_session_runtime(state, state.session_manager)
      setup_shell_editor(state)
      state.fd_path = state.request.fdPath or resolve_fd_path()
      state.autocomplete_provider = create_base_autocomplete_provider(state)
      state.editor.editor:set_autocomplete_triggers({})
      state.editor:on_action("app.model.select", function() show_model_selector(state) end)
      state.editor:on_action("app.model.cycleForward", function() cycle_model(state, "forward") end)
      state.editor:on_action("app.model.cycleBackward", function() cycle_model(state, "backward") end)
      render_current_session_state(state)
    end)

    state.reload_box = nil
    state.set_editor_focus(true)
    if ok then
      show_status(state, "Reloaded keybindings, extensions, skills, prompts, themes")
    else
      state.editor = previous_editor
      state.set_editor_focus(true)
      show_error(state, "Reload failed: " .. tostring(err))
    end
    state.async_render = true
  end)
  state.reload_task = task
  return task
end



-- interactive-mode.ts handleResumeSession.
function handle_resume_session(state, session_path)
  state.loader = nil
  local ok, switched, issue = pcall(attempt_switch_session, state, session_path, nil)
  if not ok then
    return handle_fatal_runtime_error(state, "Failed to resume session", switched)
  end
  if switched then
    render_current_session_state(state)
    show_status(state, "Resumed session")
    return
  end
  if issue and issue.cancelled then
    show_status(state, "Resume cancelled")
    return
  end
  prompt_for_missing_session_cwd(state, issue, function(confirmed)
    if not confirmed then
      show_status(state, "Resume cancelled")
      return
    end
    local retry_ok, retry_err = pcall(attempt_switch_session, state, session_path, issue.fallbackCwd)
    if not retry_ok then
      return handle_fatal_runtime_error(state, "Failed to resume session", retry_err)
    end
    render_current_session_state(state)
    show_status(state, "Resumed session in current cwd")
  end)
end

-- interactive-mode.ts handleClearCommand (/new) over
-- AgentSessionRuntime.newSession.
function handle_clear_command(state)
  state.loader = nil
  local cancelled = false
  local ok, err = pcall(function()
    local before = EXTENSION_POLICY.emit_generic({ type = "session_before_switch",
      reason = "new" }, EXTENSION_CONTEXT_POLICY.snapshot(state))
    if before and before.cancel then cancelled = true; return end
    local current = state.session_manager
    local previous_file = current:get_session_file()
    local session_manager
    if current:is_persisted() then
      session_manager = pi.session.create({
        cwd = state.cwd, sessionDir = current:get_session_dir(),
        agentDir = state.request.agentDir,
      })
    else
      session_manager = pi.session.in_memory({ cwd = state.cwd })
    end
    EXTENSION_POLICY.emit_generic({ type = "session_shutdown", reason = "new",
      targetSessionFile = session_manager:get_session_file() },
      EXTENSION_CONTEXT_POLICY.snapshot(state))
    state.extension_shutdown_emitted = true
    state.agent:abort()
    state.next_session_start_event = { type = "session_start", reason = "new",
      previousSessionFile = previous_file }
    bind_session_runtime(state, session_manager)
  end)
  if not ok then
    return handle_fatal_runtime_error(state, "Failed to create session", err)
  end
  if cancelled then return end
  render_current_session_state(state)
  state.transcript[#state.transcript + 1] = {
    kind = "text",
    text = state.theme:fg("accent", "✓ New session started"),
    padding_y = 1,
  }
end

-- interactive-mode.ts handleNameCommand.
function handle_name_command(state, text)
  local name = trim((text:gsub("^/name%s*", "", 1)))
  if name == "" then
    local current_name = state.session_manager:get_session_name()
    if current_name then
      state.transcript[#state.transcript + 1] = {
        kind = "text",
        text = state.theme:fg("dim", "Session name: " .. current_name),
      }
    else
      show_warning(state, "Usage: /name <name>")
    end
    return
  end
  -- agent-session.ts setSessionName → sessionManager.appendSessionInfo.
  state.session_manager:append_session_info(name)
  state.transcript[#state.transcript + 1] = {
    kind = "text",
    text = state.theme:fg("dim", "Session name set: " .. name),
  }
end

-- agent-session.ts getSessionStats + interactive-mode.ts
-- handleSessionCommand.
function handle_session_command(state)
  local theme = state.theme
  local messages = state.agent:get_state().messages
  local user_messages, assistant_messages, tool_results = 0, 0, 0
  local tool_calls = 0
  local input, output, cache_read, cache_write, cost = 0, 0, 0, 0, 0
  for _, message in ipairs(messages) do
    if message.role == "user" then user_messages = user_messages + 1
    elseif message.role == "toolResult" then tool_results = tool_results + 1
    elseif message.role == "assistant" then
      assistant_messages = assistant_messages + 1
      for _, block in ipairs(message.content or {}) do
        if block.type == "toolCall" then tool_calls = tool_calls + 1 end
      end
      local usage = message.usage or {}
      input = input + (usage.input or 0)
      output = output + (usage.output or 0)
      cache_read = cache_read + (usage.cacheRead or 0)
      cache_write = cache_write + (usage.cacheWrite or 0)
      cost = cost + ((usage.cost and usage.cost.total) or 0)
    end
  end

  local session_name = state.session_manager:get_session_name()
  local info = theme:bold("Session Info") .. "\n\n"
  if session_name then
    info = info .. theme:fg("dim", "Name:") .. " " .. session_name .. "\n"
  end
  info = info .. theme:fg("dim", "File:") .. " "
    .. (state.session_manager:get_session_file() or "In-memory") .. "\n"
  info = info .. theme:fg("dim", "ID:") .. " " .. state.session_manager:get_session_id() .. "\n\n"
  info = info .. theme:bold("Messages") .. "\n"
  info = info .. theme:fg("dim", "User:") .. " " .. user_messages .. "\n"
  info = info .. theme:fg("dim", "Assistant:") .. " " .. assistant_messages .. "\n"
  info = info .. theme:fg("dim", "Tool Calls:") .. " " .. tool_calls .. "\n"
  info = info .. theme:fg("dim", "Tool Results:") .. " " .. tool_results .. "\n"
  info = info .. theme:fg("dim", "Total:") .. " " .. #messages .. "\n\n"
  info = info .. theme:bold("Tokens") .. "\n"
  info = info .. theme:fg("dim", "Input:") .. " " .. to_locale_string(input) .. "\n"
  info = info .. theme:fg("dim", "Output:") .. " " .. to_locale_string(output) .. "\n"
  if cache_read > 0 then
    info = info .. theme:fg("dim", "Cache Read:") .. " " .. to_locale_string(cache_read) .. "\n"
  end
  if cache_write > 0 then
    info = info .. theme:fg("dim", "Cache Write:") .. " " .. to_locale_string(cache_write) .. "\n"
  end
  info = info .. theme:fg("dim", "Total:")
    .. " " .. to_locale_string(input + output + cache_read + cache_write) .. "\n"
  if cost > 0 then
    info = info .. "\n" .. theme:bold("Cost") .. "\n"
    info = info .. theme:fg("dim", "Total:") .. " " .. string.format("%.4f", cost)
  end

  state.transcript[#state.transcript + 1] = { kind = "text", text = info }
end

-- interactive-mode.ts showSessionSelector.
function show_session_selector(state)
  show_selector(state, function(done)
    local session_manager = state.session_manager
    return session_selector({
      theme = state.theme,
      home = state.home,
      now_ms = state.wall_now_ms,
      load_current = function()
        return pi.session.list({
          cwd = session_manager:get_cwd(),
          sessionDir = session_manager:get_session_dir(),
          agentDir = state.request.agentDir,
        })
      end,
      load_all = function()
        if session_manager:uses_default_session_dir() then
          return pi.session.list_all({ agentDir = state.request.agentDir })
        end
        return pi.session.list_all({
          sessionDir = session_manager:get_session_dir(),
          agentDir = state.request.agentDir,
        })
      end,
      on_select = function(session_path)
        done()
        handle_resume_session(state, session_path)
      end,
      on_cancel = function() done() end,
      on_exit = function() shutdown(state) end,
      rename_session = function(session_file, next_name)
        local next_value = trim(next_name or "")
        if next_value == "" then return end
        pi.session.open({ path = session_file, agentDir = state.request.agentDir })
          :append_session_info(next_value)
      end,
      show_rename_hint = true,
      current_session_file = session_manager:get_session_file(),
      delete_file = state.delete_session_file,
    })
  end)
end

-- ===========================================================================
-- Tree navigation — ports of components/tree-selector.ts,
-- components/user-message-selector.ts, agent-session.ts navigateTree +
-- getUserMessagesForForking, and agent-session-runtime.ts fork over the
-- pi.session branching mechanism (PLAN 6.4). The session_before_tree /
-- session_tree / session_before_fork extension events join item 9.
-- ===========================================================================

-- Split exactly like JS String.split(" "): editor arguments are deliberately
-- simple, and repeated/trailing spaces remain empty arguments.
function split_editor_command(command)
  local parts, start = {}, 1
  while true do
    local at = command:find(" ", start, true)
    if not at then parts[#parts + 1] = command:sub(start); break end
    parts[#parts + 1] = command:sub(start, at - 1)
    start = at + 1
  end
  return parts
end

function open_external_editor(state, editor, prefix, suffix, warn_when_missing, expanded_text)
  local editor_command = state.external_editor_command
  if editor_command == nil then editor_command = pi.env.VISUAL or pi.env.EDITOR end
  if not editor_command or editor_command == "" then
    if warn_when_missing then
      show_warning(state, "No editor configured. Set $VISUAL or $EDITOR environment variable.")
    end
    return
  end

  local temp_file = pi.path.join(pi.fs.tmpdir(), prefix .. tostring(pi.now_ms()) .. suffix)
  local write_ok, write_error = pcall(pi.fs.write_file, temp_file,
    expanded_text and editor:get_expanded_text() or editor:get_text())
  if not write_ok then
    pcall(pi.fs.remove_file, temp_file)
    error(write_error)
  end
  local command = split_editor_command(editor_command)
  local program = table.remove(command, 1)
  command[#command + 1] = temp_file
  state.next_inherited_process_id = (state.next_inherited_process_id or 0) + 1
  local id = "external-editor-" .. tostring(state.next_inherited_process_id)
  state.inherited_process_callbacks[id] = function(status)
    local read_ok, content = true, nil
    if status == 0 then read_ok, content = pcall(pi.fs.read_file, temp_file) end
    pcall(pi.fs.remove_file, temp_file)
    if not read_ok then error(content) end
    if status == 0 then editor:set_text((content:gsub("\n$", ""))) end
  end
  state.pending_inherited_process = {
    id = id, program = program, args = command, shell = pi.platform() == "win32",
    message = "Launching external editor: " .. editor_command
      .. "\nPi will resume when the editor exits.\n",
  }
end

function take_inherited_process(state)
  local action = state.pending_inherited_process
  state.pending_inherited_process = nil
  return action
end

-- Deterministic policy driver for interactive-mode.ts handleCtrlZ. The live
-- process-group stop/resume is exercised through pi.tui.process_session.
pi.register_command("interactive-suspend-policy", {
  handler = function(platform)
    local state = { platform = platform, transcript = {} }
    handle_suspend(state)
    return { suspend = take_suspend(state), transcript = state.transcript }
  end,
})

-- Deterministic driver for interactive-mode.ts/openExternalEditor and
-- ExtensionEditorComponent.openExternalEditor. Process ownership itself is
-- exercised by the public live-process example and PTY evidence.
pi.register_command("interactive-external-editor-policy", {
  handler = function(args)
    local request = pi.json.decode(args)
    local editor = pi.tui.editor(request.text or "")
    local state = { external_editor_command = request.editorCommand,
      inherited_process_callbacks = {}, next_inherited_process_id = 0 }
    open_external_editor(state, editor, request.prefix or "pi-editor-",
      request.suffix or ".pi.md", false, request.expanded == true)
    local action = take_inherited_process(state)
    if not action then return { text = editor:get_text() } end
    local temp_file = action.args[#action.args]
    local initial = pi.fs.read_file(temp_file)
    if request.replacement ~= nil then pi.fs.write_file(temp_file, request.replacement) end
    state.inherited_process_callbacks[action.id](request.status)
    return {
      text = editor:get_text(), initial = initial, tempExists = pi.fs.exists(temp_file),
      program = action.program, args = action.args, message = action.message,
    }
  end,
})

-- components/extension-editor.ts — the multi-line editor mounted in the
-- editor slot (the summarize-branch custom-prompt flow).
function extension_editor(opts)
  local theme = opts.theme
  local self = { editor = pi.tui.editor(opts.prefill or ""), focused = false }
  self.editor:set_border_style(theme.fg_codes.borderMuted, "\27[39m")
  function self:set_focused(focused)
    self.focused = focused
    self.editor:set_focused(focused)
  end
  function self:handle_input(data)
    if binding_matches(data, SELECT_KEYS.cancel) then
      opts.on_cancel()
      return
    end
    if binding_matches(data, DEFAULT_KEYS["app.editor.external"]) then
      if opts.open_external_editor then opts.open_external_editor(self.editor) end
      return
    end
    local effect = self.editor:input_effect(data)
    if effect and effect.kind == "submit" then opts.on_submit(effect.text or "") end
  end
  function self:render(width)
    local lines = { dynamic_border_line(theme, width), "" }
    append(lines, pi.tui.text_render(theme:fg("accent", opts.title), width, 1, 0))
    lines[#lines + 1] = ""
    for _, line in ipairs(self.editor:render(width)) do lines[#lines + 1] = line end
    lines[#lines + 1] = ""
    local hint = select_key_hint(theme, "confirm", "submit") .. "  "
      .. raw_key_hint(theme, "shift+enter", "newline") .. "  "
      .. select_key_hint(theme, "cancel", "cancel")
    if os.getenv("VISUAL") or os.getenv("EDITOR") then
      hint = hint .. "  " .. app_key_hint(theme, "app.editor.external", "external editor")
    end
    append(lines, pi.tui.text_render(hint, width, 1, 0))
    lines[#lines + 1] = ""
    lines[#lines + 1] = dynamic_border_line(theme, width)
    return lines
  end
  return self
end

-- interactive-mode.ts showExtensionSelector / showExtensionEditor over
-- the editor-slot swap; on_done(nil) is the promise's undefined.
function show_extension_choice(state, title, options, on_done)
  show_selector(state, function(done)
    return extension_selector({
      theme = state.theme,
      title = title,
      options = options,
      on_toggle_tools_expanded = function()
        set_tools_expanded(state, not state.tools_expanded)
      end,
      on_select = function(option) done(); on_done(option) end,
      on_cancel = function() done(); on_done(nil) end,
    })
  end)
end

function show_extension_editor(state, title, prefill, on_done)
  show_selector(state, function(done)
    return extension_editor({
      theme = state.theme,
      title = title,
      prefill = prefill,
      on_submit = function(value) done(); on_done(value) end,
      on_cancel = function() done(); on_done(nil) end,
      open_external_editor = function(editor)
        open_external_editor(state, editor, "pi-extension-editor-", ".md", false, false)
      end,
    })
  end)
end

-- components/user-message-selector.ts.
function user_message_selector(opts)
  local theme = opts.theme
  local messages = opts.messages
  local self = { selected = math.max(0, #messages - 1), focused = false }
  if opts.initial_selected_id then
    for index, message in ipairs(messages) do
      if message.id == opts.initial_selected_id then self.selected = index - 1 end
    end
  end
  local MAX_VISIBLE = 10

  function self:set_focused(focused) self.focused = focused end

  function self:handle_input(data)
    if binding_matches(data, SELECT_KEYS.up) then
      self.selected = self.selected == 0 and #messages - 1 or self.selected - 1
    elseif binding_matches(data, SELECT_KEYS.down) then
      self.selected = self.selected == #messages - 1 and 0 or self.selected + 1
    elseif binding_matches(data, SELECT_KEYS.confirm) then
      local selected = messages[self.selected + 1]
      if selected then opts.on_select(selected.id) end
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      opts.on_cancel()
    end
  end

  local function list_lines(width)
    local lines = {}
    if #messages == 0 then
      lines[#lines + 1] = theme:fg("muted", "  No user messages found")
      return lines
    end
    local start_index = math.max(0, math.min(
      self.selected - math.floor(MAX_VISIBLE / 2), #messages - MAX_VISIBLE))
    local end_index = math.min(start_index + MAX_VISIBLE, #messages)
    for i = start_index, end_index - 1 do
      local message = messages[i + 1]
      local is_selected = i == self.selected
      local normalized = trim((message.text:gsub("\n", " ")))
      local cursor = is_selected and theme:fg("accent", "› ") or "  "
      local truncated = pi.tui.truncate(normalized, width - 2)
      lines[#lines + 1] = cursor .. (is_selected and theme:bold(truncated) or truncated)
      lines[#lines + 1] = theme:fg("muted", "  Message " .. (i + 1) .. " of " .. #messages)
      lines[#lines + 1] = ""
    end
    if start_index > 0 or end_index < #messages then
      lines[#lines + 1] = theme:fg("muted", "  (" .. (self.selected + 1) .. "/" .. #messages .. ")")
    end
    return lines
  end

  function self:render(width)
    local lines = { "" } -- Spacer(1)
    append(lines, pi.tui.text_render(theme:bold("Fork from Message"), width, 1, 0))
    append(lines, pi.tui.text_render(theme:fg("muted",
      "Select a user message to copy the active path up to that point into a new session"),
      width, 1, 0))
    lines[#lines + 1] = ""
    lines[#lines + 1] = dynamic_border_line(theme, width)
    lines[#lines + 1] = ""
    append(lines, list_lines(width))
    lines[#lines + 1] = ""
    lines[#lines + 1] = dynamic_border_line(theme, width)
    return lines
  end

  return self
end

-- components/tree-selector.ts — TreeList + SearchLine + LabelInput as one
-- lines-producing component over the pi.session tree snapshot.
function tree_selector(opts)
  local theme = opts.theme
  local self = { focused = false }

  -- JS string prefix by UTF-16 units (slice(0, n)).
  local function js_unit_slice(text, max_units)
    if not utf8.len(text) then return text:sub(1, max_units) end
    local units, out = 0, {}
    for _, code in utf8.codes(text) do
      units = units + (code >= 0x10000 and 2 or 1)
      if units > max_units then break end
      out[#out + 1] = utf8.char(code)
    end
    return table.concat(out)
  end
  local function js_unit_length(text)
    if not utf8.len(text) then return #text end
    local units = 0
    for _, code in utf8.codes(text) do units = units + (code >= 0x10000 and 2 or 1) end
    return units
  end

  local function normalize(s)
    return trim((s:gsub("[\n\t]", " ")))
  end

  local function extract_content(content)
    local MAX_LEN = 200
    if type(content) == "string" then return js_unit_slice(content, MAX_LEN) end
    if type(content) == "table" then
      local result = ""
      for _, block in ipairs(content) do
        if type(block) == "table" and block.type == "text" then
          result = result .. (block.text or "")
          if js_unit_length(result) >= MAX_LEN then return js_unit_slice(result, MAX_LEN) end
        end
      end
      return result
    end
    return ""
  end

  local function has_text_content(content)
    if type(content) == "string" then return trim(content) ~= "" end
    if type(content) == "table" then
      for _, block in ipairs(content) do
        if type(block) == "table" and block.type == "text"
           and block.text and trim(block.text) ~= "" then
          return true
        end
      end
    end
    return false
  end

  -- TreeList state.
  local list = {
    flat_nodes = {},
    filtered = {},
    selected = 0, -- 0-based like the spec
    current_leaf_id = opts.current_leaf_id,
    max_visible = math.max(5, math.floor((opts.terminal_rows or 30) / 2)),
    filter_mode = opts.initial_filter_mode or "default",
    search_query = "",
    tool_call_map = {},
    multiple_roots = #opts.tree > 1,
    show_label_timestamps = false,
    active_path_ids = {},
    visible_parent = {},
    visible_children = {},
    last_selected_id = nil,
    folded = {},
  }

  local function flatten_tree(roots)
    local result = {}
    list.tool_call_map = {}

    -- Which subtrees contain the active leaf (active branch sorts first).
    local contains_active = {}
    local leaf_id = list.current_leaf_id
    do
      local all_nodes = {}
      -- Pre-order via explicit stack (the spec pops from the end).
      local pre_stack = {}
      for _, root in ipairs(roots) do pre_stack[#pre_stack + 1] = root end
      while #pre_stack > 0 do
        local node = table.remove(pre_stack)
        all_nodes[#all_nodes + 1] = node
        for i = #node.children, 1, -1 do pre_stack[#pre_stack + 1] = node.children[i] end
      end
      for i = #all_nodes, 1, -1 do
        local node = all_nodes[i]
        local has = leaf_id ~= nil and node.entry.id == leaf_id
        for _, child in ipairs(node.children) do
          if contains_active[child] then has = true end
        end
        contains_active[node] = has
      end
    end

    local multiple_roots = #roots > 1
    -- Stable partition (Array.prototype.sort is stable): the roots
    -- containing the active leaf move first, ties keep file order.
    local ordered_roots = {}
    for _, root in ipairs(roots) do
      if contains_active[root] then ordered_roots[#ordered_roots + 1] = root end
    end
    for _, root in ipairs(roots) do
      if not contains_active[root] then ordered_roots[#ordered_roots + 1] = root end
    end

    -- Stack items: {node, indent, just_branched, show_connector, is_last,
    -- gutters, is_virtual_root_child}.
    local stack = {}
    for i = #ordered_roots, 1, -1 do
      local is_last = i == #ordered_roots
      stack[#stack + 1] = { ordered_roots[i], multiple_roots and 1 or 0,
        multiple_roots, multiple_roots, is_last, {}, multiple_roots }
    end

    while #stack > 0 do
      local item = table.remove(stack)
      local node, indent, just_branched, show_connector, is_last, gutters, is_virtual_root_child =
        item[1], item[2], item[3], item[4], item[5], item[6], item[7]

      local entry = node.entry
      if entry.type == "message" and entry.message.role == "assistant"
         and type(entry.message.content) == "table" then
        for _, block in ipairs(entry.message.content) do
          if type(block) == "table" and block.type == "toolCall" then
            list.tool_call_map[block.id] = { name = block.name, arguments = block.arguments }
          end
        end
      end

      result[#result + 1] = { node = node, indent = indent,
        show_connector = show_connector, is_last = is_last, gutters = gutters,
        is_virtual_root_child = is_virtual_root_child }

      local children = node.children
      local multiple_children = #children > 1

      local prioritized, rest = {}, {}
      for _, child in ipairs(children) do
        if contains_active[child] then prioritized[#prioritized + 1] = child
        else rest[#rest + 1] = child end
      end
      local ordered_children = {}
      for _, child in ipairs(prioritized) do ordered_children[#ordered_children + 1] = child end
      for _, child in ipairs(rest) do ordered_children[#ordered_children + 1] = child end

      local child_indent
      if multiple_children then child_indent = indent + 1
      elseif just_branched and indent > 0 then child_indent = indent + 1
      else child_indent = indent end

      local connector_displayed = show_connector and not is_virtual_root_child
      local current_display_indent = list.multiple_roots and math.max(0, indent - 1) or indent
      local connector_position = math.max(0, current_display_indent - 1)
      local child_gutters = gutters
      if connector_displayed then
        child_gutters = {}
        for _, gutter in ipairs(gutters) do child_gutters[#child_gutters + 1] = gutter end
        child_gutters[#child_gutters + 1] = { position = connector_position, show = not is_last }
      end

      for i = #ordered_children, 1, -1 do
        local child_is_last = i == #ordered_children
        stack[#stack + 1] = { ordered_children[i], child_indent, multiple_children,
          multiple_children, child_is_last, child_gutters, false }
      end
    end

    return result
  end

  local function build_active_path()
    list.active_path_ids = {}
    if not list.current_leaf_id then return end
    local entry_map = {}
    for _, flat in ipairs(list.flat_nodes) do entry_map[flat.node.entry.id] = flat end
    local current = list.current_leaf_id
    while current do
      list.active_path_ids[current] = true
      local node = entry_map[current]
      if not node then break end
      current = node.node.entry.parentId
    end
  end

  local function find_nearest_visible_index(entry_id)
    if #list.filtered == 0 then return 0 end
    local entry_map = {}
    for _, flat in ipairs(list.flat_nodes) do entry_map[flat.node.entry.id] = flat end
    local visible_index = {}
    for i, flat in ipairs(list.filtered) do visible_index[flat.node.entry.id] = i - 1 end
    local current = entry_id
    while current ~= nil do
      local index = visible_index[current]
      if index ~= nil then return index end
      local node = entry_map[current]
      if not node then break end
      current = node.node.entry.parentId
    end
    return #list.filtered - 1
  end

  local function get_searchable_text(node)
    local entry = node.entry
    local parts = {}
    if node.label then parts[#parts + 1] = node.label end
    if entry.type == "message" then
      local msg = entry.message
      parts[#parts + 1] = msg.role
      if msg.content then parts[#parts + 1] = extract_content(msg.content) end
      if msg.role == "bashExecution" and msg.command then parts[#parts + 1] = msg.command end
    elseif entry.type == "custom_message" then
      parts[#parts + 1] = entry.customType
      if type(entry.content) == "string" then parts[#parts + 1] = entry.content
      else parts[#parts + 1] = extract_content(entry.content) end
    elseif entry.type == "compaction" then
      parts[#parts + 1] = "compaction"
    elseif entry.type == "branch_summary" then
      parts[#parts + 1] = "branch summary"
      parts[#parts + 1] = entry.summary
    elseif entry.type == "session_info" then
      parts[#parts + 1] = "title"
      if entry.name then parts[#parts + 1] = entry.name end
    elseif entry.type == "model_change" then
      parts[#parts + 1] = "model"
      parts[#parts + 1] = entry.modelId
    elseif entry.type == "thinking_level_change" then
      parts[#parts + 1] = "thinking"
      parts[#parts + 1] = entry.thinkingLevel
    elseif entry.type == "custom" then
      parts[#parts + 1] = "custom"
      parts[#parts + 1] = entry.customType
    elseif entry.type == "label" then
      parts[#parts + 1] = "label"
      parts[#parts + 1] = entry.label or ""
    end
    return table.concat(parts, " ")
  end

  local function recalculate_visual_structure()
    if #list.filtered == 0 then return end

    local visible_ids = {}
    for _, flat in ipairs(list.filtered) do visible_ids[flat.node.entry.id] = true end

    local entry_map = {}
    for _, flat in ipairs(list.flat_nodes) do entry_map[flat.node.entry.id] = flat end

    local function find_visible_ancestor(node_id)
      local flat = entry_map[node_id]
      local current = flat and flat.node.entry.parentId or nil
      while current ~= nil do
        if visible_ids[current] then return current end
        local parent = entry_map[current]
        current = parent and parent.node.entry.parentId or nil
      end
      return nil
    end

    -- visible_children keyed by parent id; false stands in for the null
    -- root key.
    local visible_parent, visible_children = {}, {}
    visible_children[false] = {}
    for _, flat in ipairs(list.filtered) do
      local node_id = flat.node.entry.id
      local ancestor = find_visible_ancestor(node_id)
      visible_parent[node_id] = ancestor
      -- `false` stands in for the null root key (a plain and/or would
      -- collapse it back to nil).
      local key = ancestor
      if key == nil then key = false end
      if not visible_children[key] then visible_children[key] = {} end
      local bucket = visible_children[key]
      bucket[#bucket + 1] = node_id
    end

    local visible_root_ids = visible_children[false]
    list.multiple_roots = #visible_root_ids > 1

    local filtered_map = {}
    for _, flat in ipairs(list.filtered) do filtered_map[flat.node.entry.id] = flat end

    local stack = {}
    for i = #visible_root_ids, 1, -1 do
      local is_last = i == #visible_root_ids
      stack[#stack + 1] = { visible_root_ids[i], list.multiple_roots and 1 or 0,
        list.multiple_roots, list.multiple_roots, is_last, {}, list.multiple_roots }
    end

    while #stack > 0 do
      local item = table.remove(stack)
      local node_id, indent, just_branched, show_connector, is_last, gutters, is_virtual_root_child =
        item[1], item[2], item[3], item[4], item[5], item[6], item[7]
      local flat = filtered_map[node_id]
      if flat then
        flat.indent = indent
        flat.show_connector = show_connector
        flat.is_last = is_last
        flat.gutters = gutters
        flat.is_virtual_root_child = is_virtual_root_child

        local children = visible_children[node_id] or {}
        local multiple_children = #children > 1

        local child_indent
        if multiple_children then child_indent = indent + 1
        elseif just_branched and indent > 0 then child_indent = indent + 1
        else child_indent = indent end

        local connector_displayed = show_connector and not is_virtual_root_child
        local current_display_indent = list.multiple_roots and math.max(0, indent - 1) or indent
        local connector_position = math.max(0, current_display_indent - 1)
        local child_gutters = gutters
        if connector_displayed then
          child_gutters = {}
          for _, gutter in ipairs(gutters) do child_gutters[#child_gutters + 1] = gutter end
          child_gutters[#child_gutters + 1] = { position = connector_position, show = not is_last }
        end

        for i = #children, 1, -1 do
          local child_is_last = i == #children
          stack[#stack + 1] = { children[i], child_indent, multiple_children,
            multiple_children, child_is_last, child_gutters, false }
        end
      end
    end

    list.visible_parent = visible_parent
    list.visible_children = visible_children
  end

  local function apply_filter()
    if #list.filtered > 0 then
      local selected = list.filtered[list.selected + 1]
      list.last_selected_id = selected and selected.node.entry.id or list.last_selected_id
    end

    local search_tokens = {}
    for token in list.search_query:lower():gmatch("%S+") do
      search_tokens[#search_tokens + 1] = token
    end

    list.filtered = {}
    for _, flat in ipairs(list.flat_nodes) do
      local entry = flat.node.entry
      local is_current_leaf = entry.id == list.current_leaf_id
      local keep = true

      -- Skip assistant messages with only tool calls (no text) unless
      -- error/aborted; the current leaf always shows.
      if entry.type == "message" and entry.message.role == "assistant" and not is_current_leaf then
        local msg = entry.message
        local has_text = has_text_content(msg.content)
        local is_error_or_aborted = msg.stopReason and msg.stopReason ~= "stop"
          and msg.stopReason ~= "toolUse"
        if not has_text and not is_error_or_aborted then keep = false end
      end

      if keep then
        local is_settings_entry = entry.type == "label" or entry.type == "custom"
          or entry.type == "model_change" or entry.type == "thinking_level_change"
          or entry.type == "session_info"
        local passes
        if list.filter_mode == "user-only" then
          passes = entry.type == "message" and entry.message.role == "user"
        elseif list.filter_mode == "no-tools" then
          passes = not is_settings_entry
            and not (entry.type == "message" and entry.message.role == "toolResult")
        elseif list.filter_mode == "labeled-only" then
          passes = flat.node.label ~= nil
        elseif list.filter_mode == "all" then
          passes = true
        else
          passes = not is_settings_entry
        end
        keep = passes
      end

      if keep and #search_tokens > 0 then
        local node_text = get_searchable_text(flat.node):lower()
        for _, token in ipairs(search_tokens) do
          if not node_text:find(token, 1, true) then keep = false end
        end
      end

      if keep then list.filtered[#list.filtered + 1] = flat end
    end

    -- Drop descendants of folded nodes.
    if next(list.folded) ~= nil then
      local skip = {}
      for _, flat in ipairs(list.flat_nodes) do
        local id, parent_id = flat.node.entry.id, flat.node.entry.parentId
        if parent_id ~= nil and (list.folded[parent_id] or skip[parent_id]) then
          skip[id] = true
        end
      end
      local kept = {}
      for _, flat in ipairs(list.filtered) do
        if not skip[flat.node.entry.id] then kept[#kept + 1] = flat end
      end
      list.filtered = kept
    end

    recalculate_visual_structure()

    if list.last_selected_id then
      list.selected = find_nearest_visible_index(list.last_selected_id)
    elseif list.selected >= #list.filtered then
      list.selected = math.max(0, #list.filtered - 1)
    end

    if #list.filtered > 0 then
      local selected = list.filtered[list.selected + 1]
      list.last_selected_id = selected and selected.node.entry.id or list.last_selected_id
    end
  end

  list.flat_nodes = flatten_tree(opts.tree)
  build_active_path()
  apply_filter()
  local target_id = opts.initial_selected_id or list.current_leaf_id
  list.selected = find_nearest_visible_index(target_id)
  do
    local selected = list.filtered[list.selected + 1]
    list.last_selected_id = selected and selected.node.entry.id or nil
  end

  -- formatToolCall.
  local function shorten_path(path)
    local home = opts.home or ""
    if home ~= "" and path:sub(1, #home) == home then return "~" .. path:sub(#home + 1) end
    return path
  end

  -- JS `${number}`: integral doubles print without a fraction.
  local function js_num(value)
    if type(value) == "number" and math.type(value) == "float" and value % 1 == 0
       and value == value and value ~= math.huge and value ~= -math.huge then
      return ("%d"):format(value)
    end
    return tostring(value)
  end

  local function js_str(value)
    -- String(args.x || args.y || "") — JS falsy fallbacks.
    if value == nil or value == false or value == "" or value == 0 then return nil end
    if type(value) == "number" then return js_num(value) end
    return tostring(value)
  end

  local function format_tool_call(name, args)
    args = args or {}
    if name == "read" then
      local path = shorten_path(js_str(args.path) or js_str(args.file_path) or "")
      local display = path
      if args.offset ~= nil or args.limit ~= nil then
        local start = args.offset ~= nil and args.offset or 1
        local finish = args.limit ~= nil and js_num(start + args.limit - 1) or ""
        display = display .. ":" .. js_num(start)
          .. (finish ~= "" and ("-" .. finish) or "")
      end
      return "[read: " .. display .. "]"
    elseif name == "write" then
      return "[write: " .. shorten_path(js_str(args.path) or js_str(args.file_path) or "") .. "]"
    elseif name == "edit" then
      return "[edit: " .. shorten_path(js_str(args.path) or js_str(args.file_path) or "") .. "]"
    elseif name == "bash" then
      local raw = js_str(args.command) or ""
      local cmd = js_unit_slice(trim((raw:gsub("[\n\t]", " "))), 50)
      return "[bash: " .. cmd .. (js_unit_length(raw) > 50 and "..." or "") .. "]"
    elseif name == "grep" then
      return "[grep: /" .. (js_str(args.pattern) or "") .. "/ in "
        .. shorten_path(js_str(args.path) or ".") .. "]"
    elseif name == "find" then
      return "[find: " .. (js_str(args.pattern) or "") .. " in "
        .. shorten_path(js_str(args.path) or ".") .. "]"
    elseif name == "ls" then
      return "[ls: " .. shorten_path(js_str(args.path) or ".") .. "]"
    end
    local args_json = pi.json.encode(args)
    return "[" .. name .. ": " .. js_unit_slice(args_json, 40)
      .. (js_unit_length(args_json) > 40 and "..." or "") .. "]"
  end

  local function get_entry_display_text(node, is_selected)
    local entry = node.entry
    local result
    if entry.type == "message" then
      local msg = entry.message
      local role = msg.role
      if role == "user" then
        result = theme:fg("accent", "user: ") .. normalize(extract_content(msg.content))
      elseif role == "assistant" then
        local text_content = normalize(extract_content(msg.content))
        if text_content ~= "" then
          result = theme:fg("success", "assistant: ") .. text_content
        elseif msg.stopReason == "aborted" then
          result = theme:fg("success", "assistant: ") .. theme:fg("muted", "(aborted)")
        elseif msg.errorMessage then
          local err = js_unit_slice(normalize(msg.errorMessage), 80)
          result = theme:fg("success", "assistant: ") .. theme:fg("error", err)
        else
          result = theme:fg("success", "assistant: ") .. theme:fg("muted", "(no content)")
        end
      elseif role == "toolResult" then
        local tool_call = msg.toolCallId and list.tool_call_map[msg.toolCallId] or nil
        if tool_call then
          result = theme:fg("muted", format_tool_call(tool_call.name, tool_call.arguments))
        else
          result = theme:fg("muted", "[" .. (msg.toolName or "tool") .. "]")
        end
      elseif role == "bashExecution" then
        result = theme:fg("dim", "[bash]: " .. normalize(msg.command or ""))
      else
        result = theme:fg("dim", "[" .. role .. "]")
      end
    elseif entry.type == "custom_message" then
      local content
      if type(entry.content) == "string" then content = entry.content
      else
        local texts = {}
        for _, block in ipairs(entry.content or {}) do
          if type(block) == "table" and block.type == "text" then texts[#texts + 1] = block.text end
        end
        content = table.concat(texts)
      end
      result = theme:fg("customMessageLabel", "[" .. entry.customType .. "]: ") .. normalize(content)
    elseif entry.type == "compaction" then
      -- JS Math.round: half away from zero for positives.
      local tokens = math.floor((entry.tokensBefore or 0) / 1000 + 0.5)
      result = theme:fg("borderAccent", "[compaction: " .. tokens .. "k tokens]")
    elseif entry.type == "branch_summary" then
      result = theme:fg("warning", "[branch summary]: ") .. normalize(entry.summary or "")
    elseif entry.type == "model_change" then
      result = theme:fg("dim", "[model: " .. entry.modelId .. "]")
    elseif entry.type == "thinking_level_change" then
      result = theme:fg("dim", "[thinking: " .. entry.thinkingLevel .. "]")
    elseif entry.type == "custom" then
      result = theme:fg("dim", "[custom: " .. entry.customType .. "]")
    elseif entry.type == "label" then
      result = theme:fg("dim", "[label: " .. (entry.label or "(cleared)") .. "]")
    elseif entry.type == "session_info" then
      if entry.name then
        result = theme:fg("dim", "[title: ") .. theme:fg("dim", entry.name) .. theme:fg("dim", "]")
      else
        result = theme:fg("dim", "[title: ") .. theme:italic(theme:fg("dim", "empty"))
          .. theme:fg("dim", "]")
      end
    else
      result = ""
    end
    return is_selected and theme:bold(result) or result
  end

  local function format_label_timestamp(timestamp)
    local ms = pi.session.parse_iso_ms(timestamp)
    if not ms then return timestamp end
    local date = os.date("*t", math.floor(ms / 1000))
    local now = os.date("*t", math.floor(opts.now_ms() / 1000))
    local time = ("%02d:%02d"):format(date.hour, date.min)
    if date.year == now.year and date.month == now.month and date.day == now.day then
      return time
    end
    if date.year == now.year then
      return date.month .. "/" .. date.day .. " " .. time
    end
    return tostring(date.year):sub(-2) .. "/" .. date.month .. "/" .. date.day .. " " .. time
  end

  local function get_status_labels()
    local labels = ""
    if list.filter_mode == "no-tools" then labels = labels .. " [no-tools]"
    elseif list.filter_mode == "user-only" then labels = labels .. " [user]"
    elseif list.filter_mode == "labeled-only" then labels = labels .. " [labeled]"
    elseif list.filter_mode == "all" then labels = labels .. " [all]" end
    if list.show_label_timestamps then labels = labels .. " [+label time]" end
    return labels
  end

  local function is_foldable(entry_id)
    local children = list.visible_children[entry_id]
    if not children or #children == 0 then return false end
    local parent_id = list.visible_parent[entry_id]
    if parent_id == nil then return true end
    local siblings = list.visible_children[parent_id]
    return siblings ~= nil and #siblings > 1
  end

  local function find_branch_segment_start(direction)
    local selected = list.filtered[list.selected + 1]
    local selected_id = selected and selected.node.entry.id or nil
    if not selected_id then return list.selected end
    local index_by_id = {}
    for i, flat in ipairs(list.filtered) do index_by_id[flat.node.entry.id] = i - 1 end
    local current = selected_id
    if direction == "down" then
      while true do
        local children = list.visible_children[current] or {}
        if #children == 0 then return index_by_id[current] end
        if #children > 1 then return index_by_id[children[1]] end
        current = children[1]
      end
    end
    while true do
      local parent_id = list.visible_parent[current]
      if parent_id == nil then return index_by_id[current] end
      local children = list.visible_children[parent_id] or {}
      if #children > 1 then
        local segment_start = index_by_id[current]
        if segment_start < list.selected then return segment_start end
      end
      current = parent_id
    end
  end

  local function tree_lines(width)
    local lines = {}
    if #list.filtered == 0 then
      lines[#lines + 1] = pi.tui.truncate(theme:fg("muted", "  No entries found"), width)
      lines[#lines + 1] = pi.tui.truncate(
        theme:fg("muted", "  (0/0)" .. get_status_labels()), width)
      return lines
    end

    local start_index = math.max(0, math.min(
      list.selected - math.floor(list.max_visible / 2), #list.filtered - list.max_visible))
    local end_index = math.min(start_index + list.max_visible, #list.filtered)

    for i = start_index, end_index - 1 do
      local flat = list.filtered[i + 1]
      local entry = flat.node.entry
      local is_selected = i == list.selected

      local cursor = is_selected and theme:fg("accent", "› ") or "  "
      local display_indent = list.multiple_roots and math.max(0, flat.indent - 1) or flat.indent
      local connector = ""
      if flat.show_connector and not flat.is_virtual_root_child then
        connector = flat.is_last and "└─ " or "├─ "
      end
      local connector_position = connector ~= "" and display_indent - 1 or -1

      local total_chars = display_indent * 3
      local prefix_chars = {}
      local is_folded = list.folded[entry.id] or false
      for pos = 0, total_chars - 1 do
        local level = math.floor(pos / 3)
        local pos_in_level = pos % 3
        local gutter = nil
        for _, g in ipairs(flat.gutters) do
          if g.position == level then gutter = g break end
        end
        if gutter then
          prefix_chars[#prefix_chars + 1] = pos_in_level == 0 and (gutter.show and "│" or " ") or " "
        elseif connector ~= "" and level == connector_position then
          if pos_in_level == 0 then
            prefix_chars[#prefix_chars + 1] = flat.is_last and "└" or "├"
          elseif pos_in_level == 1 then
            local foldable = is_foldable(entry.id)
            prefix_chars[#prefix_chars + 1] = is_folded and "⊞" or (foldable and "⊟" or "─")
          else
            prefix_chars[#prefix_chars + 1] = " "
          end
        else
          prefix_chars[#prefix_chars + 1] = " "
        end
      end
      local prefix = table.concat(prefix_chars)

      local shows_fold_in_connector = flat.show_connector and not flat.is_virtual_root_child
      local fold_marker = (is_folded and not shows_fold_in_connector)
        and theme:fg("accent", "⊞ ") or ""
      local path_marker = list.active_path_ids[entry.id] and theme:fg("accent", "• ") or ""
      local label = flat.node.label
        and theme:fg("warning", "[" .. flat.node.label .. "] ") or ""
      local label_timestamp = ""
      if list.show_label_timestamps and flat.node.label and flat.node.labelTimestamp then
        label_timestamp = theme:fg("muted", format_label_timestamp(flat.node.labelTimestamp) .. " ")
      end
      local content = get_entry_display_text(flat.node, is_selected)

      local line = cursor .. theme:fg("dim", prefix) .. fold_marker .. path_marker
        .. label .. label_timestamp .. content
      if is_selected then line = theme:bg("selectedBg", line) end
      lines[#lines + 1] = pi.tui.truncate(line, width)
    end

    lines[#lines + 1] = pi.tui.truncate(theme:fg("muted",
      "  (" .. (list.selected + 1) .. "/" .. #list.filtered .. ")" .. get_status_labels()), width)
    return lines
  end

  -- Label editing (LabelInput).
  local label_input = nil

  local function iso_from_ms(ms)
    local seconds = math.floor(ms / 1000)
    return os.date("!%Y-%m-%dT%H:%M:%S", seconds) .. (".%03dZ"):format(ms % 1000)
  end

  local function update_node_label(entry_id, label)
    for _, flat in ipairs(list.flat_nodes) do
      if flat.node.entry.id == entry_id then
        flat.node.label = label
        flat.node.labelTimestamp = label and iso_from_ms(opts.now_ms()) or nil
        break
      end
    end
  end

  local function hide_label_input()
    label_input = nil
  end

  local function show_label_input(entry_id, current_label)
    label_input = { entry_id = entry_id, input = pi.tui.input() }
    if current_label then label_input.input:set_value(current_label) end
    label_input.input:set_focused(self.focused)
  end

  local function handle_label_input(data)
    if binding_matches(data, SELECT_KEYS.confirm) then
      local value = trim(label_input.input:value())
      local entry_id = label_input.entry_id
      local label = value ~= "" and value or nil
      update_node_label(entry_id, label)
      if opts.on_label_change then opts.on_label_change(entry_id, label) end
      hide_label_input()
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      hide_label_input()
    else
      label_input.input:input(data)
    end
  end

  local function label_input_lines(width)
    local lines = {}
    local available = width - 2
    lines[#lines + 1] = pi.tui.truncate("  " .. theme:fg("muted", "Label (empty to remove):"), width)
    for _, line in ipairs(label_input.input:render(available)) do
      lines[#lines + 1] = pi.tui.truncate("  " .. line, width)
    end
    lines[#lines + 1] = pi.tui.truncate("  " .. select_key_hint(theme, "confirm", "save")
      .. "  " .. select_key_hint(theme, "cancel", "cancel"), width)
    return lines
  end

  local function handle_list_input(data)
    if binding_matches(data, SELECT_KEYS.up) then
      list.selected = list.selected == 0 and #list.filtered - 1 or list.selected - 1
    elseif binding_matches(data, SELECT_KEYS.down) then
      list.selected = list.selected == #list.filtered - 1 and 0 or list.selected + 1
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.foldOrUp"]) then
      local selected = list.filtered[list.selected + 1]
      local current_id = selected and selected.node.entry.id or nil
      if current_id and is_foldable(current_id) and not list.folded[current_id] then
        list.folded[current_id] = true
        apply_filter()
      else
        list.selected = find_branch_segment_start("up")
      end
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.unfoldOrDown"]) then
      local selected = list.filtered[list.selected + 1]
      local current_id = selected and selected.node.entry.id or nil
      if current_id and list.folded[current_id] then
        list.folded[current_id] = nil
        apply_filter()
      else
        list.selected = find_branch_segment_start("down")
      end
    elseif binding_matches(data, DEFAULT_KEYS["tui.editor.cursorLeft"])
        or binding_matches(data, SELECT_KEYS.pageUp) then
      list.selected = math.max(0, list.selected - list.max_visible)
    elseif binding_matches(data, DEFAULT_KEYS["tui.editor.cursorRight"])
        or binding_matches(data, SELECT_KEYS.pageDown) then
      list.selected = math.min(#list.filtered - 1, list.selected + list.max_visible)
    elseif binding_matches(data, SELECT_KEYS.confirm) then
      local selected = list.filtered[list.selected + 1]
      if selected then opts.on_select(selected.node.entry.id) end
    elseif binding_matches(data, SELECT_KEYS.cancel) then
      if list.search_query ~= "" then
        list.search_query = ""
        list.folded = {}
        apply_filter()
      else
        opts.on_cancel()
      end
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.filter.default"]) then
      list.filter_mode = "default"
      list.folded = {}
      apply_filter()
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.filter.noTools"]) then
      list.filter_mode = list.filter_mode == "no-tools" and "default" or "no-tools"
      list.folded = {}
      apply_filter()
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.filter.userOnly"]) then
      list.filter_mode = list.filter_mode == "user-only" and "default" or "user-only"
      list.folded = {}
      apply_filter()
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.filter.labeledOnly"]) then
      list.filter_mode = list.filter_mode == "labeled-only" and "default" or "labeled-only"
      list.folded = {}
      apply_filter()
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.filter.all"]) then
      list.filter_mode = list.filter_mode == "all" and "default" or "all"
      list.folded = {}
      apply_filter()
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.filter.cycleBackward"]) then
      local modes = { "default", "no-tools", "user-only", "labeled-only", "all" }
      local current = 1
      for i, mode in ipairs(modes) do if mode == list.filter_mode then current = i end end
      list.filter_mode = modes[(current - 2) % #modes + 1]
      list.folded = {}
      apply_filter()
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.filter.cycleForward"]) then
      local modes = { "default", "no-tools", "user-only", "labeled-only", "all" }
      local current = 1
      for i, mode in ipairs(modes) do if mode == list.filter_mode then current = i end end
      list.filter_mode = modes[current % #modes + 1]
      list.folded = {}
      apply_filter()
    elseif binding_matches(data, DEFAULT_KEYS["tui.editor.deleteCharBackward"]) then
      if #list.search_query > 0 then
        list.search_query = list.search_query:sub(1, -2)
        list.folded = {}
        apply_filter()
      end
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.editLabel"]) then
      local selected = list.filtered[list.selected + 1]
      if selected then
        show_label_input(selected.node.entry.id, selected.node.label)
      end
    elseif binding_matches(data, DEFAULT_KEYS["app.tree.toggleLabelTimestamp"]) then
      list.show_label_timestamps = not list.show_label_timestamps
    else
      local has_control_chars = false
      if data == "" then has_control_chars = true end
      local iterable = utf8.len(data) and utf8.codes or nil
      if iterable then
        for _, code in utf8.codes(data) do
          if code < 32 or code == 0x7f or (code >= 0x80 and code <= 0x9f) then
            has_control_chars = true
          end
        end
      else
        has_control_chars = true
      end
      if not has_control_chars then
        list.search_query = list.search_query .. data
        list.folded = {}
        apply_filter()
      end
    end
  end

  function self:set_focused(focused)
    self.focused = focused
    if label_input then label_input.input:set_focused(focused) end
  end

  function self:handle_input(data)
    if label_input then handle_label_input(data) else handle_list_input(data) end
  end

  function self:render(width)
    local lines = { "" } -- Spacer(1)
    lines[#lines + 1] = dynamic_border_line(theme, width)
    append(lines, pi.tui.text_render(theme:bold("  Session Tree"), width, 1, 0))
    local filter_keys = table.concat({
      app_key_text("app.tree.filter.default"), app_key_text("app.tree.filter.noTools"),
      app_key_text("app.tree.filter.userOnly"), app_key_text("app.tree.filter.labeledOnly"),
      app_key_text("app.tree.filter.all"),
    }, "/")
    local cycle_keys = app_key_text("app.tree.filter.cycleForward") .. "/"
      .. app_key_text("app.tree.filter.cycleBackward")
    local branch_keys = app_key_text("app.tree.foldOrUp") .. "/"
      .. app_key_text("app.tree.unfoldOrDown")
    lines[#lines + 1] = pi.tui.truncate(theme:fg("muted",
      "  ↑/↓: move. ←/→: page. " .. branch_keys .. ": fold/branch. "
        .. app_key_text("app.tree.editLabel") .. ": label. " .. filter_keys
        .. ": filters (" .. cycle_keys .. " cycle). "
        .. app_key_text("app.tree.toggleLabelTimestamp") .. ": label time"), width)
    if list.search_query ~= "" then
      lines[#lines + 1] = pi.tui.truncate("  " .. theme:fg("muted", "Type to search:")
        .. " " .. theme:fg("accent", list.search_query), width)
    else
      lines[#lines + 1] = pi.tui.truncate("  " .. theme:fg("muted", "Type to search:"), width)
    end
    lines[#lines + 1] = dynamic_border_line(theme, width)
    lines[#lines + 1] = "" -- Spacer(1)
    if label_input then
      append(lines, label_input_lines(width))
    else
      append(lines, tree_lines(width))
    end
    lines[#lines + 1] = "" -- Spacer(1)
    lines[#lines + 1] = dynamic_border_line(theme, width)
    return lines
  end

  return self
end

-- agent-session.ts _extractUserMessageText.
function extract_user_message_text(content)
  if type(content) == "string" then return content end
  if type(content) == "table" then
    local texts = {}
    for _, block in ipairs(content) do
      if type(block) == "table" and block.type == "text" then
        texts[#texts + 1] = block.text or ""
      end
    end
    return table.concat(texts)
  end
  return ""
end

-- agent-session.ts getUserMessagesForForking.
function get_user_messages_for_forking(state)
  local result = {}
  for _, entry in ipairs(state.session_manager:get_entries()) do
    if entry.type == "message" and entry.message.role == "user" then
      local text = extract_user_message_text(entry.message.content)
      if text ~= "" then
        result[#result + 1] = { entryId = entry.id, text = text }
      end
    end
  end
  return result
end

-- agent-session-runtime.ts fork: fork-to-root creates a fresh session
-- with the current file as parent; otherwise the branched-session path
-- copies root→target into a new file. The session_before_fork event and
-- teardownCurrent's session_shutdown join item 9.
function fork_session_runtime(state, entry_id, options)
  options = options or {}
  local position = options.position or "before"
  local before = EXTENSION_POLICY.emit_generic({ type = "session_before_fork",
    entryId = entry_id, position = position }, EXTENSION_CONTEXT_POLICY.snapshot(state))
  if before and before.cancel then return { cancelled = true } end
  local sm = state.session_manager
  local selected_entry = sm:get_entry(entry_id)
  if not selected_entry then error("Invalid entry ID for forking", 0) end

  local target_leaf_id, selected_text
  if position == "at" then
    target_leaf_id = selected_entry.id
  else
    if selected_entry.type ~= "message" or selected_entry.message.role ~= "user" then
      error("Invalid entry ID for forking", 0)
    end
    target_leaf_id = selected_entry.parentId
    selected_text = extract_user_message_text(selected_entry.message.content)
  end

  local previous_file = sm:get_session_file()
  local manager = sm
  if sm:is_persisted() then
    local current_file = sm:get_session_file()
    if not current_file then error("Persisted session is missing a session file", 0) end
    local session_dir = sm:get_session_dir()
    if not target_leaf_id then
      manager = pi.session.create({
        cwd = state.cwd, sessionDir = session_dir, agentDir = state.request.agentDir,
      })
      manager:new_session({ parentSession = current_file })
    else
      manager = pi.session.open({
        path = current_file, sessionDir = session_dir, agentDir = state.request.agentDir,
      })
      local forked_path = manager:create_branched_session(target_leaf_id)
      if not forked_path then error("Failed to create forked session", 0) end
    end
  elseif not target_leaf_id then
    sm:new_session({ parentSession = sm:get_session_file() })
  else
    sm:create_branched_session(target_leaf_id)
  end

  EXTENSION_POLICY.emit_generic({ type = "session_shutdown", reason = "fork",
    targetSessionFile = manager:get_session_file() }, EXTENSION_CONTEXT_POLICY.snapshot(state))
  state.extension_shutdown_emitted = true
  state.agent:abort()
  state.next_session_start_event = { type = "session_start", reason = "fork",
    previousSessionFile = previous_file }
  bind_session_runtime(state, manager)
  return { cancelled = false, selectedText = selected_text }
end

-- agent-session.ts navigateTree over the pi.session branching mechanism
-- and the branch-summarization port. Runs on a background coroutine when
-- summarizing (pi.ai.stream_simple awaits).
function navigate_session_tree(state, target_id, options)
  options = options or {}
  local sm = state.session_manager
  local old_leaf_id = sm:get_leaf_id()
  if target_id == old_leaf_id then return { cancelled = false } end
  if options.summarize and not state.model then
    error("No model available for summarization", 0)
  end
  local target_entry = sm:get_entry(target_id)
  if not target_entry then error("Entry " .. target_id .. " not found", 0) end

  local entries_to_summarize =
    branch_summary_lib.collect_entries_for_branch_summary(sm, old_leaf_id, target_id)
  local signal = options.signal or pi.abort_signal()
  local preparation = {
    targetId = target_id, oldLeafId = old_leaf_id, commonAncestorId = nil,
    entriesToSummarize = entries_to_summarize,
    userWantsSummary = options.summarize == true,
    customInstructions = options.customInstructions,
    replaceInstructions = options.replaceInstructions, label = options.label,
  }
  local before = EXTENSION_POLICY.emit_generic({ type = "session_before_tree",
    preparation = preparation, signal = signal },
    EXTENSION_CONTEXT_POLICY.snapshot(state, { signal = signal }))
  if before and before.cancel then return { cancelled = true } end
  if before then
    if before.customInstructions ~= nil then options.customInstructions = before.customInstructions end
    if before.replaceInstructions ~= nil then options.replaceInstructions = before.replaceInstructions end
    if before.label ~= nil then options.label = before.label end
  end

  local summary_text, summary_details
  local from_extension = false
  if before and before.summary and options.summarize then
    summary_text, summary_details = before.summary.summary, before.summary.details
    from_extension = true
  elseif options.summarize and #entries_to_summarize > 0 then
    local settings = pi.settings.branch_summary()
    local api_key = pi.auth.get_api_key(state.model.provider)
    local result = branch_summary_lib.generate_branch_summary(entries_to_summarize, {
      model = state.model, apiKey = api_key, signal = signal,
      customInstructions = options.customInstructions,
      replaceInstructions = options.replaceInstructions,
      reserveTokens = settings.reserveTokens, now_ms = state.wall_now_ms,
    })
    if result.aborted then return { cancelled = true, aborted = true } end
    if result.error then error(result.error, 0) end
    summary_text = result.summary
    summary_details = {
      readFiles = result.readFiles or {}, modifiedFiles = result.modifiedFiles or {},
    }
  end

  local new_leaf_id, editor_text
  if target_entry.type == "message" and target_entry.message.role == "user" then
    new_leaf_id = target_entry.parentId
    editor_text = extract_user_message_text(target_entry.message.content)
  elseif target_entry.type == "custom_message" then
    new_leaf_id = target_entry.parentId
    if type(target_entry.content) == "string" then
      editor_text = target_entry.content
    else
      local texts = {}
      for _, block in ipairs(target_entry.content or {}) do
        if type(block) == "table" and block.type == "text" then texts[#texts + 1] = block.text end
      end
      editor_text = table.concat(texts)
    end
  else
    new_leaf_id = target_id
  end

  local summary_entry
  if summary_text then
    local id = sm:branch_with_summary(new_leaf_id, summary_text, summary_details, from_extension)
    summary_entry = sm:get_entry(id)
    if options.label then sm:append_label_change(id, options.label) end
  elseif new_leaf_id == nil then
    sm:reset_leaf()
  else
    sm:branch(new_leaf_id)
  end
  if options.label and not summary_text then sm:append_label_change(target_id, options.label) end

  state.agent:set_messages(sm:build_session_context().messages)
  state.session_context = nil
  local tree_event = { type = "session_tree",
    newLeafId = sm:get_leaf_id(), oldLeafId = old_leaf_id,
    summaryEntry = summary_entry,
  }
  if summary_text then tree_event.fromExtension = from_extension end
  EXTENSION_POLICY.emit_generic(tree_event, EXTENSION_CONTEXT_POLICY.snapshot(state))
  return { editorText = editor_text, cancelled = false, summaryEntry = summary_entry }
end

-- interactive-mode.ts showUserMessageSelector.
function show_user_message_selector(state)
  local user_messages = get_user_messages_for_forking(state)
  if #user_messages == 0 then
    show_status(state, "No messages to fork from")
    return
  end
  local initial_selected_id = user_messages[#user_messages].entryId
  show_selector(state, function(done)
    local rows = {}
    for _, message in ipairs(user_messages) do
      rows[#rows + 1] = { id = message.entryId, text = message.text }
    end
    return user_message_selector({
      theme = state.theme,
      messages = rows,
      initial_selected_id = initial_selected_id,
      on_select = function(entry_id)
        local ok, result = pcall(fork_session_runtime, state, entry_id)
        if not ok then
          done()
          show_error(state, tostring(result))
          return
        end
        if result.cancelled then
          done()
          return
        end
        render_current_session_state(state)
        state.editor.editor:set_text(result.selectedText or "")
        done()
        show_status(state, "Forked to new session")
      end,
      on_cancel = function() done() end,
    })
  end)
end

-- interactive-mode.ts handleCloneCommand.
function handle_clone_command(state)
  local leaf_id = state.session_manager:get_leaf_id()
  if not leaf_id then
    show_status(state, "Nothing to clone yet")
    return
  end
  local ok, result = pcall(fork_session_runtime, state, leaf_id, { position = "at" })
  if not ok then
    show_error(state, tostring(result))
    return
  end
  if result.cancelled then return end
  render_current_session_state(state)
  state.editor.editor:set_text("")
  show_status(state, "Cloned to new session")
end

-- interactive-mode.ts showTreeSelector's onSelect continuation — the
-- summarize-choice loop, the summarizing loader with the escape-abort
-- override, and the navigation result handling.
function tree_navigate_with_options(state, entry_id, wants_summary, custom_instructions)
  local signal = nil
  if wants_summary then
    signal = pi.abort_signal()
    -- isCompacting counts branch summarization: submissions queue while
    -- the summary streams (agent-session.ts _branchSummaryAbortController).
    state.branch_summary_signal = signal
    state.escape_override = function() signal:abort() end
    state.transcript[#state.transcript + 1] = { kind = "spacer" }
    local message = "Summarizing branch... (" .. app_key_text("app.interrupt") .. " to cancel)"
    state.loader = { loader = pi.tui.loader(message), message = message,
      last_ms = pi.monotonic_ms() }
  end

  local function finish()
    if wants_summary then
      state.loader = nil
      state.escape_override = nil
      state.branch_summary_signal = nil
    end
  end

  state.turn = pi.spawn(function()
    local ok, result = pcall(navigate_session_tree, state, entry_id, {
      summarize = wants_summary,
      customInstructions = custom_instructions,
      signal = signal,
    })
    finish()
    if not ok then
      show_error(state, tostring(result))
      return
    end
    if result.aborted then
      show_status(state, "Branch summarization cancelled")
      show_tree_selector(state, entry_id)
      return
    end
    if result.cancelled then
      show_status(state, "Navigation cancelled")
      return
    end
    -- chatContainer.clear() + renderInitialMessages().
    state.transcript = {}
    state.last_status = nil
    render_initial_messages(state)
    if result.editorText and trim(state.editor.editor:get_text()) == "" then
      state.editor.editor:set_text(result.editorText)
    end
    show_status(state, "Navigated to selected point")
    flush_compaction_queue(state, { willRetry = false })
  end)
end

function tree_choose_summary(state, entry_id)
  -- Loop until the user makes a complete choice or escapes to the tree.
  show_extension_choice(state, "Summarize branch?",
    { "No summary", "Summarize", "Summarize with custom prompt" },
    function(choice)
      if choice == nil then
        -- Escape re-shows the tree selector with the same selection.
        show_tree_selector(state, entry_id)
        return
      end
      local wants_summary = choice ~= "No summary"
      if choice == "Summarize with custom prompt" then
        show_extension_editor(state, "Custom summarization instructions", nil,
          function(instructions)
            if instructions == nil then
              -- Cancel loops back to the summary selector.
              tree_choose_summary(state, entry_id)
              return
            end
            tree_navigate_with_options(state, entry_id, wants_summary, instructions)
          end)
        return
      end
      tree_navigate_with_options(state, entry_id, wants_summary, nil)
    end)
end

-- interactive-mode.ts showTreeSelector.
function show_tree_selector(state, initial_selected_id)
  local tree = state.session_manager:get_tree()
  local real_leaf_id = state.session_manager:get_leaf_id()
  local initial_filter_mode = (state.request and state.request.treeFilterMode)
    or pi.settings.tree_filter_mode()
  if #tree == 0 then
    show_status(state, "No entries in session")
    return
  end
  show_selector(state, function(done)
    return tree_selector({
      theme = state.theme,
      tree = tree,
      current_leaf_id = real_leaf_id,
      terminal_rows = state.rows or 30,
      home = state.home,
      now_ms = state.wall_now_ms,
      initial_selected_id = initial_selected_id,
      initial_filter_mode = initial_filter_mode,
      on_select = function(entry_id)
        if entry_id == real_leaf_id then
          done()
          show_status(state, "Already at this point")
          return
        end
        done()
        if (state.request and state.request.branchSummarySkipPrompt)
           or pi.settings.branch_summary().skipPrompt then
          tree_navigate_with_options(state, entry_id, false, nil)
          return
        end
        tree_choose_summary(state, entry_id)
      end,
      on_cancel = function() done() end,
      on_label_change = function(entry_id, label)
        state.session_manager:append_label_change(entry_id, label)
      end,
    })
  end)
end

-- interactive-mode.ts quoteIfNeeded + formatResumeCommand. The TTY gate is
-- the interactive mode itself (pi-rs only reaches shutdown from a live
-- terminal session).
local function quote_if_needed(value)
  if #value > 0 and not value:find("[^a-zA-Z0-9_%-./~:@]") then return value end
  return "'" .. value:gsub("'", "'\\''") .. "'"
end

-- Extension command actions are applied by the interactive process loop. A
-- successful replacement binds the fresh runtime before withSession runs;
-- captured pre-replacement contexts are already stale at that point.
function finish_extension_replacement(state, options)
  if options and options.withSession then
    options.withSession(EXTENSION_CONTEXT_POLICY.snapshot(state, { command = true }))
  end
end

function interactive_extension_action_handlers(state)
  return {
    abort = function()
      restore_queued_messages_to_editor(state, { abort = true })
    end,
    shutdown = function()
      state.shutdown_requested = true
    end,
    compact = function(action)
      local opts = action.options or {}
      state.turn = pi.spawn(function()
        local ok, result = pcall(state.session.compact, opts.customInstructions)
        if ok and opts.onComplete then opts.onComplete(result) end
        if not ok and opts.onError then opts.onError({ message = tostring(result) }) end
      end)
    end,
    new_session = function(action)
      local options = action.options or {}
      local current = state.session_manager
      local manager
      if current:is_persisted() then
        manager = pi.session.create({
          cwd = state.cwd, sessionDir = current:get_session_dir(),
          agentDir = state.request.agentDir,
        })
      else
        manager = pi.session.in_memory({ cwd = state.cwd })
      end
      if options.parentSession then manager:new_session({ parentSession = options.parentSession }) end
      state.agent:abort()
      bind_session_runtime(state, manager)
      if options.setup then
        options.setup(manager)
        state.agent:set_messages(manager:build_session_context().messages)
      end
      finish_extension_replacement(state, options)
      render_current_session_state(state)
      return { cancelled = false }
    end,
    fork = function(action)
      local result = fork_session_runtime(state, action.entryId, action.options)
      if result.cancelled then return { cancelled = true } end
      finish_extension_replacement(state, action.options)
      render_current_session_state(state)
      state.editor.editor:set_text(result.selectedText or "")
      show_status(state, "Forked to new session")
      return { cancelled = false }
    end,
    navigate_tree = function(action)
      local result = navigate_session_tree(state, action.targetId, action.options)
      if result.cancelled then return { cancelled = true } end
      render_current_session_state(state)
      if result.editorText and trim(state.editor.editor:get_text()) == "" then
        state.editor.editor:set_text(result.editorText)
      end
      show_status(state, "Navigated to selected point")
      flush_compaction_queue(state, { willRetry = false })
      return { cancelled = false }
    end,
    switch_session = function(action)
      local switched, issue = attempt_switch_session(state, action.sessionPath, nil)
      if not switched then
        local confirmed = state.extension_ui.confirm(
          "Session cwd not found", format_missing_session_cwd_prompt(issue))
        if not confirmed then
          show_status(state, "Resume cancelled")
          return { cancelled = true }
        end
        switched = attempt_switch_session(state, action.sessionPath, issue.fallbackCwd)
      end
      finish_extension_replacement(state, action.options)
      render_current_session_state(state)
      show_status(state, "Resumed session")
      return { cancelled = not switched }
    end,
    reload = function()
      local task = handle_reload_command(state)
      if task then task:join() end
    end,
  }
end

local function format_resume_command(state)
  local session_manager = state.session_manager
  if not session_manager:is_persisted() then return nil end
  local session_file = session_manager:get_session_file()
  if not session_file or not pi.fs.exists(session_file) then return nil end
  local args = { state.app_name }
  if not session_manager:uses_default_session_dir() then
    args[#args + 1] = "--session-dir"
    args[#args + 1] = quote_if_needed(session_manager:get_session_dir())
  end
  args[#args + 1] = "--session"
  args[#args + 1] = session_manager:get_session_id()
  return table.concat(args, " ")
end

-- interactive-mode.ts constructor + init(): the product state, agent, and
-- editor wiring shared by the process-session loop (run_interactive) and
-- the provider parity sequence.
local function create_interactive_state(request)
  local data = request.theme == "light" and light_json or dark_json
  local theme = create_theme(data, request.colorMode or "truecolor")
  local state = {
    theme = theme, theme_data = data, md_theme = nil, model = request.model, cwd = request.cwd or pi.cwd(),
    home = request.home, branch = request.branch,
    app_name = request.appName or "pi", version = request.version,
    thinking_level = request.thinkingLevel or "off",
    transcript = {}, streaming_row = nil, pending_tools = {},
    tools_expanded = false, header_expanded = false,
    steering_texts = {}, follow_up_texts = {},
    -- settingsManager.getDoubleEscapeAction() (fixtures pin the value).
    double_escape_action = request.doubleEscapeAction
      or pi.settings.double_escape_action(),
    now_ms = pi.monotonic_ms,
    usage = {}, exit = false,
    selector = nil, login = nil, docs_path = request.docsPath, request = request,
    auth = default_auth_seam(),
    registry = default_registry_seam(),
    scoped_models = {},
    hide_thinking_block = pi.settings.hide_thinking_block(),
    project_trusted = request.projectTrusted ~= false,
    inherited_process_callbacks = {}, pending_inherited_process = nil,
    pending_suspend = false,
    extension_ui_actions = {}, extension_ui_active = nil,
    extension_actions = {}, extension_context_generation = 0, shutdown_requested = false,
    extension_mode = "tui", extension_has_ui = true,
  }
  state.extension_ui = EXTENSION_UI_POLICY.context(state)
  -- Spec: main.ts mirrors --api-key into the auth storage as a runtime
  -- override before the session starts; per-request resolution then
  -- flows through the getApiKey seam below.
  if request.runtimeApiKey then
    pi.auth.set_runtime_api_key(request.model.provider, request.runtimeApiKey)
  end
  -- main.ts resolves settings enabledModels before session creation; the
  -- exact IDs written by /scoped-models restore their ordered cycling set.
  DEFAULT_KEYS.__scoped_models_policy.initialize(state)
  -- Spec: interactive-mode start() initializes the footer's provider count.
  update_available_provider_count(state)
  -- interactive-mode.ts ui.setFocus — the editor emits the hardware-cursor
  -- marker while focused; selector/dialog mounts move focus away.
  state.set_editor_focus = function(focused)
    if state.editor then state.editor.editor:set_focused(focused) end
  end
  state.md_theme = get_markdown_theme(theme)

  -- Date.now() for session ages (fixtures pin `nowMs`).
  state.wall_now_ms = function() return request.nowMs or pi.now_ms() end

  -- main.ts createSessionManager → sdk.ts createAgentSession: open the
  -- CLI-selected session (--continue/--session) or create a fresh one;
  -- the session's cwd is the effective runtime cwd, and the restore
  -- slice recovers the saved model, thinking level, and messages.
  local session_manager = construct_session({
    sessionFile = request.sessionFile, sessionDir = request.sessionDir,
    agentDir = request.agentDir, cwd = state.cwd,
    cwdOverride = request.cwdOverride,
  })
  local startup = bind_session_runtime(state, session_manager)
  state.extension_is_idle = function() return not state.session.is_streaming() end
  state.extension_has_pending = function()
    return #state.steering_texts > 0 or #state.follow_up_texts > 0
      or #state.compaction_queued > 0
  end
  state.extension_action_handlers = interactive_extension_action_handlers(state)
  state.extension_after_pump = function()
    if state.shutdown_requested and not state.session.is_streaming() then shutdown(state) end
  end

  setup_shell_editor(state)
  -- interactive-mode.ts setupAutocompleteProvider + editor.setAutocompleteProvider.
  state.fd_path = request.fdPath or resolve_fd_path()
  state.autocomplete_provider = create_base_autocomplete_provider(state)
  state.editor.editor:set_autocomplete_triggers({})
  state.editor:on_action("app.model.select", function() show_model_selector(state) end)
  state.editor:on_action("app.model.cycleForward", function() cycle_model(state, "forward") end)
  state.editor:on_action("app.model.cycleBackward", function() cycle_model(state, "backward") end)

  -- interactive-mode.ts rebindCurrentSession mounts startup notices before
  -- renderInitialMessages. Resumed sessions are suppressed by their rebuilt
  -- agent message context.
  local startup_notice = mount_startup_changelog(state, {
    messages = state.agent:get_state().messages or {},
    markdown = request.changelogText or CHANGELOG_MD,
    version = state.version,
  })
  if startup_notice and startup_notice.telemetry_due then
    report_install_telemetry(state, state.version)
  end
  render_initial_messages(state)
  if startup.fallback_message then show_warning(state, startup.fallback_message) end
  -- interactive-mode.ts run(): the request is fire-and-forget while the
  -- process-session loop continues handling input and render ticks.
  start_version_check(state)

  local submit_actions = shell_submit_actions(state)
  state.submit = function(text) handle_submit(text, submit_actions) end
  return state
end

local function run_interactive_state(state)
  local process = pi.tui.process_session(pi.settings.show_hardware_cursor())
  return process:run(function(event)
    if event.type == "signal" then return { exit = true } end
    if event.type == "start" or event.type == "resize" then
      state.columns = event.columns
      state.rows = event.rows
      state.editor.editor:set_terminal_rows(event.rows)
      return { lines = frontend_frame(state, event.columns), force = true, title = "pi",
        progress = pi.settings.show_terminal_progress() and state.session.is_streaming(),
        showHardwareCursor = pi.settings.show_hardware_cursor(),
        clearOnShrink = pi.settings.clear_on_shrink() }
    end
    if event.type == "input" then
      if state.selector then
        state.selector:handle_input(event.data)
        pump_login(state, 0)
        return { lines = frontend_frame(state, state.columns or 80), exit = state.exit,
          progress = pi.settings.show_terminal_progress() and state.session.is_streaming(),
          showHardwareCursor = pi.settings.show_hardware_cursor(),
          clearOnShrink = pi.settings.clear_on_shrink(),
          inheritedProcess = take_inherited_process(state),
          suspend = take_suspend(state) }
      end
      local effect = state.editor:handle_input(event.data)
      if effect.kind == "submit" then
        state.submit(effect.text or effect.value or "")
      end
      pump_login(state, 0)
      pump_editor_autocomplete(state, pi.monotonic_ms())
      return { lines = frontend_frame(state, state.columns or 80), exit = state.exit,
        progress = pi.settings.show_terminal_progress() and state.session.is_streaming(),
        showHardwareCursor = pi.settings.show_hardware_cursor(),
        clearOnShrink = pi.settings.clear_on_shrink(),
        inheritedProcess = take_inherited_process(state),
        suspend = take_suspend(state) }
    end
    if event.type == "inherited_process_result" then
      local callback = state.inherited_process_callbacks[event.id]
      state.inherited_process_callbacks[event.id] = nil
      if callback then callback(event.status) end
      return { lines = frontend_frame(state, state.columns or 80), force = true,
        progress = pi.settings.show_terminal_progress() and state.session.is_streaming(),
        showHardwareCursor = pi.settings.show_hardware_cursor(),
        clearOnShrink = pi.settings.clear_on_shrink() }
    end
    if event.type == "tick" then
      if state.exit then return { exit = true } end
      local now = pi.monotonic_ms()
      local render = false
      EXTENSION_CONTEXT_POLICY.pump(state)
      if EXTENSION_UI_POLICY.pump(state) then render = true end
      if state.async_render then
        state.async_render = false
        render = true
      end
      if state.loader then
        local elapsed = now - (state.loader.last_ms or now)
        state.loader.last_ms = now
        if state.loader.loader:advance(elapsed) then render = true end
      end
      if state.retry_countdown and now >= state.retry_countdown.next_ms then
        local countdown = state.retry_countdown
        while now >= countdown.next_ms and countdown.remaining > 0 do
          countdown.remaining = countdown.remaining - 1
          countdown.next_ms = countdown.next_ms + 1000
        end
        if state.loader and state.loader.kind == "retry" then
          state.loader.message = countdown.message(countdown.remaining)
        end
        if countdown.remaining <= 0 then state.retry_countdown = nil end
        render = true
      end
      -- A running bash command animates its component loader and streams
      -- output; one more render settles the completed frame.
      if state.bash_row then
        local row = state.bash_row
        if row.loader then
          local elapsed = now - (row.loader_last_ms or now)
          row.loader_last_ms = now
          row.loader:advance(elapsed)
        end
        state.bash_was_active = true
        render = true
      elseif state.bash_was_active then
        state.bash_was_active = false
        render = true
      end
      local autocompleted = not state.selector and pump_editor_autocomplete(state, now)
      if autocompleted then render = true end
      if state.login then
        pump_login(state, 0)
        render = true
      end
      -- Session-selector status auto-hide (the spec's setTimeout →
      -- requestRender).
      if state.selector and state.selector.needs_render
         and state.selector:needs_render(state.wall_now_ms()) then
        render = true
      end
      if state.session.is_streaming() then render = true end
      if render then
        return { lines = frontend_frame(state, state.columns or 80), exit = state.exit,
          progress = pi.settings.show_terminal_progress() and state.session.is_streaming(),
          showHardwareCursor = pi.settings.show_hardware_cursor(),
          clearOnShrink = pi.settings.clear_on_shrink() }
      end
    end
    return nil
  end)
end

-- interactive-mode.ts shutdown(): after the TUI stops, print the
-- resume-command line for persisted sessions (chalk.dim label).
local function print_resume_command(state)
  local resume_command = format_resume_command(state)
  if not resume_command then return end
  io.write("\27[2mTo resume this session:\27[22m " .. resume_command .. "\n")
end

pi.register_role({
  id = "coding-agent-interactive", role = "interactive", active = true, priority = 0,
  description = "Run the default Lua-authored interactive frontend",
  handler = function(args)
    local request = pi.json.decode(args)
    local state = create_interactive_state(request)
    local result = run_interactive_state(state)
    print_resume_command(state)
    -- handleFatalRuntimeError exits with status 1 (main.rs maps it).
    return { result = result, exitCode = state.exit_code or 0 }
  end,
})

-- cli/session-picker.ts selectSession — the --resume flag's standalone
-- selector: a TUI whose only child is the SessionSelectorComponent (no
-- rename seam, no rename hint), focused on its session list. Returns the
-- selected path, or none when cancelled; `quit` mirrors the picker's
-- onExit → process.exit(0).
pi.register_role({
  id = "coding-agent-session-picker", role = "resume-picker", active = true, priority = 0,
  description = "Select a session to resume (--resume)",
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local now_ms = function() return request.nowMs or os.time() * 1000 end
    local selected, settled, exit_requested = nil, false, false
    local selector = session_selector({
      theme = theme,
      home = request.home,
      now_ms = now_ms,
      load_current = function()
        return pi.session.list({ cwd = request.cwd, sessionDir = request.sessionDir,
          agentDir = request.agentDir })
      end,
      load_all = function()
        return pi.session.list_all({ sessionDir = request.sessionDir,
          agentDir = request.agentDir })
      end,
      on_select = function(path)
        selected = path
        settled = true
      end,
      on_cancel = function() settled = true end,
      on_exit = function()
        exit_requested = true
        settled = true
      end,
      show_rename_hint = false,
    })
    selector:set_focused(true)
    local columns = 80
    local process = pi.tui.process_session(true)
    process:run(function(event)
      if event.type == "signal" then
        settled = true
        return { exit = true }
      end
      if event.type == "start" or event.type == "resize" then
        columns = event.columns
        return { lines = selector:render(columns), force = true }
      end
      if event.type == "input" then
        selector:handle_input(event.data)
        return { lines = selector:render(columns), exit = settled }
      end
      if event.type == "tick" then
        if settled then return { exit = true } end
        if selector:needs_render(now_ms()) then
          return { lines = selector:render(columns) }
        end
      end
      return nil
    end)
    return { path = selected, quit = exit_requested }
  end,
})

-- cli/startup-ui.ts showStartupSelector — a pre-runtime TUI containing an
-- ExtensionSelectorComponent; main.rs maps the selected label to a value
-- (the missing-session-cwd prompt's Continue/Cancel).
pi.register_role({
  id = "coding-agent-startup-selector", role = "startup-selector", active = true, priority = 0,
  description = "Pre-runtime selector (startup prompts)",
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local result, settled = nil, false
    local selector = extension_selector({
      theme = theme,
      title = request.title,
      options = request.options,
      on_select = function(option)
        result = option
        settled = true
      end,
      on_cancel = function() settled = true end,
    })
    selector:set_focused(true)
    local columns = 80
    local process = pi.tui.process_session(true)
    process:run(function(event)
      if event.type == "signal" then
        settled = true
        return { exit = true }
      end
      if event.type == "start" or event.type == "resize" then
        columns = event.columns
        return { lines = selector:render(columns), force = true }
      end
      if event.type == "input" then
        selector:handle_input(event.data)
        return { lines = selector:render(columns), exit = settled }
      end
      return nil
    end)
    return { value = result }
  end,
})

-- Deterministic composition exerciser (no terminal ownership or provider I/O).
pi.register_command("interactive-frame", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local state = { theme = theme, md_theme = get_markdown_theme(theme),
      model = request.model, cwd = request.cwd, branch = request.branch,
      app_name = "pi", version = request.version, header_expanded = false,
      thinking_level = "off", transcript = request.transcript or {},
      steering_texts = {}, follow_up_texts = {},
      streaming_message = request.streaming and {
        role = "assistant", content = {{ type = "text", text = request.streaming }},
      } or nil,
      usage = {} }
    if request.status and request.status ~= "" then
      state.transcript[#state.transcript + 1] = { kind = "status", text = request.status }
    end
    state.editor = custom_editor({ value = request.editor or "", theme = theme })
    return { lines = frontend_frame(state, request.width or 80) }
  end,
})

-- Scripted public-surface exerciser for differential terminal capture. It
-- mirrors tests/ui-parity/pi-basic-turn.ts exactly: the same component tree
-- (header text, spacer, transcript, status, spacer, minimal focused editor,
-- footer), the same checkpoints, and the ported message components.
pi.register_command("interactive-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local md_theme = get_markdown_theme(theme)
    local editor = custom_editor({ theme = theme })
    local columns, rows = request.columns, request.rows
    editor.editor:set_terminal_rows(rows)
    editor.editor:set_focused(true)
    local header_text = theme:bold(theme:fg("accent", "pi"))
      .. theme:fg("dim", " v" .. (request.version or "0.79.0"))
    local footer_text = theme:fg("dim", request.cwd .. " (" .. request.branch .. ")")
    local transcript = {}
    local working = false
    local function frame()
      local lines = {}
      append(lines, pi.tui.text_render(header_text, columns, 1, 0))
      lines[#lines + 1] = "" -- Spacer(1)
      append(lines, transcript)
      if working then
        -- statusContainer's working Loader (frozen at frame 0).
        lines[#lines + 1] = ""
        append(lines, pi.tui.text_render(
          theme:fg("accent", "⠋") .. " " .. theme:fg("muted", "Working..."), columns, 1, 0))
      end
      lines[#lines + 1] = "" -- Spacer(1)
      for _, line in ipairs(editor.editor:render(columns)) do lines[#lines + 1] = line end
      append(lines, pi.tui.text_render(footer_text, columns, 0, 0))
      return lines
    end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frame())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, input in ipairs(request.input or {}) do
      editor:handle_input(input)
    end
    append(transcript, user_message_lines(request.prompt, columns, theme, md_theme))
    working = true
    capture("submitted")
    working = false
    local assistant_start = #transcript
    local function assistant_message(text)
      return { role = "assistant", content = {{ type = "text", text = text }}, stopReason = "stop" }
    end
    append(transcript, assistant_message_lines(assistant_message(request.partial), columns, theme, md_theme))
    capture("streaming")
    for index = #transcript, assistant_start + 1, -1 do transcript[index] = nil end
    append(transcript, assistant_message_lines(assistant_message(request.completion), columns, theme, md_theme))
    capture("complete")
    for _, resize in ipairs(request.resizes or { request.resize }) do
      columns, rows = resize.columns, resize.rows
      editor.editor:set_terminal_rows(rows)
      -- Re-render transcript components at the new width, like pi's
      -- width-sensitive Component.render(width).
      transcript = {}
      append(transcript, user_message_lines(request.prompt, columns, theme, md_theme))
      append(transcript, assistant_message_lines(assistant_message(request.completion), columns, theme, md_theme))
      terminal:resize(columns, rows)
      capture(resize.name or "resize", true)
    end
    return { frames = frames }
  end,
})

-- Scripted editor exerciser for differential terminal capture. Mirrors
-- tests/ui-parity/pi-editor-turn.ts exactly: the product CustomEditor policy
-- object over the pi.tui editor mechanism with the coding-agent editor theme,
-- a dim JSON scaffold row per recorded submission (both drivers use the same
-- recipe), and interactive-mode's normal-path submit handling (trim, skip
-- empty, addToHistory).
pi.register_command("interactive-editor-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local editor = custom_editor({ theme = theme })
    editor.editor:set_terminal_rows(rows)
    editor.editor:set_focused(true)
    local submitted = {}
    local function frame()
      local lines = {}
      for _, text in ipairs(submitted) do
        append(lines, pi.tui.text_render(theme:fg("dim", pi.json.encode(text)), columns, 0, 0))
      end
      for _, line in ipairs(editor.editor:render(columns)) do lines[#lines + 1] = line end
      return lines
    end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frame())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        local effect = editor:handle_input(input)
        if effect.kind == "submit" then
          local text = trim(effect.text or "")
          if text ~= "" then
            submitted[#submitted + 1] = text
            editor.editor:add_to_history(text)
          end
        end
      end
      capture(step.name, step.resize ~= nil)
    end
    return { frames = frames }
  end,
})

-- Scripted interaction-shell exerciser for differential terminal capture.
-- Mirrors tests/ui-parity/pi-shell-turn.ts exactly: the real container
-- composition (frontend_frame) and the product key-handler/submit wiring
-- (setup_shell_editor, shell_submit_actions) over a scripted session whose
-- streaming lifecycle the scenario controls. The working loader stays at
-- spinner frame 0 on both sides (the pi driver stops its interval).
pi.register_command("interactive-shell-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local initial_thinking_message = request.thinkingText and {
      role = "assistant", content = { { type = "thinking", thinking = request.thinkingText } },
      stopReason = "stop",
    } or nil
    local state = {
      theme = theme, md_theme = get_markdown_theme(theme),
      app_name = request.appName or "pi", version = request.version,
      model = request.model, cwd = request.cwd, home = request.home,
      branch = request.branch, thinking_level = "off",
      transcript = initial_thinking_message
        and { { kind = "assistant", message = initial_thinking_message } } or {},
      streaming_message = nil, hide_thinking_block = false,
      tools_expanded = false, header_expanded = false,
      steering_texts = {}, follow_up_texts = {},
      double_escape_action = request.doubleEscapeAction or "tree",
      read_clipboard_image = request.clipboardImagePath
        and function() return request.clipboardImagePath end or nil,
      now_ms = function() return 0 end,
      usage = {
        input = (request.usage or {}).input or 0,
        output = (request.usage or {}).output or 0,
        cache_read = (request.usage or {}).cacheRead or 0,
        cache_write = (request.usage or {}).cacheWrite or 0,
        cost = (request.usage or {}).cost or 0,
      }, exit = false,
      provider_count = request.providerCount or 1,
      scoped_models = {},
      pending_suspend = false,
      extension_mode = "tui", extension_has_ui = true,
      extension_ui_actions = {}, extension_actions = {}, extension_context_generation = 0,
    }
    state.extension_ui = EXTENSION_UI_POLICY.context(state)
    state.session_manager = pi.session.in_memory({ cwd = state.cwd })
    state.registry = {
      get_available = function() return {} end, find = function() return nil end,
      has_configured_auth = function() return false end,
      is_using_oauth = function() return false end,
    }
    state.external_editor_command = request.externalEditorCommand
    local events = {}
    local streaming = false
    state.session = {
      is_streaming = function() return streaming end,
      clear_queues = function() end,
      steer = function(text) events[#events + 1] = { type = "steer", text = text } end,
      follow_up = function(text) events[#events + 1] = { type = "followUp", text = text } end,
      abort = function()
        -- Scripted agent: the in-flight turn settles as an aborted
        -- assistant message (message_end) followed by agent_end.
        events[#events + 1] = { type = "abort" }
        streaming = false
        state.loader = nil
        state.transcript[#state.transcript + 1] = { kind = "assistant", message = {
          role = "assistant", content = {}, stopReason = "aborted",
          errorMessage = "Operation aborted" } }
      end,
      prompt = function(text)
        -- Scripted agent_start (working loader) + user message_start.
        events[#events + 1] = { type = "prompt", text = text }
        start_working_loader(state)
        state.transcript[#state.transcript + 1] = { kind = "user", text = text }
        streaming = true
      end,
    }
    local shortcut_context
    if request.shortcut then
      pi.register_shortcut(request.shortcut.key, {
        description = "parity shortcut",
        handler = function(ctx)
          shortcut_context = { mode = ctx.mode, hasUI = ctx.hasUI, idle = ctx.isIdle(),
            pending = ctx.hasPendingMessages(), cwd = ctx.cwd }
          ctx.ui.notify(request.shortcut.status)
        end,
      })
    end
    setup_shell_editor(state)
    state.editor.editor:set_terminal_rows(rows)
    local submit_actions = shell_submit_actions(state)
    state.submit = function(text) handle_submit(text, submit_actions) end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frontend_frame(state, columns))
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        local effect = state.editor:handle_input(input)
        if effect.kind == "submit" then
          state.submit(effect.text or effect.value or "")
        end
        pi.sleep(1)
        EXTENSION_CONTEXT_POLICY.pump(state)
        EXTENSION_UI_POLICY.pump(state)
      end
      capture(step.name, step.resize ~= nil)
    end
    return { frames = frames, events = events, exited = state.exit,
      suspend = take_suspend(state), shortcutContext = shortcut_context }
  end,
})

-- Scripted thinking-level exerciser for differential terminal capture
-- (PLAN 7.2). Mirrors tests/ui-parity/pi-thinking-turn.ts exactly: the
-- real container composition (frontend_frame) and the product shell
-- wiring (setup_shell_editor's app.thinking.cycle action, handle_submit's
-- /model route, session_set_model's re-clamp, session_set_thinking_level
-- over the real pi.ai clamp and pi.settings default) against a scenario
-- model catalog; the pi driver's settings stub matches pi-rs's real
-- settings manager over the harness's pinned empty agent dir.
pi.register_command("interactive-thinking-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local models = request.models or {}
    local function find_model(provider, id)
      for _, model in ipairs(models) do
        if model.provider == provider and model.id == id then return model end
      end
      return nil
    end
    local state = {
      theme = theme, md_theme = get_markdown_theme(theme),
      app_name = request.appName or "pi", version = request.version,
      model = request.model, cwd = request.cwd, home = request.home,
      branch = request.branch,
      thinking_level = request.thinkingLevel or "off",
      transcript = {}, streaming_message = nil,
      tools_expanded = false, header_expanded = false,
      steering_texts = {}, follow_up_texts = {},
      double_escape_action = "none",
      now_ms = function() return 0 end,
      usage = {
        input = (request.usage or {}).input or 0,
        output = (request.usage or {}).output or 0,
        cache_read = (request.usage or {}).cacheRead or 0,
        cache_write = (request.usage or {}).cacheWrite or 0,
        cost = (request.usage or {}).cost or 0,
      },
      context_percent = (request.contextUsage or {}).percent,
      exit = false,
      provider_count = request.providerCount or 1,
      scoped_models = {},
      registry = {
        refresh = function() end,
        get_error = function() return nil end,
        get_available = function() return models end,
        find = find_model,
        has_configured_auth = function(_model) return true end,
        is_using_oauth = function(_model) return false end,
      },
    }
    state.session = {
      is_streaming = function() return false end,
      clear_queues = function() end,
      steer = function() end,
      follow_up = function() end,
      abort = function() end,
      prompt = function() end,
    }
    setup_shell_editor(state)
    state.editor.editor:set_terminal_rows(rows)
    local submit_actions = shell_submit_actions(state)
    state.submit = function(text) handle_submit(text, submit_actions) end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frontend_frame(state, columns))
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        local effect = state.editor:handle_input(input)
        if effect.kind == "submit" then
          state.submit(effect.text or effect.value or "")
        end
      end
      capture(step.name, step.resize ~= nil)
    end
    return { frames = frames }
  end,
})

-- /settings editor-slot differential driver. Product routing and the real
-- settings store are used; only unrelated chat/footer containers are omitted.
pi.register_command("interactive-settings-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = (pi.settings.theme() or request.theme) == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local state = {
      theme = theme, md_theme = get_markdown_theme(theme), model = request.model,
      thinking_level = request.thinkingLevel or "off", transcript = {},
      tools_expanded = false, header_expanded = false, steering_texts = {}, follow_up_texts = {},
      double_escape_action = "none", cwd = request.cwd or pi.cwd(), now_ms = function() return 0 end,
      usage = {}, exit = false, scoped_models = {}, registry = {
        has_configured_auth = function() return true end, get_available = function() return {} end,
      },
      session = { is_streaming = function() return false end, clear_queues = function() end,
        steer = function() end, follow_up = function() end, abort = function() end,
        prompt = function() end },
    }
    state.set_editor_focus = function(focused)
      if state.editor then state.editor.editor:set_focused(focused) end
    end
    setup_shell_editor(state)
    state.editor.editor:set_terminal_rows(rows)
    local actions = shell_submit_actions(state)
    state.submit = function(text) handle_submit(text, actions) end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function lines()
      if state.selector then return state.selector:render(columns) end
      sync_editor_border(state)
      return state.editor.editor:render(columns)
    end
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(lines())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        if state.selector then state.selector:handle_input(input)
        else
          local effect = state.editor:handle_input(input)
          if effect.kind == "submit" then state.submit(effect.text or effect.value or "") end
        end
      end
      capture(step.name, step.resize ~= nil)
    end
    return { frames = frames }
  end,
})

-- /scoped-models editor-slot differential driver. The product selector,
-- session scope mutation, settings persistence, and command route are shared.
pi.register_command("interactive-scoped-models-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local theme = create_theme(dark_json, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local models = request.models or {}
    local state = {
      theme = theme, md_theme = get_markdown_theme(theme), model = models[1], transcript = {},
      thinking_level = "off", tools_expanded = false, header_expanded = false,
      steering_texts = {}, follow_up_texts = {}, double_escape_action = "none",
      cwd = pi.cwd(), now_ms = function() return 0 end, usage = {}, exit = false, scoped_models = {},
      registry = { refresh = function() end, get_available = function() return models end,
        has_configured_auth = function() return true end },
      session = { is_streaming = function() return false end, clear_queues = function() end,
        steer = function() end, follow_up = function() end, abort = function() end,
        prompt = function() end },
    }
    state.set_editor_focus = function(focused)
      if state.editor then state.editor.editor:set_focused(focused) end
    end
    setup_shell_editor(state)
    state.editor.editor:set_terminal_rows(rows)
    local actions = shell_submit_actions(state)
    state.submit = function(text) handle_submit(text, actions) end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function lines()
      if state.selector then return state.selector:render(columns) end
      sync_editor_border(state)
      return state.editor.editor:render(columns)
    end
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(lines())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    for _, step in ipairs(request.steps or {}) do
      if step.show then DEFAULT_KEYS.__scoped_models_policy.show(state) end
      if step.cycle then session_cycle_model(state, step.cycle) end
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        if state.selector then state.selector:handle_input(input)
        else
          local effect = state.editor:handle_input(input)
          if effect.kind == "submit" then state.submit(effect.text or effect.value or "") end
        end
      end
      capture(step.name, step.name == "startup" or step.resize ~= nil)
    end
    local scoped_ids = {}
    for _, scoped in ipairs(state.scoped_models) do
      scoped_ids[#scoped_ids + 1] = scoped.model.provider .. "/" .. scoped.model.id
    end
    return { frames = frames, scopedModels = scoped_ids, savedModels = pi.settings.enabled_models(),
      currentModel = state.model and (state.model.provider .. "/" .. state.model.id) or nil }
  end,
})


function extension_context_action_oracle(model, cwd)
  local function new_state(mode, has_ui)
    local manager = pi.session.in_memory({ cwd = cwd })
    return {
      cwd = cwd, model = model, session_manager = manager, project_trusted = true,
      extension_mode = mode, extension_has_ui = has_ui,
      extension_actions = {}, extension_context_generation = 0,
      system_prompt_options = { cwd = cwd },
      registry = {
        get_available = function() return { model } end,
        find = function(provider, id)
          if provider == model.provider and id == model.id then return model end
        end,
        has_configured_auth = function() return true end,
        is_using_oauth = function() return false end,
      },
      extension_is_idle = function() return true end,
      extension_has_pending = function() return false end,
    }
  end
  local function run(state, callback)
    local task = pi.spawn(callback)
    while not task:done() do
      EXTENSION_CONTEXT_POLICY.pump(state)
      pi.sleep(1)
    end
    EXTENSION_CONTEXT_POLICY.pump(state)
    return task:join()
  end
  local function stale_text(ok, value)
    if ok then return "" end
    return tostring(value):match(":%d+: (.*)$") or tostring(value)
  end

  local modes = {}
  for _, row in ipairs({ { "print", false }, { "json", false }, { "rpc", true } }) do
    local mode_state = new_state(row[1], row[2])
    local mode_context = EXTENSION_CONTEXT_POLICY.snapshot(mode_state)
    modes[#modes + 1] = { mode = mode_context.mode, hasUI = mode_context.hasUI }
  end

  local state = new_state("tui", true)
  local trace = { "wait" }
  state.extension_action_handlers = {
    new_session = function(action)
      trace[#trace + 1] = "new:" .. (action.options.parentSession or "")
      return { cancelled = action.options.parentSession == "cancel" }
    end,
    fork = function(action)
      trace[#trace + 1] = "fork:" .. action.entryId .. ":" .. (action.options.position or "before")
      return { cancelled = action.entryId == "cancel" }
    end,
    navigate_tree = function(action)
      trace[#trace + 1] = "tree:" .. action.targetId .. ":"
        .. tostring(action.options.summarize == true) .. ":" .. (action.options.label or "")
      return { cancelled = action.targetId == "cancel" }
    end,
    switch_session = function(action)
      trace[#trace + 1] = "switch:" .. action.sessionPath
      return { cancelled = action.sessionPath == "cancel" }
    end,
    reload = function() trace[#trace + 1] = "reload" end,
  }
  local base = EXTENSION_CONTEXT_POLICY.snapshot(state)
  local command = EXTENSION_CONTEXT_POLICY.snapshot(state, { command = true })
  run(state, command.waitForIdle)
  local outcomes = {
    newSession = run(state, function()
      return command.newSession({ parentSession = "parent.jsonl" })
    end),
    newCancelled = run(state, function()
      return command.newSession({ parentSession = "cancel" })
    end),
    fork = run(state, function() return command.fork("entry-1", { position = "at" }) end),
    forkCancelled = run(state, function() return command.fork("cancel") end),
    tree = run(state, function()
      return command.navigateTree("entry-2", { summarize = true, label = "kept" })
    end),
    treeCancelled = run(state, function() return command.navigateTree("cancel") end),
    switchSession = run(state, function() return command.switchSession("other.jsonl") end),
    switchCancelled = run(state, function() return command.switchSession("cancel") end),
  }
  run(state, command.reload)
  local actions = {
    restrictions = {
      baseNewSession = type(base.newSession) == "nil" and "undefined" or type(base.newSession),
      baseFork = type(base.fork) == "nil" and "undefined" or type(base.fork),
      baseTree = type(base.navigateTree) == "nil" and "undefined" or type(base.navigateTree),
      baseSwitch = type(base.switchSession) == "nil" and "undefined" or type(base.switchSession),
      baseReload = type(base.reload) == "nil" and "undefined" or type(base.reload),
      commandNewSession = type(command.newSession), commandFork = type(command.fork),
      commandTree = type(command.navigateTree), commandSwitch = type(command.switchSession),
      commandReload = type(command.reload),
    },
    trace = trace, outcomes = outcomes,
  }

  local replacement_state = new_state("tui", true)
  local replacement_trace = {}
  replacement_state.extension_action_handlers = {
    new_session = function(action)
      replacement_trace[#replacement_trace + 1] = "shutdown"
      replacement_state.extension_context_generation = replacement_state.extension_context_generation + 1
      replacement_trace[#replacement_trace + 1] = "rebind"
      if action.options.withSession then
        action.options.withSession(EXTENSION_CONTEXT_POLICY.snapshot(
          replacement_state, { command = true }))
      end
      replacement_trace[#replacement_trace + 1] = "action-return"
      return { cancelled = false }
    end,
  }
  local old = EXTENSION_CONTEXT_POLICY.snapshot(replacement_state, { command = true })
  local replacement_result = run(replacement_state, function()
    return old.newSession({ withSession = function(fresh)
      replacement_trace[#replacement_trace + 1] = "withSession"
      replacement_trace[#replacement_trace + 1] = "fresh:" .. fresh.mode .. ":" .. tostring(fresh.isIdle())
      local old_ok = pcall(old.isIdle)
      replacement_trace[#replacement_trace + 1] = "old-stale:" .. tostring(not old_ok)
    end })
  end)
  local replacement_stale = stale_text(pcall(old.isIdle))

  local reload_state = new_state("print", false)
  local reload_trace = {}
  reload_state.extension_action_handlers = {
    reload = function()
      reload_trace[#reload_trace + 1] = "shutdown"
      reload_state.extension_context_generation = reload_state.extension_context_generation + 1
      reload_trace[#reload_trace + 1] = "reloaded"
    end,
  }
  local reload_context = EXTENSION_CONTEXT_POLICY.snapshot(reload_state, { command = true })
  run(reload_state, reload_context.reload)
  local reload_stale = stale_text(pcall(reload_context.getSystemPrompt))

  return {
    modes = modes, actions = actions,
    replacement = { trace = replacement_trace, result = replacement_result, stale = replacement_stale },
    reload = { trace = reload_trace, stale = reload_stale },
  }
end

-- PLAN 9.2 base-context differential seam. This builds the same interactive
-- runtime snapshots and action pump used by loaded commands/tools.
pi.register_command("interactive-extension-context-parity", {
  handler = function(args)
    local request = pi.json.decode(args)
    local model = request.model
    local state = {
      request = request, model = model, cwd = request.cwd or pi.cwd(),
      project_trusted = true, steering_texts = {}, follow_up_texts = {},
      compaction_queued = {}, extension_actions = {},
      extension_context_generation = 0, shutdown_requested = false,
      extension_mode = "tui", extension_has_ui = true,
      async_render = false, exit = false, usage = {}, transcript = {},
      registry = {
        get_available = function() return { model } end,
        find = function(provider, id)
          if provider == model.provider and id == model.id then return model end
        end,
        has_configured_auth = function() return true end,
        is_using_oauth = function() return false end,
        refresh = function() end, get_error = function() return nil end,
      },
      wall_now_ms = pi.now_ms,
    }
    state.extension_ui = EXTENSION_UI_POLICY.context(state)
    local manager = pi.session.in_memory({ cwd = state.cwd })
    bind_session_runtime(state, manager)
    state.extension_action_handlers = {
      abort = function() state.agent:abort() end,
      shutdown = function() state.exit = true end,
      compact = function() end,
    }
    local context = EXTENSION_CONTEXT_POLICY.snapshot(state, { command = true })
    local found = context.modelRegistry.find(model.provider, model.id)
    local tool_result
    if request.tool then
      for _, tool in ipairs(pi.registered_tools()) do
        if tool.name == request.tool then
          tool_result = tool.execute("context-oracle", request.arguments or {}, nil, nil,
            EXTENSION_CONTEXT_POLICY.snapshot(state))
        end
      end
    else
      context.shutdown()
    end
    EXTENSION_CONTEXT_POLICY.pump(state)
    local usage = context.getContextUsage()
    local snapshot = {
      mode = context.mode, hasUI = context.hasUI, cwd = "{CWD}",
      trusted = context.isProjectTrusted(), idle = context.isIdle(),
      pending = context.hasPendingMessages(), hasSignal = context.signal ~= nil,
      model = { provider = context.model.provider, id = context.model.id },
      session = {
        persisted = context.sessionManager.is_persisted(), cwd = "{CWD}",
        entries = #context.sessionManager.get_entries(),
        branch = #context.sessionManager.get_branch(),
      },
      registryFound = found and { provider = found.provider, id = found.id } or nil,
      systemPromptHasCwd = context.getSystemPrompt():find(
        "Current working directory: " .. state.cwd, 1, true) ~= nil,
      systemPromptOptionsCwd = context.getSystemPromptOptions().cwd == state.cwd,
      usage = usage, waitForIdle = context.waitForIdle ~= nil,
      shutdowns = state.exit and 1 or 0,
    }
    state.extension_context_generation = state.extension_context_generation + 1
    local ok, stale = pcall(context.isIdle)
    local action_oracle = extension_context_action_oracle(model, state.cwd)
    return {
      snapshot = snapshot,
      stale = ok and "" or tostring(stale):match(":%d+: (.*)$") or tostring(stale),
      toolResult = tool_result,
      modes = action_oracle.modes,
      actions = action_oracle.actions,
      replacement = action_oracle.replacement,
      reload = action_oracle.reload,
    }
  end,
})


-- File-backed lifecycle exerciser driver. The command runs exactly as it does
-- from submit routing: background coroutine + process-loop action application.
pi.register_command("interactive-extension-action-behavior", {
  handler = function(args)
    local request = pi.json.decode(args)
    local state = create_interactive_state(request)
    local registered, observation_command
    for _, command in ipairs(pi.registered_extension_commands()) do
      if command.invocation_name == "session-lifecycle-demo" then registered = command end
      if command.invocation_name == "session-lifecycle-observation" then
        observation_command = command
      end
    end
    if not registered or not observation_command then
      error("session-lifecycle-demo extension is not loaded", 0)
    end
    local observation_context = EXTENSION_CONTEXT_POLICY.snapshot(state)
    for _, entry in ipairs(pi.extension_handlers("tool_call")) do
      local observed, observe_error = pcall(entry.handler, {
        type = "tool_call", toolCallId = "context-observation",
        toolName = "read", input = { path = "README.md" },
      }, observation_context)
      if not observed then error(entry.source .. ": " .. tostring(observe_error), 0) end
    end
    local context = EXTENSION_CONTEXT_POLICY.snapshot(state, { command = true })
    local event_context = observation_command.handler("", context)
    local task = pi.spawn(function()
      return registered.handler(pi.json.encode(request.lifecycleAction or {}), context)
    end)
    while not task:done() do
      EXTENSION_CONTEXT_POLICY.pump(state)
      EXTENSION_UI_POLICY.pump(state)
      pi.sleep(1)
    end
    EXTENSION_CONTEXT_POLICY.pump(state)
    local result = task:join()
    return {
      result = result, eventContext = event_context,
      currentSessionFile = state.session_manager:get_session_file(),
      currentSessionId = state.session_manager:get_session_id(),
      generation = state.extension_context_generation,
      status = state.last_status and state.last_status.text or nil,
    }
  end,
})
-- First queued extension-UI slice: real loaded commands/tool hooks drive the
-- shipped select→confirm→notify policy through the editor slot.
pi.register_command("interactive-extension-ui-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local theme = create_theme(dark_json, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local state = {
      theme = theme, md_theme = get_markdown_theme(theme), transcript = {},
      model = nil, thinking_level = "off", tools_expanded = false,
      header_expanded = false, steering_texts = {}, follow_up_texts = {},
      double_escape_action = "none", cwd = request.cwd or pi.cwd(),
      project_trusted = true, usage = {}, exit = false, scoped_models = {},
      extension_ui_actions = {}, extension_ui_active = nil, extension_ui_trace = {},
      session = {
        is_streaming = function() return false end,
        is_compacting = function() return false end,
        is_bash_running = function() return false end,
        clear_queues = function() end, steer = function() end,
        follow_up = function() end, abort = function() end,
      },
    }
    state.extension_ui = EXTENSION_UI_POLICY.context(state)
    state.set_editor_focus = function(focused)
      if state.editor then state.editor.editor:set_focused(focused) end
    end
    setup_shell_editor(state)
    state.editor.editor:set_terminal_rows(rows)
    local actions = shell_submit_actions(state)
    state.submit = function(text) handle_submit(text, actions) end

    local terminal = pi.tui.session(columns, rows, true)
    local frames, permission_result = {}, nil
    local function lines()
      local result = transcript_lines(state, columns)
      if state.selector then append(result, state.selector:render(columns))
      else sync_editor_border(state); append(result, state.editor.editor:render(columns)) end
      return result
    end
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(lines())
      frames[#frames + 1] = {
        name = name, columns = columns, rows = rows, ansi = terminal:output(),
      }
    end
    local function settle()
      pi.sleep(12)
      EXTENSION_UI_POLICY.pump(state)
    end
    local function feed(data)
      if state.selector then state.selector:handle_input(data)
      else
        local effect = state.editor:handle_input(data)
        if effect.kind == "submit" then state.submit(effect.text or effect.value or "") end
      end
      settle()
    end

    terminal:start()
    for _, step in ipairs(request.steps or {}) do
      if step.submit then
        feed("\27[200~" .. step.submit .. "\27[201~")
        feed("\r")
      end
      if step.permission then
        pi.spawn(function()
          permission_result = EXTENSION_POLICY.emit_tool_call({
            toolCall = { id = "permission", name = "bash" },
            args = { command = step.permission },
          }, {
            cwd = state.cwd, mode = "interactive", hasUI = true,
            isProjectTrusted = function() return true end,
            ui = state.extension_ui,
          })
        end)
        settle()
      end
      for _, input in ipairs(step.input or {}) do feed(input) end
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      capture(step.name, step.name == "startup" or step.resize ~= nil)
    end
    return { frames = frames, permissionResult = permission_result,
      actions = state.extension_ui_trace }
  end,
})


-- /trust editor-slot differential driver. Uses the shipped selector, command
-- route, and public trust store; unrelated transcript/footer rows are omitted.
pi.register_command("interactive-trust-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local theme = create_theme(dark_json, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    pi.trust.set(request.cwd, nil)
    local options = pi.trust.options(request.cwd, false)
    if options[2] and options[2].savedPath then pi.trust.set(options[2].savedPath, nil) end
    local state = {
      theme = theme, md_theme = get_markdown_theme(theme), transcript = {
        { kind = "warning_raw", text = "This project is not trusted. Project .pi resources and packages are ignored. Use /trust to save a trust decision, then restart pi." },
      },
      model = nil, thinking_level = "off", tools_expanded = false, header_expanded = false,
      steering_texts = {}, follow_up_texts = {}, double_escape_action = "none",
      cwd = request.cwd, project_trusted = false, now_ms = function() return 0 end,
      usage = {}, exit = false, scoped_models = {},
      registry = { has_configured_auth = function() return true end, get_available = function() return {} end },
      session = { is_streaming = function() return false end, clear_queues = function() end,
        steer = function() end, follow_up = function() end, abort = function() end,
        prompt = function() end },
    }
    state.set_editor_focus = function(focused)
      if state.editor then state.editor.editor:set_focused(focused) end
    end
    setup_shell_editor(state)
    state.editor.editor:set_terminal_rows(rows)
    local actions = shell_submit_actions(state)
    state.submit = function(text) handle_submit(text, actions) end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function lines()
      local result = transcript_lines(state, columns)
      if state.selector then append(result, state.selector:render(columns))
      else sync_editor_border(state); append(result, state.editor.editor:render(columns)) end
      return result
    end
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(lines())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    for _, step in ipairs(request.steps or {}) do
      if step.show then DEFAULT_KEYS.__trust_policy.show(state) end
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        if state.selector then state.selector:handle_input(input)
        else
          local effect = state.editor:handle_input(input)
          if effect.kind == "submit" then state.submit(effect.text or effect.value or "") end
        end
      end
      capture(step.name, step.name == "startup" or step.resize ~= nil)
    end
    return { frames = frames, saved = pi.trust.get_entry(request.cwd) }
  end,
})


-- Real-stack provider exerciser for differential terminal capture (PLAN
-- 4.2). Mirrors tests/ui-parity/pi-provider-turn.ts exactly: the product
-- interactive machinery (create_interactive_state — the real agent loop,
-- real registered tools, the real anthropic protocol against the
-- scenario's local SSE stub) with frames captured at exact agent-event
-- points from the subscribe seam, the same points the pi driver's awaited
-- listener captures at.
pi.register_command("interactive-provider-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local state = create_interactive_state(request)
    -- The pi driver pins the footer's provider count the same way.
    state.provider_count = request.providerCount or 1
    local columns, rows = request.columns, request.rows
    state.columns = columns
    state.editor.editor:set_terminal_rows(rows)
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frontend_frame(state, columns))
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    -- Event-triggered captures: each step arms triggers; the subscribe
    -- hook captures at a trigger's nth matching event and then runs its
    -- action (escape aborts the in-flight turn through the product
    -- handler). The provider stream cannot advance past a capture: the
    -- listener runs synchronously inside the agent coroutine.
    local triggers = {}
    state.event_hook = function(event)
      for _, trigger in ipairs(triggers) do
        if not trigger.fired and event.type == trigger.event
           and (trigger.role == nil or (event.message and event.message.role == trigger.role)) then
          trigger.seen = (trigger.seen or 0) + 1
          if trigger.seen >= (trigger.count or 1) then
            trigger.fired = true
            capture(trigger.name)
            if trigger.action == "escape" then handle_escape(state) end
          end
        end
      end
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.columns = columns
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      triggers = step.captures or {}
      for _, input in ipairs(step.input or {}) do
        local effect = state.editor:handle_input(input)
        if effect.kind == "submit" then
          state.submit(effect.text or effect.value or "")
        end
      end
      -- Settle the background turn; captures fire from the event hook.
      if state.turn then state.turn:join(); state.turn = nil end
      if step.name then capture(step.name, step.resize ~= nil) end
    end
    -- Restore outcomes for differential pinning (tests/resume_replay.rs);
    -- ui-diff compares frames only.
    return {
      frames = frames,
      model = { provider = state.model.provider, id = state.model.id },
      thinkingLevel = state.thinking_level,
    }
  end,
})

-- Real-stack bash-mode exerciser for differential terminal capture (PLAN
-- 7.1). Mirrors tests/ui-parity/pi-bash-turn.ts: the product machinery
-- (create_interactive_state — the real `!`/`!!` submit routing, the real
-- bash executor over pi.exec, and the real agent loop against the
-- scenario's SSE stub for the deferred-during-streaming section). Steps
-- gate on the held bash task (waitBash) or leave a hanging turn running
-- (waitIdle false) exactly where the pi driver awaits its promises.
pi.register_command("interactive-bash-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local state = create_interactive_state(request)
    state.provider_count = request.providerCount or 1
    local columns, rows = request.columns, request.rows
    state.columns = columns
    state.editor.editor:set_terminal_rows(rows)
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frontend_frame(state, columns))
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    -- Chunk-time renders: pi renders streamed bash output before the
    -- completion settles; the shrink's scroll effect must reach the
    -- emulated terminal.
    state.bash_chunk_hook = function()
      terminal:request_render(false)
      terminal:render(frontend_frame(state, columns))
    end
    local triggers = {}
    local function triggers_pending()
      for _, trigger in ipairs(triggers) do
        if not trigger.fired then return true end
      end
      return false
    end
    state.event_hook = function(event)
      for _, trigger in ipairs(triggers) do
        if not trigger.fired and event.type == trigger.event
           and (trigger.role == nil or (event.message and event.message.role == trigger.role)) then
          trigger.seen = (trigger.seen or 0) + 1
          if trigger.seen >= (trigger.count or 1) then
            trigger.fired = true
            capture(trigger.name)
            if trigger.action == "escape" then handle_escape(state) end
          end
        end
      end
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.columns = columns
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      triggers = step.captures or {}
      for _, input in ipairs(step.input or {}) do
        local effect = state.editor:handle_input(input)
        if effect.kind == "submit" then
          state.submit(effect.text or effect.value or "")
        end
      end
      -- The pi driver awaits the held bash promise before capturing;
      -- join runs the spawned executor to completion. Chunk-time renders
      -- land through bash_chunk_hook, so the terminal sees the same
      -- intermediate (still-running, output-present) frames pi renders —
      -- their scroll effects survive into the settled capture.
      if step.waitBash and state.bash_task then
        state.bash_task:join()
        state.bash_task = nil
      end
      if step.waitIdle == false then
        -- A hanging turn stays in flight; pump the LocalSet until the
        -- armed captures have fired (the pi driver's capture gates).
        local budget = 0
        while triggers_pending() and budget < 5000 do
          pi.sleep(1)
          budget = budget + 1
        end
      elseif state.turn then
        state.turn:join()
        state.turn = nil
      end
      if step.name then capture(step.name, step.resize ~= nil) end
    end
    return { frames = frames }
  end,
})

-- Scripted autocomplete exerciser for differential terminal capture.
-- Mirrors tests/ui-parity/pi-autocomplete-turn.ts exactly: the product
-- CustomEditor with the coding-agent editor/select-list theme and the
-- createBaseAutocompleteProvider wiring (builtin commands, /model argument
-- completions over pinned models, file paths from the scenario tree, @-fuzzy
-- via the scenario's fd stub). Requests are pumped synchronously; the pi
-- driver's debounce settles inside its capture waits.
pi.register_command("interactive-autocomplete-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local editor = custom_editor({ theme = theme })
    editor.editor:set_terminal_rows(rows)
    editor.editor:set_focused(true)
    local state = {
      cwd = request.cwd,
      fd_path = request.fdPath,
      scoped_models = {},
      registry = { get_available = function() return request.models or {} end },
    }
    local provider = create_base_autocomplete_provider(state)
    editor.editor:set_autocomplete_triggers({})
    local function pump()
      local pending = editor.editor:take_autocomplete_request()
      if pending then resolve_editor_autocomplete(editor.editor, provider, pending) end
    end
    local submitted = {}
    local function frame()
      local lines = {}
      for _, text in ipairs(submitted) do
        append(lines, pi.tui.text_render(theme:fg("dim", pi.json.encode(text)), columns, 0, 0))
      end
      for _, line in ipairs(editor.editor:render(columns)) do lines[#lines + 1] = line end
      return lines
    end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frame())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        local effect = editor:handle_input(input)
        pump()
        if effect.kind == "submit" then
          local text = trim(effect.text or "")
          if text ~= "" then
            submitted[#submitted + 1] = text
            editor.editor:add_to_history(text)
          end
        end
      end
      capture(step.name, step.resize ~= nil)
    end
    return { frames = frames }
  end,
})

-- Scripted tool-transcript exerciser for differential terminal capture.
-- Mirrors tests/ui-parity/pi-tool-turn.ts exactly: per section it mounts
-- ToolExecutionComponents with the real registered tool definitions and
-- captures pending → (partial →) results → expanded checkpoints. The
-- scripted clock stands in for Date.now so bash Elapsed/Took lines are
-- deterministic on both sides.
pi.register_command("interactive-tool-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local clock = 0
    local function now_ms() return clock end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local tool_rows = {}
    local expanded = false
    local function frame()
      local lines = {}
      for _, row in ipairs(tool_rows) do
        append(lines, tool_execution_lines(row, columns, theme, {
          cwd = request.cwd, expanded = expanded, now_ms = now_ms,
        }))
      end
      return lines
    end
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frame())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    local first = true
    for _, section in ipairs(request.sections or {}) do
      tool_rows = {}
      expanded = false
      local clocks = section.clocks or {}
      clock = clocks.pending or clock
      local has_partial = false
      for _, tool in ipairs(section.tools) do
        tool_rows[#tool_rows + 1] = {
          kind = "tool", toolCallId = tool.id, name = tool.name, args = tool.args,
          state = "pending", executionStarted = true, argsComplete = true,
          render_state = {},
        }
        if tool.partialResult then has_partial = true end
      end
      capture(section.name .. "-pending", first)
      first = false
      if has_partial then
        clock = clocks.partial or clock
        for index, tool in ipairs(section.tools) do
          if tool.partialResult then
            tool_rows[index].result = { content = tool.partialResult.content,
              details = tool.partialResult.details }
          end
        end
        capture(section.name .. "-partial")
      end
      clock = clocks.results or clock
      for index, tool in ipairs(section.tools) do
        if tool.result then
          tool_rows[index].result = { content = tool.result.content, details = tool.result.details }
          tool_rows[index].state = tool.result.isError and "error" or "success"
        end
      end
      capture(section.name .. "-results")
      expanded = true
      capture(section.name .. "-expanded")
    end
    return { frames = frames }
  end,
})

-- Scripted selector-overlay exerciser for differential terminal capture.
-- Mirrors tests/ui-parity/pi-selector-turn.ts exactly: a minimal focused
-- editor stand-in in the editor slot, the showSelector swap mounting the
-- oauth-selector template over it, filter/move/cancel input routed to the
-- focused component, and focus restore on done().
pi.register_command("interactive-selector-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local credentials = request.credentials or {}
    local auth_status = request.authStatus or {}
    local editor = custom_editor({ theme = theme })
    editor.editor:set_terminal_rows(rows)
    editor.editor:set_focused(true)
    local state
    state = {
      selector = nil,
      set_editor_focus = function(focused) editor.editor:set_focused(focused) end,
    }
    local events = {}
    local function open_selector()
      show_selector(state, function(done)
        return oauth_selector({
          mode = request.mode or "login",
          providers = request.providers or {},
          theme = theme,
          get_credential = function(id) return credentials[id] end,
          get_auth_status = function(id) return auth_status[id] end,
          on_select = function(id)
            events[#events + 1] = { type = "select", id = id }
            done()
          end,
          on_cancel = function()
            events[#events + 1] = { type = "cancel" }
            done()
          end,
        })
      end)
    end
    local function frame()
      local lines = {}
      if state.selector then
        append(lines, state.selector:render(columns))
      else
        for _, line in ipairs(editor.editor:render(columns)) do lines[#lines + 1] = line end
      end
      return lines
    end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frame())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.show then open_selector() end
      for _, input in ipairs(step.input or {}) do
        if state.selector then
          state.selector:handle_input(input)
        else
          editor:handle_input(input)
        end
      end
      capture(step.name)
    end
    return { frames = frames, events = events }
  end,
})

-- Scripted /login ⁄ /logout exerciser for differential terminal capture.
-- Mirrors tests/ui-parity/pi-login-turn.ts: the product wiring
-- (show_oauth_selector → selectors → login-dialog → completion) drives the
-- frames; the OAuth flow itself is a scripted handle whose events the
-- scenario emits between captures, exactly where pi's callbacks fire.
pi.register_command("interactive-login-parity-sequence", {
  handler = function(args)
    local CURSOR_MARKER = "\27_pi:c\7"
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local credentials = request.credentials or {}
    local auth_status = request.authStatus or {}
    local recorded = {}
    local editor_value = ""
    local editor_focused = true
    local current_handle = nil

    local function scripted_login_handle()
      local queue = {}
      local handle = {}
      function handle:push(event) queue[#queue + 1] = event end
      function handle:next_event(_timeout) return table.remove(queue, 1) end
      function handle:respond(text) recorded[#recorded + 1] = { type = "respond", value = text } end
      function handle:cancel() recorded[#recorded + 1] = { type = "cancel" } end
      return handle
    end

    local state
    state = {
      theme = theme,
      transcript = {},
      selector = nil,
      login = nil,
      docs_path = request.docsPath,
      model = request.model,
      open_browser = function(url) recorded[#recorded + 1] = { type = "open", value = url } end,
      set_editor_focus = function(focused) editor_focused = focused end,
      auth = {
        get = function(id) return credentials[id] end,
        get_auth_status = function(id) return auth_status[id] or { configured = false } end,
        list = function()
          local ids = {}
          for id in pairs(credentials) do ids[#ids + 1] = id end
          table.sort(ids)
          return ids
        end,
        set = function(id, credential) credentials[id] = credential end,
        remove = function(id) credentials[id] = nil end,
        oauth_providers = function() return request.oauthProviders or {} end,
        login_start = function(_id)
          current_handle = scripted_login_handle()
          return current_handle
        end,
        env_api_key = function(_id) return nil end,
        resolve_config_value = function(value) return value end,
        auth_path = request.authPath,
      },
      -- Scripted registry seam: the visual fixture pins frames, not
      -- registry behavior (the pi driver's refresh/count are no-ops too).
      registry = {
        refresh = function() end,
        get_error = function() return nil end,
        get_available = function() return {} end,
        find = function() return nil end,
        has_configured_auth = function() return false end,
        is_using_oauth = function() return false end,
      },
      -- Login option lists are scenario data (pi computes them from its
      -- full provider registry; provider breadth is the auth-compatibility
      -- milestone). Logout options run the product computation over the
      -- scripted auth seam on both sides.
      login_provider_options = function(auth_type)
        return (request.loginProviders or {})[auth_type] or {}
      end,
    }

    local function frame()
      local lines = {}
      append(lines, transcript_lines(state, columns))
      if state.selector then
        append(lines, state.selector:render(columns))
      else
        lines[#lines + 1] = theme:fg("accent", editor_value)
          .. (editor_focused and CURSOR_MARKER or "")
      end
      return lines
    end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frame())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.show then show_oauth_selector(state, step.show) end
      for _, event in ipairs(step.emit or {}) do
        if event.type == "done" and state.login then
          -- The host contract: login_start persists the credential
          -- before reporting done.
          credentials[state.login.provider_id] = { type = "oauth" }
        end
        if current_handle then current_handle:push(event) end
      end
      if step.emit then pump_login(state, 0) end
      for _, input in ipairs(step.input or {}) do
        if state.selector then
          state.selector:handle_input(input)
        elseif input == "\r" then
          editor_value = ""
        else
          editor_value = editor_value .. input
        end
      end
      capture(step.name)
    end
    return { frames = frames, recorded = recorded }
  end,
})

-- Headless behavior exerciser: routes /login ⁄ /logout through
-- handle_submit and the real pi.auth seam (tests register a stub OAuth
-- provider and point PI_CODING_AGENT_DIR at a scratch dir). Steps feed
-- key input to the mounted overlay; awaited pumps drain the real flow.
pi.register_command("interactive-login-flow", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local editor_focused = true
    local trace = {}
    local state
    state = {
      theme = theme,
      transcript = {},
      selector = nil,
      login = nil,
      docs_path = request.docsPath,
      model = request.model,
      open_browser = function(url) trace[#trace + 1] = { type = "open", value = url } end,
      set_editor_focus = function(focused) editor_focused = focused end,
      auth = default_auth_seam(),
      registry = default_registry_seam(),
    }
    for _, step in ipairs(request.steps or {}) do
      if step.submit then
        handle_submit(step.submit, {
          set_text = function(_value) end,
          quit = function() trace[#trace + 1] = { type = "quit" } end,
          prompt = function(value) trace[#trace + 1] = { type = "prompt", value = value } end,
          show_oauth_selector = function(mode) show_oauth_selector(state, mode) end,
        })
      end
      for _, input in ipairs(step.input or {}) do
        if state.selector then state.selector:handle_input(input) end
      end
      if step.pump then pump_login(state, step.pump) end
    end
    local rows = {}
    for _, row in ipairs(state.transcript) do
      rows[#rows + 1] = { kind = row.kind, text = row.text }
    end
    return {
      trace = trace,
      transcript = rows,
      overlay = state.selector ~= nil,
      editor_focused = editor_focused,
      providers = state.auth.list(),
    }
  end,
})

-- Scripted /model ⁄ model-selector exerciser for differential terminal
-- capture. Mirrors tests/ui-parity/pi-model-turn.ts: the product wiring
-- (handle_model_command → show_model_selector → session_set_model,
-- cycle_model) drives the frames over a scripted registry seam; the real
-- footer renders below the editor slot from the same scenario data.
pi.register_command("interactive-model-parity-sequence", {
  handler = function(args)
    local CURSOR_MARKER = "\27_pi:c\7"
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local models = request.models or {}
    local editor_value = ""
    local editor_focused = true

    local function find_model(provider, id)
      for _, model in ipairs(models) do
        if model.provider == provider and model.id == id then return model end
      end
      return nil
    end

    local state
    state = {
      theme = theme,
      transcript = {},
      selector = nil,
      model = request.model,
      scoped_models = {},
      provider_count = request.providerCount or 1,
      thinking_level = request.thinkingLevel or "off",
      set_editor_focus = function(focused) editor_focused = focused end,
      registry = {
        refresh = function() end,
        get_error = function() return request.registryError end,
        get_available = function() return models end,
        find = find_model,
        has_configured_auth = function(_model) return true end,
        is_using_oauth = function(_model) return request.subscription or false end,
      },
      auth = {
        get = function(_id) return nil end,
        env_api_key = function(_id) return nil end,
        resolve_config_value = function(value) return value end,
      },
    }

    local function footer_lines()
      local usage = request.usage or {}
      local context = request.contextUsage or {}
      return footer({
        width = columns, cwd = request.cwd, home = request.home,
        branch = request.branch, session_name = "",
        usage = { input = usage.input or 0, output = usage.output or 0,
          cache_read = usage.cacheRead or 0, cache_write = usage.cacheWrite or 0,
          cost = usage.cost or 0 },
        context_percent = context.percent, context_window = context.contextWindow or 0,
        auto_compact = true, model_id = state.model.id, provider = state.model.provider,
        provider_count = state.provider_count, reasoning = state.model.reasoning,
        thinking_level = state.thinking_level,
        subscription = request.subscription or false,
      }, theme)
    end

    local function frame()
      local lines = {}
      append(lines, transcript_lines(state, columns))
      if state.selector then
        append(lines, state.selector:render(columns))
      else
        lines[#lines + 1] = theme:fg("accent", editor_value)
          .. (editor_focused and CURSOR_MARKER or "")
      end
      append(lines, footer_lines())
      return lines
    end
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frame())
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.scoped ~= nil then
        if step.scoped then
          state.scoped_models = {}
          for _, ref in ipairs(request.scopedModels or {}) do
            state.scoped_models[#state.scoped_models + 1] = {
              model = find_model(ref.provider, ref.id),
              thinkingLevel = ref.thinkingLevel,
            }
          end
        else
          state.scoped_models = {}
        end
      end
      if step.show then show_model_selector(state, step.search) end
      if step.command ~= nil then handle_model_command(state, step.command) end
      if step.cycle then cycle_model(state, step.cycle) end
      for _, input in ipairs(step.input or {}) do
        if state.selector then
          state.selector:handle_input(input)
        elseif input == "\r" then
          editor_value = ""
        else
          editor_value = editor_value .. input
        end
      end
      capture(step.name)
    end
    return { frames = frames }
  end,
})

-- Headless behavior exerciser: routes /model through handle_submit and
-- the real pi.ai registry / pi.auth seams, drives the selector with key
-- input, and runs prompts through a scripted stream function so the test
-- can assert which model and API key the next provider request used.
pi.register_command("interactive-model-flow", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local editor_focused = true
    local requests = {}
    local state
    state = {
      theme = theme,
      transcript = {},
      selector = nil,
      model = request.model,
      scoped_models = {},
      thinking_level = "off",
      set_editor_focus = function(focused) editor_focused = focused end,
      auth = default_auth_seam(),
      registry = default_registry_seam(),
    }
    update_available_provider_count(state)
    local agent = pi.agent.new({
      initialState = { model = request.model, tools = {}, messages = {} },
      getApiKey = function(provider) return pi.auth.get_api_key(provider) end,
      streamFn = function(model, _context, options, _on_event)
        requests[#requests + 1] = {
          provider = model.provider, model = model.id, apiKey = options.apiKey,
        }
        return {
          role = "assistant", content = { { type = "text", text = "ok" } },
          api = model.api, provider = model.provider, model = model.id,
          usage = { input = 1, output = 1, cacheRead = 0, cacheWrite = 0,
            cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, total = 0 } },
          stopReason = "stop", timestamp = os.time() * 1000,
        }
      end,
    })
    state.agent = agent
    for _, step in ipairs(request.steps or {}) do
      if step.submit then
        handle_submit(step.submit, {
          set_text = function(_value) end,
          quit = function() end,
          prompt = function(value) agent:prompt(value) end,
          show_oauth_selector = function(mode) show_oauth_selector(state, mode) end,
          model_command = function(search) handle_model_command(state, search) end,
        })
      end
      if step.cycle then cycle_model(state, step.cycle) end
      for _, input in ipairs(step.input or {}) do
        if state.selector then state.selector:handle_input(input) end
      end
    end
    local rows = {}
    for _, row in ipairs(state.transcript) do
      rows[#rows + 1] = { kind = row.kind, text = row.text }
    end
    return {
      requests = requests,
      transcript = rows,
      overlay = state.selector ~= nil,
      editor_focused = editor_focused,
      model = { provider = state.model.provider, id = state.model.id },
      agent_model = agent:get_state().model.id,
      provider_count = state.provider_count,
    }
  end,
})

-- Session-UI exerciser for differential terminal capture (PLAN 6.3).
-- Mirrors tests/ui-parity/pi-session-turn.ts: the full product wiring
-- (create_interactive_state over the scenario's session-dir fixture) with
-- inputs routed exactly like run_interactive — the selector when mounted,
-- the editor otherwise (slash commands travel the handle_submit route).
-- Scripted tree-navigation and compaction exerciser for differential
-- terminal capture (PLAN 6.4/6.5). Mirrors tests/ui-parity/pi-tree-turn.ts
-- and pi-compaction-turn.ts: the product machinery
-- (create_interactive_state) over a restored session fixture; steps with
-- `settle = false` capture before the spawned task runs (the pi driver
-- holds its stub until the matching release step), so loader frames are
-- deterministic on both sides. `step.captures` arms provider-turn-style
-- event triggers: the subscribe/emit hook captures at a trigger's nth
-- matching event and runs its action — "escape" routes through
-- handle_escape (abort/abort-compaction), "submit" feeds text through
-- the editor + submit path (queueing during compaction) — all
-- synchronously inside the emitting coroutine, so no timing is involved.
pi.register_command("interactive-tree-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local state = create_interactive_state(request)
    state.registry = {
      refresh = function() end,
      get_error = function() return nil end,
      get_available = function() return { state.model } end,
      find = function() return nil end,
      has_configured_auth = function() return false end,
      is_using_oauth = function() return false end,
    }
    update_available_provider_count(state)
    local columns, rows = request.columns, request.rows
    state.columns = columns
    state.rows = rows
    state.editor.editor:set_terminal_rows(rows)
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frontend_frame(state, columns))
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    local function feed(input)
      if state.selector then
        state.selector:handle_input(input)
      else
        local effect = state.editor:handle_input(input)
        if effect.kind == "submit" then
          state.submit(effect.text or effect.value or "")
        end
      end
    end
    local triggers = {}
    state.event_hook = function(event)
      for _, trigger in ipairs(triggers) do
        if not trigger.fired and event.type == trigger.event
           and (trigger.role == nil or (event.message and event.message.role == trigger.role)) then
          trigger.seen = (trigger.seen or 0) + 1
          if trigger.seen >= (trigger.count or 1) then
            trigger.fired = true
            -- Submit acts before capture; Escape/countdown capture first.
            if trigger.action == "submit" then
              feed("\27[200~" .. (trigger.text or "") .. "\27[201~")
              feed("\r")
              if trigger.name then capture(trigger.name) end

            else
              if trigger.name then capture(trigger.name) end
              if trigger.action == "escape" then handle_escape(state) end
              if trigger.action == "countdown" then
                pi.sleep(trigger.afterMs or 1100)
                local countdown, now = state.retry_countdown, pi.monotonic_ms()
                if countdown then
                  while now >= countdown.next_ms and countdown.remaining > 0 do
                    countdown.remaining = countdown.remaining - 1
                    countdown.next_ms = countdown.next_ms + 1000
                  end
                  if state.loader and state.loader.kind == "retry" then
                    state.loader.message = countdown.message(countdown.remaining)
                  end
                  if countdown.remaining <= 0 then state.retry_countdown = nil end
                end
                if trigger.afterName then capture(trigger.afterName) end
              end
            end
          end
        end
      end
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.columns = columns
        state.rows = rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      if step.captures then triggers = step.captures end
      for _, input in ipairs(step.input or {}) do
        feed(input)
        -- The pi driver renders after every keystroke (ui.requestRender
        -- from the components); the hidden hardware cursor's resting
        -- position depends on those intermediate differential writes, so
        -- mirror them. The output accumulates into the step's capture,
        -- exactly like the pi driver's terminal.take() batches.
        terminal:request_render(false)
        terminal:render(frontend_frame(state, columns))
      end
      -- Settle successive background tasks: a compaction_end flush can
      -- spawn a follow-on prompt turn while the first join is running.
      while state.turn and step.settle ~= false do
        local turn = state.turn
        turn:join()
        if state.turn == turn then state.turn = nil end
      end
      if step.name then capture(step.name, step.resize ~= nil) end
    end
    return {
      frames = frames,
      sessionFile = state.session_manager:get_session_file(),
      leafId = state.session_manager:get_leaf_id(),
      editorText = state.editor.editor:get_text(),
      cwd = state.cwd,
    }
  end,
})

-- Differential seam for core/export-html/index.ts. It drives the shipped Lua
-- exporter with controlled AgentState metadata while the session itself still
-- crosses the public pi.session boundary.
pi.register_command("export-html-parity", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local manager = pi.session.open({ path = request.sessionFile })
    local agent_state = { systemPrompt = request.systemPrompt, tools = request.tools or {} }
    local state = {
      session_manager = manager,
      agent = { get_state = function() return agent_state end },
      theme = create_theme(data, request.colorMode or "truecolor"),
      theme_data = data, cwd = manager:get_cwd(), app_name = request.appName or "pi",
    }
    return { outputPath = export_html_lib.generate(state, request.outputPath) }
  end,
})


pi.register_command("interactive-session-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local state = create_interactive_state(request)
    state.copy_text = function(text) state.copied_text = text end
    -- The pi driver pins the footer's provider count; runtime rebinds
    -- recompute it, so pin through the registry seam (one scenario model
    -- → one provider).
    state.registry = {
      refresh = function() end,
      get_error = function() return nil end,
      get_available = function() return { state.model } end,
      find = function() return nil end,
      has_configured_auth = function() return false end,
      is_using_oauth = function() return false end,
    }
    update_available_provider_count(state)
    local columns, rows = request.columns, request.rows
    state.columns = columns
    state.rows = rows
    state.editor.editor:set_terminal_rows(rows)
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    local function capture(name, force)
      terminal:request_render(force or false)
      terminal:render(frontend_frame(state, columns))
      frames[#frames + 1] = { name = name, columns = columns, rows = rows, ansi = terminal:output() }
    end
    terminal:start()
    capture("startup", true)
    for _, step in ipairs(request.steps or {}) do
      if step.resize then
        columns, rows = step.resize.columns, step.resize.rows
        state.columns = columns
        state.rows = rows
        state.editor.editor:set_terminal_rows(rows)
        terminal:resize(columns, rows)
      end
      for _, input in ipairs(step.input or {}) do
        if state.selector then
          state.selector:handle_input(input)
        else
          local effect = state.editor:handle_input(input)
          if effect.kind == "submit" then
            state.submit(effect.text or effect.value or "")
          end
        end
      end
      if step.sleepMs then pi.sleep(step.sleepMs) end
      -- Settle any background turn (prompts run as spawned coroutines).
      if state.turn then state.turn:join(); state.turn = nil end
      if step.name then capture(step.name, step.resize ~= nil) end
    end
    -- Switch outcomes for differential pinning (tests/interactive_session.rs);
    -- ui-diff compares frames only.
    return {
      frames = frames,
      sessionFile = state.session_manager:get_session_file(),
      cwd = state.cwd,
      copiedText = state.copied_text,
    }
  end,
})

-- Startup changelog differential driver. Product policy is
-- mount_startup_changelog; this command only scripts its stable frames.
pi.register_command("interactive-startup-changelog-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    terminal:start()
    for _, step in ipairs(request.steps or {}) do
      local state = {
        theme = theme, md_theme = get_markdown_theme(theme), transcript = {},
        version = request.version, request = request, app_name = request.appName or "pi",
      }
      local result
      if step.release then
        mount_update_notification(state, step.release)
        result = { displayed = true, release = step.release.version }
      else
        local messages = {}
        if step.resumed then messages[1] = { role = "user", content = "existing" } end
        result = mount_startup_changelog(state, {
          messages = messages, last_version = step.lastVersion, fresh = step.fresh,
          markdown = request.changelogText, version = request.version,
          collapsed = step.collapsed, no_persist = true,
        })
      end
      terminal:request_render(step.force or false)
      terminal:render(transcript_lines(state, columns))
      frames[#frames + 1] = {
        name = step.name, columns = columns, rows = rows, ansi = terminal:output(),
        result = result,
      }
    end
    return { frames = frames }
  end,
})

-- Deterministic network-policy exerciser: drives the same telemetry/version
-- functions as create_interactive_state against caller-supplied loopback URLs.
pi.register_command("startup-network-parity", {
  handler = function(args)
    local request = pi.json.decode(args)
    local state = {
      request = request, version = request.version, app_name = request.appName or "pi",
      transcript = {},
    }

    local telemetry = report_install_telemetry(state, state.version)
    local version_check = start_version_check(state)
    if telemetry and telemetry ~= false then telemetry:join() end
    local release = version_check and version_check:join() or nil
    return {
      release = release,
      transcript = state.transcript,
      comparisons = {
        newer = is_newer_package_version("1.2.4", "1.2.3"),
        equal = is_newer_package_version("1.2.3", "1.2.3"),
        prerelease = is_newer_package_version("1.2.3", "1.2.3-beta.1"),
        fallback = is_newer_package_version("next", "current"),
      },
    }
  end,
})


-- Deterministic /reload UI driver. It invokes the product handler with only
-- the runtime reload operation injected; guards, box mounting, settlement,
-- and status/error presentation are the shipped policy.
pi.register_command("interactive-reload-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local terminal = pi.tui.session(columns, rows, true)
    local frames = {}
    terminal:start()

    for _, step in ipairs(request.steps or {}) do
      local state = {
        theme = theme, md_theme = get_markdown_theme(theme), transcript = {},
        request = { reloadYieldMs = step.delayMs or 1 }, editor = {},
        set_editor_focus = function(_) end,
        session = {
          is_streaming = function() return step.streaming or false end,
          is_compacting = function() return step.compacting or false end,
        },
        reload_impl = function()
          if step.fail then error("scripted reload failure", 0) end
        end,
      }
      local task = handle_reload_command(state)
      if step.phase ~= "loading" and task then task:join() end
      local lines = transcript_lines(state, columns)
      if state.reload_box then append(lines, state.reload_box:render(columns)) end
      terminal:request_render(step.force or false)
      terminal:render(lines)
      frames[#frames + 1] = {
        name = step.name, columns = columns, rows = rows, ansi = terminal:output(),
      }
      if step.phase == "loading" and task then task:join() end
    end
    return { frames = frames }
  end,
})


-- Headless reload behavior exerciser: create the real interactive runtime,
-- mutate fixture files as an external editor would, then route `/reload`.
pi.register_command("interactive-reload-behavior", {
  handler = function(args)
    local request = pi.json.decode(args)
    local state = create_interactive_state(request)
    local before = state.agent:get_state().systemPrompt
    if request.contextPath and request.contextAfter then
      pi.fs.write_file(request.contextPath, request.contextAfter)
    end
    if request.settingsPath and request.settingsAfter then
      pi.fs.write_file(request.settingsPath, request.settingsAfter)
    end
    local task = handle_reload_command(state)
    if task then task:join() end
    return {
      before = before,
      after = state.agent:get_state().systemPrompt,
      theme = state.theme.name,
      hideThinking = state.hide_thinking_block,
      status = state.last_status and state.last_status.text or nil,
      failed = state.transcript[#state.transcript]
        and state.transcript[#state.transcript].kind == "error" or false,
    }
  end,
})


-- PLAN 7.10 differential driver. Classification calls the exact policy helper;
-- run cases enter through the real interactive AgentSession runtime with a
-- scripted public streamFn and retain only stable retry/event/context fields.
pi.register_command("retry-policy-parity", {
  handler = function(args)
    local request = pi.json.decode(args)
    local case = request.case
    if case.mode == "classify" then
      return { retryable = retry_policy_is_retryable(request.model, case.message) }
    end

    request.retryParityTurns = case.turns
    request.cwd = request.cwd or pi.cwd()
    request.version = request.version or "0.79.0"
    request.thinkingLevel = "off"
    request.modelFromCli = true
    request.thinkingFromCli = true
    request.runtimeApiKey = "retry-oracle-key"
    request.apiKey = "retry-oracle-key"
    local state = create_interactive_state(request)
    local events = {}
    local cancelled, queued_on_retry = false, false
    local function stable_message(message)
      if not message then return nil end
      local text = ""
      for _, block in ipairs(message.content or {}) do
        if block.type == "text" then text = text .. (block.text or "") end
      end
      return { role = message.role, text = text, stopReason = message.stopReason,
        errorMessage = message.errorMessage }
    end
    state.event_hook = function(event)
      if event.type == "agent_end" then
        events[#events + 1] = { type = event.type, willRetry = event.willRetry or false }
        if case.queueOnRetry and event.willRetry and not queued_on_retry then
          queued_on_retry = true
          state.session.follow_up(case.queueOnRetry)
        end
      elseif event.type == "auto_retry_start" then
        events[#events + 1] = { type = event.type, attempt = event.attempt,
          maxAttempts = event.maxAttempts, delayMs = event.delayMs,
          errorMessage = event.errorMessage }
        if case.cancelAttempt == event.attempt and not cancelled then
          cancelled = true
          state.session.abort_retry()
        end
      elseif event.type == "auto_retry_end" then
        events[#events + 1] = { type = event.type, success = event.success,
          attempt = event.attempt, finalError = event.finalError }
      elseif event.type == "message_end" and event.message
          and event.message.role == "assistant" then
        local stable = stable_message(event.message)
        stable.type = event.type
        events[#events + 1] = stable
      end
    end
    state.session.prompt(case.prompt or "test")
    if state.turn then state.turn:join() end
    local final = {}
    for _, message in ipairs(state.agent:get_state().messages or {}) do
      final[#final + 1] = stable_message(message)
    end
    local contexts = {}
    for _, context in ipairs((request.__retry_recorder or {}).requests or {}) do
      local stable = {}
      for _, message in ipairs(context) do stable[#stable + 1] = stable_message(message) end
      contexts[#contexts + 1] = stable
    end
    return { events = events, callCount = #contexts, contexts = contexts, messages = final }
  end,
})

-- Deterministic hidden-easter-egg UI driver. It advances the shipped component
-- state directly, replacing only wall-clock timers and Armin's random choice.
pi.register_command("interactive-easter-eggs-parity-sequence", {
  handler = function(args)
    local request = pi.json.decode(args)
    local data = request.theme == "light" and light_json or dark_json
    local theme = create_theme(data, request.colorMode or "truecolor")
    local columns, rows = request.columns, request.rows
    local terminal = pi.tui.session(columns, rows, true)
    local frames, armin_ticks = {}, 0
    local armin = new_armin_row("typewriter", function() return 0 end)
    terminal:start()
    for _, step in ipairs(request.steps or {}) do
      local transcript
      if step.kind == "armin" then
        if step.final then while not tick_armin(armin) do armin_ticks = armin_ticks + 1 end
        else
          while armin_ticks < (step.tick or 0) do
            tick_armin(armin)
            armin_ticks = armin_ticks + 1
          end
        end
        transcript = { { kind = "spacer" }, armin }
      elseif step.kind == "daxnuts" then
        transcript = { { kind = "spacer" }, { kind = "daxnuts", tick = step.tick or 0 } }
      else transcript = { { kind = "spacer" }, { kind = "earendil" } } end
      terminal:request_render(step.force or false)
      terminal:render(transcript_lines({ transcript = transcript, theme = theme,
        md_theme = get_markdown_theme(theme) }, columns))
      frames[#frames + 1] = {
        name = step.name, columns = columns, rows = rows, ansi = terminal:output(),
      }
    end
    return { frames = frames }
  end,
})
