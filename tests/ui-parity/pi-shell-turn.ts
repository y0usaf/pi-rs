import { readFileSync } from "node:fs";
import type { Model } from "../../ref/pi/packages/ai/src/models.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import { AssistantMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/assistant-message.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import {
  keyHint,
  keyText,
  keyDisplayText,
  rawKeyHint,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts";
import { UserMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/user-message.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Loader } from "../../ref/pi/packages/tui/src/components/loader.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { TruncatedText } from "../../ref/pi/packages/tui/src/components/truncated-text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { type KeyId, matchesKey } from "../../ref/pi/packages/tui/src/keys.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = { name: string; input?: string[]; resize?: { columns: number; rows: number } };
type Scenario = {
  columns: number;
  rows: number;
  appName: string;
  version: string;
  cwd: string;
  home: string;
  branch: string;
  providerCount: number;
  doubleEscapeAction: "fork" | "tree" | "none";
  clipboardImagePath?: string;
  shortcut?: { key: string; status: string };
  usage: { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number };
  contextUsage: { percent: number | null; contextWindow: number };
  model: Model<any>;
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
process.env.HOME = scenario.home;
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
initTheme("dark", false);

const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);

// interactive-mode.ts ExpandableText (private helper class).
class ExpandableText extends Text {
  private readonly getCollapsedText: () => string;
  private readonly getExpandedText: () => string;
  constructor(getCollapsedText: () => string, getExpandedText: () => string, expanded = false, paddingX = 0, paddingY = 0) {
    super(expanded ? getExpandedText() : getCollapsedText(), paddingX, paddingY);
    this.getCollapsedText = getCollapsedText;
    this.getExpandedText = getExpandedText;
  }
  setExpanded(expanded: boolean): void {
    this.setText(expanded ? this.getExpandedText() : this.getCollapsedText());
  }
}

// interactive-mode.ts init(): the startup header built from keybinding hints.
const logo = theme.bold(theme.fg("accent", scenario.appName)) + theme.fg("dim", ` v${scenario.version}`);
const hint = keyHint;
const expandedInstructions = [
  hint("app.interrupt", "to interrupt"),
  hint("app.clear", "to clear"),
  rawKeyHint(`${keyText("app.clear")} twice`, "to exit"),
  hint("app.exit", "to exit (empty)"),
  hint("app.suspend", "to suspend"),
  keyHint("tui.editor.deleteToLineEnd", "to delete to end"),
  hint("app.thinking.cycle", "to cycle thinking level"),
  rawKeyHint(`${keyText("app.model.cycleForward")}/${keyText("app.model.cycleBackward")}`, "to cycle models"),
  hint("app.model.select", "to select model"),
  hint("app.tools.expand", "to expand tools"),
  hint("app.thinking.toggle", "to expand thinking"),
  hint("app.editor.external", "for external editor"),
  rawKeyHint("/", "for commands"),
  rawKeyHint("!", "to run bash"),
  rawKeyHint("!!", "to run bash (no context)"),
  hint("app.message.followUp", "to queue follow-up"),
  hint("app.message.dequeue", "to edit all queued messages"),
  hint("app.clipboard.pasteImage", "to paste image"),
  rawKeyHint("drop files", "to attach"),
].join("\n");
const compactInstructions = [
  hint("app.interrupt", "interrupt"),
  rawKeyHint(`${keyText("app.clear")}/${keyText("app.exit")}`, "clear/exit"),
  rawKeyHint("/", "commands"),
  rawKeyHint("!", "bash"),
  hint("app.tools.expand", "more"),
].join(theme.fg("muted", " · "));
const compactOnboarding = theme.fg(
  "dim",
  `Press ${keyText("app.tools.expand")} to show full startup help and loaded resources.`,
);
const onboarding = theme.fg(
  "dim",
  `Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.`,
);
const builtInHeader = new ExpandableText(
  () => `${logo}\n${compactInstructions}\n${compactOnboarding}\n\n${onboarding}`,
  () => `${logo}\n${expandedInstructions}\n\n${onboarding}`,
  false,
  1,
  0,
);

// interactive-mode.ts init() ui.addChild composition.
const headerContainer = new Container();
headerContainer.addChild(new Spacer(1));
headerContainer.addChild(builtInHeader);
headerContainer.addChild(new Spacer(1));
const chatContainer = new Container();
const pendingMessagesContainer = new Container();
const statusContainer = new Container();
const widgetContainerAbove = new Container();
widgetContainerAbove.addChild(new Spacer(1)); // renderWidgets' spacer-when-empty default
const widgetContainerBelow = new Container();
const editorContainer = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, {
  paddingX: 0,
  autocompleteMaxVisible: 5,
});
editorContainer.addChild(editor);

