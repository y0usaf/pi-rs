-- Exerciser for the reusable per-file mutation queue module (PLAN 9.9):
-- pi.module.require("pi.tools.file-mutation").
--
-- A file-backed mutating tool imports the same queue the builtin write/edit
-- tools use, exercising the ownership/lease contract: a mutation holds the
-- file's ownership while it runs and releases it on completion or error
-- (finally), so no file stays locked. Under sequential dispatch the queue is
-- the direct call plus ownership bookkeeping.
local pi = ...

local mutation = pi.module.require("pi.tools.file-mutation", "1")

pi.register_command("mutation-demo", {
  description = "Exercise the per-file mutation queue lease/release contract",
  handler = function(arg)
    local path = arg
    pi.fs.write_file(path, "initial\n")
    local result = mutation.with_file_mutation_queue(path, function()
      assert(mutation.is_file_locked(path), "ownership should be held inside the mutation")
      local before = mutation.active_lock_count()
      pi.fs.write_file(path, "mutated\n")
      return {
        locked_inside = mutation.is_file_locked(path),
        active_before_release = before,
      }
    end)
    local after = mutation.with_file_mutation_queue(path, function()
      return pi.fs.read_file(path)
    end)
    -- Ownership must be fully released after the first mutation errors too.
    local released_after_error = true
    local ok = pcall(function()
      mutation.with_file_mutation_queue(path, function()
        error("boom", 0)
      end)
    end)
    if not ok then
      released_after_error = mutation.active_lock_count() == 0
    end
    return {
      locked_inside = result.locked_inside,
      active_before_release = result.active_before_release,
      content = after,
      released_after_error = released_after_error,
      locks_after = mutation.active_lock_count(),
    }
  end,
})