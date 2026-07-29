-- Applying configuration: validated settings become declarations.
--
-- Validating a section proves the file is well formed; it does not make the
-- product behave differently. This module is the second half — the step that
-- turns published settings into something the rest of the distribution can
-- read without knowing that a configuration package exists at all.
--
-- Two mechanisms carry everything, and neither is new:
--
-- | Section | Applied through |
-- |---|---|
-- | `modules` | `pi.kernel.v1.module.require(name, version)` |
-- | `theme`, `keymaps`, `providers` | `pi.kernel.v1.declare(kind, definition)` |
--
-- **Why a plan.** `plan()` is pure: it reads settings and returns declaration
-- rows plus errors, touching nothing. It runs during composition, so a theme
-- of the wrong shape or a provider naming a model the catalog does not carry
-- fails the reload and rolls back with every other failure, instead of half
-- applying after the settings were already published.
--
-- **Why a separate declaration package.** `pi.kernel.v1.declare` refuses a
-- second declaration of the same `kind`/`id`, and a declaration lives exactly
-- as long as the package scope that made it. The configuration package's own
-- scope outlives every reload, so it can never re-declare its own theme.
-- Instead the staged plan is replayed by a tiny package (`SOURCE` below) that
-- the configuration package loads and disposes like any other: disposing it
-- retracts its declarations, so the next revision may declare the same ids.
-- The replayed source is a two-line constant — configuration data crosses
-- through this module, never through generated code.
--
-- Declaration ids live in a `pi.config.` namespace so a configured provider
-- never silently collides with one a package declared, and so a consumer can
-- tell that a declaration came from configuration. Every row carries the
-- `layer` and `origin` file that produced it, the same provenance the
-- effective settings already expose.

local pi = ...
local kernel = pi.kernel.v1
local module = kernel.module
local models = pi.models.v1

local schema = module.require("pi.config.schema", "1")

--- Name of the package that replays a staged plan.
local PACKAGE_NAME = "pi.config.declarations"

--- Its complete source. It carries no configuration data: the plan is staged
--- on this module and read back by identity.
local SOURCE = 'local pi = ...\n'
  .. 'pi.kernel.v1.module.require("pi.config.apply", "1").commit()\n'

--- The plan waiting to be replayed.
---
--- It lives at file scope, outside the factory, on purpose: disposing *any*
--- package clears every module's cached value, so the next `require` re-runs
--- the factory and hands back a different table. Staging into a factory local
--- would therefore stage onto the instance the disposal just retired, and the
--- declaration package loaded a moment later would find nothing to declare.
--- The chunk runs once per package load, so this local is the one place both
--- instances agree on.
local staged = nil

