-- Configuration package: layered discovery, atomic publication, inspection.
--
-- Layers, lowest precedence first:
--
-- | Layer | Source | Executed as |
-- |---|---|---|
-- | `defaults` | `pi.config.defaults` | ordinary Lua table |
-- | `user` | XDG `config.lua`, else legacy `settings.json` | sandboxed chunk / JSON |
-- | `project` | `<root>/.pi/config.lua` | sandboxed chunk, trusted directories only |
--
-- A user configuration file is *not* a package: it is a chunk evaluated with
-- an explicit environment that holds pure Lua libraries and nothing else — no
-- `pi`, no `io`, no `os`, no loader. Capability arrives the ordinary way, by
-- naming packages in `packages`, which are then loaded through
-- `pi.packages.v1` like any other package. That keeps one loading mechanism
-- and makes a configuration file safe to read at face value.
--
-- Publication is atomic. Discovery, validation, merging, and package loading
-- all complete before anything visible changes; any failure leaves the
-- previous effective configuration, its package generation, and its revision
-- exactly as they were. Recomposing an unchanged configuration publishes
-- nothing and does not reload packages, so a repeated startup is free.
--
-- Nothing here writes a configuration file, and nothing writes under the
-- legacy root: the only durable write is a trust decision under the state
-- root.

local pi = ...
local kernel = pi.kernel.v1
local module = kernel.module
local roots = pi.roots.v1
local effects = pi.effects.v1
local fs, path = effects.fs, effects.path
local packages = pi.packages.v1
local models = pi.models.v1

local paths = module.require("pi.config.paths", "1")
local schema = module.require("pi.config.schema", "1")
local json = module.require("pi.config.json", "1")
local trust = module.require("pi.config.trust", "1")
local defaults = module.require("pi.config.defaults", "1")
local apply = module.require("pi.config.apply", "1")
local tool_policy = module.require("pi.config.tools", "1")

local SECTIONS = schema.schema.fields

-- Pure libraries a configuration chunk may use. Each is wrapped read-only so
-- one file cannot rewrite `string.format` for the whole VM.
local function readonly(library)
  return setmetatable({}, {
    __index = library,
    __newindex = function()
      error("configuration may not modify a standard library", 0)
    end,
    __metatable = false,
  })
end

local function sandbox()
  return {
    assert = assert,
    error = error,
    ipairs = ipairs,
    next = next,
    pairs = pairs,
    select = select,
    tonumber = tonumber,
    tostring = tostring,
    type = type,
    math = readonly(math),
    string = readonly(string),
    table = readonly(table),
    utf8 = readonly(utf8),
  }
end

local state = {
  revision = 0,
  loaded = false,
  settings = nil,
  provenance = {},
  sources = {},
  context = nil,
  package_sources = {},
  generation = {},
  declarations = nil,
  plan = {},
  tools = nil,
  modules = {},
  options = {},
  errors = {},
}

-- ---------------------------------------------------------------------------
-- Layer evaluation
-- ---------------------------------------------------------------------------

local function evaluate_chunk(file, context)
  local read_ok, text = pcall(fs.read, file)
  if not read_ok then
    return nil, "cannot read " .. file .. ": " .. tostring(text)
  end
  local chunk, message = load(text, "@" .. file, "t", sandbox())
  if chunk == nil then
    return nil, tostring(message)
  end
  local ok, value = pcall(chunk, context)
  if not ok then
    return nil, tostring(value)
  end
  if type(value) ~= "table" then
    return nil, file .. " must return a table, got " .. type(value)
  end
  return value
end

-- Legacy `settings.json` is a foreign format: keys that match a section are
-- used, `null` is dropped, and anything else is reported instead of failing,
-- because pi-rs promises storage provenance for that file, not format
-- compatibility.
local function strip_null(value)
  if type(value) ~= "table" then
    return value
  end
  local result = {}
  for key, item in pairs(value) do
    if item ~= json.null then
      result[key] = strip_null(item)
    end
  end
  return result
end

