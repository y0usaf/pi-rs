-- core/package-manager.ts — the package lifecycle policy, exposed as the
-- public `pi.packages` module. Embedded and file-backed packages use the same
-- module. Persistence goes through `pi.settings` (the canonical
-- SettingsManager packages channel); source parsing reuses `pi.git` grammar
-- for git/npm routing and `pi.path`/`pi.fs` for local paths.
--
-- Transports (DESIGN, extension distribution): npm-registry (npm CLI archive
-- install), Git URL/ref (git clone/checkout), and local path. Package
-- contents stay Lua/modules/data; package JavaScript (package.json `pi`
-- manifest aside) is never evaluated — it is inert package metadata.
do
local pi = ...

-- parseSource -> { type = "npm", spec, name, pinned } | { type = "git", repo,
-- host, path, ref, pinned } | { type = "local", path }.
-- npm: strips the `npm:` prefix and splits spec/name@version.
local function parse_npm_spec(spec)
  -- Pi's parseNpmSpec regex: /^(@?[^@]+(?:\/[^@]+)?)(?:@(.+))?$/
  -- Lua has no non-capturing groups, so emulate the split: an optional leading
  -- scope (`@user/name`) then an optional version separator (`@...`). The
  -- version separator is the first `@` at/after the base name — never inside
  -- the scope account.
  local function split_at(at)
    local name, version = spec:sub(1, at - 1), spec:sub(at + 1)
    if name == "" or version == "" then
      return { name = spec }
    end
    return { name = name, version = version }
  end
  if spec:sub(1, 1) == "@" then
    -- Scoped: `@user/name` or `@user/name@version`. Find the `/` after the
    -- scope account; the version separator is the first `@` at/after it.
    local slash = spec:find("/", 1, true)
    local from
    if slash then
      from = slash
    else
      -- `@user` or `@user@version` (no `/`): version separator (if any) is the
      -- first `@` after position 1.
      from = 1
    end
    local at = spec:find("@", from + 1, true)
    if at then return split_at(at) end
    return { name = spec }
  end
  -- Unscoped: split at the first `@` (version separator).
  local at = spec:find("@", 1, true)
  if at then return split_at(at) end
  return { name = spec }
end

local function is_local_path(value)
  local trimmed = value:gsub("^%s+", ""):gsub("%s+$", "")
  return not (trimmed:match("^npm:") or trimmed:match("^git:")
    or trimmed:match("^github:") or trimmed:match("^http:")
    or trimmed:match("^https:") or trimmed:match("^ssh:"))
end

-- Reject npm package names that would escape the managed `<base>/npm/node_modules`
-- root (mirrors the git host/path safety set: NUL, backslash, absolute, `..`).
local function unsafe_npm_name(name)
  if not name or name == "" then return true end
  if name:find("\0", 1, true) or name:find("\\", 1, true) then return true end
  if name:find(":", 1, true) then return true end
  if name:sub(1, 1) == "/" then return true end
  for seg in name:gmatch("[^/]+") do
    if seg == ".." then return true end
  end
  return false
end

