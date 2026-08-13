-- Translation of Pi v0.79.0 examples/extensions/claude-rules.ts.
-- Scans the project's .claude/rules/ folder for rule files and lists them
-- in the system prompt.
local pi = ...

local function find_markdown_files(dir, base_path)
  base_path = base_path or ""
  local results = {}
  local ok, entries = pcall(pi.fs.read_dir, dir)
  if not ok then return results end

  for _, name in ipairs(entries) do
    local full = pi.path.join(dir, name)
    local relative = base_path ~= "" and (base_path .. "/" .. name) or name
    local stat_ok, info = pcall(pi.fs.stat, full)
    if stat_ok then
      if info.type == "dir" then
        local sub = find_markdown_files(full, relative)
        for _, r in ipairs(sub) do results[#results + 1] = r end
      elseif info.type == "file" and name:sub(-3) == ".md" then
        results[#results + 1] = relative
      end
    end
  end
  return results
end

local rule_files = {}
local rules_dir = ""

pi.on("session_start", function(_event, ctx)
  rules_dir = pi.path.join(ctx.cwd, ".claude", "rules")
  rule_files = find_markdown_files(rules_dir)

  if #rule_files > 0 then
    ctx.ui.notify("Found " .. #rule_files .. " rule(s) in .claude/rules/", "info")
  end
end)

pi.on("before_agent_start", function(event)
  if #rule_files == 0 then
    return nil
  end

  local rules_list = {}
  for _, f in ipairs(rule_files) do rules_list[#rules_list + 1] = "- .claude/rules/" .. f end

  return {
    systemPrompt = event.systemPrompt .. "\n\n## Project Rules\n\nThe following project rules are available in .claude/rules/:\n\n" .. table.concat(rules_list, "\n") .. "\n\nWhen working on tasks related to these rules, use the read tool to load the relevant rule files for guidance.\n",
  }
end)