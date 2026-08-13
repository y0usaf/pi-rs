-- File-backed consumer of the shared agent-policy exact-version modules
-- defined by the builtin agent-core pack. Load after the builtin packages.
-- These public modules (pi.agent.messages / pi.agent.branch-summary /
-- pi.agent.compaction / pi.agent.system-prompt / pi.agent.session-runtime /
-- pi.agent.bash-executor) are resolved through the same non-privileged
-- pi.module.require mechanism the builtin interactive and coding-agent packs
-- use — no hidden native modules, load-order globals, or JS runtime. A
-- file-backed replacement may re-define the same exact-version name only when
-- the builtin definition is not loaded; here we exercise the shared closures.
local pi = ...

local messages = pi.module.require("pi.agent.messages", "1")
local branch_summary = pi.module.require("pi.agent.branch-summary", "1")
local compaction = pi.module.require("pi.agent.compaction", "1")
local system_prompt = pi.module.require("pi.agent.system-prompt", "1")
local session_runtime = pi.module.require("pi.agent.session-runtime", "1")
local bash_executor = pi.module.require("pi.agent.bash-executor", "1")

pi.register_command("agent-core-module-demo", {
  description = "Exercise the shared agent-policy modules from a file-backed package",
  handler = function()
    -- messages: bashExecution -> llm text.
    local to_llm = messages.convert_to_llm({
      {
        role = "bashExecution",
        command = "ls",
        output = "a.txt",
        exitCode = 0,
        timestamp = 1,
      },
    })
    -- branch-summary token estimation (shared with compaction).
    local estimated = branch_summary.estimate_tokens({ role = "user", content = "abcd" })
    -- compaction default settings / overflow check.
    local is_overflow = compaction.is_context_overflow(
      { stopReason = "error", errorMessage = "prompt is too long" },
      128000
    )
    -- system-prompt builder over a minimal request.
    local prompt = system_prompt.build_system_prompt({
      cwd = pi.cwd(),
      selectedTools = { "read" },
      toolSnippets = { read = "Read a file" },
      contextFiles = {},
      skills = {},
      readmePath = "",
      docsPath = "",
      examplesPath = "",
      now = 1700000000,
    })
    -- session-runtime: expose the construct/session_startup closures.
    local deps = {
      construct = type(session_runtime.construct_session) == "function",
      startup = type(session_runtime.session_startup_from_request) == "function",
      persist = type(session_runtime.persist_agent_event) == "function",
    }
    -- bash-executor: get_shell_config resolves a shell deterministically.
    local shell_path = bash_executor.get_shell_config(pi.env.SHELL or "")
    return {
      toLlmRole = to_llm[1] and to_llm[1].role or nil,
      estimated = estimated,
      overflow = is_overflow,
      promptHasGuidelines = prompt:find("Guidelines:") ~= nil,
      session = deps,
      shell = shell_path,
    }
  end,
})