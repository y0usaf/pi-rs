import { readFileSync } from "node:fs";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import commandsExtension from "../../ref/pi/packages/coding-agent/examples/extensions/commands.ts";
import permissionGate from "../../ref/pi/packages/coding-agent/examples/extensions/permission-gate.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { ExtensionSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/extension-selector.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

setCapabilities({ images: null, trueColor: true, hyperlinks: false });
type Step = { name: string; submit?: string; permission?: string; input?: string[]; resize?: { columns: number; rows: number } };
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
const keybindings = new KeybindingsManager(); setKeybindings(keybindings); initTheme("dark", false);
const terminal = new CaptureTerminal(scenario.columns, scenario.rows); const ui = new TUI(terminal, true);
const root = new Container(); const chatContainer = new Container(); const editorContainer = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, { paddingX: 0, autocompleteMaxVisible: 5 });
root.addChild(chatContainer); editorContainer.addChild(editor); root.addChild(editorContainer); ui.addChild(root); ui.setFocus(editor); ui.start();
let selector: ExtensionSelectorComponent | undefined; let statusText: Text | undefined;
function restoreEditor(): void { editorContainer.clear(); editorContainer.addChild(editor); selector = undefined; ui.setFocus(editor); ui.requestRender(); }
function showStatus(message: string): void {
  if (statusText) statusText.setText(theme.fg("dim", message));
  else { chatContainer.addChild(new Spacer(1)); statusText = new Text(theme.fg("dim", message), 1, 0); chatContainer.addChild(statusText); }
}
const actions: any[] = [];
function dialog(kind: "select" | "confirm", title: string, options: string[]): Promise<string | undefined> {
  actions.push({ type: kind, title, options });
  return new Promise((resolve) => {
    const finish = (value: string | undefined) => {
      actions.push({ type: `${kind}_result`, ...(value === undefined ? {} : { value }) });
      restoreEditor(); resolve(value);
    };
    selector = new ExtensionSelectorComponent(title, options, finish, () => finish(undefined), { tui: ui });
    editorContainer.clear(); editorContainer.addChild(selector); ui.setFocus(selector); ui.requestRender();
  });
}
const extensionUi = {
  select: (title: string, options: string[]) => dialog("select", title, options),
  confirm: async (title: string, message: string) => (await dialog("confirm", `${title}\n${message}`, ["Yes", "No"])) === "Yes",
  notify: (message: string, level?: string) => { actions.push({ type: "notify", message, level: level ?? null }); showStatus(message); },
};
let command: any; let toolCallHandler: any;
const commandInfo = { name: "commands", description: "List available slash commands", source: "extension", sourceInfo: { path: "examples/extensions/commands.lua", source: "cli", scope: "temporary", origin: "top-level" } };
commandsExtension({ registerCommand: (_name: string, definition: any) => { command = definition; }, getCommands: () => [commandInfo] } as any);
permissionGate({ on: (event: string, handler: any) => { if (event === "tool_call") toolCallHandler = handler; } } as any);
const context = { mode: "tui", hasUI: true, cwd: process.cwd(), ui: extensionUi } as any;
let permissionResult: any;
editor.onSubmit = (text: string) => {
  editor.setText("");
  if (text === "/commands extension") void command.handler("extension", context);
};
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function settle(): Promise<void> { await new Promise<void>((resolve) => setTimeout(resolve, 15)); }
async function capture(name: string, force = false) { ui.requestRender(force); await settle(); frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() }); }
async function main() {
  for (const step of scenario.steps) {
    if (step.submit) { editor.handleInput(`\x1b[200~${step.submit}\x1b[201~`); editor.handleInput("\r"); await settle(); }
    if (step.permission) { void toolCallHandler({ toolName: "bash", input: { command: step.permission } }, context).then((value: any) => { permissionResult = value; }); await settle(); }
    for (const data of step.input ?? []) { (selector ?? editor).handleInput(data); await settle(); }
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    await capture(step.name, step.name === "startup" || Boolean(step.resize));
  }
  ui.stop();
  const output = { frames, permissionResult, actions };
  process.stdout.write(process.env.EXTENSION_UI_ACTIONS_ONLY
    ? `${JSON.stringify({ permissionResult, actions }, null, 2)}\n`
    : JSON.stringify(output));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
