-- Translation of Pi v0.79.0 examples/extensions/inline-bash.ts.
-- Expands inline `!{command}` patterns in user prompts by executing them
-- and replacing them with their output.
local pi = ...

local TIMEOUT_MS = 30000

local function find_matches(text)
  local matches = {}
  for full, command in text:gmatch("(%!%{([^}]+)%})") do
    matches[#matches + 1] = { full = full, command = command }
  end
  return matches
end

pi.on("input", function(event, ctx)
  local text = event.text

  -- Don't process whole-line bash commands starting with `!` (except `!{`)
  if text:gsub("^%s+", ""):sub(1, 1) == "!" and text:gsub("^%s+", ""):sub(1, 2) ~= "!{" then
    return { action = "continue" }
  end

  local matches = find_matches(text)
  if #matches == 0 then
    return { action = "continue" }
  end

  local result = text
  local expansions = {}

  for _, m in ipairs(matches) do
    local ok, bash_result = pcall(pi.exec, "bash", { "-c", m.command }, { timeout = TIMEOUT_MS })
    local output, trimmed, error_msg
    if ok then
      output = bash_result.stdout ~= "" and bash_result.stdout or bash_result.stderr
      trimmed = output:gsub("^%s+", ""):gsub("%s+$", "")
      if bash_result.code ~= 0 and bash_result.stderr ~= "" then
        error_msg = "exit code " .. bash_result.code
      end
    else
      error_msg = tostring(bash_result):gsub("^.-:%d+: ", "")
      trimmed = ""
    end

    local entry = { command = m.command, output = trimmed }
    if error_msg then entry.error = error_msg end
    expansions[#expansions + 1] = entry
    result = result:gsub(m.full, (error_msg and not ok) and ("[error: " .. error_msg .. "]") or trimmed, 1)
  end

  if ctx.hasUI and #expansions > 0 then
    local summary = {}
    for _, e in ipairs(expansions) do
      local status = e.error and (" (" .. e.error .. ")") or ""
      local preview = e.output:len() > 50 and (e.output:sub(1, 50) .. "...") or e.output
      summary[#summary + 1] = "!{" .. e.command .. "}" .. status .. ' -> "' .. preview .. '"'
    end
    ctx.ui.notify("Expanded " .. #expansions .. " inline command(s):\n" .. table.concat(summary, "\n"), "info")
  end

  return { action = "transform", text = result, images = event.images }
end)