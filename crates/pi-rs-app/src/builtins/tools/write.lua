-- write.ts — the write tool. The spec's incremental highlight cache is a
-- perf mechanism over highlightCode; pi-rs re-renders pure components per
-- frame, so formatWriteCall recomputes the identical lines directly.
-- Abort: spec checks signal.aborted inside the mutation queue.

-- JS String.prototype.length: UTF-16 code units of a UTF-8 Lua string
-- (the spec's success message interpolates content.length).
local function js_string_length(s)
  local units, i, n = 0, 1, #s
  while i <= n do
    local b = s:byte(i)
    local size = b < 0x80 and 1 or (b < 0xE0 and 2 or (b < 0xF0 and 3 or 4))
    units = units + (size == 4 and 2 or 1)
    i = i + size
  end
  return units
end

local function write_raw_path(args)
  if args == nil then return "" end
  if args.file_path ~= nil then return str(args.file_path) end
  return str(args.path)
end

local function format_write_call(args, options, theme, base_cwd)
  local raw_path = write_raw_path(args)
  local file_content = ""
  if args ~= nil then file_content = str(args.content) end
  local path_display = render_tool_path(raw_path, theme, base_cwd)
  local text = theme:fg("toolTitle", theme:bold("write")) .. " " .. path_display

  if file_content == nil then
    text = text .. "\n\n" .. theme:fg("error", "[invalid content arg - expected string]")
  elseif file_content ~= "" then
    local lang = (raw_path ~= nil and raw_path ~= "") and get_language_from_path(raw_path) or nil
    local rendered_lines = lang
      and highlight_code(replace_tabs(normalize_display_text(file_content)), lang, theme)
      or split(normalize_display_text(file_content), "\n")
    local lines = trim_trailing_empty_lines(rendered_lines)
    local total_lines = #lines
    local max_lines = options.expanded and #lines or 10
    local remaining = #lines - max_lines
    local display = {}
    for i = 1, math.min(max_lines, #lines) do
      local line = lines[i]
      display[i] = lang and line or theme:fg("toolOutput", replace_tabs(line))
    end
    text = text .. "\n\n" .. table.concat(display, "\n")
    if remaining > 0 then
      text = text .. theme:fg("muted", ("\n... (%d more lines, %d total,"):format(remaining, total_lines))
        .. " " .. key_hint(theme, "app.tools.expand", "to expand") .. theme:fg("muted", ")")
    end
  end
  return text
end

local function format_write_result(result, is_error, theme)
  if not is_error then return nil end
  local parts = {}
  for _, block in ipairs(result.content or {}) do
    if block.type == "text" then parts[#parts + 1] = block.text or "" end
  end
  local output = table.concat(parts, "\n")
  if output == "" then return nil end
  return "\n" .. theme:fg("error", output)
end

pi.register_tool({
  name = "write",
  active_by_default = true,
  label = "write",
  description = "Write content to a file. Creates the file if it doesn't exist, overwrites if it does."
    .. " Automatically creates parent directories.",
  promptSnippet = "Create or overwrite files",
  promptGuidelines = { "Use write only for new files or complete rewrites." },
  parameters = {
    type = "object",
    properties = {
      path = { type = "string", description = "Path to the file to write (relative or absolute)" },
      content = { type = "string", description = "Content to write to the file" },
    },
    required = { "path", "content" },
  },
  execute = function(_tool_call_id, params, signal)
    local path, content = params.path, params.content
    local absolute_path = resolve_to_cwd(path)
    local dir = pi.path.dirname(absolute_path)
    return with_file_mutation_queue(absolute_path, function()
      if signal and signal:is_aborted() then error("Operation aborted", 0) end
      -- Create parent directories if needed, then write.
      pi.fs.mkdir(dir)
      pi.fs.write_file(absolute_path, content)
      return {
        content = { { type = "text", text = ("Successfully wrote %d bytes to %s"):format(js_string_length(content), path) } },
      }
    end)
  end,
  renderCall = function(args, theme, context)
    return text_component(
      format_write_call(args, { expanded = context.expanded, isPartial = context.isPartial }, theme, context.cwd),
      0, 0)
  end,
  renderResult = function(result, _options, theme, context)
    local output = format_write_result(result, context.isError, theme)
    if not output then
      return function() return {} end
    end
    return text_component(output, 0, 0)
  end,
})
