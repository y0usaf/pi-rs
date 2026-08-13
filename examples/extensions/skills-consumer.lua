-- File-backed package consumer of the public skill loader
-- (`pi.resources.skills`) — the same exact-version module the embedded
-- interactive pack uses. Proves embedded and file-backed packages share one
-- dependency mechanism without hidden native modules.
local pi = ...

pi.register_command("skills-consumer", {
  description = "Load skills from a directory and format them for the system prompt",
  handler = function(args)
    local c = pi.json.decode(args)
    local m = pi.module.require("pi.resources.skills", "1")
    local result = m.load_skills_from_dir(c.dir, c.source or "path")
    local prompt = m.format_skills_for_prompt(result.skills)
    return {
      count = #result.skills,
      names = (function()
        local out = {}
        for _, s in ipairs(result.skills) do out[#out + 1] = s.name end
        return out
      end)(),
      diagnostics = #result.diagnostics,
      prompt = prompt,
    }
  end,
})