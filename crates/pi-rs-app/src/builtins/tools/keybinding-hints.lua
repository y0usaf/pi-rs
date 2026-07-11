-- modes/interactive/components/keybinding-hints.ts — the keyText/keyHint
-- slice the tool renderers consume. Bindings come from the tui default
-- keybinding table (getKeybindings); user-configured keybindings arrive
-- with the interaction-shell milestone. The darwin alt→option display
-- rename is carried until a platform binding exists.
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
