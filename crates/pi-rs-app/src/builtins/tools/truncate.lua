-- truncate.ts — shared truncation utilities for tool outputs.
--
-- Two independent limits, whichever is hit first wins: lines (default
-- 2000) and bytes (default 50KB). Never returns partial lines (except the
-- bash tail-truncation edge case). Result tables keep the spec's
-- camelCase field names: they cross the bridge as `details.truncation`
-- and must serialize 1:1 with pi's JSON. Divergence noted: the spec's
-- `truncatedBy: null` serializes as an absent key here (Lua tables have
-- no null values); JS string.length is bytes here (Lua strings are byte
-- strings — equal for the spec's byte-length uses).
local DEFAULT_MAX_LINES = 2000
local DEFAULT_MAX_BYTES = 50 * 1024
local GREP_MAX_LINE_LENGTH = 500 -- max chars per grep match line

local function split_lines_for_counting(content)
  if content == "" then
    return {}
  end
  local lines = split(content, "\n")
  if content:sub(-1) == "\n" then
    lines[#lines] = nil
  end
  return lines
end

-- Format bytes as human-readable size.
local function format_size(bytes)
  if bytes < 1024 then
    return ("%dB"):format(bytes)
  elseif bytes < 1024 * 1024 then
    return ("%.1fKB"):format(bytes / 1024)
  else
    return ("%.1fMB"):format(bytes / (1024 * 1024))
  end
end

-- Truncate a string to fit within a byte limit (from the end), keeping
-- UTF-8 boundaries intact.
local function truncate_string_to_bytes_from_end(str, max_bytes)
  if #str <= max_bytes then
    return str
  end
  local start = #str - max_bytes + 1
  while start <= #str do
    local b = str:byte(start)
    if b < 0x80 or b >= 0xC0 then
      break
    end
    start = start + 1
  end
  return str:sub(start)
end

-- Truncate content from the head (keep first N lines/bytes). Never
-- returns partial lines; if the first line exceeds the byte limit,
-- returns empty content with firstLineExceedsLimit=true.
local function truncate_head(content, options)
  options = options or {}
  local max_lines = options.maxLines or DEFAULT_MAX_LINES
  local max_bytes = options.maxBytes or DEFAULT_MAX_BYTES
  local total_bytes = #content
  local lines = split_lines_for_counting(content)
  local total_lines = #lines

  if total_lines <= max_lines and total_bytes <= max_bytes then
    return {
      content = content,
      truncated = false,
      totalLines = total_lines,
      totalBytes = total_bytes,
      outputLines = total_lines,
      outputBytes = total_bytes,
      lastLinePartial = false,
      firstLineExceedsLimit = false,
      maxLines = max_lines,
      maxBytes = max_bytes,
    }
  end

  if #(lines[1] or "") > max_bytes then
    return {
      content = "",
      truncated = true,
      truncatedBy = "bytes",
      totalLines = total_lines,
      totalBytes = total_bytes,
      outputLines = 0,
      outputBytes = 0,
      lastLinePartial = false,
      firstLineExceedsLimit = true,
      maxLines = max_lines,
      maxBytes = max_bytes,
    }
  end

  local out, out_bytes = {}, 0
  local truncated_by = "lines"
  local i = 1
  while i <= total_lines and i <= max_lines do
    local line = lines[i]
    local line_bytes = #line + (i > 1 and 1 or 0) -- +1 for newline
    if out_bytes + line_bytes > max_bytes then
      truncated_by = "bytes"
      break
    end
    out[#out + 1] = line
    out_bytes = out_bytes + line_bytes
    i = i + 1
  end
  if #out >= max_lines and out_bytes <= max_bytes then
    truncated_by = "lines"
  end

  local out_content = table.concat(out, "\n")
  return {
    content = out_content,
    truncated = true,
    truncatedBy = truncated_by,
    totalLines = total_lines,
    totalBytes = total_bytes,
    outputLines = #out,
    outputBytes = #out_content,
    lastLinePartial = false,
    firstLineExceedsLimit = false,
    maxLines = max_lines,
    maxBytes = max_bytes,
  }
end

-- Truncate content from the tail (keep last N lines/bytes); may return a
-- partial first line when a single line exceeds the byte limit.
local function truncate_tail(content, options)
  options = options or {}
  local max_lines = options.maxLines or DEFAULT_MAX_LINES
  local max_bytes = options.maxBytes or DEFAULT_MAX_BYTES
  local total_bytes = #content
  local lines = split_lines_for_counting(content)
  local total_lines = #lines

  if total_lines <= max_lines and total_bytes <= max_bytes then
    return {
      content = content,
      truncated = false,
      totalLines = total_lines,
      totalBytes = total_bytes,
      outputLines = total_lines,
      outputBytes = total_bytes,
      lastLinePartial = false,
      firstLineExceedsLimit = false,
      maxLines = max_lines,
      maxBytes = max_bytes,
    }
  end

  local out, out_bytes = {}, 0
  local truncated_by = "lines"
  local last_line_partial = false
  local i = total_lines
  while i >= 1 and #out < max_lines do
    local line = lines[i]
    local line_bytes = #line + (#out > 0 and 1 or 0) -- +1 for newline
    if out_bytes + line_bytes > max_bytes then
      truncated_by = "bytes"
      -- Edge case: no lines yet and this one exceeds maxBytes — take the
      -- end of the line (partial).
      if #out == 0 then
        local truncated_line = truncate_string_to_bytes_from_end(line, max_bytes)
        table.insert(out, 1, truncated_line)
        out_bytes = #truncated_line
        last_line_partial = true
      end
      break
    end
    table.insert(out, 1, line)
    out_bytes = out_bytes + line_bytes
    i = i - 1
  end
  if #out >= max_lines and out_bytes <= max_bytes then
    truncated_by = "lines"
  end

  local out_content = table.concat(out, "\n")
  return {
    content = out_content,
    truncated = true,
    truncatedBy = truncated_by,
    totalLines = total_lines,
    totalBytes = total_bytes,
    outputLines = #out,
    outputBytes = #out_content,
    lastLinePartial = last_line_partial,
    firstLineExceedsLimit = false,
    maxLines = max_lines,
    maxBytes = max_bytes,
  }
end

-- Truncate a single line to max characters, adding a [truncated] suffix
-- (grep match lines). Divergence noted: JS counts UTF-16 units; bytes
-- here — revisit when the grep port lands.
local function truncate_line(line, max_chars)
  max_chars = max_chars or GREP_MAX_LINE_LENGTH
  if #line <= max_chars then
    return { text = line, wasTruncated = false }
  end
  return { text = line:sub(1, max_chars) .. "... [truncated]", wasTruncated = true }
end

-- Public exact-version module: builtin and file-backed packages import the
-- same closures. No `_G` export or load-order-only global remains.
pi.module.define({
  name = "pi.tools.truncate",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      truncate_head = truncate_head,
      truncate_tail = truncate_tail,
      truncate_line = truncate_line,
      format_size = format_size,
      DEFAULT_MAX_LINES = DEFAULT_MAX_LINES,
      DEFAULT_MAX_BYTES = DEFAULT_MAX_BYTES,
    }
  end,
})
