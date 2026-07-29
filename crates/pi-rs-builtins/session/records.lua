-- Session record schema and reconstruction.
--
-- Pure Lua: this module names no destination, opens no store, and performs no
-- effect. It decides what a persisted conversation *is* — the record kinds, the
-- text budget, and the fold that turns an append-only log back into a
-- conversation. `pi.session.store@1` makes those records durable and
-- `session/init.lua` decides which of the shipped agent's actions become
-- records.
--
-- The log is append-only, so every later fact is a later record rather than an
-- edit: a rename appends `title`, a model switch appends `model`, and a
-- compaction appends `compaction` instead of rewriting the messages it
-- summarises. Reconstruction is therefore a left fold with no seeking.
--
-- | Record | Meaning when folded |
-- |---|---|
-- | `header` | identity of this log: `id`, `created_ms`, optional `title`/`model`/`parent` |
-- | `message` | one `user`, `assistant`, or `tool` message appended to the conversation |
-- | `title` | renames the session from this point on |
-- | `model` | the model in force from this point on |
-- | `compaction` | replaces messages `1..through` with one summary message |
-- | `branch` | re-identifies the log after a prefix copy: new `id`, `parent` it grew from |
-- | `note` | provenance or diagnostic text; never part of the conversation |
--
-- Folding is deliberately tolerant, because a log is durable and outlives the
-- package that wrote it: an unknown record kind, a missing header, or a
-- compaction pointing past the end is counted and described in `diagnostics`
-- rather than raising. A log written by another package therefore still yields
-- whatever conversation it does contain. Writing is the opposite: every
-- constructor is fail-closed, so a malformed record never reaches disk.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.session.records",
  version = "1",
  factory = function()
    -- Bumped only when a fold rule changes meaning; `header.schema` records
    -- which rules a log was written under.
    local SCHEMA_VERSION = 1

    -- Per-field text budget. The record store refuses a record larger than
    -- `pi.records.v1.default_limits.max_record_bytes` (1 MiB), and a single
    -- tool can emit far more than that, so text is truncated here — a
    -- truncated turn is recoverable, a refused append loses the turn.
    local MAX_TEXT_BYTES = 16384
    local TRUNCATION_SUFFIX = "…[truncated]"

    -- A fold reports at most this many diagnostics; the counters stay exact.
    local MAX_DIAGNOSTICS = 16

    local ROLES = { user = true, assistant = true, tool = true }

    local function fail(message)
      error(message, 0)
    end

    local function text_of(value, field)
      if value == nil then
        return "", false
      end
      if type(value) ~= "string" then
        fail(field .. ": must be a string, got " .. type(value))
      end
      if #value <= MAX_TEXT_BYTES then
        return value, false
      end
      -- Cut on a byte budget, then drop any partial UTF-8 tail so the stored
      -- text stays valid for every later reader.
      local cut = value:sub(1, MAX_TEXT_BYTES)
      while #cut > 0 and utf8.len(cut) == nil do
        cut = cut:sub(1, #cut - 1)
      end
      return cut .. TRUNCATION_SUFFIX, true
    end

    local function identifier(value, field)
      if type(value) ~= "string" or value == "" then
        fail(field .. ": must be a non-empty string")
      end
      -- The id is also the store name on disk, so it is restricted to what
      -- the record store accepts as a file name.
      if value:match("^[%w%.%-_]+$") == nil then
        fail(field .. ": '" .. value .. "' may only use letters, digits, '.', '-', and '_'")
      end
      if value == "." or value == ".." then
        fail(field .. ": '" .. value .. "' is not a usable name")
      end
      return value
    end

    local function count_of(value, field)
      local number = tonumber(value)
      if number == nil or number < 0 or number ~= math.floor(number) then
        fail(field .. ": must be a non-negative whole number")
      end
      return number
    end

    local function parent_of(value, field)
      if value == nil then
        return nil
      end
      if type(value) ~= "table" then
        fail(field .. ": must be a table")
      end
      return {
        id = identifier(value.id, field .. ".id"),
        records = count_of(value.records, field .. ".records"),
      }
    end

    -- ---------------------------------------------------------------------
    -- Constructors: fail-closed, so nothing malformed is ever appended
    -- ---------------------------------------------------------------------

    local function header(fields)
      fields = fields or {}
      local record = {
        kind = "header",
        schema = SCHEMA_VERSION,
        id = identifier(fields.id, "header.id"),
        created_ms = count_of(fields.created_ms or 0, "header.created_ms"),
      }
      if fields.title ~= nil then
        record.title = (text_of(fields.title, "header.title"))
      end
      if fields.model ~= nil then
        record.model = (text_of(fields.model, "header.model"))
      end
      record.parent = parent_of(fields.parent, "header.parent")
      return record
    end

    local function message(fields)
      fields = fields or {}
      if not ROLES[fields.role] then
        fail("message.role: must be 'user', 'assistant', or 'tool'")
      end
      local text, truncated = text_of(fields.text, "message.text")
      local record = { kind = "message", role = fields.role, text = text }
      if truncated then
        record.truncated = true
      end
      if fields.name ~= nil then
        record.name = (text_of(fields.name, "message.name"))
      end
      if fields.call_id ~= nil then
        record.call_id = (text_of(fields.call_id, "message.call_id"))
      end
      if fields.ok ~= nil then
        record.ok = fields.ok and true or false
      end
      return record
    end

    local function title(value)
      return { kind = "title", title = (text_of(value, "title.title")) }
    end

    local function model(value)
      return { kind = "model", model = (text_of(value, "model.model")) }
    end

    local function compaction(fields)
      fields = fields or {}
      local summary, truncated = text_of(fields.summary, "compaction.summary")
      if summary == "" then
        fail("compaction.summary: must be a non-empty string")
      end
      local record = {
        kind = "compaction",
        through = count_of(fields.through, "compaction.through"),
        summary = summary,
      }
      if record.through == 0 then
        fail("compaction.through: must name at least one message")
      end
      if truncated then
        record.truncated = true
      end
      return record
    end

    local function branch(fields)
      fields = fields or {}
      local parent = parent_of(fields.parent, "branch.parent")
      if parent == nil then
        fail("branch.parent: must name the log this branch grew from")
      end
      return {
        kind = "branch",
        id = identifier(fields.id, "branch.id"),
        parent = parent,
      }
    end

    local function note(fields)
      fields = fields or {}
      local topic = text_of(fields.topic, "note.topic")
      if topic == "" then
        fail("note.topic: must be a non-empty string")
      end
      return { kind = "note", topic = topic, text = (text_of(fields.text, "note.text")) }
    end

    -- ---------------------------------------------------------------------
    -- Reconstruction: one left fold, tolerant of foreign records
    -- ---------------------------------------------------------------------

    local function new_state()
      return {
        schema = nil,
        id = nil,
        created_ms = nil,
        title = nil,
        model = nil,
        parent = nil,
        messages = {},
        records = 0,
        compactions = 0,
        notes = 0,
        unknown = 0,
        diagnostics = {},
      }
    end

    local function report(state, message_text)
      if #state.diagnostics < MAX_DIAGNOSTICS then
        state.diagnostics[#state.diagnostics + 1] = message_text
      end
    end

    --- Fold one record into `state`. `index` is the record's one-based
    --- position in the log and is only used to describe a diagnostic.
    local function apply(state, record, index)
      state.records = index
      if type(record) ~= "table" or type(record.kind) ~= "string" then
        state.unknown = state.unknown + 1
        report(state, "record " .. index .. ": not a session record")
        return state
      end

      local kind = record.kind
      if kind == "header" then
        if index ~= 1 then
          report(state, "record " .. index .. ": duplicate session header ignored")
          return state
        end
        state.schema = tonumber(record.schema)
        state.id = record.id
        state.created_ms = tonumber(record.created_ms) or 0
        state.title = record.title
        state.model = record.model
        state.parent = record.parent
        return state
      end

      if index == 1 then
        -- A log that does not start with a header is foreign, not broken:
        -- fold whatever it does carry and say so once.
        report(state, "record 1: no session header; folding a foreign log")
      end

      if kind == "message" then
        if not ROLES[record.role] then
          state.unknown = state.unknown + 1
          report(state, "record " .. index .. ": message with unknown role")
          return state
        end
        state.messages[#state.messages + 1] = {
          role = record.role,
          text = type(record.text) == "string" and record.text or "",
          name = record.name,
          call_id = record.call_id,
          ok = record.ok,
          truncated = record.truncated,
        }
        return state
      end

      if kind == "title" then
        state.title = record.title
        return state
      end

      if kind == "model" then
        state.model = record.model
        return state
      end

      if kind == "branch" then
        state.id = record.id or state.id
        state.parent = record.parent
        return state
      end

      if kind == "compaction" then
        local through = tonumber(record.through) or 0
        if through < 1 or through > #state.messages then
          report(state, "record " .. index .. ": compaction covers no live message")
          return state
        end
        local kept = {
          {
            role = "user",
            text = type(record.summary) == "string" and record.summary or "",
            compacted = true,
            replaced = through,
          },
        }
        for position = through + 1, #state.messages do
          kept[#kept + 1] = state.messages[position]
        end
        state.messages = kept
        state.compactions = state.compactions + 1
        return state
      end

      if kind == "note" then
        state.notes = state.notes + 1
        return state
      end

      state.unknown = state.unknown + 1
      report(state, "record " .. index .. ": unknown record kind '" .. kind .. "'")
      return state
    end

    --- Reconstruct a whole log. `list` is the records in append order.
    local function fold(list)
      local state = new_state()
      for index, record in ipairs(list or {}) do
        apply(state, record, index)
      end
      return state
    end

    return {
      schema_version = SCHEMA_VERSION,
      max_text_bytes = MAX_TEXT_BYTES,
      max_diagnostics = MAX_DIAGNOSTICS,
      header = header,
      message = message,
      title = title,
      model = model,
      compaction = compaction,
      branch = branch,
      note = note,
      new_state = new_state,
      apply = apply,
      fold = fold,
    }
  end,
})
