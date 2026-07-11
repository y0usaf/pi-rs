-- file-mutation-queue.ts — serialize mutation operations targeting the
-- same file (keyed by realpath so hardlinked/symlinked paths share a
-- queue).
--
-- The host currently drives one dispatch at a time to completion
-- (pi-rs-host vm.rs: `block_on` per message), so two tool executions can
-- never interleave and the spec's promise chain reduces to the key
-- computation plus a direct call. Real queuing lands with parallel tool
-- dispatch (WS4). Divergence noted: the spec rethrows non-ENOENT/ENOTDIR
-- realpath errors; pcall falls back to the resolved path for any error.
local function mutation_queue_key(file_path)
  local resolved = pi.path.resolve(file_path)
  local ok, real = pcall(pi.fs.realpath, resolved)
  if ok then
    return real
  end
  return resolved
end

local function with_file_mutation_queue(file_path, fn)
  local _ = mutation_queue_key(file_path)
  return fn()
end
