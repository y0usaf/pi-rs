-- Exerciser for the managed subprocess mechanism: pi.process.spawn and
-- the process-tree cancellation/pipe primitives it exposes.
--
-- Translations from Node built-ins the dogfood suite relies on:
--   node:child_process#spawn        → pi.process.spawn
--   child_process#ChildProcess      → the returned handle (pid, pipes, wait)
--   process.kill                    → handle:kill / pi.process.kill
--   process.stdio_pipes             → handle:read_stdout/read_stderr/write_stdin
--
-- The handle owns the child and kills its whole process tree on disposal,
-- so a dropped handle (reload/shutdown) leaves no process behind.
local pi = ...

-- Spawn a child that reads stdin, writes to stdout and stderr, and exits
-- with a known code; drive it through its pipes.
pi.register_command("process-demo", {
  description = "Spawn a child and drive its pipes via pi.process",
  handler = function()
    local p = pi.process.spawn("sh", {
      "-c",
      [[read line; echo "got:$line" >&1; echo "errline" >&2; exit 3]],
    })
    p:write_stdin("ping\n")
    pi.sleep(30)
    local out = p:read_stdout()
    local err = p:read_stderr()
    local code = p:wait()
    return {
      pid_positive = p:pid() > 0,
      out = out,
      err = err,
      code = code,
      running_after_wait = p:is_running(),
    }
  end,
})

-- Kill a long-running child tree via an AbortSignal.
pi.register_command("process-abort", {
  description = "Kill a spawned process tree on AbortSignal",
  handler = function()
    local signal = pi.abort_signal()
    local p = pi.process.spawn("sh", { "-c", "sleep 60" }, { signal = signal })
    local pid = p:pid()
    signal:abort()
    pi.sleep(50) -- give the signal-driven killer a tick
    local code = p:wait()
    return { pid = pid, signal_killed = pid > 0 and code == nil, code = code }
  end,
})

-- Explicit disposal kills the tree immediately and deterministically; wait()
-- then reaps and reports the signal death (nil code).
pi.register_command("process-disposal", {
  description = "Verify a spawned process is reaped via explicit dispose",
  handler = function()
    local p = pi.process.spawn("sh", { "-c", "sleep 60" })
    local pid = p:pid()
    p:dispose()
    local code = p:wait() -- reaps; signal death => nil
    return { spawned = pid > 0, terminated = code == nil }
  end,
})