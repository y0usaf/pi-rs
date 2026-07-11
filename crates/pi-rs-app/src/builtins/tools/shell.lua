-- utils/shell.ts slice. Shell policy stays in Lua; pi.exec is only the
-- non-shell streaming subprocess mechanism.
local function shell_config()
  if pi.fs.exists("/bin/bash") then
    return "/bin/bash", { "-c" }
  end
  local probe = pi.exec("which", { "bash" })
  local found = probe.stdout:match("^([^\r\n]+)")
  if probe.code == 0 and found and found ~= "" then
    return found, { "-c" }
  end
  return "sh", { "-c" }
end
