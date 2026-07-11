-- core/system-prompt.ts (buildSystemPrompt), the resource-loader.ts
-- project-context slice (loadContextFileFromDir / loadProjectContextFiles),
-- the skills.ts formatSkillsForPrompt slice, and agent-session.ts's
-- prompt-snippet/guideline normalization + _rebuildSystemPrompt
-- composition over the registered-tool definitions.
--
-- Shared fragment: included by the interactive pack and the one-shot
-- coding-agent pack, so it only assumes the chunk argument.
--
-- Seams recorded here:
-- - the spec's getReadmePath/getDocsPath/getExamplesPath come from
--   config.ts (PI_PACKAGE_DIR / executable dir); pi-rs's Rust config port
--   resolves them and passes readmePath/docsPath/examplesPath through the
--   request (mechanism data, not policy);
-- - options.now is a test seam for the spec's `new Date()` (epoch
--   seconds, defaults to os.time(); parity oracles pin it);
-- - the loader's --system-prompt/--append-system-prompt resolution
--   (resolvePromptInput) and skills loading join with PLAN items 7/9;
--   callers pass customPrompt/appendSystemPrompt/skills through unchanged.
local pi = ...

local function sp_trim(text)
  return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

-- skills.ts escapeXml (sequential replace, & first).
local function sp_escape_xml(text)
  return (text:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;")
    :gsub('"', "&quot;"):gsub("'", "&apos;"))
end

