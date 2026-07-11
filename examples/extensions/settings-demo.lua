-- Exerciser for `pi.settings`, the public SettingsManager mechanism.
-- Product policy and `/settings` UI live in the embedded Lua frontend;
local pi = ...

pi.register_command("settings-demo", {
  description = "Walk the settings bindings",
  handler = function()
    local initial = pi.settings.block_images()

    -- setBlockImages writes the global scope and persists granularly.
    pi.settings.set_block_images(true)
    local blocked = pi.settings.block_images()

    pi.settings.set_block_images(false)
    local unblocked = pi.settings.block_images()

    -- Tree-navigation reads (PLAN 6.4): getDoubleEscapeAction (default
    -- "tree"), getTreeFilterMode (validated, default "default"), and
    -- getBranchSummarySettings (reserveTokens 16384, skipPrompt false).
    local branch_summary = pi.settings.branch_summary()

    -- Compaction read (PLAN 6.5): getCompactionSettings — enabled true,
    -- reserveTokens 16384, keepRecentTokens 20000 by default.
    local compaction = pi.settings.compaction_settings()

    -- Bash-mode reads (PLAN 7.1): getShellCommandPrefix / getShellPath —
    -- both unset by default; AgentSession.executeBash consumes them.
    local shell_prefix = pi.settings.shell_command_prefix()
    local shell_path = pi.settings.shell_path()

    -- Thinking default (PLAN 7.2): getDefaultThinkingLevel /
    -- setDefaultThinkingLevel — agent-session.ts setThinkingLevel
    -- persists the level whenever it changes on a thinking-capable model.
    local thinking_initial = pi.settings.default_thinking_level()
    pi.settings.set_default_thinking_level("high")
    local thinking_set = pi.settings.default_thinking_level()

    -- `/settings` family (PLAN 7.3): representative scalar, nested-object,
    -- queue-mode, UI, theme, timeout, and model-default persistence.
    pi.settings.set_theme("light")
    pi.settings.set_steering_mode("all")
    pi.settings.set_follow_up_mode("all")
    pi.settings.set_http_idle_timeout_ms(60000)
    pi.settings.set_editor_padding_x(2)
    pi.settings.set_autocomplete_max_visible(7)
    pi.settings.set_warnings({ anthropicExtraUsage = false })
    pi.settings.set_default_model_and_provider("anthropic", "claude-opus-4-6")
    -- `/scoped-models` stores its ordered session cycling set globally.
    pi.settings.set_enabled_models({ "anthropic/claude-opus-4-6", "openai/gpt-5.4" })
    local enabled_models = pi.settings.enabled_models()
    -- Startup changelog policy records the version it has consumed.
    local changelog_initial = pi.settings.last_changelog_version()
    pi.settings.set_last_changelog_version("0.79.0")

    return {
      initial = initial, blocked = blocked, unblocked = unblocked,
      shellCommandPrefixUnset = shell_prefix == nil,
      shellPathUnset = shell_path == nil,
      doubleEscapeAction = pi.settings.double_escape_action(),
      treeFilterMode = pi.settings.tree_filter_mode(),
      branchSummaryReserveTokens = branch_summary.reserveTokens,
      branchSummarySkipPrompt = branch_summary.skipPrompt,
      compactionEnabled = compaction.enabled,
      compactionReserveTokens = compaction.reserveTokens,
      compactionKeepRecentTokens = compaction.keepRecentTokens,
      defaultThinkingLevelUnset = thinking_initial == nil,
      defaultThinkingLevelSet = thinking_set,
      theme = pi.settings.theme(), steeringMode = pi.settings.steering_mode(),
      followUpMode = pi.settings.follow_up_mode(), httpIdleTimeoutMs = pi.settings.http_idle_timeout_ms(),
      editorPaddingX = pi.settings.editor_padding_x(),
      autocompleteMaxVisible = pi.settings.autocomplete_max_visible(),
      anthropicWarning = pi.settings.warnings().anthropicExtraUsage,
      enabledModels = enabled_models,
      lastChangelogVersionUnset = changelog_initial == nil,
      lastChangelogVersion = pi.settings.last_changelog_version(),
    }
  end,
})
