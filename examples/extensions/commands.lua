-- Translation of Pi's commands.ts. Lists extension, prompt-template, and skill
-- slash commands exposed by the current product session.
local pi = ...

pi.register_command("commands", {
  description = "List available slash commands",
  get_argument_completions = function(prefix)
    local items = {}
    for _, source in ipairs({ "extension", "prompt", "skill" }) do
      if source:sub(1, #prefix) == prefix then
        items[#items + 1] = { value = source, label = source }
      end
    end
    if #items == 0 then return nil end
    return items
  end,
  handler = function(args, ctx)
    local commands = pi.get_commands()
    local source_filter = args:match("^%s*(.-)%s*$")
    local filtered = {}
    for _, command in ipairs(commands) do
      if source_filter == "" or command.source == source_filter then
        filtered[#filtered + 1] = command
      end
    end

    if #filtered == 0 then
      ctx.ui.notify(source_filter ~= "" and ("No " .. source_filter .. " commands found") or "No commands found", "info")
      return
    end

    local items = {}
    for _, group in ipairs({
      { key = "extension", label = "Extensions" },
      { key = "prompt", label = "Prompts" },
      { key = "skill", label = "Skills" },
    }) do
      local found = false
      for _, command in ipairs(filtered) do
        if command.source == group.key then
          if not found then
            items[#items + 1] = "--- " .. group.label .. " ---"
            found = true
          end
          local description = command.description and (" - " .. command.description) or ""
          items[#items + 1] = "/" .. command.name .. description
        end
      end
    end

    local selected = ctx.ui.select("Available Commands", items)
    if selected and selected:sub(1, 3) ~= "---" then
      local command_name = selected:match("^/([^ ]+)")
      for _, command in ipairs(commands) do
        if command.name == command_name and command.sourceInfo and command.sourceInfo.path then
          if ctx.ui.confirm(command.name, "View source path?\n" .. command.sourceInfo.path) then
            ctx.ui.notify(command.sourceInfo.path, "info")
          end
          break
        end
      end
    end
  end,
})
