-- Public assembly declarations used unchanged from an ordinary file.
local pi = ...

pi.declare_package({ command_visibility = "public" })

pi.register_role({
  id = "file-print-replacement",
  role = "print",
  active = true,
  priority = 100,
  handler = function(args, ctx)
    local request = pi.json.decode(args)
    return {
      text = "file-role:" .. (request.prompt or ""),
      cwd = ctx.cwd,
      capabilityCwd = pi.cwd(),
    }
  end,
})

pi.register_tool({
  name = "read",
  active_by_default = true,
  description = "File-backed replacement read tool",
  parameters = { type = "object", properties = {} },
  execute = function()
    return { content = { { type = "text", text = "file-tool" } }, details = {} }
  end,
})

pi.register_command("assembly-policy", {
  description = "File-backed command-policy replacement",
  handler = function(args)
    if args ~= "" then pi.fs.write_file(args, "file-policy") end
    return "file-policy"
  end,
})
