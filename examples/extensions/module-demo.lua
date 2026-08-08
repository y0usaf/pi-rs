-- File-backed consumer of the same exact-version helpers used by builtins.
-- Load after the builtin packages.
local pi = ...

local truncate = pi.module.require("pi.tools.truncate", "1")
local render = pi.module.require("pi.tools.render", "1")
-- The shared policy fragments the coding-agent and interactive packs use.
local messages = pi.module.require("pi.utils.messages", "1")
local branch_summary = pi.module.require("pi.utils.branch-summary", "1")
local system_prompt_mod = pi.module.require("pi.utils.system-prompt", "1")
local agent_session = pi.module.require("pi.utils.agent-session", "1")
local extensions = pi.module.require("pi.utils.extensions", "1")
local bash_executor = pi.module.require("pi.utils.bash-executor", "1")
local export_html = pi.module.require("pi.utils.export-html", "1")

pi.register_command("module-demo", {
  description = "Exercise public builtin Lua modules",
  handler = function()
    local result = truncate.truncate_head("alpha\nbeta\ngamma", { maxLines = 2 })
    local converted = messages.convert_to_llm({
      { role = "user", content = "hello" },
      { role = "bashExecution", command = "ls", output = "a", exitCode = 0 },
    })
    local estimated = branch_summary.estimate_tokens({ role = "user", content = "hello world" })
    local snippet = system_prompt_mod.normalize_prompt_snippet("  <description>demo</description>  ")
    local session_exports = pi.module.list()
    local present = {}
    for _, m in ipairs(session_exports) do present[m.name .. "@" .. m.version] = true end
    local expected = {
      "pi.tools.prelude@1", "pi.tools.truncate@1", "pi.tools.path-utils@1",
      "pi.tools.mime@1", "pi.tools.shell@1", "pi.tools.output-accumulator@1",
      "pi.tools.keybinding-hints@1", "pi.tui.visual-truncate@1", "pi.tools.render@1",
      "pi.tools.diff@1", "pi.tools.edit-diff@1", "pi.utils.syntax-highlight@1",
      "pi.utils.messages@1", "pi.utils.extensions@1", "pi.utils.branch-summary@1",
      "pi.utils.system-prompt@1", "pi.utils.agent-session@1", "pi.utils.bash-executor@1",
      "pi.utils.export-html@1", "pi.tools.file-mutation-queue@1",
    }
    local all_modules_present = true
    for _, key in ipairs(expected) do
      if not present[key] then all_modules_present = false end
    end
    local no_ui_confirm = extensions.headless_ui.confirm()
    local shell_path = bash_executor.get_shell_config()
    local encoded = export_html.base64_encode("demo")
    return {
      content = result.content,
      path = render.shorten_path((pi.env.HOME or "") .. "/demo.txt"),
      truncated = result.truncated,
      convertedRoles = #converted,
      bashRole = converted[2] and converted[2].role or nil,
      estimatedTokens = estimated,
      normalizedSnippet = snippet,
      allModulesPresent = all_modules_present,
      noUiConfirm = no_ui_confirm,
      shell = shell_path,
      base64Demo = encoded,
    }
  end,
})
