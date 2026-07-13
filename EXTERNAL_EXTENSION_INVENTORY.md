# External extension capability inventory

Generated from checked fixtures for all 15 extensions at `pi-flake` `94694da7321ce74aa7b82c13db7e60e28c0caba6`
(`extensions` tree `c4a04dfe88314b5e48ebb200ccfd546645c3af9e`). The source fixture hashes live in
`tests/external-extension-inventory/provenance.json`; normal generation/checking is offline.

Statuses are closed: `implemented`, a specific `planned 9.x` rung, or an explicit DESIGN exception.

## Pi API use

| Capability | Extensions | Status | Evidence / target |
|---|---|---|---|
| `ExtensionAPI.appendEntry` | `earendil_pi-review`, `pi-context-janitor` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionAPI.exec` | `earendil_pi-review` | implemented | Existing public host registries/process seam; complete dogfood behavior remains covered by owning rows |
| `ExtensionAPI.getActiveTools` | `pi-tool-management` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionAPI.getAllTools` | `pi-tool-management` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionAPI.getThinkingLevel` | `pi-minimal-editor`, `pi-working-indicator` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionAPI.on` | `earendil_pi-review`, `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-gecko-websearch`, `pi-hashline`, `pi-minimal-editor`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `pi-rtk`, `pi-tool-management`, `pi-working-indicator`, `sting8k_pi-vcc` | implemented | Existing public host registries/process seam; complete dogfood behavior remains covered by owning rows |
| `ExtensionAPI.registerCommand` | `earendil_pi-review`, `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-morph`, `pi-pomodoro`, `pi-tool-management`, `sting8k_pi-vcc` | implemented | Existing public host registries/process seam; complete dogfood behavior remains covered by owning rows |
| `ExtensionAPI.registerMessageRenderer` | `pi-compact`, `pi-context-janitor`, `pi-rlm` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionAPI.registerTool` | `pi-gecko-websearch`, `pi-hashline`, `pi-morph`, `pi-rlm`, `pi-rtk`, `pi-webfetch`, `sting8k_pi-vcc` | implemented | Existing public host registries/process seam; complete dogfood behavior remains covered by owning rows |
| `ExtensionAPI.sendMessage` | `pi-context-janitor`, `pi-rlm`, `sting8k_pi-vcc` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionAPI.sendUserMessage` | `earendil_pi-review` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionAPI.setActiveTools` | `pi-rlm`, `pi-tool-management` | planned 9.4 | Complete non-UI ExtensionAPI actions and dynamic registries |
| `ExtensionContext.compact` | `sting8k_pi-vcc` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.cwd` | `earendil_pi-review`, `pi-codex-fast`, `pi-compact`, `pi-hashline`, `pi-morph`, `pi-pomodoro`, `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.getContextUsage` | `pi-minimal-editor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.hasUI` | `earendil_pi-review`, `pi-compact`, `pi-context-janitor`, `pi-hashline`, `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.isIdle` | `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.model` | `pi-codex-fast`, `pi-context-janitor`, `pi-minimal-editor`, `pi-rlm`, `pi-working-indicator` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.modelRegistry` | `pi-context-janitor`, `pi-minimal-editor`, `pi-morph`, `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.navigateTree` | `earendil_pi-review` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.sessionManager` | `earendil_pi-review`, `pi-context-janitor`, `pi-minimal-editor`, `pi-rlm`, `sting8k_pi-vcc` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionContext.ui` | `earendil_pi-review`, `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-hashline`, `pi-minimal-editor`, `pi-morph`, `pi-pomodoro`, `pi-tool-management`, `pi-working-indicator`, `sting8k_pi-vcc` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ExtensionUI.custom` | `earendil_pi-review`, `pi-context-janitor`, `pi-tool-management` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.editor` | `earendil_pi-review` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.getEditorText` | `earendil_pi-review` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.notify` | `earendil_pi-review`, `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-hashline`, `pi-morph`, `pi-pomodoro`, `pi-tool-management`, `sting8k_pi-vcc` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.select` | `earendil_pi-review` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.setEditorComponent` | `pi-minimal-editor` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.setEditorText` | `earendil_pi-review` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.setFooter` | `pi-minimal-editor` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.setStatus` | `pi-codex-fast`, `pi-context-janitor`, `pi-morph`, `pi-pomodoro` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.setWidget` | `earendil_pi-review` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.setWorkingIndicator` | `pi-working-indicator` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.setWorkingMessage` | `pi-working-indicator` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ExtensionUI.theme` | `pi-codex-fast`, `pi-compact`, `pi-morph`, `pi-working-indicator` | planned 9.5 | Complete composable extension UI and rendering actions |
| `ModelRegistry.authStorage` | `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ModelRegistry.find` | `pi-context-janitor`, `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ModelRegistry.getAll` | `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ModelRegistry.getApiKeyAndHeaders` | `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ModelRegistry.getApiKeyForProvider` | `pi-context-janitor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ModelRegistry.getAvailable` | `pi-context-janitor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `ModelRegistry.isUsingOAuth` | `pi-minimal-editor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getBranch` | `earendil_pi-review`, `pi-context-janitor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getCwd` | `pi-minimal-editor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getEntries` | `earendil_pi-review`, `pi-minimal-editor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getLeafId` | `earendil_pi-review` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getSessionDir` | `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getSessionFile` | `pi-rlm`, `sting8k_pi-vcc` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getSessionId` | `pi-rlm` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `SessionManager.getSessionName` | `pi-minimal-editor` | planned 9.2 | Complete immutable context facades and queued lifecycle/session actions |
| `event.agent_end` | `pi-context-janitor`, `pi-rlm` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.before_agent_start` | `pi-rlm`, `pi-working-indicator` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.before_provider_request` | `pi-codex-fast`, `pi-morph`, `pi-rlm` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.context` | `pi-context-janitor`, `pi-rlm` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.input` | `pi-rlm` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.message_end` | `pi-compact` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.message_update` | `pi-compact` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.model_select` | `pi-codex-fast`, `pi-context-janitor`, `pi-morph`, `pi-working-indicator` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.session_before_compact` | `sting8k_pi-vcc` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.session_compact` | `sting8k_pi-vcc` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.session_shutdown` | `pi-context-janitor`, `pi-gecko-websearch`, `pi-minimal-editor`, `pi-pomodoro`, `pi-rlm`, `pi-working-indicator` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.session_start` | `earendil_pi-review`, `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-hashline`, `pi-minimal-editor`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `pi-working-indicator` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.session_tree` | `earendil_pi-review`, `pi-context-janitor`, `pi-rlm` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.turn_end` | `pi-context-janitor` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |
| `event.user_bash` | `pi-rtk` | planned 9.3 | Complete event emission, ordering, folding, cancellation, and cleanup |

