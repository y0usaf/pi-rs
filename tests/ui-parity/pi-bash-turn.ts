// Real-stack bash-mode driver (PLAN 7.1): Pi's real BashExecutionComponent
// + executeBashWithOperations/createLocalBashOperations over real
// subprocesses, wired into the interactive-mode composition with the
// copied `!`/`!!` bodies (isBashMode border, handleBashCommand, executeBash
// / recordBashResult / abortBash, flushPendingBashComponents), plus the
// real Agent against the scenario's SSE stub for the deferred-during-
// streaming section. Component loaders are stopped on construction so the
// interval cannot advance the spinner between captures (frame-0 on both
// sides, like the working loader).
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Agent } from "../../ref/pi/packages/agent/src/agent.ts";
import type { Model } from "../../ref/pi/packages/ai/src/models.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { estimateContextTokens } from "../../ref/pi/packages/coding-agent/src/core/compaction/compaction.ts";
import { type BashResult, executeBashWithOperations } from "../../ref/pi/packages/coding-agent/src/core/bash-executor.ts";
import { createLocalBashOperations } from "../../ref/pi/packages/coding-agent/src/core/tools/bash.ts";
import type { TruncationResult } from "../../ref/pi/packages/coding-agent/src/core/tools/truncate.ts";
import { createCodingTools } from "../../ref/pi/packages/coding-agent/src/core/tools/index.ts";
import { convertToLlm } from "../../ref/pi/packages/coding-agent/src/core/messages.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import { AssistantMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/assistant-message.ts";
import { BashExecutionComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/bash-execution.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import {
  keyHint,
  keyText,
  rawKeyHint,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts";
import { UserMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/user-message.ts";
import { getEditorTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Loader } from "../../ref/pi/packages/tui/src/components/loader.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type SseEvent = { event: string; data: unknown };
type ScriptedResponse = {
  status: number;
  sse?: string;
  events?: SseEvent[];
  json?: unknown;
  text?: string;
  hang?: boolean;
};
type Capture = { name: string; event: string; role?: string; count?: number; action?: string };
type Step = {
  name?: string;
  input?: string[];
  resize?: { columns: number; rows: number };
  captures?: Capture[];
  waitBash?: boolean;
  waitIdle?: boolean;
};
type Scenario = {
  columns: number;
  rows: number;
  appName: string;
  version: string;
  branch: string;
  homeFromCwd?: boolean;
  providerCount: number;
  apiKey: string;
  files?: Record<string, string>;
  model: Model<"anthropic-messages">;
  stub: { sse?: Record<string, SseEvent[]>; responses: ScriptedResponse[] };
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

setCapabilities({ images: null, trueColor: true, hyperlinks: false });
const cwd = mkdtempSync(join(tmpdir(), "pi-rs-ui-parity-"));
for (const [name, contents] of Object.entries(scenario.files ?? {})) {
  writeFileSync(join(cwd, name), contents);
}
if (scenario.homeFromCwd) process.env.HOME = cwd;
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
initTheme("dark", false);

// --- Scripted provider stub (pi-provider-turn.ts, without pacing) ---
function sseBody(events: SseEvent[]): string {
  return events.map((e) => `event: ${e.event}\ndata: ${JSON.stringify(e.data)}\n\n`).join("");
}
function responseBody(response: ScriptedResponse): { body: string; contentType: string } {
  const events = response.sse ? scenario.stub.sse![response.sse]! : response.events;
  if (events) return { body: sseBody(events), contentType: "text/event-stream" };
  if (response.json !== undefined) return { body: JSON.stringify(response.json), contentType: "application/json" };
  return { body: response.text ?? "", contentType: "text/plain" };
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
    const { body, contentType } = responseBody(scripted);
    res.writeHead(scripted.status, { "content-type": contentType });
    if (scripted.hang) {
      res.write(body); // Hold the connection open; the driver aborts.
    } else {
      res.end(body);
    }
  });
});
let model: Model<"anthropic-messages">;
const agent = new Agent({
  initialState: { model: scenario.model, tools: createCodingTools(cwd) },
  // sdk.ts wires messages.ts convertToLlm so bashExecution rows reach the
  // provider as their text form (pi-rs's product does the same).
  convertToLlm,
  getApiKey: () => scenario.apiKey,
});

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

// FooterComponent over the real agent state (AgentSession's slice of it).
const sessionStub = {
  get state() {
    return agent.state as never;
  },
  sessionManager: {
    getEntries: () => agent.state.messages.map((message) => ({ type: "message", message })),
    getCwd: () => cwd,
    getSessionName: () => undefined,
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

// --- interactive-mode.ts handleEvent bodies over the real agent ---
let streamingComponent: AssistantMessageComponent | undefined;
let streamingMessage: import("../../ref/pi/packages/agent/src/types.ts").AgentMessage | undefined;
let loadingAnimation: Loader | undefined;
let toolOutputExpanded = false;

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

function addUserMessageToChat(text: string): void {
  if (chatContainer.children.length > 0) {
    chatContainer.addChild(new Spacer(1));
  }
  chatContainer.addChild(new UserMessageComponent(text));
}

function handleEvent(event: never): void {
  const e = event as {
    type: string;
    message?: { role: string; content: Array<{ type: string; text?: string }>; stopReason?: string; errorMessage?: string };
  };
  footer.invalidate();
  switch (e.type) {
    case "agent_start":
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
        const text = e.message.content
          .filter((block) => block.type === "text")
          .map((block) => block.text ?? "")
          .join("\n");
        addUserMessageToChat(text);
        ui.requestRender();
      } else if (e.message?.role === "assistant") {
        streamingComponent = new AssistantMessageComponent(undefined, false);
        streamingMessage = e.message as never;
        chatContainer.addChild(streamingComponent);
        streamingComponent.updateContent(streamingMessage as never);
        ui.requestRender();
      }
      break;
    case "message_update":
      if (streamingComponent && e.message?.role === "assistant") {
        streamingMessage = e.message as never;
        streamingComponent.updateContent(streamingMessage as never);
        ui.requestRender();
      }
      break;
    case "message_end": {
      if (e.message?.role === "user") break;
      if (streamingComponent && e.message?.role === "assistant") {
        streamingMessage = e.message as never;
        if (e.message.stopReason === "aborted") {
          e.message.errorMessage = "Operation aborted";
        }
        streamingComponent.updateContent(streamingMessage as never);
        streamingComponent = undefined;
        streamingMessage = undefined;
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
        streamingMessage = undefined;
      }
      ui.requestRender();
      break;
  }
}

// interactive-mode.ts showWarning / showError.
function showWarning(warningMessage: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("warning", `Warning: ${warningMessage}`), 1, 0));
  ui.requestRender();
}
function showError(errorMessage: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("error", `Error: ${errorMessage}`), 1, 0));
  chatContainer.addChild(new Spacer(1));
  ui.requestRender();
}

