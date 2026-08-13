-- utils/shell.ts slice. Shell policy stays in Lua; pi.exec is only the
-- non-shell streaming subprocess mechanism.
--
-- Public exact-version module: builtin and file-backed packages resolve the
-- same shell policy. `shell_config` closes over the host `pi.exec`/`pi.fs`
-- mechanisms and is exported so ordinary tools/packages never re-implement
-- the binary probe.
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

pi.module.define({
  name = "pi.tools.shell",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      shell_config = shell_config,
    }
  end,
})
