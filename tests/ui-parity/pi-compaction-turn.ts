// Compaction driver (PLAN 6.5): Pi's real Agent + streamAnthropic against
// the scenario's scripted local SSE stub, the real compaction pipeline
// (prepareCompaction/compact from core/compaction/compaction.ts) over a
// real SessionManager restored from the scenario fixture, and the real
// CompactionSummaryMessageComponent — wired with the copied
// interactive-mode.ts bodies: handleEvent, handleCompactCommand,
// compaction_start/compaction_end (loader + escape-override swap + chat
// rebuild), queueCompactionMessage/flushCompactionQueue, and the copied
// agent-session.ts compact()/_checkCompaction/_runAutoCompaction slice.
// Frames are captured at exact event points from the awaited listeners —
// the same points pi-rs's product sequence captures at — so no stub pacing
// is needed: a compaction_start capture always lands before the
// summarization request leaves.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { dirname, join } from "node:path";
import { Agent } from "../../ref/pi/packages/agent/src/agent.ts";
import type { AgentMessage, AssistantMessage } from "../../ref/pi/packages/agent/src/types.ts";
import type { Model } from "../../ref/pi/packages/ai/src/models.ts";
import { isContextOverflow } from "../../ref/pi/packages/ai/src/utils/overflow.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import {
  calculateContextTokens,
  compact,
  estimateContextTokens,
  prepareCompaction,
  shouldCompact,
} from "../../ref/pi/packages/coding-agent/src/core/compaction/compaction.ts";
import { createCompactionSummaryMessage } from "../../ref/pi/packages/coding-agent/src/core/messages.ts";
import { getLatestCompactionEntry, SessionManager } from "../../ref/pi/packages/coding-agent/src/core/session-manager.ts";
import {
  createCodingToolDefinitions,
  createCodingTools,
  type ToolDef,
} from "../../ref/pi/packages/coding-agent/src/core/tools/index.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import { AssistantMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/assistant-message.ts";
import { BranchSummaryMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/branch-summary-message.ts";
import { CompactionSummaryMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/compaction-summary-message.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import {
  keyDisplayText,
  keyHint,
  keyText,
  rawKeyHint,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts";
import { ToolExecutionComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/tool-execution.ts";
import { UserMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/user-message.ts";
import { getEditorTheme, getMarkdownTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Loader } from "../../ref/pi/packages/tui/src/components/loader.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { TruncatedText } from "../../ref/pi/packages/tui/src/components/truncated-text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type SseEvent = { event: string; data: unknown };
type Capture = { name?: string; event: string; count?: number; action?: string; text?: string };
type Step = {
  name?: string;
  input?: string[];
  resize?: { columns: number; rows: number };
  settle?: boolean;
  captures?: Capture[];
};
type Scenario = {
  columns: number;
  rows: number;
  appName: string;
  version: string;
  branch: string;
  providerCount: number;
  fixedCwd: string;
  sessionFile: string;
  sessionDir: string;
  apiKey: string;
  compaction?: { enabled?: boolean; reserveTokens?: number; keepRecentTokens?: number };
  files: Record<string, string>;
  model: Model<"anthropic-messages">;
  stub: { sse?: Record<string, SseEvent[]>; responses: Array<{ status?: number; sse?: string; events?: SseEvent[]; json?: unknown }> };
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
const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

// Deterministic environment.
setCapabilities({ images: null, trueColor: true, hyperlinks: false });
process.env.PATH = "";
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

// --- Scripted provider stub (provider-turn driver, without pacing) ---
function sseBody(events: SseEvent[]): string {
  return events.map((e) => `event: ${e.event}\ndata: ${JSON.stringify(e.data)}\n\n`).join("");
}
let responseIndex = 0;
const server = createServer((req, res) => {
  req.on("data", () => {});
  req.on("end", () => {
    const responses = scenario.stub.responses;
    const scripted = responses[responseIndex] ?? responses[responses.length - 1];
    responseIndex += 1;
    if (!scripted) {
      res.writeHead(500).end("no scripted response");
      return;
    }
    const events = scripted.sse ? scenario.stub.sse![scripted.sse]! : scripted.events;
    if (events) {
      res.writeHead(scripted.status ?? 200, { "content-type": "text/event-stream" });
      res.end(sseBody(events));
      return;
    }
    res.writeHead(scripted.status ?? 200, { "content-type": "application/json" });
    res.end(JSON.stringify(scripted.json ?? {}));
  });
});

// main.ts createSessionManager + sdk.ts createAgentSession restore slice.
const sessionDirAbs = join(root, scenario.sessionDir);
const sessionManager = SessionManager.open(join(root, scenario.sessionFile), sessionDirAbs);
const cwd = sessionManager.getCwd();
const context = sessionManager.buildSessionContext();

let model: Model<"anthropic-messages">;
const agent = new Agent({
  initialState: { model: scenario.model, tools: createCodingTools(cwd) },
  getApiKey: () => scenario.apiKey,
});
agent.state.messages = context.messages as AgentMessage[];
const toolDefinitions = new Map<string, ToolDef>(
  createCodingToolDefinitions(cwd).map((def) => [def.name, def]),
);
const getRegisteredToolDefinition = (name: string) => toolDefinitions.get(name);

// settings-manager.ts getCompactionSettings, pinned by the scenario.
const compactionSettings = {
  enabled: true,
  reserveTokens: 16384,
  keepRecentTokens: 20000,
  ...(scenario.compaction ?? {}),
};

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

// FooterComponent over the real agent state and session manager.
const sessionStub = {
  get state() {
    return agent.state as never;
  },
  get sessionManager() {
    return sessionManager;
  },
  getContextUsage: () => {
    const contextWindow = scenario.model.contextWindow ?? 0;
    if (contextWindow <= 0) return undefined;
    const estimate = estimateContextTokens(agent.state.messages);
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

// --- interactive-mode.ts chrome helpers ---
let toolOutputExpanded = false;
const pendingTools = new Map<string, ToolExecutionComponent>();
let streamingComponent: AssistantMessageComponent | undefined;
let loadingAnimation: Loader | undefined;

function showStatus(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("dim", message), 1, 0));
}
function showError(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("error", `Error: ${message}`), 1, 0));
  chatContainer.addChild(new Spacer(1));
}
function showWarning(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("warning", `Warning: ${message}`), 1, 0));
}

function getUserMessageText(message: AgentMessage): string {
  if (message.role !== "user") return "";
  const blocks =
    typeof message.content === "string"
      ? [{ type: "text", text: message.content }]
      : message.content.filter((c: { type: string }) => c.type === "text");
  return blocks.map((c) => (c as { text: string }).text).join("");
}

// interactive-mode.ts addMessageToChat (the reachable slice).
function addMessageToChat(message: AgentMessage, options?: { populateHistory?: boolean }): void {
  switch (message.role) {
    case "user": {
      const textContent = getUserMessageText(message);
      if (textContent) {
        if (chatContainer.children.length > 0) chatContainer.addChild(new Spacer(1));
        chatContainer.addChild(new UserMessageComponent(textContent));
        if (options?.populateHistory) editor.addToHistory?.(textContent);
      }
      break;
    }
    case "assistant": {
      chatContainer.addChild(new AssistantMessageComponent(message as never, false));
      break;
    }
    case "compactionSummary": {
      chatContainer.addChild(new Spacer(1));
      const component = new CompactionSummaryMessageComponent(message as never, getMarkdownTheme());
      component.setExpanded(toolOutputExpanded);
      chatContainer.addChild(component);
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
function renderSessionContext(
  sessionContext: { messages: AgentMessage[] },
  options: { updateFooter?: boolean; populateHistory?: boolean } = {},
): void {
  pendingTools.clear();
  const renderedPendingTools = new Map<string, ToolExecutionComponent>();
  if (options.updateFooter) footer.invalidate();
  for (const message of sessionContext.messages) {
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
            component.updateResult({ content: [{ type: "text", text: errorMessage }], isError: true } as never);
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

// interactive-mode.ts rebuildChatFromMessages.
function rebuildChatFromMessages(): void {
  chatContainer.clear();
  renderSessionContext(sessionManager.buildSessionContext() as never);
}

// interactive-mode.ts renderInitialMessages.
function renderInitialMessages(): void {
  renderSessionContext(sessionManager.buildSessionContext() as never, {
    updateFooter: true,
    populateHistory: true,
  });
  const compactionCount = sessionManager.getEntries().filter((e) => e.type === "compaction").length;
  if (compactionCount > 0) {
    const times = compactionCount === 1 ? "1 time" : `${compactionCount} times`;
    showStatus(`Session compacted ${times}`);
  }
}

// --- queue display (interactive-mode.ts) ---
type CompactionQueuedMessage = { text: string; mode: "steer" | "followUp" };
let compactionQueuedMessages: CompactionQueuedMessage[] = [];
const steeringTexts: string[] = [];
const followUpTexts: string[] = [];

function getAllQueuedMessages(): { steering: string[]; followUp: string[] } {
  return {
    steering: [...steeringTexts, ...compactionQueuedMessages.filter((m) => m.mode === "steer").map((m) => m.text)],
    followUp: [...followUpTexts, ...compactionQueuedMessages.filter((m) => m.mode === "followUp").map((m) => m.text)],
  };
}

function updatePendingMessagesDisplay(): void {
  pendingMessagesContainer.clear();
  const { steering, followUp } = getAllQueuedMessages();
  if (steering.length > 0 || followUp.length > 0) {
    pendingMessagesContainer.addChild(new Spacer(1));
    for (const message of steering) {
      pendingMessagesContainer.addChild(new TruncatedText(theme.fg("dim", `Steering: ${message}`), 1, 0));
    }
    for (const message of followUp) {
      pendingMessagesContainer.addChild(new TruncatedText(theme.fg("dim", `Follow-up: ${message}`), 1, 0));
    }
    const dequeueHint = keyDisplayText("app.message.dequeue");
    pendingMessagesContainer.addChild(
      new TruncatedText(theme.fg("dim", `↳ ${dequeueHint} to edit all queued messages`), 1, 0),
    );
  }
}

// --- agent-session.ts compaction slice (copied bodies) ---
let lastAssistantMessage: AssistantMessage | undefined;
let overflowRecoveryAttempted = false;
let compactionAbortController: AbortController | undefined;
let autoCompactionAbortController: AbortController | undefined;

function isCompacting(): boolean {
  return compactionAbortController !== undefined || autoCompactionAbortController !== undefined;
}
function abortCompaction(): void {
  compactionAbortController?.abort();
  autoCompactionAbortController?.abort();
}
function findLastAssistantMessage(): AssistantMessage | undefined {
  const messages = agent.state.messages;
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") return messages[i] as AssistantMessage;
  }
  return undefined;
}

type SessionEvent = { type: string; reason?: string; result?: unknown; aborted?: boolean; willRetry?: boolean; errorMessage?: string };

async function compactSession(customInstructions?: string): Promise<void> {
  await agent.waitForIdle();
  compactionAbortController = new AbortController();
  await emitSession({ type: "compaction_start", reason: "manual" });
  try {
    const pathEntries = sessionManager.getBranch();
    const preparation = prepareCompaction(pathEntries, compactionSettings);
    if (!preparation) {
      const lastEntry = pathEntries[pathEntries.length - 1];
      if (lastEntry?.type === "compaction") throw new Error("Already compacted");
      throw new Error("Nothing to compact (session too small)");
    }
    const result = await compact(
      preparation,
      model,
      scenario.apiKey,
      undefined,
      customInstructions,
      compactionAbortController.signal,
      undefined,
      undefined,
    );
    if (compactionAbortController.signal.aborted) throw new Error("Compaction cancelled");
    sessionManager.appendCompaction(result.summary, result.firstKeptEntryId, result.tokensBefore, result.details, false);
    agent.state.messages = sessionManager.buildSessionContext().messages as AgentMessage[];
    await emitSession({ type: "compaction_end", reason: "manual", result, aborted: false, willRetry: false });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const aborted = message === "Compaction cancelled" || (error instanceof Error && error.name === "AbortError");
    await emitSession({
      type: "compaction_end",
      reason: "manual",
      result: undefined,
      aborted,
      willRetry: false,
      errorMessage: aborted ? undefined : `Compaction failed: ${message}`,
    });
  } finally {
    compactionAbortController = undefined;
  }
}

async function runAutoCompaction(reason: "overflow" | "threshold", willRetry: boolean): Promise<boolean> {
  await emitSession({ type: "compaction_start", reason });
  autoCompactionAbortController = new AbortController();
  try {
    const pathEntries = sessionManager.getBranch();
    const preparation = prepareCompaction(pathEntries, compactionSettings);
    if (!preparation) {
      await emitSession({ type: "compaction_end", reason, result: undefined, aborted: false, willRetry: false });
      return false;
    }
    const result = await compact(
      preparation,
      model,
      scenario.apiKey,
      undefined,
      undefined,
      autoCompactionAbortController.signal,
      undefined,
      undefined,
    );
    if (autoCompactionAbortController.signal.aborted) {
      await emitSession({ type: "compaction_end", reason, result: undefined, aborted: true, willRetry: false });
      return false;
    }
    sessionManager.appendCompaction(result.summary, result.firstKeptEntryId, result.tokensBefore, result.details, false);
    agent.state.messages = sessionManager.buildSessionContext().messages as AgentMessage[];
    await emitSession({ type: "compaction_end", reason, result, aborted: false, willRetry });
    if (willRetry) {
      const messages = agent.state.messages;
      const lastMsg = messages[messages.length - 1];
      if (lastMsg?.role === "assistant" && (lastMsg as AssistantMessage).stopReason === "error") {
        agent.state.messages = messages.slice(0, -1);
      }
      return true;
    }
    return agent.hasQueuedMessages();
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : "compaction failed";
    await emitSession({
      type: "compaction_end",
      reason,
      result: undefined,
      aborted: false,
      willRetry: false,
      errorMessage:
        reason === "overflow"
          ? `Context overflow recovery failed: ${errorMessage}`
          : `Auto-compaction failed: ${errorMessage}`,
    });
    return false;
  } finally {
    autoCompactionAbortController = undefined;
  }
}

async function checkCompaction(assistantMessage: AssistantMessage, skipAbortedCheck = true): Promise<boolean> {
  if (!compactionSettings.enabled) return false;
  if (skipAbortedCheck && assistantMessage.stopReason === "aborted") return false;
  const contextWindow = scenario.model.contextWindow ?? 0;
  const sameModel = assistantMessage.provider === scenario.model.provider && assistantMessage.model === scenario.model.id;
  const compactionEntry = getLatestCompactionEntry(sessionManager.getBranch());
  const assistantIsFromBeforeCompaction =
    compactionEntry !== null && assistantMessage.timestamp <= new Date(compactionEntry.timestamp).getTime();
  if (assistantIsFromBeforeCompaction) return false;

  if (sameModel && isContextOverflow(assistantMessage, contextWindow)) {
    if (overflowRecoveryAttempted) {
      await emitSession({
        type: "compaction_end",
        reason: "overflow",
        result: undefined,
        aborted: false,
        willRetry: false,
        errorMessage:
          "Context overflow recovery failed after one compact-and-retry attempt. Try reducing context or switching to a larger-context model.",
      });
      return false;
    }
    overflowRecoveryAttempted = true;
    const messages = agent.state.messages;
    if (messages.length > 0 && messages[messages.length - 1].role === "assistant") {
      agent.state.messages = messages.slice(0, -1);
    }
    return await runAutoCompaction("overflow", true);
  }

  let contextTokens: number;
  if (assistantMessage.stopReason === "error") {
    const messages = agent.state.messages;
    const estimate = estimateContextTokens(messages);
    if (estimate.lastUsageIndex === null) return false;
    const usageMsg = messages[estimate.lastUsageIndex];
    if (
      compactionEntry &&
      usageMsg.role === "assistant" &&
      (usageMsg as AssistantMessage).timestamp <= new Date(compactionEntry.timestamp).getTime()
    ) {
      return false;
    }
    contextTokens = estimate.tokens;
  } else {
    contextTokens = calculateContextTokens(assistantMessage.usage);
  }
  if (shouldCompact(contextTokens, contextWindow, compactionSettings)) {
    return await runAutoCompaction("threshold", false);
  }
  return false;
}

async function handlePostAgentRun(): Promise<boolean> {
  const msg = lastAssistantMessage;
  lastAssistantMessage = undefined;
  if (!msg) return false;
  if (await checkCompaction(msg)) return true;
  return agent.hasQueuedMessages();
}

async function runAgentPrompt(text: string): Promise<void> {
  await agent.prompt(text);
  while (await handlePostAgentRun()) {
    await agent.continue();
  }
}

async function promptWithChecks(text: string): Promise<void> {
  const lastAssistant = findLastAssistantMessage();
  if (lastAssistant && (await checkCompaction(lastAssistant, false))) {
    await agent.continue();
    while (await handlePostAgentRun()) {
      await agent.continue();
    }
  }
  await runAgentPrompt(text);
}

// --- interactive-mode.ts compaction handlers ---
let autoCompactionLoader: Loader | undefined;
let autoCompactionEscapeHandler: (() => void) | undefined;
let pendingPrompt: Promise<void> | undefined;
let pendingCompact: Promise<void> | undefined;

function queueCompactionMessage(text: string, mode: "steer" | "followUp"): void {
  compactionQueuedMessages.push({ text, mode });
  editor.addToHistory?.(text);
  editor.setText("");
  updatePendingMessagesDisplay();
  showStatus("Queued message for after compaction");
}

async function flushCompactionQueue(options?: { willRetry?: boolean }): Promise<void> {
  if (compactionQueuedMessages.length === 0) return;
  const queuedMessages = [...compactionQueuedMessages];
  compactionQueuedMessages = [];
  updatePendingMessagesDisplay();
  if (options?.willRetry) {
    for (const message of queuedMessages) {
      if (message.mode === "followUp") {
        followUpTexts.push(message.text);
        await agent.followUp({ role: "user", content: [{ type: "text", text: message.text }], timestamp: Date.now() } as never);
      } else {
        steeringTexts.push(message.text);
        await agent.steer({ role: "user", content: [{ type: "text", text: message.text }], timestamp: Date.now() } as never);
      }
    }
    updatePendingMessagesDisplay();
    return;
  }
  const first = queuedMessages[0]!;
  pendingPrompt = promptWithChecks(first.text);
  for (const message of queuedMessages.slice(1)) {
    if (message.mode === "followUp") {
      followUpTexts.push(message.text);
      await agent.followUp({ role: "user", content: [{ type: "text", text: message.text }], timestamp: Date.now() } as never);
    } else {
      steeringTexts.push(message.text);
      await agent.steer({ role: "user", content: [{ type: "text", text: message.text }], timestamp: Date.now() } as never);
    }
  }
  updatePendingMessagesDisplay();
}

function handleCompactionEvent(event: SessionEvent): void {
  switch (event.type) {
    case "compaction_start": {
      autoCompactionEscapeHandler = editor.onEscape;
      editor.onEscape = () => {
        abortCompaction();
      };
      statusContainer.clear();
      const cancelHint = `(${keyText("app.interrupt")} to cancel)`;
      const label =
        event.reason === "manual"
          ? `Compacting context... ${cancelHint}`
          : `${event.reason === "overflow" ? "Context overflow detected, " : ""}Auto-compacting... ${cancelHint}`;
      autoCompactionLoader = new Loader(
        ui,
        (spinner) => theme.fg("accent", spinner),
        (text) => theme.fg("muted", text),
        label,
      );
      autoCompactionLoader.stop();
      statusContainer.addChild(autoCompactionLoader);
      ui.requestRender();
      break;
    }
    case "compaction_end": {
      if (autoCompactionEscapeHandler) {
        editor.onEscape = autoCompactionEscapeHandler;
        autoCompactionEscapeHandler = undefined;
      }
      if (autoCompactionLoader) {
        autoCompactionLoader.stop();
        autoCompactionLoader = undefined;
        statusContainer.clear();
      }
      if (event.aborted) {
        if (event.reason === "manual") {
          showError("Compaction cancelled");
        } else {
          showStatus("Auto-compaction cancelled");
        }
      } else if (event.result) {
        const result = event.result as { summary: string; tokensBefore: number };
        chatContainer.clear();
        rebuildChatFromMessages();
        addMessageToChat(
          createCompactionSummaryMessage(result.summary, result.tokensBefore, new Date().toISOString()) as never,
        );
        footer.invalidate();
      } else if (event.errorMessage) {
        if (event.reason === "manual") {
          showError(event.errorMessage);
        } else {
          chatContainer.addChild(new Spacer(1));
          chatContainer.addChild(new Text(theme.fg("error", event.errorMessage), 1, 0));
        }
      }
      void flushCompactionQueue({ willRetry: event.willRetry });
      ui.requestRender();
      break;
    }
    default:
      break;
  }
}

// interactive-mode.ts handleCompactCommand.
async function handleCompactCommand(customInstructions?: string): Promise<void> {
  const entries = sessionManager.getEntries();
  const messageCount = entries.filter((e) => e.type === "message").length;
  if (messageCount < 2) {
    showWarning("Nothing to compact (no messages yet)");
    return;
  }
  if (loadingAnimation) {
    loadingAnimation.stop();
    loadingAnimation = undefined;
  }
  statusContainer.clear();
  try {
    await compactSession(customInstructions);
  } catch {
    // Ignore, surfaced as an event.
  }
}

// --- provider-turn handleEvent bodies over the real agent ---
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

function handleEvent(event: never): void {
  const e = event as {
    type: string;
    message?: AgentMessage & { stopReason?: string; errorMessage?: string };
    toolCallId?: string;
    toolName?: string;
    args?: unknown;
    partialResult?: { content: unknown; details?: unknown };
    result?: { content: unknown; details?: unknown };
    isError?: boolean;
  };
  footer.invalidate();
  switch (e.type) {
    case "agent_start":
      pendingTools.clear();
      if (loadingAnimation) {
        loadingAnimation.stop();
        loadingAnimation = undefined;
        statusContainer.clear();
      }
      startWorkingLoader();
      ui.requestRender();
      break;
    case "message_start":
      if (e.message?.role === "user") {
        addMessageToChat(e.message);
        ui.requestRender();
      } else if (e.message?.role === "assistant") {
        streamingComponent = new AssistantMessageComponent(undefined, false);
        chatContainer.addChild(streamingComponent);
        streamingComponent.updateContent(e.message as never);
        ui.requestRender();
      }
      break;
    case "message_update":
      if (streamingComponent && e.message?.role === "assistant") {
        streamingComponent.updateContent(e.message as never);
        ui.requestRender();
      }
      break;
    case "message_end": {
      if (e.message?.role === "user") break;
      if (streamingComponent && e.message?.role === "assistant") {
        let errorMessage: string | undefined;
        if (e.message.stopReason === "aborted") {
          errorMessage = "Operation aborted";
          e.message.errorMessage = errorMessage;
        }
        streamingComponent.updateContent(e.message as never);
        if (e.message.stopReason === "aborted" || e.message.stopReason === "error") {
          if (!errorMessage) errorMessage = e.message.errorMessage || "Error";
          for (const [, component] of pendingTools.entries()) {
            component.updateResult({ content: [{ type: "text", text: errorMessage }], isError: true } as never);
          }
          pendingTools.clear();
        }
        streamingComponent = undefined;
        footer.invalidate();
      }
      ui.requestRender();
      break;
    }
    case "agent_end":
      if (loadingAnimation) {
        loadingAnimation.stop();
        loadingAnimation = undefined;
        statusContainer.clear();
      }
      if (streamingComponent) {
        chatContainer.removeChild(streamingComponent);
        streamingComponent = undefined;
      }
      pendingTools.clear();
      ui.requestRender();
      break;
    default:
      break;
  }
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
editor.onEscape = () => {
  if (agent.state.isStreaming) agent.abort();
};

// setupEditorSubmitHandler: the reachable slice (/compact + queueing).
editor.onSubmit = (text: string) => {
  text = text.trim();
  if (!text) return;
  if (text === "/compact" || text.startsWith("/compact ")) {
    const customInstructions = text.startsWith("/compact ") ? text.slice(9).trim() : undefined;
    editor.setText("");
    pendingCompact = handleCompactCommand(customInstructions);
    return;
  }
  if (isCompacting()) {
    queueCompactionMessage(text, "steer");
    return;
  }
  editor.addToHistory?.(text);
  editor.setText("");
  if (agent.state.isStreaming) {
    steeringTexts.push(text);
    void agent.steer({ role: "user", content: [{ type: "text", text }], timestamp: Date.now() } as never);
    updatePendingMessagesDisplay();
    return;
  }
  pendingPrompt = promptWithChecks(text);
};

// --- capture machinery (provider-turn style, compaction events included) ---
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force);
  await sleep(20);
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}

type ArmedCapture = Capture & { seen?: number; fired?: boolean };
let triggers: ArmedCapture[] = [];
async function runTriggers(eventType: string): Promise<void> {
  for (const trigger of triggers) {
    if (!trigger.fired && eventType === trigger.event) {
      trigger.seen = (trigger.seen ?? 0) + 1;
      if (trigger.seen >= (trigger.count ?? 1)) {
        trigger.fired = true;
        if (trigger.action === "submit") {
          terminal.send(`\x1b[200~${trigger.text ?? ""}\x1b[201~`);
          terminal.send("\r");
          if (trigger.name) await capture(trigger.name);
        } else {
          if (trigger.name) await capture(trigger.name);
          if (trigger.action === "escape") editor.onEscape?.();
        }
      }
    }
  }
}

async function emitSession(event: SessionEvent): Promise<void> {
  handleCompactionEvent(event);
  await runTriggers(event.type);
}

agent.subscribe(async (event) => {
  const e = event as { type: string; message?: AgentMessage };
  // agent-session.ts _handleAgentEvent: persistence + compaction
  // bookkeeping, then the UI handler.
  if (e.type === "message_start" && e.message?.role === "user") {
    overflowRecoveryAttempted = false;
  }
  if (e.type === "message_end" && e.message) {
    if (e.message.role === "user" || e.message.role === "assistant" || e.message.role === "toolResult") {
      sessionManager.appendMessage(e.message as never);
    }
    if (e.message.role === "assistant") {
      lastAssistantMessage = e.message as AssistantMessage;
      if ((e.message as AssistantMessage).stopReason !== "error") {
        overflowRecoveryAttempted = false;
      }
    }
  }
  handleEvent(event as never);
  await runTriggers(e.type);
});

async function main() {
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  model = {
    ...scenario.model,
    baseUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
  };
  agent.state.model = model as never;
  renderInitialMessages();
  await capture("startup", true);
  for (const step of scenario.steps) {
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    if (step.captures) triggers = step.captures as ArmedCapture[];
    for (const data of step.input ?? []) {
      terminal.send(data);
      await sleep(25);
    }
    if (step.settle !== false) {
      // Settle chained work: a compaction_end flush can start a prompt
      // turn while the compact promise resolves.
      for (let i = 0; i < 6; i++) {
        await pendingCompact;
        await pendingPrompt;
        await agent.waitForIdle();
        await sleep(5);
      }
    }
    if (step.name) await capture(step.name, Boolean(step.resize));
  }
  ui.stop();
  server.close();
  server.closeAllConnections?.();
  process.stdout.write(JSON.stringify({ frames }));
  process.exit(0);
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
