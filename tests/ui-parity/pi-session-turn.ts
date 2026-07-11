// Session-UI driver (PLAN 6.3): Pi's real SessionSelectorComponent and
// ExtensionSelectorComponent over a scripted session-dir fixture, wired
// with the copied interactive-mode.ts bodies — showSelector,
// showSessionSelector, handleResumeSession (with the real session-cwd
// assertions), handleClearCommand, handleNameCommand, and
// handleSessionCommand — plus the restore path from pi-resume-turn.ts.
// The scenario pins `fixedCwd` (absolute, recreated per run) because
// /session and the missing-cwd prompt surface absolute paths, `nowMs`
// (a fixed Date) for session ages, and PATH="" so the delete flow's
// `trash` probe fails deterministically into unlink.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { AgentMessage, AssistantMessage } from "../../ref/pi/packages/agent/src/types.ts";
import type { Model } from "../../ref/pi/packages/ai/src/models.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { estimateContextTokens } from "../../ref/pi/packages/coding-agent/src/core/compaction/compaction.ts";
import {
  createCodingToolDefinitions,
  type ToolDef,
} from "../../ref/pi/packages/coding-agent/src/core/tools/index.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import {
  MissingSessionCwdError,
  assertSessionCwdExists,
  formatMissingSessionCwdPrompt,
} from "../../ref/pi/packages/coding-agent/src/core/session-cwd.ts";
import { SessionManager } from "../../ref/pi/packages/coding-agent/src/core/session-manager.ts";
import { AssistantMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/assistant-message.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { ExtensionSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/extension-selector.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import {
  keyHint,
  keyText,
  rawKeyHint,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts";
import { SessionSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/session-selector.ts";
import { ToolExecutionComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/tool-execution.ts";
import { UserMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/user-message.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import type { Component } from "../../ref/pi/packages/tui/src/tui.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = { name: string; input?: string[]; resize?: { columns: number; rows: number } };
type Scenario = {
  columns: number;
  rows: number;
  appName: string;
  version: string;
  branch: string;
  providerCount: number;
  fixedCwd: string;
  nowMs: number;
  sessionFile: string;
  sessionDir: string;
  files: Record<string, string>;
  model: Model<"anthropic-messages">;
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

// Deterministic environment.
setCapabilities({ images: null, trueColor: true, hyperlinks: false });
process.env.PATH = "";
const RealDate = Date;
class FixedDate extends RealDate {
  constructor(...args: ConstructorParameters<typeof RealDate>) {
    if (args.length === 0) {
      super(scenario.nowMs);
    } else {
      super(...args);
    }
  }
  static now(): number {
    return scenario.nowMs;
  }
}
(globalThis as { Date: unknown }).Date = FixedDate;

const root = scenario.fixedCwd;
rmSync(root, { recursive: true, force: true });
mkdirSync(root, { recursive: true });
for (const [name, contents] of Object.entries(scenario.files ?? {})) {
  const path = join(root, name);
  if (name.endsWith("/")) {
    mkdirSync(path, { recursive: true });
    continue;
  }
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, contents);
}
process.env.HOME = root;
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
initTheme("dark", false);

// main.ts createSessionManager: open the scenario session in the custom
// session dir; the header cwd is the effective runtime cwd.
const sessionDirAbs = join(root, scenario.sessionDir);
let sessionManager = SessionManager.open(join(root, scenario.sessionFile), sessionDirAbs);
let cwd = sessionManager.getCwd();
let context = sessionManager.buildSessionContext();

// sdk.ts createAgentSession: agent.state.messages = existingSession.messages.
const agentState = {
  model: scenario.model,
  messages: context.messages as AgentMessage[],
  isStreaming: false,
};

const toolDefinitions = new Map<string, ToolDef>(
  createCodingToolDefinitions(cwd).map((def) => [def.name, def]),
);
const getRegisteredToolDefinition = (name: string) => toolDefinitions.get(name);

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
const editorContainer = new Container();
const editor = new CustomEditor(ui, getEditorTheme(), keybindings, {
  paddingX: 0,
  autocompleteMaxVisible: 5,
});
editorContainer.addChild(editor);

// FooterComponent over the (replaceable) session manager and restored state.
const sessionStub = {
  get state() {
    return agentState as never;
  },
  get sessionManager() {
    return sessionManager;
  },
  getContextUsage: () => {
    const contextWindow = scenario.model.contextWindow ?? 0;
    if (contextWindow <= 0) return undefined;
    const estimate = estimateContextTokens(agentState.messages as never);
    return { tokens: estimate.tokens, contextWindow, percent: (estimate.tokens / contextWindow) * 100 };
  },
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
ui.addChild(footer);
ui.setFocus(editor);
ui.start();

let toolOutputExpanded = false;
const pendingTools = new Map<string, ToolExecutionComponent>();

// interactive-mode.ts getUserMessageText.
function getUserMessageText(message: AgentMessage): string {
  if (message.role !== "user") return "";
  const textBlocks =
    typeof message.content === "string"
      ? [{ type: "text", text: message.content }]
      : message.content.filter((c: { type: string }) => c.type === "text");
  return textBlocks.map((c) => (c as { text: string }).text).join("");
}

// interactive-mode.ts addMessageToChat (the restored slice).
function addMessageToChat(message: AgentMessage, options?: { populateHistory?: boolean }): void {
  switch (message.role) {
    case "user": {
      const textContent = getUserMessageText(message);
      if (textContent) {
        if (chatContainer.children.length > 0) {
          chatContainer.addChild(new Spacer(1));
        }
        chatContainer.addChild(new UserMessageComponent(textContent));
        if (options?.populateHistory) {
          editor.addToHistory?.(textContent);
        }
      }
      break;
    }
    case "assistant": {
      const assistantComponent = new AssistantMessageComponent(message as never, false);
      chatContainer.addChild(assistantComponent);
      break;
    }
    default:
      break;
  }
}

// interactive-mode.ts renderSessionContext.
function renderSessionContext(options: { updateFooter?: boolean; populateHistory?: boolean } = {}): void {
  pendingTools.clear();
  const renderedPendingTools = new Map<string, ToolExecutionComponent>();

  if (options.updateFooter) {
    footer.invalidate();
  }

  for (const message of context.messages as AgentMessage[]) {
    if (message.role === "assistant") {
      addMessageToChat(message);
      for (const content of message.content) {
        if (content.type === "toolCall") {
          const component = new ToolExecutionComponent(
            content.name,
            content.id,
            content.arguments,
            {},
            getRegisteredToolDefinition(content.name),
            ui,
            sessionManager.getCwd(),
          );
          component.setExpanded(toolOutputExpanded);
          chatContainer.addChild(component);

          if (message.stopReason === "aborted" || message.stopReason === "error") {
            const errorMessage = message.stopReason === "aborted" ? "Operation aborted" : message.errorMessage || "Error";
            component.updateResult({ content: [{ type: "text", text: errorMessage }], isError: true });
          } else {
            renderedPendingTools.set(content.id, component);
          }
        }
      }
    } else if (message.role === "toolResult") {
      const component = renderedPendingTools.get(message.toolCallId);
      if (component) {
        component.updateResult(message as never);
        renderedPendingTools.delete(message.toolCallId);
      }
    } else {
      addMessageToChat(message, options);
    }
  }

  for (const [toolCallId, component] of renderedPendingTools) {
    pendingTools.set(toolCallId, component);
  }
  ui.requestRender();
}

// interactive-mode.ts showStatus.
function showStatus(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("dim", message), 1, 0));
}

function showWarning(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("warning", `Warning: ${message}`), 1, 0));
}

// interactive-mode.ts renderInitialMessages.
function renderInitialMessages(): void {
  renderSessionContext({ updateFooter: true, populateHistory: true });
  const allEntries = sessionManager.getEntries();
  const compactionCount = allEntries.filter((e) => e.type === "compaction").length;
  if (compactionCount > 0) {
    const times = compactionCount === 1 ? "1 time" : `${compactionCount} times`;
    showStatus(`Session compacted ${times}`);
  }
}

// interactive-mode.ts renderCurrentSessionState.
function renderCurrentSessionState(): void {
  chatContainer.clear();
  pendingMessagesContainer.clear();
  pendingTools.clear();
  renderInitialMessages();
}

// interactive-mode.ts setToolsExpanded.
function setToolsExpanded(expanded: boolean): void {
  toolOutputExpanded = expanded;
  builtInHeader.setExpanded(expanded);
  for (const child of chatContainer.children) {
    if (typeof (child as { setExpanded?: unknown }).setExpanded === "function") {
      (child as { setExpanded: (expanded: boolean) => void }).setExpanded(expanded);
    }
  }
  ui.requestRender();
}
editor.onAction("app.tools.expand", () => setToolsExpanded(!toolOutputExpanded));

// interactive-mode.ts showSelector.
function showSelector(create: (done: () => void) => { component: Component; focus: Component }): void {
  const done = () => {
    editorContainer.clear();
    editorContainer.addChild(editor);
    ui.setFocus(editor);
  };
  const { component, focus } = create(done);
  editorContainer.clear();
  editorContainer.addChild(component);
  ui.setFocus(focus);
  ui.requestRender();
}

// interactive-mode.ts showExtensionSelector/showExtensionConfirm (the
// missing-session-cwd prompt's reachable slice).
function showExtensionConfirm(title: string, message: string): Promise<boolean> {
  return new Promise((resolve) => {
    const selector = new ExtensionSelectorComponent(
      `${title}\n${message}`,
      ["Yes", "No"],
      (option) => {
        hide();
        resolve(option === "Yes");
      },
      () => {
        hide();
        resolve(false);
      },
      { tui: ui, onToggleToolsExpanded: () => setToolsExpanded(!toolOutputExpanded) },
    );
    const hide = () => {
      selector.dispose();
      editorContainer.clear();
      editorContainer.addChild(editor);
      ui.setFocus(editor);
      ui.requestRender();
    };
    editorContainer.clear();
    editorContainer.addChild(selector);
    ui.setFocus(selector);
    ui.requestRender();
  });
}

async function promptForMissingSessionCwd(error: MissingSessionCwdError): Promise<string | undefined> {
  const confirmed = await showExtensionConfirm("Session cwd not found", formatMissingSessionCwdPrompt(error.issue));
  return confirmed ? error.issue.fallbackCwd : undefined;
}

// AgentSessionRuntime.switchSession's observable slice for this driver:
// open + assert cwd, then swap the restored state.
function switchSessionLike(sessionPath: string, cwdOverride?: string): void {
  const next = SessionManager.open(sessionPath, sessionDirAbs, cwdOverride);
  assertSessionCwdExists(next, cwd);
  sessionManager = next;
  cwd = sessionManager.getCwd();
  context = sessionManager.buildSessionContext();
  agentState.messages = context.messages as AgentMessage[];
}

// interactive-mode.ts handleResumeSession.
async function handleResumeSession(sessionPath: string): Promise<void> {
  statusContainer.clear();
  try {
    switchSessionLike(sessionPath);
    renderCurrentSessionState();
    showStatus("Resumed session");
  } catch (error: unknown) {
    if (error instanceof MissingSessionCwdError) {
      const selectedCwd = await promptForMissingSessionCwd(error);
      if (!selectedCwd) {
        showStatus("Resume cancelled");
        return;
      }
      switchSessionLike(sessionPath, selectedCwd);
      renderCurrentSessionState();
      showStatus("Resumed session in current cwd");
      return;
    }
    throw error;
  }
}

// interactive-mode.ts showSessionSelector (verbatim wiring).
function showSessionSelector(): void {
  showSelector((done) => {
    const selector = new SessionSelectorComponent(
      (onProgress) => SessionManager.list(sessionManager.getCwd(), sessionManager.getSessionDir(), onProgress),
      (onProgress) =>
        sessionManager.usesDefaultSessionDir()
          ? SessionManager.listAll(onProgress)
          : SessionManager.listAll(sessionManager.getSessionDir(), onProgress),
      async (sessionPath) => {
        done();
        await handleResumeSession(sessionPath);
      },
      () => {
        done();
        ui.requestRender();
      },
      () => {},
      () => ui.requestRender(),
      {
        renameSession: async (sessionFilePath: string, nextName: string | undefined) => {
          const next = (nextName ?? "").trim();
          if (!next) return;
          const mgr = SessionManager.open(sessionFilePath);
          mgr.appendSessionInfo(next);
        },
        showRenameHint: true,
        keybindings,
      },
      sessionManager.getSessionFile(),
    );
    return { component: selector, focus: selector };
  });
}

// interactive-mode.ts handleClearCommand over AgentSessionRuntime.newSession.
function handleClearCommand(): void {
  statusContainer.clear();
  const next = SessionManager.create(cwd, sessionManager.getSessionDir());
  sessionManager = next;
  context = sessionManager.buildSessionContext();
  agentState.messages = context.messages as AgentMessage[];
  renderCurrentSessionState();
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(`${theme.fg("accent", "✓ New session started")}`, 1, 1));
  ui.requestRender();
}

// interactive-mode.ts handleNameCommand.
function handleNameCommand(text: string): void {
  const name = text.replace(/^\/name\s*/, "").trim();
  if (!name) {
    const currentName = sessionManager.getSessionName();
    if (currentName) {
      chatContainer.addChild(new Spacer(1));
      chatContainer.addChild(new Text(theme.fg("dim", `Session name: ${currentName}`), 1, 0));
    } else {
      showWarning("Usage: /name <name>");
    }
    ui.requestRender();
    return;
  }
  sessionManager.appendSessionInfo(name);
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("dim", `Session name set: ${name}`), 1, 0));
  ui.requestRender();
}