// --- The `!`/`!!` bash surface: copied interactive-mode.ts +
// agent-session.ts bodies over the real executor and component ---
let isBashMode = false;
let bashComponent: BashExecutionComponent | undefined;
const pendingBashComponents: BashExecutionComponent[] = [];
let bashAbortController: AbortController | undefined;
const pendingBashMessages: object[] = [];
let lastBashPromise: Promise<void> | undefined;

function updateEditorBorderColor(): void {
  if (isBashMode) {
    editor.borderColor = theme.getBashModeBorderColor();
  } else {
    editor.borderColor = theme.getThinkingBorderColor("off");
  }
  ui.requestRender();
}

editor.onChange = (text: string) => {
  const wasBashMode = isBashMode;
  isBashMode = text.trimStart().startsWith("!");
  if (wasBashMode !== isBashMode) {
    updateEditorBorderColor();
  }
};

const isBashRunning = () => bashAbortController !== undefined;
function abortBash(): void {
  bashAbortController?.abort();
}

// agent-session.ts recordBashResult + _flushPendingBashMessages.
function recordBashResult(command: string, result: BashResult, options?: { excludeFromContext?: boolean }): void {
  const bashMessage = {
    role: "bashExecution",
    command,
    output: result.output,
    exitCode: result.exitCode,
    cancelled: result.cancelled,
    truncated: result.truncated,
    fullOutputPath: result.fullOutputPath,
    timestamp: Date.now(),
    excludeFromContext: options?.excludeFromContext,
  };
  if (agent.state.isStreaming) {
    pendingBashMessages.push(bashMessage);
  } else {
    agent.state.messages.push(bashMessage as never);
  }
}
function flushPendingBashMessages(): void {
  for (const message of pendingBashMessages) {
    agent.state.messages.push(message as never);
  }
  pendingBashMessages.length = 0;
}

