import { readFileSync } from "node:fs";
import { clampThinkingLevel, getSupportedThinkingLevels, type Model } from "../../ref/pi/packages/ai/src/models.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { findExactModelReferenceMatch } from "../../ref/pi/packages/coding-agent/src/core/model-resolver.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import {
  keyHint,
  keyText,
  rawKeyHint,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
type Step = { name: string; input?: string[]; resize?: { columns: number; rows: number } };
type Scenario = {
  columns: number;
  rows: number;
  appName: string;
  version: string;
  cwd: string;
  home: string;
  branch: string;
  providerCount: number;
  thinkingLevel?: ThinkingLevel;
  usage: { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number };
  contextUsage: { percent: number | null; contextWindow: number };
  model: Model<any>;
  models: Model<any>[];
  steps: Step[];
};

class CaptureTerminal implements Terminal {
  private input?: (data: string) => void; private resized?: () => void; private chunks: string[] = [];
  kittyProtocolActive = true;
  constructor(public columns: number, public rows: number) {}
  start(input: (data: string) => void, resized: () => void): void { this.input = input; this.resized = resized; }
  async drainInput(): Promise<void> {} stop(): void {}
  write(data: string): void { this.chunks.push(data); }
  moveBy(lines: number): void { if (lines > 0) this.write(`\x1b[${lines}B`); else if (lines < 0) this.write(`\x1b[${-lines}A`); }
  hideCursor(): void { this.write("\x1b[?25l"); } showCursor(): void { this.write("\x1b[?25h"); }
  clearLine(): void { this.write("\x1b[K"); } clearFromCursor(): void { this.write("\x1b[J"); }
  clearScreen(): void { this.write("\x1b[2J\x1b[H"); } setTitle(): void {} setProgress(): void {}
  send(data: string): void { this.input?.(data); }
  resize(columns: number, rows: number): void { this.columns = columns; this.rows = rows; this.resized?.(); }
  take(): string { const result = this.chunks.join(""); this.chunks = []; return result; }
}

const scenario = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Scenario;
process.env.HOME = scenario.home;
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
initTheme("dark", false);

const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);

// interactive-mode.ts ExpandableText (private helper class).
class ExpandableText extends Text {
  private readonly getCollapsedText: () => string;
  private readonly getExpandedText: () => string;
  constructor(getCollapsedText: () => string, getExpandedText: () => string, expanded = false, paddingX = 0, paddingY = 0) {
    super(expanded ? getExpandedText() : getCollapsedText(), paddingX, paddingY);
    this.getCollapsedText = getCollapsedText;
    this.getExpandedText = getExpandedText;
  }
  setExpanded(expanded: boolean): void {
    this.setText(expanded ? this.getExpandedText() : this.getCollapsedText());
  }
}

// interactive-mode.ts init(): the startup header built from keybinding hints.
const logo = theme.bold(theme.fg("accent", scenario.appName)) + theme.fg("dim", ` v${scenario.version}`);
const compactInstructions = [
  keyHint("app.interrupt", "interrupt"),
  rawKeyHint(`${keyText("app.clear")}/${keyText("app.exit")}`, "clear/exit"),
  rawKeyHint("/", "commands"),
  rawKeyHint("!", "bash"),
  keyHint("app.tools.expand", "more"),
].join(theme.fg("muted", " · "));
const compactOnboarding = theme.fg(
  "dim",
  `Press ${keyText("app.tools.expand")} to show full startup help and loaded resources.`,
);
const onboarding = theme.fg(
  "dim",
  `Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.`,
);
const builtInHeader = new ExpandableText(
  () => `${logo}\n${compactInstructions}\n${compactOnboarding}\n\n${onboarding}`,
  () => `${logo}\n${compactInstructions}\n${compactOnboarding}\n\n${onboarding}`,
  false,
  1,
  0,
);

