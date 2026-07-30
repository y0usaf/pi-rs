-- Credential source policy for the shipped agent.
--
-- Resolution itself is a Rust mechanism. `pi.auth.v1` expands a stored
-- api-key row's `$NAME`/`!command` form, refreshes an expired OAuth row and
-- writes the new row back under the store's own lock, and hands back a key
-- only after checking it. This module owns the one decision Rust cannot make:
-- which store file a provider's row lives in.
--
-- Without this module nothing ever asks the store. `pi.models.v1.stream`
-- reads the `apiKey` option and does not resolve on a caller's behalf, so an
-- agent that never resolves settles every request as
-- "No API key for provider: <id>" no matter what the store holds.

local pi = ...
local module = pi.kernel.v1.module
local auth = pi.auth.v1

module.define({
  name = "pi.agent.credentials",
  version = "1",
  factory = function()
    -- Cached once built. The store holds no secret: it is a pair of paths and
    -- a lock, and every read goes back to the file.
    local store = nil

    -- Store paths are resource policy owned by `pi.config.paths`: the
    -- canonical XDG entry first, the read-only legacy entry only when the
    -- canonical one is absent. A distribution without the config package has
    -- no credential source, and a run there must pass an explicit `apiKey`
    -- agent option instead.
    --
    -- No store is built until a credential file is actually present.
    -- Constructing one takes a lock beside the canonical path and so would
    -- create the state root, which is a visible side effect for a product
    -- that was never asked to store anything.
    local function credential_store()
      if store ~= nil then
        return store
      end
      local function build()
        local paths = module.require("pi.config.paths", "1")
        local row = paths.resolve({}).resources.credentials
        if type(row) ~= "table" or type(row.selected) ~= "string" then
          return nil
        end
        return auth.store({ canonical = row.canonical, legacy = row.legacy })
      end
      local ok, built = pcall(build)
      if not ok or built == nil then
        return nil
      end
      store = built
      return store
    end

    --- Resolve one provider's key.
    ---
    --- Returns the key on success. Returns `nil, nil` when there is simply no
    --- row to read, so the request still fails with the provider's own
    --- diagnostic rather than a second competing one. Returns `nil, reason`
    --- only when a row exists and resolving it failed — an expired OAuth row
    --- whose refresh was rejected is the case that matters, because that is
    --- settled, not transient.
    local function resolve(provider)
      if type(provider) ~= "string" or provider == "" then
        return nil, nil
      end
      local selected = credential_store()
      if selected == nil then
        return nil, nil
      end
      local ok, resolved = pcall(function()
        return selected:resolve(provider)
      end)
      if not ok then
        return nil, "credential for " .. provider .. " could not be resolved: " .. tostring(resolved)
      end
      if type(resolved) ~= "table" or type(resolved.api_key) ~= "string" then
        return nil, nil
      end
      return resolved.api_key, nil
    end

    return { resolve = resolve }
  end,
})
