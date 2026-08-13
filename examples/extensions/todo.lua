-- Translation of Pi v0.79.0 examples/extensions/todo.ts.
-- Manage a todo list via a `todo` tool, with state stored in tool-result
-- details so forking keeps the correct history. A /todos command shows the
-- current list in the TUI.
local pi = ...

-- In-memory state (reconstructed from the session on load)
local todos = {}
local next_id = 1

local function reconstruct_state(ctx)
  todos = {}
  next_id = 1
  for _, entry in ipairs(ctx.sessionManager:get_branch()) do
    if entry.type == "message" then
      local msg = entry.message
      if msg.role == "toolResult" and msg.toolName == "todo" then
        local details = msg.details
        if details and details.todos then
          todos = details.todos
          next_id = details.nextId
        end
      end
    end
  end
end

local function copy_todos()
  local out = {}
  for _, t in ipairs(todos) do out[#out + 1] = { id = t.id, text = t.text, done = t.done } end
  return out
end

pi.on("session_start", function(_event, ctx) reconstruct_state(ctx) end)
pi.on("session_tree", function(_event, ctx) reconstruct_state(ctx) end)

pi.register_tool({
  name = "todo",
  label = "Todo",
  description = "Manage a todo list. Actions: list, add (text), toggle (id), clear",
  parameters = {
    type = "object",
    properties = {
      action = { type = "string", enum = { "list", "add", "toggle", "clear" }, description = "Todo action" },
      text = { type = "string", description = "Todo text (for add)" },
      id = { type = "number", description = "Todo ID (for toggle)" },
    },
    required = { "action" },
  },

  execute = function(_tool_call_id, params, _signal, _on_update, _ctx)
    local action = params.action
    if action == "list" then
      local lines = {}
      for _, t in ipairs(todos) do lines[#lines + 1] = "[" .. (t.done and "x" or " ") .. "] #" .. t.id .. ": " .. t.text end
      return {
        content = { { type = "text", text = #lines > 0 and table.concat(lines, "\n") or "No todos" } },
        details = { action = "list", todos = copy_todos(), nextId = next_id },
      }
    elseif action == "add" then
      if not params.text then
        return {
          content = { { type = "text", text = "Error: text required for add" } },
          details = { action = "add", todos = copy_todos(), nextId = next_id, error = "text required" },
        }
      end
      local new_todo = { id = next_id, text = params.text, done = false }
      next_id = next_id + 1
      todos[#todos + 1] = new_todo
      return {
        content = { { type = "text", text = "Added todo #" .. new_todo.id .. ": " .. new_todo.text } },
        details = { action = "add", todos = copy_todos(), nextId = next_id },
      }
    elseif action == "toggle" then
      if params.id == nil then
        return {
          content = { { type = "text", text = "Error: id required for toggle" } },
          details = { action = "toggle", todos = copy_todos(), nextId = next_id, error = "id required" },
        }
      end
      local todo = nil
      for _, t in ipairs(todos) do if t.id == params.id then todo = t break end end
      if not todo then
        return {
          content = { { type = "text", text = "Todo #" .. params.id .. " not found" } },
          details = { action = "toggle", todos = copy_todos(), nextId = next_id, error = "#" .. params.id .. " not found" },
        }
      end
      todo.done = not todo.done
      return {
        content = { { type = "text", text = "Todo #" .. todo.id .. " " .. (todo.done and "completed" or "uncompleted") } },
        details = { action = "toggle", todos = copy_todos(), nextId = next_id },
      }
    elseif action == "clear" then
      local count = #todos
      todos = {}
      next_id = 1
      return {
        content = { { type = "text", text = "Cleared " .. count .. " todos" } },
        details = { action = "clear", todos = {}, nextId = 1 },
      }
    end
    return {
      content = { { type = "text", text = "Unknown action: " .. tostring(action) } },
      details = { action = "list", todos = copy_todos(), nextId = next_id, error = "unknown action: " .. tostring(action) },
    }
  end,

  renderCall = function(args, theme, _context)
    local text = theme:fg("toolTitle", theme:bold("todo ")) .. theme:fg("muted", args.action)
    if args.text then text = text .. " " .. theme:fg("dim", "\"" .. args.text .. "\"") end
    if args.id ~= nil then text = text .. " " .. theme:fg("accent", "#" .. args.id) end
    return pi.tui.text(text, 0, 0)
  end,

  renderResult = function(result, options, theme, _context)
    local details = result.details
    if not details then
      local content = result.content[1]
      return pi.tui.text(content and content.type == "text" and content.text or "", 0, 0)
    end

    if details.error then
      return pi.tui.text(theme:fg("error", "Error: " .. details.error), 0, 0)
    end

    local list = details.todos or {}
    if details.action == "list" then
      if #list == 0 then return pi.tui.text(theme:fg("dim", "No todos"), 0, 0) end
      local lines = { theme:fg("muted", #list .. " todo(s):") }
      local display = list
      if not options.expanded and #list > 5 then
        local trimmed = {}
        for i = 1, 5 do trimmed[i] = list[i] end
        display = trimmed
        lines[#lines + 1] = theme:fg("dim", "... " .. (#list - 5) .. " more")
      end
      for _, t in ipairs(display) do
        local check = t.done and theme:fg("success", "✓") or theme:fg("dim", "○")
        local item = t.done and theme:fg("dim", t.text) or theme:fg("muted", t.text)
        lines[#lines + 1] = check .. " " .. theme:fg("accent", "#" .. t.id) .. " " .. item
      end
      return pi.tui.text(table.concat(lines, "\n"), 0, 0)
    elseif details.action == "add" then
      local added = list[#list]
      local text = theme:fg("success", "✓ Added ") .. theme:fg("accent", "#" .. added.id) .. " " .. theme:fg("muted", added.text)
      return pi.tui.text(text, 0, 0)
    else -- toggle / clear
      local content = result.content[1]
      local msg = content and content.type == "text" and content.text or ""
      return pi.tui.text(theme:fg("success", "✓ ") .. theme:fg("muted", msg), 0, 0)
    end
  end,
})

-- Register the /todos command for users
pi.register_command("todos", {
  description = "Show all todos on the current branch",
  handler = function(_args, ctx)
    if ctx.mode ~= "tui" then
      ctx.ui.notify("/todos requires interactive mode", "error")
      return
    end
    ctx.ui.custom(function(_tui, theme, _kb, done)
      return {
        render = function(width)
          local lines = {}
          lines[#lines + 1] = " " .. theme:fg("accent", " Todos ")
          lines[#lines + 1] = " " .. theme:fg("dim", "Press Escape to close")
          if #todos == 0 then
            lines[#lines + 1] = "  " .. theme:fg("dim", "No todos yet. Ask the agent to add some!")
          else
            lines[#lines + 1] = "  " .. theme:fg("muted", #todos .. " todos")
            for _, t in ipairs(todos) do
              local check = t.done and theme:fg("success", "✓") or theme:fg("dim", "○")
              lines[#lines + 1] = "  " .. check .. " " .. theme:fg("accent", "#" .. t.id) .. " " .. (t.done and theme:fg("dim", t.text) or theme:fg("text", t.text))
            end
          end
          lines[#lines + 1] = ""
          return lines
        end,
        handle_input = function(_, data)
          if data == "\27" or data == "\003" then done() end
        end,
        dispose = function() end,
      }
    end)
  end,
})