## Package imports

| Capability | Extensions | Status | Evidence / target |
|---|---|---|---|
| `@earendil-works/pi-ai#StringEnum` | `pi-gecko-websearch`, `pi-hashline`, `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-ai#completeSimple` | `pi-context-janitor`, `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-ai#type:Message` | `sting8k_pi-vcc` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-ai#type:Model` | `pi-context-janitor` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-ai#type:ToolResultMessage` | `pi-context-janitor` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-ai#type:Usage` | `pi-context-janitor` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#AssistantMessageComponent` | `pi-compact` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#BorderedLoader` | `earendil_pi-review` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#CustomEditor` | `pi-minimal-editor` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#CustomMessageComponent` | `pi-compact` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#DEFAULT_MAX_BYTES` | `pi-gecko-websearch`, `pi-hashline`, `pi-webfetch` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#DEFAULT_MAX_LINES` | `pi-gecko-websearch`, `pi-hashline`, `pi-webfetch` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#DefaultResourceLoader` | `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#DynamicBorder` | `earendil_pi-review` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#SessionManager` | `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#SettingsManager` | `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#ToolExecutionComponent` | `pi-compact` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#UserMessageComponent` | `pi-compact` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#convertToLlm` | `sting8k_pi-vcc` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#createAgentSession` | `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#createBashTool` | `pi-rtk` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#createLocalBashOperations` | `pi-rtk` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#createReadTool` | `pi-hashline` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#defineTool` | `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#formatSize` | `pi-gecko-websearch`, `pi-hashline`, `pi-morph`, `pi-webfetch` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#getAgentDir` | `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-gecko-websearch`, `pi-morph`, `pi-rlm`, `pi-tool-management` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#getMarkdownTheme` | `pi-rlm` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#getSettingsListTheme` | `pi-tool-management` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-coding-agent#truncateHead` | `pi-gecko-websearch`, `pi-hashline`, `pi-webfetch` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#type:ExtensionAPI` | `earendil_pi-review`, `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-gecko-websearch`, `pi-hashline`, `pi-minimal-editor`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `pi-rtk`, `pi-tool-management`, `pi-webfetch`, `pi-working-indicator`, `sting8k_pi-vcc` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#type:ExtensionCommandContext` | `earendil_pi-review` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#type:ExtensionContext` | `earendil_pi-review`, `pi-codex-fast`, `pi-context-janitor`, `pi-minimal-editor`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `pi-working-indicator` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-coding-agent#withFileMutationQueue` | `pi-hashline`, `pi-morph` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `@earendil-works/pi-tui#Box` | `pi-rlm` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#Container` | `earendil_pi-review`, `pi-tool-management` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#Input` | `earendil_pi-review` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#Markdown` | `pi-rlm` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#SelectList` | `earendil_pi-review` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#SettingsList` | `pi-tool-management` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#Spacer` | `earendil_pi-review`, `pi-rlm` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#Text` | `earendil_pi-review`, `pi-gecko-websearch`, `pi-hashline`, `pi-morph`, `pi-rlm`, `pi-tool-management`, `pi-webfetch` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#fuzzyFilter` | `earendil_pi-review` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#truncateToWidth` | `pi-compact`, `pi-context-janitor`, `pi-minimal-editor` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#type:Component` | `pi-compact`, `pi-context-janitor` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#type:SelectItem` | `earendil_pi-review` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#type:SettingItem` | `pi-tool-management` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@earendil-works/pi-tui#visibleWidth` | `pi-compact`, `pi-minimal-editor` | planned 9.5 | Public TUI components/rendering composition; no private frontend patching |
| `@sinclair/typebox#Type` | `pi-gecko-websearch`, `pi-hashline`, `pi-morph`, `pi-webfetch`, `sting8k_pi-vcc` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `child_process#spawn` | `pi-gecko-websearch` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `child_process#spawnSync` | `pi-gecko-websearch` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `child_process#type:ChildProcess` | `pi-gecko-websearch` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `fs#*` | `pi-gecko-websearch` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `fs#existsSync` | `sting8k_pi-vcc` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `fs#mkdirSync` | `sting8k_pi-vcc` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `fs#readFileSync` | `sting8k_pi-vcc` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `fs#writeFileSync` | `sting8k_pi-vcc` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `net#*` | `pi-gecko-websearch` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:child_process#execFileSync` | `pi-rtk` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:child_process#spawn` | `pi-rlm` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:crypto#createHash` | `pi-context-janitor` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:crypto#randomUUID` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#constants` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#existsSync` | `pi-codex-fast`, `pi-compact`, `pi-morph`, `pi-pomodoro`, `pi-rlm` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#mkdirSync` | `pi-pomodoro` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#promises` | `earendil_pi-review`, `pi-rlm` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#readFileSync` | `pi-codex-fast`, `pi-compact`, `pi-morph`, `pi-pomodoro`, `pi-rlm` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#renameSync` | `pi-pomodoro` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#unwatchFile` | `pi-pomodoro` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#watchFile` | `pi-pomodoro` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs#writeFileSync` | `pi-pomodoro` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#access` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#chmod` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#lstat` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#mkdir` | `pi-context-janitor`, `pi-hashline`, `pi-morph`, `pi-tool-management` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#open` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#readFile` | `pi-context-janitor`, `pi-hashline`, `pi-morph`, `pi-rlm`, `pi-tool-management` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#readlink` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#rename` | `pi-hashline`, `pi-morph` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#stat` | `pi-hashline`, `pi-morph` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#unlink` | `pi-hashline`, `pi-morph` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:fs/promises#writeFile` | `pi-context-janitor`, `pi-morph`, `pi-tool-management` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:os#*` | `pi-hashline`, `pi-rlm` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:os#homedir` | `pi-compact` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:os#tmpdir` | `pi-pomodoro` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#*` | `pi-rlm` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#default` | `earendil_pi-review` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#dirname` | `pi-hashline`, `pi-morph`, `pi-pomodoro` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#isAbsolute` | `pi-hashline`, `pi-morph` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#join` | `pi-codex-fast`, `pi-compact`, `pi-context-janitor`, `pi-hashline`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `pi-tool-management` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#parse` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#resolve` | `pi-hashline`, `pi-morph` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:path#sep` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:readline#createInterface` | `pi-rlm` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `node:util#TextDecoder` | `pi-hashline` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `os#*` | `pi-gecko-websearch` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `os#homedir` | `sting8k_pi-vcc` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `path#*` | `pi-gecko-websearch` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `path#dirname` | `sting8k_pi-vcc` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `path#join` | `sting8k_pi-vcc` | planned 9.9 | Lua-native host mechanisms replace Node built-in modules |
| `turndown#dynamic` | `pi-webfetch` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |
| `typebox#Type` | `pi-rlm` | planned 9.7 | Deterministic packaged Lua modules or reviewed pure-Lua dependencies |

