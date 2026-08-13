-- core/skills.ts — the resource-loader disk discovery for SKILL.md.
--
-- Port of `loadSkillsFromDir` + `loadSkillFromFile` + validation + the
-- `formatSkillsForPrompt` XML serializer, sharing the `pi.fs`/`pi.path`/
-- `pi.parse_frontmatter` mechanism so embedded and file-backed packages use
-- the same loader.
--
-- Gitignore handling follows the spec (`.gitignore`/`.ignore`/`.fdignore`
-- with pattern prefixing, comment/negation stripping, and child-ignore
-- merging); the subset here supports the common directive forms. The
-- resource-loader precedence/dedupe/collisions are owned by the package
-- manager (PLAN 9.7) — this module returns the raw scanned skills plus the
-- per-file diagnostics Pi's loader emits.
do
local pi = ...

local MAX_NAME_LENGTH = 64
local MAX_DESCRIPTION_LENGTH = 1024
local IGNORE_FILE_NAMES = { ".gitignore", ".ignore", ".fdignore" }

-- toPosixPath: forward slashes (the posix port keeps them).
local function to_posix_path(p)
  local out = p:gsub("\\", "/")
  return out
end

-- prefixIgnorePattern: trim, drop blank, drop `#` comments (unless `\#`),
-- strip a leading negation marker into a prefixed `!pattern`.
local function prefix_ignore_pattern(line, prefix)
  local trimmed = line:gsub("^%s+", ""):gsub("%s+$", "")
  if trimmed == "" then return nil end
  if trimmed:sub(1, 1) == "#" and trimmed:sub(1, 2) ~= "\\#" then return nil end
  local pattern = line
  local negated = false
  if pattern:sub(1, 1) == "!" then
    negated = true
    pattern = pattern:sub(2)
  elseif pattern:sub(1, 2) == "\\!" then
    pattern = pattern:sub(2)
  end
  if pattern:sub(1, 1) == "/" then
    pattern = pattern:sub(2)
  end
  local prefixed = (prefix and prefix ~= "") and (prefix .. pattern) or pattern
  return (negated and ("!" .. prefixed) or prefixed)
end

-- A small ignore engine: anchored path segment/glob matching for the
-- directive forms Pi's `ignore` package supports. Patterns are posix-relative
-- to the directory that declared them. Returns true when `rel_path` is
-- matched by a non-negated pattern.
local Ignore = {}
Ignore.__index = Ignore
function Ignore.new()
  return setmetatable({ patterns = {} }, Ignore)
end

