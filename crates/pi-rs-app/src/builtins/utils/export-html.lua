-- core/export-html: HTML session export policy. The template, client, CSS, and
-- vendored browser libraries are injected by the embedded pack from Pi v0.79.0.
do
local pi = ...
local export_html = {}

local COLOR_ORDER = {
  "accent", "border", "borderAccent", "borderMuted", "success", "error", "warning",
  "muted", "dim", "text", "thinkingText", "selectedBg", "userMessageBg",
  "userMessageText", "customMessageBg", "customMessageText", "customMessageLabel",
  "toolPendingBg", "toolSuccessBg", "toolErrorBg", "toolTitle", "toolOutput",
  "mdHeading", "mdLink", "mdLinkUrl", "mdCode", "mdCodeBlock", "mdCodeBlockBorder",
  "mdQuote", "mdQuoteBorder", "mdHr", "mdListBullet", "toolDiffAdded",
  "toolDiffRemoved", "toolDiffContext", "syntaxComment", "syntaxKeyword",
  "syntaxFunction", "syntaxVariable", "syntaxString", "syntaxNumber", "syntaxType",
  "syntaxOperator", "syntaxPunctuation", "thinkingOff", "thinkingMinimal", "thinkingLow",
  "thinkingMedium", "thinkingHigh", "thinkingXhigh", "bashMode",
}

local function resolved_colors(theme_data)
  local result, vars = {}, theme_data.vars or {}
  for _, key in ipairs(COLOR_ORDER) do
    local value = theme_data.colors[key]
    local seen = {}
    while type(value) == "string" and value ~= "" and value:sub(1, 1) ~= "#" do
      if seen[value] or vars[value] == nil then break end
      seen[value], value = true, vars[value]
    end
    result[key] = value
  end
  return result
end

local function parse_hex(color)
  local hex = type(color) == "string" and color:match("^#(%x%x%x%x%x%x)$")
  if not hex then return nil end
  return tonumber(hex:sub(1, 2), 16), tonumber(hex:sub(3, 4), 16), tonumber(hex:sub(5, 6), 16)
end

local function luminance(r, g, b)
  local function linear(c)
    local s = c / 255
    return s <= 0.03928 and s / 12.92 or ((s + 0.055) / 1.055) ^ 2.4
  end
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
end

local function adjust_brightness(color, factor)
  local r, g, b = parse_hex(color)
  if not r then return color end
  local function adjust(c) return math.min(255, math.max(0, math.floor(c * factor + 0.5))) end
  return ("rgb(%d, %d, %d)"):format(adjust(r), adjust(g), adjust(b))
end

local function derived_export_colors(base)
  local r, g, b = parse_hex(base)
  if not r then return { pageBg = "rgb(24, 24, 30)", cardBg = "rgb(30, 30, 36)", infoBg = "rgb(60, 55, 40)" } end
  if luminance(r, g, b) > 0.5 then
    return {
      pageBg = adjust_brightness(base, 0.96), cardBg = base,
      infoBg = ("rgb(%d, %d, %d)"):format(math.min(255, r + 10), math.min(255, g + 5), math.max(0, b - 20)),
    }
  end
  return {
    pageBg = adjust_brightness(base, 0.7), cardBg = adjust_brightness(base, 0.85),
    infoBg = ("rgb(%d, %d, %d)"):format(math.min(255, r + 20), math.min(255, g + 15), b),
  }
end

