-- render-utils.ts — shared presentation helpers for the tool renderers,
-- plus the utils/ansi.ts stripAnsi and utils/shell.ts sanitizeBinaryOutput
-- slices they call, the tui terminal-image.ts imageFallback text, and the
-- theme.ts getLanguageFromPath/highlightCode pair.
--
-- Renderer component contract (pi-rs's Lua port of pi-tui components): a
-- renderer returns `function(width) -> lines`. The lines are exactly what
-- the pi component's render(width) yields; composition (tool shells,
-- boxes) happens in the interactive pack's tool-execution port.

-- utils/ansi.ts stripAnsi (ansi-regex): CSI sequences, OSC terminated by
-- BEL or ST, and other single ESC-introduced sequences.
local function strip_ansi(value)
  if not value:find("\27", 1, true) and not value:find("\155", 1, true) then
    return value
  end
  value = value:gsub("\27%][^\7\27]*\7", "")      -- OSC ... BEL
  value = value:gsub("\27%][^\27]-\27\\", "")     -- OSC ... ST
  value = value:gsub("\27%[[%d;:?]*[%a~]", "")    -- CSI
  value = value:gsub("\155[%d;:?]*[%a~]", "")     -- 8-bit CSI
  value = value:gsub("\27[%(%)][%w]", "")
  value = value:gsub("\27[%w=<>]", "")
  return value
end

-- utils/shell.ts sanitizeBinaryOutput: drop control characters (except
-- \t \n \r) and Unicode interlinear-annotation format characters.
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

-- terminal-image.ts imageFallback (no filename/dimension callers here;
-- getImageDimensions is carried with the images milestone).
local function image_fallback(mime_type)
  return "[Image: [" .. mime_type .. "]]"
end

