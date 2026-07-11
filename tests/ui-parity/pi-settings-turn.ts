import { readFileSync } from "node:fs";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { SettingsManager } from "../../ref/pi/packages/coding-agent/src/core/settings-manager.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { SettingsSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/settings-selector.ts";
import {
  getAvailableThemes,
  getEditorTheme,
  initTheme,
  setTheme,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

setCapabilities({ images: null, trueColor: true, hyperlinks: false });
type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
type Step = { name: string; input?: string[]; resize?: { columns: number; rows: number } };
type Scenario = { columns: number; rows: number; model: { reasoning?: boolean }; thinkingLevel?: ThinkingLevel; steps: Step[] };

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
setKeybindings(keybindings);
initTheme("dark", false);
const settings = SettingsManager.inMemory();
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);
const editorContainer = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, { paddingX: 0, autocompleteMaxVisible: 5 });
editorContainer.addChild(editor);
ui.addChild(editorContainer);
ui.setFocus(editor);
ui.start();

let thinkingLevel = scenario.thinkingLevel ?? "off";
let hideThinkingBlock = settings.getHideThinkingBlock();
const levels: ThinkingLevel[] = scenario.model.reasoning
  ? ["off", "minimal", "low", "medium", "high", "xhigh"]
  : ["off"];

function showSettingsSelector(): void {
  const done = () => {
    editorContainer.clear(); editorContainer.addChild(editor); ui.setFocus(editor); ui.requestRender();
  };
  const selector = new SettingsSelectorComponent({
    autoCompact: settings.getCompactionEnabled(), showImages: settings.getShowImages(),
    imageWidthCells: settings.getImageWidthCells(), autoResizeImages: settings.getImageAutoResize(),
    blockImages: settings.getBlockImages(), enableSkillCommands: settings.getEnableSkillCommands(),
    steeringMode: settings.getSteeringMode(), followUpMode: settings.getFollowUpMode(),
    transport: settings.getTransport(), httpIdleTimeoutMs: settings.getHttpIdleTimeoutMs(),
    thinkingLevel, availableThinkingLevels: levels, currentTheme: settings.getTheme() || "dark",
    availableThemes: getAvailableThemes(), hideThinkingBlock, collapseChangelog: settings.getCollapseChangelog(),
    enableInstallTelemetry: settings.getEnableInstallTelemetry(), doubleEscapeAction: settings.getDoubleEscapeAction(),
    treeFilterMode: settings.getTreeFilterMode(), showHardwareCursor: settings.getShowHardwareCursor(),
    editorPaddingX: settings.getEditorPaddingX(), autocompleteMaxVisible: settings.getAutocompleteMaxVisible(),
    quietStartup: settings.getQuietStartup(), defaultProjectTrust: settings.getDefaultProjectTrust(),
    clearOnShrink: settings.getClearOnShrink(), showTerminalProgress: settings.getShowTerminalProgress(),
    warnings: settings.getWarnings(),
  }, {
    onAutoCompactChange: (value) => settings.setCompactionEnabled(value),
    onShowImagesChange: (value) => settings.setShowImages(value),
    onImageWidthCellsChange: (value) => settings.setImageWidthCells(value),
    onAutoResizeImagesChange: (value) => settings.setImageAutoResize(value),
    onBlockImagesChange: (value) => settings.setBlockImages(value),
    onEnableSkillCommandsChange: (value) => settings.setEnableSkillCommands(value),
    onSteeringModeChange: (value) => settings.setSteeringMode(value),
    onFollowUpModeChange: (value) => settings.setFollowUpMode(value),
    onTransportChange: (value) => settings.setTransport(value),
    onHttpIdleTimeoutMsChange: (value) => settings.setHttpIdleTimeoutMs(value),
    onThinkingLevelChange: (value) => { thinkingLevel = value; settings.setDefaultThinkingLevel(value); },
    onThemeChange: (name) => { setTheme(name, false); settings.setTheme(name); ui.invalidate(); },
    onThemePreview: (name) => { setTheme(name, false); ui.invalidate(); ui.requestRender(); },
    onHideThinkingBlockChange: (value) => { hideThinkingBlock = value; settings.setHideThinkingBlock(value); },
    onCollapseChangelogChange: (value) => settings.setCollapseChangelog(value),
    onEnableInstallTelemetryChange: (value) => settings.setEnableInstallTelemetry(value),
    onDoubleEscapeActionChange: (value) => settings.setDoubleEscapeAction(value),
    onTreeFilterModeChange: (value) => settings.setTreeFilterMode(value),
    onShowHardwareCursorChange: (value) => { settings.setShowHardwareCursor(value); ui.setShowHardwareCursor(value); },
    onEditorPaddingXChange: (value) => { settings.setEditorPaddingX(value); editor.setPaddingX(value); },
    onAutocompleteMaxVisibleChange: (value) => { settings.setAutocompleteMaxVisible(value); editor.setAutocompleteMaxVisible(value); },
    onQuietStartupChange: (value) => settings.setQuietStartup(value),
    onDefaultProjectTrustChange: (value) => settings.setDefaultProjectTrust(value),
    onClearOnShrinkChange: (value) => { settings.setClearOnShrink(value); ui.setClearOnShrink(value); },
    onShowTerminalProgressChange: (value) => settings.setShowTerminalProgress(value),
    onWarningsChange: (value) => settings.setWarnings(value), onCancel: done,
  });
  editorContainer.clear(); editorContainer.addChild(selector); ui.setFocus(selector.getSettingsList()); ui.requestRender();
}

editor.onSubmit = (text: string) => {
  text = text.trim(); if (!text) return;
  if (text === "/settings") { showSettingsSelector(); editor.setText(""); }
};

const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force); await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}
async function main() {
  await capture("startup", true);
  for (const step of scenario.steps) {
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    for (const data of step.input ?? []) terminal.send(data);
    await capture(step.name, Boolean(step.resize));
  }
  ui.stop(); process.stdout.write(JSON.stringify({ frames }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
