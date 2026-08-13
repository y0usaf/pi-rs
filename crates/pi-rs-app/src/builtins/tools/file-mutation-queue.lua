-- file-mutation-queue.ts — serialize mutation operations targeting the
-- same file (keyed by realpath so hardlinked/symlinked paths share a
-- queue), with per-file ownership that is deterministically released.
--
-- Exposed as the public module `pi.tools.file-mutation` so embedded and
-- file-backed mutating tools import the same implementation (PLAN 7/9.9),
-- while a compatibility `with_file_mutation_queue` stays available to the
-- concatenated tool pack. Also registered under `pi.tools.file-mutation-queue`
-- (the PLAN 9.7 exact-version name) so builtin tools and file-backed packages
-- share one dependency mechanism.
--
-- Ownership/leak contract: when the host later drives parallel tool dispatch,
-- each mutation acquires the file's (Lua-side) mutex; a mutation that errors
-- or is disposed before it completes releases the ownership (finally), so no
-- file is left locked (morph's per-file mutation ownership). Any lock still
-- held when the dispatch returns is dropped with the dispatch's state.
--
-- Under the current single-dispatch drive (pi-rs-host vm.rs: `block_on` per
-- message) two tool executions cannot interleave anyway, so the queue reduces
-- to the key computation plus the direct call — matching the spec's promise
-- chain outcome. The module still upholds the serialization and release
-- contract so file-backed translations build on the real mechanism.

local function mutation_queue_key(file_path)
  local resolved = pi.path.resolve(file_path)
  local ok, real = pcall(pi.fs.realpath, resolved)
  if ok then
    return real
  end
  return resolved
end

-- Per-file ownership: a map from queue key -> owner token. Ownership is
-- released on completion or error (finally), so no handle is left held.
local queue_owners = {}

local function release(key)
  queue_owners[key] = nil
end

-- Serialize a mutation on `file_path`: run `fn` while holding that file's
-- ownership. With sequential dispatch this is the direct call; the ownership
-- bookkeeping keeps the lease contract and prepares for parallel dispatch.
local function with_file_mutation_queue(file_path, fn)
  local key = mutation_queue_key(file_path)
  local owner = queue_owners[key]
  if owner then
    -- The same mutation (or a re-entrant one) already holds ownership. Under
    -- sequential dispatch this means concurrent mutation is in progress on the
    -- same file; fail closed rather than corrupt it.
    error(
      ("file is already being mutated: %s (nested/concurrent write)"):format(file_path),
      0
    )
  end
  queue_owners[key] = { held = true }
  local ok, result = pcall(fn)
  release(key)
  if not ok then
    error(result, 0)
  end
  return result
end

local function is_file_locked(file_path)
  local key = mutation_queue_key(file_path)
  return queue_owners[key] ~= nil
end

local function active_lock_count()
  local n = 0
  for _ in pairs(queue_owners) do
    n = n + 1
  end
  return n
end

local function factory()
  return {
    with_file_mutation_queue = with_file_mutation_queue,
    is_file_locked = is_file_locked,
    active_lock_count = active_lock_count,
    key = mutation_queue_key,
    mutation_queue_key = mutation_queue_key,
  }
end

pi.module.define({
  name = "pi.tools.file-mutation",
  version = "1",
  dependencies = {},
  factory = factory,
})

pi.module.define({
  name = "pi.tools.file-mutation-queue",
  version = "1",
  dependencies = {},
  factory = factory,
})
