-- find.ts — fd-backed glob search on the public extension surface.
-- Abort: entry check, a post-ensureTool check, kill-the-child via
-- pi.exec's signal option, and a close-time check ahead of exit-code
-- handling (spec: the settle order in execute).
local FIND_DEFAULT_LIMIT = 1000

local function format_find_call(args, theme)
  local pattern, raw_path, limit
  if args ~= nil then
    pattern = str(args.pattern)
    raw_path = str(args.path)
    limit = args.limit
  else
    pattern, raw_path = "", ""
  end
  local display_path = raw_path ~= nil and shorten_path(raw_path ~= "" and raw_path or ".") or nil
  local invalid_arg = invalid_arg_text(theme)
  local text = theme:fg("toolTitle", theme:bold("find")) .. " "
    .. (pattern == nil and invalid_arg or theme:fg("accent", pattern))
    .. theme:fg("toolOutput", " in " .. (display_path == nil and invalid_arg or display_path))
  if limit ~= nil then
    text = text .. theme:fg("toolOutput", " (limit " .. fmt_num(limit) .. ")")
  end
  return text
end

local function format_find_result(result, options, theme, show_images)
  local output = js_trim(get_text_output(result, show_images))
  local text = ""
  if output ~= "" then
    local lines = split(output, "\n")
    local max_lines = options.expanded and #lines or 20
    local remaining = #lines - max_lines
    local display = {}
    for i = 1, math.min(max_lines, #lines) do
      display[i] = theme:fg("toolOutput", lines[i])
    end
    text = text .. "\n" .. table.concat(display, "\n")
    if remaining > 0 then
      text = text .. theme:fg("muted", ("\n... (%d more lines,"):format(remaining))
        .. " " .. key_hint(theme, "app.tools.expand", "to expand") .. theme:fg("muted", ")")
    end
  end

  local result_limit = result.details and result.details.resultLimitReached
  local truncation = result.details and result.details.truncation
  if result_limit or (truncation and truncation.truncated) then
    local warnings = {}
    if result_limit then warnings[#warnings + 1] = fmt_num(result_limit) .. " results limit" end
    if truncation and truncation.truncated then
      warnings[#warnings + 1] = format_size(truncation.maxBytes or DEFAULT_MAX_BYTES) .. " limit"
    end
    text = text .. "\n" .. theme:fg("warning", "[Truncated: " .. table.concat(warnings, ", ") .. "]")
  end
  return text
end
local MAX_SAFE_INTEGER = 9007199254740991

local function find_trim(value)
  return (value:gsub("^%s+", ""):gsub("%s+$", ""))
end

pi.register_tool({
  name = "find",
  active_by_default = false,
  label = "find",
  description = "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore. Output is truncated to 1000 results or 50KB (whichever is hit first).",
  promptSnippet = "Find files by glob pattern (respects .gitignore)",
  parameters = {
    type = "object",
    properties = {
      pattern = { type = "string", description = "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'" },
      path = { type = "string", description = "Directory to search in (default: current directory)" },
      limit = { type = "number", description = "Maximum number of results (default: 1000)" },
    },
    required = { "pattern" },
  },
  execute = function(_tool_call_id, params, signal)
    if signal and signal:is_aborted() then error("Operation aborted", 0) end
    local search_path = resolve_to_cwd((params.path ~= nil and params.path ~= "") and params.path or ".")
    -- Spec: no existence pre-check on the fd path ("Path not found" is
    -- the custom-glob branch only) — a missing path surfaces fd's stderr.
    local effective_limit = params.limit or FIND_DEFAULT_LIMIT

    local command = "fd"
    local probe = pi.exec(command, { "--version" })
    if probe.code ~= 0 then
      command = "fdfind"
      probe = pi.exec(command, { "--version" })
    end
    if signal and signal:is_aborted() then error("Operation aborted", 0) end
    if probe.code ~= 0 then error("fd is not available and could not be downloaded", 0) end

    local args = { "--glob", "--color=never", "--hidden", "--no-require-git", "--max-results", tostring(effective_limit) }
    local pattern = params.pattern
    if pattern:find("/", 1, true) then
      args[#args + 1] = "--full-path"
      if pattern:sub(1, 1) ~= "/" and pattern:sub(1, 3) ~= "**/" and pattern ~= "**" then
        pattern = "**/" .. pattern
      end
    end
    args[#args + 1] = "--"
    args[#args + 1] = pattern
    args[#args + 1] = search_path
    local result = pi.exec(command, args, { signal = signal })
    if signal and signal:is_aborted() then error("Operation aborted", 0) end
    local stdout = result.stdout or ""
    if result.code ~= 0 and stdout == "" then
      local message = find_trim(result.stderr or "")
      error(message ~= "" and message or ("fd exited with code " .. result.code), 0)
    end
    if stdout == "" then
      return { content = { { type = "text", text = "No files found matching pattern" } } }
    end

    local paths = {}
    for _, raw in ipairs(split(stdout, "\n")) do
      local line = find_trim(raw:gsub("\r$", ""))
      if line ~= "" then
        local trailing = line:sub(-1) == "/" or line:sub(-1) == "\\"
        local relative
        if line:sub(1, #search_path) == search_path then
          relative = line:sub(#search_path + 1):gsub("^[/\\]", "")
        else
          relative = pi.path.relative(search_path, line)
        end
        relative = relative:gsub("\\", "/")
        if trailing and relative:sub(-1) ~= "/" then relative = relative .. "/" end
        paths[#paths + 1] = relative
      end
    end
    if #paths == 0 then
      return { content = { { type = "text", text = "No files found matching pattern" } } }
    end

    local truncation = truncate_head(table.concat(paths, "\n"), { maxLines = MAX_SAFE_INTEGER })
    local output, details, notices = truncation.content, {}, {}
    if #paths >= effective_limit then
      notices[#notices + 1] = ("%d results limit reached. Use limit=%d for more, or refine pattern"):format(effective_limit, effective_limit * 2)
      details.resultLimitReached = effective_limit
    end
    if truncation.truncated then
      notices[#notices + 1] = format_size(DEFAULT_MAX_BYTES) .. " limit reached"
      details.truncation = truncation
    end
    if #notices > 0 then output = output .. "\n\n[" .. table.concat(notices, ". ") .. "]" end
    return { content = { { type = "text", text = output } }, details = next(details) and details or nil }
  end,
  renderCall = function(args, theme, _context)
    return text_component(format_find_call(args, theme), 0, 0)
  end,
  renderResult = function(result, options, theme, context)
    return text_component(format_find_result(result, options, theme, context.showImages), 0, 0)
  end,
})
