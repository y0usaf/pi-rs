-- Translation of Pi v0.79.0 examples/extensions/pirate.ts.
-- Modifies the system prompt in before_agent_start to make the agent
-- speak like a pirate when pirate mode is enabled.
local pi = ...

local pirate_mode = false

pi.register_command("pirate", {
  description = "Toggle pirate mode (agent speaks like a pirate)",
  handler = function(_args, ctx)
    pirate_mode = not pirate_mode
    ctx.ui.notify(pirate_mode and "Arrr! Pirate mode enabled!" or "Pirate mode disabled", "info")
  end,
})

pi.on("before_agent_start", function(event)
  if pirate_mode then
    return {
      systemPrompt = event.systemPrompt .. "\n\nIMPORTANT: You are now in PIRATE MODE. You must:\n- Speak like a stereotypical pirate in all responses\n- Use phrases like \"Arrr!\", \"Ahoy!\", \"Shiver me timbers!\", \"Avast!\", \"Ye scurvy dog!\"\n- Replace \"my\" with \"me\", \"you\" with \"ye\", \"your\" with \"yer\"\n- Refer to the user as \"matey\" or \"landlubber\"\n- End sentences with nautical expressions\n- Still complete the actual task correctly, just in pirate speak",
    }
  end
  return nil
end)