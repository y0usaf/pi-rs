-- Applying the `roots` section: which registered root each kind resolves to.
--
-- These are the *replaceable roots* — `application`, `agent`, `frontend` — not
-- the storage roots of `pi.config.paths@1`, which are directories. A
-- distribution registers one root per kind; so may a package a configuration
-- loads. Without a selection the host resolves a kind by the highest priority
-- among the active registrations and fails on a tie, so replacing a root meant
-- outbidding it. `roots.<kind> = "<id>"` says it directly instead.
--
-- Two halves, the same split `pi.config.tools@1` uses:
--
-- | Half | When | Effect |
-- |---|---|---|
-- | `plan(settings)` | composition, after the configuration's own packages load | validates against the live registry; an unknown id rolls the whole reload back |
-- | `reconcile(policy, revision)` | the `-49` middleware stage | calls `pi.roots.v1.select` |
--
-- Validation happens *after* `packages` are loaded, not while the settings are
-- being merged: the usual reason to name a root is that a package the same
-- configuration loads registers it, and a root registered one step later would
-- otherwise fail its own selection.
--
-- The honest limit, checked in `crates/pi-rs-builtins/tests/config_package.rs`:
-- the host resolves a root *before* the event middleware of that dispatch
-- runs, so a selection applied during dispatch N governs dispatch N+1 of that
-- kind. Selecting the `frontend` or `agent` root from the application's own
-- stage is therefore immediate in practice — those dispatches come later —
-- while replacing the `application` root itself takes effect on the next
-- application dispatch.
--
-- A selection is owned by this package's source and scope: a second package
-- selecting the same kind is a deterministic conflict, and disposing the
-- configuration hands the kind back to priority resolution.

local pi = ...
local kernel = pi.kernel.v1
local module = kernel.module
local roots = pi.roots.v1

local schema = module.require("pi.config.schema", "1")

--- Root kinds a configuration may select, in the order they are reported.
--- Exactly the kinds `pi.roots.v1` registers, so a selection that validates
--- can always be applied.
local KINDS = { "application", "agent", "frontend" }

--- `session` is a schema field so that configuring it is answered precisely
--- rather than as an unknown key: the shipped session package integrates as
--- two middleware stages and owns no root, so there is nothing to select. It
--- is replaced or removed the way it is shipped — through the package index,
--- or by naming a replacement in `packages`.
local SESSION_ADVICE = "roots.session: the session package integrates as middleware stages and registers no root; "
  .. "replace it through the package index or `packages`, not `roots.session`"

--- What was last selected, per kind. File scope for the same reason
--- `pi.config.tools@1` keeps `applied` there: disposing any package clears
--- every module's cached value, so a factory local would forget the live
--- selection whenever an unrelated package was disposed.
local applied = nil

module.define({
  name = "pi.config.roots",
  version = "1",
  factory = function()
    local function first_line(value)
      local text = tostring(value)
      return (string.match(text, "^[^\n]*")) or text
    end

    local function section_of(settings)
      local section = (settings or {}).roots
      if type(section) ~= "table" then
        return {}
      end
      return section
    end

    --- Every active registration, indexed `kind -> id -> true`, plus the ids
    --- per kind in list order for the diagnostics.
    local function registry()
      local index, names = {}, {}
      for _, kind in ipairs(KINDS) do
        index[kind], names[kind] = {}, {}
      end
      local ok, rows = pcall(roots.list)
      if not ok or type(rows) ~= "table" then
        return index, names
      end
      for _, row in ipairs(rows) do
        if row.active and index[row.kind] ~= nil then
          index[row.kind][row.id] = true
          names[row.kind][#names[row.kind] + 1] = row.id
        end
      end
      return index, names
    end

    --- Validate the section against the live root registry. Returns
    --- `policy, errors`; `policy` is `nil` when nothing is configured.
    local function plan(settings)
      local section = section_of(settings)
      local errors = {}
      local wanted = {}
      for _, kind in ipairs(KINDS) do
        if type(section[kind]) == "string" then
          wanted[kind] = section[kind]
        end
      end
      if type(section.session) == "string" then
        errors[#errors + 1] = SESSION_ADVICE
      end
      if next(wanted) == nil then
        return nil, errors
      end

      local index, names = registry()
      for _, kind in ipairs(KINDS) do
        local id = wanted[kind]
        if id ~= nil and not index[kind][id] then
          local available = #names[kind] > 0 and table.concat(names[kind], ", ") or "none"
          errors[#errors + 1] = "roots."
            .. kind
            .. ": no active "
            .. kind
            .. " root named '"
            .. id
            .. "' (registered: "
            .. available
            .. ")"
        end
      end

      if #errors > 0 then
        return nil, errors
      end
      return wanted, errors
    end

    --- Hand one kind to `pi.roots.v1.select`, or clear it with `nil`.
    local function apply(kind, id)
      local ok, failure = pcall(roots.select, kind, id)
      if ok then
        return true
      end
      return false, first_line(failure)
    end

    --- Bring the live selection in line with the published policy. Called
    --- from the `-49` application stage on every dispatch, so it is a
    --- comparison first and an action second.
    ---
    --- A kind that leaves the section is *cleared*, not frozen: removing
    --- `roots.frontend` hands the frontend back to priority resolution the
    --- same way removing `tools.suppress` hands the tools back to the
    --- distribution.
    local function reconcile(policy, revision)
      if applied ~= nil and applied.revision == revision then
        return true
      end
      local wanted = policy or {}
      local previous = (applied or {}).kinds or {}
      local landed, failure = {}, nil
      for _, kind in ipairs(KINDS) do
        if wanted[kind] == previous[kind] then
          landed[kind] = previous[kind]
        else
          local ok, message = apply(kind, wanted[kind])
          if ok then
            -- Record what is live, not what was asked for: a refused
            -- selection must not be reported as applied.
            landed[kind] = wanted[kind]
          else
            landed[kind] = previous[kind]
            failure = failure or ("roots." .. kind .. ": " .. tostring(message))
          end
        end
      end
      if failure ~= nil then
        -- Leave the revision unset so the next dispatch retries the refused
        -- kind instead of comparing it away.
        applied = { revision = nil, kinds = landed }
        return false, failure
      end
      applied = { revision = revision, kinds = landed }
      return true
    end

    --- The configuration's own account of the live selection: the id per kind
    --- and the revision behind it. `nil` while every kind is resolved by
    --- priority, which is also the zero-configuration answer.
    local function report()
      if applied == nil or next(applied.kinds) == nil then
        return nil
      end
      return {
        revision = applied.revision,
        selected = schema.copy(applied.kinds),
      }
    end

    return {
      kinds = KINDS,
      plan = plan,
      reconcile = reconcile,
      report = report,
    }
  end,
})
