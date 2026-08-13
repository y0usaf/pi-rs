-- Exerciser for the remaining composable extension-UI operations on the
-- public surface: raw terminal input (onTerminalInput), custom editor
-- (setEditorComponent/getEditorComponent), theme access/switching
-- (getTheme/getAllThemes/setTheme), and editor autocomplete providers
-- (addAutocompleteProvider). Driven through interactive-extension-ui-
-- parity-sequence so effects are observed at the real event/feed seam.
--
-- ui-ops-setup runs against the live interactive ctx: it registers an
-- onTerminalInput listener (remaps "z"->"Z", consumes "q") and reads theme
-- access/switching. Raw-input routing is then observed by typing into the
-- shell editor afterwards and reading uiState.editorText. ui-ops-readback
-- returns the same module state for the readbackCommands seam.
local pi = ...

local state = {
  rawHasCleanup = false,
  themeName = nil,
  themes = 0,
  hasLight = false,
  hasDark = false,
  switchSuccess = false,
  customEditorMounted = false,
  autocompleteRegistered = false,
}

pi.register_command("ui-ops-setup", {
  description = "Register onTerminalInput, theme, autocomplete, custom editor",
  handler = function(_args, ctx)
    local cleanup = ctx.ui.onTerminalInput(function(data)
      if data == "z" then return { data = "Z" } end
      if data == "q" then return { consume = true, data = nil } end
      return nil
    end)
    state.rawHasCleanup = type(cleanup) == "function" and true or false

    local theme = ctx.ui.theme
    state.themeName = theme and theme.name or nil
    state.themes = #(ctx.ui.getAllThemes() or {})
    state.hasLight = ctx.ui.getTheme("light") ~= nil
    state.hasDark = ctx.ui.getTheme("dark") ~= nil
    state.switchSuccess = (ctx.ui.setTheme("light") or {}).success == true

    -- Editor autocomplete provider registration (addAutocompleteProvider).
    local ok_autocomplete, _ = pcall(function()
      ctx.ui.addAutocompleteProvider(function()
        return { triggerCharacters = { "." } }
      end)
    end)
    state.autocompleteRegistered = ok_autocomplete == true

    -- Custom editor mount (getEditorComponent reflects the factory presence).
    ctx.ui.setEditorComponent(function(_, _theme)
      return {
        editor = pi.tui.editor("custom"),
        render = function(self, width) return self.editor:render(width) end,
        handle_input = function(self, data) return self.editor:input_effect(data) end,
      }
    end)
    return { ok = true }
  end,
})

pi.register_command("ui-ops-readback", {
  description = "Report listener/theme/editor state captured by ui-ops-setup",
  handler = function(_, ctx)
    -- customEditorMounted is evaluated lazily from the live context so the
    -- pump has applied the setEditorComponent action by the time we read.
    state.customEditorMounted = (ctx and ctx.ui and ctx.ui.getEditorComponent() ~= nil)
      or state.customEditorMounted or false
    return state
  end,
})