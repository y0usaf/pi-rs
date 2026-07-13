-- Exercises command-only ExtensionCommandContext session actions through the
-- public file-backed surface. Session replacement work uses only withSession's
-- fresh context; the captured command context must be stale afterwards.
local pi = ...

local function error_text(value)
  local text = tostring(value)
  text = text:match("^(.-)\nstack traceback:") or text
  return text:match("^.-:%d+: (.*)$") or text
end

local observed_event_context
pi.on("tool_call", function(_event, ctx)
  observed_event_context = {
    mode = ctx.mode,
    hasUI = ctx.hasUI,
    cwd = ctx.cwd,
    idle = ctx.isIdle(),
    pending = ctx.hasPendingMessages(),
    commandOnly = type(ctx.newSession) == "function",
  }
end)

pi.register_command("session-lifecycle-observation", {
  description = "Return the most recently observed event context",
  handler = function() return observed_event_context end,
})

pi.register_command("session-lifecycle-demo", {
  description = "Exercise queued session lifecycle actions",
  handler = function(args, ctx)
    local request = args == "" and {} or pi.json.decode(args)
    local before = ctx.sessionManager.get_session_file()
    local before_id = ctx.sessionManager.get_session_id()
    local callback
    local options = {
      withSession = function(fresh)
        callback = {
          mode = fresh.mode,
          cwd = fresh.cwd,
          sessionFile = fresh.sessionManager.get_session_file(),
          sessionId = fresh.sessionManager.get_session_id(),
          idle = fresh.isIdle(),
        }
      end,
    }
    local result
    if request.action == "new" then
      options.parentSession = before
      result = ctx.newSession(options)
    elseif request.action == "fork" then
      options.position = request.position
      result = ctx.fork(request.entryId, options)
    elseif request.action == "tree" then
      result = ctx.navigateTree(request.entryId, request.options or {})
    elseif request.action == "switch" then
      result = ctx.switchSession(request.sessionPath, options)
    elseif request.action == "reload" then
      ctx.reload()
      result = { cancelled = false }
    else
      error("unknown lifecycle action: " .. tostring(request.action), 0)
    end

    local active, stale = pcall(ctx.isIdle)
    return {
      result = result,
      before = before,
      beforeId = before_id,
      callback = callback,
      stale = active and "" or error_text(stale),
    }
  end,
})