// agent-session.ts executeBash (no settings manager: prefix/shellPath
// resolve to undefined, like a default settings file).
async function executeBash(
  command: string,
  onChunk?: (chunk: string) => void,
  options?: { excludeFromContext?: boolean },
): Promise<BashResult> {
  bashAbortController = new AbortController();
  try {
    const result = await executeBashWithOperations(command, cwd, createLocalBashOperations({}), {
      onChunk,
      signal: bashAbortController.signal,
    });
    recordBashResult(command, result, options);
    return result;
  } finally {
    bashAbortController = undefined;
  }
}

// interactive-mode.ts handleBashCommand (normal execution path; the
// user_bash extension event has no listeners here).
async function handleBashCommand(command: string, excludeFromContext = false): Promise<void> {
  const isDeferred = agent.state.isStreaming;
  bashComponent = new BashExecutionComponent(command, ui, excludeFromContext);
  // Determinism: stop the component's animation interval; the spinner
  // stays at frame 0 exactly like pi-rs's tick-less parity sequence.
  (bashComponent as unknown as { loader: Loader }).loader.stop();
  if (isDeferred) {
    pendingMessagesContainer.addChild(bashComponent);
    pendingBashComponents.push(bashComponent);
  } else {
    chatContainer.addChild(bashComponent);
  }
  ui.requestRender();
  try {
    const result = await executeBash(
      command,
      (chunk) => {
        if (bashComponent) {
          bashComponent.appendOutput(chunk);
          ui.requestRender();
        }
      },
      { excludeFromContext },
    );
    if (bashComponent) {
      bashComponent.setComplete(
        result.exitCode,
        result.cancelled,
        result.truncated ? ({ truncated: true, content: result.output } as TruncationResult) : undefined,
        result.fullOutputPath,
      );
    }
  } catch (error) {
    if (bashComponent) {
      bashComponent.setComplete(undefined, false);
    }
    showError(`Bash command failed: ${error instanceof Error ? error.message : "Unknown error"}`);
  }
  bashComponent = undefined;
  ui.requestRender();
}

// interactive-mode.ts flushPendingBashComponents.
function flushPendingBashComponents(): void {
  for (const component of pendingBashComponents) {
    pendingMessagesContainer.removeChild(component);
    chatContainer.addChild(component);
  }
  pendingBashComponents.length = 0;
}

// interactive-mode.ts updatePendingMessagesDisplay (no queue rows in this
// fixture; the clear drops deferred bash components from the display).
function updatePendingMessagesDisplay(): void {
  pendingMessagesContainer.clear();
}

// interactive-mode.ts restoreQueuedMessagesToEditor (empty queues here).
function restoreQueuedMessagesToEditor(options?: { abort?: boolean }): number {
  updatePendingMessagesDisplay();
  if (options?.abort) agent.abort();
  return 0;
}

// interactive-mode.ts setToolsExpanded (active header + expandable children).
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

