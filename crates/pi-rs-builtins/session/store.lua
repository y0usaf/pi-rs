-- Durable session logs over the public record store.
--
-- This module turns the generic append-only record store (`pi.records.v1`)
-- into *sessions*: a log has an id that is also its file name, a folded state,
-- a branch operation, a compaction record, and a retention rule. It still names
-- no directory of its own — every entry point takes an explicit `directory`
-- (where writes go) and an optional read-only `legacy` directory. Choosing
-- those two locations is `session/init.lua`'s job, which asks the one path
-- policy, `pi.config.paths@1`.
--
-- Two rules make the legacy directory safe:
--
-- 1. `list` reads both directories and labels every row `canonical` or
--    `legacy`; nothing is written to a legacy path, ever.
-- 2. `open` on a legacy-only id copies the log forward into `directory` first
--    and continues there, exactly as the credential store promotes a legacy
--    row on first write. The original file is left untouched.
--
-- Every returned failure is `nil, message`; nothing here raises for an
-- ordinary storage failure, because a session is optional policy and must not
-- be able to break a dispatch that would otherwise succeed.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.session.store",
  version = "1",
  factory = function()
    local store_api = pi.records.v1
    local effects = pi.effects.v1
    local fs, path = effects.fs, effects.path
    local schema = module.require("pi.session.records", "1")

    -- Bound on one reconstruction: a log longer than this folds its prefix and
    -- reports a diagnostic instead of reading without limit.
    local MAX_FOLDED_RECORDS = 4096
    local WINDOW_RECORDS = 256

    -- Ids are time-ordered so a directory listing sorts oldest-first, and a
    -- per-process counter keeps two sessions started in the same second
    -- distinct. `os.date`/`os.time` are ordinary Lua: no host clock exists,
    -- and none is needed.
    local sequence = 0

    local function generate_id()
      sequence = sequence + 1
      local stamp = os.date("!%Y%m%dT%H%M%S")
      return string.format("%s-%04d", stamp, sequence % 10000)
    end

    local function now_ms()
      return math.floor(os.time() * 1000)
    end

    local function first_line(value)
      local text = tostring(value)
      return (string.match(text, "^[^\n]*")) or text
    end

    local function file_name(id)
      return id .. "." .. store_api.extension
    end

    local function id_of(name)
      local suffix = "%." .. store_api.extension .. "$"
      return (name:gsub(suffix, ""))
    end

    local function present(candidate)
      if type(candidate) ~= "string" or candidate == "" then
        return false
      end
      local ok, present = pcall(fs.exists, candidate)
      return ok and present == true
    end

    -- -------------------------------------------------------------------
    -- Reading
    -- -------------------------------------------------------------------

    --- Fold an open store into a session state, bounded by
    --- `MAX_FOLDED_RECORDS`.
    local function fold_store(handle)
      local state = schema.new_state()
      local ok, cursor = pcall(handle.cursor, handle)
      if not ok then
        return nil, first_line(cursor)
      end
      local index = 0
      while true do
        local read, window = pcall(cursor.next, cursor, { max_records = WINDOW_RECORDS })
        if not read then
          return nil, first_line(window)
        end
        for _, record in ipairs(window.records or {}) do
          index = index + 1
          if index > MAX_FOLDED_RECORDS then
            state.diagnostics[#state.diagnostics + 1] =
              "log exceeds " .. MAX_FOLDED_RECORDS .. " records; folded the prefix"
            return state
          end
          schema.apply(state, record, index)
        end
        if window.done then
          return state
        end
      end
    end

    -- -------------------------------------------------------------------
    -- Session handle
    -- -------------------------------------------------------------------

    local Session = {}
    Session.__index = Session

    --- Wrap an open record store plus its folded state.
    local function adopt(handle, state, origin, directory)
      return setmetatable({
        handle = handle,
        state = state,
        origin = origin,
        directory = directory,
        id = state.id or id_of(path.basename(handle:path())),
      }, Session)
    end

    function Session:path()
      return self.handle:path()
    end

    function Session:closed()
      local ok, closed = pcall(self.handle.closed, self.handle)
      return (not ok) or closed == true
    end

    function Session:close()
      pcall(self.handle.close, self.handle)
    end

    function Session:record_count()
      local ok, count = pcall(self.handle.record_count, self.handle)
      if not ok then
        return self.state.records
      end
      return count
    end

    --- Append one already-constructed record and fold it into the live state.
    --- A closed store reports the reason instead of raising, so a stale handle
    --- is an ordinary refusal that the caller can recover from by starting a
    --- new session.
    function Session:append(record)
      local ok, result = pcall(self.handle.append, self.handle, record)
      if not ok then
        return nil, first_line(result)
      end
      schema.apply(self.state, record, self.state.records + 1)
      return result
    end

    function Session:rename(value)
      local built, record = pcall(schema.title, value)
      if not built then
        return nil, first_line(record)
      end
      return self:append(record)
    end

    function Session:compact(options)
      options = options or {}
      local through = tonumber(options.through)
      if through == nil then
        through = #self.state.messages
      end
      if through < 1 or through > #self.state.messages then
        return nil,
          "compaction.through: this session has "
            .. #self.state.messages
            .. " live messages"
      end
      local built, record = pcall(schema.compaction, {
        through = through,
        summary = options.summary,
      })
      if not built then
        return nil, first_line(record)
      end
      return self:append(record)
    end

    --- Copy a prefix of this log to a new id in `directory` and re-identify
    --- the copy with a `branch` record. The copy is returned open; the caller
    --- decides which of the two becomes live.
    function Session:branch(options)
      options = options or {}
      local id = options.id or generate_id()
      local records = tonumber(options.records) or self:record_count()
      if records < 1 then
        return nil, "branch.records: must copy at least the session header"
      end
      local copied, handle = pcall(self.handle.copy, self.handle, {
        directory = self.directory,
        name = id,
        record_count = records,
      })
      if not copied then
        return nil, first_line(handle)
      end
      local state, reason = fold_store(handle)
      if state == nil then
        pcall(handle.close, handle)
        return nil, reason
      end
      local session = adopt(handle, state, "canonical", self.directory)
      local built, record = pcall(schema.branch, {
        id = id,
        parent = { id = self.id, records = records },
      })
      if not built then
        session:close()
        return nil, first_line(record)
      end
      local appended, failure = session:append(record)
      if appended == nil then
        session:close()
        return nil, failure
      end
      session.id = id
      return session
    end

    --- Everything a caller can ask about a live session without folding it
    --- again.
    function Session:report()
      return {
        id = self.id,
        path = self:path(),
        origin = self.origin,
        records = self:record_count(),
        messages = #self.state.messages,
        title = self.state.title,
        model = self.state.model,
        created_ms = self.state.created_ms,
        parent = self.state.parent,
        compactions = self.state.compactions,
        unknown = self.state.unknown,
        diagnostics = self.state.diagnostics,
      }
    end

    --- The same shape `describe` returns for a log that is not open, built
    --- from the live fold. A live log holds its own exclusive lock, so it
    --- must never be reopened to be read.
    function Session:describe()
      local described = self:report()
      local conversation = {}
      for index, message in ipairs(self.state.messages) do
        conversation[index] = {
          role = message.role,
          text = message.text,
          name = message.name,
          call_id = message.call_id,
          ok = message.ok,
          compacted = message.compacted,
          truncated = message.truncated,
        }
      end
      described.conversation = conversation
      return described
    end

    -- -------------------------------------------------------------------
    -- Module entry points
    -- -------------------------------------------------------------------

    --- Start a new log in `directory`. The header is the first record, so a
    --- store that exists at all is identifiable.
    local function start(options)
      options = options or {}
      if type(options.directory) ~= "string" or options.directory == "" then
        return nil, "session directory is required"
      end
      -- A generated id carries a one-second timestamp and a per-process
      -- counter, so two processes that start a session in the same second can
      -- land on the same name. That is a collision, not a failure: try again
      -- with the next counter value. An id the caller *asked* for is tried
      -- once, because silently writing to a different name would be worse.
      local attempts = options.id and 1 or 8
      local id, handle, failure
      for _ = 1, attempts do
        id = options.id or generate_id()
        local created, result = pcall(store_api.create, {
          directory = options.directory,
          name = id,
        })
        if created then
          handle = result
          break
        end
        failure = first_line(result)
        if string.find(failure, "already exists", 1, true) == nil then
          return nil, failure
        end
      end
      if handle == nil then
        return nil, failure
      end

      local built, record = pcall(schema.header, {
        id = id,
        created_ms = options.created_ms or now_ms(),
        title = options.title,
        model = options.model,
        parent = options.parent,
      })
      if not built then
        pcall(handle.close, handle)
        return nil, first_line(record)
      end
      local session = adopt(handle, schema.new_state(), "canonical", options.directory)
      session.id = id
      local appended, failure = session:append(record)
      if appended == nil then
        session:close()
        return nil, failure
      end
      return session
    end

    --- Where an id lives: canonical first, legacy only when the canonical
    --- entry is absent. Mirrors `pi.config.paths@1`'s resource rule.
    local function locate(options)
      local id = options.id
      local canonical = path.join(options.directory, file_name(id))
      if present(canonical) then
        return canonical, "canonical"
      end
      if type(options.legacy) == "string" and options.legacy ~= "" then
        local legacy = path.join(options.legacy, file_name(id))
        if present(legacy) then
          return legacy, "legacy"
        end
      end
      return nil, nil
    end

    --- Open an existing log for appending. A legacy-only log is copied forward
    --- into `directory` first: the resumed session always writes canonically.
    local function open(options)
      options = options or {}
      if type(options.directory) ~= "string" or options.directory == "" then
        return nil, "session directory is required"
      end
      if type(options.id) ~= "string" or options.id == "" then
        return nil, "session id is required"
      end
      local file, origin = locate(options)
      if file == nil then
        return nil, "no session '" .. options.id .. "'"
      end

      local opened, handle = pcall(store_api.open, { path = file })
      if not opened then
        return nil, first_line(handle)
      end
      local state, reason = fold_store(handle)
      if state == nil then
        pcall(handle.close, handle)
        return nil, reason
      end

      if origin == "canonical" then
        return adopt(handle, state, origin, options.directory)
      end

      -- Legacy read, canonical write: copy the whole log forward, then keep
      -- writing to the copy and let go of the legacy file.
      local copied, promoted = pcall(handle.copy, handle, {
        directory = options.directory,
        name = options.id,
      })
      pcall(handle.close, handle)
      if not copied then
        return nil, first_line(promoted)
      end
      local promoted_state, failure = fold_store(promoted)
      if promoted_state == nil then
        pcall(promoted.close, promoted)
        return nil, failure
      end
      local session = adopt(promoted, promoted_state, "promoted", options.directory)
      session.id = options.id
      local built, record = pcall(schema.note, {
        topic = "promoted",
        text = "copied forward from " .. file,
      })
      if built then
        session:append(record)
      end
      return session
    end

    local function listing_of(directory, origin, rows, diagnostics)
      if not present(directory) then
        return
      end
      local ok, listing = pcall(store_api.list, { directory = directory })
      if not ok then
        diagnostics[#diagnostics + 1] = {
          path = directory,
          kind = "io",
          origin = origin,
          message = first_line(listing),
        }
        return
      end
      for _, entry in ipairs(listing.stores or {}) do
        rows[#rows + 1] = {
          id = entry.name,
          path = entry.path,
          origin = origin,
          records = entry.record_count,
          bytes = entry.bytes,
        }
      end
      for _, entry in ipairs(listing.diagnostics or {}) do
        diagnostics[#diagnostics + 1] = {
          path = entry.path,
          kind = entry.kind,
          origin = origin,
          message = entry.message,
        }
      end
    end

    --- Every session visible in both directories, canonical first. A file that
    --- is locked, damaged, or not a record store at all becomes a diagnostic
    --- row rather than disappearing.
    local function list(options)
      options = options or {}
      local rows, diagnostics = {}, {}
      listing_of(options.directory, "canonical", rows, diagnostics)
      local seen = {}
      for _, row in ipairs(rows) do
        seen[row.id] = true
      end
      local legacy_rows = {}
      listing_of(options.legacy, "legacy", legacy_rows, diagnostics)
      for _, row in ipairs(legacy_rows) do
        -- A canonical log shadows its legacy counterpart, the same way the
        -- path policy prefers a canonical resource entry.
        row.shadowed = seen[row.id] == true
        rows[#rows + 1] = row
      end
      return { sessions = rows, diagnostics = diagnostics }
    end

    --- Fold one log without keeping it open. Used to describe a session the
    --- caller is not writing to.
    local function describe(options)
      options = options or {}
      local file, origin = locate(options)
      if file == nil then
        return nil, "no session '" .. tostring(options.id) .. "'"
      end
      local opened, handle = pcall(store_api.open, { path = file })
      if not opened then
        return nil, first_line(handle)
      end
      local state, reason = fold_store(handle)
      pcall(handle.close, handle)
      if state == nil then
        return nil, reason
      end
      return {
        id = state.id or options.id,
        path = file,
        origin = origin,
        records = state.records,
        messages = #state.messages,
        title = state.title,
        model = state.model,
        created_ms = state.created_ms,
        parent = state.parent,
        compactions = state.compactions,
        unknown = state.unknown,
        diagnostics = state.diagnostics,
        conversation = state.messages,
      }
    end

    --- Retention: keep the `keep` most recently modified canonical logs and
    --- remove the rest. Three things are never removed: a legacy log (pi-rs
    --- does not own that directory), a log named in `protect` (which is how
    --- the live session survives), and a log the listing could not open at
    --- all — a locked or damaged file is diagnosed, not deleted, so retention
    --- cannot destroy the one log a reader most wants to look at.
    local function retain(options)
      options = options or {}
      local keep = tonumber(options.keep)
      if keep == nil or keep < 0 or keep ~= math.floor(keep) then
        return nil, "retention.keep: must be a non-negative whole number"
      end
      if not present(options.directory) then
        return { removed = {}, kept = {} }
      end
      local protected = {}
      for _, id in ipairs(options.protect or {}) do
        protected[id] = true
      end

      local candidates = {}
      local inventory = list({ directory = options.directory })
      for _, row in ipairs(inventory.sessions) do
        local ok, info = pcall(fs.stat, row.path)
        candidates[#candidates + 1] = {
          id = row.id,
          path = row.path,
          modified_ms = (ok and tonumber(info.modified_ms)) or 0,
        }
      end
      -- Newest first, then by id so the order is total and reproducible.
      table.sort(candidates, function(left, right)
        if left.modified_ms == right.modified_ms then
          return left.id > right.id
        end
        return left.modified_ms > right.modified_ms
      end)

      -- A protected log counts against `keep` even though the listing cannot
      -- see it: the live session holds its own exclusive lock, so it comes
      -- back as a diagnostic rather than a row, and counting it here keeps
      -- `keep = 2` meaning two logs rather than three.
      local removed, kept = {}, {}
      for _, id in ipairs(options.protect or {}) do
        kept[#kept + 1] = id
      end
      for _, candidate in ipairs(candidates) do
        if protected[candidate.id] or #kept < keep then
          kept[#kept + 1] = candidate.id
        else
          local ok, failure = pcall(fs.remove_file, candidate.path)
          if ok then
            -- The store's lock file sits beside it; leaving it behind would
            -- litter the directory with entries no listing explains.
            pcall(fs.remove_file, candidate.path .. ".lock")
            removed[#removed + 1] = candidate.id
          else
            kept[#kept + 1] = candidate.id
            inventory.diagnostics[#inventory.diagnostics + 1] = {
              path = candidate.path,
              kind = "io",
              origin = "canonical",
              message = first_line(failure),
            }
          end
        end
      end
      return { removed = removed, kept = kept, diagnostics = inventory.diagnostics }
    end

    return {
      max_folded_records = MAX_FOLDED_RECORDS,
      generate_id = generate_id,
      now_ms = now_ms,
      start = start,
      open = open,
      list = list,
      describe = describe,
      retain = retain,
    }
  end,
})
