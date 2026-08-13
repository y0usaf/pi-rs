-- File-backed package consumer of the public resource-resolution engine
-- (`pi.resources`) — the same exact-version module the embedded interactive
-- pack uses. Proves embedded and file-backed packages resolve resources
-- (precedence/dedupe/collisions/toggles/attribution) through one module graph
-- without hidden native modules or a JS runtime.
local pi = ...

pi.register_command("resources-consumer", {
  description = "Resolve resources from a hermetic fixture and report the sorted result",
  handler = function(args)
    local c = pi.json.decode(args)
    local m = pi.module.require("pi.resources", "1")
    local resolved = m.resolve(c.options or {})
    local out = {}
    for _, kind in ipairs({ "extensions", "skills", "prompts", "themes" }) do
      local rows = {}
      if type(resolved[kind]) == "table" then
        for _, entry in ipairs(resolved[kind]) do
          rows[#rows + 1] = {
            path = entry.path,
            enabled = entry.enabled,
            precedence = entry.precedence,
            source = entry.metadata.source,
            scope = entry.metadata.scope,
            origin = entry.metadata.origin,
            baseDir = entry.metadata.baseDir,
          }
        end
      end
      out[kind] = rows
    end
    return out
  end,
})