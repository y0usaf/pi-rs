import { readFileSync } from "node:fs";
import type { AuthStatus, AuthStorage } from "../../ref/pi/packages/coding-agent/src/core/auth-storage.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import {
  type AuthSelectorProvider,
  OAuthSelectorComponent,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/oauth-selector.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = { name: string; show?: boolean; input?: string[] };
type Scenario = {
  columns: number;
  rows: number;
  mode: "login" | "logout";
  providers: AuthSelectorProvider[];
  credentials?: Record<string, { type: "oauth" | "api_key" }>;
  authStatus?: Record<string, AuthStatus>;
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
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
initTheme("dark", false);
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);

// The real CustomEditor in the editor slot; the selector swap moves focus
// away from it and done() moves focus back.
const editorContainer = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, { paddingX: 0, autocompleteMaxVisible: 5 });
editor.onSubmit = () => {};
ui.addChild(editorContainer);
editorContainer.addChild(editor);
ui.setFocus(editor);
ui.start();

const authStorage = { get: (id: string) => scenario.credentials?.[id] } as unknown as AuthStorage;
const getAuthStatus = (id: string): AuthStatus => scenario.authStatus?.[id] ?? { configured: false };

// interactive-mode.ts showSelector/showOAuthSelector over the editor slot.
function showSelector(): void {
  const done = () => {
    editorContainer.clear();
    editorContainer.addChild(editor);
    ui.setFocus(editor);
  };
  const selector = new OAuthSelectorComponent(
    scenario.mode,
    authStorage,
    scenario.providers,
    () => done(),
    () => done(),
    getAuthStatus,
  );
  editorContainer.clear();
  editorContainer.addChild(selector);
  ui.setFocus(selector);
  ui.requestRender();
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
    if (step.show) showSelector();
    for (const data of step.input ?? []) terminal.send(data);
    await capture(step.name);
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