module.define({
  name = "pi.config.apply",
  version = "1",
  factory = function()
    -- Host errors carry a traceback; a configuration diagnostic wants the
    -- sentence.
    local function first_line(value)
      local text = tostring(value)
      return (string.match(text, "^[^\n]*")) or text
    end

    -- The layer and file behind a section. Provenance is recorded per leaf,
    -- so a section's origin is the origin of its first leaf in sorted order;
    -- a section written by exactly one layer therefore reports that layer,
    -- and a merged one reports the layer of its lowest-sorting key.
    local function origin_under(provenance, prefix)
      local best = nil
      for path, origin in pairs(provenance or {}) do
        if path == prefix or string.sub(path, 1, #prefix + 1) == prefix .. "." then
          if best == nil or path < best.path then
            best = { path = path, layer = origin.layer, origin = origin.source }
          end
        end
      end
      if best == nil then
        return nil
      end
      return { layer = best.layer, origin = best.origin }
    end

    local function stamp(definition, provenance, prefix)
      local origin = origin_under(provenance, prefix)
      if origin ~= nil then
        definition.layer = origin.layer
        definition.origin = origin.origin
      end
      return definition
    end

    -- One provider row becomes one declaration carrying validated model rows.
    -- A configured model starts from its reviewed catalog row and takes the
    -- section's endpoint overrides, so nothing here invents a cost, a context
    -- window, or a token budget. A model the catalog does not carry is an
    -- error naming its dotted path rather than a row assembled from guesses.
    local function provider_models(name, row, errors)
      local declared = {}
      for index, id in ipairs(row.models or {}) do
        local path = "providers." .. name .. ".models[" .. index .. "]"
        local found_ok, found = pcall(models.find, name, id)
        if not found_ok then
          errors[#errors + 1] = path .. ": " .. first_line(found)
        elseif type(found) ~= "table" then
          errors[#errors + 1] = path
            .. ": no reviewed catalog row for "
            .. name
            .. "/"
            .. id
        else
          local candidate = schema.copy(found)
          if type(row.api) == "string" then
            candidate.api = row.api
          end
          if type(row.base_url) == "string" then
            candidate.baseUrl = row.base_url
          end
          local valid, validated = pcall(models.validate, candidate)
          if not valid then
            errors[#errors + 1] = path .. ": " .. first_line(validated)
          else
            declared[#declared + 1] = validated
          end
        end
      end
      return declared
    end

    --- Translate published settings into declaration rows. Pure: it reads,
    --- validates, and returns; it declares nothing and stores nothing.
    --- Returns `rows, errors`.
    local function plan(settings, provenance)
      settings = settings or {}
      local rows, errors = {}, {}

      if type(settings.theme) == "string" then
        rows[#rows + 1] = {
          kind = "theme",
          definition = stamp({
            id = "pi.config.theme",
            name = settings.theme,
            order = 0,
          }, provenance, "theme"),
        }
      end

      -- Sorted so the declaration order of a configuration is a property of
      -- its content, not of Lua's table iteration order.
      local keymaps = settings.keymaps or {}
      for index, binding in ipairs(schema.sorted_keys(keymaps)) do
        rows[#rows + 1] = {
          kind = "keymap",
          definition = stamp({
            id = "pi.config.keymap:" .. binding,
            binding = binding,
            action = keymaps[binding],
            order = index,
          }, provenance, "keymaps." .. binding),
        }
      end

      local providers = settings.providers or {}
      for index, name in ipairs(schema.sorted_keys(providers)) do
        local row = providers[name] or {}
        rows[#rows + 1] = {
          kind = "provider",
          definition = stamp({
            id = "pi.config.provider:" .. name,
            provider = name,
            api = row.api,
            base_url = row.base_url,
            models = provider_models(name, row, errors),
            order = index,
          }, provenance, "providers." .. name),
        }
      end

      return rows, errors
    end

    --- Resolve every module identity the configuration names. An identity
    --- that does not resolve is a configuration error, so a file pinning a
    --- version its packages do not provide fails the reload instead of
    --- surfacing as a missing dependency later. Returns `resolved, errors`.
    local function pin(settings)
      local resolved, errors = {}, {}
      for index, entry in ipairs((settings or {}).modules or {}) do
        local ok, value = pcall(module.require, entry.name, entry.version)
        if not ok then
          errors[#errors + 1] = "modules[" .. index .. "]: " .. first_line(value)
        else
          resolved[#resolved + 1] = entry.name .. "@" .. entry.version
        end
      end
      return resolved, errors
    end

    --- Stage a plan for the declaration package to replay. `nil` clears it.
    local function stage(rows)
      staged = rows
    end

    --- Replay the staged plan. Called only from the declaration package, so
    --- the declarations belong to that package's scope and are retracted when
    --- the configuration package disposes it.
    local function commit()
      local rows = staged or {}
      for _, row in ipairs(rows) do
        kernel.declare(row.kind, row.definition)
      end
      return #rows
    end

    return {
      package_name = PACKAGE_NAME,
      source = SOURCE,
      plan = plan,
      pin = pin,
      stage = stage,
      commit = commit,
    }
  end,
})