## Node ambient capabilities

| Capability | Extensions | Status | Evidence / target |
|---|---|---|---|
| `AbortController` | `pi-context-janitor`, `pi-rlm` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `AbortSignal` | `pi-context-janitor`, `pi-hashline`, `pi-morph`, `pi-rlm`, `pi-webfetch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `Buffer.alloc` | `pi-gecko-websearch`, `pi-hashline` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `Buffer.byteLength` | `pi-gecko-websearch`, `pi-hashline`, `pi-morph`, `pi-rlm`, `pi-webfetch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `Buffer.concat` | `pi-gecko-websearch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `Buffer.from` | `pi-hashline` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `Bun.hash.xxHash32` | `pi-hashline` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `URL` | `pi-webfetch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `clearInterval` | `pi-compact`, `pi-context-janitor`, `pi-pomodoro` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `clearTimeout` | `pi-context-janitor`, `pi-gecko-websearch`, `pi-rlm`, `pi-working-indicator` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `fetch` | `pi-morph`, `pi-webfetch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.cwd` | `pi-gecko-websearch`, `pi-rtk` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.AI_GATEWAY_API_KEY` | `pi-morph` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.HOME` | `pi-minimal-editor`, `pi-morph`, `pi-pomodoro` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_GECKO_BINARY` | `pi-gecko-websearch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_GECKO_MAX_BROWSERS` | `pi-gecko-websearch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_GECKO_PROFILE` | `pi-gecko-websearch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_GECKO_PROFILE_ROOT` | `pi-gecko-websearch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_HASHLINE_DEBUG` | `pi-hashline` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_RLM_LOG_DIR` | `pi-rlm` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_RLM_LOG_PATH` | `pi-rlm` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_RLM_PYTHON` | `pi-rlm` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_RLM_ROOT_EXTERNALIZE_CHARS` | `pi-rlm` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.PI_VCC_CONFIG_PATH` | `sting8k_pi-vcc` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.USER` | `pi-pomodoro` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.USERPROFILE` | `pi-minimal-editor`, `pi-pomodoro` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env.XDG_RUNTIME_DIR` | `pi-pomodoro` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.env[dynamic]` | `pi-morph` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `process.platform` | `pi-gecko-websearch` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `setInterval` | `pi-context-janitor`, `pi-pomodoro` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |
| `setTimeout` | `pi-context-janitor`, `pi-gecko-websearch`, `pi-rlm`, `pi-working-indicator`, `sting8k_pi-vcc` | planned 9.9 | Lua-native environment, binary-data, HTTP, abort, and timer mechanisms |