-- render-utils.ts shortenPath.
local function shorten_path(path)
  if type(path) ~= "string" then return "" end
  local home = pi.env.HOME
  if home and home ~= "" and path:sub(1, #home) == home then
    return "~" .. path:sub(#home + 1)
  end
  return path
end

-- render-utils.ts linkPath. pi-rs has no terminal-capability detection yet
-- (getCapabilities mechanism); hyperlinks are off, matching pi's
-- conservative unknown-terminal default, so the styled text passes through.
local function link_path(styled_text, _raw_path, _cwd)
  return styled_text
end

-- render-utils.ts str: string passes through, null/undefined become "",
-- anything else is the invalid-arg marker (nil).
local function str(value)
  if type(value) == "string" then return value end
  if value == nil then return "" end
  return nil
end

-- render-utils.ts replaceTabs / normalizeDisplayText.
local function replace_tabs(text)
  return (text:gsub("\t", "   "))
end

local function normalize_display_text(text)
  return (text:gsub("\r", ""))
end

-- render-utils.ts getTextOutput. Terminal image capabilities are not
-- detected yet (caps.images = nil), so image blocks always render their
-- text indicators — pi's behavior on capability-less terminals.
local function get_text_output(result, _show_images)
  if not result then return "" end
  local text_parts, image_parts = {}, {}
  for _, block in ipairs(result.content or {}) do
    if block.type == "text" then
      text_parts[#text_parts + 1] = sanitize_binary_output(strip_ansi(block.text or "")):gsub("\r", "")
    elseif block.type == "image" then
      image_parts[#image_parts + 1] = image_fallback(block.mimeType or "image/unknown")
    end
  end
  local output = table.concat(text_parts, "\n")
  if #image_parts > 0 then
    local indicators = table.concat(image_parts, "\n")
    output = output ~= "" and (output .. "\n" .. indicators) or indicators
  end
  return output
end

-- render-utils.ts invalidArgText / renderToolPath.
local function invalid_arg_text(theme)
  return theme:fg("error", "[invalid arg]")
end

local function render_tool_path(raw_path, theme, base_cwd, options)
  if raw_path == nil then return invalid_arg_text(theme) end
  local value = raw_path
  if value == "" and options and options.emptyFallback then value = options.emptyFallback end
  if value == "" then return theme:fg("toolOutput", "...") end
  return link_path(theme:fg("accent", shorten_path(value)), value, base_cwd)
end

-- theme.ts getLanguageFromPath.
local EXT_TO_LANG = {
  ts = "typescript", tsx = "typescript", js = "javascript", jsx = "javascript",
  mjs = "javascript", cjs = "javascript", py = "python", rb = "ruby",
  rs = "rust", go = "go", java = "java", kt = "kotlin", swift = "swift",
  c = "c", h = "c", cpp = "cpp", cc = "cpp", cxx = "cpp", hpp = "cpp",
  cs = "csharp", php = "php", sh = "bash", bash = "bash", zsh = "bash",
  fish = "fish", ps1 = "powershell", sql = "sql", html = "html",
  htm = "html", css = "css", scss = "scss", sass = "sass", less = "less",
  json = "json", yaml = "yaml", yml = "yaml", toml = "toml", xml = "xml",
  md = "markdown", markdown = "markdown", dockerfile = "dockerfile",
  makefile = "makefile", cmake = "cmake", lua = "lua", perl = "perl",
  r = "r", scala = "scala", clj = "clojure", ex = "elixir", exs = "elixir",
  erl = "erlang", hs = "haskell", ml = "ocaml", vim = "vim",
  graphql = "graphql", proto = "protobuf", tf = "hcl", hcl = "hcl",
}

local function get_language_from_path(file_path)
  local parts = split(file_path, ".")
  local ext = parts[#parts]
  if ext == nil or ext == "" then return nil end
  return EXT_TO_LANG[ext:lower()]
end

-- theme.ts highlightCode over the pi.hljs engine; the port lives in the
-- shared utils/syntax-highlight.lua fragment.
local function highlight_code(code, lang, theme)
  return theme_highlight_code(code, lang, theme)
end

-- Shared by the read/write renderers (each spec module defines its own
-- copy of trimTrailingEmptyLines).
local function trim_trailing_empty_lines(lines)
  local last = #lines
  while last > 0 and lines[last] == "" do last = last - 1 end
  local out = {}
  for i = 1, last do out[i] = lines[i] end
  return out
end

-- JS String.prototype.trim.
local function js_trim(s)
  return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

-- pi-tui Text(text, paddingX, paddingY) as a renderer component.
local function text_component(text, padding_x, padding_y)
  return function(width)
    return pi.tui.text_render(text, width, padding_x or 0, padding_y or 0)
  end
end

-- pi-tui Box(paddingX, paddingY, bgFn) over renderer components
-- (applyBackgroundToLine pads each line to width before styling).
local function box_component(children, padding_x, padding_y, bg)
  return function(width)
    local content_width = math.max(1, width - padding_x * 2)
    local left = string.rep(" ", padding_x)
    local child_lines = {}
    for _, child in ipairs(children) do
      for _, line in ipairs(child(content_width)) do
        child_lines[#child_lines + 1] = left .. line
      end
    end
    if #child_lines == 0 then return {} end
    local function apply(line)
      local pad = width - pi.tui.visible_width(line)
      if pad > 0 then line = line .. string.rep(" ", pad) end
      if bg then return bg(line) end
      return line
    end
    local out = {}
    for _ = 1, padding_y do out[#out + 1] = apply("") end
    for _, line in ipairs(child_lines) do out[#out + 1] = apply(line) end
    for _ = 1, padding_y do out[#out + 1] = apply("") end
    return out
  end
end

-- pi-tui Spacer(lines) as a renderer component.
local function spacer_component(lines)
  return function(_width)
    local out = {}
    for _ = 1, lines or 1 do out[#out + 1] = "" end
    return out
  end
end

-- Public rendering helpers. The module value closes over this pack's reviewed
-- policy and host mechanisms; callers do not import frontend implementation
-- classes or rely on globals.
pi.module.define({
  name = "pi.tools.render",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      strip_ansi = strip_ansi,
      sanitize_binary_output = sanitize_binary_output,
      shorten_path = shorten_path,
      str = str,
      get_text_output = get_text_output,
      render_tool_path = render_tool_path,
      get_language_from_path = get_language_from_path,
      trim_trailing_empty_lines = trim_trailing_empty_lines,
      text_component = text_component,
      box_component = box_component,
      spacer_component = spacer_component,
    }
  end,
})
