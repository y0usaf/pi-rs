-- core/messages.ts — the coding agent's custom message roles as the LLM
-- context sees them: bashExecutionToText and convertToLlm — plus the
-- sdk.ts convertToLlmWithBlockImages defense-in-depth image filter the
-- product packs pass as the agent's convertToLlm option.
--
-- Shared fragment: included by the interactive pack and the one-shot
-- coding-agent pack, so it only assumes the chunk argument.
local pi = ...

-- JS `${number}`: integral values print without a fraction.
local function msg_num(value)
  if math.type(value) == "float" and value % 1 == 0
    and value == value and value ~= math.huge and value ~= -math.huge then
    return ("%d"):format(value)
  end
  return tostring(value)
end

-- messages.ts bashExecutionToText.
local function bash_execution_to_text(msg)
  local text = "Ran `" .. msg.command .. "`\n"
  if msg.output ~= nil and msg.output ~= "" then
    text = text .. "```\n" .. msg.output .. "\n```"
  else
    text = text .. "(no output)"
  end
  if msg.cancelled then
    text = text .. "\n\n(command cancelled)"
  elseif msg.exitCode ~= nil and msg.exitCode ~= 0 then
    text = text .. "\n\nCommand exited with code " .. msg_num(msg.exitCode)
  end
  if msg.truncated and msg.fullOutputPath then
    text = text .. "\n\n[Output truncated. Full output: " .. msg.fullOutputPath .. "]"
  end
  return text
end

-- messages.ts convertToLlm.
local function convert_to_llm(messages)
  local result = {}
  for _, m in ipairs(messages) do
    if m.role == "bashExecution" then
      -- Skip messages excluded from context (!! prefix).
      if not m.excludeFromContext then
        result[#result + 1] = {
          role = "user",
          content = { { type = "text", text = bash_execution_to_text(m) } },
          timestamp = m.timestamp,
        }
      end
    elseif m.role == "custom" then
      local content = m.content
      if type(content) == "string" then
        content = { { type = "text", text = content } }
      end
      result[#result + 1] = { role = "user", content = content, timestamp = m.timestamp }
    elseif m.role == "branchSummary" then
      result[#result + 1] = {
        role = "user",
        content = { { type = "text", text = "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n" .. m.summary .. "</summary>" } },
        timestamp = m.timestamp,
      }
    elseif m.role == "compactionSummary" then
      result[#result + 1] = {
        role = "user",
        content = { { type = "text", text = "The conversation history before this point was compacted into the following summary:\n\n<summary>\n" .. m.summary .. "\n</summary>" } },
        timestamp = m.timestamp,
      }
    elseif m.role == "user" or m.role == "assistant" or m.role == "toolResult" then
      result[#result + 1] = m
    end
  end
  return result
end

-- sdk.ts convertToLlmWithBlockImages: filter ImageContent out of user /
-- toolResult messages when the blockImages setting is on. The setting is
-- read per call (spec: "Check setting dynamically so mid-session changes
-- take effect"); consecutive placeholder texts dedupe against the
-- *mapped* array, exactly like the spec's filter over `arr`.
local BLOCK_IMAGES_TEXT = "Image reading is disabled."

local function convert_to_llm_with_block_images(messages)
  local converted = convert_to_llm(messages)
  if not pi.settings.block_images() then
    return converted
  end
  local result = {}
  for i, msg in ipairs(converted) do
    local out = msg
    if (msg.role == "user" or msg.role == "toolResult") and type(msg.content) == "table" then
      local has_images = false
      for _, block in ipairs(msg.content) do
        if block.type == "image" then has_images = true end
      end
      if has_images then
        local mapped = {}
        for j, block in ipairs(msg.content) do
          if block.type == "image" then
            mapped[j] = { type = "text", text = BLOCK_IMAGES_TEXT }
          else
            mapped[j] = block
          end
        end
        local filtered = {}
        for j, block in ipairs(mapped) do
          local dedupe = block.type == "text" and block.text == BLOCK_IMAGES_TEXT and j > 1
            and mapped[j - 1].type == "text" and mapped[j - 1].text == BLOCK_IMAGES_TEXT
          if not dedupe then filtered[#filtered + 1] = block end
        end
        out = {}
        for key, value in pairs(msg) do out[key] = value end
        out.content = filtered
      end
    end
    result[i] = out
  end
  return result
end
