-- File-mutation serialization for the shipped core tools.
--
-- Effects yield: a tool that reads, edits, and writes a file crosses the
-- bounded effect queue several times, so two mutations of the same path could
-- interleave inside one dispatch (or across nested dispatches) and lose an
-- edit. Serialization is tool policy, not a host privilege: this module is an
-- ordinary cooperative lock plus a per-path revision that a replacement tool
-- package may ignore.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.locks",
  version = "1",
  factory = function()
    local effects = pi.effects.v1
    local DEFAULT_WAIT_MS = 5000
    local POLL_MS = 2

    local held = {}
    local revisions = {}
    local sequence = 0

    local function acquire(path, options)
      if type(path) ~= "string" or #path == 0 then
        return nil, "lock path must be a non-empty string"
      end
      options = options or {}
      local budget = tonumber(options.wait_ms) or DEFAULT_WAIT_MS
      local waited = 0
      while held[path] ~= nil do
        if waited >= budget then
          return nil, "path is busy: " .. path
        end
        effects.timer.sleep(POLL_MS)
        waited = waited + POLL_MS
      end
      sequence = sequence + 1
      local token = { path = path, id = sequence }
      held[path] = token
      return token
    end

    local function release(token)
      if type(token) == "table" and held[token.path] == token then
        held[token.path] = nil
        return true
      end
      return false
    end

    local function is_held(path)
      return held[path] ~= nil
    end

    local function revision(path)
      return revisions[path] or 0
    end

    local function bump(path)
      revisions[path] = revision(path) + 1
      return revisions[path]
    end

    -- Runs `body` while holding the path lock and always releases it, so a
    -- failing tool cannot strand the path for the rest of the session.
    local function guard(path, body, options)
      local token, reason = acquire(path, options)
      if token == nil then
        return nil, reason
      end
      local ok, result = pcall(body)
      release(token)
      if not ok then
        error(result, 0)
      end
      return result
    end

    return {
      acquire = acquire,
      release = release,
      guard = guard,
      is_held = is_held,
      revision = revision,
      bump = bump,
      default_wait_ms = DEFAULT_WAIT_MS,
    }
  end,
})
