-- resource-loader.ts + package-manager.ts resolve() — the deterministic
-- resource-resolution engine, exposed as the public `pi.resources` module.
--
-- It produces the resolved, precedence-sorted, de-duplicated resource list
-- (extensions / skills / prompts / themes) that the interactive runtime and
-- `/reload` consume. Resolution matches Pi's `DefaultPackageManager.resolve()`:
--   1. packages (configured + auto-installed npm/git/local) — project scope
--      first, then user scope, de-duplicated by identity (project wins);
--   2. settings entries (`extensions`/`skills`/`prompts`/`themes` arrays in
--      each scope's settings) with pattern filters;
--   3. auto-discovered resources from the conventional directories
--      (`<scope>/extensions`, `skills`, `prompts`, `themes`; `.agents/skills`);
--   4. collision resolution: a canonical path appears once, first-wins, and
--      the entries are sorted by precedence (project+settings > project+auto >
--      user+settings > user+auto > package).
--
-- Trust: project-scope resources are gathered only when the project is trusted
-- (`pi.settings.is_project_trusted`). Attribution (PathMetadata:
-- source/scope/origin/baseDir) attaches to every resolved entry. This module is
-- deterministic and hermetic-tested; it never touches the network.
--
-- Embedded and file-backed packages import it through `pi.module.require`
-- exactly like the other public modules; it depends on `pi.packages`.
do
local pi = ...

local RESOURCE_TYPES = { "extensions", "skills", "prompts", "themes" }
local IGNORE_FILE_NAMES = { ".gitignore", ".ignore", ".fdignore" }

-- The resolved package-manager module (injected by the module graph).
local PM

-- ---------------------------------------------------------------------------
-- Path helpers
-- ---------------------------------------------------------------------------
local function posix(p) return (p:gsub("\\", "/")) end

local function basename(p)
  local clean = (p:gsub("/+$", ""))
  local _, _, last = clean:match("^(.*)/([^/]+)$")
  return last or clean
end

local function dirname(p)
  local clean = (p:gsub("/+$", ""))
  local _, _, d = clean:match("^(.*)/([^/]+)$")
  return (d == "" and "/" or d)
end

-- ---------------------------------------------------------------------------
-- Canonical path + precedence
-- ---------------------------------------------------------------------------
local function canonicalize(p)
  local ok, real = pcall(pi.fs.realpath, p)
  if ok and real and real ~= "" then return real end
  return pi.path.resolve(p)
end

-- Spec resourcePrecedenceRank: lower rank = higher precedence.
--   0 project+settings  1 project+auto  2 user+settings  3 user+auto  4 package
local function precedence_rank(m)
  if (m.origin or "top-level") == "package" then return 4 end
  local scope_base = (m.scope == "project") and 0 or 2
  local source_base = (m.source == "settings") and 0 or 1
  return scope_base + source_base
end

-- ---------------------------------------------------------------------------
-- Ignore matcher + pattern application (spec: ignore package + applyPatterns)
-- ---------------------------------------------------------------------------
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
  if pattern:sub(1, 1) == "/" then pattern = pattern:sub(2) end
  local prefixed = (prefix and prefix ~= "") and (prefix .. pattern) or pattern
  return (negated and ("!" .. prefixed) or prefixed)
end

local Matcher = {}
Matcher.__index = Matcher
function Matcher.new() return setmetatable({ patterns = {} }, Matcher) end
function Matcher:add(lines)
  for _, l in ipairs(lines) do if l then self.patterns[#self.patterns + 1] = l end end
end
local function pat_matches(pattern, rel_path)
  if pattern:find("**", 1, true) then
    local base = pattern:sub(1, pattern:find("**", 1, true) - 1):gsub("%.$", "")
    if base == "" or rel_path:sub(1, #base) == base then return true end
    return false
  end
  if pattern:find("*", 1, true) then
    local lp = pattern
      :gsub("[%(%)%+%-%.]", function(c) return "%" .. c end)
      :gsub("%*", "[^/]*")
      :gsub("%?", "[^/]")
    return rel_path == pattern or rel_path:match("^" .. lp .. "$") ~= nil
  end
  return rel_path == pattern
end
function Matcher:ignores(rel_path)
  local matched = false
  for _, pat in ipairs(self.patterns) do
    if pat:sub(1, 1) == "!" then
      if pat_matches(pat:sub(2), rel_path) then matched = false end
    elseif pat_matches(pat, rel_path) then
      matched = true
    end
  end
  return matched
end
local function add_ignore_rules(ig, dir, root)
  local rel_dir = pi.path.relative(root, dir)
  local prefix = (rel_dir ~= "" and rel_dir ~= ".") and (posix(rel_dir) .. "/") or ""
  for _, filename in ipairs(IGNORE_FILE_NAMES) do
    local ip = pi.path.join(dir, filename)
    local ok, exists = pcall(pi.fs.exists, ip)
    if ok and exists then
      local rok, content = pcall(pi.fs.read_file, ip)
      if rok then
        local lines = {}
        for l in content:gmatch("[^\r\n]+") do
          local p = prefix_ignore_pattern(l, prefix)
          if p then lines[#lines + 1] = p end
        end
        ig:add(lines)
      end
    end
  end
end

local function matches_rel(pattern, rel)
  if pattern:find("*", 1, true) or pattern:find("?", 1, true) then
    local lp = pattern
      :gsub("[%(%)%+%-%.]", function(c) return "%" .. c end)
      :gsub("%*", ".*")
      :gsub("%?", ".")
    return rel:match("^" .. lp .. "$") ~= nil
  end
  return pattern == rel
end

local function pattern_matches_entry(file_path, pattern, base_dir)
  local rel = posix(pi.path.relative(base_dir, file_path))
  local name = basename(file_path)
  return matches_rel(pattern, rel) or matches_rel(pattern, name)
end

-- applyPatterns (spec): plain includes (all if none), `!` excludes, `+`
-- force-include, `-` force-exclude. Returns map path->boolean (only true).
local function apply_patterns(all_paths, patterns, base_dir)
  local result = {}
  local includes, excludes, force_includes, force_excludes = {}, {}, {}, {}
  for _, p in ipairs(patterns) do
    local c = p:sub(1, 1)
    if c == "+" then force_includes[#force_includes + 1] = p:sub(2)
    elseif c == "-" then force_excludes[#force_excludes + 1] = p:sub(2)
    elseif c == "!" then excludes[#excludes + 1] = p:sub(2)
    else includes[#includes + 1] = p end
  end

  local keep = {}
  if #includes == 0 then
    for _, f in ipairs(all_paths) do keep[f] = true end
  else
    for _, f in ipairs(all_paths) do
      for _, p in ipairs(includes) do
        if pattern_matches_entry(f, p, base_dir) then keep[f] = true break end
      end
    end
  end
  for _, p in ipairs(excludes) do
    for _, f in ipairs(all_paths) do
      if pattern_matches_entry(f, p, base_dir) then keep[f] = nil end
    end
  end
  for _, p in ipairs(force_includes) do
    for _, f in ipairs(all_paths) do
      if pattern_matches_entry(f, p, base_dir) then keep[f] = true end
    end
  end
  for _, p in ipairs(force_excludes) do
    for _, f in ipairs(all_paths) do
      if pattern_matches_entry(f, p, base_dir) then keep[f] = nil end
    end
  end
  return keep
end

-- ---------------------------------------------------------------------------
-- Directory collection
-- ---------------------------------------------------------------------------
-- collectFiles: recurse `dir`, skip dotfiles/node_modules/ignored, collect
-- files matching `is_target(filename)`.
local function collect_files(dir, is_target, opts)
  opts = opts or {}
  local out = {}
  local ok, entries = pcall(pi.fs.read_dir, dir)
  if not ok or entries == nil then return out end
  table.sort(entries)
  local root = opts.root or dir
  for _, entry in ipairs(entries) do
    if entry:sub(1, 1) == "." then goto continue end
    if entry == "node_modules" then goto continue end
    local full = pi.path.join(dir, entry)
    local ok_stat, st = pcall(pi.fs.stat, full)
    if not ok_stat then goto continue end
    local rel = posix(pi.path.relative(root, full))
    if st.type == "dir" then
      if opts.ig and opts.ig:ignores(rel .. "/") then goto continue end
      local sub = collect_files(full, is_target, opts)
      for _, f in ipairs(sub) do out[#out + 1] = f end
    elseif st.type == "file" and is_target(entry) then
      if not (opts.ig and opts.ig:ignores(rel)) then out[#out + 1] = full end
    end
    ::continue::
  end
  return out
end

-- collectSkillEntries: a dir with SKILL.md is a root (consume children);
-- `pi` mode also collects inline `.md` at the root; otherwise recurse.
local function collect_skills(dir, mode, opts)
  opts = opts or {}
  local out = {}
  local ok, entries = pcall(pi.fs.read_dir, dir)
  if not ok or entries == nil then return out end
  table.sort(entries)
  local root = opts.root or dir
  for _, entry in ipairs(entries) do
    if entry == "SKILL.md" then
      local full = pi.path.join(dir, entry)
      local rel = posix(pi.path.relative(root, full))
      local ok_stat, st = pcall(pi.fs.stat, full)
      if ok_stat and st.type == "file"
        and not (opts.ig and opts.ig:ignores(rel)) then
        out[#out + 1] = full
        return out
      end
    end
  end
  for _, entry in ipairs(entries) do
    if entry:sub(1, 1) == "." then goto continue end
    if entry == "node_modules" then goto continue end
    local full = pi.path.join(dir, entry)
    local ok_stat, st = pcall(pi.fs.stat, full)
    if not ok_stat then goto continue end
    local rel = posix(pi.path.relative(root, full))
    if st.type == "file" then
      if mode == "pi" and dir == root and entry:sub(-3) == ".md"
        and entry ~= "SKILL.md" and not (opts.ig and opts.ig:ignores(rel)) then
        out[#out + 1] = full
      end
    elseif st.type == "dir" then
      if not (opts.ig and opts.ig:ignores(rel .. "/")) then
        local sub = collect_skills(full, mode, opts)
        for _, s in ipairs(sub) do out[#out + 1] = s end
      end
    end
    ::continue::
  end
  return out
end

local function read_pi_manifest(root)
  local p = pi.path.join(root, "package.json")
  local ok, exists = pcall(pi.fs.exists, p)
  if not (ok and exists) then return nil end
  local rok, content = pcall(pi.fs.read_file, p)
  if not rok then return nil end
  local okj, parsed = pcall(pi.json.decode, content)
  if not okj or type(parsed) ~= "table" then return nil end
  local mf = parsed.pi
  return type(mf) == "table" and mf or nil
end

-- resolveExtensionEntries: package.json `pi.extensions`, else index.lua.
local function resolve_extension_entries(dir)
  local pkg_path = pi.path.join(dir, "package.json")
  local ok, exists = pcall(pi.fs.exists, pkg_path)
  if ok and exists then
    local rok, content = pcall(pi.fs.read_file, pkg_path)
    if rok then
      local okj, parsed = pcall(pi.json.decode, content)
      if okj and type(parsed) == "table" and type(parsed.pi) == "table"
        and type(parsed.pi.extensions) == "table" and #parsed.pi.extensions > 0 then
        local entries = {}
        for _, rel in ipairs(parsed.pi.extensions) do
          local full = pi.path.is_absolute(rel) and pi.path.normalize(rel)
            or pi.path.join(dir, rel)
          local e, eok = pcall(pi.fs.exists, full)
          if eok and e then entries[#entries + 1] = full end
        end
        if #entries > 0 then return entries end
      end
    end
  end
  local index = pi.path.join(dir, "index.lua")
  local ok2, exists2 = pcall(pi.fs.exists, index)
  if ok2 and exists2 then return { index } end
  return nil
end

local function collect_auto_extensions(dir)
  local out = {}
  local ok_dir, ok = pcall(pi.fs.read_dir, dir)
  if not (ok_dir and ok) then return out end
  local root_entries = resolve_extension_entries(dir)
  if root_entries then return root_entries end
  table.sort(ok)
  local ig = Matcher.new()
  add_ignore_rules(ig, dir, dir)
  for _, entry in ipairs(ok) do
    if entry:sub(1, 1) == "." then goto continue end
    if entry == "node_modules" then goto continue end
    local full = pi.path.join(dir, entry)
    local ok_stat, st = pcall(pi.fs.stat, full)
    if not ok_stat then goto continue end
    local rel = posix(pi.path.relative(dir, full))
    if st.type == "file" and entry:sub(-4) == ".lua" then
      if not ig:ignores(rel) then out[#out + 1] = full end
    elseif st.type == "dir" then
      if not ig:ignores(rel .. "/") then
        local sub = resolve_extension_entries(full)
        if sub then for _, s in ipairs(sub) do out[#out + 1] = s end end
      end
    end
    ::continue::
  end
  return out
end

-- collectResourceFiles per type.
local function collect_resource_files(dir, resource_type)
  if resource_type == "skills" then
    local ig = Matcher.new()
    add_ignore_rules(ig, dir, dir)
    return collect_skills(dir, "pi", { ig = ig, root = dir })
  end
  if resource_type == "extensions" then
    return collect_auto_extensions(dir)
  end
  if resource_type == "themes" then
    local ig = Matcher.new()
    add_ignore_rules(ig, dir, dir)
    return collect_files(dir, function(name) return name:sub(-5) == ".json" end,
      { ig = ig, root = dir })
  end
  local ig = Matcher.new()
  add_ignore_rules(ig, dir, dir)
  return collect_files(dir, function(name) return name:sub(-3) == ".md" end,
    { ig = ig, root = dir })
end

-- collectAncestorAgentsSkillDirs up to git root.
local function collect_ancestor_agents_skills(start_dir)
  local out = {}
  local dir = pi.path.resolve(start_dir)
  local git_root
  local probe = dir
  while true do
    local ok, exists = pcall(pi.fs.exists, pi.path.join(probe, ".git"))
    if ok and exists then git_root = probe break end
    local parent = pi.path.dirname(probe)
    if parent == probe then break end
    probe = parent
  end
  local current = dir
  while true do
    out[#out + 1] = pi.path.join(current, ".agents", "skills")
    if git_root and current == git_root then break end
    local parent = pi.path.dirname(current)
    if parent == current then break end
    current = parent
  end
  return out
end

-- ---------------------------------------------------------------------------
-- Accumulator + precedence sort
-- ---------------------------------------------------------------------------
local function new_accumulator()
  local acc = {}
  for _, t in ipairs(RESOURCE_TYPES) do acc[t] = {} end
  return acc
end

local function add_resource(acc, resource_type, path, metadata, enabled)
  if not path or path == "" then return end
  local key = canonicalize(path)
  if acc[resource_type][key] then return end
  acc[resource_type][key] = { path = path, metadata = metadata, enabled = enabled }
end

local function to_resolved(entries)
  local resolved = {}
  for _, entry in pairs(entries) do
    resolved[#resolved + 1] = {
      path = entry.path,
      enabled = entry.enabled,
      metadata = entry.metadata,
      precedence = precedence_rank(entry.metadata),
    }
  end
  table.sort(resolved, function(a, b)
    if a.precedence ~= b.precedence then return a.precedence < b.precedence end
    if a.metadata.scope ~= b.metadata.scope then
      return a.metadata.scope == "project"
    end
    return a.path < b.path
  end)
  return resolved
end

-- ---------------------------------------------------------------------------
-- Package resource collection
-- ---------------------------------------------------------------------------
local function glob_files(root, entries, resource_type, base_dir)
  -- Expand non-override manifest entries (exact paths; glob via files scan).
  local all = {}
  for _, entry in ipairs(entries) do
    if entry:sub(1, 1) ~= "!" and entry:sub(1, 1) ~= "+" and entry:sub(1, 1) ~= "-" then
      local full = pi.path.is_absolute(entry) and pi.path.normalize(entry)
        or pi.path.join(root, entry)
      local ok, exists = pcall(pi.fs.exists, full)
      if ok and exists then
        local ok_stat, st = pcall(pi.fs.stat, full)
        if ok_stat then
          if st.type == "file" then
            all[#all + 1] = full
          elseif st.type == "dir" then
            local files = collect_resource_files(full, resource_type)
            for _, f in ipairs(files) do all[#all + 1] = f end
          end
        end
      end
    end
  end
  return all
end

local function add_manifest_entries(entries, root, resource_type, acc, metadata)
  if not entries or #entries == 0 then return end
  local all = glob_files(root, entries, resource_type, root)
  local enabled = apply_patterns(all, entries, root)
  for _, f in ipairs(all) do
    add_resource(acc, resource_type, f, metadata, enabled[f] or false)
  end
end

local function collect_default_resources(root, resource_type, acc, metadata)
  local manifest = read_pi_manifest(root)
  local entries = manifest and manifest[resource_type]
  if entries and #entries > 0 then
    add_manifest_entries(entries, root, resource_type, acc, metadata)
    return
  end
  local dir = pi.path.join(root, resource_type)
  local ok, exists = pcall(pi.fs.exists, dir)
  if ok and exists then
    local files = collect_resource_files(dir, resource_type)
    for _, f in ipairs(files) do add_resource(acc, resource_type, f, metadata, true) end
  end
end

local function collect_manifest_files(root, resource_type)
  local manifest = read_pi_manifest(root)
  local entries = manifest and manifest[resource_type]
  if entries and #entries > 0 then
    return glob_files(root, entries, resource_type, root)
  end
  local dir = pi.path.join(root, resource_type)
  local ok, exists = pcall(pi.fs.exists, dir)
  if ok and exists then return collect_resource_files(dir, resource_type) end
  return {}
end

local function apply_package_filter(root, patterns, resource_type, acc, metadata)
  local all = collect_manifest_files(root, resource_type)
  if #patterns == 0 then
    for _, f in ipairs(all) do add_resource(acc, resource_type, f, metadata, false) end
    return
  end
  local enabled = apply_patterns(all, patterns, root)
  for _, f in ipairs(all) do
    add_resource(acc, resource_type, f, metadata, enabled[f] or false)
  end
end

local function collect_package_dir(root, acc, filter, metadata)
  if type(filter) == "table" then
    for _, resource_type in ipairs(RESOURCE_TYPES) do
      if filter[resource_type] ~= nil then
        apply_package_filter(root, filter[resource_type], resource_type, acc, metadata)
      else
        collect_default_resources(root, resource_type, acc, metadata)
      end
    end
    return true
  end
  local manifest = read_pi_manifest(root)
  if manifest then
    for _, resource_type in ipairs(RESOURCE_TYPES) do
      add_manifest_entries(manifest[resource_type], root, resource_type, acc, metadata)
    end
    return true
  end
  local has_any = false
  for _, resource_type in ipairs(RESOURCE_TYPES) do
    local dir = pi.path.join(root, resource_type)
    local ok, exists = pcall(pi.fs.exists, dir)
    if ok and exists then
      local files = collect_resource_files(dir, resource_type)
      for _, f in ipairs(files) do add_resource(acc, resource_type, f, metadata, true) end
      has_any = true
    end
  end
  return has_any
end

-- Collect a configured package's resources for a scope.
local function collect_package_resources(scope, source, acc, cwd, agent_dir)
  local parsed = PM.parse_source(source)
  local metadata = { source = source, scope = scope, origin = "package" }
  if parsed.type == "local" then
    -- Resolve against the scope base dir (spec getBaseDirForScope +
    -- resolveSourcePath): project-local sources live under `.pi`, user-local
    -- under the agent dir. This must agree with pi.packages.get_installed_path
    -- so a configured local package is found by the resolver.
    local base = (scope == "project") and pi.path.join(cwd, ".pi") or (agent_dir or "")
    local path = parsed.path
    local resolved
    if pi.path.is_absolute(path) then
      resolved = pi.path.normalize(path)
    else
      local stripped = (path:sub(1, 2) == "./") and path:sub(3) or path
      resolved = pi.path.resolve(pi.path.join(base, stripped))
    end
    local ok, exists = pcall(pi.fs.exists, resolved)
    if not (ok and exists) then return end
    local ok_stat, st = pcall(pi.fs.stat, resolved)
    if ok_stat then
      if st.type == "file" then
        metadata.baseDir = pi.path.dirname(resolved)
        add_resource(acc, "extensions", resolved, metadata, true)
        return
      end
      metadata.baseDir = resolved
      local okc = collect_package_dir(resolved, acc, nil, metadata)
      if not okc then
        add_resource(acc, "extensions", resolved, metadata, true)
      end
    end
    return
  end
  local installed = PM.get_installed_path(source, scope, cwd, agent_dir)
  if not installed then return end
  metadata.baseDir = installed
  collect_package_dir(installed, acc, nil, metadata)
end

-- ---------------------------------------------------------------------------
-- Settings entries (per scope) + auto-discovery
-- ---------------------------------------------------------------------------
local function settings_paths(resource_type, scope)
  local key = resource_type:sub(1, #resource_type - 1) -- extensions->extension
  local fn = (scope == "project") and ("project_" .. key .. "_paths")
    or (key .. "_paths")
  local ok, f = pcall(function() return pi.settings[fn] end)
  if not (ok and type(f) == "function") then return {} end
  local r, v = pcall(f)
  if r and type(v) == "table" then
    local out = {}
    for _, item in ipairs(v) do out[#out + 1] = item end
    return out
  end
  return {}
end

local function add_settings_entries(paths, resource_type, acc, metadata, base_dir)
  if #paths == 0 then return end
  local plain, patterns = {}, {}
  for _, p in ipairs(paths) do
    local c = p:sub(1, 1)
    if c == "!" or c == "+" or c == "-" or p:find("*", 1, true)
      or p:find("?", 1, true) then
      patterns[#patterns + 1] = p
    else
      plain[#plain + 1] = p
    end
  end
  -- Extend patterns with plain paths for filter inclusion.
  local all_patterns = {}
  for _, p in ipairs(plain) do all_patterns[#all_patterns + 1] = p end
  for _, p in ipairs(patterns) do all_patterns[#all_patterns + 1] = p end

  local all_files = {}
  for _, p in ipairs(plain) do
    local resolved
    if pi.path.is_absolute(p) then
      resolved = pi.path.normalize(p)
    else
      -- Spec resolvePathFromBase: resolve against the scope base dir (with a
      -- leading `./` removed so it is treated as relative-to-base).
      local stripped = (p:sub(1, 2) == "./") and p:sub(3) or p
      resolved = pi.path.resolve(pi.path.join(base_dir, stripped))
    end
    local ok, exists = pcall(pi.fs.exists, resolved)
    if ok and exists then
      local ok_stat, st = pcall(pi.fs.stat, resolved)
      if ok_stat then
        if st.type == "file" then all_files[#all_files + 1] = resolved
        elseif st.type == "dir" then
          local files = collect_resource_files(resolved, resource_type)
          for _, f in ipairs(files) do all_files[#all_files + 1] = f end
        end
      end
    end
  end

  -- Enabled: every collected file unless excluded; pattern filters apply.
  local enabled = apply_patterns(all_files, all_patterns, base_dir)
  for _, f in ipairs(all_files) do
    local on = true
    if enabled[f] ~= nil then on = enabled[f] or false end
    add_resource(acc, resource_type, f, metadata, on)
  end
end

local function add_auto(resource_type, dir, metadata, acc)
  local ok, exists = pcall(pi.fs.exists, dir)
  if not (ok and exists) then return end
  local files = collect_resource_files(dir, resource_type)
  for _, f in ipairs(files) do add_resource(acc, resource_type, f, metadata, true) end
end

local function add_auto_discovered(acc, cwd, agent_dir, home, project_trusted, global_base, projects_base)
  local user_meta = { source = "auto", scope = "user", origin = "top-level", baseDir = global_base }
  local project_meta = { source = "auto", scope = "project", origin = "top-level", baseDir = projects_base }

  local user_dirs = {
    extensions = pi.path.join(global_base, "extensions"),
    skills = pi.path.join(global_base, "skills"),
    prompts = pi.path.join(global_base, "prompts"),
    themes = pi.path.join(global_base, "themes"),
  }
  local project_dirs = {
    extensions = pi.path.join(projects_base, "extensions"),
    skills = pi.path.join(projects_base, "skills"),
    prompts = pi.path.join(projects_base, "prompts"),
    themes = pi.path.join(projects_base, "themes"),
  }
  local user_agents_skills = pi.path.join(home, ".agents", "skills")
  local project_agents = project_trusted and collect_ancestor_agents_skills(cwd) or {}

  if project_trusted then
    add_auto("extensions", project_dirs.extensions, project_meta, acc)
    add_auto("skills", project_dirs.skills, project_meta, acc)
  end
  for _, agents_dir in ipairs(project_agents) do
    local agents_meta = { source = "auto", scope = "project", origin = "top-level",
      baseDir = pi.path.dirname(agents_dir) }
    add_auto("skills", agents_dir, agents_meta, acc)
  end
  if project_trusted then
    add_auto("prompts", project_dirs.prompts, project_meta, acc)
    add_auto("themes", project_dirs.themes, project_meta, acc)
  end

  add_auto("extensions", user_dirs.extensions, user_meta, acc)
  add_auto("skills", user_dirs.skills, user_meta, acc)
  add_auto("skills", user_agents_skills, {
    source = "auto", scope = "user", origin = "top-level",
    baseDir = pi.path.dirname(user_agents_skills),
  }, acc)
  add_auto("prompts", user_dirs.prompts, user_meta, acc)
  add_auto("themes", user_dirs.themes, user_meta, acc)
end

-- ---------------------------------------------------------------------------
-- resolve
-- ---------------------------------------------------------------------------
-- options: { cwd, agentDir, home, projectTrusted }
-- returns: { extensions/skills/prompts/themes = [ { path, enabled, metadata, precedence } ] }
local function resolve(options)
  local cwd = options.cwd or pi.cwd()
  local agent_dir = options.agentDir
  local home = options.home or ""
  if home == "" and os.getenv then home = os.getenv("HOME") or "" end
  local project_trusted = options.projectTrusted
  if project_trusted == nil then
    local ok = pcall(pi.settings.is_project_trusted)
    project_trusted = ok and pi.settings.is_project_trusted() or false
  end

  local acc = new_accumulator()
  local projects_base = pi.path.join(cwd, ".pi")
  local global_base = agent_dir and agent_dir or pi.path.dirname(projects_base)

  -- 1. Packages: project scope first (wins collisions), then user.
  local configured_packages = PM.list_configured_packages(cwd, agent_dir)
  for _, scope in ipairs({ "project", "user" }) do
    for _, pkg in ipairs(configured_packages) do
      if pkg.scope == scope then
        if scope == "project" and not project_trusted then goto continue end
        collect_package_resources(scope, pkg.source, acc, cwd, agent_dir)
        ::continue::
      end
    end
  end

  -- 2. Settings entries per resource type: project then user.
  for _, resource_type in ipairs(RESOURCE_TYPES) do
    if project_trusted then
      add_settings_entries(
        settings_paths(resource_type, "project"), resource_type, acc,
        { source = "settings", scope = "project", origin = "top-level", baseDir = projects_base },
        projects_base)
    end
    add_settings_entries(
      settings_paths(resource_type, "user"), resource_type, acc,
      { source = "settings", scope = "user", origin = "top-level", baseDir = global_base },
      global_base)
  end

  -- 3. Auto-discovery.
  add_auto_discovered(acc, cwd, agent_dir, home, project_trusted, global_base, projects_base)

  local out = {}
  for _, resource_type in ipairs(RESOURCE_TYPES) do
    out[resource_type] = to_resolved(acc[resource_type])
  end
  return out
end

-- ---------------------------------------------------------------------------
-- Theme registry (spec: theme.ts setRegisteredThemes/getAvailableThemesWithPaths
-- getThemeByName). `pi.resources` owns the disk theme registry so the
-- interactive runtime and extensions resolve custom `.json` themes the same
-- way, and `/reload` re-populates it from the resolved theme paths.
-- ---------------------------------------------------------------------------
local theme_registry = {} -- path -> parsed theme table { name, colors, vars? }
local theme_name_to_path = {} -- name -> canonical path (first wins)
local theme_order = {} -- canonical path insertion order (stable list)
local theme_by_name = {} -- name -> resolved preference (set once, first wins)

local function register_theme(name, data, source_path)
  source_path = source_path or (data and data.sourcePath)
  local key = source_path and source_path ~= "" and canonicalize(source_path) or name
  if theme_registry[key] then return end
  theme_registry[key] = data
  if source_path and source_path ~= "" then
    theme_order[#theme_order + 1] = key
  end
  local nm = (type(data) == "table" and data.name) or name
  if nm and theme_name_to_path[nm] == nil and source_path and source_path ~= "" then
    theme_name_to_path[nm] = key
  end
end

local function get_available_themes()
  -- Built-ins plus registered custom themes, name-sorted (spec getAvailableThemesWithPaths).
  local names = { "dark", "light" }
  local seen = { dark = true, light = true }
  for _, key in ipairs(theme_order) do
    local data = theme_registry[key]
    local nm = (type(data) == "table" and data.name) or nil
    if nm and not seen[nm] then seen[nm] = true names[#names + 1] = nm end
  end
  table.sort(names)
  return names
end

-- Register/resolve a theme by name; returns nil when unknown (spec getThemeByName).
local function get_theme(name)
  if name == "dark" or name == "light" then return nil end
  local key = theme_name_to_path[name]
  if not key then return nil end
  return theme_registry[key]
end

local function has_theme(name)
  return get_theme(name) ~= nil
end

-- load_theme_from_path: read a `.json` theme file and validate/register it.
-- Returns { theme = {name,colors,path} } or { error = message } (spec
-- loadThemeFromPath / parseThemeJson). Deterministic and hermetic.
local function load_theme_from_path(source_path, mode)
  local ok, exists = pcall(pi.fs.exists, source_path)
  if not (ok and exists) then
    return { error = "Theme file does not exist: " .. tostring(source_path) }
  end
  local rok, content = pcall(pi.fs.read_file, source_path)
  if not rok then
    return { error = "Failed to read theme file " .. tostring(source_path) }
  end
  local okj, parsed = pcall(pi.json.decode, content)
  if not okj or type(parsed) ~= "table" then
    return { error = "Failed to parse theme " .. tostring(source_path) .. ": " .. tostring(parsed) }
  end
  if type(parsed.name) ~= "string" then
    return { error = 'Invalid theme: /name: Expected string' }
  end
  if type(parsed.colors) ~= "table" then
    return { error = 'Invalid theme "' .. parsed.name .. '": /colors: Expected object' }
  end
  parsed.sourcePath = source_path
  parsed.mode = mode
  register_theme(parsed.name, parsed, source_path)
  return { theme = parsed }
end

-- Rebuild the theme registry from a list of resolved theme paths.
-- Returns { themes = { { name, path } }, diagnostics = { ... } }.
local function sync_themes(resolved_paths)
  local themes, diagnostics = {}, {}
  for _, entry in ipairs(resolved_paths or {}) do
    if entry.path and entry.path:sub(-5) == ".json" then
      local res = load_theme_from_path(entry.path)
      if res.error then
        diagnostics[#diagnostics + 1] = { type = "error", message = res.error, path = entry.path }
      elseif res.theme then
        themes[#themes + 1] = { name = res.theme.name, path = entry.path }
      end
    end
  end
  table.sort(themes, function(a, b) return a.name < b.name end)
  return { themes = themes, diagnostics = diagnostics }
end

-- ---------------------------------------------------------------------------
-- Module
-- ---------------------------------------------------------------------------
pi.module.define({
  name = "pi.resources",
  version = "1",
  dependencies = {
    package_manager = { name = "pi.packages", version = "1" },
  },
  factory = function(deps)
    PM = deps and deps.package_manager
    return {
      resolve = resolve,
      precedence_rank = precedence_rank,
      canonicalize = canonicalize,
      apply_patterns = apply_patterns,
      collect_resource_files = collect_resource_files,
      collect_skills = collect_skills,
      collect_files = collect_files,
      register_theme = register_theme,
      get_available_themes = get_available_themes,
      get_theme = get_theme,
      has_theme = has_theme,
      load_theme_from_path = load_theme_from_path,
      sync_themes = sync_themes,
    }
  end,
})
end