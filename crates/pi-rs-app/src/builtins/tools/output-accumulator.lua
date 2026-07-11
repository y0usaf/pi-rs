-- output-accumulator.ts — bounded streaming collection and tail snapshots.
-- Raw persistence is performed by pi.exec; this object retains only enough
-- bytes to calculate the exact displayed tail and exact aggregate counters.
local function new_output_accumulator(full_path)
  local tail, total_bytes, newline_count = "", 0, 0
  local last_was_newline, finished = false, false
  -- Spec currentLineBytes: cumulative bytes of the open last line, kept
  -- across tail trims (getLastLineBytes reports the true line size).
  local current_line_bytes = 0
  -- Extra bytes let utf8_lossy repair a character split at the rolling edge.
  local RETAIN_BYTES = DEFAULT_MAX_BYTES + 4

  local function trim_tail()
    if #tail > RETAIN_BYTES then
      tail = tail:sub(#tail - RETAIN_BYTES + 1)
    end
  end

  local function totals()
    local total_lines = newline_count
    if total_bytes > 0 and not last_was_newline then total_lines = total_lines + 1 end
    return total_lines
  end

  return {
    append = function(data)
      if finished then error("Cannot append to a finished output accumulator") end
      if #data == 0 then return end
      total_bytes = total_bytes + #data
      local _, count = data:gsub("\n", "")
      newline_count = newline_count + count
      if count == 0 then
        current_line_bytes = current_line_bytes + #data
      else
        current_line_bytes = #(data:match("[^\n]*$"))
      end
      last_was_newline = data:sub(-1) == "\n"
      tail = tail .. data
      trim_tail()
    end,
    finish = function() finished = true end,
    snapshot = function()
      local truncation = truncate_tail(utf8_lossy(tail))
      local total_lines = totals()
      local globally_truncated = total_bytes > #tail or total_bytes > DEFAULT_MAX_BYTES
        or total_lines > DEFAULT_MAX_LINES
      truncation.totalBytes = total_bytes
      truncation.totalLines = total_lines
      truncation.truncated = globally_truncated
      if globally_truncated and not truncation.truncatedBy then
        truncation.truncatedBy = total_lines > DEFAULT_MAX_LINES and "lines" or "bytes"
      end
      return {
        content = truncation.content,
        truncation = truncation,
        fullOutputPath = globally_truncated and full_path or nil,
      }
    end,
    last_line_bytes = function()
      return current_line_bytes
    end,
  }
end
