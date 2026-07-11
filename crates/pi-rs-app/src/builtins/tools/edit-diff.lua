-- edit-diff.ts -- matching, line endings, and diff details. Diff
-- computation goes through the jsdiff mechanism (pi.diff, jsdiff 8.0.4
-- parity) exactly like the spec's `import * as Diff from "diff"`.
local function normalize_to_lf(text)
  return (text:gsub("\r\n", "\n"):gsub("\r", "\n"))
end

local function detect_line_ending(content)
  local crlf, lf = content:find("\r\n", 1, true), content:find("\n", 1, true)
  if crlf and lf and crlf <= lf then return "\r\n" end
  return "\n"
end

local function restore_line_endings(text, ending)
  if ending == "\r\n" then return (text:gsub("\n", "\r\n")) end
  return text
end

local function normalize_for_fuzzy_match(text)
  text = pi.text.nfkc(text)
  local lines = split(text, "\n")
  for i = 1, #lines do lines[i] = lines[i]:gsub("%s+$", "") end
  text = table.concat(lines, "\n")
  for _, q in ipairs({ "‘", "’", "‚", "‛" }) do text = text:gsub(q, "'") end
  for _, q in ipairs({ "“", "”", "„", "‟" }) do text = text:gsub(q, '"') end
  for _, d in ipairs({ "‐", "‑", "‒", "–", "—", "―", "−" }) do text = text:gsub(d, "-") end
  for _, s in ipairs({ " ", " ", " ", " ", " ", " ", " ", " ", " ", " ", " ", " ", "　" }) do text = text:gsub(s, " ") end
  return text
end

