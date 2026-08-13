-- Translation of Pi v0.79.0 examples/extensions/tool-override.ts.
-- Overrides the built-in `read` tool to log access and block sensitive
-- paths, delegating execution to the original implementation.
local pi = ...

-- Pi resolves the log path under getAgentDir(); pi-rs has no public agent-dir
-- binding on `pi`, so this translation writes under cwd/.pi (the project
-- agent root), which keeps the documented audit intent.
local LOG_FILE = pi.path.join(pi.cwd(), ".pi", "read-access.log")
local mutation_queue = pi.module.require("pi.tools.file-mutation-queue", "1")

local BLOCKED_PATTERNS = {
  "%.env$",
  "%.env%..+$",
  "[Ss]ecrets?%.(json|yaml|yml|toml)$",
  "[Cc]redentials?%.(json|yaml|yml|toml)$",
  "/%.ssh/",
  "/%.aws/",
  "/%.gnupg/",
}

local function is_blocked_path(path)
  for _, pattern in ipairs(BLOCKED_PATTERNS) do
    if path:find(pattern) then return true end
  end
  return false
end

local function log_access(path, allowed, reason)
  local timestamp = os.date("!%Y-%m-%dT%H:%M:%SZ")
  local status = allowed and "ALLOWED" or "BLOCKED"
  local msg = reason and (" (" .. reason .. ")") or ""
  local line = "[" .. timestamp .. "] " .. status .. ": " .. path .. msg .. "\n"

  local ok = pcall(function()
    mutation_queue.with_file_mutation_queue(LOG_FILE, function()
      pi.fs.append_file(LOG_FILE, line)
    end)
  end)
  -- Ignore logging errors
end

-- Find the original read tool definition to delegate execution.
local original_read = nil
for _, definition in ipairs(pi.registered_tools()) do
  if definition.name == "read" then original_read = definition break end
end

pi.register_tool({
  name = "read",
  label = "read (audited)",
  description = "Read the contents of a file with access logging. Some sensitive paths (.env, secrets, credentials) are blocked.",
  parameters = original_read and original_read.parameters or {
    type = "object",
    properties = { path = { type = "string", description = "Path to the file to read" } },
    required = { "path" },
  },

  execute = function(tool_call_id, params, signal, on_update, ctx)
    local absolute_path = pi.path.resolve(params.path, ctx.cwd or pi.cwd())

    if is_blocked_path(absolute_path) then
      log_access(absolute_path, false, "matches blocked pattern")
      return {
        content = { { type = "text", text = 'Access denied: "' .. params.path .. '" matches a blocked pattern (sensitive file). This tool blocks access to .env files, secrets, credentials, and SSH/AWS/GPG directories.' } },
        details = { blocked = true },
      }
    end

    log_access(absolute_path, true)

    if original_read and original_read.execute then
      -- Delegate to the original implementation
      local ok, result = pcall(original_read.execute, tool_call_id, params, signal, on_update, ctx)
      if ok then return result end
      return {
        content = { { type = "text", text = "Error reading file: " .. tostring(result) } },
        details = { error = true },
      }
    end

    -- Fallback: direct read when the original is unavailable
    local ok, content = pcall(pi.fs.read_file, absolute_path)
    if ok then
      local lines_count = select(2, content:gsub("\n", "")) + 1
      return { content = { { type = "text", text = content } }, details = { lines = lines_count } }
    end
    return { content = { { type = "text", text = "Error reading file: " .. tostring(content) } }, details = { error = true } }
  end,
})

-- Register a command to view the access log
pi.register_command("read-log", {
  description = "View the file access log",
  handler = function(_args, ctx)
    local ok, log = pcall(pi.fs.read_file, LOG_FILE)
    if not ok then
      ctx.ui.notify("No access log found", "info")
      return
    end
    local lines = {}
    for line in log:gmatch("[^\n]+") do lines[#lines + 1] = line end
    local recent = {}
    for i = math.max(1, #lines - 19), #lines do recent[#recent + 1] = lines[i] end
    ctx.ui.notify("Recent file access:\n" .. table.concat(recent, "\n"), "info")
  end,
})