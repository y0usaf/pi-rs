-- Applying the `tools` section: workspace root, suppression, per-tool settings.
--
-- The shipped tool suite (`pi.tools.suite@1`) declares its tools into the
-- shipped agent's registry (`pi.agent.tools@1`) when its package loads, and the
-- distribution re-declares them from `defaults/init.lua`
-- (`pi.builtins.defaults.tool-root`, order `-99`) once the launcher context
-- first names a root. A configuration is a *higher* layer than a distribution
-- default, so it applies after that stage rather than before it: `init.lua`
-- registers `pi.builtins.config.tools` at order `-50`, and the last stage to
-- run owns the registry. That is ordering, not privilege — a package that wants
-- the final word registers a later stage the same way.
--
-- Two halves, the same split the rest of the package uses:
--
-- | Half | When | Effect |
-- |---|---|---|
-- | `plan(settings)` | composition, before publication | validates; a bad name rolls the whole reload back |
-- | `reconcile(policy, root, revision)` | the `-50` middleware stage | re-declares the suite |
--
-- Re-declaring costs one unregister plus one declare, so it happens only when
-- the published revision or the launcher root changes — the two events that
-- make the live declaration stale. A dispatch that changes neither does
-- nothing at all.

local pi = ...
local kernel = pi.kernel.v1
local module = kernel.module
local path = pi.effects.v1.path

local schema = module.require("pi.config.schema", "1")

local SUITE_NAME, SUITE_VERSION = "pi.tools.suite", "1"
local REGISTRY_NAME, REGISTRY_VERSION = "pi.agent.tools", "1"

--- What was last handed to the suite.
---
--- It lives at file scope, outside the factory, for the same reason
--- `pi.config.apply@1` stages its plan there: disposing *any* package clears
--- every module's cached value, so the next `require` re-runs the factory and
--- returns a different table. A factory local would forget what is live
--- whenever an unrelated package was disposed, and the next dispatch would
--- re-declare the suite for no reason.
local applied = nil

