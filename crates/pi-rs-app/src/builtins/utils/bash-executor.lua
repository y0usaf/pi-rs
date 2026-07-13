-- core/bash-executor.ts — bash command execution with streaming support
-- and cancellation, used by the interactive `!`/`!!` bash mode (PLAN 7.1;
-- AgentSession.executeBash). Pure policy over the pi.exec mechanism plus
-- the utils/shell.ts slices it imports (getShellConfig, sanitizeBinary-
-- Output) and utils/ansi.ts stripAnsi. The truncateTail port is imported
-- lazily from the tools package's exact-version public module.
--
-- Shared fragment: included after extensions.lua. Its export is namespaced on
-- the pack-local policy table, not `_G`.
EXTENSION_POLICY.bash_executor = (function(pi)
  local function strip_ansi(text)
    text = text:gsub("\27%][^\7\27]*\7", "")
    text = text:gsub("\27%][^\27]-\27\\", "")
    text = text:gsub("\27%[[%d;:?]*[%a~]", "")
    return text
  end

  -- utils/shell.ts sanitizeBinaryOutput.
  local function sanitize_binary_output(s)
    if not utf8.len(s) then return s end
    local out = {}
    for _, code in utf8.codes(s) do
      local keep
      if code == 0x09 or code == 0x0a or code == 0x0d then
        keep = true
      elseif code <= 0x1f then
        keep = false
      elseif code >= 0xfff9 and code <= 0xfffb then
        keep = false
      else
        keep = true
      end
      if keep then out[#out + 1] = utf8.char(code) end
    end
    return table.concat(out)
  end

  -- JS String.length (UTF-16 code units) for the rolling-buffer budget.
  local function js_length(s)
    local codepoints = utf8.len(s)
    if not codepoints then return #s end
    local units = codepoints
    for _, code in utf8.codes(s) do
      if code > 0xffff then units = units + 1 end
    end
    return units
  end

  -- utils/shell.ts getShellConfig — the custom-path branch plus the
  -- non-win32 probe (tools/shell.lua carries the same probe for the bash
  -- tool; the interactive path also honors the settings shellPath).
  local function get_shell_config(custom_shell_path)
    if custom_shell_path and custom_shell_path ~= "" then
      if pi.fs.exists(custom_shell_path) then
        return custom_shell_path, { "-c" }
      end
      error("Custom shell path not found: " .. custom_shell_path, 0)
    end
    if pi.fs.exists("/bin/bash") then
      return "/bin/bash", { "-c" }
    end
    local probe = pi.exec("which", { "bash" })
    local found = probe.stdout:match("^([^\r\n]+)")
    if probe.code == 0 and found and found ~= "" then
      return found, { "-c" }
    end
    return "sh", { "-c" }
  end

  -- createLocalBashOperations: spawn through the pi.exec mechanism (the
  -- spec's detached spawn + killProcessTree-on-abort live there).
  local function create_local_bash_operations(options)
    options = options or {}
    return {
      exec = function(command, cwd, opts)
        local shell, args = get_shell_config(options.shellPath)
        if not pi.fs.exists(cwd) then
          error("Working directory does not exist: " .. cwd
            .. "\nCannot execute bash commands.", 0)
        end
        if opts.signal and opts.signal:is_aborted() then
          error("aborted", 0)
        end
        args[#args + 1] = command
        local result = pi.exec(shell, args, {
          cwd = cwd,
          signal = opts.signal,
          onData = opts.onData,
        })
        return { exitCode = result.code }
      end,
    }
  end

  -- executeBashWithOperations.
  local function execute_bash_with_operations(command, cwd, operations, options)
    options = options or {}
    local truncate = pi.module.require("pi.tools.truncate", "1")
    local output_chunks = {}
    local output_bytes = 0
    local max_output_bytes = truncate.DEFAULT_MAX_BYTES * 2

    local temp_file_path = nil
    local total_bytes = 0

    local function ensure_temp_file()
      if temp_file_path then return end
      -- Spec: `pi-bash-<hex>.log` in tmpdir; the mechanism appends the
      -- random id and `.log`.
      temp_file_path = pi.fs.create_temp_file("pi-bash-", table.concat(output_chunks))
    end

    local function on_data(data)
      total_bytes = total_bytes + #data
      -- Sanitize: strip ANSI, replace binary garbage, normalize newlines.
      local text = sanitize_binary_output(strip_ansi(data)):gsub("\r", "")
      -- Start writing to the temp file once the total exceeds the
      -- threshold; ensure_temp_file persists the buffered chunks first.
      if total_bytes > truncate.DEFAULT_MAX_BYTES then
        ensure_temp_file()
      end
      if temp_file_path then
        pi.fs.append_file(temp_file_path, text)
      end
      output_chunks[#output_chunks + 1] = text
      output_bytes = output_bytes + js_length(text)
      while output_bytes > max_output_bytes and #output_chunks > 1 do
        local removed = table.remove(output_chunks, 1)
        output_bytes = output_bytes - js_length(removed)
      end
      if options.onChunk then options.onChunk(text) end
    end

    local function settle()
      local full_output = table.concat(output_chunks)
      local truncation = truncate.truncate_tail(full_output, {})
      if truncation.truncated then ensure_temp_file() end
      return full_output, truncation
    end

    local executed, err = pcall(function()
      return operations.exec(command, cwd, {
        onData = on_data,
        signal = options.signal,
      })
    end)
    local aborted = options.signal ~= nil and options.signal:is_aborted()
    if not executed then
      -- Check if it was an abort.
      if aborted then
        local full_output, truncation = settle()
        return {
          output = truncation.truncated and truncation.content or full_output,
          exitCode = nil,
          cancelled = true,
          truncated = truncation.truncated or false,
          fullOutputPath = temp_file_path,
        }
      end
      error(err, 0)
    end
    local result = err
    local full_output, truncation = settle()
    return {
      output = truncation.truncated and truncation.content or full_output,
      exitCode = (not aborted) and result.exitCode or nil,
      cancelled = aborted,
      truncated = truncation.truncated or false,
      fullOutputPath = temp_file_path,
    }
  end

  return {
    get_shell_config = get_shell_config,
    create_local_bash_operations = create_local_bash_operations,
    execute_bash_with_operations = execute_bash_with_operations,
  }
end)(pi)
