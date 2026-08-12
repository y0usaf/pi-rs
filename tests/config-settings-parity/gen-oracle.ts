// Generate tests/config-settings-parity/oracle.json by driving Pi's real
// SettingsManager (merge + migration + typed getters) and KeybindingsManager
// (resolution) over a corpus of declaration outcomes. The oracle records
// Pi's effective read-model: for each scenario the global settings, project
// settings, the deep-merged effective settings, and the typed getters that
// the merged result resolves to. pi-rs replays equivalent config.lua
// declarations and asserts the same getters.
//
// Run via scripts/config-settings-oracle. Offline normal checks consume the
// checked oracle; opt-in regeneration drives the pinned Pi source.
import { writeFileSync } from "node:fs";
import {
  SettingsManager,
  type SettingsScope,
  type SettingsStorage,
  type Settings,
} from "../../ref/pi/packages/coding-agent/src/core/settings-manager.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { migrateKeybindingsConfig } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";

// In-memory storage that holds both scopes, mirroring pi-rs's replay.
class DualStorage implements SettingsStorage {
  private global: string | undefined;
  private project: string | undefined;
  constructor(global?: string, project?: string) {
    this.global = global;
    this.project = project;
  }
  withLock(scope: SettingsScope, fn: (current: string | undefined) => string | undefined): void {
    if (scope === "global") {
      this.global = fn(this.global);
    } else {
      this.project = fn(this.project);
    }
  }
}

// The complete set of typed getters we can compare between Pi and pi-rs.
const GETTERS = [
  "getTheme",
  "getDefaultProvider",
  "getDefaultModel",
  "getSteeringMode",
  "getFollowUpMode",
  "getDefaultThinkingLevel",
  "getTransport",
  "getCompactionEnabled",
  "getCompactionReserveTokens",
  "getCompactionKeepRecentTokens",
  "getCompactionSettings",
  "getBranchSummarySettings",
  "getBranchSummarySkipPrompt",
  "getRetryEnabled",
  "getRetrySettings",
  "getHttpIdleTimeoutMs",
  "getProviderRetrySettings",
  "getWebSocketConnectTimeoutMs",
  "getHideThinkingBlock",
  "getShellPath",
  "getQuietStartup",
  "getDefaultProjectTrust",
  "getShellCommandPrefix",
  "getNpmCommand",
  "getCollapseChangelog",
  "getEnableInstallTelemetry",
  "getPackages",
  "getExtensionPaths",
  "getSkillPaths",
  "getPromptTemplatePaths",
  "getThemePaths",
  "getEnableSkillCommands",
  "getThinkingBudgets",
  "getShowImages",
  "getImageWidthCells",
  "getClearOnShrink",
  "getShowTerminalProgress",
  "getImageAutoResize",
  "getBlockImages",
  "getEnabledModels",
  "getDoubleEscapeAction",
  "getTreeFilterMode",
  "getShowHardwareCursor",
  "getEditorPaddingX",
  "getAutocompleteMaxVisible",
  "getCodeBlockIndent",
  "getWarnings",
] as const;

function stringify(v: unknown): unknown {
  if (v === undefined) return null;
  if (typeof v === "object") return JSON.parse(JSON.stringify(v));
  return v;
}

function getterOuts(m: SettingsManager): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const g of GETTERS) {
    try {
      out[g] = stringify((m as unknown as Record<string, () => unknown>)[g].call(m));
    } catch (e) {
      out[g] = { __error: e instanceof Error ? e.message : String(e) };
    }
  }
  return out;
}

