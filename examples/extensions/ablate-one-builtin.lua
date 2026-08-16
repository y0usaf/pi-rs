-- Public-surface ablation of one shipped builtin tool.
--
-- This ordinary file-backed user extension removes exactly ONE first-party
-- builtin tool — the `bash` worker — through the public `pi.unregister_tool`
-- surface (the same RPC-style capability a user extension has; no Rust edit to
-- the ablated unit). Everything else in the tools pack stays loaded. This is
-- the ablation evidence the construction DESIGN requires: a synthetic file
-- source carries no capability, so an ordinary extension can disable a
-- builtin policy unit and the bare core (substrate + remaining packs) still
-- boots.
--
-- `ablate-selfcheck` reports the surviving active tool names so the harness
-- can assert exactly the ablated unit is gone.
local pi = ...

-- Remove exactly the shipped builtin `bash` tool. The rest of the tools pack
-- (`read`, `write`, `edit`, ...) stays active.
pi.unregister_tool("bash")

pi.register_command("ablate-selfcheck", {
  description = "Report active tools after ablating the builtin bash tool",
  handler = function()
    local names = {}
    for _, tool in ipairs(pi.registered_active_tools()) do
      names[#names + 1] = tool.name
    end
    local bashStillThere = false
    for _, name in ipairs(names) do
      if name == "bash" then bashStillThere = true end
    end
    return {
      bashAblated = not bashStillThere,
      tools = names,
    }
  end,
})