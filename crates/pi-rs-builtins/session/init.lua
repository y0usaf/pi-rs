-- Shipped session policy: optional persistent conversations.
--
-- Persistence is an *addition*, never a requirement. This package registers
-- two public stages and owns no root, so a distribution that does not load it
-- is exactly the ephemeral application 3.6 shipped: nothing is written, no
-- directory is created, and a `session` event falls through to the application
-- root's own unhandled-event notice.
--
-- | Stage | Kind/phase | Order | Job |
-- |---|---|---|---|
-- | `pi.builtins.session.record` | `agent` / `render` | `100` | folds the settled agent batch into records and appends them |
-- | `pi.builtins.session.command` | `application` / `event` | `-60` | answers `session` events with queued `session_result` actions |
--
-- The recording stage runs *after* the agent root settles and returns the
-- batch unchanged, so persistence can never alter, delay, or fail a turn: an
-- unwritable session directory costs the run one diagnostic in
-- `session_result.error`, not the conversation. That is also why every failure
-- here is caught — the shipped agent must survive a broken disk.
--
-- What is recorded comes from the agent's *public* action vocabulary, so a
-- replacement session package sees exactly what this one sees, and a
-- replacement agent that emits the same actions is persisted with no change
-- here. The consequence is honest and worth stating: `agent_message` publishes
-- the settled text and a tool-call count, not the provider content blocks, so a
-- persisted assistant turn carries its text, and the tool results that follow
-- carry their own id, name, and output.
--
-- Locations come from the one path policy, `pi.config.paths@1`: writes go to
-- the canonical XDG state entry, and `~/.pi/agent/sessions` is read-only.
-- Without that module loaded this package refuses to write at all rather than
-- inventing a second directory rule.

local pi = ...
local module = pi.kernel.v1.module
local middleware = pi.roots.v1.middleware

local schema = module.require("pi.session.records", "1")
local store = module.require("pi.session.store", "1")

local PATHS_NAME, PATHS_VERSION = "pi.config.paths", "1"
local RESOURCE = "sessions"

-- Live state lives at file scope for the same reason `pi.config.tools@1`
-- stages its plan there: disposing *any* package clears every module's cached
-- value, so a factory local would forget the open session whenever an
-- unrelated package was disposed.
local live = nil
local locations = nil
local model_id = nil
-- A model record built before any conversation existed: held here until the
-- first real record starts the log (see `record_batch`).
local deferred_model = nil
local last_error = nil

local function first_line(value)
  local text = tostring(value)
  return (string.match(text, "^[^\n]*")) or text
end

--- Canonical write destination plus the read-only legacy directory, asked of
--- the one path policy. Resolved once and remembered; a configuration reload
--- that changes the environment is 4.4's integration concern.
local function resolve_locations()
  if locations ~= nil then
    if locations.directory ~= nil then
      return locations
    end
    return nil, locations.reason
  end
  local found, paths = pcall(module.require, PATHS_NAME, PATHS_VERSION)
  if not found or type(paths) ~= "table" then
    locations = {
      reason = "no path policy (" .. PATHS_NAME .. "@" .. PATHS_VERSION .. ") is loaded",
    }
    return nil, locations.reason
  end
  local resolved, matrix = pcall(paths.resolve, {})
  if not resolved then
    locations = { reason = first_line(matrix) }
    return nil, locations.reason
  end
  local row = (matrix.resources or {})[RESOURCE]
  if type(row) ~= "table" or row.destination == nil then
    locations = { reason = "no usable state root for sessions" }
    return nil, locations.reason
  end
  locations = {
    directory = row.destination,
    legacy = row.legacy,
    source = row.source,
  }
  return locations
end

local function close_live()
  if live ~= nil then
    live:close()
    live = nil
  end
end

