// Tree-navigation driver (PLAN 6.4): Pi's real TreeSelectorComponent,
// UserMessageSelectorComponent, BranchSummaryMessageComponent,
// ExtensionSelector/ExtensionEditor, and the real branch summarizer
// (generateBranchSummary → completeSimple against a held local stub)
// over a branched session fixture, wired with the copied
// interactive-mode.ts bodies — showSelector, showTreeSelector (the
// summarize-choice loop, the summarizing Loader, the escape-abort
// override), showUserMessageSelector, handleCloneCommand, and the
// AgentSessionRuntime.fork slice. The stub holds each response until the
// scenario's `release` step so the summarize-loader frames are
// deterministic; `awaitNavigate` settles the pending navigation after an
// abort. The Loader is stopped at construction so both sides capture
// spinner frame 0.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { dirname, join } from "node:path";
import type { AgentMessage, AssistantMessage } from "../../ref/pi/packages/agent/src/types.ts";
import type { Model } from "../../ref/pi/packages/ai/src/models.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { estimateContextTokens } from "../../ref/pi/packages/coding-agent/src/core/compaction/compaction.ts";
import {
  collectEntriesForBranchSummary,
  generateBranchSummary,
} from "../../ref/pi/packages/coding-agent/src/core/compaction/branch-summarization.ts";
import {
  createCodingToolDefinitions,
  type ToolDef,
} from "../../ref/pi/packages/coding-agent/src/core/tools/index.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import { SessionManager } from "../../ref/pi/packages/coding-agent/src/core/session-manager.ts";
import { AssistantMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/assistant-message.ts";
import { BranchSummaryMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/branch-summary-message.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { ExtensionEditorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/extension-editor.ts";
import { ExtensionSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/extension-selector.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import {
  keyHint,
  keyText,
  rawKeyHint,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts";
import { ToolExecutionComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/tool-execution.ts";
import { TreeSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/tree-selector.ts";
import { UserMessageSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/user-message-selector.ts";
import { UserMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/user-message.ts";
import { getEditorTheme, getMarkdownTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import type { Component } from "../../ref/pi/packages/tui/src/tui.ts";
import { Loader } from "../../ref/pi/packages/tui/src/components/loader.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type SseEvent = { event: string; data: unknown };
type Step = {
  name: string;
  input?: string[];
  resize?: { columns: number; rows: number };
  settle?: boolean;
  release?: boolean;
  awaitNavigate?: boolean;
};
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
  apiKey: string;
  files: Record<string, string>;
  model: Model<"anthropic-messages">;
  stub: {
    sse?: Record<string, SseEvent[]>;
    responses: Array<{ status?: number; sse?: string; events?: SseEvent[]; hang?: boolean }>;
  };
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

// The scripted provider stub: each response is HELD until the scenario's
// `release` step asks for it (the client abort settles held hang
// responses), so mid-summarize frames are deterministic.
function sseBody(events: SseEvent[]): string {
  return events.map((event) => `event: ${event.event}\ndata: ${JSON.stringify(event.data)}\n\n`).join("");
}
let responseIndex = 0;
const heldResponses: Array<() => void> = [];
const releaseWaiters: Array<() => void> = [];
const server = createServer((req, res) => {
  req.on("data", () => {});
  req.on("end", () => {
    const responses = scenario.stub.responses;
    const scripted = responses[responseIndex] ?? responses[responses.length - 1];
    responseIndex += 1;
    const send = () => {
      const events = scripted?.sse ? scenario.stub.sse![scripted.sse]! : scripted?.events;
      const body = events ? sseBody(events) : "";
      res.writeHead(scripted?.status ?? 200, { "content-type": "text/event-stream" });
      if (scripted?.hang) {
        res.write(body); // Hold the connection open; the client aborts.
      } else {
        res.end(body);
      }
    };
    heldResponses.push(send);
    releaseWaiters.shift()?.();
  });
});
async function releaseStub(): Promise<void> {
  if (heldResponses.length === 0) {
    await new Promise<void>((resolve) => releaseWaiters.push(resolve));
  }
  heldResponses.shift()?.();
}

// main.ts createSessionManager: open the scenario session in the custom
// session dir; the header cwd is the effective runtime cwd.
const sessionDirAbs = join(root, scenario.sessionDir);
let sessionManager = SessionManager.open(join(root, scenario.sessionFile), sessionDirAbs);
const cwd = sessionManager.getCwd();
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

// agent-session.ts _extractUserMessageText.
function extractUserMessageText(content: string | Array<{ type: string; text?: string }>): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .filter((c): c is { type: "text"; text: string } => c.type === "text")
      .map((c) => c.text)
      .join("");
  }
  return "";
}

// interactive-mode.ts addMessageToChat (the restored slice + branchSummary).
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
    case "branchSummary": {
      chatContainer.addChild(new Spacer(1));
      const component = new BranchSummaryMessageComponent(message as never, getMarkdownTheme());
      component.setExpanded(toolOutputExpanded);
      chatContainer.addChild(component);
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

function showError(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("error", `Error: ${message}`), 1, 0));
  chatContainer.addChild(new Spacer(1));
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

// interactive-mode.ts showExtensionSelector.
function showExtensionSelector(title: string, options: string[]): Promise<string | undefined> {
  return new Promise((resolve) => {
    const selector = new ExtensionSelectorComponent(
      title,
      options,
      (option) => {
        hide();
        resolve(option);
      },
      () => {
        hide();
        resolve(undefined);
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

// interactive-mode.ts showExtensionEditor.
function showExtensionEditor(title: string, prefill?: string): Promise<string | undefined> {
  return new Promise((resolve) => {
    const extensionEditor = new ExtensionEditorComponent(
      ui,
      keybindings,
      title,
      prefill,
      (value) => {
        hide();
        resolve(value);
      },
      () => {
        hide();
        resolve(undefined);
      },
    );
    const hide = () => {
      editorContainer.clear();
      editorContainer.addChild(editor);
      ui.setFocus(editor);
      ui.requestRender();
    };
    editorContainer.clear();
    editorContainer.addChild(extensionEditor);
    ui.setFocus(extensionEditor);
    ui.requestRender();
  });
}

// AgentSessionRuntime.fork's observable slice for this driver: the
// branched-session copy (or fresh parented session for fork-to-root) and
// the restored state swap.
function fork(entryId: string, options?: { position?: "before" | "at" }): { cancelled: boolean; selectedText?: string } {
  const position = options?.position ?? "before";
  const selectedEntry = sessionManager.getEntry(entryId);
  if (!selectedEntry) throw new Error("Invalid entry ID for forking");
  let targetLeafId: string | null;
  let selectedText: string | undefined;
  if (position === "at") {
    targetLeafId = selectedEntry.id;
  } else {
    if (selectedEntry.type !== "message" || selectedEntry.message.role !== "user") {
      throw new Error("Invalid entry ID for forking");
    }
    targetLeafId = selectedEntry.parentId;
    selectedText = extractUserMessageText(selectedEntry.message.content as never);
  }
  const currentSessionFile = sessionManager.getSessionFile()!;
  const sessionDir = sessionManager.getSessionDir();
  if (!targetLeafId) {
    const next = SessionManager.create(cwd, sessionDir);
    next.newSession({ parentSession: currentSessionFile });
    sessionManager = next;
  } else {
    const next = SessionManager.open(currentSessionFile, sessionDir);
    if (!next.createBranchedSession(targetLeafId)) {
      throw new Error("Failed to create forked session");
    }
    sessionManager = next;
  }
  context = sessionManager.buildSessionContext();
  agentState.messages = context.messages as AgentMessage[];
  return { cancelled: false, selectedText };
}

// agent-session.ts navigateTree (the reachable slice: settings defaults,
// no extension hooks) over the real generateBranchSummary/completeSimple.
let branchSummaryAbort: AbortController | undefined;
async function navigateTree(
  targetId: string,
  options: { summarize?: boolean; customInstructions?: string },
): Promise<{ editorText?: string; cancelled: boolean; aborted?: boolean }> {
  const oldLeafId = sessionManager.getLeafId();
  if (targetId === oldLeafId) return { cancelled: false };
  const targetEntry = sessionManager.getEntry(targetId);
  if (!targetEntry) throw new Error(`Entry ${targetId} not found`);
  const { entries: entriesToSummarize } = collectEntriesForBranchSummary(sessionManager, oldLeafId, targetId);
  branchSummaryAbort = new AbortController();
  try {
    let summaryText: string | undefined;
    let summaryDetails: unknown;
    if (options.summarize && entriesToSummarize.length > 0) {
      const result = await generateBranchSummary(entriesToSummarize, {
        model: scenario.model,
        apiKey: scenario.apiKey,
        signal: branchSummaryAbort.signal,
        customInstructions: options.customInstructions,
        reserveTokens: 16384,
      });
      if (result.aborted) return { cancelled: true, aborted: true };
      if (result.error) throw new Error(result.error);
      summaryText = result.summary;
      summaryDetails = { readFiles: result.readFiles || [], modifiedFiles: result.modifiedFiles || [] };
    }

    let newLeafId: string | null;
    let editorText: string | undefined;
    if (targetEntry.type === "message" && targetEntry.message.role === "user") {
      newLeafId = targetEntry.parentId;
      editorText = extractUserMessageText(targetEntry.message.content as never);
    } else if (targetEntry.type === "custom_message") {
      newLeafId = targetEntry.parentId;
      editorText =
        typeof targetEntry.content === "string"
          ? targetEntry.content
          : targetEntry.content
              .filter((c): c is { type: "text"; text: string } => c.type === "text")
              .map((c) => c.text)
              .join("");
    } else {
      newLeafId = targetId;
    }

    if (summaryText) {
      sessionManager.branchWithSummary(newLeafId, summaryText, summaryDetails, false);
    } else if (newLeafId === null) {
      sessionManager.resetLeaf();
    } else {
      sessionManager.branch(newLeafId);
    }

    context = sessionManager.buildSessionContext();
    agentState.messages = context.messages as AgentMessage[];
    return { editorText, cancelled: false };
  } finally {
    branchSummaryAbort = undefined;
  }
}

// interactive-mode.ts showTreeSelector (settings defaults: treeFilterMode
// "default", branchSummary.skipPrompt false).
let pendingNavigate: Promise<void> | undefined;
function showTreeSelector(initialSelectedId?: string): void {
  const tree = sessionManager.getTree();
  const realLeafId = sessionManager.getLeafId();
  const initialFilterMode = "default" as const;

  if (tree.length === 0) {
    showStatus("No entries in session");
    return;
  }

  showSelector((done) => {
    const selector = new TreeSelectorComponent(
      tree,
      realLeafId,
      terminal.rows,
      async (entryId) => {
        if (entryId === realLeafId) {
          done();
          showStatus("Already at this point");
          return;
        }
        done();

        let wantsSummary = false;
        let customInstructions: string | undefined;
        while (true) {
          const summaryChoice = await showExtensionSelector("Summarize branch?", [
            "No summary",
            "Summarize",
            "Summarize with custom prompt",
          ]);
          if (summaryChoice === undefined) {
            showTreeSelector(entryId);
            return;
          }
          wantsSummary = summaryChoice !== "No summary";
          if (summaryChoice === "Summarize with custom prompt") {
            customInstructions = await showExtensionEditor("Custom summarization instructions");
            if (customInstructions === undefined) {
              continue;
            }
          }
          break;
        }

        let summaryLoader: Loader | undefined;
        const originalOnEscape = editor.onEscape;
        if (wantsSummary) {
          editor.onEscape = () => {
            branchSummaryAbort?.abort();
          };
          chatContainer.addChild(new Spacer(1));
          summaryLoader = new Loader(
            ui,
            (spinner) => theme.fg("accent", spinner),
            (text) => theme.fg("muted", text),
            `Summarizing branch... (${keyText("app.interrupt")} to cancel)`,
          );
          // Stopped immediately so the interval cannot advance the spinner
          // between captures (both sides pin frame 0).
          summaryLoader.stop();
          statusContainer.addChild(summaryLoader);
          ui.requestRender();
        }

        pendingNavigate = (async () => {
          try {
            const result = await navigateTree(entryId, {
              summarize: wantsSummary,
              customInstructions,
            });
            if (result.aborted) {
              showStatus("Branch summarization cancelled");
              showTreeSelector(entryId);
              return;
            }
            if (result.cancelled) {
              showStatus("Navigation cancelled");
              return;
            }
            chatContainer.clear();
            renderInitialMessages();
            if (result.editorText && !editor.getText().trim()) {
              editor.setText(result.editorText);
            }
            showStatus("Navigated to selected point");
          } catch (error) {
            showError(error instanceof Error ? error.message : String(error));
          } finally {
            if (summaryLoader) {
              summaryLoader.stop();
              statusContainer.clear();
            }
            editor.onEscape = originalOnEscape;
          }
        })();
      },
      () => {
        done();
        ui.requestRender();
      },
      (entryId, label) => {
        sessionManager.appendLabelChange(entryId, label);
        ui.requestRender();
      },
      initialSelectedId,
      initialFilterMode,
    );
    return { component: selector, focus: selector };
  });
}

// agent-session.ts getUserMessagesForForking.
function getUserMessagesForForking(): Array<{ entryId: string; text: string }> {
  const result: Array<{ entryId: string; text: string }> = [];
  for (const entry of sessionManager.getEntries()) {
    if (entry.type !== "message") continue;
    if (entry.message.role !== "user") continue;
    const text = extractUserMessageText(entry.message.content as never);
    if (text) {
      result.push({ entryId: entry.id, text });
    }
  }
  return result;
}

// interactive-mode.ts showUserMessageSelector.
function showUserMessageSelector(): void {
  const userMessages = getUserMessagesForForking();

  if (userMessages.length === 0) {
    showStatus("No messages to fork from");
    return;
  }

  const initialSelectedId = userMessages[userMessages.length - 1]?.entryId;

  showSelector((done) => {
    const selector = new UserMessageSelectorComponent(
      userMessages.map((m) => ({ id: m.entryId, text: m.text })),
      (entryId) => {
        try {
          const result = fork(entryId);
          if (result.cancelled) {
            done();
            ui.requestRender();
            return;
          }
          renderCurrentSessionState();
          editor.setText(result.selectedText ?? "");
          done();
          showStatus("Forked to new session");
        } catch (error: unknown) {
          done();
          showError(error instanceof Error ? error.message : String(error));
        }
      },
      () => {
        done();
        ui.requestRender();
      },
      initialSelectedId,
    );
    return { component: selector, focus: selector.getMessageList() };
  });
}

// interactive-mode.ts handleCloneCommand.
function handleCloneCommand(): void {
  const leafId = sessionManager.getLeafId();
  if (!leafId) {
    showStatus("Nothing to clone yet");
    return;
  }
  try {
    const result = fork(leafId, { position: "at" });
    if (result.cancelled) {
      ui.requestRender();
      return;
    }
    renderCurrentSessionState();
    editor.setText("");
    showStatus("Cloned to new session");
  } catch (error: unknown) {
    showError(error instanceof Error ? error.message : String(error));
  }
}

// interactive-mode.ts setupKeyHandlers onEscape (the double-escape slice;
// the summarize flow swaps this handler like the spec does).
let lastEscapeTime = 0;
editor.onEscape = () => {
  if (!editor.getText().trim()) {
    const action = "tree"; // settings default
    const now = Date.now();
    if (now - lastEscapeTime < 500) {
      showTreeSelector();
      lastEscapeTime = 0;
    } else {
      lastEscapeTime = now;
    }
  }
};

// interactive-mode.ts setupEditorSubmitHandler (the routed slice).
editor.onSubmit = (text: string) => {
  text = text.trim();
  if (!text) return;
  if (text === "/fork") {
    showUserMessageSelector();
    editor.setText("");
    return;
  }
  if (text === "/clone") {
    editor.setText("");
    handleCloneCommand();
    return;
  }
  if (text === "/tree") {
    showTreeSelector();
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
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  scenario.model = {
    ...scenario.model,
    baseUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
  };
  await capture("startup", true);
  for (const step of scenario.steps) {
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    for (const data of step.input ?? []) {
      terminal.send(data);
      // Selection handlers settle between keystrokes (the async selector
      // flows await microtasks; the held stub keeps summaries pending).
      // 25ms > TUI.MIN_RENDER_INTERVAL_MS so every keystroke renders —
      // the hidden hardware cursor's resting position depends on the
      // per-key differential writes, and pi-rs renders per input.
      await new Promise<void>((resolve) => setTimeout(resolve, 25));
    }
    if (step.release) {
      await releaseStub();
    }
    if (step.release || step.awaitNavigate) {
      await pendingNavigate;
      await new Promise<void>((resolve) => setTimeout(resolve, 10));
    }
    await capture(step.name, Boolean(step.resize));
  }
  ui.stop();
  server.close();
  process.stdout.write(JSON.stringify({ frames }));
  process.exit(0);
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
