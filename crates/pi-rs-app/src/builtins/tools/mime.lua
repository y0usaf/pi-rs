-- utils/mime.ts — supported-image MIME detection from file magic (never
-- from the extension). Offsets stay 0-indexed like the spec; the byte
-- accessors convert.
local IMAGE_TYPE_SNIFF_BYTES = 4100

local function read_u32_be(buf, offset)
  local b1, b2, b3, b4 = buf:byte(offset + 1, offset + 4)
  return (b1 or 0) * 0x1000000 + (b2 or 0) * 0x10000 + (b3 or 0) * 0x100 + (b4 or 0)
end

local function starts_with_ascii(buf, offset, text)
  if #buf < offset + #text then
    return false
  end
  return buf:sub(offset + 1, offset + #text) == text
end

local PNG_SIGNATURE = "\137PNG\r\n\26\10"

local function is_png(buf)
  return #buf >= 16 and read_u32_be(buf, #PNG_SIGNATURE) == 13 and starts_with_ascii(buf, 12, "IHDR")
end

local function is_animated_png(buf)
  local offset = #PNG_SIGNATURE
  while offset + 8 <= #buf do
    local chunk_length = read_u32_be(buf, offset)
    local chunk_type_offset = offset + 4
    if starts_with_ascii(buf, chunk_type_offset, "acTL") then
      return true
    end
    if starts_with_ascii(buf, chunk_type_offset, "IDAT") then
      return false
    end
    local next_offset = offset + 8 + chunk_length + 4
    if next_offset <= offset or next_offset > #buf then
      return false
    end
    offset = next_offset
  end
  return false
end

local function detect_supported_image_mime_type(buf)
  local b1, b2, b3, b4 = buf:byte(1, 4)
  if b1 == 0xFF and b2 == 0xD8 and b3 == 0xFF then
    if b4 == 0xF7 then
      return nil
    end
    return "image/jpeg"
  end
  if buf:sub(1, #PNG_SIGNATURE) == PNG_SIGNATURE then
    if is_png(buf) and not is_animated_png(buf) then
      return "image/png"
    end
    return nil
  end
  if starts_with_ascii(buf, 0, "GIF") then
    return "image/gif"
  end
  if starts_with_ascii(buf, 0, "RIFF") and starts_with_ascii(buf, 8, "WEBP") then
    return "image/webp"
  end
  return nil
end
