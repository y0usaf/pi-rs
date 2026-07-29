-- Key decoding for the shipped frontend.
--
-- Rust hands Lua bounded terminal batches (`pi.terminal.v1.input_buffer`),
-- never one callback per byte. Turning those batches into named keys is
-- product policy: the kernel has no notion of "submit", "interrupt", or
-- "newline", and a replacement frontend may decode differently.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.frontend.keys",
  version = "1",
  factory = function()
    local DEFAULT_MAX_KEYS = 512

    -- Final characters of the CSI sequences this frontend understands.
    local CSI_KEYS = {
      A = "up",
      B = "down",
      C = "right",
      D = "left",
      H = "home",
      F = "end",
    }

    local CONTROL_KEYS = {
      [3] = "interrupt", -- ctrl-c
      [4] = "eof", -- ctrl-d
      [8] = "backspace",
      [10] = "submit",
      [13] = "submit",
      [20] = "toggle_thinking", -- ctrl-t
      [21] = "clear_line", -- ctrl-u
      [127] = "backspace",
    }

    local function is_csi_final(byte)
      return byte >= 0x40 and byte <= 0x7e
    end

    -- Decode one data chunk in place. Printable bytes are consumed as whole
    -- runs, so a paste-sized chunk produces one text key rather than one key
    -- per character.
    local function decode_chunk(text, keys, limit)
      local index = 1
      local length = #text
      while index <= length and #keys < limit do
        local byte = text:byte(index)
        if byte == 27 then
          local following = text:sub(index + 1, index + 1)
          if following == "[" or following == "O" then
            local final = text:sub(index + 2, index + 2)
            local named = CSI_KEYS[final]
            if named then
              keys[#keys + 1] = { kind = named }
              index = index + 3
            else
              local scan = index + 2
              while scan <= length and not is_csi_final(text:byte(scan)) do
                scan = scan + 1
              end
              keys[#keys + 1] = { kind = "unknown" }
              index = scan + 1
            end
          elseif following == "\r" or following == "\n" then
            -- alt+enter inserts a line instead of submitting.
            keys[#keys + 1] = { kind = "newline" }
            index = index + 2
          else
            keys[#keys + 1] = { kind = "escape" }
            index = index + 1
          end
        elseif CONTROL_KEYS[byte] then
          keys[#keys + 1] = { kind = CONTROL_KEYS[byte] }
          index = index + 1
        elseif byte < 32 then
          index = index + 1
        else
          local scan = index
          while scan <= length do
            local current = text:byte(scan)
            if current < 32 or current == 127 then
              break
            end
            scan = scan + 1
          end
          keys[#keys + 1] = { kind = "text", text = text:sub(index, scan - 1) }
          index = scan
        end
      end
      return keys
    end

    -- `events` is the batch returned by a terminal input buffer.
    local function decode(events, limit)
      local bound = tonumber(limit) or DEFAULT_MAX_KEYS
      local keys = {}
      if type(events) ~= "table" then
        return keys
      end
      for _, event in ipairs(events) do
        if type(event) == "table" and type(event.data) == "string" then
          if event.kind == "paste" then
            if #keys < bound then
              keys[#keys + 1] = { kind = "text", text = event.data, pasted = true }
            end
          else
            decode_chunk(event.data, keys, bound)
          end
        end
      end
      return keys
    end

    return {
      decode = decode,
      max_keys = DEFAULT_MAX_KEYS,
    }
  end,
})