local function evaluate_legacy(file)
  local read_ok, text = pcall(fs.read, file)
  if not read_ok then
    return nil, "cannot read " .. file .. ": " .. tostring(text)
  end
  local decoded_ok, document = pcall(json.decode, text)
  if not decoded_ok then
    return nil, file .. ": " .. tostring(document)
  end
  if type(document) ~= "table" then
    return nil, file .. " must contain a JSON object"
  end
  local settings, ignored = {}, {}
  for _, key in ipairs(schema.sorted_keys(document)) do
    local value = document[key]
    if value == json.null then -- an explicit null is an unset key
      ignored[#ignored + 1] = key
    elseif SECTIONS[key] ~= nil then
      settings[key] = strip_null(value)
    else
      ignored[#ignored + 1] = key
    end
  end
  return settings, nil, ignored
end

-- ---------------------------------------------------------------------------
-- Trust
-- ---------------------------------------------------------------------------

local function trust_directory(context)
  local resources = context and context.resources
  local row = resources and resources.trust
  return row and row.destination or nil
end

-- The store holds an exclusive lock, so it is opened for one operation and
-- closed again rather than held for the process lifetime.
local function with_trust(context, operation)
  local directory = trust_directory(context)
  if directory == nil then
    return false, "no usable state root for trust decisions"
  end
  local opened, store = pcall(trust.open, { directory = directory })
  if not opened then
    return false, tostring(store)
  end
  local ok, result = pcall(operation, store)
  store.close()
  if not ok then
    return false, tostring(result)
  end
  return true, result
end

-- ---------------------------------------------------------------------------
-- Composition
-- ---------------------------------------------------------------------------

local function chunk_context(context, layer, project_root)
  return {
    layer = layer,
    project_root = project_root,
    paths = {
      config = context.roots.config,
      data = context.roots.data,
      state = context.roots.state,
      cache = context.roots.cache,
      legacy = context.roots.legacy,
    },
  }
end

local function resolve_package(entry, context)
  if path.is_absolute(entry) then
    return entry
  end
  local row = context.resources.packages
  local base = row and (row.selected or row.destination) or nil
  if base == nil then
    return nil, "no packages directory for relative entry '" .. entry .. "'"
  end
  return path.join(base, entry)
end

--- Discover, evaluate, validate, and merge every layer. Nothing is published
--- and no package is loaded here.
local function compose(options)
  options = options or {}
  local context = paths.resolve({
    environment = options.environment,
    project_root = options.project_root,
  })
  local sources, errors = {}, {}
  local settings, provenance = {}, {}

  local function apply_layer(layer, source, values)
    local ok, layer_errors = schema.validate(values)
    if not ok then
      for _, message in ipairs(layer_errors) do
        errors[#errors + 1] = layer .. ": " .. message
      end
      return false
    end
    settings = schema.merge(settings, values, { layer = layer, source = source }, provenance)
    return true
  end

  -- 1. Shipped defaults.
  apply_layer("defaults", "<builtins>", schema.copy(defaults.settings))
  sources[#sources + 1] = {
    layer = "defaults",
    kind = "lua",
    source = "<builtins>",
    outcome = "selected",
  }

  -- 2. User configuration: canonical XDG first, legacy only when the
  --    canonical entry is absent. A present but broken canonical file is an
  --    error, never a fall-through.
  local config_row = context.resources.config
  if config_row.source == "canonical" then
    local values, message = evaluate_chunk(
      config_row.selected,
      chunk_context(context, "user", options.project_root)
    )
    if values == nil then
      errors[#errors + 1] = "user: " .. message
      sources[#sources + 1] = {
        layer = "user",
        kind = "lua",
        source = config_row.selected,
        outcome = "invalid",
        diagnostic = message,
      }
    else
      local accepted = apply_layer("user", config_row.selected, values)
      sources[#sources + 1] = {
        layer = "user",
        kind = "lua",
        source = config_row.selected,
        outcome = accepted and "selected" or "invalid",
      }
    end
  elseif config_row.source == "legacy" then
    local values, message, ignored = evaluate_legacy(config_row.selected)
    if values == nil then
      errors[#errors + 1] = "user: " .. message
      sources[#sources + 1] = {
        layer = "user",
        kind = "json",
        source = config_row.selected,
        outcome = "invalid",
        diagnostic = message,
      }
    else
      local accepted = apply_layer("user", config_row.selected, values)
      local diagnostic = nil
      if #ignored > 0 then
        diagnostic = "ignored legacy keys: " .. table.concat(ignored, ", ")
      end
      sources[#sources + 1] = {
        layer = "user",
        kind = "json",
        source = config_row.selected,
        outcome = accepted and "selected" or "invalid",
        diagnostic = diagnostic,
      }
    end
  else
    sources[#sources + 1] = {
      layer = "user",
      kind = "lua",
      source = config_row.canonical or "<unavailable>",
      outcome = config_row.source,
      diagnostic = config_row.diagnostic,
    }
  end

  -- 3. Project configuration, only for a directory the user has trusted.
  local project = context.project
  if project ~= nil and project.present then
    local read, decision = with_trust(context, function(store)
      return store.decision(project.root)
    end)
    if not read then
      sources[#sources + 1] = {
        layer = "project",
        kind = "lua",
        source = project.file,
        outcome = "untrusted",
        diagnostic = tostring(decision),
      }
    elseif decision == "trust" then
      local values, message = evaluate_chunk(
        project.file,
        chunk_context(context, "project", project.root)
      )
      if values == nil then
        errors[#errors + 1] = "project: " .. message
        sources[#sources + 1] = {
          layer = "project",
          kind = "lua",
          source = project.file,
          outcome = "invalid",
          diagnostic = message,
        }
      else
        local accepted = apply_layer("project", project.file, values)
        sources[#sources + 1] = {
          layer = "project",
          kind = "lua",
          source = project.file,
          outcome = accepted and "selected" or "invalid",
        }
      end
    else
      sources[#sources + 1] = {
        layer = "project",
        kind = "lua",
        source = project.file,
        outcome = decision == "deny" and "denied" or "untrusted",
        diagnostic = decision == "deny"
            and "this project directory is denied"
          or "trust this directory to load its configuration",
      }
    end
  elseif project ~= nil then
    sources[#sources + 1] = {
      layer = "project",
      kind = "lua",
      source = project.file,
      outcome = "absent",
    }
  end

  -- 4. Package selection resolves against the packages resource, so a
  --    relative entry follows the same canonical/legacy precedence as every
  --    other resource.
  local package_sources = {}
  if #errors == 0 then
    local seen = {}
    for _, entry in ipairs(settings.packages or {}) do
      local resolved, message = resolve_package(entry, context)
      if resolved == nil then
        errors[#errors + 1] = "packages: " .. message
      elseif seen[resolved] then
        -- The host refuses to load one source twice, so a duplicate entry is
        -- reported here rather than as a load failure later.
        errors[#errors + 1] = "packages: duplicate entry '" .. resolved .. "'"
      else
        seen[resolved] = true
        package_sources[#package_sources + 1] = resolved
      end
    end
  end

  -- 5. The declaration plan. It is computed here, before anything is
  --    published, so a theme, keymap, or provider the product cannot accept
  --    fails the reload and rolls back with every other failure instead of
  --    half applying afterwards.
  local plan = {}
  if #errors == 0 then
    local rows, plan_errors = apply.plan(settings, provenance)
    for _, message in ipairs(plan_errors) do
      errors[#errors + 1] = message
    end
    if #plan_errors == 0 then
      plan = rows
    end
  end

  -- 6. The tool policy, validated against the live suite for the same
  --    reason: a suppressed tool that does not exist is a typo, and a typo
  --    must fail the reload rather than quietly change nothing. Applying it
  --    happens later, in the `-50` stage, because the distribution's own
  --    tool-root stage runs after publication.
  local tools = nil
  if #errors == 0 then
    local policy, tool_errors = tool_policy.plan(settings)
    for _, message in ipairs(tool_errors) do
      errors[#errors + 1] = message
    end
    if #tool_errors == 0 then
      tools = policy
    end
  end

  return {
    ok = #errors == 0,
    settings = settings,
    provenance = provenance,
    sources = sources,
    context = context,
    package_sources = package_sources,
    plan = plan,
    tools = tools,
    errors = errors,
  }
end

-- ---------------------------------------------------------------------------
-- Publication
-- ---------------------------------------------------------------------------

local function dispose_generation(generation)
  -- Dispose in reverse load order so a composed package outlives the one that
  -- required it.
  for index = #generation, 1, -1 do
    local handle = generation[index]
    if handle ~= nil and not handle:disposed() then
      pcall(handle.dispose, handle)
    end
  end
end

-- Host errors carry a stack traceback; a configuration diagnostic wants the
-- sentence, not the trace.
local function first_line(value)
  local text = tostring(value)
  return (string.match(text, "^[^\n]*")) or text
end

-- Reconcile the live package generation with the selected one. A source that
-- is already loaded is *kept*, never reloaded: the host refuses to load one
-- source twice, and restarting an unrelated package because some other
-- setting changed would be a surprise. Only genuinely new sources load, and
-- the sources that left the selection are returned for disposal after the
-- publication swap. The packages this attempt introduced are returned too, so
-- a later step of the same attempt can still undo them.
local function reconcile_generation(next_sources, errors)
  local live = {}
  for index, source in ipairs(state.package_sources) do
    local handle = state.generation[index]
    if handle ~= nil and not handle:disposed() then
      live[source] = handle
    end
  end

  local generation, added = {}, {}
  for _, source in ipairs(next_sources) do
    local handle = live[source]
    if handle ~= nil then
      live[source] = nil
    else
      local ok, loaded = pcall(packages.load, { path = source })
      if not ok then
        errors[#errors + 1] = "packages: cannot load " .. source .. ": " .. first_line(loaded)
        -- Only the packages this attempt introduced are disposed; every
        -- retained package stays exactly as it was.
        dispose_generation(added)
        return nil
      end
      handle = loaded
      added[#added + 1] = handle
    end
    generation[#generation + 1] = handle
  end

  local retired = {}
  for _, handle in pairs(live) do
    retired[#retired + 1] = handle
  end
  return generation, retired, added
end

local function same_list(left, right)
  if #left ~= #right then
    return false
  end
  for index, value in ipairs(left) do
    if right[index] ~= value then
      return false
    end
  end
  return true
end

-- Configuration-derived declarations live in their own package generation.
-- `pi.kernel.v1.declare` refuses a second declaration of one kind and id, and
-- a declaration lives exactly as long as the package scope that made it, so
-- the previous declaration package is disposed *before* the next one loads.
-- The ids are deliberately stable — a consumer asks for "the configured
-- theme", not for revision seven's theme — and that is what makes the order
-- dispose-then-load rather than load-then-dispose.
--
-- The plan being replayed was already validated during composition, so this
-- step is a replay of accepted data; if the host refuses it anyway, the
-- caller puts the previous plan back.
local function publish_declarations(plan, errors)
  if state.declarations ~= nil then
    if not state.declarations:disposed() then
      pcall(state.declarations.dispose, state.declarations)
    end
    state.declarations = nil
  end
  if #plan == 0 then
    return true
  end
  apply.stage(plan)
  local ok, handle = pcall(packages.load, {
    name = apply.package_name,
    source = apply.source,
  })
  apply.stage(nil)
  if not ok then
    errors[#errors + 1] = "declarations: " .. first_line(handle)
    return false
  end
  state.declarations = handle
  return true
end
--- Compose and publish. On any failure the previous publication — settings,
--- provenance, package generation, and revision — stays exactly as it was.
---
--- The observations move even when the policy does not: `sources()` and
--- `errors()` always describe the most recent attempt, because the reason a
--- reload was refused is exactly what the user needs to see, while
--- `effective()`, `provenance()`, and `revision()` keep describing the last
--- configuration that actually took effect.
local function reload(options)
  options = options or state.options or {}
  state.options = options
  local report = compose(options)
  if not report.ok then
    state.sources = report.sources
    state.context = report.context
    state.errors = report.errors
    report.changed = false
    report.revision = state.revision
    return report
  end

  local unchanged = state.loaded
    and schema.equal(report.settings, state.settings)
    and same_list(report.package_sources, state.package_sources)
  if unchanged then
    -- Idempotent recomposition: republish the observations that describe the
    -- same configuration, but keep the revision and the live packages.
    state.sources = report.sources
    state.context = report.context
    state.errors = {}
    report.changed = false
    report.revision = state.revision
    return report
  end

  local errors = {}
  local generation, retired, added = reconcile_generation(report.package_sources, errors)
  if generation == nil then
    report.ok = false
    report.errors = errors
    report.changed = false
    report.revision = state.revision
    state.errors = errors
    return report
  end

  -- Applying the sections is part of the same attempt: a module identity the
  -- configuration pins but no package provides, or a declaration the host
  -- refuses, rolls the whole reload back instead of publishing settings the
  -- product could not act on.
  local resolved, module_errors = apply.pin(report.settings)
  for _, message in ipairs(module_errors) do
    errors[#errors + 1] = message
  end
  local applied = #module_errors == 0 and publish_declarations(report.plan, errors)
  if not applied then
    if #module_errors == 0 then
      -- Only the declaration swap retracts anything before it can fail; put
      -- the still-published configuration's declarations back.
      publish_declarations(state.plan, {})
    end
    dispose_generation(added)
    report.ok = false
    report.errors = errors
    report.changed = false
    report.revision = state.revision
    state.errors = errors
    return report
  end

  -- Swap first, then dispose what the new selection retired: a disposal
  -- failure can no longer leave the published configuration half-applied.
  state.settings = report.settings
  state.provenance = report.provenance
  state.sources = report.sources
  state.context = report.context
  state.package_sources = report.package_sources
  state.generation = generation
  state.plan = report.plan
  state.tools = report.tools
  state.modules = resolved
  state.revision = state.revision + 1
  state.loaded = true
  state.errors = {}
  dispose_generation(retired)

  report.changed = true
  report.revision = state.revision
  return report
end

-- Package-scope cleanup: disposing the configuration package disposes every
-- package its configuration selected — including the declaration package that
-- carries its theme, keymaps, and providers — so no configuration-derived
-- declaration outlives the configuration itself.
kernel.resource(function()
  if state.declarations ~= nil and not state.declarations:disposed() then
    pcall(state.declarations.dispose, state.declarations)
  end
  state.declarations = nil
  dispose_generation(state.generation)
  state.generation = {}
end)

-- ---------------------------------------------------------------------------
-- Public module
-- ---------------------------------------------------------------------------

module.define({
  name = "pi.config.settings",
  version = "1",
  factory = function()
    local api = {}

    api.reload = reload

    function api.ensure(options)
      if state.loaded then
        return { ok = true, changed = false, revision = state.revision, errors = {} }
      end
      return reload(options)
    end

    function api.loaded()
      return state.loaded
    end

    function api.revision()
      return state.revision
    end

    function api.effective()
      return schema.copy(state.settings or {})
    end

    function api.provenance()
      local result = {}
      for key, origin in pairs(state.provenance) do
        result[key] = { layer = origin.layer, source = origin.source }
      end
      return result
    end

    function api.sources()
      return schema.copy(state.sources)
    end

    function api.resources()
      return schema.copy(state.context and state.context.resources or {})
    end

    function api.roots()
      return schema.copy(state.context and state.context.roots or {})
    end

    function api.package_sources()
      local result = {}
      for index, source in ipairs(state.package_sources) do
        result[index] = source
      end
      return result
    end

    --- The declaration rows the published configuration produced, in the
    --- order they were declared. `pi.kernel.v1.registered(kind)` is the same
    --- information from the consumer's side; this is the configuration's own
    --- account of what it applied.
    function api.declarations()
      local result = {}
      for index, row in ipairs(state.plan) do
        result[index] = { kind = row.kind, definition = schema.copy(row.definition) }
      end
      return result
    end

    --- Module identities the published configuration resolved, in order.
    function api.modules()
      local result = {}
      for index, identity in ipairs(state.modules) do
        result[index] = identity
      end
      return result
    end

    --- The live tool declaration, as the configuration sees it: the root the
    --- shipped suite runs with, the suppressed names, the per-tool settings,
    --- and the revision behind them. `nil` while the distribution's own tool
    --- policy is in force, which is also the zero-configuration answer.
    function api.tools()
      return tool_policy.report(state.tools)
    end

    function api.errors()
      return schema.copy(state.errors)
    end

    function api.leaves()
      return schema.leaves(state.settings or {})
    end

    --- Record a trust decision for one project directory. Returns
    --- `changed`: recording a decision a directory already carries writes
    --- nothing.
    function api.trust(directory, decision)
      local context = state.context or paths.resolve(state.options or {})
      local ok, result = with_trust(context, function(store)
        return store.record(directory, decision)
      end)
      if not ok then
        error(tostring(result), 0)
      end
      return result
    end

    function api.trust_decision(directory)
      local context = state.context or paths.resolve(state.options or {})
      local ok, result = with_trust(context, function(store)
        return store.decision(directory)
      end)
      if not ok then
        return nil
      end
      return result
    end

    function api.trust_list()
      local context = state.context or paths.resolve(state.options or {})
      local ok, result = with_trust(context, function(store)
        return store.list()
      end)
      if not ok then
        return {}
      end
      return result
    end

    return api
  end,
})

-- ---------------------------------------------------------------------------
-- Application seam
-- ---------------------------------------------------------------------------

-- The configuration is composed once, on the first application dispatch —
-- the moment the launcher context first names the project root — and
-- republished into every event so a package reads policy from its snapshot
-- instead of reaching for a module. Recomposition is an explicit
-- `config_reload` event, never per-dispatch work: a dispatch must not stat or
-- read a file every time a key is pressed. A broken configuration file never
-- blocks startup; it publishes diagnostics and leaves the event untouched.
local function configured_model(settings)
  local choice = settings.model
  if type(choice) ~= "table" then
    return nil
  end
  if type(choice.provider) ~= "string" or type(choice.id) ~= "string" then
    return nil
  end
  local ok, found = pcall(models.find, choice.provider, choice.id)
  if not ok then
    return nil
  end
  if type(found) == "table" then
    return found
  end
  return nil
end

local RELOAD_EVENT = "config_reload"

roots.middleware.register({
  kind = "application",
  phase = "event",
  id = "pi.builtins.config",
  order = -200,
  handler = function(snapshot)
    local event = snapshot.event
    if type(event) ~= "table" then
      return nil
    end
    local context = snapshot.context
    local project_root = type(context) == "table" and context.root or nil
    if not state.loaded or event.kind == RELOAD_EVENT then
      pcall(reload, { project_root = project_root })
    end
    if not state.loaded then
      return nil
    end
    local next_event = schema.copy(event)
    next_event.config = schema.copy(state.settings)
    next_event.config_revision = state.revision
    if next_event.model == nil then
      local model = configured_model(state.settings)
      if model ~= nil then
        next_event.model = model
      end
    end
    return { event = next_event }
  end,
})

-- Applying the tool policy is a *second* stage, deliberately ordered after the
-- distribution's own `pi.builtins.defaults.tool-root` (order `-99`). The
-- shipped tool suite is declared when its package loads and re-declared when
-- the launcher root first appears, so a configuration that re-declared it
-- during publication (order `-200`) would be overwritten later in the same
-- dispatch. Configuration outranks a distribution default, so it runs last and
-- wins by ordering rather than by privilege.
--
-- The stage is a comparison first: it re-declares only when the published
-- revision or the launcher root changes.
roots.middleware.register({
  kind = "application",
  phase = "event",
  id = "pi.builtins.config.tools",
  order = -50,
  handler = function(snapshot)
    if not state.loaded then
      return nil
    end
    local context = snapshot.context
    local root = type(context) == "table" and context.root or nil
    local ok, message = tool_policy.reconcile(state.tools, root, state.revision)
    if not ok and message ~= nil then
      -- Publication already happened, so this cannot roll anything back; it
      -- becomes an inspectable diagnostic instead of a broken dispatch.
      state.errors[#state.errors + 1] = "tools: " .. tostring(message)
    end
    return nil
  end,
})
