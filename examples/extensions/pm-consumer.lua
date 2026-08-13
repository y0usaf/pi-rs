-- File-backed package consumer of the public package-manager module
-- (`pi.packages`) — the same exact-version module the embedded interactive
-- pack uses. Proves embedded and file-backed packages share one dependency
-- mechanism and one package-lifecycle channel without hidden native modules
-- or a JS runtime.
local pi = ...

pi.register_command("pm-consumer", {
  description = "Report how a configured source routes to a transport",
  handler = function(args)
    local c = pi.json.decode(args)
    local m = pi.module.require("pi.packages", "1")
    local parsed = m.parse_source(c.source)
    return {
      type = parsed.type,
      identity = m.package_identity(c.source),
      added = m.add_source_to_settings(c.source, { ["local"] = c["local"] or false }),
    }
  end,
})