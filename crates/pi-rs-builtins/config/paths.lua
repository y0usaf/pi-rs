-- Resource-path policy: XDG-first canonical locations with a read-only
-- `~/.pi/agent` fallback resolved per resource.
--
-- Every directory name, precedence rule, and fallback below is Lua. The host
-- contributes only an immutable environment snapshot, pure path arithmetic,
-- and bounded filesystem metadata. Rust never names `config.lua`,
-- `settings.json`, or `~/.pi/agent`; the launcher's own startup report is a
-- separate provenance report for the launcher, not an input here.
--
-- The rules, in order:
--
-- 1. An explicit, absolute `XDG_*_HOME` wins for its class; an empty value
--    has the XDG-defined meaning of "unset" and a relative one is refused
--    with a diagnostic instead of being silently accepted.
-- 2. Otherwise the documented `$HOME` default is used.
-- 3. Without a usable root the class is `unavailable` — never the working
--    directory.
-- 4. A resource is read from its canonical XDG entry when that entry exists,
--    and only then from its legacy counterpart. A present-but-broken
--    canonical entry never falls through.
-- 5. Every destination is canonical. Nothing here writes, and no caller is
--    handed a legacy destination.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.config.paths",
  version = "1",
  factory = function()
    local effects = pi.effects.v1
    local path, fs = effects.path, effects.fs

    -- Class roots: environment variable, `$HOME`-relative default.
    local CLASSES = {
      { class = "config", variable = "XDG_CONFIG_HOME", default = { ".config" } },
      { class = "data", variable = "XDG_DATA_HOME", default = { ".local", "share" } },
      { class = "state", variable = "XDG_STATE_HOME", default = { ".local", "state" } },
      { class = "cache", variable = "XDG_CACHE_HOME", default = { ".cache" } },
    }

    -- Product resources. `legacy` is nil for a resource with no inherited
    -- counterpart: trust decisions are a pi-rs concept, so they have none.
    local RESOURCES = {
      {
        resource = "config",
        class = "config",
        entry = { "config.lua" },
        legacy = { "settings.json" },
      },
      { resource = "packages", class = "data", entry = { "packages" }, legacy = { "packages" } },
      { resource = "sessions", class = "state", entry = { "sessions" }, legacy = { "sessions" } },
      {
        resource = "credentials",
        class = "state",
        entry = { "credentials.json" },
        legacy = { "auth.json" },
      },
      { resource = "cache", class = "cache", entry = {}, legacy = { "cache" } },
      { resource = "trust", class = "state", entry = { "trust" }, legacy = nil },
    }

    local PROJECT_DIRECTORY = ".pi"
    local PROJECT_FILE = "config.lua"

    local function order()
      local names = {}
      for index, entry in ipairs(RESOURCES) do
        names[index] = entry.resource
      end
      return names
    end

    local function join(base, components)
      local result = base
      for _, component in ipairs(components) do
        result = path.join(result, component)
      end
      return result
    end

    local VARIABLES = {
      "HOME",
      "XDG_CONFIG_HOME",
      "XDG_DATA_HOME",
      "XDG_STATE_HOME",
      "XDG_CACHE_HOME",
    }

    local function read_environment(source)
      local values = {}
      for _, name in ipairs(VARIABLES) do
        if type(source) == "table" then
          local value = source[name]
          if type(value) == "string" then
            values[name] = value
          end
        else
          values[name] = effects.env.get(name)
        end
      end
      return values
    end

    --- Roots for every storage class, plus the read-only legacy root.
    local function roots(environment)
      local values = read_environment(environment)
      local diagnostics = {}
      local home = values.HOME
      if type(home) ~= "string" or home == "" then
        home = nil
        diagnostics[#diagnostics + 1] = "HOME is unset; only explicit absolute XDG roots are usable"
      elseif not path.is_absolute(home) then
        diagnostics[#diagnostics + 1] = "HOME '" .. home .. "' is not absolute and was ignored"
        home = nil
      end

      local resolved = {}
      for _, entry in ipairs(CLASSES) do
        local value = values[entry.variable]
        if type(value) == "string" and value ~= "" then
          if path.is_absolute(value) then
            resolved[entry.class] = path.join(value, "pi")
          else
            diagnostics[#diagnostics + 1] =
              entry.variable .. " '" .. value .. "' is not absolute and was ignored"
          end
        end
        if resolved[entry.class] == nil and home ~= nil then
          resolved[entry.class] = path.join(join(home, entry.default), "pi")
        end
      end
      resolved.legacy = home and path.join(home, ".pi", "agent") or nil
      return resolved, diagnostics, home
    end

    local function exists(candidate)
      local ok, present = pcall(fs.exists, candidate)
      return ok and present == true
    end

    --- Resolve one resource: canonical first, legacy only when the canonical
    --- entry is absent. `destination` is always canonical.
    local function resolve_resource(entry, roots_by_class)
      local base = roots_by_class[entry.class]
      local legacy_root = roots_by_class.legacy
      local row = {
        resource = entry.resource,
        class = entry.class,
        canonical = base and join(base, entry.entry) or nil,
        legacy = (entry.legacy and legacy_root) and join(legacy_root, entry.legacy) or nil,
        destination = base and join(base, entry.entry) or nil,
        source = "absent",
        selected = nil,
      }
      if row.canonical == nil then
        row.source = "unavailable"
        row.diagnostic = "no usable " .. entry.class .. " root"
        return row
      end
      if exists(row.canonical) then
        row.source = "canonical"
        row.selected = row.canonical
      elseif row.legacy ~= nil and exists(row.legacy) then
        row.source = "legacy"
        row.selected = row.legacy
      end
      return row
    end

    --- Complete resource matrix for one environment.
    ---
    --- `options.environment` supplies explicit values instead of the host
    --- snapshot; `options.project_root` names the directory whose
    --- `.pi/config.lua` is the project layer.
    local function resolve(options)
      options = options or {}
      local roots_by_class, diagnostics, home = roots(options.environment)
      local resources = {}
      local rows = {}
      for _, entry in ipairs(RESOURCES) do
        local row = resolve_resource(entry, roots_by_class)
        resources[entry.resource] = row
        rows[#rows + 1] = row
      end
      local project = nil
      local project_root = options.project_root
      if type(project_root) == "string" and project_root ~= "" then
        project = {
          root = project_root,
          file = path.join(project_root, PROJECT_DIRECTORY, PROJECT_FILE),
        }
        project.present = exists(project.file)
      end
      return {
        home = home,
        roots = roots_by_class,
        resources = resources,
        rows = rows,
        order = order(),
        project = project,
        diagnostics = diagnostics,
      }
    end

    return {
      resolve = resolve,
      roots = roots,
      order = order,
      project_directory = PROJECT_DIRECTORY,
      project_file = PROJECT_FILE,
    }
  end,
})
