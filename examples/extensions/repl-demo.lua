-- repl-demo.lua — P1 pi.repl tier-2 binding exerciser.
--
-- File-backed proof that the kernel bridge works from ordinary Lua policy:
-- spawn, host_request pump (pi.spawn coroutine), execute with persistence,
-- snapshot/restore, interrupt, shutdown. Returns tables the Rust test
-- asserts against.

local pi = ...

pi.register_command("repl-basic", {
  description = "spawn a kernel, pump host_requests, execute cells",
  handler = function()
    local kernel, requests = pi.repl.spawn({ watchdog_ms = 30000 })

    -- The host_request pump: kernel cells await rlm.host_request(...);
    -- this coroutine answers them (doctrine 02: events in, actions out).
    pi.spawn(function()
      while true do
        local req = requests:receive()
        local kind = req:get_kind()
        if kind == "demo.echo" then
          local payload = req:get_payload()
          req:reply({ status = "ok", echoed = payload.n })
        else
          req:reply({ status = "error", error = "unknown kind: " .. kind })
        end
      end
    end)

    local r1 = kernel:execute("x = 1")
    assert(r1.status == "ok", "cell1 status")

    local r2 = kernel:execute("x + 1")
    assert(r2.status == "ok" and r2.result == "2", "persistence across cells")

    -- host_request round trip from inside a cell (via the vendored rlm).
    -- The await expression is the last statement so its value is the cell
    -- result (an assignment cell would report no result).
    local r3 = kernel:execute("import asyncio, rlm\nawait rlm.host_request('demo.echo', {'n': 42})")
    assert(r3.status == "ok" and r3.result and r3.result:find("42"), "host_request round trip")

    -- stdout/stderr capture.
    local r4 = kernel:execute("print('out'); import sys; print('err', file=sys.stderr)")
    assert(r4.stdout:find("out") and r4.stderr:find("err"), "stream capture")

    -- exception reporting.
    local r5 = kernel:execute("1 / 0")
    assert(r5.status == "error" and r5.error.ename == "ZeroDivisionError", "exception")

    kernel:shutdown()
    return {
      persistence = r2.result,
      host_request_ok = r3.status == "ok",
      stdout = r4.stdout,
      stderr = r4.stderr,
      exception = r5.error.ename,
    }
  end,
})

pi.register_command("repl-snapshot", {
  description = "kernel snapshot/restore round trip",
  handler = function()
    local kernel, requests = pi.repl.spawn({ watchdog_ms = 30000 })
    pi.spawn(function()
      while true do
        local req = requests:receive()
        req:reply({ status = "error", error = "no handler" })
      end
    end)
    kernel:execute("x = 42")
    local tag = tostring(pi.now_ms())
    local dill_path = "/tmp/repl-demo-" .. tag .. ".dill"
    local manifest_path = "/tmp/repl-demo-" .. tag .. ".json"
    local snap = kernel:snapshot(dill_path, manifest_path)
    assert(snap.saved and #snap.saved > 0, "snapshot saved something")
    local restored = kernel:restore(dill_path)
    local r = kernel:execute("x + 100")
    kernel:shutdown()
    os.remove(dill_path)
    os.remove(manifest_path)
    return {
      snapshot_saved_x = contains(snap.saved, "x"),
      restore_restored_x = contains(restored.restored, "x"),
      value_after_restore = r.result,
    }
  end,
})

function contains(list, item)
  for _, v in ipairs(list or {}) do
    if v == item then return true end
  end
  return false
end

pi.register_command("repl-leak", {
  description = "spawn a kernel and rely on VM-drop disposal (no explicit shutdown)",
  handler = function()
    local kernel, requests = pi.repl.spawn({ watchdog_ms = 30000 })
    pi.spawn(function()
      while true do
        local req = requests:receive()
        req:reply({ status = "error", error = "no handler" })
      end
    end)
    local r = kernel:execute("1 + 1")
    return { value = r.result }  -- kernel deliberately NOT shut down here
  end,
})