// agent-session.ts getSessionStats + interactive-mode.ts handleSessionCommand.
function handleSessionCommand(): void {
  const messages = agentState.messages;
  const userMessages = messages.filter((m) => m.role === "user").length;
  const assistantMessages = messages.filter((m) => m.role === "assistant").length;
  const toolResults = messages.filter((m) => m.role === "toolResult").length;
  let toolCalls = 0;
  let totalInput = 0;
  let totalOutput = 0;
  let totalCacheRead = 0;
  let totalCacheWrite = 0;
  let totalCost = 0;
  for (const message of messages) {
    if (message.role === "assistant") {
      const assistantMsg = message as AssistantMessage;
      toolCalls += assistantMsg.content.filter((c) => c.type === "toolCall").length;
      totalInput += assistantMsg.usage.input;
      totalOutput += assistantMsg.usage.output;
      totalCacheRead += assistantMsg.usage.cacheRead;
      totalCacheWrite += assistantMsg.usage.cacheWrite;
      totalCost += assistantMsg.usage.cost.total;
    }
  }
  const stats = {
    sessionFile: sessionManager.getSessionFile(),
    sessionId: sessionManager.getSessionId(),
    userMessages,
    assistantMessages,
    toolCalls,
    toolResults,
    totalMessages: messages.length,
    tokens: {
      input: totalInput,
      output: totalOutput,
      cacheRead: totalCacheRead,
      cacheWrite: totalCacheWrite,
      total: totalInput + totalOutput + totalCacheRead + totalCacheWrite,
    },
    cost: totalCost,
  };
  const sessionName = sessionManager.getSessionName();

  let info = `${theme.bold("Session Info")}\n\n`;
  if (sessionName) {
    info += `${theme.fg("dim", "Name:")} ${sessionName}\n`;
  }
  info += `${theme.fg("dim", "File:")} ${stats.sessionFile ?? "In-memory"}\n`;
  info += `${theme.fg("dim", "ID:")} ${stats.sessionId}\n\n`;
  info += `${theme.bold("Messages")}\n`;
  info += `${theme.fg("dim", "User:")} ${stats.userMessages}\n`;
  info += `${theme.fg("dim", "Assistant:")} ${stats.assistantMessages}\n`;
  info += `${theme.fg("dim", "Tool Calls:")} ${stats.toolCalls}\n`;
  info += `${theme.fg("dim", "Tool Results:")} ${stats.toolResults}\n`;
  info += `${theme.fg("dim", "Total:")} ${stats.totalMessages}\n\n`;
  info += `${theme.bold("Tokens")}\n`;
  info += `${theme.fg("dim", "Input:")} ${stats.tokens.input.toLocaleString()}\n`;
  info += `${theme.fg("dim", "Output:")} ${stats.tokens.output.toLocaleString()}\n`;
  if (stats.tokens.cacheRead > 0) {
    info += `${theme.fg("dim", "Cache Read:")} ${stats.tokens.cacheRead.toLocaleString()}\n`;
  }
  if (stats.tokens.cacheWrite > 0) {
    info += `${theme.fg("dim", "Cache Write:")} ${stats.tokens.cacheWrite.toLocaleString()}\n`;
  }
  info += `${theme.fg("dim", "Total:")} ${stats.tokens.total.toLocaleString()}\n`;
  if (stats.cost > 0) {
    info += `\n${theme.bold("Cost")}\n`;
    info += `${theme.fg("dim", "Total:")} ${stats.cost.toFixed(4)}`;
  }

  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(info, 1, 0));
  ui.requestRender();
}

// interactive-mode.ts setupEditorSubmitHandler (the routed slice).
editor.onSubmit = async (text: string) => {
  text = text.trim();
  if (!text) return;
  if (text === "/name" || text.startsWith("/name ")) {
    handleNameCommand(text);
    editor.setText("");
    return;
  }
  if (text === "/session") {
    handleSessionCommand();
    editor.setText("");
    return;
  }
  if (text === "/new") {
    editor.setText("");
    handleClearCommand();
    return;
  }
  if (text === "/resume") {
    showSessionSelector();
    editor.setText("");
    return;
  }
};

renderInitialMessages();

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
    for (const data of step.input ?? []) {
      terminal.send(data);
      // Selector loads, deletion, and resume handlers settle between
      // keystrokes (the component awaits its async loaders).
      await new Promise<void>((resolve) => setTimeout(resolve, 10));
    }
    await capture(step.name, Boolean(step.resize));
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
  process.exit(0);
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
