import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { ProjectTrustStore } from "../../ref/pi/packages/coding-agent/src/core/trust-manager.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { TrustSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/trust-selector.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

setCapabilities({ images: null, trueColor: true, hyperlinks: false });
type Step = { name: string; show?: boolean; input?: string[]; resize?: { columns: number; rows: number } };
type Scenario = { columns: number; rows: number; cwd: string; steps: Step[] };
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
const agentDir = mkdtempSync(join(tmpdir(), "pi-trust-turn-"));
const trustStore = new ProjectTrustStore(agentDir);
const keybindings = new KeybindingsManager(); setKeybindings(keybindings); initTheme("dark", false);
const terminal = new CaptureTerminal(scenario.columns, scenario.rows); const ui = new TUI(terminal, true);
const root = new Container(); const chatContainer = new Container(); const editorContainer = new Container(); const editor = new CustomEditor(ui, getEditorTheme(), keybindings, { paddingX: 0, autocompleteMaxVisible: 5 });
chatContainer.addChild(new Text(theme.fg("warning", "This project is not trusted. Project .pi resources and packages are ignored. Use /trust to save a trust decision, then restart pi."), 1, 0));
root.addChild(chatContainer); editorContainer.addChild(editor); root.addChild(editorContainer); ui.addChild(root); ui.setFocus(editor); ui.start();
function restoreEditor(): void { editorContainer.clear(); editorContainer.addChild(editor); ui.setFocus(editor); ui.requestRender(); }
let statusText: Text | undefined;
function showStatus(message: string): void {
  if (statusText) statusText.setText(theme.fg("dim", message));
  else { chatContainer.addChild(new Spacer(1)); statusText = new Text(theme.fg("dim", message), 1, 0); chatContainer.addChild(statusText); }
}
function showTrustSelector(): void {
  const selector = new TrustSelectorComponent({ cwd: scenario.cwd, savedDecision: trustStore.getEntry(scenario.cwd), projectTrusted: false,
    onSelect: (selection) => { trustStore.setMany(selection.updates); restoreEditor(); showStatus(`Saved trust decision: ${selection.trusted ? "trusted" : "untrusted"}. Restart pi for this to take effect.`); }, onCancel: restoreEditor });
  editorContainer.clear(); editorContainer.addChild(selector); ui.setFocus(selector); ui.requestRender();
}
editor.onSubmit = (text: string) => { if (text.trim() === "/trust") { showTrustSelector(); editor.setText(""); } };
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) { ui.requestRender(force); await new Promise<void>((resolve) => setTimeout(resolve, 20)); frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() }); }
async function main() {
  for (const step of scenario.steps) {
    if (step.show) showTrustSelector();
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    for (const data of step.input ?? []) { terminal.send(data); await new Promise<void>((resolve) => setTimeout(resolve, 0)); }
    await capture(step.name, step.name === "startup" || Boolean(step.resize));
  }
  ui.stop(); rmSync(agentDir, { recursive: true, force: true }); process.stdout.write(JSON.stringify({ frames }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