// Scenario: effective settings outcomes Pi produces from given global/project
// settings maps. Each map is a plain Settings object (the outcome pi-rs must
// match), not Pi's JSON file format.
const SCENARIOS: { name: string; global: Settings; project?: Settings; projectTrusted?: boolean }[] = [
  { name: "defaults", global: {} },
  {
    name: "global-only-theme-retry",
    global: { theme: "dark", retry: { enabled: true, maxRetries: 2 } },
  },
  {
    name: "project-overrides-global",
    global: { theme: "dark", retry: { enabled: true, maxRetries: 2 }, enabledModels: ["openai/*"] },
    project: { theme: "light", retry: { enabled: false }, enabledModels: ["anthropic/*"] },
  },
  {
    name: "untrusted-project-ignored",
    global: { theme: "dark" },
    project: { theme: "light" },
    projectTrusted: false,
  },
  {
    name: "nested-one-level-merge-keeps-untouched",
    global: { retry: { enabled: true, maxRetries: 3, baseDelayMs: 5000 } },
    project: { retry: { enabled: false } },
  },
  {
    name: "migrations-queueMode-websockets",
    global: { queueMode: "one-at-a-time", websockets: false },
  },
  {
    name: "migrations-skills-object-format",
    global: {
      skills: { enableSkillCommands: false, customDirectories: ["dir1", "dir2"] } as unknown as string[],
    },
  },
  {
    name: "migrations-retry-maxDelayMs",
    global: { retry: { maxDelayMs: 9000 } as unknown as object },
  },
  {
    name: "migrations-retry-maxDelayMs-keeps-provider",
    global: { retry: { maxDelayMs: 9000, provider: { timeoutMs: 1000 } } as unknown as object },
  },
  {
    name: "terminal-images-context",
    global: {
      terminal: { showImages: false, imageWidthCells: 40, clearOnShrink: true, showTerminalProgress: true },
      images: { autoResize: false, blockImages: true },
    },
  },
  {
    name: "clamped-getters-editorPadding-autocomplete",
    global: { editorPaddingX: 9, autocompleteMaxVisible: 99, imageWidthCells: 200 },
  },
  {
    name: "thinking-levels",
    global: { defaultThinkingLevel: "high", hideThinkingBlock: true },
  },
  {
    name: "session-navigation-reads",
    global: { doubleEscapeAction: "fork", treeFilterMode: "no-tools" },
  },
  {
    name: "resource-paths-packages",
    global: {
      extensions: ["ext1"],
      skills: ["skill1"],
      prompts: ["prompt1"],
      themes: ["theme1"],
      packages: ["pkg1", { source: "git:demo", extensions: ["a.lua"] }],
    },
  },
  {
    name: "project-merge-all-kinds",
    global: {
      theme: "dark",
      defaultProvider: "anthropic",
      defaultModel: "claude-opus-4-6",
      terminal: { showImages: false, imageWidthCells: 50 },
      packages: ["global-pkg"],
    },
    project: {
      theme: "light",
      terminal: { showImages: true },
      packages: ["project-pkg"],
    },
  },
];

const out: Record<string, unknown> = {
  oracle: "Pi v0.79.0 c5582102",
  getters: GETTERS,
  scenarios: SCENARIOS.map((s) => {
    const storage = new DualStorage(JSON.stringify(s.global), JSON.stringify(s.project ?? {}));
    const m = SettingsManager.fromStorage(storage, { projectTrusted: s.projectTrusted ?? true });
    const projectContent = s.projectTrusted === false ? {} : m.getProjectSettings();
    return {
      name: s.name,
      projectTrusted: s.projectTrusted ?? true,
      global: stringify(m.getGlobalSettings()),
      project: stringify(projectContent),
      getters: getterOuts(m),
    };
  }),
  // Effective keybinding resolution + legacy-name migration, matching Pi's
  // KeybindingsManager (keybindings.ts). User bindings override defaults; the
  // resolved map collapses single-key arrays to the bare key; unknown actions
  // and legacy names are handled by migrateKeybindingsConfig.
  keybindings: {
    migrate: [
      { name: "legacy-and-modern", raw: { cursorUp: "ctrl+p", "app.exit": ["ctrl+q"] } },
      { name: "legacy-collision-keeps-modern", raw: { cursorUp: "ctrl+p", "tui.editor.cursorUp": "ctrl+e" } },
      { name: "unknown-and-array", raw: { "no.such.action": "ctrl+x", "app.tools.expand": ["ctrl+o", "ctrl+1"] } },
    ].map((c) => ({ name: c.name, raw: c.raw, ...migrateKeybindingsConfig(c.raw) })),
    resolved: [
      { name: "simple-overrides", user: { "app.exit": ["ctrl+d", "ctrl+q"], "tui.editor.cursorLeft": "ctrl+h" } },
      { name: "empty-defaults-action", user: { "app.session.new": "ctrl+n" } },
      { name: "conflicts", user: { "app.exit": "ctrl+a", "app.clear": "ctrl+a" } },
      { name: "no-user", user: {} },
    ].map((c) => {
      const km = new KeybindingsManager(c.user);
      return { name: c.name, user: c.user, resolved: km.getResolvedBindings(), conflicts: km.getConflicts() };
    }),
  },
};

writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[2]}`);
