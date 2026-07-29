-- Shipped default settings: the lowest configuration layer.
--
-- It deliberately carries no visible product policy. A theme, a model, or a
-- keymap default belongs to the package that renders or uses it, so that
-- suppressing the configuration package removes configurability, not the
-- product's appearance. What this layer does own is the *shape*: every
-- section exists, so a higher layer always merges onto something and an
-- unset section is an empty container rather than a missing key.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.config.defaults",
  version = "1",
  factory = function()
    return {
      settings = {
        keymaps = {},
        packages = {},
        modules = {},
        providers = {},
        tools = { suppress = {} },
        roots = {},
      },
    }
  end,
})