// interactive-mode.ts setupKeyHandlers onEscape (streaming / bash branches).
function handleEscape(): void {
  if (agent.state.isStreaming) {
    restoreQueuedMessagesToEditor({ abort: true });
  } else if (isBashRunning()) {
    abortBash();
  } else if (isBashMode) {
    editor.setText("");
    isBashMode = false;
    updateEditorBorderColor();
  }
}
editor.onEscape = handleEscape;
editor.onAction("app.tools.expand", () => setToolsExpanded(!toolOutputExpanded));

// interactive-mode.ts handleCtrlC / clearEditor.
let lastSigintTime = 0;
let exited = false;
editor.onAction("app.clear", () => {
  const now = Date.now();
  if (now - lastSigintTime < 500) {
    exited = true;
  } else {
    editor.setText("");
    ui.requestRender();
    lastSigintTime = now;
  }
});
void exited;

// setupEditorSubmitHandler: the `!`/`!!` branch plus the plain prompt path.
editor.onSubmit = (text: string) => {
  text = text.trim();
  if (!text) return;
  if (text.startsWith("!")) {
    const isExcluded = text.startsWith("!!");
    const command = isExcluded ? text.slice(2).trim() : text.slice(1).trim();
    if (command) {
      if (isBashRunning()) {
        showWarning("A bash command is already running. Press Esc to cancel it first.");
        editor.setText(text);
        return;
      }
      editor.addToHistory?.(text);
      lastBashPromise = handleBashCommand(command, isExcluded).then(() => {
        isBashMode = false;
        updateEditorBorderColor();
      });
      return;
    }
  }
  editor.addToHistory?.(text);
  if (agent.state.isStreaming) {
    void agent.steer({ role: "user", content: [{ type: "text", text }], timestamp: Date.now() } as never);
    return;
  }
  flushPendingBashComponents();
  // _runAgentPrompt's finally flushes bash messages queued mid-turn.
  void agent.prompt(text).finally(() => flushPendingBashMessages());
};

const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
let capturesInFlight = 0;
async function capture(name: string, force = false) {
  capturesInFlight += 1;
  ui.requestRender(force);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
  capturesInFlight -= 1;
}

type ArmedCapture = Capture & { seen?: number; fired?: boolean };
let triggers: ArmedCapture[] = [];
agent.subscribe(async (event) => {
  handleEvent(event as never);
  for (const trigger of triggers) {
    if (
      !trigger.fired &&
      (event as { type: string }).type === trigger.event &&
      (trigger.role === undefined ||
        (event as { message?: { role?: string } }).message?.role === trigger.role)
    ) {
      trigger.seen = (trigger.seen ?? 0) + 1;
      if (trigger.seen >= (trigger.count ?? 1)) {
        trigger.fired = true;
        await capture(trigger.name);
        if (trigger.action === "escape") handleEscape();
      }
    }
  }
});

async function main() {
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  model = {
    ...scenario.model,
    baseUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
  };
  agent.state.model = model as never;
  await capture("startup", true);
  for (const step of scenario.steps) {
    if (step.resize) terminal.resize(step.resize.columns, step.resize.rows);
    triggers = (step.captures ?? []) as ArmedCapture[];
    for (const data of step.input ?? []) terminal.send(data);
    if (step.waitBash && lastBashPromise) {
      await lastBashPromise;
      lastBashPromise = undefined;
    }
    if (step.waitIdle === false) {
      // A hanging turn stays in flight; poll until the armed captures
      // have fired (pi-rs's parity sequence pumps its LocalSet the same way).
      let budget = 0;
      while (
        (triggers.some((trigger) => !trigger.fired) || capturesInFlight > 0) &&
        budget < 5000
      ) {
        await new Promise<void>((resolve) => setTimeout(resolve, 1));
        budget += 1;
      }
    } else {
      await agent.waitForIdle();
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