// interactive-mode.ts init() ui.addChild composition.
const headerContainer = new Container();
headerContainer.addChild(new Spacer(1));
headerContainer.addChild(builtInHeader);
headerContainer.addChild(new Spacer(1));
const chatContainer = new Container();
const pendingMessagesContainer = new Container();
const statusContainer = new Container();
const widgetContainerAbove = new Container();
widgetContainerAbove.addChild(new Spacer(1)); // renderWidgets' spacer-when-empty default
const editorContainer = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, {
  paddingX: 0,
  autocompleteMaxVisible: 5,
});
editorContainer.addChild(editor);

const models = scenario.models;
let currentModel: Model<any> = scenario.model;
let thinkingLevel: ThinkingLevel = scenario.thinkingLevel ?? "off";

// core/settings-manager.ts: the defaultThinkingLevel slice as an
// in-memory store (pi-rs's side runs the real settings manager against the
// harness's pinned empty agent dir — both start unset).
let settingsDefaultThinkingLevel: ThinkingLevel | undefined;
const settingsManager = {
  getDefaultThinkingLevel: (): ThinkingLevel | undefined => settingsDefaultThinkingLevel,
  setDefaultThinkingLevel: (level: ThinkingLevel): void => {
    settingsDefaultThinkingLevel = level;
  },
};

// FooterComponent over the mutable model/thinking state.
const sessionStub = {
  get state() {
    return { model: currentModel, thinkingLevel } as never;
  },
  sessionManager: {
    getEntries: () => [
      {
        type: "message",
        message: {
          role: "assistant",
          usage: {
            input: scenario.usage.input,
            output: scenario.usage.output,
            cacheRead: scenario.usage.cacheRead,
            cacheWrite: scenario.usage.cacheWrite,
            cost: { total: scenario.usage.cost },
          },
        },
      },
    ],
    getCwd: () => scenario.cwd,
    getSessionName: () => undefined,
  },
  getContextUsage: () => scenario.contextUsage,
  modelRegistry: { isUsingOAuth: () => false },
} as unknown as AgentSession;
const footerData = {
  getGitBranch: () => scenario.branch,
  getAvailableProviderCount: () => scenario.providerCount,
  getExtensionStatuses: () => new Map<string, string>(),
} as unknown as ReadonlyFooterDataProvider;
const footer = new FooterComponent(sessionStub, footerData);

ui.addChild(headerContainer);
ui.addChild(chatContainer);
ui.addChild(pendingMessagesContainer);
ui.addChild(statusContainer);
ui.addChild(widgetContainerAbove);
ui.addChild(editorContainer);
ui.addChild(footer);
ui.setFocus(editor);
ui.start();

// interactive-mode.ts UI helper ports over the harness containers.
let lastStatusSpacer: Spacer | undefined;
let lastStatusText: Text | undefined;
function showStatus(message: string): void {
  const children = chatContainer.children;
  const last = children.length > 0 ? children[children.length - 1] : undefined;
  const secondLast = children.length > 1 ? children[children.length - 2] : undefined;
  if (last && secondLast && last === lastStatusText && secondLast === lastStatusSpacer) {
    lastStatusText!.setText(theme.fg("dim", message));
    ui.requestRender();
    return;
  }
  const spacer = new Spacer(1);
  const text = new Text(theme.fg("dim", message), 1, 0);
  chatContainer.addChild(spacer);
  chatContainer.addChild(text);
  lastStatusSpacer = spacer;
  lastStatusText = text;
  ui.requestRender();
}
function showError(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("error", `Error: ${message}`), 1, 0));
  chatContainer.addChild(new Spacer(1));
  ui.requestRender();
}

// --- agent-session.ts thinking slice over the real pi-ai helpers ---

function getAvailableThinkingLevels(): ThinkingLevel[] {
  if (!currentModel) return ["off", "minimal", "low", "medium", "high", "xhigh"];
  return getSupportedThinkingLevels(currentModel) as ThinkingLevel[];
}