## Timers and background lifetimes

| Capability | Extensions | Status | Evidence / target |
|---|---|---|---|
| `cleanup.session_shutdown` | `pi-context-janitor`, `pi-gecko-websearch`, `pi-pomodoro`, `pi-rlm`, `pi-working-indicator` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |
| `resource.child_process` | `pi-gecko-websearch`, `pi-rlm` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |
| `resource.file_watcher` | `pi-pomodoro` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |
| `resource.tcp_socket` | `pi-gecko-websearch` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |
| `timer.clearInterval` | `pi-compact`, `pi-context-janitor`, `pi-pomodoro` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |
| `timer.clearTimeout` | `pi-context-janitor`, `pi-gecko-websearch`, `pi-rlm`, `pi-working-indicator` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |
| `timer.setInterval` | `pi-context-janitor`, `pi-pomodoro` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |
| `timer.setTimeout` | `pi-context-janitor`, `pi-gecko-websearch`, `pi-rlm`, `pi-working-indicator`, `sting8k_pi-vcc` | planned 9.9 | Explicit runtime/session/dispatch ownership with deterministic cancellation and disposal |

## Process / socket / filesystem / crypto needs

| Capability | Extensions | Status | Evidence / target |
|---|---|---|---|
| `crypto.createHash` | `pi-context-janitor` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `crypto.randomUUID` | `pi-hashline` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `crypto.sha256` | `pi-context-janitor` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `crypto.xxHash32` | `pi-hashline` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.access` | `pi-hashline` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.appendFile` | `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.chmod` | `pi-hashline` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.constants` | `pi-hashline` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.copyFileSync` | `pi-gecko-websearch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.existsSync` | `pi-codex-fast`, `pi-compact`, `pi-gecko-websearch`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `sting8k_pi-vcc` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.lstat` | `pi-hashline`, `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.mkdir` | `pi-context-janitor`, `pi-hashline`, `pi-morph`, `pi-rlm`, `pi-tool-management` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.mkdirSync` | `pi-pomodoro`, `sting8k_pi-vcc` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.mkdtemp` | `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.mkdtempSync` | `pi-gecko-websearch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.open` | `pi-hashline` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.readFile` | `earendil_pi-review`, `pi-context-janitor`, `pi-hashline`, `pi-morph`, `pi-rlm`, `pi-tool-management` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.readFileSync` | `pi-codex-fast`, `pi-compact`, `pi-gecko-websearch`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `sting8k_pi-vcc` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.readdir` | `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.readdirSync` | `pi-gecko-websearch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.readlink` | `pi-hashline` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.rename` | `pi-hashline`, `pi-morph` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.renameSync` | `pi-pomodoro` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.rm` | `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.rmSync` | `pi-gecko-websearch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.stat` | `earendil_pi-review`, `pi-hashline`, `pi-morph` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.unlink` | `pi-hashline`, `pi-morph` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.unwatchFile` | `pi-pomodoro` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.watchFile` | `pi-pomodoro` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.writeFile` | `pi-context-janitor`, `pi-morph`, `pi-rlm`, `pi-tool-management` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `filesystem.writeFileSync` | `pi-gecko-websearch`, `pi-pomodoro`, `sting8k_pi-vcc` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `network.http` | `pi-morph`, `pi-webfetch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `process.environment` | `pi-gecko-websearch`, `pi-hashline`, `pi-minimal-editor`, `pi-morph`, `pi-pomodoro`, `pi-rlm`, `sting8k_pi-vcc` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `process.execFileSync` | `pi-rtk` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `process.kill` | `pi-gecko-websearch`, `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `process.pi.exec` | `earendil_pi-review` | implemented | Public pi.exec process mechanism |
| `process.spawn` | `pi-gecko-websearch`, `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `process.spawnSync` | `pi-gecko-websearch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `process.stdio_pipes` | `pi-gecko-websearch`, `pi-rlm` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `socket.Socket` | `pi-gecko-websearch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |
| `socket.createConnection` | `pi-gecko-websearch` | planned 9.9 | Inventory-owned process/socket/filesystem/network/crypto mechanisms |