// FooterComponent over stubbed session/provider data (as in pi-model-turn.ts).
const sessionStub = {
  get state() {
    return { model: scenario.model, thinkingLevel: "off" } as never;
  },
  sessionManager: {
    getEntries: () => [
      {
        type: "message",
        message: {
          role: "assistant",
          usage: {
            input: scenario.usage.input,
            output: scenario.usage.output,
            cacheRead: scenario.usage.cacheRead,
            cacheWrite: scenario.usage.cacheWrite,
            cost: { total: scenario.usage.cost },
          },
        },
      },
    ],
    getCwd: () => scenario.cwd,
    getSessionName: () => undefined,
  },
  getContextUsage: () => scenario.contextUsage,
  modelRegistry: { isUsingOAuth: () => false },
} as unknown as AgentSession;
const footerData = {
  getGitBranch: () => scenario.branch,
  getAvailableProviderCount: () => scenario.providerCount,
  getExtensionStatuses: () => new Map<string, string>(),
} as unknown as ReadonlyFooterDataProvider;
const footer = new FooterComponent(sessionStub, footerData);

ui.addChild(headerContainer);
ui.addChild(chatContainer);
ui.addChild(pendingMessagesContainer);
ui.addChild(statusContainer);
ui.addChild(widgetContainerAbove);
ui.addChild(editorContainer);
ui.addChild(widgetContainerBelow);
ui.addChild(footer);
ui.setFocus(editor);
ui.start();

// Scripted session state; streaming toggles through the copied wiring below.
let isStreaming = false;
let steeringMessages: string[] = [];
let followUpMessages: string[] = [];
let loadingAnimation: Loader | undefined;
let toolOutputExpanded = false;
let lastSigintTime = 0;
let lastEscapeTime = 0;
let exited = false;

// interactive-mode.ts UI helper ports over the harness containers.
let lastStatusSpacer: Spacer | undefined;
let lastStatusText: Text | undefined;
function showStatus(message: string): void {
  const children = chatContainer.children;
  const last = children.length > 0 ? children[children.length - 1] : undefined;
  const secondLast = children.length > 1 ? children[children.length - 2] : undefined;
  if (last && secondLast && last === lastStatusText && secondLast === lastStatusSpacer) {
    lastStatusText!.setText(theme.fg("dim", message));
    ui.requestRender();
    return;
  }
  const spacer = new Spacer(1);
  const text = new Text(theme.fg("dim", message), 1, 0);
  chatContainer.addChild(spacer);
  chatContainer.addChild(text);
  lastStatusSpacer = spacer;
  lastStatusText = text;
  ui.requestRender();
}

// interactive-mode.ts createWorkingLoader / agent_start body. The loader is
// stopped immediately so its interval cannot advance the spinner between
// captures; the displayed cells equal a frame-0 capture on both sides.
function startWorkingLoader(): void {
  statusContainer.clear();
  loadingAnimation = new Loader(
    ui,
    (spinner) => theme.fg("accent", spinner),
    (text) => theme.fg("muted", text),
    "Working...",
  );
  loadingAnimation.stop();
  statusContainer.addChild(loadingAnimation);
}

function stopWorkingLoader(): void {
  if (loadingAnimation) {
    loadingAnimation.stop();
    loadingAnimation = undefined;
  }
  statusContainer.clear();
}

// interactive-mode.ts updatePendingMessagesDisplay.
function updatePendingMessagesDisplay(): void {
  pendingMessagesContainer.clear();
  if (steeringMessages.length > 0 || followUpMessages.length > 0) {
    pendingMessagesContainer.addChild(new Spacer(1));
    for (const message of steeringMessages) {
      const text = theme.fg("dim", `Steering: ${message}`);
      pendingMessagesContainer.addChild(new TruncatedText(text, 1, 0));
    }
    for (const message of followUpMessages) {
      const text = theme.fg("dim", `Follow-up: ${message}`);
      pendingMessagesContainer.addChild(new TruncatedText(text, 1, 0));
    }
    const dequeueHint = keyDisplayText("app.message.dequeue");
    const hintText = theme.fg("dim", `↳ ${dequeueHint} to edit all queued messages`);
    pendingMessagesContainer.addChild(new TruncatedText(hintText, 1, 0));
  }
}

// Scripted agent abort: the in-flight turn settles as an aborted assistant
// message (message_end body) followed by agent_end (loader stopped).
const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
function abortAgent(): void {
  isStreaming = false;
  stopWorkingLoader();
  chatContainer.addChild(
    new AssistantMessageComponent({
      role: "assistant",
      content: [],
      api: "faux",
      provider: "faux",
      model: "faux-1",
      usage,
      stopReason: "aborted",
      errorMessage: "Operation aborted",
      timestamp: 0,
    } as never),
  );
  ui.requestRender();
}

