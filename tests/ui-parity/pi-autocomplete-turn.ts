import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { BUILTIN_SLASH_COMMANDS } from "../../ref/pi/packages/coding-agent/src/core/slash-commands.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { CombinedAutocompleteProvider, type AutocompleteItem, type SlashCommand } from "../../ref/pi/packages/tui/src/autocomplete.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { fuzzyFilter } from "../../ref/pi/packages/tui/src/fuzzy.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = { name: string; input?: string[]; waitMs?: number; resize?: { columns: number; rows: number } };
type Scenario = {
  columns: number;
  rows: number;
  files?: Record<string, string>;
  fdPath?: string;
  models?: Array<{ id: string; provider: string }>;
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
initTheme("dark", false);
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);

// Scenario tree in a fresh temp cwd, mirroring the pi-rs harness's `files`
// handling; nested paths create their parents.
const cwd = mkdtempSync(join(tmpdir(), "pi-rs-ui-parity-"));
for (const [name, contents] of Object.entries(scenario.files ?? {})) {
  const path = join(cwd, name);
  if (name.endsWith("/")) {
    mkdirSync(path, { recursive: true });
  } else {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, contents);
  }
}

// interactive-mode.ts createBaseAutocompleteProvider wiring body: builtin
// commands plus /model argument completions over the scenario's pinned
// model list (fuzzy over "<id> <provider>").
const slashCommands: SlashCommand[] = BUILTIN_SLASH_COMMANDS.map((command) => ({
  name: command.name,
  description: command.description,
}));
const modelCommand = slashCommands.find((command) => command.name === "model");
if (modelCommand) {
  modelCommand.getArgumentCompletions = (prefix: string): AutocompleteItem[] | null => {
    const models = scenario.models ?? [];
    if (models.length === 0) return null;
    const items = models.map((m) => ({
      id: m.id,
      provider: m.provider,
      label: `${m.provider}/${m.id}`,
    }));
    const filtered = fuzzyFilter(items, prefix, (item) => `${item.id} ${item.provider}`);
    if (filtered.length === 0) return null;
    return filtered.map((item) => ({
      value: item.label,
      label: item.id,
      description: item.provider,
    }));
  };
}
const fdPath = scenario.fdPath ? resolve(scenario.fdPath) : null;
const provider = new CombinedAutocompleteProvider(slashCommands, cwd, fdPath);

// Submission scaffold: one dim JSON row per recorded submission, mirroring
// interactive-mode's normal path (trim, skip empty, addToHistory). Both
// drivers construct the identical row; the pinned cells are the editor's.
const submitted = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, {
  paddingX: 0,
  autocompleteMaxVisible: 5,
});
editor.setAutocompleteProvider(provider);
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
async function capture(name: string, waitMs: number, force = false) {
  ui.requestRender(force);
  await new Promise<void>((resolve) => setTimeout(resolve, waitMs));
  // Debounced/async suggestion arrivals request their own render; flush it
  // into the same capture.
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}

async function main() {
  await capture("startup", 20, true);
  for (const step of scenario.steps) {
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    for (const data of step.input ?? []) {
      terminal.send(data);
      // Let each keystroke's debounce timer and provider request settle
      // before the next, matching the pi-rs driver's per-input pump.
      await new Promise<void>((resolve) => setTimeout(resolve, step.waitMs ?? 30));
    }
    await capture(step.name, step.waitMs ?? 20, Boolean(step.resize));
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
