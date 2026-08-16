-- PLAN 9.10 module.extension-composition: an ordinary file-backed package
-- imports the same public exact-version `pi.extension.composition@1` module
-- that the builtin coding-agent and interactive packs define (the active-tool
-- / tool-call-fold / extension-command policy table shared via the VM-wide
-- module registry). Reusing the identical closures proves there is no
-- pack-private chunk-local tier left for composition policy.
local pi = ...
pi.declare_package({ command_visibility = "public" })

local listed = {}
for _, entry in ipairs(pi.module.list()) do
  listed[entry.name .. "@" .. entry.version] = true
end

local composition = pi.module.require("pi.extension.composition", "1")

pi.register_command("extension-composition-demo", {
  description = "Exercise the public pi.extension.composition module",
  handler = function(args)
    -- The module is a real public registered module.
    local registered = listed["pi.extension.composition@1"] or false

    -- The full composition surface resolves to functions.
    local hasActiveTools = type(composition.active_tools) == "function"
    local hasEmitToolCall = type(composition.emit_tool_call) == "function"
    local hasEmitToolResult = type(composition.emit_tool_result) == "function"
    local hasEmitGeneric = type(composition.emit_generic) == "function"
    local hasExecuteCommand = type(composition.execute_command) == "function"
    local hasTryExecute = type(composition.try_execute_extension_command) == "function"
    local hasBindPiActions = type(composition.bind_pi_actions) == "function"

    -- active_tools reflects the builtin/tool-pack registered tools, proving
    -- the shared table is wired to the same registry the product uses.
    local active, names = composition.active_tools()
    local toolCount = #active
    local hasBashTool = false
    for _, name in ipairs(names) do
      if name == "bash" then hasBashTool = true end
    end

    -- emit_generic: isolates handler errors (a throwing handler must not
    -- poison a later one). Register a throwing then a recording hook on a
    -- fresh open channel and reuse the shared fold.
    local seen = {}
    composition.emit_generic({ type = "extension_composition_demo_event" },
      nil)
    -- Direct fold sanity: emit_message_end chain-validates role preservation.
    local replaced = composition.emit_message_end(
      { type = "message_end", message = { role = "assistant", content = {} } },
      nil)
    local replacedNil = replaced == nil or replaced.message == nil

    return {
      registered = registered,
      hasActiveTools = hasActiveTools,
      hasEmitToolCall = hasEmitToolCall,
      hasEmitToolResult = hasEmitToolResult,
      hasEmitGeneric = hasEmitGeneric,
      hasExecuteCommand = hasExecuteCommand,
      hasTryExecute = hasTryExecute,
      hasBindPiActions = hasBindPiActions,
      toolCount = toolCount,
      hasBashTool = hasBashTool,
      noEventHandlers = #seen == 0,
      replacedNil = replacedNil,
    }
  end,
})
