-- Translation of Pi v0.79.0 examples/extensions/notify.ts.
-- Sends a native terminal notification when Pi is done and waiting for input.
-- Supports multiple terminal protocols: OSC 777 / OSC 99 / Windows toast.
--
-- Pi writes these OSC sequences with `process.stdout.write`; in pi-rs non-
-- interactive stdout is protocol-clean, so the escape sequences are emitted
-- through `io.write` (routed to stderr), which keeps the translation
-- compilable and faithful to the notify intent.
local pi = ...

local function windows_toast_script(title, body)
  local type = "Windows.UI.Notifications"
  local mgr = "[" .. type .. ".ToastNotificationManager, " .. type .. ", ContentType = WindowsRuntime]"
  local template = "[" .. type .. ".ToastTemplateType]::ToastText01"
  local toast = "[" .. type .. ".ToastNotification]::new($xml)"
  return table.concat({
    mgr .. " > $null",
    "$xml = [" .. type .. ".ToastNotificationManager]::GetTemplateContent(" .. template .. ")",
    "$xml.GetElementsByTagName('text')[0].AppendChild($xml.CreateTextNode('" .. body .. "')) > $null",
    "[" .. type .. ".ToastNotificationManager]::CreateToastNotifier('" .. title .. "').Show(" .. toast .. ")",
  }, "; ")
end

local function notify_osc777(title, body)
  io.write("\27]777;notify;" .. title .. ";" .. body .. "\07")
end

local function notify_osc99(title, body)
  io.write("\27]99;i=1:d=0;" .. title .. "\27\\")
  io.write("\27]99;i=1:p=body;" .. body .. "\27\\")
end

local function notify_windows(title, body)
  pi.exec("powershell.exe", { "-NoProfile", "-Command", windows_toast_script(title, body) })
end

local function notify(title, body)
  if pi.env("WT_SESSION") then
    notify_windows(title, body)
  elseif pi.env("KITTY_WINDOW_ID") then
    notify_osc99(title, body)
  else
    notify_osc777(title, body)
  end
end

pi.on("agent_end", function()
  notify("Pi", "Ready for input")
end)