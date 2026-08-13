-- Translation of Pi v0.79.0 examples/extensions/prompt-customizer.ts.
-- Adds tool-specific guidance to the system prompt based on the active tool
-- set, respecting user configuration.
--
-- Pi's BuildSystemPromptOptions carried selectedTools/skills/appendSystemPrompt;
-- pi-rs's system_prompt_options exposes the resolved active `toolNames` (see
-- coding-agent.lua), so this translation reads that equivalent field.
local pi = ...

local function has_tool(options, name)
  for _, tool in ipairs(options.toolNames or {}) do
    if tool == name then return true end
  end
  return false
end

local function add_tool_guidance(options, base_prompt)
  local parts = {}

  if has_tool(options, "read") then
    parts[#parts + 1] = "• Use the `read` tool for file contents (supports text and images)."
    parts[#parts + 1] = "  - For large files, use `offset` and `limit` to read in chunks."
  end

  if has_tool(options, "bash") then
    parts[#parts + 1] = "• Execute commands with the `bash` tool. Use it for file operations like `ls`, `find`, `grep`."
  end

  if has_tool(options, "edit") then
    parts[#parts + 1] = "• Use the `edit` tool for precise text replacements in files. Match exact content including whitespace."
  end

  if has_tool(options, "write") then
    parts[#parts + 1] = "• Use the `write` tool to create new files or overwrite existing ones completely."
  end

  if #parts == 0 then
    return base_prompt
  end

  return base_prompt .. "\n\n## Tool Guidance\n\n" .. table.concat(parts, "\n") .. "\n"
end

local function merge_with_user_append(options)
  local user_append = options.appendSystemPrompt
  local extension_specific = "\n## Extension-Added Context\n\nThis prompt includes tool guidance and skill information loaded dynamically.\nIf you have additional requirements, configure them via --append-system-prompt or project context files.\n"

  if user_append then
    return user_append .. "\n\n" .. extension_specific
  end

  return extension_specific
end

pi.on("before_agent_start", function(event)
  local system_prompt, system_prompt_options = event.systemPrompt, event.systemPromptOptions

  local custom_prompt = add_tool_guidance(system_prompt_options, system_prompt)
  local append_section = merge_with_user_append(system_prompt_options)

  return {
    systemPrompt = custom_prompt .. append_section,
  }
end)