local function fuzzy_find_text(content, old_text)
  local index = content:find(old_text, 1, true)
  if index then return { found = true, index = index, length = #old_text, fuzzy = false } end
  local haystack, needle = normalize_for_fuzzy_match(content), normalize_for_fuzzy_match(old_text)
  index = haystack:find(needle, 1, true)
  if not index then return { found = false, fuzzy = false } end
  return { found = true, index = index, length = #needle, fuzzy = true }
end

local function count_occurrences(content, old_text)
  local haystack, needle = normalize_for_fuzzy_match(content), normalize_for_fuzzy_match(old_text)
  local count, start = 0, 1
  while true do
    local i = haystack:find(needle, start, true)
    if not i then return count end
    count, start = count + 1, i + #needle
  end
end

local function indexed_message(path, i, total, single, multi)
  if total == 1 then return single:format(path) end
  return multi:format(i - 1, path)
end

local function apply_edits_to_normalized_content(content, edits, path)
  local normalized, use_fuzzy = {}, false
  for i, edit in ipairs(edits) do
    normalized[i] = { oldText = normalize_to_lf(edit.oldText), newText = normalize_to_lf(edit.newText) }
    if normalized[i].oldText == "" then
      error(indexed_message(path, i, #edits, "oldText must not be empty in %s.", "edits[%d].oldText must not be empty in %s."), 0)
    end
    if fuzzy_find_text(content, normalized[i].oldText).fuzzy then use_fuzzy = true end
  end
  local base = use_fuzzy and normalize_for_fuzzy_match(content) or content
  local matches = {}
  for i, edit in ipairs(normalized) do
    local match = fuzzy_find_text(base, edit.oldText)
    if not match.found then
      error(indexed_message(path, i, #edits, "Could not find the exact text in %s. The old text must match exactly including all whitespace and newlines.", "Could not find edits[%d] in %s. The oldText must match exactly including all whitespace and newlines."), 0)
    end
    local occurrences = count_occurrences(base, edit.oldText)
    if occurrences > 1 then
      if #edits == 1 then error(("Found %d occurrences of the text in %s. The text must be unique. Please provide more context to make it unique."):format(occurrences, path), 0) end
      error(("Found %d occurrences of edits[%d] in %s. Each oldText must be unique. Please provide more context to make it unique."):format(occurrences, i - 1, path), 0)
    end
    matches[#matches + 1] = { editIndex = i - 1, index = match.index, length = match.length, newText = edit.newText }
  end
  table.sort(matches, function(a, b) return a.index < b.index end)
  for i = 2, #matches do
    local previous, current = matches[i - 1], matches[i]
    if previous.index + previous.length > current.index then
      error(("edits[%d] and edits[%d] overlap in %s. Merge them into one edit or target disjoint regions."):format(previous.editIndex, current.editIndex, path), 0)
    end
  end
  local result = base
  for i = #matches, 1, -1 do
    local m = matches[i]
    result = result:sub(1, m.index - 1) .. m.newText .. result:sub(m.index + m.length)
  end
  if result == base then
    if #edits == 1 then error("No changes made to " .. path .. ". The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected.", 0) end
    error("No changes made to " .. path .. ". The replacements produced identical content.", 0)
  end
  return base, result
end

-- generateDiffString: a display-oriented diff with line numbers and
-- context, computed over jsdiff diffLines (the pi.diff.lines binding).
local function generate_diff_string(old_content, new_content, context)
  context = context or 4
  local parts = pi.diff.lines(old_content, new_content)
  local output = {}

  local max_line_num = math.max(#split(old_content, "\n"), #split(new_content, "\n"))
  local width = #tostring(max_line_num)
  local function pad(n) return string.format("%" .. width .. "d", n) end
  local blank = string.rep(" ", width)

  local old_line, new_line = 1, 1
  local last_was_change = false
  local first_changed_line

  for i, part in ipairs(parts) do
    local raw = split(part.value, "\n")
    if raw[#raw] == "" then table.remove(raw) end

    if part.added or part.removed then
      -- Capture the first changed line (in the new file).
      if first_changed_line == nil then first_changed_line = new_line end
      for _, line in ipairs(raw) do
        if part.added then
          output[#output + 1] = "+" .. pad(new_line) .. " " .. line
          new_line = new_line + 1
        else
          output[#output + 1] = "-" .. pad(old_line) .. " " .. line
          old_line = old_line + 1
        end
      end
      last_was_change = true
    else
      -- Context lines: only show a few before/after changes.
      local next_part = parts[i + 1]
      local has_leading_change = last_was_change
      local has_trailing_change = next_part ~= nil and (next_part.added or next_part.removed)

      local function push_context(line)
        output[#output + 1] = " " .. pad(old_line) .. " " .. line
        old_line, new_line = old_line + 1, new_line + 1
      end

      if has_leading_change and has_trailing_change then
        if #raw <= context * 2 then
          for _, line in ipairs(raw) do push_context(line) end
        else
          local skipped = #raw - context * 2
          for j = 1, context do push_context(raw[j]) end
          output[#output + 1] = " " .. blank .. " ..."
          old_line, new_line = old_line + skipped, new_line + skipped
          for j = #raw - context + 1, #raw do push_context(raw[j]) end
        end
      elseif has_leading_change then
        local shown = math.min(#raw, context)
        for j = 1, shown do push_context(raw[j]) end
        local skipped = #raw - shown
        if skipped > 0 then
          output[#output + 1] = " " .. blank .. " ..."
          old_line, new_line = old_line + skipped, new_line + skipped
        end
      elseif has_trailing_change then
        local skipped = math.max(0, #raw - context)
        if skipped > 0 then
          output[#output + 1] = " " .. blank .. " ..."
          old_line, new_line = old_line + skipped, new_line + skipped
        end
        for j = skipped + 1, #raw do push_context(raw[j]) end
      else
        -- Skip these context lines entirely.
        old_line, new_line = old_line + #raw, new_line + #raw
      end
      last_was_change = false
    end
  end

  return table.concat(output, "\n"), first_changed_line
end

-- generateUnifiedPatch: Diff.createTwoFilesPatch(path, path, old, new,
-- undefined, undefined, { context = 4, headerOptions = FILE_HEADERS_ONLY }).
local function generate_unified_patch(path, old_content, new_content)
  return pi.diff.unified_patch(path, path, old_content, new_content,
    { context = 4, headers = "file" })
end

-- stripBom (edit.ts inlines the same check in execute).
local function strip_bom(text)
  if text:sub(1, 3) == "\239\187\191" then
    return "\239\187\191", text:sub(4)
  end
  return "", text
end

-- computeEditsDiff: preview the diff for an edit call without applying
-- it (drives the edit renderCall's diff preview). Returns
-- { diff, firstChangedLine } or { error }.
local function compute_edits_diff(path, edits, base_cwd)
  local absolute_path = resolve_to_cwd(path, base_cwd)
  if not pi.fs.exists(absolute_path) then
    return { error = "Could not edit file: " .. path .. ". Error code: ENOENT." }
  end
  local ok, result = pcall(function()
    local raw = utf8_lossy(pi.fs.read_bytes(absolute_path))
    local _, content = strip_bom(raw)
    local base, changed = apply_edits_to_normalized_content(normalize_to_lf(content), edits, path)
    local diff, first = generate_diff_string(base, changed)
    return { diff = diff, firstChangedLine = first }
  end)
  if ok then return result end
  return { error = tostring(result) }
end
