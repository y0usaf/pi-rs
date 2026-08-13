-- Translation of Pi v0.79.0 examples/extensions/working-message-test.ts.
-- Sets a custom working message and indicator on session start so they
-- survive across loader recreations (between agent turns).
local pi = ...

local CUSTOM_MESSAGE = "\27[38;2;155;86;63mWorking... (custom)\27[39m"
local CUSTOM_INDICATOR = { frames = { "\27[38;2;155;86;63m●\27[39m" } }

pi.on("session_start", function(_event, ctx)
  ctx.ui.setWorkingMessage(CUSTOM_MESSAGE)
  ctx.ui.setWorkingIndicator(CUSTOM_INDICATOR)
end)