module.define({
  name = "pi.config.tools",
  version = "1",
  factory = function()
    local function first_line(value)
      local text = tostring(value)
      return (string.match(text, "^[^\n]*")) or text
    end

    --- The suite and the registry it declares into, or `nil` plus the reason.
    --- A distribution without the shipped tool package is not an error by
    --- itself; configuring `tools` in one is.
    local function resolve()
      local found, suite = pcall(module.require, SUITE_NAME, SUITE_VERSION)
      if not found or type(suite) ~= "table" then
        return nil, nil, "no tool suite (" .. SUITE_NAME .. "@" .. SUITE_VERSION .. ") is loaded"
      end
      local declared, registry = pcall(module.require, REGISTRY_NAME, REGISTRY_VERSION)
      if not declared or type(registry) ~= "table" then
        return nil,
          nil,
          "no tool registry (" .. REGISTRY_NAME .. "@" .. REGISTRY_VERSION .. ") is loaded"
      end
      return suite, registry
    end

    local function section_of(settings)
      local section = (settings or {}).tools
      if type(section) ~= "table" then
        return {}
      end
      return section
    end

    --- Does this section ask for anything? An empty section leaves the
    --- distribution's own tool policy untouched instead of restating it.
    local function configured(section)
      if type(section.root) == "string" then
        return true
      end
      if #(section.suppress or {}) > 0 then
        return true
      end
      return next(section.settings or {}) ~= nil
    end

    --- Validate the section against the live suite. Pure apart from resolving
    --- the suite module, which is idempotent. Returns `policy, errors`;
    --- `policy` is `nil` when nothing is configured.
    local function plan(settings)
      local section = section_of(settings)
      local errors = {}
      if not configured(section) then
        return nil, errors
      end

      if section.root ~= nil and not path.is_absolute(section.root) then
        errors[#errors + 1] = "tools.root: must be an absolute path, got '" .. section.root .. "'"
      end

      local suite, _, reason = resolve()
      if suite == nil then
        errors[#errors + 1] = "tools: " .. reason
        return nil, errors
      end

      local known = {}
      for _, name in ipairs(suite.names()) do
        known[name] = true
      end

      local suppress = {}
      for index, name in ipairs(section.suppress or {}) do
        if not known[name] then
          errors[#errors + 1] = "tools.suppress[" .. index .. "]: no tool named '" .. name .. "'"
        else
          suppress[name] = true
        end
      end

      local per_tool = {}
      for _, name in ipairs(schema.sorted_keys(section.settings or {})) do
        local prefix = "tools.settings." .. name
        if not known[name] then
          errors[#errors + 1] = prefix .. ": no tool named '" .. name .. "'"
        elseif suppress[name] then
          -- Settings for a tool this configuration also removes are the
          -- classic silent no-op; say so instead of applying nothing.
          errors[#errors + 1] = prefix .. ": '" .. name .. "' is suppressed by tools.suppress"
        else
          local values = {}
          for _, key in ipairs(schema.sorted_keys(section.settings[name])) do
            if key == "name" then
              -- The suite removes a tool by its default name, so a rename
              -- would leak the old declaration on the next reload.
              errors[#errors + 1] = prefix .. ".name: a tool cannot be renamed by configuration"
            else
              values[key] = section.settings[name][key]
            end
          end
          per_tool[name] = values
        end
      end

      if #errors > 0 then
        return nil, errors
      end
      return { root = section.root, suppress = suppress, settings = per_tool }, errors
    end

    --- Re-declare the whole suite from one policy. `policy` may be `nil`,
    --- which restores exactly what the distribution declares: every tool, with
    --- the launcher root.
    local function declare_suite(policy, root)
      local suite, registry, reason = resolve()
      if suite == nil then
        return false, reason
      end

      local suppress = (policy or {}).suppress or {}
      local configured_settings = (policy or {}).settings or {}
      local options = { suppress = {}, tools = {} }
      local shared = (type(root) == "string" and #root > 0) and root or nil

      for _, name in ipairs(suite.names()) do
        if suppress[name] then
          options.suppress[name] = true
        else
          -- `suite.declare` uses `options.tools[name]` *instead of*
          -- `options.shared`, so the workspace root is merged in here; a
          -- per-tool `root` deliberately wins over it.
          local values = {}
          if shared ~= nil then
            values.root = shared
          end
          for key, value in pairs(configured_settings[name] or {}) do
            values[key] = value
          end
          if next(values) ~= nil then
            options.tools[name] = values
          end
        end
      end

      suite.unregister(registry)
      local ok, failure = pcall(suite.declare, registry, options)
      if not ok then
        -- A tool refused its settings after the others were already removed.
        -- Leave the distribution's own declaration behind rather than a
        -- half-applied suite.
        pcall(suite.unregister, registry)
        pcall(suite.declare, registry, shared and { shared = { root = shared } } or nil)
        applied = nil
        return false, first_line(failure)
      end
      return true
    end

    --- Bring the live tool declaration in line with the published policy.
    --- Called from the `-50` application stage on every dispatch, so it is a
    --- comparison first and an action second.
    local function reconcile(policy, context_root, revision)
      if policy == nil then
        if applied == nil then
          return true
        end
        -- The section was removed: hand the tools back to the distribution.
        local ok, message = declare_suite(nil, context_root)
        if ok then
          applied = nil
        end
        return ok, message
      end

      local root = policy.root or context_root
      if
        applied ~= nil
        and applied.revision == revision
        and applied.root == root
        and applied.context_root == context_root
      then
        return true
      end

      local ok, message = declare_suite(policy, root)
      if ok then
        applied = { revision = revision, root = root, context_root = context_root }
      end
      return ok, message
    end

    --- The configuration's own account of the live tool declaration: the root
    --- the suite runs with, the suppressed names, the per-tool settings, and
    --- the revision that produced them. `nil` while the distribution's own
    --- policy is in force.
    local function report(policy)
      if applied == nil then
        return nil
      end
      local names = {}
      for _, name in ipairs(schema.sorted_keys((policy or {}).suppress or {})) do
        names[#names + 1] = name
      end
      return {
        root = applied.root,
        revision = applied.revision,
        suppress = names,
        settings = schema.copy((policy or {}).settings or {}),
      }
    end

    return {
      plan = plan,
      reconcile = reconcile,
      report = report,
    }
  end,
})
