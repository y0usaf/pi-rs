import { readFileSync } from "node:fs";
import {
  clampThinkingLevel,
  getSupportedThinkingLevels,
  modelsAreEqual,
  type Model,
} from "../../ref/pi/packages/ai/src/models.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { findExactModelReferenceMatch } from "../../ref/pi/packages/coding-agent/src/core/model-resolver.ts";
import type { ModelRegistry } from "../../ref/pi/packages/coding-agent/src/core/model-registry.ts";
import type { SettingsManager } from "../../ref/pi/packages/coding-agent/src/core/settings-manager.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import { ModelSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/model-selector.ts";
import { initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { Container, CURSOR_MARKER, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = {
  name: string;
  show?: boolean;
  search?: string;
  command?: string;
  cycle?: "forward" | "backward";
  scoped?: boolean;
  input?: string[];
};
type Scenario = {
  columns: number;
  rows: number;
  cwd: string;
  home: string;
  branch: string;
  providerCount: number;
  thinkingLevel?: string;
  subscription?: boolean;
  registryError?: string;
  usage: { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number };
  contextUsage: { percent: number | null; contextWindow: number };
  model: Model<any>;
  models: Model<any>[];
  scopedModels?: Array<{ provider: string; id: string; thinkingLevel?: string }>;
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
setKeybindings(new KeybindingsManager());
initTheme("dark", false);

const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);

const models = scenario.models;
let currentModel: Model<any> = scenario.model;
let scopedModels: Array<{ model: Model<any>; thinkingLevel?: string }> = [];
let thinkingLevel = scenario.thinkingLevel ?? "off";

// core/settings-manager.ts: the defaultThinkingLevel slice as an
// in-memory store (pi-rs's side runs the real settings manager against the
// harness's pinned empty agent dir — both start unset).
let settingsDefaultThinkingLevel: string | undefined;

const findModel = (provider: string, id: string) =>
  models.find((m) => m.provider === provider && m.id === id);

// Registry / settings stubs over scenario data (the registry itself is
// pinned by pi-rs-host tests; this fixture pins presentation and wiring).
const modelRegistry = {
  refresh() {},
  getError: () => scenario.registryError,
  getAvailable: async () => models,
  find: (provider: string, id: string) => findModel(provider, id),
} as unknown as ModelRegistry;
const settingsManager = {
  setDefaultModelAndProvider() {},
  getDefaultThinkingLevel: () => settingsDefaultThinkingLevel,
  setDefaultThinkingLevel(level: string) {
    settingsDefaultThinkingLevel = level;
  },
} as unknown as SettingsManager;

// FooterComponent over stubbed session/provider data.
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
  modelRegistry: { isUsingOAuth: () => scenario.subscription ?? false },
} as unknown as AgentSession;
const footerData = {
  getGitBranch: () => scenario.branch,
  getAvailableProviderCount: () => scenario.providerCount,
  getExtensionStatuses: () => new Map<string, string>(),
} as unknown as ReadonlyFooterDataProvider;
const footer = new FooterComponent(sessionStub, footerData);

const chatContainer = new Container();
const editorContainer = new Container();
let editorValue = "";
const editor = {
  focused: false,
  invalidate() {},
  handleInput(data: string) { if (data === "\r") editorValue = ""; else editorValue += data; },
  render() { return [theme.fg("accent", editorValue) + (editor.focused ? CURSOR_MARKER : "")]; },
};
ui.addChild(chatContainer);
ui.addChild(editorContainer);
editorContainer.addChild(editor);
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

function restoreEditor(): void {
  editorContainer.clear();
  editorContainer.addChild(editor);
  ui.setFocus(editor);
  ui.requestRender();
}

// interactive-mode.ts showSelector.
function showSelector(create: (done: () => void) => { component: unknown; focus: unknown }): void {
  const { component, focus } = create(() => restoreEditor());
  editorContainer.clear();
  editorContainer.addChild(component as never);
  ui.setFocus(focus as never);
  ui.requestRender();
}

// agent-session.ts getAvailableThinkingLevels / supportsThinking /
// setThinkingLevel / _getThinkingLevelForModelSwitch — the model-switch
// re-clamp slice (PLAN 7.2; the cycle surface is pinned by thinking-turn).
function supportsThinking(): boolean {
  return !!currentModel?.reasoning;
}

function setThinkingLevel(level: string): void {
  const availableLevels = getSupportedThinkingLevels(currentModel) as string[];
  const effectiveLevel = availableLevels.includes(level)
    ? level
    : (clampThinkingLevel(currentModel, level as never) as string);
  const isChanging = effectiveLevel !== thinkingLevel;
  thinkingLevel = effectiveLevel;
  if (isChanging) {
    if (supportsThinking() || effectiveLevel !== "off") {
      settingsManager.setDefaultThinkingLevel(effectiveLevel as never);
    }
    // interactive-mode.ts "thinking_level_changed" handler body.
    footer.invalidate();
  }
}

function getThinkingLevelForModelSwitch(explicitLevel?: string): string {
  if (explicitLevel !== undefined) return explicitLevel;
  if (!supportsThinking()) {
    return settingsManager.getDefaultThinkingLevel() ?? "medium";
  }
  return thinkingLevel;
}

// agent-session.ts setModel — the reachable slice (all scenario models
// have configured auth).
function setModel(model: Model<any>, explicitThinkingLevel?: string): void {
  const level = getThinkingLevelForModelSwitch(explicitThinkingLevel);
  currentModel = model;
  footer.invalidate();
  // Re-clamp thinking level for new model's capabilities.
  setThinkingLevel(level);
}

function getModelCandidates(): Model<any>[] {
  if (scopedModels.length > 0) return scopedModels.map((s) => s.model);
  return models;
}

// interactive-mode.ts showModelSelector.
function showModelSelector(initialSearchInput?: string): void {
  showSelector((done) => {
    const selector = new ModelSelectorComponent(
      ui,
      currentModel,
      settingsManager,
      modelRegistry,
      scopedModels,
      async (model) => {
        try {
          setModel(model);
          done();
          showStatus(`Model: ${model.id}`);
        } catch (error) {
          done();
          showError(error instanceof Error ? error.message : String(error));
        }
      },
      () => {
        done();
        ui.requestRender();
      },
      initialSearchInput,
    );
    return { component: selector, focus: selector };
  });
}

// interactive-mode.ts handleModelCommand.
function handleModelCommand(searchTerm?: string): void {
  if (!searchTerm) {
    showModelSelector();
    return;
  }
  const model = findExactModelReferenceMatch(searchTerm, getModelCandidates());
  if (model) {
    setModel(model);
    showStatus(`Model: ${model.id}`);
    return;
  }
  showModelSelector(searchTerm);
}

// agent-session.ts cycleModel + interactive-mode.ts cycleModel status rows.
function cycleModel(direction: "forward" | "backward"): void {
  const candidates = getModelCandidates();
  if (candidates.length <= 1) {
    showStatus(scopedModels.length > 0 ? "Only one model in scope" : "Only one model available");
    return;
  }
  let currentIndex = candidates.findIndex((m) => modelsAreEqual(m, currentModel));
  if (currentIndex === -1) currentIndex = 0;
  const len = candidates.length;
  const nextIndex = direction === "forward" ? (currentIndex + 1) % len : (currentIndex - 1 + len) % len;
  const next = candidates[nextIndex]!;
  // _cycleScopedModel: an explicit scoped thinking level overrides the
  // session preference; _cycleAvailableModel re-clamps the current one.
  const explicitLevel = scopedModels.length > 0 ? scopedModels[nextIndex]?.thinkingLevel : undefined;
  setModel(next, explicitLevel);
  const thinkingStr = next.reasoning && thinkingLevel !== "off" ? ` (thinking: ${thinkingLevel})` : "";
  showStatus(`Switched to ${next.name || next.id}${thinkingStr}`);
}

const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}

async function main() {
  await capture("startup", true);
  for (const step of scenario.steps) {
    if (step.scoped !== undefined) {
      scopedModels = step.scoped
        ? (scenario.scopedModels ?? []).map((ref) => ({
            model: findModel(ref.provider, ref.id)!,
            thinkingLevel: ref.thinkingLevel,
          }))
        : [];
    }
    if (step.show) showModelSelector(step.search);
    if (step.command !== undefined) handleModelCommand(step.command);
    if (step.cycle) cycleModel(step.cycle);
    for (const data of step.input ?? []) {
      terminal.send(data);
      // Let promise continuations (select -> async onSelect) settle.
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
    await capture(step.name);
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
