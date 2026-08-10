-- prime/harness.lua — P4 continual harness as Lua policy.
--
-- A self-contained Lua policy package over the P2 record layer (pi.records).
-- Rust knows none of the names here: schema collections (memory/skill/prompt/
-- subagent/refinement) are data, and local vs. global scope is two store
-- paths chosen by this policy.
--
-- It deliberately does NOT depend on prime/rlm.lua. Both are independent
-- packages loaded through the public loader; the RLM loop boots and runs with
-- an empty harness when this package is absent (verified in the host test
-- prime_harness.rs). When both are present, /refine is available as an
-- ordinary command declaration and the pure projection is the "injected
-- overview" an RLM displays.
--
-- Delivered surface:
--   * scoped store CRUD over pi.records — two paths:
--       local  = <session_dir>/harness.jsonl
--       global = (~/.pi/agent or PI_CODING_AGENT_DIR)/harness.jsonl
--   * /refine as pi.register_command — validated CRUD + rollback +
--     record_refinement.
--   * a pure projection function local+global -> block string with
--     local-over-global precedence, markdown skills surfaced as items.
--   * a role + command exposing the projection for host-side fixture tests.

local pi = ...

local COLLECTIONS = { "memory", "skill", "prompt", "subagent", "refinement" }

local function now_ms()
  return pi.now_ms()
end

local function deep_copy(value, seen)
  seen = seen or {}
  if type(value) ~= "table" then return value end
  if seen[value] then return seen[value] end
  local copy = {}
  seen[value] = copy
  for k, v in pairs(value) do copy[k] = deep_copy(v, seen) end
  return copy
end

local function is_known_collection(name)
  for _, c in ipairs(COLLECTIONS) do if c == name then return true end end
  return false
end

local function agent_root()
  return os.getenv("PI_CODING_AGENT_DIR") or "~/.pi/agent"
end

-- Scope -> store path. Rust knows no collection names; scope is pure path
-- policy owned here.
local function path_for_scope(session_dir, scope)
  if scope == "global" then
    return agent_root() .. "/harness.jsonl"
  end
  return (session_dir or agent_root() .. "/rlm/<session>") .. "/harness.jsonl"
end

-- Two-scope store facade over pi.records. Methods use colon syntax, so a
-- call site must write store:method(...) (self is the leading argument).
local function open_store(session_dir)
  local store = {}
  store.stores = {
    local_store  = pi.records.open(path_for_scope(session_dir, "local")),
    global_store = pi.records.open(path_for_scope(session_dir, "global")),
  }
  local function pick(scope)
    return scope == "global" and store.stores.global_store or store.stores.local_store
  end
  function store:put(scope, collection, key, value)
    pick(scope):put(collection, key, deep_copy(value))
  end
  function store:delete(scope, collection, key)
    pick(scope):delete(collection, key)
  end
  function store:get(scope, collection, key)
    return pick(scope):get(collection, key)
  end
  function store:list(scope, collection)
    return pick(scope):list(collection)
  end
  return store
end

-- ---------------------------------------------------------------------------
-- PURE prompt projection. Deterministic: takes two maps of kind -> entry arrays
-- ({ key, value }) and returns the same rendered string for the same input.
-- Local entries override global entries per (scope, kind, key) — local wins —
-- and global-only entries still surface. Markdown skills (here: the "skill"
-- kind) are special-cased in the block header.
-- ---------------------------------------------------------------------------

