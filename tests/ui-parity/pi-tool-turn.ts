import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { ToolExecutionComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/tool-execution.ts";
import { initTheme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type ToolResult = { content: Array<{ type: string; text?: string }>; details?: unknown; isError?: boolean };
type ScenarioTool = { id: string; name: string; args: unknown; partialResult?: ToolResult; result?: ToolResult };
type Section = { name: string; clocks?: { pending?: number; partial?: number; results?: number }; tools: ScenarioTool[] };
type Scenario = { columns: number; rows: number; files?: Record<string, string>; sections: Section[] };

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
// Deterministic environment: fixed capabilities (no hyperlinks/images) and
// a scripted clock standing in for Date.now, mirrored by the Lua driver.
setCapabilities({ images: null, trueColor: true, hyperlinks: false });
// The interactive session installs the coding-agent keybinding definitions
// (interactive-mode.ts setKeybindings); the oracle uses the defaults.
setKeybindings(new KeybindingsManager());
let clock = 0;
Date.now = () => clock;
initTheme("dark", false);

const cwd = mkdtempSync(join(tmpdir(), "pi-rs-ui-parity-"));
for (const [name, contents] of Object.entries(scenario.files ?? {})) {
  writeFileSync(join(cwd, name), contents);
}

const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);
const transcript = new Container();
ui.addChild(transcript);
ui.start();

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force); await sleep(20);
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}

async function main() {
  let first = true;
  for (const section of scenario.sections) {
    transcript.clear();
    clock = section.clocks?.pending ?? clock;
    const components: ToolExecutionComponent[] = [];
    for (const tool of section.tools) {
      const component = new ToolExecutionComponent(tool.name, tool.id, tool.args, {}, undefined, ui, cwd);
      component.markExecutionStarted();
      component.setArgsComplete();
      components.push(component);
      transcript.addChild(component);
    }
    // Let async call previews (edit's computeEditsDiff) settle.
    await sleep(30);
    await capture(`${section.name}-pending`, first);
    first = false;
    if (section.tools.some((tool) => tool.partialResult)) {
      clock = section.clocks?.partial ?? clock;
      section.tools.forEach((tool, index) => {
        if (tool.partialResult) {
          components[index]!.updateResult(
            { content: tool.partialResult.content as any, details: tool.partialResult.details, isError: false },
            true,
          );
        }
      });
      await capture(`${section.name}-partial`);
    }
    clock = section.clocks?.results ?? clock;
    section.tools.forEach((tool, index) => {
      if (tool.result) {
        components[index]!.updateResult(
          { content: tool.result.content as any, details: tool.result.details, isError: tool.result.isError ?? false },
          false,
        );
      }
    });
    await capture(`${section.name}-results`);
    for (const component of components) component.setExpanded(true);
    await capture(`${section.name}-expanded`);
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
