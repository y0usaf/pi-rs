-- core/prompt-templates.ts — the text helpers plus the disk loader
-- (`loadPromptTemplates`). The loader shares the resource-loader seam: it
-- reads through `pi.fs`, resolves paths against the project cwd, and
-- parses frontmatter through `pi.parse_frontmatter`, so file-backed and
-- embedded packages use exactly the same mechanism. Discovery follows the
-- spec: global `agentDir/prompts/`, project `cwd/.pi/prompts/`, then
-- explicit prompt paths (files or directories), glued by source scope.
--
-- Shared fragment: included by both product packs, so it only assumes the
-- chunk argument `pi` (used for fs access, path resolution, parse_frontmatter,
-- and the module surface).
do
local pi = ...

-- JS String.prototype.split for a plain separator: keeps empty segments
-- including a trailing one.
local function split_plain(s, sep)
  local out = {}
  local start = 1
  while true do
    local i = s:find(sep, start, true)
    if not i then
      out[#out + 1] = s:sub(start)
      return out
    end
    out[#out + 1] = s:sub(start, i - 1)
    start = i + #sep
  end
end

local function is_whitespace_char(ch)
  return ch == " " or ch == "\t" or ch == "\n" or ch == "\r" or ch == "\f"
    or ch == "\v"
end

-- prompt-templates.ts parseCommandArgs: bash-style quoted tokenization.
-- A quote character opens a quoted run until its close; whitespace between
-- runs splits tokens. Unterminated quotes consume the rest of the input.
local function parse_command_args(args_string)
  local args, current = {}, {}
  local in_quote = nil
  local i, n = 1, #args_string
  while i <= n do
    local char = args_string:sub(i, i)
    if in_quote then
      if char == in_quote then
        in_quote = nil
      else
        current[#current + 1] = char
      end
    elseif char == '"' or char == "'" then
      in_quote = char
    elseif is_whitespace_char(char) then
      if #current > 0 then
        args[#args + 1] = table.concat(current)
        current = {}
      end
    else
      current[#current + 1] = char
    end
    i = i + 1
  end
  if #current > 0 then
    args[#args + 1] = table.concat(current)
  end
  return args
end

local function parse_int_10(str)
  return tonumber(str) or 0
end

local function slice_join(args, start_idx, length)
  local parts = {}
  local n = #args
  for k = start_idx, math.min(n, start_idx + length - 1) do
    parts[#parts + 1] = args[k]
  end
  return table.concat(parts, " ")
end

local function slice_from(args, start_idx)
  local parts = {}
  for k = start_idx, #args do
    parts[#parts + 1] = args[k]
  end
  return table.concat(parts, " ")
end

-- prompt-templates.ts substituteArgs. The JS regex:
--   /\$\{(\d+):-([^}]*)\}|\$\{@:(\d+)(?::(\d+))?\}|\$(ARGUMENTS|@|\d+)/g
-- We walk the string left-to-right and match at each '$' using string.find
-- (which returns start, stop and every capture). Values replaced are never
-- recursively substituted (we copy literal text around matches).
local function substitute_args(content, args)
  local all_args = table.concat(args, " ")
  local out, i, n = {}, 1, #content
  while i <= n do
    -- Default form: ${N:-default}
    local s, e, num, def = content:find("%$%{(%d+):%-([^}]*)%}", i)
    if s == i then
      local value = args[parse_int_10(num)]
      out[#out + 1] = (value and #value > 0) and value or def
      i = e + 1
      goto continue
    end
    -- Slice with length: ${@:N:L}
    s, e, start_s, len_s = content:find("%$%{@:(%d+):(%d+)%}", i)
    if s == i then
      local start = parse_int_10(start_s) - 1
      if start < 0 then start = 0 end
      out[#out + 1] = slice_join(args, start + 1, parse_int_10(len_s))
      i = e + 1
      goto continue
    end
    -- Slice from: ${@:N}
    s, e, start_s = content:find("%$%{@:(%d+)%}", i)
    if s == i then
      local start = parse_int_10(start_s) - 1
      if start < 0 then start = 0 end
      out[#out + 1] = slice_from(args, start + 1)
      i = e + 1
      goto continue
    end
    -- Simple form: $ARGUMENTS, $@, $N (three literal patterns; Lua pattern
    -- alternation is unreliable across engines, so each is matched directly).
    local s_simple, e_simple, simple
    s_simple, e_simple, simple = content:find("%$@", i)
    if s_simple == i then
      out[#out + 1] = all_args
      i = e_simple + 1
      goto continue
    end
    s_simple, e_simple, simple = content:find("%$ARGUMENTS", i)
    if s_simple == i then
      out[#out + 1] = all_args
      i = e_simple + 1
      goto continue
    end
    s_simple, e_simple, simple = content:find("%$%d+", i)
    if s_simple == i then
      local digits = content:sub(i + 1, e_simple)
      out[#out + 1] = args[parse_int_10(digits)] or ""
      i = e_simple + 1
      goto continue
    end
    -- Lone '$' with no match: literal.
    out[#out + 1] = content:sub(i, i)
    i = i + 1
    ::continue::
  end
  return table.concat(out)
end

-- prompt-templates.ts expandPromptTemplate: if the text starts with "/",
-- match `/<name>[ <args>]` and expand against the first template with that
-- name; otherwise return the text unchanged.
local function expand_prompt_template(text, templates)
  if not text or text:sub(1, 1) ~= "/" then return text end
  local name, args_string = text:match("^/([^%s]+)%s?(.*)$")
  if not name then return text end
  local template = nil
  for _, t in ipairs(templates or {}) do
    if t.name == name then template = t break end
  end
  if template then
    local args = parse_command_args(args_string or "")
    return substitute_args(template.content, args)
  end
  return text
end

-- prompt-templates.ts loadTemplateFromFile: parse frontmatter, name from the
-- filename without `.md`, description from frontmatter or the first non-empty
-- line (truncated to 60 chars with a "..." marker), optional argument-hint,
-- content from the body. Returns nil on any read/parse failure (spec: return
-- null in the catch; the host never matches on errors here).
local function load_template_from_file(file_path, source_info)
  local ok_read, raw_content = pcall(pi.fs.read_file, file_path)
  if not ok_read then return nil end
  local ok_fm, doc = pcall(pi.parse_frontmatter, raw_content)
  if not ok_fm then return nil end
  local frontmatter, body = doc.frontmatter, doc.body

  local name = file_path:gsub("%.md$", "")
  name = pi.path.basename(name)

  local description = ""
  if type(frontmatter.description) == "string" then
    description = frontmatter.description
  end
  if description == "" then
    local first_line = (body:match("[^\r\n]+"))
    if first_line then
      if #first_line > 60 then
        description = first_line:sub(1, 60) .. "..."
      else
        description = first_line
      end
    end
  end

  local template = {
    name = name,
    description = description,
    content = body,
    sourceInfo = source_info,
    filePath = file_path,
  }
  if frontmatter["argument-hint"] then
    template.argumentHint = tostring(frontmatter["argument-hint"])
  end
  return template
end

-- prompt-templates.ts loadTemplatesFromDir: non-recursive scan of a directory
-- for `.md` files (symlinks followed; broken symlinks and read errors skipped).
-- Returns templates only (the spec's loader has no diagnostics for prompts).
local function load_templates_from_dir(dir, get_source_info)
  local ok, entries = pcall(pi.fs.read_dir, dir)
  if not ok then return {} end
  local templates = {}
  for _, entry in ipairs(entries) do
    local full_path = pi.path.join(dir, entry)
    if entry:sub(-3) == ".md" then
      local ok_stat, st = pcall(pi.fs.stat, full_path)
      if ok_stat and st.type == "file" then
        local template = load_template_from_file(full_path, get_source_info(full_path))
        if template then templates[#templates + 1] = template end
      end
    end
  end
  return templates
end

-- prompt-templates.ts getSourceInfo classification for a resolved path:
-- user (`agentDir/prompts`) > project (`cwd/.pi/prompts`) > generic local
-- with a baseDir of the directory (or its parent when the path is a file).
local function prompts_is_under_path(target, root)
  if target == root then return true end
  return target:sub(1, #root + 1) == root .. "/"
end

local function get_prompt_source_info(resolved_path, global_dir, project_dir, cwd)
  local info = { source = "local", origin = "top-level" }
  if prompts_is_under_path(resolved_path, global_dir) then
    info.scope = "user"
    info.baseDir = global_dir
  elseif prompts_is_under_path(resolved_path, project_dir) then
    info.scope = "project"
    info.baseDir = project_dir
  else
    info.scope = "temporary"
    local ok, st = pcall(pi.fs.stat, resolved_path)
    if ok and st.type == "dir" then
      info.baseDir = resolved_path
    else
      info.baseDir = pi.path.dirname(resolved_path)
    end
  end
  info.path = resolved_path
  return info
end

-- prompt-templates.ts loadPromptTemplates. Discovery order: global
-- `agentDir/prompts`, project `cwd/.pi/prompts` (both gated by includeDefaults),
-- then explicit prompt paths (each a file or directory, resolved against cwd
-- with `~` expansion and trimmbing). Missing explicit paths are skipped.
-- Returns a flat array (dedupe/collisions are owned by the resource loader,
-- PLAN 9.7 package-manager/resource-loader).
local function load_prompt_templates(options)
  local cwd = options.cwd or pi.cwd()
  local agent_dir = options.agentDir or ""
  local prompt_paths = options.promptPaths or {}
  local include_defaults = options.includeDefaults ~= false
  local config_dir = ".pi"

  local project_prompts_dir = pi.path.resolve(cwd, config_dir, "prompts")
  local global_prompts_dir = (agent_dir and agent_dir ~= "")
    and pi.path.resolve(agent_dir, "prompts") or nil

  local function get_source_info(resolved_path)
    return get_prompt_source_info(
      resolved_path,
      global_prompts_dir or resolved_path,
      project_prompts_dir,
      cwd)
  end

  local templates = {}
  if include_defaults then
    if global_prompts_dir then
      local ok, _ = pcall(pi.fs.stat, global_prompts_dir)
      if ok then
        local from_global = load_templates_from_dir(global_prompts_dir, get_source_info)
        for _, t in ipairs(from_global) do templates[#templates + 1] = t end
      end
    end
    local ok, _ = pcall(pi.fs.stat, project_prompts_dir)
    if ok then
      local from_project = load_templates_from_dir(project_prompts_dir, get_source_info)
      for _, t in ipairs(from_project) do templates[#templates + 1] = t end
    end
  end

  for _, raw_path in ipairs(prompt_paths) do
    local resolved = pi.path.is_absolute(raw_path) and pi.path.normalize(raw_path)
      or pi.path.resolve(cwd, raw_path)
    local ok, st = pcall(pi.fs.stat, resolved)
    if ok then
      if st.type == "dir" then
        local from_dir = load_templates_from_dir(resolved, get_source_info)
        for _, t in ipairs(from_dir) do templates[#templates + 1] = t end
      elseif st.type == "file" and resolved:sub(-3) == ".md" then
        local template = load_template_from_file(resolved, get_source_info(resolved))
        if template then templates[#templates + 1] = template end
      end
    end
  end

  return templates
end

-- Public exact-version module: builtin and file-backed packages import the
-- same deterministic prompt-template helpers, including the disk loader.
-- No chunk-local cross-pack global remains.
pi.module.define({
  name = "pi.interactive.prompts",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      parse_command_args = parse_command_args,
      substitute_args = substitute_args,
      expand_prompt_template = expand_prompt_template,
      load_prompt_templates = load_prompt_templates,
      load_template_from_file = load_template_from_file,
      load_templates_from_dir = load_templates_from_dir,
    }
  end,
})
end