// interactive-mode.ts clearAllQueues + restoreQueuedMessagesToEditor.
function clearAllQueues(): { steering: string[]; followUp: string[] } {
  const steering = [...steeringMessages];
  const followUp = [...followUpMessages];
  steeringMessages = [];
  followUpMessages = [];
  return { steering, followUp };
}

function restoreQueuedMessagesToEditor(options?: { abort?: boolean }): number {
  const { steering, followUp } = clearAllQueues();
  const allQueued = [...steering, ...followUp];
  if (allQueued.length === 0) {
    updatePendingMessagesDisplay();
    if (options?.abort) abortAgent();
    return 0;
  }
  const queuedText = allQueued.join("\n\n");
  const currentText = editor.getText();
  const combinedText = [queuedText, currentText].filter((t) => t.trim()).join("\n\n");
  editor.setText(combinedText);
  updatePendingMessagesDisplay();
  if (options?.abort) abortAgent();
  return allQueued.length;
}

// interactive-mode.ts handleCtrlC / clearEditor.
function handleCtrlC(): void {
  const now = Date.now();
  if (now - lastSigintTime < 500) {
    exited = true;
  } else {
    editor.setText("");
    ui.requestRender();
    lastSigintTime = now;
  }
}

// interactive-mode.ts setToolsExpanded (active header + expandable children).
function setToolsExpanded(expanded: boolean): void {
  toolOutputExpanded = expanded;
  builtInHeader.setExpanded(expanded);
  ui.requestRender();
}

// interactive-mode.ts handleDequeue.
function handleDequeue(): void {
  const restored = restoreQueuedMessagesToEditor();
  if (restored === 0) {
    showStatus("No queued messages to restore");
  } else {
    showStatus(`Restored ${restored} queued message${restored > 1 ? "s" : ""} to editor`);
  }
}

// interactive-mode.ts handleFollowUp (streaming and non-streaming branches).
function handleFollowUp(): void {
  const text = (editor.getExpandedText?.() ?? editor.getText()).trim();
  if (!text) return;
  if (isStreaming) {
    editor.addToHistory?.(text);
    editor.setText("");
    followUpMessages.push(text);
    updatePendingMessagesDisplay();
    ui.requestRender();
  } else if (editor.onSubmit) {
    editor.setText("");
    editor.onSubmit(text);
  }
}

// interactive-mode.ts setupKeyHandlers.
editor.onEscape = () => {
  if (isStreaming) {
    restoreQueuedMessagesToEditor({ abort: true });
  } else if (!editor.getText().trim()) {
    const action = scenario.doubleEscapeAction;
    if (action !== "none") {
      const now = Date.now();
      if (now - lastEscapeTime < 500) {
        // tree/fork selectors are outside this fixture's scope
        lastEscapeTime = 0;
      } else {
        lastEscapeTime = now;
      }
    }
  }
};
editor.onAction("app.clear", () => handleCtrlC());
editor.onCtrlD = () => {
  exited = true;
};
editor.onAction("app.tools.expand", () => setToolsExpanded(!toolOutputExpanded));
editor.onAction("app.message.followUp", () => handleFollowUp());
editor.onAction("app.message.dequeue", () => handleDequeue());

// interactive-mode.ts handleClipboardImagePaste: readClipboardImage is
// stubbed by the scenario's pre-written temp path; the insert is the body.
editor.onPasteImage = () => {
  if (!scenario.clipboardImagePath) return;
  editor.insertTextAtCursor?.(scenario.clipboardImagePath);
  ui.requestRender();
};

// setupExtensionShortcuts' onExtensionShortcut body over the scenario
// shortcut; the handler's visible effect is a status row.
if (scenario.shortcut) {
  editor.onExtensionShortcut = (data: string) => {
    if (matchesKey(data, scenario.shortcut!.key as KeyId)) {
      showStatus(scenario.shortcut!.status);
      return true;
    }
    return false;
  };
}

// setupEditorSubmitHandler: the reachable slice (plain prompts only) —
// streaming submissions steer, otherwise a normal submission starts the
// scripted turn (user message + agent_start loader).
editor.onSubmit = (text: string) => {
  text = text.trim();
  if (!text) return;
  if (isStreaming) {
    editor.addToHistory?.(text);
    editor.setText("");
    steeringMessages.push(text);
    updatePendingMessagesDisplay();
    ui.requestRender();
    return;
  }
  editor.addToHistory?.(text);
  chatContainer.addChild(new UserMessageComponent(text));
  isStreaming = true;
  startWorkingLoader();
  ui.requestRender();
};

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
  process.stdout.write(JSON.stringify({ frames, exited }));
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
