-- File-backed consumer of the same public export-html module the builtin
-- interactive frontend uses. Load after the interactive package. The
-- builtin defines `pi.interactive.export-html`; this on-disk package imports
-- it through the same non-privileged `pi.module.require` mechanism with no
-- hidden native modules, load-order globals, or JS runtime.
local pi = ...

local export_html = pi.module.require("pi.interactive.export-html", "1")

pi.register_command("export-html-consumer", {
  description = "Exercise pi.interactive.export-html from a file-backed package",
  handler = function()
    -- base64_encode is a pure, deterministic helper exported by the public
    -- module; pre_render_tools over an empty entry list exercises the same
    -- closures the builtin exporter funnels through. Both prove a file-backed
    -- package resolves the identical exact-version module as the builtins.
    local encoded = export_html.base64_encode("hello\n\npi export")
    local rendered = export_html.pre_render_tools({}, { fg = function() end }, pi.cwd())
    return {
      base64 = encoded,
      rendered = rendered,
      resolved = export_html.generate ~= nil,
    }
  end,
})