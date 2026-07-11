import { readFileSync } from "node:fs";
import type { Model } from "../../ref/pi/packages/ai/src/types.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { resolveModelScope } from "../../ref/pi/packages/coding-agent/src/core/model-resolver.ts";
import type { ModelRegistry } from "../../ref/pi/packages/coding-agent/src/core/model-registry.ts";
import { SettingsManager } from "../../ref/pi/packages/coding-agent/src/core/settings-manager.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { ScopedModelsSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts";
import { getEditorTheme, initTheme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

setCapabilities({ images: null, trueColor: true, hyperlinks: false });
type Step = { name: string; show?: boolean; cycle?: "forward" | "backward"; input?: string[]; resize?: { columns: number; rows: number } };
type Scenario = { columns: number; rows: number; models: Model<any>[]; steps: Step[] };

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
const keybindings = new KeybindingsManager();
setKeybindings(keybindings); initTheme("dark", false);
const settings = SettingsManager.inMemory();
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);
const editorContainer = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, { paddingX: 0, autocompleteMaxVisible: 5 });
editorContainer.addChild(editor); ui.addChild(editorContainer); ui.setFocus(editor); ui.start();

const allModels = scenario.models;
let scopedModels: Array<{ model: Model<any>; thinkingLevel?: string }> = [];
let currentModel = allModels[0]!;
const modelRegistry = {
  refresh() {},
  getAvailable: async () => allModels,
} as unknown as ModelRegistry;

function restoreEditor(): void {
  editorContainer.clear(); editorContainer.addChild(editor); ui.setFocus(editor); ui.requestRender();
}
async function showModelsSelector(): Promise<void> {
  modelRegistry.refresh();
  const models = await modelRegistry.getAvailable();
  let currentEnabledIds: string[] | null = null;
  if (scopedModels.length > 0) {
    currentEnabledIds = scopedModels.map((scoped) => `${scoped.model.provider}/${scoped.model.id}`);
  } else {
    const patterns = settings.getEnabledModels();
    if (patterns !== undefined && patterns.length > 0) {
      const resolved = await resolveModelScope(patterns, modelRegistry);
      currentEnabledIds = resolved.map((scoped) => `${scoped.model.provider}/${scoped.model.id}`);
    }
  }
  const update = async (enabledIds: string[] | null) => {
    if (enabledIds && enabledIds.length > 0 && enabledIds.length < models.length) {
      scopedModels = (await resolveModelScope(enabledIds, modelRegistry)).map((item) => ({
        model: item.model, thinkingLevel: item.thinkingLevel,
      }));
    } else scopedModels = [];
    ui.requestRender();
  };
  const selector = new ScopedModelsSelectorComponent({ allModels: models, enabledModelIds: currentEnabledIds }, {
    onChange: update,
    onPersist: (enabledIds) => settings.setEnabledModels(
      enabledIds === null || enabledIds.length === models.length ? undefined : [...enabledIds],
    ),
    onCancel: restoreEditor,
  });
  editorContainer.clear(); editorContainer.addChild(selector); ui.setFocus(selector); ui.requestRender();
}
editor.onSubmit = (text: string) => {
  if (text.trim() === "/scoped-models") { void showModelsSelector(); editor.setText(""); }
};

const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force); await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}
async function main() {
  for (const step of scenario.steps) {
    if (step.show) await showModelsSelector();
    if (step.cycle && scopedModels.length > 1) {
      let index = scopedModels.findIndex((item) => item.model.provider === currentModel.provider && item.model.id === currentModel.id);
      if (index < 0) index = 0;
      index = step.cycle === "forward" ? (index + 1) % scopedModels.length
        : (index - 1 + scopedModels.length) % scopedModels.length;
      currentModel = scopedModels[index]!.model;
    }
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    for (const data of step.input ?? []) { terminal.send(data); await new Promise<void>((resolve) => setTimeout(resolve, 0)); }
    await capture(step.name, step.name === "startup" || Boolean(step.resize));
  }
  ui.stop(); process.stdout.write(JSON.stringify({ frames, currentModel: `${currentModel.provider}/${currentModel.id}` }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