-- skills.ts formatSkillsForPrompt.
local function format_skills_for_prompt(skills)
  local visible = {}
  for _, skill in ipairs(skills or {}) do
    if not skill.disableModelInvocation then visible[#visible + 1] = skill end
  end
  if #visible == 0 then return "" end
  local lines = {
    "\n\nThe following skills provide specialized instructions for specific tasks.",
    "Use the read tool to load a skill's file when the task matches its description.",
    "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.",
    "",
    "<available_skills>",
  }
  for _, skill in ipairs(visible) do
    lines[#lines + 1] = "  <skill>"
    lines[#lines + 1] = "    <name>" .. sp_escape_xml(skill.name) .. "</name>"
    lines[#lines + 1] = "    <description>" .. sp_escape_xml(skill.description) .. "</description>"
    lines[#lines + 1] = "    <location>" .. sp_escape_xml(skill.filePath) .. "</location>"
    lines[#lines + 1] = "  </skill>"
  end
  lines[#lines + 1] = "</available_skills>"
  return table.concat(lines, "\n")
end

-- resource-loader.ts loadContextFileFromDir: first readable candidate wins.
local CONTEXT_FILE_CANDIDATES = { "AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD" }

local function load_context_file_from_dir(dir)
  for _, filename in ipairs(CONTEXT_FILE_CANDIDATES) do
    local file_path = pi.path.join(dir, filename)
    if pi.fs.exists(file_path) then
      local ok, content = pcall(pi.fs.read_file, file_path)
      if ok then return { path = file_path, content = content } end
      -- Spec: console.error(chalk.yellow(...)) and keep scanning.
      io.stderr:write("\27[33mWarning: Could not read " .. file_path .. ": "
        .. tostring(content) .. "\27[39m\n")
    end
  end
  return nil
end

-- resource-loader.ts loadProjectContextFiles: the global agent-dir file
-- first, then ancestor files ordered root-most to cwd, deduped by path.
local function load_project_context_files(options)
  local resolved_cwd = pi.path.resolve(options.cwd)
  local context_files, seen = {}, {}
  -- Spec always has an agent dir; harness requests may omit it.
  if options.agentDir and options.agentDir ~= "" then
    local global_context = load_context_file_from_dir(pi.path.resolve(options.agentDir))
    if global_context then
      context_files[#context_files + 1] = global_context
      seen[global_context.path] = true
    end
  end
  local ancestors = {}
  local current_dir = resolved_cwd
  local root = pi.path.resolve("/")
  while true do
    local context_file = load_context_file_from_dir(current_dir)
    if context_file and not seen[context_file.path] then
      table.insert(ancestors, 1, context_file)
      seen[context_file.path] = true
    end
    if current_dir == root then break end
    local parent_dir = pi.path.resolve(current_dir, "..")
    if parent_dir == current_dir then break end
    current_dir = parent_dir
  end
  for _, file in ipairs(ancestors) do context_files[#context_files + 1] = file end
  return context_files
end

-- system-prompt.ts buildSystemPrompt.
local function build_system_prompt(options)
  local custom_prompt = options.customPrompt
  local selected_tools = options.selectedTools
  local tool_snippets = options.toolSnippets or {}
  local prompt_guidelines = options.promptGuidelines
  local append_system_prompt = options.appendSystemPrompt
  local resolved_cwd = options.cwd
  local prompt_cwd = (resolved_cwd:gsub("\\", "/"))
  local date = os.date("%Y-%m-%d", options.now or os.time())
  local append_section = (append_system_prompt ~= nil and append_system_prompt ~= "")
    and ("\n\n" .. append_system_prompt) or ""
  local context_files = options.contextFiles or {}
  local skills = options.skills or {}

  local function append_project_context(prompt)
    if #context_files > 0 then
      prompt = prompt .. "\n\n<project_context>\n\n"
        .. "Project-specific instructions and guidelines:\n\n"
      for _, file in ipairs(context_files) do
        prompt = prompt .. '<project_instructions path="' .. file.path .. '">\n'
          .. file.content .. "\n</project_instructions>\n\n"
      end
      prompt = prompt .. "</project_context>\n"
    end
    return prompt
  end

  if custom_prompt ~= nil and custom_prompt ~= "" then
    local prompt = custom_prompt
    if append_section ~= "" then prompt = prompt .. append_section end
    prompt = append_project_context(prompt)
    -- Skills only when the read tool is available.
    local custom_prompt_has_read = true
    if selected_tools then
      custom_prompt_has_read = false
      for _, name in ipairs(selected_tools) do
        if name == "read" then custom_prompt_has_read = true end
      end
    end
    if custom_prompt_has_read and #skills > 0 then
      prompt = prompt .. format_skills_for_prompt(skills)
    end
    prompt = prompt .. "\nCurrent date: " .. date
    prompt = prompt .. "\nCurrent working directory: " .. prompt_cwd
    return prompt
  end

  local readme_path = options.readmePath or ""
  local docs_path = options.docsPath or ""
  local examples_path = options.examplesPath or ""

  -- A tool appears in Available tools only with a one-line snippet.
  local tools = selected_tools or { "read", "bash", "edit", "write" }
  local visible_rows = {}
  for _, name in ipairs(tools) do
    if tool_snippets[name] then
      visible_rows[#visible_rows + 1] = "- " .. name .. ": " .. tool_snippets[name]
    end
  end
  local tools_list = #visible_rows > 0 and table.concat(visible_rows, "\n") or "(none)"

  local guidelines_list, guidelines_set = {}, {}
  local function add_guideline(guideline)
    if guidelines_set[guideline] then return end
    guidelines_set[guideline] = true
    guidelines_list[#guidelines_list + 1] = guideline
  end
  local has = {}
  for _, name in ipairs(tools) do has[name] = true end
  if has.bash and not has.grep and not has.find and not has.ls then
    add_guideline("Use bash for file operations like ls, rg, find")
  end
  for _, guideline in ipairs(prompt_guidelines or {}) do
    local normalized = sp_trim(guideline)
    if #normalized > 0 then add_guideline(normalized) end
  end
  add_guideline("Be concise in your responses")
  add_guideline("Show file paths clearly when working with files")
  local guideline_rows = {}
  for i, guideline in ipairs(guidelines_list) do
    guideline_rows[i] = "- " .. guideline
  end
  local guidelines = table.concat(guideline_rows, "\n")

  local prompt = "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.\n\nAvailable tools:\n"
    .. tools_list
    .. "\n\nIn addition to the tools above, you may have access to other custom tools depending on the project.\n\nGuidelines:\n"
    .. guidelines
    .. "\n\nPi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):\n- Main documentation: "
    .. readme_path
    .. "\n- Additional docs: "
    .. docs_path
    .. "\n- Examples: "
    .. examples_path
    .. " (extensions, custom tools, SDK)\n- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory\n- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md)\n- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing\n- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)"

  if append_section ~= "" then prompt = prompt .. append_section end
  prompt = append_project_context(prompt)
  if has.read and #skills > 0 then
    prompt = prompt .. format_skills_for_prompt(skills)
  end
  prompt = prompt .. "\nCurrent date: " .. date
  prompt = prompt .. "\nCurrent working directory: " .. prompt_cwd
  return prompt
end

-- agent-session.ts _normalizePromptSnippet.
local function normalize_prompt_snippet(text)
  if text == nil or text == "" then return nil end
  local one_line = sp_trim(text:gsub("[\r\n]+", " "):gsub("%s+", " "))
  if #one_line > 0 then return one_line end
  return nil
end

-- agent-session.ts _normalizePromptGuidelines.
local function normalize_prompt_guidelines(guidelines)
  if guidelines == nil or #guidelines == 0 then return {} end
  local unique, seen = {}, {}
  for _, guideline in ipairs(guidelines) do
    local normalized = sp_trim(guideline)
    if #normalized > 0 and not seen[normalized] then
      seen[normalized] = true
      unique[#unique + 1] = normalized
    end
  end
  return unique
end

-- agent-session.ts _rebuildSystemPrompt: resolve snippets/guidelines for
-- the valid active tool names from the registered-tool definitions
-- (pi.registered_tools() is the definition registry), then build.
local function build_session_system_prompt(args)
  local registered = {}
  for _, definition in ipairs(pi.registered_tools()) do
    registered[definition.name] = definition
  end
  local valid_tool_names = {}
  for _, name in ipairs(args.toolNames or {}) do
    if registered[name] then valid_tool_names[#valid_tool_names + 1] = name end
  end
  local tool_snippets, prompt_guidelines = {}, {}
  for _, name in ipairs(valid_tool_names) do
    local definition = registered[name]
    local snippet = normalize_prompt_snippet(definition.promptSnippet)
    if snippet then tool_snippets[name] = snippet end
    for _, guideline in ipairs(normalize_prompt_guidelines(definition.promptGuidelines)) do
      prompt_guidelines[#prompt_guidelines + 1] = guideline
    end
  end
  local append_list = args.appendSystemPrompt or {}
  local append_system_prompt = #append_list > 0 and table.concat(append_list, "\n\n") or nil
  local context_files = args.contextFiles
    or load_project_context_files({ cwd = args.cwd, agentDir = args.agentDir })
  return build_system_prompt({
    cwd = args.cwd,
    skills = args.skills or {},
    contextFiles = context_files,
    customPrompt = args.customPrompt,
    appendSystemPrompt = append_system_prompt,
    selectedTools = valid_tool_names,
    toolSnippets = tool_snippets,
    promptGuidelines = prompt_guidelines,
    readmePath = args.readmePath,
    docsPath = args.docsPath,
    examplesPath = args.examplesPath,
    now = args.now,
  })
end
