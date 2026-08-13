-- runtime-actions-demo — exercises the ExtensionAPI runtime action/view
-- methods bound onto `pi` per live session (spec runner.ts bindCoreActions ->
-- agent-session.ts). These mirror the translated Pi examples (tools.ts,
-- preset.ts, session-name.ts, message-renderer.ts, send-user-message.ts):
--
--   pi.getActiveTools() / pi.getAllTools() / pi.setActiveTools(names)
--   pi.getSessionName() / pi.setSessionName(name)
--   pi.getThinkingLevel() / pi.setThinkingLevel(level)
--   pi.sendMessage(msg) / pi.sendUserMessage(content)
--   pi.appendEntry(type, data) / pi.setLabel(entryId, label)
--   pi.setModel(model) / pi.refreshTools()
--
-- All reads return immutable snapshots; all mutations are queued actions
-- applied by the session loop, so an extension can call them from an event
-- or command handler without touching mutable host state.
local pi = ...

-- Register a helper tool after startup to confirm refreshTools() surfaces it.
pi.register_command("bind-actions", {
	description = "Snapshot the bound runtime-action API methods",
	handler = function()
		local ok_active, active = pcall(pi.getActiveTools)
		local ok_all, all = pcall(pi.getAllTools)
		local ok_name, name = pcall(pi.getSessionName)
		local ok_think, think = pcall(pi.getThinkingLevel)
		-- Dynamic tool: register a tool at runtime, refresh the registry, and
		-- confirm it participates in the tool inventory (Pi dynamic-tools.ts).
		local dyn = "echo_session"
		pi.register_tool({
			name = dyn, label = "Echo Session", description = "Echo",
			parameters = { type = "object", properties = {}, required = {} },
			execute = function() return { content = { { type = "text", text = "echo" } }, details = {} } end,
		})
		pi.refreshTools()
		local ok_all2, all2 = pcall(pi.getAllTools)
		local has_dyn = false
		if ok_all2 then
			for _, t in ipairs(all2 or {}) do if t.name == dyn then has_dyn = true break end end
		end
		return {
			ok_active = ok_active, active = active,
			ok_all = ok_all, all_names = all and #all or 0,
			contains_runtime_tool = has_dyn,
			ok_name = ok_name, name = name,
			sendMessage = type(pi.sendMessage),
			sendUserMessage = type(pi.sendUserMessage),
			appendEntry = type(pi.appendEntry),
			setSessionName = type(pi.setSessionName),
			setLabel = type(pi.setLabel),
			getActiveTools = type(pi.getActiveTools),
			getAllTools = type(pi.getAllTools),
			setActiveTools = type(pi.setActiveTools),
			setModel = type(pi.setModel),
			getThinkingLevel = type(pi.getThinkingLevel),
			setThinkingLevel = type(pi.setThinkingLevel),
			refreshTools = type(pi.refreshTools),
			ok_think = ok_think, think = think,
		}
	end,
})