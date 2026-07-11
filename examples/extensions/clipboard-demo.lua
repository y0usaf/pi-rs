-- Exerciser for the clipboard-image mechanism: pi.clipboard.read_image,
-- pi.clipboard.extension_for_mime_type, pi.random_uuid, pi.fs.tmpdir.
--
-- In pi these are utils/clipboard-image.ts `readClipboardImage` (wl-paste
-- / xclip / WSL PowerShell probing with format preference and PNG
-- conversion for unsupported formats), `extensionForImageMimeType`, and
-- Node's crypto.randomUUID / os.tmpdir — the pieces
-- interactive-mode.ts `handleClipboardImagePaste` composes into a temp
-- file whose path lands in the editor. The read accepts the spec's test
-- seam ({ env, platform }) so behavior is scriptable.
local pi = ...

pi.register_command("clipboard-demo", {
  description = "Walk the clipboard-image mechanism",
  handler = function()
    -- Termux has no clipboard tools: the spec returns null immediately.
    local termux = pi.clipboard.read_image({
      env = { TERMUX_VERSION = "0.118" }, platform = "linux",
    })

    -- Plain X11 Linux without the native addon loaded: no probe fires.
    local no_session = pi.clipboard.read_image({
      env = {}, platform = "linux",
    })

    -- A Wayland session probes wl-paste (then xclip). Whether an image
    -- comes back depends on the machine's clipboard, exactly like pi —
    -- the demo only exercises the probe.
    local wayland = pi.clipboard.read_image({
      env = { WAYLAND_DISPLAY = "wayland-1" }, platform = "linux",
    })

    -- handleClipboardImagePaste's temp-path composition.
    local ext = pi.clipboard.extension_for_mime_type("image/jpeg;charset=x") or "png"
    local temp_path = pi.path.join(pi.fs.tmpdir(), "pi-clipboard-" .. pi.random_uuid() .. "." .. ext)

    return {
      termux_was_nil = termux == nil,
      no_session_was_nil = no_session == nil,
      wayland_kind = wayland == nil and "nil" or "image",
      ext = ext,
      unsupported_ext_was_nil = pi.clipboard.extension_for_mime_type("image/bmp") == nil,
      temp_path = temp_path,
    }
  end,
})
