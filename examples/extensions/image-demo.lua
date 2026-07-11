-- Exerciser for the photon image bindings: pi.image.resize and
-- pi.image.convert_to_png.
--
-- In pi these are utils/image-resize-core.ts `resizeImageInProcess`
-- (the read tool's auto-resize: 2000x2000 max, 4.5MB base64 cap,
-- PNG-vs-JPEG candidates, quality steps, 0.75 dimension backoff) and
-- utils/image-convert.ts `convertToPng` (kitty-graphics PNG
-- normalization), both running @silvia-odwyer/photon-node 0.3.4. pi-rs
-- ports the same library slice as a binding, byte-for-byte
-- (tests/image-parity):
--   resizeImage(bytes, mime, opts)  → pi.image.resize(bytes, mime, opts)
--     → { data (base64), mimeType, originalWidth, originalHeight,
--         width, height, wasResized } | nil
--   convertToPng(base64, mime)      → pi.image.convert_to_png(base64, mime)
--     → { data (base64), mimeType } | nil
local pi = ...

-- A 4x2 red PNG (raw bytes via base64) — small enough to stay a
-- passthrough at default limits, big enough to shrink with maxWidth=2.
local TINY_PNG_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAQAAAACCAYAAAB/qH1jAAAAEklEQVR4nGP4z8DwHxkzoAsAAA8hD/EEN8afAAAAAElFTkSuQmCC"

local B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local REV = {}
for i = 1, #B64 do REV[B64:sub(i, i)] = i - 1 end
local function base64_decode(data)
  local out, bits, count = {}, 0, 0
  for i = 1, #data do
    local v = REV[data:sub(i, i)]
    if v ~= nil then
      bits = (bits << 6) | v
      count = count + 6
      if count >= 8 then
        count = count - 8
        out[#out + 1] = string.char((bits >> count) & 0xFF)
      end
    end
  end
  return table.concat(out)
end

pi.register_command("image-demo", {
  description = "Walk the photon resize/convert bindings",
  handler = function()
    local bytes = base64_decode(TINY_PNG_BASE64)

    -- Within all limits: the original bytes pass through untouched.
    local passthrough = pi.image.resize(bytes, "image/png")

    -- Over the dimension limit: Lanczos3 resize, PNG-vs-JPEG candidates.
    local resized = pi.image.resize(bytes, "image/png", { maxWidth = 2, maxHeight = 2 })

    -- Impossible byte budget: the spec returns null.
    local impossible = pi.image.resize(bytes, "image/png", { maxBytes = 1 })

    -- PNG input short-circuits; the mime tag comes back unchanged.
    local converted = pi.image.convert_to_png(TINY_PNG_BASE64, "image/png")

    return {
      passthrough = passthrough,
      resized = resized,
      impossible_was_nil = impossible == nil,
      converted = converted,
    }
  end,
})
