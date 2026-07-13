-- ls.ts — the ls tool.
--
-- Divergences noted: the spec sorts with localeCompare on lowercased
-- names (ICU collation, stable); this sorts by lowercased byte order —
-- identical for ASCII names.
local LS_DEFAULT_LIMIT = 500

local function format_ls_call(args, theme, base_cwd)
  local limit = args ~= nil and args.limit or nil
  local raw_path = ""
  if args ~= nil then raw_path = str(args.path) end
  local path_display = render_tool_path(raw_path, theme, base_cwd, { emptyFallback = "." })
  local text = theme:fg("toolTitle", theme:bold("ls")) .. " " .. path_display
  if limit ~= nil then
    text = text .. theme:fg("toolOutput", " (limit " .. fmt_num(limit) .. ")")
  end
  return text
end

local function format_ls_result(result, options, theme, show_images)
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

  local entry_limit = result.details and result.details.entryLimitReached
  local truncation = result.details and result.details.truncation
  if entry_limit or (truncation and truncation.truncated) then
    local warnings = {}
    if entry_limit then warnings[#warnings + 1] = fmt_num(entry_limit) .. " entries limit" end
    if truncation and truncation.truncated then
      warnings[#warnings + 1] = format_size(truncation.maxBytes or DEFAULT_MAX_BYTES) .. " limit"
    end
    text = text .. "\n" .. theme:fg("warning", "[Truncated: " .. table.concat(warnings, ", ") .. "]")
  end
  return text
end
local MAX_SAFE_INTEGER = 9007199254740991 -- JS Number.MAX_SAFE_INTEGER

pi.register_tool({
  name = "ls",
  active_by_default = false,
  label = "ls",
  description = (
    "List directory contents. Returns entries sorted alphabetically, with '/' suffix for"
    .. " directories. Includes dotfiles. Output is truncated to %d entries or %dKB"
    .. " (whichever is hit first)."
  ):format(LS_DEFAULT_LIMIT, DEFAULT_MAX_BYTES // 1024),
  promptSnippet = "List directory contents",
  parameters = {
    type = "object",
    properties = {
      path = { type = "string", description = "Directory to list (default: current directory)" },
      limit = { type = "number", description = "Maximum number of entries to return (default: 500)" },
    },
  },
  execute = function(_tool_call_id, params, signal)
    if signal and signal:is_aborted() then error("Operation aborted", 0) end
    local path, limit = params.path, params.limit
    local dir_path = resolve_to_cwd((path ~= nil and path ~= "") and path or ".")
    local effective_limit = limit or LS_DEFAULT_LIMIT

    if not pi.fs.exists(dir_path) then
      error(("Path not found: %s"):format(dir_path), 0)
    end
    local st = pi.fs.stat(dir_path)
    if st.type ~= "dir" then
      error(("Not a directory: %s"):format(dir_path), 0)
    end
    local ok, entries = pcall(pi.fs.read_dir, dir_path)
    if not ok then
      error(("Cannot read directory: %s"):format(tostring(entries)), 0)
    end

    -- Sort alphabetically, case-insensitive.
    table.sort(entries, function(a, b)
      return a:lower() < b:lower()
    end)

    -- Format entries with directory indicators.
    local results = {}
    local entry_limit_reached = false
    for _, entry in ipairs(entries) do
      if #results >= effective_limit then
        entry_limit_reached = true
        break
      end
      local full_path = pi.path.join(dir_path, entry)
      local ok_stat, entry_stat = pcall(pi.fs.stat, full_path)
      -- Skip entries we cannot stat.
      if ok_stat then
        results[#results + 1] = entry .. (entry_stat.type == "dir" and "/" or "")
      end
    end

    if #results == 0 then
      return { content = { { type = "text", text = "(empty directory)" } } }
    end

    local raw_output = table.concat(results, "\n")
    -- Byte truncation only: entry count is already capped.
    local truncation = truncate_head(raw_output, { maxLines = MAX_SAFE_INTEGER })
    local output = truncation.content
    local details = {}
    -- Actionable notices for truncation and entry limits.
    local notices = {}
    if entry_limit_reached then
      notices[#notices + 1] = ("%d entries limit reached. Use limit=%d for more"):format(
        effective_limit,
        effective_limit * 2
      )
      details.entryLimitReached = effective_limit
    end
    if truncation.truncated then
      notices[#notices + 1] = ("%s limit reached"):format(format_size(DEFAULT_MAX_BYTES))
      details.truncation = truncation
    end
    if #notices > 0 then
      output = output .. ("\n\n[%s]"):format(table.concat(notices, ". "))
    end

    return {
      content = { { type = "text", text = output } },
      details = next(details) ~= nil and details or nil,
    }
  end,
  renderCall = function(args, theme, context)
    return text_component(format_ls_call(args, theme, context.cwd), 0, 0)
  end,
  renderResult = function(result, options, theme, context)
    return text_component(format_ls_result(result, options, theme, context.showImages), 0, 0)
  end,
})