local function base64_encode(value)
  local alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
  local out = {}
  for i = 1, #value, 3 do
    local a, b, c = value:byte(i, i + 2)
    local n = a * 65536 + (b or 0) * 256 + (c or 0)
    out[#out + 1] = alphabet:sub(math.floor(n / 262144) % 64 + 1, math.floor(n / 262144) % 64 + 1)
    out[#out + 1] = alphabet:sub(math.floor(n / 4096) % 64 + 1, math.floor(n / 4096) % 64 + 1)
    out[#out + 1] = b and alphabet:sub(math.floor(n / 64) % 64 + 1, math.floor(n / 64) % 64 + 1) or "="
    out[#out + 1] = c and alphabet:sub(n % 64 + 1, n % 64 + 1) or "="
  end
  return table.concat(out)
end

local ANSI_COLORS = {
  "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
  "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
}

local function color256(index)
  if index < 16 then return ANSI_COLORS[index + 1] end
  if index < 232 then
    local cube = index - 16
    local function component(n) return n == 0 and 0 or 55 + n * 40 end
    return ("#%02x%02x%02x"):format(component(math.floor(cube / 36)), component(math.floor(cube % 36 / 6)), component(cube % 6))
  end
  local gray = 8 + (index - 232) * 10
  return ("#%02x%02x%02x"):format(gray, gray, gray)
end

local function escape_html(text)
  return (text:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;"):gsub('"', "&quot;"):gsub("'", "&#039;"))
end

local function ansi_to_html(text)
  local style = { bold = false, dim = false, italic = false, underline = false }
  local result, offset, open = {}, 1, false
  local function close_span() if open then result[#result + 1], open = "</span>", false end end
  local function open_span()
    local css = {}
    if style.fg then css[#css + 1] = "color:" .. style.fg end
    if style.bg then css[#css + 1] = "background-color:" .. style.bg end
    if style.bold then css[#css + 1] = "font-weight:bold" end
    if style.dim then css[#css + 1] = "opacity:0.6" end
    if style.italic then css[#css + 1] = "font-style:italic" end
    if style.underline then css[#css + 1] = "text-decoration:underline" end
    if #css > 0 then result[#result + 1], open = '<span style="' .. table.concat(css, ";") .. '">', true end
  end
  while true do
    local first, last, params = text:find("\27%[([%d;]*)m", offset)
    if not first then break end
    result[#result + 1] = escape_html(text:sub(offset, first - 1))
    close_span()
    local codes = {}
    if params == "" then codes[1] = 0 else for code in params:gmatch("[^;]+") do codes[#codes + 1] = tonumber(code) or 0 end end
    local i = 1
    while i <= #codes do
      local code = codes[i]
      if code == 0 then style = { bold = false, dim = false, italic = false, underline = false }
      elseif code == 1 then style.bold = true elseif code == 2 then style.dim = true
      elseif code == 3 then style.italic = true elseif code == 4 then style.underline = true
      elseif code == 22 then style.bold, style.dim = false, false elseif code == 23 then style.italic = false
      elseif code == 24 then style.underline = false elseif code >= 30 and code <= 37 then style.fg = ANSI_COLORS[code - 29]
      elseif code == 39 then style.fg = nil elseif code >= 40 and code <= 47 then style.bg = ANSI_COLORS[code - 39]
      elseif code == 49 then style.bg = nil elseif code >= 90 and code <= 97 then style.fg = ANSI_COLORS[code - 81]
      elseif code >= 100 and code <= 107 then style.bg = ANSI_COLORS[code - 91]
      elseif (code == 38 or code == 48) and codes[i + 1] == 5 and codes[i + 2] then
        if code == 38 then style.fg = color256(codes[i + 2]) else style.bg = color256(codes[i + 2]) end; i = i + 2
      elseif (code == 38 or code == 48) and codes[i + 1] == 2 and codes[i + 4] then
        local color = ("rgb(%d,%d,%d)"):format(codes[i + 2], codes[i + 3], codes[i + 4])
        if code == 38 then style.fg = color else style.bg = color end; i = i + 4
      end
      i = i + 1
    end
    open_span(); offset = last + 1
  end
  result[#result + 1] = escape_html(text:sub(offset)); close_span()
  return table.concat(result)
end

local function ansi_lines_to_html(lines)
  local out = {}
  for _, line in ipairs(lines) do
    local html = ansi_to_html(line)
    out[#out + 1] = '<div class="ansi-line">' .. (html == "" and "&nbsp;" or html) .. "</div>"
  end
  return table.concat(out)
end

local function blank_ansi(line) return (line:gsub("\27%[[%d;]*m", ""):match("^%s*$")) ~= nil end
local function trim_rendered_lines(lines)
  local first, last = 1, #lines
  while first <= last and blank_ansi(lines[first]) do first = first + 1 end
  while last >= first and blank_ansi(lines[last]) do last = last - 1 end
  local result = {}; for i = first, last do result[#result + 1] = lines[i] end
  return result
end

local TEMPLATE_TOOLS = { bash = true, read = true, write = true, edit = true, ls = true }
local function pre_render_tools(entries, theme, cwd)
  local rendered, definitions, states, args = {}, {}, {}, {}
  for _, definition in ipairs(pi.registered_tools()) do definitions[definition.name] = definition end
  local function context(id, last, expanded, partial, is_error)
    states[id] = states[id] or {}
    return { args = args[id], toolCallId = id, invalidate = function() end, lastComponent = last,
      state = states[id], cwd = cwd, executionStarted = true, argsComplete = true,
      isPartial = partial, expanded = expanded, showImages = false, isError = is_error }
  end
  for _, entry in ipairs(entries) do
    if entry.type == "message" then
      local message = entry.message
      if message.role == "assistant" and type(message.content) == "table" then
        for _, block in ipairs(message.content) do
          if block.type == "toolCall" and not TEMPLATE_TOOLS[block.name] then
            local definition = definitions[block.name]
            if definition and definition.renderCall then
              args[block.id] = block.arguments
              local ok, component = pcall(definition.renderCall, block.arguments, theme, context(block.id, nil, false, true, false))
              if ok and component then rendered[block.id] = { callHtml = ansi_lines_to_html(component(100)) } end
            end
          end
        end
      elseif message.role == "toolResult" and message.toolCallId then
        local definition, existing = definitions[message.toolName or ""], rendered[message.toolCallId]
        if (existing or not TEMPLATE_TOOLS[message.toolName or ""]) and definition and definition.renderResult then
          local value = { content = message.content, details = message.details, isError = message.isError or false }
          local ok1, collapsed = pcall(definition.renderResult, value, { expanded = false, isPartial = false }, theme,
            context(message.toolCallId, nil, false, false, message.isError or false))
          local ok2, expanded = pcall(definition.renderResult, value, { expanded = true, isPartial = false }, theme,
            context(message.toolCallId, ok1 and collapsed or nil, true, false, message.isError or false))
          if ok2 and expanded then
            existing = existing or {}
            local expanded_html = ansi_lines_to_html(trim_rendered_lines(expanded(100)))
            if ok1 and collapsed then
              local collapsed_html = ansi_lines_to_html(trim_rendered_lines(collapsed(100)))
              if collapsed_html ~= expanded_html then existing.resultHtmlCollapsed = collapsed_html end
            end
            existing.resultHtmlExpanded = expanded_html; rendered[message.toolCallId] = existing
          end
        end
      end
    end
  end
  return next(rendered) and rendered or nil
end

local function replace_literal(value, marker, replacement)
  local first, last = value:find(marker, 1, true)
  if not first then return value end
  local prefix, suffix, expanded, index = value:sub(1, first - 1), value:sub(last + 1), {}, 1
  -- JavaScript String.replace replacement-string substitutions. Pi passes
  -- vendored JS as the replacement string, so its literal `$&`, `$`` and
  -- `$$` sequences intentionally acquire native replace semantics.
  while index <= #replacement do
    local dollar = replacement:find("$", index, true)
    if not dollar then expanded[#expanded + 1] = replacement:sub(index); break end
    expanded[#expanded + 1] = replacement:sub(index, dollar - 1)
    local token = replacement:sub(dollar + 1, dollar + 1)
    if token == "$" then expanded[#expanded + 1], index = "$", dollar + 2
    elseif token == "&" then expanded[#expanded + 1], index = marker, dollar + 2
    elseif token == "`" then expanded[#expanded + 1], index = prefix, dollar + 2
    elseif token == "'" then expanded[#expanded + 1], index = suffix, dollar + 2
    else expanded[#expanded + 1], index = "$", dollar + 1 end
  end
  return prefix .. table.concat(expanded) .. suffix
end

function export_html.generate(state, output_path)
  local manager, session_file = state.session_manager, state.session_manager:get_session_file()
  if not session_file then error("Cannot export in-memory session to HTML", 0) end
  if not pi.fs.exists(session_file) then error("Nothing to export yet - start a conversation first", 0) end

  local entries, agent_state = manager:get_entries(), state.agent:get_state()
  local tools = {}
  for _, tool in ipairs(agent_state.tools or {}) do
    local exported_tool = pi.json.decode('{"name":null,"description":null,"parameters":null}')
    exported_tool.name, exported_tool.description, exported_tool.parameters = tool.name, tool.description, tool.parameters
    tools[#tools + 1] = exported_tool
  end
  -- Seed from JSON so the bridge retains JavaScript object construction
  -- order and explicit null slots through JSON.stringify.
  local session_data = pi.json.decode('{"header":null,"entries":null,"leafId":null,"systemPrompt":null,"tools":null}')
  session_data.header, session_data.entries, session_data.leafId = manager:get_header(), entries, manager:get_leaf_id()
  session_data.systemPrompt, session_data.tools = agent_state.systemPrompt, tools
  session_data.renderedTools = pre_render_tools(entries, state.theme, state.cwd)
  local encoded = base64_encode(pi.json.encode(session_data, false))
  local colors = resolved_colors(state.theme_data)
  local derived = derived_export_colors(colors.userMessageBg or "#343541")
  local export = state.theme_data.export or {}
  local vars = {}
  for _, key in ipairs(COLOR_ORDER) do vars[#vars + 1] = "--" .. key .. ": " .. tostring(colors[key]) .. ";" end
  vars[#vars + 1] = "--exportPageBg: " .. (export.pageBg or derived.pageBg) .. ";"
  vars[#vars + 1] = "--exportCardBg: " .. (export.cardBg or derived.cardBg) .. ";"
  vars[#vars + 1] = "--exportInfoBg: " .. (export.infoBg or derived.infoBg) .. ";"
  local css = replace_literal(EXPORT_TEMPLATE_CSS, "{{THEME_VARS}}", table.concat(vars, "\n      "))
  css = replace_literal(css, "{{BODY_BG}}", export.pageBg or derived.pageBg)
  css = replace_literal(css, "{{CONTAINER_BG}}", export.cardBg or derived.cardBg)
  css = replace_literal(css, "{{INFO_BG}}", export.infoBg or derived.infoBg)
  local html = replace_literal(EXPORT_TEMPLATE_HTML, "{{CSS}}", css)
  html = replace_literal(html, "{{JS}}", EXPORT_TEMPLATE_JS)
  html = replace_literal(html, "{{SESSION_DATA}}", encoded)
  html = replace_literal(html, "{{MARKED_JS}}", EXPORT_MARKED_JS)
  html = replace_literal(html, "{{HIGHLIGHT_JS}}", EXPORT_HIGHLIGHT_JS)

  if not output_path then
    output_path = state.app_name .. "-session-" .. pi.path.basename(session_file, ".jsonl") .. ".html"
  else output_path = pi.path.normalize(output_path) end
  pi.fs.write_file(output_path, html)
  return output_path
end

export_html_lib = export_html
end
