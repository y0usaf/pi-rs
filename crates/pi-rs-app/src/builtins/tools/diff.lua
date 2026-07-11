-- modes/interactive/components/diff.ts — renderDiff with word-level
-- intra-line inverse highlighting over the jsdiff mechanism
-- (pi.diff.words). The spec styles through the module-global theme; the
-- Lua port receives the same theme object from the caller.

-- parseDiffLine: /^([+-\s])(\s*\d*)\s(.*)$/
local function parse_diff_line(line)
  local prefix, line_num, content = line:match("^([+%-%s])(%s*%d*)%s(.*)$")
  if prefix == nil then return nil end
  return { prefix = prefix, lineNum = line_num, content = content }
end

local function diff_replace_tabs(text)
  return (text:gsub("\t", "   "))
end

-- Word-level diff rendered with inverse on changed parts; leading
-- whitespace of the first changed part stays unhighlighted.
local function render_intra_line_diff(old_content, new_content, theme)
  local word_diff = pi.diff.words(old_content, new_content)
  local removed_line, added_line = {}, {}
  local is_first_removed, is_first_added = true, true
  for _, part in ipairs(word_diff) do
    if part.removed then
      local value = part.value
      if is_first_removed then
        local leading = value:match("^(%s*)") or ""
        value = value:sub(#leading + 1)
        removed_line[#removed_line + 1] = leading
        is_first_removed = false
      end
      if value ~= "" then removed_line[#removed_line + 1] = theme:inverse(value) end
    elseif part.added then
      local value = part.value
      if is_first_added then
        local leading = value:match("^(%s*)") or ""
        value = value:sub(#leading + 1)
        added_line[#added_line + 1] = leading
        is_first_added = false
      end
      if value ~= "" then added_line[#added_line + 1] = theme:inverse(value) end
    else
      removed_line[#removed_line + 1] = part.value
      added_line[#added_line + 1] = part.value
    end
  end
  return table.concat(removed_line), table.concat(added_line)
end

local function render_diff(diff_text, theme, _options)
  local lines = split(diff_text, "\n")
  local result = {}
  local i = 1
  while i <= #lines do
    local line = lines[i]
    local parsed = parse_diff_line(line)
    if not parsed then
      result[#result + 1] = theme:fg("toolDiffContext", line)
      i = i + 1
    elseif parsed.prefix == "-" then
      local removed_lines = {}
      while i <= #lines do
        local p = parse_diff_line(lines[i])
        if not p or p.prefix ~= "-" then break end
        removed_lines[#removed_lines + 1] = p
        i = i + 1
      end
      local added_lines = {}
      while i <= #lines do
        local p = parse_diff_line(lines[i])
        if not p or p.prefix ~= "+" then break end
        added_lines[#added_lines + 1] = p
        i = i + 1
      end
      -- Intra-line diffing only for a single-line modification.
      if #removed_lines == 1 and #added_lines == 1 then
        local removed, added = removed_lines[1], added_lines[1]
        local removed_line, added_line = render_intra_line_diff(
          diff_replace_tabs(removed.content), diff_replace_tabs(added.content), theme)
        result[#result + 1] = theme:fg("toolDiffRemoved", "-" .. removed.lineNum .. " " .. removed_line)
        result[#result + 1] = theme:fg("toolDiffAdded", "+" .. added.lineNum .. " " .. added_line)
      else
        for _, removed in ipairs(removed_lines) do
          result[#result + 1] = theme:fg("toolDiffRemoved", "-" .. removed.lineNum .. " " .. diff_replace_tabs(removed.content))
        end
        for _, added in ipairs(added_lines) do
          result[#result + 1] = theme:fg("toolDiffAdded", "+" .. added.lineNum .. " " .. diff_replace_tabs(added.content))
        end
      end
    elseif parsed.prefix == "+" then
      result[#result + 1] = theme:fg("toolDiffAdded", "+" .. parsed.lineNum .. " " .. diff_replace_tabs(parsed.content))
      i = i + 1
    else
      result[#result + 1] = theme:fg("toolDiffContext", " " .. parsed.lineNum .. " " .. diff_replace_tabs(parsed.content))
      i = i + 1
    end
  end
  return table.concat(result, "\n")
end
