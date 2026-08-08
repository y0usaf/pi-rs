-- modes/interactive/components/keybinding-hints.ts — the keyText/keyHint
-- slice the tool renderers consume. Bindings come from the tui default
-- keybinding table (getKeybindings); user-configured keybindings arrive
-- with the interaction-shell milestone. The darwin alt→option display
-- rename is carried until a platform binding exists.
do
local pi = ...
local HINT_KEYBINDINGS = {
  ["app.tools.expand"] = "ctrl+o",
}

local function format_key_text(key)
  local result = {}
  for slash_part in key:gmatch("[^/]+") do
    result[#result + 1] = slash_part
  end
  return table.concat(result, "/")
end

local function key_text(binding)
  return format_key_text(HINT_KEYBINDINGS[binding] or "")
end

-- keyHint(keybinding, description); pi styles through the module-global
-- theme, which is the same object the renderers receive — passed here.
local function key_hint(theme, binding, description)
  return theme:fg("dim", key_text(binding)) .. theme:fg("muted", " " .. description)
end

-- Public exact-version module: builtin and file-backed packages import the
-- same closures. No _G export or load-order-only global remains.
pi.module.define({
  name = "pi.tools.keybinding-hints",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      format_key_text = format_key_text,
      key_text = key_text,
      key_hint = key_hint,
      HINT_KEYBINDINGS = HINT_KEYBINDINGS,
    }
  end,
})
end
