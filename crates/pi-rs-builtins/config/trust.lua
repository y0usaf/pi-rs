-- Durable project-trust decisions.
--
-- A project's `.pi/config.lua` is code that arrived with a checkout rather
-- than with the user, so it is never evaluated until the user has decided
-- about that exact directory. The decision outlives the process, so it is a
-- record in the state root — the generic append-only store, which knows
-- nothing about trust.
--
-- Append-only with last-write-wins gives the idempotence the configuration
-- contract needs: re-recording the decision a directory already has appends
-- nothing, so replaying a startup never grows the file. Revoking is an
-- ordinary later record, so the history stays readable.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.config.trust",
  version = "1",
  factory = function()
    local records = pi.records.v1
    local fs, path = pi.effects.v1.fs, pi.effects.v1.path

    local STORE_NAME = "trust"
    local DECISIONS = { trust = true, deny = true }

    local function normalize(directory)
      if type(directory) ~= "string" or directory == "" then
        error("trust requires a directory path", 0)
      end
      local normalized = path.normalize(directory)
      -- A trailing separator would make the same directory two keys.
      if #normalized > 1 and string.sub(normalized, -1) == path.separator then
        normalized = string.sub(normalized, 1, -2)
      end
      return normalized
    end

    --- Open the trust store under one state directory.
    ---
    --- Asking whether a directory is trusted must not create anything: the
    --- store, its directory, and its lock appear on the first decision, not
    --- on the first question. An absent store simply has no decisions.
    local function open(options)
      options = options or {}
      local directory = options.directory
      if type(directory) ~= "string" or directory == "" then
        error("trust.open requires a directory", 0)
      end
      local file = path.join(directory, STORE_NAME .. "." .. records.extension)
      local store = nil
      if fs.exists(file) then
        store = records.open({ path = file })
      end

      local decisions = {}
      local history = {}

      local function replay()
        decisions = {}
        history = {}
        if store == nil then
          return
        end
        local cursor = store:cursor()
        while true do
          local window = cursor:next()
          for _, record in ipairs(window.records) do
            if type(record) == "table"
              and type(record.directory) == "string"
              and DECISIONS[record.decision]
            then
              decisions[record.directory] = record.decision
              history[#history + 1] = {
                directory = record.directory,
                decision = record.decision,
              }
            end
          end
          if window.done then
            break
          end
        end
      end

      replay()

      local handle = {}

      --- Current decision for one directory, or nil when undecided.
      function handle.decision(target)
        return decisions[normalize(target)]
      end

      --- Record a decision. Returns `changed`: false when the directory
      --- already carries it, so repeating a decision writes nothing. The
      --- decided directory is deliberately *not* named `directory`: that name
      --- belongs to the store's own location, and shadowing it would write
      --- the store into the project being decided about.
      function handle.record(target, decision)
        if not DECISIONS[decision] then
          error("trust decision must be 'trust' or 'deny'", 0)
        end
        local key = normalize(target)
        if decisions[key] == decision then
          return false
        end
        if store == nil then
          fs.make_directory(directory)
          store = records.create({ directory = directory, name = STORE_NAME })
        end
        store:append({ directory = key, decision = decision })
        decisions[key] = decision
        history[#history + 1] = { directory = key, decision = decision }
        return true
      end

      --- Every decision still in force, sorted by directory.
      function handle.list()
        local keys = {}
        for key in pairs(decisions) do
          keys[#keys + 1] = key
        end
        table.sort(keys)
        local rows = {}
        for index, key in ipairs(keys) do
          rows[index] = { directory = key, decision = decisions[key] }
        end
        return rows
      end

      --- Appended decisions in write order, including superseded ones.
      function handle.history()
        local rows = {}
        for index, row in ipairs(history) do
          rows[index] = { directory = row.directory, decision = row.decision }
        end
        return rows
      end

      function handle.record_count()
        return store and store:record_count() or 0
      end

      function handle.path()
        return file
      end

      function handle.exists()
        return store ~= nil
      end

      function handle.close()
        if store ~= nil and not store:closed() then
          store:close()
        end
      end

      function handle.closed()
        return store == nil or store:closed()
      end

      return handle
    end

    return {
      open = open,
      store_name = STORE_NAME,
      decisions = { trust = "trust", deny = "deny" },
      normalize = normalize,
    }
  end,
})