function supportsThinking(): boolean {
  return !!currentModel?.reasoning;
}

// agent-session.ts setThinkingLevel (the session/settings persistence is
// side-effect-free here beyond the settings stub; pi-rs pins the persisted
// artifacts in its behavior test).
function setThinkingLevel(level: ThinkingLevel): void {
  const availableLevels = getAvailableThinkingLevels();
  const effectiveLevel = availableLevels.includes(level)
    ? level
    : currentModel
      ? (clampThinkingLevel(currentModel, level) as ThinkingLevel)
      : "off";
  const isChanging = effectiveLevel !== thinkingLevel;
  thinkingLevel = effectiveLevel;
  if (isChanging) {
    if (supportsThinking() || effectiveLevel !== "off") {
      settingsManager.setDefaultThinkingLevel(effectiveLevel);
    }
    // interactive-mode.ts "thinking_level_changed" handler body.
    footer.invalidate();
    updateEditorBorderColor();
  }
}

// agent-session.ts cycleThinkingLevel.
function sessionCycleThinkingLevel(): ThinkingLevel | undefined {
  if (!supportsThinking()) return undefined;
  const levels = getAvailableThinkingLevels();
  const currentIndex = levels.indexOf(thinkingLevel);
  const nextIndex = (currentIndex + 1) % levels.length;
  const nextLevel = levels[nextIndex]!;
  setThinkingLevel(nextLevel);
  return nextLevel;
}

// agent-session.ts _getThinkingLevelForModelSwitch.
function getThinkingLevelForModelSwitch(explicitLevel?: ThinkingLevel): ThinkingLevel {
  if (explicitLevel !== undefined) return explicitLevel;
  if (!supportsThinking()) {
    return settingsManager.getDefaultThinkingLevel() ?? "medium";
  }
  return thinkingLevel;
}

// agent-session.ts setModel — the reachable slice (all scenario models
// have configured auth).
function setModel(model: Model<any>): void {
  const level = getThinkingLevelForModelSwitch();
  currentModel = model;
  footer.invalidate();
  // Re-clamp thinking level for new model's capabilities.
  setThinkingLevel(level);
}

// interactive-mode.ts updateEditorBorderColor (no bash mode in this
// fixture's scenario space; the isBashMode branch is pinned by bash-turn).
function updateEditorBorderColor(): void {
  editor.borderColor = theme.getThinkingBorderColor(thinkingLevel);
  ui.requestRender();
}

// interactive-mode.ts cycleThinkingLevel.
function cycleThinkingLevel(): void {
  const newLevel = sessionCycleThinkingLevel();
  if (newLevel === undefined) {
    showStatus("Current model does not support thinking");
  } else {
    footer.invalidate();
    updateEditorBorderColor();
    showStatus(`Thinking level: ${newLevel}`);
  }
}

// interactive-mode.ts handleModelCommand — the exact-reference slice (the
// selector path is pinned by model-turn).
function handleModelCommand(searchTerm: string): void {
  const model = findExactModelReferenceMatch(searchTerm, models);
  if (model) {
    try {
      setModel(model);
      showStatus(`Model: ${model.id}`);
    } catch (error) {
      showError(error instanceof Error ? error.message : String(error));
    }
  }
}

editor.onAction("app.thinking.cycle", () => cycleThinkingLevel());

// setupEditorSubmitHandler: the reachable slice (/model exact references).
editor.onSubmit = (text: string) => {
  text = text.trim();
  if (!text) return;
  editor.setText("");
  if (text.startsWith("/model ")) {
    handleModelCommand(text.slice("/model ".length).trim());
  }
};

updateEditorBorderColor();

const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}

async function main() {
  await capture("startup", true);
  for (const step of scenario.steps) {
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    for (const data of step.input ?? []) terminal.send(data);
    await capture(step.name, Boolean(step.resize));
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
