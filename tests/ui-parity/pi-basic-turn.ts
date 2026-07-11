import { readFileSync } from "node:fs";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { AssistantMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/assistant-message.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { UserMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/user-message.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Loader } from "../../ref/pi/packages/tui/src/components/loader.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Resize = { columns: number; rows: number; name?: string };
type Scenario = { columns: number; rows: number; resize?: Resize; resizes?: Resize[]; input: string[]; prompt: string; partial: string; completion: string };
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
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);
const header = new Text(theme.bold(theme.fg("accent", "pi")) + theme.fg("dim", " v0.79.0"), 1, 0);
const transcript = new Container(); const status = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, { paddingX: 0, autocompleteMaxVisible: 5 });
editor.onSubmit = () => {};
const footer = new Text(theme.fg("dim", "/work/project (main)"), 0, 0);
ui.addChild(header); ui.addChild(new Spacer(1)); ui.addChild(transcript); ui.addChild(status);
ui.addChild(new Spacer(1)); ui.addChild(editor); ui.addChild(footer); ui.setFocus(editor); ui.start();
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force); await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}
const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
const message = (text: string) => ({ role: "assistant" as const, content: [{ type: "text" as const, text }], api: "faux", provider: "faux", model: "faux-1", usage, stopReason: "stop" as const, timestamp: 0 });
async function main() {
await capture("startup", true);
for (const input of scenario.input) terminal.send(input);
transcript.addChild(new UserMessageComponent(scenario.prompt));
// interactive-mode.ts agent_start: the working Loader in the status
// container (stopped so the spinner stays deterministically at frame 0).
const loader = new Loader(ui, (s) => theme.fg("accent", s), (t) => theme.fg("muted", t), "Working...");
loader.stop();
status.addChild(loader);
await capture("submitted"); status.clear();
const assistant = new AssistantMessageComponent(message(scenario.partial)); transcript.addChild(assistant);
await capture("streaming"); assistant.updateContent(message(scenario.completion)); await capture("complete");
for (const resize of scenario.resizes ?? (scenario.resize ? [scenario.resize] : [])) {
	terminal.resize(resize.columns, resize.rows); await capture(resize.name ?? "resize", true);
}
ui.stop();
process.stdout.write(JSON.stringify({ frames }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
