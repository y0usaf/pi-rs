// /reload driver: Pi's real component composition around the copied
// handleReloadCommand guard/settlement policy.
import { readFileSync } from "node:fs";
import { DynamicBorder } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/dynamic-border.ts";
import { initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = { name: string; streaming?: boolean; compacting?: boolean; phase?: "loading"; fail?: boolean; force?: boolean };
type Scenario = { columns: number; rows: number; steps: Step[] };

class CaptureTerminal implements Terminal {
  private resized?: () => void; private chunks: string[] = [];
  kittyProtocolActive = true;
  constructor(public columns: number, public rows: number) {}
  start(_input: (data: string) => void, resized: () => void): void { this.resized = resized; }
  async drainInput(): Promise<void> {} stop(): void {}
  write(data: string): void { this.chunks.push(data); }
  moveBy(lines: number): void { if (lines > 0) this.write(`\x1b[${lines}B`); else if (lines < 0) this.write(`\x1b[${-lines}A`); }
  hideCursor(): void { this.write("\x1b[?25l"); } showCursor(): void { this.write("\x1b[?25h"); }
  clearLine(): void { this.write("\x1b[K"); } clearFromCursor(): void { this.write("\x1b[J"); }
  clearScreen(): void { this.write("\x1b[2J\x1b[H"); } setTitle(): void {} setProgress(): void {}
  take(): string { const result = this.chunks.join(""); this.chunks = []; return result; }
}

const scenario = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Scenario;
initTheme("dark", false);
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);
const root = new Container();
ui.addChild(root);
ui.start();
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];

function showWarning(message: string): void {
  root.addChild(new Spacer(1));
  root.addChild(new Text(theme.fg("warning", `Warning: ${message}`), 1, 0));
}
function showStatus(message: string): void {
  root.addChild(new Spacer(1));
  root.addChild(new Text(theme.fg("dim", message), 1, 0));
}
function showError(message: string): void {
  root.addChild(new Spacer(1));
  root.addChild(new Text(theme.fg("error", `Error: ${message}`), 1, 0));
  root.addChild(new Spacer(1));
}
function mountReloadBox(): void {
  root.addChild(new DynamicBorder((text) => theme.fg("border", text)));
  root.addChild(new Spacer(1));
  root.addChild(new Text(theme.fg("muted", "Reloading keybindings, extensions, skills, prompts, themes..."), 1, 0));
  root.addChild(new Spacer(1));
  root.addChild(new DynamicBorder((text) => theme.fg("border", text)));
}

async function main() {
  for (const step of scenario.steps) {
    root.clear();
    if (step.streaming) showWarning("Wait for the current response to finish before reloading.");
    else if (step.compacting) showWarning("Wait for compaction to finish before reloading.");
    else if (step.phase === "loading") mountReloadBox();
    else if (step.fail) showError("Reload failed: scripted reload failure");
    else showStatus("Reloaded keybindings, extensions, skills, prompts, themes");
    ui.requestRender(step.force ?? false);
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
    frames.push({ name: step.name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
