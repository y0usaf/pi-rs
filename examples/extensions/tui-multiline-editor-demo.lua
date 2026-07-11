-- Exercises the pi v0.79.0 multiline editor mechanism through public Lua policy.
local pi = ...

pi.register_command("tui-multiline-editor-demo", {
  description = "Exercise multiline editor state, effects, rendering, paste, and history",
  handler = function()
    -- Legacy WS6.3 methods remain source-compatible.
    local legacy = pi.tui.editor("hello")
    legacy:input("\x1b[D")
    legacy:insert("!")
    legacy:input("\x7f")
    legacy:undo()

    local editor = pi.tui.editor()
    editor:set_text("alpha\r\nbeta\t!")
    local normalized_lines = editor:get_lines()
    editor:insert_text_at_cursor("\nγ")

    local pasted = {}
    for i = 1, 11 do pasted[i] = "paste-" .. i end
    local paste_text = table.concat(pasted, "\n")
    local paste_effect = editor:input_effect("\x1b[200~" .. paste_text .. "\x1b[201~")
    local stored_text = editor:get_text()
    local expanded_text = editor:get_expanded_text()
    local cursor = editor:get_cursor()

    editor:set_padding_x(1)
    editor:set_terminal_rows(12)
    editor:set_focused(true)
    editor:set_autocomplete_max_visible(99)
    local rendered = editor:render(24)
    local submitted = editor:input_effect("\r")
    local after_submit = editor:get_text()
    -- History is app policy (interactive-mode.ts adds after routing), so the
    -- integration records the submission before browsing with the up arrow.
    editor:add_to_history(submitted.text)
    local history_effect = editor:input_effect("\x1b[A")
    local history_text = editor:get_text()

    local newline_editor = pi.tui.editor("policy")
    newline_editor:set_disable_submit(true)
    -- Enter with submit disabled is a no-op; shift+enter inserts the newline.
    local disabled_submit_effect = newline_editor:input_effect("\r")
    local newline_effect = newline_editor:input_effect("\n")

    local completion = pi.tui.editor()
    completion:set_autocomplete_triggers({ "$" })
    completion:input_effect("/")
    completion:input_effect("m")
    local first_request = completion:take_autocomplete_request()
    completion:input_effect("o")
    local current_request = completion:take_autocomplete_request()
    local stale_response = completion:apply_autocomplete(first_request.id, {
      prefix = "/m", items = {{ value = "models", label = "models" }},
    })
    local current_response = completion:apply_autocomplete(current_request.id, {
      prefix = "/mo", items = {
        { value = "models", label = "models", description = "choose a model" },
        { value = "more", label = "more" },
      },
    })
    local completion_render = completion:render(48)
    completion:input_effect("\x1b[B")
    local completion_effect = completion:input_effect("\t")

    local forced = pi.tui.editor("src/")
    forced:set_autocomplete_triggers({})
    forced:input_effect("\t")
    local forced_request = forced:take_autocomplete_request()
    local forced_response = forced:apply_autocomplete(forced_request.id, {
      prefix = "src/", items = {{ value = "src/lib.rs", label = "lib.rs" }},
    })
    forced:undo()
    local forced_undo = forced:get_text()

    return {
      legacy = { value = legacy:value(), cursor = legacy:cursor() },
      normalized_lines = normalized_lines,
      paste_effect = paste_effect,
      stored_text = stored_text,
      expanded_text = expanded_text,
      cursor = cursor,
      rendered = rendered,
      submitted = submitted,
      after_submit = after_submit,
      history_effect = history_effect,
      history_text = history_text,
      disabled_submit_effect = disabled_submit_effect,
      padding_x = editor:padding_x(),
      autocomplete_max_visible = editor:autocomplete_max_visible(),
      newline_effect = newline_effect,
      newline_text = newline_editor:get_text(),
      disable_submit = newline_editor:disable_submit(),
      autocomplete = {
        first_request = first_request,
        current_request = current_request,
        stale_accepted = stale_response.accepted,
        current_accepted = current_response.accepted,
        showing = completion:autocomplete_showing(),
        rendered = completion_render,
        effect = completion_effect,
        value = completion:get_text(),
        forced_request = forced_request,
        forced_value = forced_response.text,
        forced_changed = forced_response.changed,
        forced_undo = forced_undo,
      },
    }
  end,
})