local function pattern_matches(pattern, rel_path)
  -- Negation already stripped before insertion; here we do segment/glob
  -- matching: exact segment, `dir/**`, `*.ext` basename, and plain substring
  -- glob via pattern expansion.
  if pattern:find("**", 1, true) then
    local base = pattern:sub(1, pattern:find("**", 1, true) - 1)
    base = base:gsub("%.$", "")
    if base == "" or rel_path:sub(1, #base) == base then return true end
    return false
  end
  if pattern:find("*", 1, true) then
    -- Convert shell glob to a Lua pattern on the whole path.
    local lua_pattern = pattern
      :gsub("%.%.", ".") -- `..` handled loosely
      :gsub("[%(%)%+%-%.]", function(c) return "%" .. c end)
      :gsub("%*", "[^/]*")
      :gsub("%?", "[^/]")
    return rel_path == pattern or rel_path:match("^" .. lua_pattern .. "$") ~= nil
  end
  return rel_path == pattern
end

function Ignore:add(lines)
  for _, line in ipairs(lines) do
    if line then self.patterns[#self.patterns + 1] = line end
  end
end

function Ignore:ignores(rel_path)
  local matched = false
  -- Later rules override earlier ones; a trailing negation wins.
  for _, pat in ipairs(self.patterns) do
    if pat:sub(1, 1) == "!" then
      if pattern_matches(pat:sub(2), rel_path) then matched = false end
    elseif pattern_matches(pat, rel_path) then
      matched = true
    end
  end
  return matched
end

-- addIgnoreRules: read each ignore file in `dir`, prefix its patterns with the
-- dir's relative path from `root`, and merge into `ig`.
local function add_ignore_rules(ig, dir, root)
  local rel_dir = pi.path.relative(root, dir)
  local prefix = (rel_dir ~= "" and rel_dir ~= ".") and (to_posix_path(rel_dir) .. "/") or ""
  for _, filename in ipairs(IGNORE_FILE_NAMES) do
    local ignore_path = pi.path.join(dir, filename)
    local ok_exists, exists = pcall(pi.fs.exists, ignore_path)
    if ok_exists and exists then
      local ok_read, content = pcall(pi.fs.read_file, ignore_path)
      if ok_read then
        local lines = {}
        for l in content:gmatch("[^\r\n]+") do
          local prefixed = prefix_ignore_pattern(l, prefix)
          if prefixed then lines[#lines + 1] = prefixed end
        end
        ig:add(lines)
      end
    end
  end
end

-- Validate a skill name per the Agent Skills spec. Returns error messages
-- (empty when valid).
local function validate_skill_name(name)
  local errors = {}
  if #name > MAX_NAME_LENGTH then
    errors[#errors + 1] = ("name exceeds %d characters (%d)"):format(MAX_NAME_LENGTH, #name)
  end
  if not name:match("^[a-z0-9%-]+$") then
    errors[#errors + 1] = "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)"
  end
  if name:sub(1, 1) == "-" or name:sub(-1) == "-" then
    errors[#errors + 1] = "name must not start or end with a hyphen"
  end
  if name:find("--", 1, true) then
    errors[#errors + 1] = "name must not contain consecutive hyphens"
  end
  return errors
end

local function validate_skill_description(description)
  local errors = {}
  if not description or (description:gsub("%s+", "") == "") then
    errors[#errors + 1] = "description is required"
  elseif #description > MAX_DESCRIPTION_LENGTH then
    errors[#errors + 1] = ("description exceeds %d characters (%d)"):format(MAX_DESCRIPTION_LENGTH, #description)
  end
  return errors
end

-- createSkillSourceInfo: source/scope/baseDir per how the skill was discovered.
local function create_skill_source_info(file_path, base_dir, source)
  local info = { source = "local", origin = "top-level"
  }
  if source == "user" then
    info.scope = "user"
    info.baseDir = base_dir
  elseif source == "project" then
    info.scope = "project"
    info.baseDir = base_dir
  elseif source == "path" then
    info.scope = "temporary"
    info.baseDir = base_dir
  else
    info.scope = "temporary"
    info.source = source
    info.baseDir = base_dir
  end
  info.path = file_path
  return info
end

-- loadSkillFromFile: parse frontmatter, validate description + name, and
-- build the skill (nil + diagnostics when description is missing). When
-- frontmatter parse fails a warning diagnostic records the file.
local function load_skill_from_file(file_path, source)
  local diagnostics = {}
  local ok_read, raw_content = pcall(pi.fs.read_file, file_path)
  if not ok_read then
    diagnostics[#diagnostics + 1] = { type = "warning", message = raw_content or "failed to read skill file", path = file_path }
    return { skill = nil, diagnostics = diagnostics }
  end
  local ok_fm, parsed = pcall(pi.parse_frontmatter, raw_content)
  if not ok_fm then
    diagnostics[#diagnostics + 1] = { type = "warning", message = parsed, path = file_path }
    return { skill = nil, diagnostics = diagnostics }
  end
  local frontmatter = parsed.frontmatter
  local skill_dir = pi.path.dirname(file_path)
  local parent_dir_name = pi.path.basename(skill_dir)

  local description = frontmatter.description
  local desc_errors = validate_skill_description(description)
  for _, e in ipairs(desc_errors) do
    diagnostics[#diagnostics + 1] = { type = "warning", message = e, path = file_path }
  end

  local name = (type(frontmatter.name) == "string") and frontmatter.name or parent_dir_name
  local name_errors = validate_skill_name(name)
  for _, e in ipairs(name_errors) do
    diagnostics[#diagnostics + 1] = { type = "warning", message = e, path = file_path }
  end

  if not description or (description:gsub("%s+", "") == "") then
    return { skill = nil, diagnostics = diagnostics }
  end

  local skill = {
    name = name,
    description = description,
    filePath = file_path,
    baseDir = skill_dir,
    sourceInfo = create_skill_source_info(file_path, skill_dir, source),
    disableModelInvocation = frontmatter["disable-model-invocation"] == true,
  }
  return { skill = skill, diagnostics = diagnostics }
end

-- loadSkillsFromDir: scan `dir` for SKILL.md (treating a dir with SKILL.md as
-- a root), or load direct `.md` children (includeRootFiles=true) and recurse
-- into subdirectories. See spec: skills.ts loadSkillsFromDirInternal.
local function load_skills_from_dir_internal(dir, source, include_root_files, ig, root)
  local skills, diagnostics = {}, {}
  local ok, entries = pcall(pi.fs.read_dir, dir)
  if not ok then
    return { skills = skills, diagnostics = diagnostics }
  end

  for _, entry in ipairs(entries) do
    if entry == "SKILL.md" then
      local full_path = pi.path.join(dir, entry)
      local rel_path = to_posix_path(pi.path.relative(root, full_path))
      local ok_stat, st = pcall(pi.fs.stat, full_path)
      if ok_stat and st.type == "file" and not ig:ignores(rel_path) then
        local result = load_skill_from_file(full_path, source)
        if result.skill then skills[#skills + 1] = result.skill end
        for _, d in ipairs(result.diagnostics) do diagnostics[#diagnostics + 1] = d end
      end
    end
  end

  for _, entry in ipairs(entries) do
    if entry:sub(1, 1) == "." then goto continue end
    if entry == "node_modules" then goto continue end
    local full_path = pi.path.join(dir, entry)
    local rel_path = to_posix_path(pi.path.relative(root, full_path))
    local ok_stat, st = pcall(pi.fs.stat, full_path)
    if not ok_stat then goto continue end
    if st.type == "dir" then
      if not ig:ignores(rel_path .. "/") then
        local sub = load_skills_from_dir_internal(full_path, source, false, ig, root)
        for _, s in ipairs(sub.skills) do skills[#skills + 1] = s end
        for _, d in ipairs(sub.diagnostics) do diagnostics[#diagnostics + 1] = d end
      end
      goto continue
    end
    if include_root_files and st.type == "file" and entry:sub(-3) == ".md"
      and entry ~= "SKILL.md" and not ig:ignores(rel_path) then
      local result = load_skill_from_file(full_path, source)
      if result.skill then skills[#skills + 1] = result.skill end
      for _, d in ipairs(result.diagnostics) do diagnostics[#diagnostics + 1] = d end
    end
    ::continue::
  end

  return { skills = skills, diagnostics = diagnostics }
end

-- formatSkillsForPrompt: build the available_skills XML block; skills with
-- disableModelInvocation are excluded.
local function skill_escape_xml(str)
  return (str
    :gsub("&", "&amp;")
    :gsub("<", "&lt;")
    :gsub(">", "&gt;")
    :gsub('"', "&quot;")
    :gsub("'", "&apos;"))
end

local function format_skills_for_prompt(skills)
  local visible = {}
  for _, s in ipairs(skills or {}) do
    if not s.disableModelInvocation then visible[#visible + 1] = s end
  end
  if #visible == 0 then return "" end
  local lines = {
    "\n\nThe following skills provide specialized instructions for specific tasks.",
    "Use the read tool to load a skill's file when the task matches its description.",
    "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.",
    "",
    "<available_skills>",
  }
  for _, s in ipairs(visible) do
    lines[#lines + 1] = "  <skill>"
    lines[#lines + 1] = "    <name>" .. skill_escape_xml(s.name) .. "</name>"
    lines[#lines + 1] = "    <description>" .. skill_escape_xml(s.description) .. "</description>"
    lines[#lines + 1] = "    <location>" .. skill_escape_xml(s.filePath) .. "</location>"
    lines[#lines + 1] = "  </skill>"
  end
  lines[#lines + 1] = "</available_skills>"
  return table.concat(lines, "\n")
end

-- loadSkillsFromDir public entry (source: "user"|"project"|"path"|other).
local function load_skills_from_dir(dir, source)
  if not dir or dir == "" then return { skills = {}, diagnostics = {} } end
  local root = pi.path.resolve(dir)
  local ig = Ignore.new()
  return load_skills_from_dir_internal(root, source, true, ig, root)
end

-- Public exact-version module.
pi.module.define({
  name = "pi.resources.skills",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      load_skills_from_dir = load_skills_from_dir,
      load_skill_from_file = load_skill_from_file,
      format_skills_for_prompt = format_skills_for_prompt,
      validate_skill_name = validate_skill_name,
      validate_skill_description = validate_skill_description,
      _ignore = Ignore,
    }
  end,
})
end