local function as_array(t)
  local out = {}
  for _, e in ipairs(t or {}) do out[#out + 1] = e end
  return out
end

local function merge_groups(local_groups, global_groups)
  -- Returns { kind = { key -> entry } } with local-over-global precedence,
  -- preserving stable insertion order (local first, then un-shadowed global).
  local merged = {}
  local order = {}
  local function key_for(kind, key) return kind .. "\0" .. tostring(key) end
  for _, kind in ipairs(COLLECTIONS) do order[kind] = {} end
  for kind, entries in pairs(global_groups or {}) do
    merged[kind] = merged[kind] or {}
    for _, e in ipairs(as_array(entries)) do
      local k = key_for(kind, e.key)
      if merged[kind][k] == nil then
        merged[kind][k] = e
        order[kind][#order[kind] + 1] = k
      end
    end
  end
  for kind, entries in pairs(local_groups or {}) do
    merged[kind] = merged[kind] or {}
    local seen = {}
    for _, e in ipairs(as_array(entries)) do
      local k = key_for(kind, e.key)
      merged[kind][k] = e
      if not seen[k] then
        seen[k] = true
        order[kind][#order[kind] + 1] = k
      end
    end
  end
  return { merged = merged, order = order }
end

local function entry_text(value)
  if type(value) == "string" then return value end
  if type(value) == "table" then
    return value.text or value.description or value.summary or "—"
  end
  return tostring(value)
end

local function render_group(merged, order, group_kind)
  local lines = {}
  for _, k in ipairs(order[group_kind] or {}) do
    local e = merged[group_kind][k]
    lines[#lines + 1] = string.format("- %s %s: %s", group_kind, e.key, entry_text(e.value))
  end
  return lines
end

local function project_block(local_groups, global_groups)
  local g = merge_groups(local_groups, global_groups)
  local lines = {}
  for _, kind in ipairs({ "prompt", "memory" }) do
    for _, line in ipairs(render_group(g.merged, g.order, kind)) do
      lines[#lines + 1] = line
    end
  end
  if #lines == 0 then return "" end
  local header = "# Persistent harness state (durable across turns; update via /refine)"
  lines[#lines + 1] = ""
  -- Markdown skills surface as context data.
  local skill_lines = render_group(g.merged, g.order, "skill")
  if #skill_lines > 0 then
    lines[#lines + 1] = "## Skills"
    for _, line in ipairs(skill_lines) do lines[#lines + 1] = line end
  end
  return header .. "\n\n" .. table.concat(lines, "\n") .. "\n"
end

-- Group the records returned by a scope/list into kind -> entries.
local function collect(session_dir, scopes)
  local groups = {}
  local store = open_store(session_dir)
  for _, scope in ipairs(scopes) do
    local g = {}
    for _, kind in ipairs(COLLECTIONS) do
      g[kind] = store:list(scope, kind)
    end
    groups[scope] = g
  end
  return groups
end

-- ---------------------------------------------------------------------------
-- /refine as an ordinary command declaration. Validated CRUD with rollback and
-- record_refinement, plus get/list.
-- ---------------------------------------------------------------------------

-- Module-local transaction log so rollback can restore prior values across
-- command invocations on the same VM (each /refine call is a separate
-- handler invocation, so state lives here in the package closure).
local txn_active = false
local txn_log = {} -- { {restore=fn} }

local function validate(payload, session_dir)
  local function bad(msg) return nil, msg end
  local op = payload.op
  local scope = payload.scope or "local"
  if scope ~= "local" and scope ~= "global" then return bad("scope must be 'local' or 'global'") end
  local store = open_store(session_dir)
  if op == "put" or op == "update" then
    if not payload.collection or not is_known_collection(payload.collection) then
      return bad("collection must be one of " .. table.concat(COLLECTIONS, ", "))
    end
    if not payload.key or not tostring(payload.key):match("%S") then
      return bad("key must be a non-empty string")
    end
    if payload.value == nil then return bad("value is required for put") end
  elseif op == "delete" then
    if not payload.collection or not is_known_collection(payload.collection) then
      return bad("collection must be one of " .. table.concat(COLLECTIONS, ", "))
    end
    if not payload.key or not tostring(payload.key):match("%S") then
      return bad("key must be a non-empty string")
    end
  elseif op == "get" or op == "list" then
    if payload.collection and not is_known_collection(payload.collection) then
      return bad("collection must be one of " .. table.concat(COLLECTIONS, ", "))
    end
  elseif op == "record_refinement" then
    if not payload.text then return bad("text is required for record_refinement") end
  elseif op == "begin" or op == "commit" or op == "rollback" then
    -- no extra validation
  else
    return bad("op must be one of put, update, delete, get, list, record_refinement, begin, commit, rollback")
  end
  return { store = store, scope = scope, op = op }, nil
end

local function apply_refine(session_dir, payload)
  local spec, verr = validate(payload, session_dir)
  if spec == nil then return { ok = false, error = verr or "validation error" } end
  local store = spec.store
  local scope = spec.scope
  local op = spec.op
  if op == "begin" then
    txn_active = true
    txn_log = {}
    return { ok = true, result = { txn = "open" } }
  end
  if op == "commit" then
    txn_active = false
    txn_log = {}
    return { ok = true, result = { txn = "committed" } }
  end
  if op == "rollback" then
    for i = #txn_log, 1, -1 do
      local entry = txn_log[i]
      entry.restore()
    end
    txn_active = false
    txn_log = {}
    return { ok = true, result = { txn = "rolled_back" } }
  end
  if op == "put" or op == "update" then
    local prior = store:get(scope, payload.collection, payload.key)
    if txn_active then
      txn_log[#txn_log + 1] = {
        restore = function()
          if prior then
            store:put(scope, payload.collection, payload.key, prior)
          else
            store:delete(scope, payload.collection, payload.key)
          end
        end,
      }
    end
    store:put(scope, payload.collection, payload.key, payload.value)
    return { ok = true, result = { op = "put", scope = scope, collection = payload.collection, key = payload.key } }
  end
  if op == "delete" then
    local prior = store:get(scope, payload.collection, payload.key)
    if not prior then
      return { ok = true, result = { op = "delete", existed = false, scope = scope, collection = payload.collection, key = payload.key } }
    end
    if txn_active then
      txn_log[#txn_log + 1] = {
        restore = function() store:put(scope, payload.collection, payload.key, prior) end,
      }
    end
    store:delete(scope, payload.collection, payload.key)
    return { ok = true, result = { op = "delete", existed = true, scope = scope, collection = payload.collection, key = payload.key } }
  end
  if op == "get" then
    local value = store:get(scope, payload.collection, payload.key)
    return { ok = true, result = { op = "get", value = value } }
  end
  if op == "list" then
    local all = {}
    if payload.collection then
      all[payload.collection] = store:list(scope, payload.collection)
    else
      for _, c in ipairs(COLLECTIONS) do all[c] = store:list(scope, c) end
    end
    return { ok = true, result = { op = "list", scope = scope, collections = all } }
  end
  if op == "record_refinement" then
    local key = "refinement-" .. tostring(now_ms())
    store:put(scope, "refinement", key, {
      text = payload.text,
      prompt = payload.prompt,
      timestamp = now_ms(),
    })
    return { ok = true, result = { op = "record_refinement", scope = scope, key = key } }
  end
  return { ok = false, error = "unknown op" }
end

pi.register_command("refine", {
  description = "Update the continual harness store: validated CRUD with rollback and record_refinement.",
  handler = function(args)
    local ok, payload = pcall(pi.json.decode, args or "")
    if not ok or type(payload) ~= "table" then
      return { ok = false, error = "args must be JSON payload" }
    end
    return apply_refine(payload.sessionDir, payload)
  end,
})

-- ---------------------------------------------------------------------------
-- Roles for host-side fixture tests (pure projection + scoped CRUD + overview).
-- ---------------------------------------------------------------------------

local function normalize_groups(raw)
  -- raw: { memory = { {key=.., value=..}, ... }, ... } -> { kind = {key=..,value=..} }
  local out = {}
  for kind, entries in pairs(raw or {}) do
    local arr = {}
    for _, e in ipairs(entries or {}) do
      if type(e) == "table" then
        local key = e.key
        local value = e.value
        if key ~= nil and value ~= nil then arr[#arr + 1] = { key = key, value = value } end
      end
    end
    out[kind] = arr
  end
  return out
end

-- prime-harness-project-pure: { local=..., global=... } -> { block=... }
-- Pure: same input always yields the same output; no store or time touch.
pi.register_role({
  id = "prime-harness-project-pure",
  role = "prime-harness-project-pure",
  active = true,
  priority = 0,
  description = "Pure harness prompt projection (fixture test surface)",
  handler = function(args)
    local ok, payload = pcall(pi.json.decode, args or "")
    if not ok or type(payload) ~= "table" then
      return { error = "args must be JSON" }
    end
    local block = project_block(normalize_groups(payload["local"]), normalize_groups(payload["global"]))
    return { block = block }
  end,
})

-- prime-harness-crud: drive the two-scope store for restart tests.
-- args: { sessionDir, scope, collection, key, value, op=put|get|list|delete }
pi.register_role({
  id = "prime-harness-crud",
  role = "prime-harness-crud",
  active = true,
  priority = 0,
  description = "Scoped harness store CRUD (fixture test surface)",
  handler = function(args)
    local ok, payload = pcall(pi.json.decode, args or "")
    if not ok or type(payload) ~= "table" then
      return { error = "args must be JSON" }
    end
    local store = open_store(payload.sessionDir)
    local scope = payload.scope or "local"
    local collection = payload.collection
    local key = payload.key
    if payload.op == "put" then
      store:put(scope, collection, key, payload.value)
      return { ok = true }
    elseif payload.op == "get" then
      return { ok = true, value = store:get(scope, collection, key) }
    elseif payload.op == "list" then
      return { ok = true, entries = store:list(scope, collection) }
    elseif payload.op == "delete" then
      store:delete(scope, collection, key)
      return { ok = true }
    end
    return { ok = false, error = "unknown crud op" }
  end,
})

-- prime-harness-overview: { sessionDir } -> injected overview block from a
-- fresh session's local+global store (the restart fixture): any memory/skill/
-- prompt written before restart appears here again.
pi.register_role({
  id = "prime-harness-overview",
  role = "prime-harness-overview",
  active = true,
  priority = 0,
  description = "Full harness overview from local+global stores (restart fixture)",
  handler = function(args)
    local ok, payload = pcall(pi.json.decode, args or "")
    if not ok or type(payload) ~= "table" then
      return { error = "args must be JSON" }
    end
    local groups = collect(payload.sessionDir, { "local", "global" })
    local block = project_block(groups["local"], groups["global"])
    return { block = block, sessionDir = payload.sessionDir }
  end,
})
