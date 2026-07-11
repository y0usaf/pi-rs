-- Exerciser for pi.exec (spec: ExtensionAPI.exec → core/exec.ts).
--
-- Distilled from the non-UI path of pi's dirty-repo-guard.ts: block a
-- session switch/fork when the repo has uncommitted changes. The full
-- translation (ctx.hasUI, ctx.ui.select/notify) lands when ctx crosses
-- the bridge in a later WS1 step.
local pi = ...

local function dirty_file_count()
  local r = pi.exec("git", { "status", "--porcelain" })
  if r.code ~= 0 then
    return 0 -- not a git repo (or no git): allow the action
  end
  local n = 0
  for _ in r.stdout:gmatch("[^\n]+") do
    n = n + 1
  end
  return n
end

local function check_dirty_repo()
  if dirty_file_count() > 0 then
    -- Non-interactive: block by default (spec: `if (!ctx.hasUI)`).
    return { cancel = true }
  end
end

pi.on("session_before_switch", check_dirty_repo)
pi.on("session_before_fork", check_dirty_repo)

pi.register_command("git-dirty", {
  description = "Count uncommitted files via pi.exec",
  handler = function()
    return ("%d uncommitted file(s)"):format(dirty_file_count())
  end,
})

-- pi.exec options.signal (spec core/exec.ts): aborting the signal kills
-- the child like a timeout would — the result resolves with killed=true
-- and whatever output arrived first. This is the same mechanism the
-- bash/grep/find tools use for cancellation.
pi.register_command("exec-abort", {
  description = "Abort a running pi.exec via an abort signal",
  handler = function()
    local signal = pi.abort_signal()
    pi.spawn(function()
      pi.sleep(50)
      signal:abort()
    end)
    local r = pi.exec("sh", { "-c", "printf partial; sleep 10" }, { signal = signal })
    return ("killed=%s output=%q"):format(tostring(r.killed), r.stdout)
  end,
})
