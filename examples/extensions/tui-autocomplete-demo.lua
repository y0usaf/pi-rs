-- Exercises the CombinedAutocompleteProvider port through the public Lua
-- seam: slash-command menus (fuzzy + argument hints), argument completions
-- via a Lua callback, file-path completion, and applyCompletion.
local pi = ...
pi.register_command("tui-autocomplete-demo", {
  description = "Complete commands and paths through pi.tui.autocomplete_provider",
  handler = function()
    local provider = pi.tui.autocomplete_provider({
      commands = {
        { name = "model", description = "choose a model",
          get_argument_completions = function(prefix)
            local models = { "anthropic/claude-opus-4", "openai/gpt-5.4" }
            local items = {}
            for _, model in ipairs(models) do
              if model:find(prefix, 1, true) then
                items[#items + 1] = { value = model, label = model }
              end
            end
            if #items == 0 then return nil end
            return items
          end },
        { name = "resume", description = "resume a session", argument_hint = "<session>" },
      },
      base_path = pi.cwd(),
    })
    local menu = provider:get_suggestions({ "/re" }, 0, 3, { force = false })
    local arguments = provider:get_suggestions({ "/model gpt" }, 0, 10, { force = false })
    local files = provider:get_suggestions({ "./" }, 0, 2, { force = true })
    local applied = provider:apply_completion({ "/re" }, 0, 3, menu.items[1], menu.prefix)
    return {
      menu = menu,
      arguments = arguments,
      file_count = files and #files.items or 0,
      line = applied.lines[1],
      cursor = applied.cursor_col,
      tab_allowed = provider:should_trigger_file_completion({ "hello " }, 0, 6),
    }
  end,
})
