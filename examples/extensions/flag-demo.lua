-- Extension CLI flag registry demo (translation of Pi's registerFlag/getFlag
-- surface). The product CLI applies parsed values in PLAN 9.4.
local pi = ...

pi.register_flag("demo-enabled", {
  description = "Enable the flag demo",
  type = "boolean",
  default = false,
})

pi.register_flag("demo-label", {
  description = "Set the flag demo label",
  type = "string",
  default = "default",
})

pi.register_command("flag-demo", {
  description = "Show the flag demo values",
  handler = function()
    return {
      enabled = pi.get_flag("demo-enabled"),
      label = pi.get_flag("demo-label"),
      unregistered = pi.get_flag("not-registered"),
    }
  end,
})
