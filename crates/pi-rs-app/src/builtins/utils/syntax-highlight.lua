-- utils/syntax-highlight.ts + the utils/html.ts entity slice it imports,
-- plus the theme.ts syntax-highlighting pair (buildCliHighlightTheme /
-- highlightCode and getMarkdownTheme's highlightCode variant). The
-- highlight.js 10.7.3 engine itself is the Rust mechanism behind
-- `pi.hljs` (grammar catalog: crates/pi-rs-host/data/hljs-grammars.json);
-- everything Pi wrote on top of the library lives here.
--
-- Shared fragment: included by both the tools pack (read/write renderers)
-- and the interactive pack (markdown code blocks), so it only assumes the
-- chunk argument.
local pi = ...

-- JS String.prototype.split("\n") (plain, keeps empty segments).
local function hl_split_lines(s)
  local out = {}
  local start = 1
  while true do
    local i = s:find("\n", start, true)
    if not i then
      out[#out + 1] = s:sub(start)
      return out
    end
    out[#out + 1] = s:sub(start, i - 1)
    start = i + 1
  end
end

-- utils/html.ts decodeHtmlEntity. Number.parseInt parses a leading digit
-- prefix; the engine only emits well-formed entities, so prefix parsing
-- only matters for hand-written input.
local function decode_code_point(code_point)
  if code_point == nil or code_point < 0 or code_point > 0x10FFFF then return nil end
  return utf8.char(code_point)
end

local function decode_html_entity(entity)
  if entity == "amp" then return "&" end
  if entity == "lt" then return "<" end
  if entity == "gt" then return ">" end
  if entity == "quot" then return '"' end
  if entity == "apos" then return "'" end
  local head = entity:sub(1, 2)
  if head == "#x" or head == "#X" then
    local digits = entity:sub(3):match("^%x+")
    return digits and decode_code_point(tonumber(digits, 16)) or nil
  end
  if entity:sub(1, 1) == "#" then
    local digits = entity:sub(2):match("^%d+")
    return digits and decode_code_point(tonumber(digits, 10)) or nil
  end
  return nil
end

-- utils/html.ts decodeHtmlEntityAt(html, index) -> text, length.
local function decode_html_entity_at(html, index)
  local semicolon = html:find(";", index + 1, true)
  if not semicolon or semicolon - index > 16 then return nil end
  local decoded = decode_html_entity(html:sub(index + 1, semicolon - 1))
  if decoded == nil then return nil end
  return decoded, semicolon - index + 1
end

-- syntax-highlight.ts getScopeFromSpanTag. Engine output always uses
-- double-quoted class attributes.
local function get_scope_from_span_tag(tag)
  local class_value = tag:match("%sclass%s*=%s*\"([^\"]*)\"") or tag:match("%sclass%s*=%s*'([^']*)'")
  if not class_value or class_value == "" then return nil end
  for class_name in class_value:gmatch("%S+") do
    if class_name:sub(1, 5) == "hljs-" then return class_name:sub(6) end
  end
  return nil
end

-- syntax-highlight.ts getScopeFormatter: exact scope, then the prefix
-- before the first '.' or '-'.
local function get_scope_formatter(scope, theme)
  local exact = theme[scope]
  if exact then return exact end
  local dot = scope:find(".", 1, true)
  if dot then
    local formatter = theme[scope:sub(1, dot - 1)]
    if formatter then return formatter end
  end
  local dash = scope:find("-", 1, true)
  if dash then
    local formatter = theme[scope:sub(1, dash - 1)]
    if formatter then return formatter end
  end
  return nil
end

-- syntax-highlight.ts getActiveFormatter. Unscoped spans (sublanguage
-- wrappers) sit on the stack as `false` so parent formatting shows through.
local function get_active_formatter(scopes, theme)
  for i = #scopes, 1, -1 do
    local scope = scopes[i]
    if scope then
      local formatter = get_scope_formatter(scope, theme)
      if formatter then return formatter end
    end
  end
  return theme.default
end

local function is_span_open_tag_start(html, index)
  if html:sub(index, index + 4) ~= "<span" then return false end
  local next_char = html:sub(index + 5, index + 5)
  return next_char == ">" or next_char == " " or next_char == "\t"
    or next_char == "\n" or next_char == "\r"
end

-- syntax-highlight.ts renderHighlightedHtml: walk highlight.js HTML,
-- applying the innermost themed scope to each text run.
local function render_highlighted_html(html, theme)
  theme = theme or {}
  local output = {}
  local text_buffer = {}
  local scopes = {}

  local function flush_text()
    if #text_buffer == 0 then return end
    local text = table.concat(text_buffer)
    text_buffer = {}
    local formatter = get_active_formatter(scopes, theme)
    output[#output + 1] = formatter and formatter(text) or text
  end

  local index = 1
  local length = #html
  while index <= length do
    local handled = false
    if is_span_open_tag_start(html, index) then
      local tag_end = html:find(">", index + 5, true)
      if tag_end then
        flush_text()
        scopes[#scopes + 1] = get_scope_from_span_tag(html:sub(index, tag_end)) or false
        index = tag_end + 1
        handled = true
      end
    end
    if not handled and html:sub(index, index + 6) == "</span>" then
      flush_text()
      if #scopes > 0 then scopes[#scopes] = nil end
      index = index + 7
      handled = true
    end
    if not handled and html:sub(index, index) == "&" then
      local text, entity_length = decode_html_entity_at(html, index)
      if text then
        text_buffer[#text_buffer + 1] = text
        index = index + entity_length
        handled = true
      end
    end
    if not handled then
      text_buffer[#text_buffer + 1] = html:sub(index, index)
      index = index + 1
    end
  end

  flush_text()
  return table.concat(output)
end

-- syntax-highlight.ts highlight(code, options): the library call plus the
-- themed render. Raises on an unknown language, exactly like hljs.
local function syntax_highlight(code, options)
  options = options or {}
  local result
  if options.language then
    result = pi.hljs.highlight(code, {
      language = options.language,
      ignore_illegals = options.ignore_illegals,
    })
  else
    result = pi.hljs.highlight(code, { language_subset = options.language_subset })
  end
  return render_highlighted_html(result.value, options.theme)
end

-- theme.ts buildCliHighlightTheme + the per-theme cache.
local function build_cli_highlight_theme(theme)
  local function fg(key)
    return function(s) return theme:fg(key, s) end
  end
  return {
    keyword = fg("syntaxKeyword"),
    built_in = fg("syntaxType"),
    literal = fg("syntaxNumber"),
    number = fg("syntaxNumber"),
    regexp = fg("syntaxString"),
    string = fg("syntaxString"),
    comment = fg("syntaxComment"),
    doctag = fg("syntaxComment"),
    meta = fg("muted"),
    ["function"] = fg("syntaxFunction"),
    title = fg("syntaxFunction"),
    class = fg("syntaxType"),
    type = fg("syntaxType"),
    tag = fg("syntaxPunctuation"),
    name = fg("syntaxKeyword"),
    attr = fg("syntaxVariable"),
    variable = fg("syntaxVariable"),
    params = fg("syntaxVariable"),
    operator = fg("syntaxOperator"),
    punctuation = fg("syntaxPunctuation"),
    emphasis = function(s) return theme:italic(s) end,
    strong = function(s) return theme:bold(s) end,
    link = function(s) return theme:underline(s) end,
    addition = fg("toolDiffAdded"),
    deletion = fg("toolDiffRemoved"),
  }
end

local cli_highlight_theme_cache = setmetatable({}, { __mode = "k" })
local function get_cli_highlight_theme(theme)
  local cached = cli_highlight_theme_cache[theme]
  if not cached then
    cached = build_cli_highlight_theme(theme)
    cli_highlight_theme_cache[theme] = cached
  end
  return cached
end

-- Shared body of the two spec copies of highlightCode: nil when no valid
-- language (the caller styles the fallback), themed lines otherwise, and
-- `false` when highlighting raised (the caller styles the catch).
local function try_highlight_lines(code, lang, theme)
  local valid = (lang and pi.hljs.supports_language(lang)) and lang or nil
  if not valid then return nil end
  local ok, rendered = pcall(syntax_highlight, code, {
    language = valid,
    ignore_illegals = true,
    theme = get_cli_highlight_theme(theme),
  })
  if not ok then return false end
  return hl_split_lines(rendered)
end

local function md_code_block_lines(code, theme)
  local lines = {}
  for i, line in ipairs(hl_split_lines(code)) do
    lines[i] = theme:fg("mdCodeBlock", line)
  end
  return lines
end

-- theme.ts highlightCode: no valid language -> mdCodeBlock-styled lines;
-- highlight failure -> unstyled lines.
local function theme_highlight_code(code, lang, theme)
  local lines = try_highlight_lines(code, lang, theme)
  if lines == nil then return md_code_block_lines(code, theme) end
  if lines == false then return hl_split_lines(code) end
  return lines
end

-- getMarkdownTheme's highlightCode: identical except the catch also takes
-- the mdCodeBlock styling.
local function markdown_highlight_code(code, lang, theme)
  local lines = try_highlight_lines(code, lang, theme)
  if lines == nil or lines == false then return md_code_block_lines(code, theme) end
  return lines
end
