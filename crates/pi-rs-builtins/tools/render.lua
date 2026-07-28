-- Presentation policy for the shipped core tools: line windows, bounded
-- output, and line diffs. Tool results are data; this module only decides
-- what a model and a transcript row see. Rust neither formats nor truncates
-- tool output.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.render",
  version = "1",
  factory = function()
    local DEFAULT_MAX_OUTPUT_BYTES = 32 * 1024
    local DEFAULT_MAX_LINES = 400
    local DEFAULT_DIFF_ROWS = 40
    local DIFF_CONTEXT = 2

    local function split_lines(text)
      local out = {}
      if type(text) ~= "string" or #text == 0 then
        return out
      end
      local position = 1
      while true do
        local newline = string.find(text, "\n", position, true)
        if newline == nil then
          if position <= #text then
            out[#out + 1] = string.sub(text, position)
          end
          return out
        end
        out[#out + 1] = string.sub(text, position, newline - 1)
        position = newline + 1
      end
    end

    -- Byte-bounded clip with an explicit notice: a truncated result never
    -- pretends to be complete, and the bound is a declared limit.
    local function clip(text, options)
      options = options or {}
      local limit = tonumber(options.max_bytes) or DEFAULT_MAX_OUTPUT_BYTES
      if limit < 1 then
        limit = 1
      end
      if type(text) ~= "string" then
        text = tostring(text)
      end
      if #text <= limit then
        return { text = text, truncated = false, bytes = #text, total_bytes = #text }
      end
      local kept = string.sub(text, 1, limit)
      local dropped = #text - limit
      return {
        text = kept .. "\n… truncated " .. tostring(dropped) .. " more bytes",
        truncated = true,
        bytes = limit,
        total_bytes = #text,
      }
    end

    -- One line window with 1-based numbering, bounded by line count and then
    -- by bytes.
    local function number_lines(text, options)
      options = options or {}
      local all = split_lines(text)
      local total = #all
      local first = math.max(1, math.floor(tonumber(options.offset) or 1))
      local limit = math.floor(tonumber(options.limit) or DEFAULT_MAX_LINES)
      if limit < 1 then
        limit = 1
      end
      local last = math.min(total, first + limit - 1)
      local width = #tostring(math.max(last, 1))
      local rows = {}
      for index = first, last do
        rows[#rows + 1] = string.format("%" .. width .. "d| %s", index, all[index])
      end
      local body = table.concat(rows, "\n")
      local bounded = clip(body, options)
      return {
        text = bounded.text,
        total = total,
        first = first,
        last = last,
        shown = math.max(0, last - first + 1),
        truncated = bounded.truncated or last < total,
        bytes = bounded.total_bytes,
      }
    end

    -- Line diff without an edit-distance search: shared prefix and suffix,
    -- then one changed block. Bounded rows keep a large rewrite from
    -- producing an unbounded transcript row.
    local function diff(before, after, options)
      options = options or {}
      local max_rows = math.floor(tonumber(options.max_rows) or DEFAULT_DIFF_ROWS)
      if max_rows < 1 then
        max_rows = 1
      end
      local old_lines = split_lines(before)
      local new_lines = split_lines(after)
      local prefix = 0
      while
        prefix < #old_lines
        and prefix < #new_lines
        and old_lines[prefix + 1] == new_lines[prefix + 1]
      do
        prefix = prefix + 1
      end
      local suffix = 0
      while
        suffix < (#old_lines - prefix)
        and suffix < (#new_lines - prefix)
        and old_lines[#old_lines - suffix] == new_lines[#new_lines - suffix]
      do
        suffix = suffix + 1
      end

      local rows = {}
      local removed = 0
      local added = 0
      local truncated = false
      local function push(kind, line, text)
        if #rows >= max_rows then
          truncated = true
          return
        end
        rows[#rows + 1] = { kind = kind, line = line, text = text }
      end

      for index = math.max(1, prefix - DIFF_CONTEXT + 1), prefix do
        push("context", index, old_lines[index])
      end
      for index = prefix + 1, #old_lines - suffix do
        removed = removed + 1
        push("remove", index, old_lines[index])
      end
      for index = prefix + 1, #new_lines - suffix do
        added = added + 1
        push("add", index, new_lines[index])
      end
      local tail_start = #old_lines - suffix + 1
      for index = tail_start, math.min(#old_lines, tail_start + DIFF_CONTEXT - 1) do
        push("context", index, old_lines[index])
      end

      local marker = { context = "  ", remove = "- ", add = "+ " }
      local text_rows = {}
      for _, row in ipairs(rows) do
        text_rows[#text_rows + 1] = marker[row.kind] .. row.text
      end
      if truncated then
        text_rows[#text_rows + 1] = "… diff truncated at " .. tostring(max_rows) .. " rows"
      end

      return {
        rows = rows,
        added = added,
        removed = removed,
        truncated = truncated,
        changed = added > 0 or removed > 0,
        text = table.concat(text_rows, "\n"),
      }
    end

    return {
      split_lines = split_lines,
      clip = clip,
      number_lines = number_lines,
      diff = diff,
      default_max_output_bytes = DEFAULT_MAX_OUTPUT_BYTES,
      default_max_lines = DEFAULT_MAX_LINES,
    }
  end,
})
