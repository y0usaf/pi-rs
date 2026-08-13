-- Translation of Pi v0.79.0 examples/extensions/git-merge-and-resolve.ts.
-- Keeps the working branch up to date with its upstream tracking ref. After
-- each agent turn, fetches and merges. Clean merges complete silently; when
-- conflicts arise the working tree is left dirty and the agent receives a
-- follow-up message listing each conflict block with file, line range, and
-- ours/theirs sections so it can resolve them.
local pi = ...

-- Parse conflict markers from working tree files with unmerged paths.
local function find_conflicts(cwd)
  local diff = pi.exec("git", { "diff", "--name-only", "--diff-filter=U" })
  if diff.code ~= 0 or diff.stdout:gsub("%s+$", "") == "" then
    return {}
  end

  local blocks = {}
  for file in diff.stdout:gmatch("[^\n]+") do
    local content = pi.fs.read_file(cwd .. "/" .. file)
    if content then
      local line_no = 0
      local block_start, separator_line
      for line in content:gmatch("([^\n]*)\n?") do
        line_no = line_no + 1
        if line:sub(1, 7) == "<<<<<<<" then
          block_start = line_no
          separator_line = nil
        elseif line:sub(1, 7) == "=======" and block_start then
          separator_line = line_no
        elseif line:sub(1, 7) == ">>>>>>>" and block_start and separator_line then
          blocks[#blocks + 1] = {
            file = file, startLine = block_start, separatorLine = separator_line, endLine = line_no,
          }
          block_start = nil
          separator_line = nil
        end
      end
    end
  end
  return blocks
end

local function format_range(start_line, end_line)
  if start_line > end_line then return "empty" end
  if start_line == end_line then return tostring(start_line) end
  return start_line .. "-" .. end_line
end

local function format_conflicts(ref, blocks)
  local lines = { "Merged " .. ref .. " with conflicts:", "" }
  for _, b in ipairs(blocks) do
    local ours = format_range(b.startLine + 1, b.separatorLine - 1)
    local theirs = format_range(b.separatorLine + 1, b.endLine - 1)
    lines[#lines + 1] = ("  %s:%d-%d (ours %s, theirs %s)"):format(
      b.file, b.startLine, b.endLine, ours, theirs)
  end
  lines[#lines + 1] = ""
  lines[#lines + 1] = "Resolve these conflicts."
  return table.concat(lines, "\n")
end

pi.on("agent_end", function(_event, ctx)
  local rev_parse = pi.exec("git", { "rev-parse", "--git-dir" })
  if rev_parse.code ~= 0 then return end

  local ref = "MERGE_HEAD"

  -- If not already in a merge, attempt one
  local merge_head = pi.exec("git", { "rev-parse", "MERGE_HEAD" })
  if merge_head.code ~= 0 then
    -- Only attempt a new merge if the working tree is clean
    local status = pi.exec("git", { "status", "--porcelain" })
    if status.stdout:gsub("%s+$", "") ~= "" then return end

    local upstream = pi.exec("git", {
      "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}",
    })
    if upstream.code ~= 0 then return end

    ref = upstream.stdout:gsub("%s+$", "")
    local remote = ref:match("^([^/]+)") or ""
    ctx.ui.notify("git-merge-and-resolve: fetching " .. remote .. ", merging " .. ref, "info")

    local fetch = pi.exec("git", { "fetch", remote })
    if fetch.code ~= 0 then
      ctx.ui.notify("git-merge-and-resolve: fetch failed: " .. fetch.stderr:gsub("%s+$", ""), "warning")
      return
    end

    local merge = pi.exec("git", { "merge", "--no-ff", ref })
    if merge.code == 0 then return end
  end

  -- Either we just merged with conflicts, or we were already in an unfinished merge
  local conflicts = find_conflicts(ctx.cwd)
  if #conflicts == 0 then return end

  pi.sendUserMessage(format_conflicts(ref, conflicts), { deliverAs = "followUp" })
end)