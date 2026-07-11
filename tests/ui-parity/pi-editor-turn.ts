import { readFileSync } from "node:fs";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = { name: string; input?: string[]; resize?: { columns: number; rows: number } };
type Scenario = { columns: number; rows: number; steps: Step[] };

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
initTheme("dark", false);
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);

// Submission scaffold: one dim JSON row per recorded submission, mirroring
// interactive-mode's normal path (trim, skip empty, addToHistory). Both
// drivers construct the identical row; the pinned cells are the editor's.
const submitted = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, {
  paddingX: 0,
  autocompleteMaxVisible: 5,
});
editor.onSubmit = (text: string) => {
  text = text.trim();
  if (!text) return;
  submitted.addChild(new Text(theme.fg("dim", JSON.stringify(text)), 0, 0));
  editor.addToHistory(text);
};
ui.addChild(submitted);
ui.addChild(editor);
ui.setFocus(editor);
ui.start();

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