--- The live session, started on first use. A session file therefore appears
--- only once a conversation actually has something to persist.
local function ensure_live()
  if live ~= nil and not live:closed() then
    return live
  end
  live = nil
  local where, reason = resolve_locations()
  if where == nil then
    return nil, reason
  end
  local session, failure = store.start({
    directory = where.directory,
    model = model_id,
  })
  if session == nil then
    return nil, failure
  end
  live = session
  return live
end

-- ---------------------------------------------------------------------------
-- Recording
-- ---------------------------------------------------------------------------

--- Turn one settled agent batch into ordered steps. A step is either a record
--- to append, a *deferred* record that must not by itself start a log, or the
--- one control step this vocabulary has: `agent_reset` ends the log, because a
--- cleared conversation is a different conversation.
local function steps_for(actions)
  local steps = {}
  local function record(built, value)
    if built then
      steps[#steps + 1] = { record = value }
    end
  end
  for _, action in ipairs(actions or {}) do
    local kind = action.kind
    local payload = action.payload
    if type(payload) ~= "table" then
      payload = {}
    end
    if kind == "agent_turn_start" then
      record(pcall(schema.message, { role = "user", text = payload.prompt }))
    elseif kind == "agent_steered" then
      record(pcall(schema.message, { role = "user", text = payload.text }))
    elseif kind == "agent_message" then
      record(pcall(schema.message, { role = "assistant", text = payload.text }))
    elseif kind == "agent_tool_result" then
      record(pcall(schema.message, {
        role = "tool",
        text = payload.output,
        name = payload.name,
        call_id = payload.id,
        ok = payload.ok,
      }))
    elseif kind == "agent_error" then
      record(pcall(schema.note, { topic = "error", text = payload.reason }))
    elseif kind == "agent_cancelled" then
      record(pcall(schema.note, { topic = "cancelled", text = payload.reason }))
    elseif kind == "agent_configured" then
      local id = payload.model
      if type(id) == "string" and id ~= model_id then
        model_id = id
        -- Selecting a model is not a conversation: the shipped distribution
        -- configures one on every startup, so recording it eagerly would
        -- create a durable log for a launch that never says anything. The
        -- record is deferred instead — it is appended when the conversation
        -- actually starts, keeping the written order identical.
        local built, value = pcall(schema.model, id)
        if built then
          steps[#steps + 1] = { record = value, deferred = true }
        end
      end
    elseif kind == "agent_reset" then
      steps[#steps + 1] = { reset = true }
    end
  end
  return steps
end

local function record_batch(actions)
  local steps = steps_for(actions)
  if #steps == 0 then
    return
  end
  for _, step in ipairs(steps) do
    if step.reset then
      close_live()
      -- The next log's header carries the model, so a deferred record left
      -- over from the closed conversation would only repeat it.
      deferred_model = nil
    elseif step.deferred and (live == nil or live:closed()) then
      deferred_model = step.record
    else
      local session, reason = ensure_live()
      if session == nil then
        last_error = reason
        return
      end
      local queued = step.record
      if deferred_model ~= nil then
        local carried = deferred_model
        deferred_model = nil
        if carried ~= queued then
          local appended, failure = session:append(carried)
          if appended == nil then
            last_error = failure
            live = nil
            return
          end
        end
      end
      local appended, failure = session:append(queued)
      if appended == nil then
        -- A closed or unwritable store is dropped rather than retried in a
        -- loop; the next batch starts a fresh log.
        last_error = failure
        live = nil
        return
      end
      last_error = nil
    end
  end
end

middleware.register({
  kind = "agent",
  phase = "render",
  id = "pi.builtins.session.record",
  order = 100,
  handler = function(snapshot)
    -- Persistence never fails a turn: the batch is returned untouched and any
    -- storage failure is reported through `session` commands instead.
    local ok, failure = pcall(record_batch, snapshot.actions)
    if not ok then
      last_error = first_line(failure)
    end
    return nil
  end,
})

-- ---------------------------------------------------------------------------
-- Commands
-- ---------------------------------------------------------------------------

local function status()
  local where, reason = resolve_locations()
  local report = {
    directory = where and where.directory or nil,
    legacy = where and where.legacy or nil,
    source = where and where.source or nil,
    error = last_error or reason,
  }
  if live ~= nil and not live:closed() then
    report.session = live:report()
  end
  return report
end

local function require_live()
  if live == nil or live:closed() then
    return nil, "no live session"
  end
  return live
end

local COMMANDS = {}

function COMMANDS.status()
  return status()
end

function COMMANDS.list()
  local where, reason = resolve_locations()
  if where == nil then
    return nil, reason
  end
  return store.list({ directory = where.directory, legacy = where.legacy })
end

function COMMANDS.describe(event)
  if live ~= nil and not live:closed() and live.id == event.id then
    -- The live log is already folded in memory and holds its own exclusive
    -- lock; reopening it would refuse against that lock.
    return live:describe()
  end
  local where, reason = resolve_locations()
  if where == nil then
    return nil, reason
  end
  return store.describe({
    directory = where.directory,
    legacy = where.legacy,
    id = event.id,
  })
end

function COMMANDS.resume(event)
  local where, reason = resolve_locations()
  if where == nil then
    return nil, reason
  end
  if live ~= nil and not live:closed() and live.id == event.id then
    -- The live log already holds its own exclusive lock; resuming it would
    -- deadlock against that lock instead of reporting the obvious answer.
    return live:report()
  end
  local session, failure = store.open({
    directory = where.directory,
    legacy = where.legacy,
    id = event.id,
  })
  if session == nil then
    return nil, failure
  end
  close_live()
  live = session
  return live:report()
end

function COMMANDS.new()
  close_live()
  last_error = nil
  return { started = false }
end

function COMMANDS.name(event)
  local session, reason = require_live()
  if session == nil then
    return nil, reason
  end
  local appended, failure = session:rename(event.title)
  if appended == nil then
    return nil, failure
  end
  return session:report()
end

function COMMANDS.compact(event)
  local session, reason = require_live()
  if session == nil then
    return nil, reason
  end
  local appended, failure = session:compact({
    through = event.through,
    summary = event.summary,
  })
  if appended == nil then
    return nil, failure
  end
  return session:report()
end

function COMMANDS.branch(event)
  local session, reason = require_live()
  if session == nil then
    return nil, reason
  end
  local branched, failure = session:branch({ id = event.id, records = event.records })
  if branched == nil then
    return nil, failure
  end
  -- The branch becomes the live log: branching is how a conversation
  -- continues differently, not how it is archived.
  close_live()
  live = branched
  return live:report()
end

function COMMANDS.retain(event)
  local where, reason = resolve_locations()
  if where == nil then
    return nil, reason
  end
  local protect = {}
  if live ~= nil and not live:closed() then
    protect[1] = live.id
  end
  return store.retain({
    directory = where.directory,
    keep = event.keep,
    protect = protect,
  })
end

function COMMANDS.close()
  close_live()
  return { closed = true }
end

local function run_command(event)
  local name = tostring(event.command or "status")
  local command = COMMANDS[name]
  if command == nil then
    return { command = name, ok = false, error = "unknown session command '" .. name .. "'" }
  end
  local ok, result, failure = pcall(command, event)
  if not ok then
    return { command = name, ok = false, error = first_line(result) }
  end
  if result == nil then
    return { command = name, ok = false, error = first_line(failure or "session command failed") }
  end
  result.command = name
  result.ok = true
  return result
end

middleware.register({
  kind = "application",
  phase = "event",
  id = "pi.builtins.session.command",
  order = -60,
  handler = function(snapshot)
    local event = snapshot.event
    if type(event) ~= "table" or event.kind ~= "session" then
      return nil
    end
    -- The command is answered here, so the chain stops and these queued
    -- actions become the whole dispatch batch.
    return {
      stop = true,
      actions = { { kind = "session_result", payload = run_command(event) } },
    }
  end,
})
