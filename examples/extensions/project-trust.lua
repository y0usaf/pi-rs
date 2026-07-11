-- Project Trust Extension — translation of pi's
-- examples/extensions/project-trust.ts (spec: ProjectTrustHandler).
--
-- Install globally or load explicitly:
--
--   mkdir -p ~/.pi/agent/extensions
--   cp examples/extensions/project-trust.lua ~/.pi/agent/extensions/
--
-- Multiple handlers for project_trust are allowed. The first handler that
-- returns { trusted = "yes" } or { trusted = "no" } wins and suppresses
-- the built-in trust prompt. Return { trusted = "undecided" } to let
-- another handler or the built-in flow decide.
--
-- This is the non-UI path of pi's example (the ui.select branches return
-- when ctx lands): headless, the decision comes from the filesystem — a
-- project carrying a `.pi-trust` marker file is trusted for the session.
local pi = ...

pi.on("project_trust", function(event)
    if pi.fs.exists(pi.path.join(event.cwd, ".pi-trust")) then
        return { trusted = "yes" }
    end
    return { trusted = "undecided" }
end)