-- parseSource mirrors package-manager.ts: npm: prefix, isLocalPath, else git.
local function parse_source(source)
  if source:sub(1, 4) == "npm:" then
    local spec = source:sub(5):gsub("^%s+", ""):gsub("%s+$", "")
    local parsed = parse_npm_spec(spec)
    return { type = "npm", spec = spec, name = parsed.name, pinned = parsed.version ~= nil }
  end
  if is_local_path(source) then
    return { type = "local", path = source }
  end
  -- Try the git grammar (exposed by the host's pi.git source grammar).
  local ok, git_parsed = pcall(function()
    if pi.git then return pi.git.parse_git_url(source) end
    return nil
  end)
  if ok and git_parsed then
    return git_parsed
  end
  return { type = "local", path = source }
end

-- A package's stable identity ignoring version/ref (spec getPackageIdentity).
local function package_identity(source, scope)
  local parsed = parse_source(source)
  if parsed.type == "npm" then
    return "npm:" .. parsed.name
  end
  if parsed.type == "git" then
    return "git:" .. parsed.host .. "/" .. parsed.path
  end
  return "local:" .. (parsed.path or source)
end

-- Resolve a local path against the scope base dir (project .pi or agentDir).
local function resolve_source_path(parsed, cwd, agent_dir, scope)
  local base = cwd
  if scope == "project" then
    base = pi.path.join(cwd, ".pi")
  elseif scope == "user" then
    base = agent_dir
  end
  local resolved = pi.path.is_absolute(parsed.path or "") and pi.path.normalize(parsed.path or "")
    or pi.path.resolve(base, parsed.path or "")
  return resolved
end

-- Project packages aren't directly exposed by `pi.settings.packages`; read the
-- resolved project settings the SettingsManager holds (spec readProjectPackages
-- reads projectSettings.packages) so a project `.pi/config.lua` declaration is
-- honored, not just a raw `.pi/settings.json` file.
local function read_project_packages(cwd)
  local ok, entries = pcall(function()
    if pi.settings.project_packages then return pi.settings.project_packages() end
    return nil
  end)
  if ok and type(entries) == "table" then
    local out = {}
    for _, entry in ipairs(entries) do out[#out + 1] = entry end
    return out
  end
  local path = pi.path.join(cwd, ".pi", "settings.json")
  local ok_exists, exists = pcall(pi.fs.exists, path)
  if not (ok_exists and exists) then return {} end
  local ok_read, content = pcall(pi.fs.read_file, path)
  if not ok_read then return {} end
  local ok_json, parsed = pcall(pi.json.decode, content)
  if not ok_json then return {} end
  local packages = parsed.packages or {}
  return packages
end

-- packages channel helpers over pi.settings. User scope reads the global
-- settings packages; project scope reads the resolved project settings
-- (spec: listConfiguredPackages uses getGlobalSettings()/getProjectSettings()).
local CHANNELS = {
  user = { get = function()
    if pi.settings.global_packages then return pi.settings.global_packages() end
    return pi.settings.packages()
  end,
           set = function(v) pi.settings.set_packages(v) end },
  project = { get = function() return read_project_packages(pi.cwd()) end,
              set = function(v) pi.settings.set_project_packages(v) end },
}

local function source_of(entry)
  if type(entry) == "string" then return entry end
  return entry and entry.source
end

-- addSourceToSettings: add/replace a source in the scope's packages list,
-- keyed by normalized identity. Returns true when a change was made.
local function add_source_to_settings(source, options)
  local scope = (options and options["local"]) and "project" or "user"
  local existing = CHANNELS[scope].get()
  local found = nil
  for i, entry in ipairs(existing) do
    if source_of(entry) == source then found = i break end
  end
  if found then return false end
  existing[#existing + 1] = source
  CHANNELS[scope].set(existing)
  return true
end

-- removeSourceFromSettings: drop every entry matching source. Returns true on
-- change.
local function remove_source_from_settings(source, options)
  local scope = (options and options["local"]) and "project" or "user"
  local existing = CHANNELS[scope].get()
  local next_list = {}
  local changed = false
  for _, entry in ipairs(existing) do
    if source_of(entry) == source then
      changed = true
    else
      next_list[#next_list + 1] = entry
    end
  end
  if not changed then return false end
  CHANNELS[scope].set(next_list)
  return true
end

-- getInstalledPath for a source + scope (declared before list so it is an
-- upvalue, not a forward global).
local get_installed_path

-- listConfiguredPackages: [{ source, scope, filtered, installedPath }].
local function list_configured_packages(cwd, agent_dir)
  local out = {}
  local user_packages = CHANNELS.user.get()
  for _, entry in ipairs(user_packages) do
    local s = source_of(entry)
    if s then
      out[#out + 1] = { source = s, scope = "user",
        filtered = type(entry) == "table",
        installedPath = get_installed_path(s, "user", cwd, agent_dir) }
    end
  end
  local project_packages = read_project_packages(cwd)
  for _, entry in ipairs(project_packages) do
    local s = source_of(entry)
    if s then
      out[#out + 1] = { source = s, scope = "project",
        filtered = type(entry) == "table",
        installedPath = get_installed_path(s, "project", cwd, agent_dir) }
    end
  end
  return out
end

-- getInstalledPath for a source + scope; local path resolves to the on-disk
-- path (returned when it exists), matching package-manager.ts getInstalledPath.
get_installed_path = function(source, scope, cwd, agent_dir)
  local parsed = parse_source(source)
  if parsed.type == "local" then
    local resolved = resolve_source_path(parsed, cwd, agent_dir, scope)
    local ok, exists = pcall(pi.fs.exists, resolved)
    return (ok and exists) and resolved or nil
  end
  -- npm/git managed paths (offline-safe layout, spec getNpmInstallPath /
  -- getGitInstallPath). npm: <base>/npm/node_modules/<name>; git:
  -- <base>/git/<host>/<path>.
  local base = (scope == "project") and pi.path.join(cwd, ".pi") or agent_dir
  if parsed.type == "npm" then
    if unsafe_npm_name(parsed.name) then return nil end
    local p = pi.path.join(base, "npm", "node_modules", parsed.name)
    local ok, exists = pcall(pi.fs.exists, p)
    return (ok and exists) and p or nil
  end
  if parsed.type == "git" then
    local p = pi.path.join(base, "git", parsed.host, parsed.path)
    local ok, exists = pcall(pi.fs.exists, p)
    return (ok and exists) and p or nil
  end
  return nil
end

-- Sequential install: local path validates existence; npm invokes the
-- configured npm CLI into the managed project root; git clones+checks out.
-- All writes go through pi.fs/pi.exec (public mechanisms), never a JS runtime.
local function install(source, options)
  options = options or {}
  local agent_dir = options.agentDir or pi.cwd()
  local parsed = parse_source(source)
  local scope = (options and options["local"]) and "project" or "user"
  if parsed.type == "local" then
    -- Spec install(): a local source resolves against the package manager's
    -- cwd (this.cwd), not the scope base dir — `resolvePath(parsed.path)`
    -- defaults baseDir to process.cwd() (spec package-manager.ts:977). Only
    -- getInstalledPath / list resolution is scope-base-aware. Tilde expansion
    -- and trimming mirror resolvePath's normalizePath.
    local local_path = parsed.path:gsub("^%s+", ""):gsub("%s+$", "")
    if local_path == "~" then
      local_path = pi.env.HOME or local_path
    elseif local_path:sub(1, 2) == "~/" then
      local_path = (pi.env.HOME or "/") .. local_path:sub(2)
    end
    local resolved = pi.path.is_absolute(local_path) and pi.path.normalize(local_path)
      or pi.path.resolve(pi.cwd(), local_path)
    local ok, exists = pcall(pi.fs.exists, resolved)
    if not (ok and exists) then
      error("Path does not exist: " .. resolved, 0)
    end
    return { installedPath = resolved }
  end
  if parsed.type == "npm" then
    local base = (scope == "project") and pi.path.join(pi.cwd(), ".pi") or agent_dir
    local root = pi.path.join(base, "npm")
    pi.fs.mkdir(root)
    -- npm install <spec> --prefix <root> --legacy-peer-deps (JS inert: package
    -- JavaScript is never evaluated by pi-rs; npm is a lifecycle archive tool).
    -- Guard the spec against option injection (leading `-` would be parsed as
    -- an npm CLI flag, escaping the managed root).
    if parsed.spec:sub(1, 1) == "-" or parsed.spec == "" then
      error(("npm install refused for option-like spec: %s"):format(parsed.spec), 0)
    end
    if unsafe_npm_name(parsed.name) then
      error(("npm install refused for unsafe package name: %s"):format(parsed.name), 0)
    end
    -- Use the configured npm command if present (spec getNpmCommand: an array
    -- of argv tokens, e.g. {"pnpm","--use-stderr"}), defaulting to `npm`; argv
    -- form means no shell interpolation.
    local ok_cmd, configured = pcall(pi.settings.npm_command)
    local tokens = (ok_cmd and type(configured) == "table") and configured or {}
    local first = (tokens[1] ~= nil and tokens[1] ~= "") and tokens[1] or "npm"
    local cmd = first
    local prefix_args = {}
    for i = 2, #tokens do
      if tokens[i] ~= "" then prefix_args[#prefix_args + 1] = tokens[i] end
    end
    local argv = { "install", parsed.spec, "--prefix", root, "--legacy-peer-deps" }
    for i = #prefix_args, 1, -1 do table.insert(argv, 1, prefix_args[i]) end
    local res = pi.exec(cmd, argv, { cwd = agent_dir })
    if res.code ~= 0 then
      error(("npm install failed (code %s): %s"):format(res.code, res.stderr or ""), 0)
    end
    return { installedPath = pi.path.join(root, "node_modules", parsed.name) }
  end
  if parsed.type == "git" then
    local base = (scope == "project") and pi.path.join(pi.cwd(), ".pi") or agent_dir
    local target = pi.path.join(base, "git", parsed.host, parsed.path)
    local ok, exists = pcall(pi.fs.exists, target)
    if not (ok and exists) then
      pi.fs.mkdir(pi.path.dirname(target))
      local res = pi.exec("git", { "clone", parsed.repo, target }, { cwd = pi.cwd() })
      if res.code ~= 0 then
        error(("git clone failed (code %s)"):format(res.code), 0)
      end
      if parsed.ref then
        -- Only a commitish is meaningful as a checkout target; a leading `-`
        -- would be parsed by git as an option (e.g. `--detach` / `-b`),
        -- escaping the managed path. Reject option-like refs upfront.
        if parsed.ref:sub(1, 1) == "-" or parsed.ref == "" then
          error(("git checkout refused for option-like ref: %s"):format(parsed.ref), 0)
        end
        local co = pi.exec("git", { "checkout", parsed.ref }, { cwd = target })
        if co.code ~= 0 then
          error(("git checkout failed (code %s)"):format(co.code), 0)
        end
      end
    end
    return { installedPath = target }
  end
  error("Unsupported install source: " .. source, 0)
end

-- installAndPersist: install then add to settings.
local function install_and_persist(source, options)
  install(source, options)
  add_source_to_settings(source, options)
end

-- removeAndPersist: remove from settings (returns whether a change occurred).
local function remove_and_persist(source, options)
  return remove_source_from_settings(source, options)
end

-- isOfflineModeEnabled (spec core/package-manager.ts): off-by-default flag;
-- when set, every network-modulated leg short-circuits with no pi.exec and no
-- registry contact. The deterministic offline-skip behavior for (un)installed
-- packages is exactly this: sources are collected but no network runs.
local function is_offline_mode_enabled()
  local value = pi.env.PI_OFFLINE
  if value == nil or value == "" then return false end
  local flag = tostring(value):lower()
  return flag == "1" or flag == "true" or flag == "yes" or flag == "on"
end

-- npmUpdate: reinstall `name@latest` into the managed root via pi.exec over the
-- configured npm command. Package JavaScript stays inert (npm is a lifecycle
-- archive tool; pi-rs never evaluates installed JS).
local function npm_update(parsed, scope, cwd, agent_dir)
  local base = (scope == "project") and pi.path.join(cwd, ".pi") or agent_dir
  local root = pi.path.join(base, "npm")
  pi.fs.mkdir(root)
  if parsed.spec:sub(1, 1) == "-" or parsed.spec == "" then
    error(("npm update refused for option-like spec: %s"):format(parsed.spec), 0)
  end
  if unsafe_npm_name(parsed.name) then
    error(("npm update refused for unsafe package name: %s"):format(parsed.name), 0)
  end
  local ok_cmd, configured = pcall(pi.settings.npm_command)
  local tokens = (ok_cmd and type(configured) == "table") and configured or {}
  local first = (tokens[1] ~= nil and tokens[1] ~= "") and tokens[1] or "npm"
  local argv = { "install", parsed.name .. "@latest", "--prefix", root, "--legacy-peer-deps" }
  for i = #tokens, 2, -1 do
    if tokens[i] ~= "" then table.insert(argv, 1, tokens[i]) end
  end
  local res = pi.exec(first, argv, { cwd = agent_dir })
  if res.code ~= 0 then
    error(("npm update failed (code %s): %s"):format(res.code, res.stderr or ""), 0)
  end
  return pi.path.join(root, "node_modules", parsed.name)
end

-- Forward upvalue: install_target is defined below (after git_update) but
-- both git_update and npm install legs share its clone/checkout body.
local install_target

-- gitUpdate: pull/ref-reconcile an existing managed clone (mirrors spec
-- updateGit/ensureGitRef) through pi.exec. A fresh clone (uninstalled
-- package) falls back to the install leg so a missing install is not skipped
-- when online.
local function git_update(parsed, scope, cwd, agent_dir)
  local base = (scope == "project") and pi.path.join(cwd, ".pi") or agent_dir
  local target = pi.path.join(base, "git", parsed.host, parsed.path)
  local ok, exists = pcall(pi.fs.exists, target)
  if not (ok and exists) then return install_target(parsed, scope, cwd, agent_dir) end
  if parsed.ref then
    -- Reconcile a configured checkout ref; a leading `-` would be parsed as a
    -- git option and escape the managed path, so reject option-like refs.
    if parsed.ref:sub(1, 1) == "-" or parsed.ref == "" then
      error(("git checkout refused for option-like ref: %s"):format(parsed.ref), 0)
    end
    local fetch = pi.exec("git", { "fetch", "origin", parsed.ref }, { cwd = target })
    if fetch.code ~= 0 then
      error(("git fetch failed (code %s)"):format(fetch.code), 0)
    end
    local commit = (parsed.host == "github.com" or parsed.host == "gitlab.com")
      and "FETCH_HEAD" or "FETCH_HEAD"
    _ = commit
    local co = pi.exec("git", { "checkout", "FETCH_HEAD" }, { cwd = target })
    if co.code ~= 0 then
      error(("git checkout failed (code %s)"):format(co.code), 0)
    end
    return target
  end
  -- Default branch: fetch origin then hard-reset the checked-out branch head.
  local fetch = pi.exec("git", { "fetch", "origin" }, { cwd = target })
  if fetch.code ~= 0 then
    error(("git fetch failed (code %s)"):format(fetch.code), 0)
  end
  local head = pi.exec("git", { "rev-parse", "HEAD" }, { cwd = target })
  if head.code ~= 0 then
    error(("git rev-parse failed (code %s)"):format(head.code), 0)
  end
  local orig = head.stdout:gsub("%s+$", "")
  local upstream = pi.exec("git", { "rev-parse", "@{upstream}^{commit}" }, { cwd = target })
  if upstream.code ~= 0 then
    error(("git rev-parse @{upstream} failed (code %s)"):format(upstream.code), 0)
  end
  local target_commit = upstream.stdout:gsub("%s+$", "")
  if orig ~= target_commit then
    local reset = pi.exec("git", { "reset", "--hard", target_commit }, { cwd = target })
    if reset.code ~= 0 then
      error(("git reset failed (code %s)"):format(reset.code), 0)
    end
    local clean = pi.exec("git", { "clean", "-fdx" }, { cwd = target })
    if clean.code ~= 0 then
      error(("git clean failed (code %s)"):format(clean.code), 0)
    end
  end
  return target
end

-- install_target: shared clone/checkout used by both `install` and the git
-- update fresh-clone fallback (spec installGit / getGitInstallPath).
install_target = function(parsed, scope, cwd, agent_dir)
  local base = (scope == "project") and pi.path.join(cwd, ".pi") or agent_dir
  local target = pi.path.join(base, "git", parsed.host, parsed.path)
  local ok, exists = pcall(pi.fs.exists, target)
  if ok and exists then return target end
  pi.fs.mkdir(pi.path.dirname(target))
  local res = pi.exec("git", { "clone", parsed.repo, target }, { cwd = cwd })
  if res.code ~= 0 then
    error(("git clone failed (code %s)"):format(res.code), 0)
  end
  if parsed.ref then
    if parsed.ref:sub(1, 1) == "-" or parsed.ref == "" then
      error(("git checkout refused for option-like ref: %s"):format(parsed.ref), 0)
    end
    local co = pi.exec("git", { "checkout", parsed.ref }, { cwd = target })
    if co.code ~= 0 then
      error(("git checkout failed (code %s)"):format(co.code), 0)
    end
  end
  return target
end

-- updateConfiguredSources (spec core/package-manager.ts private resolver):
-- collect every configured source (user then project) that matches an optional
-- `source` identity and actually perform the npm/git update. In offline mode
-- (or when nothing is configured) this returns immediately with no network —
-- the deterministic offline-skip behavior. For an `extensions.updateSource`
-- request the caller already filtered to one identity; here we mirror the
-- CLI-path resolution where `update()` filters by identity and throws
-- buildNoMatchingPackageMessage when a requested source has no match.
local function collect_update_sources(cwd, agent_dir, identity)
  local out = {}
  local user_packages = CHANNELS.user.get()
  for _, entry in ipairs(user_packages) do
    local s = source_of(entry)
    if s and (not identity or package_identity(s, "user") == identity) then
      out[#out + 1] = { source = s, scope = "user" }
    end
  end
  local project_packages = read_project_packages(cwd)
  for _, entry in ipairs(project_packages) do
    local s = source_of(entry)
    if s and (not identity or package_identity(s, "project") == identity) then
      out[#out + 1] = { source = s, scope = "project" }
    end
  end
  return out
end

local function build_no_matching_package_message(source, configured)
  local list_parts = {}
  for _, s in ipairs(configured) do
    if type(s) == "string" then list_parts[#list_parts + 1] = s
    elseif s and s.source then list_parts[#list_parts + 1] = s.source end
  end
  return ("No matching package found for %s. Installed: %s"):format(
    source, table.concat(list_parts, ", ")
  )
end

-- update(source?, cwd, agent_dir): mirror spec update(). Returns `{ done,
-- updated }` where `updated` is a list of source strings actually refreshed.
-- Offline (or empty) sources short-circuit to `{ done = true }` with no exec,
-- which is the deterministic offline-skip for uninstalled packages.
local function update(source, cwd, agent_dir)
  cwd = cwd or pi.cwd()
  agent_dir = agent_dir or cwd
  local identity = source and package_identity(source, "user") or nil
  local matched = false
  local configured = {}
  for _, e in ipairs(CHANNELS.user.get()) do
    local s = source_of(e)
    if s then configured[#configured + 1] = s end
  end
  for _, e in ipairs(read_project_packages(cwd)) do
    local s = source_of(e)
    if s then configured[#configured + 1] = s end
  end
  if identity then
    for _, existing in ipairs(configured) do
      if package_identity(existing, "user") == identity then matched = true break end
    end
    if not matched then
      error(build_no_matching_package_message(source, configured), 0)
    end
  end
  local sources = collect_update_sources(cwd, agent_dir, identity)
  if is_offline_mode_enabled() or #sources == 0 then
    return { done = true, updated = {} }
  end
  local updated = {}
  for _, entry in ipairs(sources) do
    local parsed = parse_source(entry.source)
    if parsed.type == "npm" then
      if not parsed.pinned then
        npm_update(parsed, entry.scope, cwd, agent_dir)
        updated[#updated + 1] = { source = entry.source, scope = entry.scope }
      end
    elseif parsed.type == "git" then
      git_update(parsed, entry.scope, cwd, agent_dir)
      updated[#updated + 1] = { source = entry.source, scope = entry.scope }
    end
  end
  return { done = true, updated = updated }
end

-- self_update_plan: for a native pi-rs install there is no npm/Bun package to
-- reinstall, so the plan always stays "unavailable" — the self leg prints
-- Pi's cannot-self-update error (DESIGN platform boundary, ref
-- coding.platform-update); we do not fabricate a network release check.
local function self_update_plan(force)
  if force then return { packageName = "", shouldRun = true } end
  return { packageName = "", shouldRun = true }
end

local function print_self_update_unavailable(npm_command)
  local npm = table.concat(npm_command or {}, " ")
  local parts = { "error: pi cannot self-update this installation." }
  if npm ~= "" then
    parts[#parts + 1] = "This pi was installed as a native (compiled) build. To update, run:\n  " .. npm .. ""
  else
    parts[#parts + 1] = "This pi was installed as a native (compiled) build without a package reinstall command; update it manually."
  end
  return table.concat(parts, "\n") .. "\n"
end

pi.module.define({
  name = "pi.packages",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      parse_source = parse_source,
      parse_npm_spec = parse_npm_spec,
      is_local_path = is_local_path,
      package_identity = package_identity,
      add_source_to_settings = add_source_to_settings,
      remove_source_from_settings = remove_source_from_settings,
      list_configured_packages = list_configured_packages,
      get_installed_path = get_installed_path,
      install = install,
      install_and_persist = install_and_persist,
      remove_and_persist = remove_and_persist,
      read_project_packages = read_project_packages,
      update = update,
      collect_update_sources = collect_update_sources,
      is_offline_mode_enabled = is_offline_mode_enabled,
      npm_update = npm_update,
      git_update = git_update,
      self_update_plan = self_update_plan,
      print_self_update_unavailable = print_self_update_unavailable,
    }
  end,
})

-- ---------------------------------------------------------------------------
-- package-manager-cli.ts handlePackageCommand — the CLI *execution* legs
-- (install/remove/list/update after the parse/help/early-error prefix).
-- Rust parses the raw argv (`cli/packages.rs`) and, when a command would
-- proceed to execution, dispatches the parsed options here so the package
-- lifecycle runs through the same public `pi.packages` mechanism embedded
-- builtins and file-backed packages share. Output is collected and returned
-- to Rust to mirror Pi's console.log/console.error ordering byte-for-byte;
-- npm/git install/update route through the public `pi.exec`/`pi.fs` and are
-- network-modulated (offline-skip when PI_OFFLINE).
pi.register_role({
  id = "coding-agent-package-cli",
  role = "pkg-exec",
  active = true,
  priority = 0,
  description = "Run the packages CLI execution legs through pi.packages",
  handler = function(args)
    local request = pi.json.decode(args)
    local out, err = {}, {}
    local function write_out(s) out[#out + 1] = s end
    local function write_err(s) err[#err + 1] = s end

    local command = request.command
    local source = request.source
    local local_scope = request.local_scope == true
    local options = {}
    if local_scope then options["local"] = true end
    if request.agentDir then options.agentDir = request.agentDir end
    local cwd = request.cwd or pi.cwd()
    local agent_dir = request.agentDir

    -- Project-trust gate (spec package-manager-cli.ts): install/remove that
    -- write project config require the project to be trusted.
    local writes_project = local_scope
      and (command == "install" or command == "remove")
    if writes_project and not pi.settings.is_project_trusted() then
      write_err("Project is not trusted. Use --approve to modify local package config.\n")
      return { exitCode = 1, stdout = table.concat(out), stderr = table.concat(err) }
    end

    local ok, result = pcall(function()
      local pm = pi.module.require("pi.packages", "1")
      if command == "list" then
        local configured = pm.list_configured_packages(cwd, agent_dir)
        local user_pkgs, project_pkgs = {}, {}
        for _, pkg in ipairs(configured) do
          if pkg.scope == "user" then user_pkgs[#user_pkgs + 1] = pkg
          else project_pkgs[#project_pkgs + 1] = pkg end
        end
        if #configured == 0 then
          write_out("No packages installed.\n")
          return { done = true }
        end
        local function format_pkg(pkg)
          local display = pkg.filtered and (pkg.source .. " (filtered)") or pkg.source
          write_out("  " .. display .. "\n")
          if pkg.installedPath then
            write_out("    " .. pkg.installedPath .. "\n")
          end
        end
        if #user_pkgs > 0 then
          write_out("User packages:\n")
          for _, pkg in ipairs(user_pkgs) do format_pkg(pkg) end
        end
        if #project_pkgs > 0 then
          if #user_pkgs > 0 then write_out("\n") end
          write_out("Project packages:\n")
          for _, pkg in ipairs(project_pkgs) do format_pkg(pkg) end
        end
        return { done = true }
      end
      if command == "install" then
        pm.install_and_persist(source, options)
        write_out("Installed " .. source .. "\n")
        return { done = true }
      end
      if command == "remove" then
        local removed = pm.remove_and_persist(source, options)
        if not removed then
          write_err("No matching package found for " .. source .. "\n")
          return { exit = 1 }
        end
        write_out("Removed " .. source .. "\n")
        return { done = true }
      end
      if command == "update" then
        -- Spec handlePackageCommand `update` case: run the extensions leg when
        -- the target includes extensions, then the self leg when it includes
        -- self. The offline extension leg short-circuits with no network
        -- (updateConfiguredSources early-return); a requested source with no
        -- match throws buildNoMatchingPackageMessage.
        local target = request.update_target or "all"
        local includes_ext = target == "all" or target == "extensions"
        local includes_self = target == "all" or target == "self"
        local force = request.force == true
        if includes_ext then
          -- update(source?) throws when a requested source has no configured
          -- match; offline it still throws on a missing match, matching Pi's
          -- update() ordering (identity check before updateConfiguredSources).
          local upd = pm.update(source, cwd, agent_dir)
          if source then
            write_out("Updated " .. source .. "\n")
          else
            write_out("Updated packages\n")
          end
          _ = upd
        end
        if includes_self then
          local plan = pm.self_update_plan(force)
          if plan.shouldRun then
            -- Native pi-rs cannot self-update (DESIGN platform boundary):
            -- print Pi's cannot-self-update error and exit 1, exactly the
            -- printSelfUpdateUnavailable path a non-npm install reaches.
            local ok_cmd, cfg = pcall(pi.settings.npm_command)
            local npm_cmd = (ok_cmd and type(cfg) == "table") and cfg or nil
            write_err(pm.print_self_update_unavailable(npm_cmd))
            return { exit = 1 }
          end
        end
        return { done = true }
      end
      return { out_of_scope = true }
    end)

    if not ok then
      write_err("Error: " .. tostring(result) .. "\n")
      return { exitCode = 1, stdout = table.concat(out), stderr = table.concat(err) }
    end
    local exit = result and result.exit or 0
    return { exitCode = exit, stdout = table.concat(out), stderr = table.concat(err) }
  end,
})

-- ---------------------------------------------------------------------------
-- package-manager-cli.ts handleConfigCommand — the `pi config` TUI command.
--
-- Rust dispatches `pi config` to this `config-exec` role after the project-trust
-- / settings preamble. The full interactive resource/config selector is a TUI
-- component owned by the interactive frontend; this role mirrors the
-- deterministic preamble (project trust, settings errors, resource resolution)
-- and, for the headless CLI path, emits the resolved resource paths so the
-- command wiring is observable and file-backed. The selector UI itself lives in
-- the interactive pack (settings selector); here we cover the command plumbing
-- and the resolved-resource outcome.
pi.register_role({
  id = "coding-agent-config-cli",
  role = "config-exec",
  active = true,
  priority = 0,
  description = "Run the pi config command preamble + resource resolution",
  handler = function(args)
    local request = pi.json.decode(args)
    local out, err = {}, {}
    local function write_out(s) out[#out + 1] = s end
    local function write_err(s) err[#err + 1] = s end
    local cwd = request.cwd or pi.cwd()
    local agent_dir = request.agentDir

    -- Project-trust preamble (spec createCommandSettingsManager): report
    -- non-fatal trust/settings warnings; the interactive selector then runs.
    local trusted = pi.settings.is_project_trusted()
    if request.projectTrustOverride ~= nil
      and request.projectTrustOverride == false and not trusted
    then
      write_err("Project is not trusted. Use --approve to modify local package config.\n")
    end

    write_out("Configured resources for " .. cwd .. "\n")

    -- Packages come from the shared pi.packages module (the config command's
    -- resolvedPaths list configured package sources; spec packageManager.
    -- resolve / listConfiguredPackages).
    local ok_pm, pm = pcall(function()
      return pi.module.require("pi.packages", "1")
    end)
    if ok_pm and pm and pm.list_configured_packages then
      local configured = pm.list_configured_packages(cwd, agent_dir)
      if #configured > 0 then
        write_out("PACKAGES:\n")
        for _, entry in ipairs(configured) do
          write_out("  " .. tostring(entry.source) .. "\n")
        end
      end
    end

    -- Resolve configured skills/prompts/themes through the shared pi.resources
    -- module so the config command reflects the same precedence/attribution
    -- engine as the interactive frontend's /reload.
    local ok_res, resources = pcall(function()
      return pi.module.require("pi.resources", "1")
    end)
    if ok_res and resources and resources.resolve then
      local resolved = resources.resolve({
        cwd = cwd, agentDir = agent_dir, home = request.home,
        projectTrusted = nil,
      })
      for _, kind in ipairs({ "extensions", "skills", "prompts", "themes" }) do
        local entries = resolved[kind] or {}
        if type(entries) == "table" and next(entries) then
          write_out(string.upper(kind) .. ":\n")
          for _, entry in ipairs(entries) do
            if type(entry) == "table" then
              write_out("  " .. tostring(entry.source or entry.path or "") .. "\n")
            else
              write_out("  " .. tostring(entry) .. "\n")
            end
          end
        end
      end
    end
    return { exitCode = 0, stdout = table.concat(out), stderr = table.concat(err) }
  end,
})
end