## Private Pi implementation dependencies

| Capability | Extensions | Status | Evidence / target |
|---|---|---|---|
| `AssistantMessageComponent` | `pi-compact` | planned 9.5 | Replace pi-compact class patching with public ordered rendering middleware |
| `CustomMessageComponent` | `pi-compact` | planned 9.5 | Replace pi-compact class patching with public ordered rendering middleware |
| `DefaultResourceLoader` | `pi-rlm` | planned 9.7 | Replace RLM concrete session/resource construction with public modules and mechanisms |
| `SessionManager` | `pi-rlm` | planned 9.7 | Replace RLM concrete session/resource construction with public modules and mechanisms |
| `SettingsManager` | `pi-rlm` | planned 9.7 | Replace RLM concrete session/resource construction with public modules and mechanisms |
| `ToolExecutionComponent` | `pi-compact` | planned 9.5 | Replace pi-compact class patching with public ordered rendering middleware |
| `UserMessageComponent` | `pi-compact` | planned 9.5 | Replace pi-compact class patching with public ordered rendering middleware |
| `createAgentSession` | `pi-rlm` | planned 9.7 | Replace RLM concrete session/resource construction with public modules and mechanisms |

## Per-extension coverage

| Extension | Source files | Capability rows |
|---|---:|---:|
| `earendil_pi-review` | 1 | 39 |
| `pi-codex-fast` | 1 | 19 |
| `pi-compact` | 16 | 32 |
| `pi-context-janitor` | 9 | 54 |
| `pi-gecko-websearch` | 4 | 49 |
| `pi-hashline` | 10 | 57 |
| `pi-minimal-editor` | 1 | 23 |
| `pi-morph` | 11 | 47 |
| `pi-pomodoro` | 1 | 38 |
| `pi-rlm` | 19 | 82 |
| `pi-rtk` | 1 | 9 |
| `pi-tool-management` | 1 | 22 |
| `pi-webfetch` | 1 | 14 |
| `pi-working-indicator` | 1 | 18 |
| `sting8k_pi-vcc` | 30 | 30 |

Inventory counts: extensions=15, source_files=107, pi_api=65, package_imports=101, node_ambient=31, lifetimes=8, system_needs=41, private_pi=8.
