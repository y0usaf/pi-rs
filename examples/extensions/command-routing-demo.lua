-- PLAN 9.10 command-routing: per-command suppression + first-wins replacement
-- from an ordinary file-backed package, using the public `pi.commands` module
-- (the same registry `handle_submit` dispatches through). This file suppresses
-- the builtin `/model` route and registers a file-backed replacement that a
-- trace harness (assembly.rs) drives through the real `handle_submit` path.
local pi = ...

pi.declare_package({ command_visibility = "public" })

local commands = pi.module.require("pi.commands", "1")

-- Return the ordered route table before any mutation.
local before = commands.routes()

-- Suppress only the builtin `/model` command (per-command ablation); the rest
-- of the frontend stays active.
local disabled = commands.disable("model")

-- Register a file-backed replacement first-wins now that the builtin is gone.
commands.register("model", {
  description = "file-backed model",
  run = function(actions, arg)
    actions.set_text("")
    actions.model_command(arg == nil and "replacement-default" or ("replacement:" .. arg))
  end,
})

local after = commands.routes()

pi.register_command("command-routing-demo", {
  description = "Exercise pi.commands per-command suppression + replacement",
  handler = function(args)
    local request = pi.json.decode(args)
    local result = {}
    if request.texts then
      for _, text in ipairs(request.texts) do
        local actions = {
          set_text = function(v) table.insert(result, { action = "set_text", value = v }) end,
          model_command = function(search)
            table.insert(result, { action = "model_command", value = search })
          end,
          settings_command = function()
            table.insert(result, { action = "settings_command" })
          end,
          show_oauth_selector = function(_m)
            table.insert(result, { action = "show_oauth_selector" })
          end,
          prompt = function(v) table.insert(result, { action = "prompt", value = v }) end,
        }
        commands.dispatch(text, actions)
      end
    end
    local names = {}
    for _, r in ipairs(after) do names[#names + 1] = r.name end
    return {
      disabled = disabled,
      modelStillRegistered = commands.routes(),
      modelReplaced = after[#after] ~= nil and after[#after].name == "model",
      modelRouteSourceDescription = "file-backed model",
      beforeNames = before,
      afterNames = names,
      trace = result,
    }
  end,
})
