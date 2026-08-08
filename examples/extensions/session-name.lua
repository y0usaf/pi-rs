-- session-name: translation of the spec's session-name.ts —
-- set_session_name / get_session_name (PLAN 9.4).
local pi = ...

pi.register_command("session-name", {
  description = "Set or show session name (usage: /session-name [new name])",
  handler = function(args)
    local name = args:match("^%s*(.-)%s*$") or ""
    if name ~= "" then
      pi.set_session_name(name)
      return { set = name, now = pi.get_session_name() }
    end
    return { current = pi.get_session_name() }
  end,
})
