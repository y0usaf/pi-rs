// Real-stack provider driver (PLAN 4.2): Pi's real Agent + streamAnthropic
// against the scenario's scripted local SSE stub, wired into the
// interactive-mode container composition with the copied handleEvent
// bodies. Frames are captured at exact agent-event points from the awaited
// subscribe listener — the same points pi-rs's product sequence captures at.
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Agent } from "../../ref/pi/packages/agent/src/agent.ts";
import type { Model } from "../../ref/pi/packages/ai/src/models.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { estimateContextTokens } from "../../ref/pi/packages/coding-agent/src/core/compaction/compaction.ts";
import {
  createCodingToolDefinitions,
  createCodingTools,
  type ToolDef,
} from "../../ref/pi/packages/coding-agent/src/core/tools/index.ts";
import type { AgentSession } from "../../ref/pi/packages/coding-agent/src/core/agent-session.ts";
import type { ReadonlyFooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";
import { AssistantMessageComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/assistant-message.ts";
import { CustomEditor } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/custom-editor.ts";
import { FooterComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/footer.ts";
import {
  keyHint,
  keyText,
  rawKeyHint,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts";
import { ToolExecutionComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/tool-execution.ts";
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
  // Pacing (pi side only): stop the body after events[afterEvent] until
  // the named capture lands. Pi's provider consumes buffered SSE eagerly
  // and mutates the shared partial, so a fully-buffered body would render
  // completed text in "mid-stream" frames; withholding bytes reproduces
  // real streaming pace. pi-rs snapshots per event and needs no pacing.
  pauseAfter?: Array<{ afterEvent: number; untilCapture: string }>;
};
type Capture = { name: string; event: string; role?: string; count?: number; action?: string };
type Step = {
  name?: string;
  input?: string[];
  resize?: { columns: number; rows: number };
  captures?: Capture[];
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

// Deterministic environment: fixed capabilities and the scenario file tree
// as cwd/HOME (the footer shows "~"; the Rust harness pins the same).
setCapabilities({ images: null, trueColor: true, hyperlinks: false });
const cwd = mkdtempSync(join(tmpdir(), "pi-rs-ui-parity-"));
for (const [name, contents] of Object.entries(scenario.files ?? {})) {
  writeFileSync(join(cwd, name), contents);
}
if (scenario.homeFromCwd) process.env.HOME = cwd;
const keybindings = new KeybindingsManager();
setKeybindings(keybindings);
initTheme("dark", false);

// --- Scripted provider stub (gen-oracle.ts serveCase, without capture) ---
function sseBody(events: SseEvent[]): string {
  return events.map((e) => `event: ${e.event}\ndata: ${JSON.stringify(e.data)}\n\n`).join("");
}
function responseBody(response: ScriptedResponse): { body: string; contentType: string } {
  const events = response.sse ? scenario.stub.sse![response.sse]! : response.events;
  if (events) return { body: sseBody(events), contentType: "text/event-stream" };
  if (response.json !== undefined) return { body: JSON.stringify(response.json), contentType: "application/json" };
  return { body: response.text ?? "", contentType: "text/plain" };
}
// Capture gates for stub pacing: a paused response resumes when the
// capture it waits on has landed.
const captureGates = new Map<string, { promise: Promise<void>; resolve: () => void }>();
function gateFor(name: string): { promise: Promise<void>; resolve: () => void } {
  let gate = captureGates.get(name);
  if (!gate) {
    let resolve!: () => void;
    const promise = new Promise<void>((r) => {
      resolve = r;
    });
    gate = { promise, resolve };
    captureGates.set(name, gate);
  }
  return gate;
}
let responseIndex = 0;
const server = createServer((req, res) => {
  req.on("data", () => {});
  req.on("end", () => {
    void (async () => {
      const responses = scenario.stub.responses;
      const scripted = responses[responseIndex] ?? responses[responses.length - 1];
      responseIndex += 1;
      if (!scripted) {
        res.writeHead(500).end("no scripted response");
        return;
      }
      const events = scripted.sse ? scenario.stub.sse![scripted.sse]! : scripted.events;
      if (events && scripted.pauseAfter?.length) {
        res.writeHead(scripted.status, { "content-type": "text/event-stream" });
        let start = 0;
        for (const pause of scripted.pauseAfter) {
          res.write(sseBody(events.slice(start, pause.afterEvent + 1)));
          await gateFor(pause.untilCapture).promise;
          start = pause.afterEvent + 1;
        }
        res.write(sseBody(events.slice(start)));
        if (!scripted.hang) res.end();
        return;
      }
      const { body, contentType } = responseBody(scripted);
      res.writeHead(scripted.status, { "content-type": contentType });
      if (scripted.hang) {
        res.write(body); // Hold the connection open; the driver aborts.
      } else {
        res.end(body);
      }
    })();
  });
});
// Real agent over real coding tools; the model's baseUrl points at the
// stub once it is listening (assigned at the top of main — tsx compiles
// to cjs, so no top-level await).
let model: Model<"anthropic-messages">;
const agent = new Agent({
  initialState: { model: scenario.model, tools: createCodingTools(cwd) },
  getApiKey: () => scenario.apiKey,
});
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
  // agent-session.ts getContextUsage() without a compaction boundary.
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
const pendingTools = new Map<string, ToolExecutionComponent>();
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
  // Stopped immediately so the interval cannot advance the spinner between
  // captures; the displayed cells equal a frame-0 capture on both sides.
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
    message?: { role: string; content: Array<{ type: string; id?: string; name?: string; arguments?: unknown; text?: string }>; stopReason?: string; errorMessage?: string };
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
        for (const content of e.message.content) {
          if (content.type === "toolCall" && content.id) {
            if (!pendingTools.has(content.id)) {
              const component = new ToolExecutionComponent(
                content.name!,
                content.id,
                content.arguments,
                {},
                getRegisteredToolDefinition(content.name!),
                ui,
                cwd,
              );
              component.setExpanded(toolOutputExpanded);
              chatContainer.addChild(component);
              pendingTools.set(content.id, component);
            } else {
              pendingTools.get(content.id)?.updateArgs(content.arguments);
            }
          }
        }
        ui.requestRender();
      }
      break;
    case "message_end": {
      if (e.message?.role === "user") break;
      if (streamingComponent && e.message?.role === "assistant") {
        streamingMessage = e.message as never;
        let errorMessage: string | undefined;
        if (e.message.stopReason === "aborted") {
          errorMessage = "Operation aborted";
          e.message.errorMessage = errorMessage;
        }
        streamingComponent.updateContent(streamingMessage as never);
        if (e.message.stopReason === "aborted" || e.message.stopReason === "error") {
          if (!errorMessage) {
            errorMessage = e.message.errorMessage || "Error";
          }
          for (const [, component] of pendingTools.entries()) {
            component.updateResult({
              content: [{ type: "text", text: errorMessage }],
              isError: true,
            } as never);
          }
          pendingTools.clear();
        } else {
          for (const [, component] of pendingTools.entries()) {
            component.setArgsComplete();
          }
        }
        streamingComponent = undefined;
        streamingMessage = undefined;
        footer.invalidate();
      }
      ui.requestRender();
      break;
    }
    case "tool_execution_start": {
      let component = pendingTools.get(e.toolCallId!);
      if (!component) {
        component = new ToolExecutionComponent(
          e.toolName!,
          e.toolCallId!,
          e.args,
          {},
          getRegisteredToolDefinition(e.toolName!),
          ui,
          cwd,
        );
        component.setExpanded(toolOutputExpanded);
        chatContainer.addChild(component);
        pendingTools.set(e.toolCallId!, component);
      }
      component.markExecutionStarted();
      ui.requestRender();
      break;
    }
    case "tool_execution_update": {
      const component = pendingTools.get(e.toolCallId!);
      if (component) {
        component.updateResult({ ...e.partialResult, isError: false } as never, true);
        ui.requestRender();
      }
      break;
    }
    case "tool_execution_end": {
      const component = pendingTools.get(e.toolCallId!);
      if (component) {
        component.updateResult({ ...e.result, isError: e.isError } as never);
        pendingTools.delete(e.toolCallId!);
        ui.requestRender();
      }
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
      pendingTools.clear();
      ui.requestRender();
      break;
  }
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

// interactive-mode.ts setupKeyHandlers onEscape (streaming branch: empty
// queues restore nothing and the agent aborts).
function handleEscape(): void {
  if (agent.state.isStreaming) {
    agent.abort();
  }
}
editor.onEscape = handleEscape;
editor.onAction("app.tools.expand", () => setToolsExpanded(!toolOutputExpanded));

// setupEditorSubmitHandler: the reachable slice (plain prompts only).
editor.onSubmit = (text: string) => {
  text = text.trim();
  if (!text) return;
  editor.addToHistory?.(text);
  if (agent.state.isStreaming) {
    void agent.steer({ role: "user", content: [{ type: "text", text }], timestamp: Date.now() } as never);
    return;
  }
  void agent.prompt(text);
};

const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
  gateFor(name).resolve();
}

// Event-triggered captures: the listener is awaited by the agent, so the
// stream cannot advance past a capture (pi-rs's synchronous listener has the
// same property).
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
    // Settle the background turn; captures fire from the listener. The
    // editor submit handler kicked the prompt off without awaiting it.
    await agent.waitForIdle();
    if (step.name) await capture(step.name, Boolean(step.resize));
  }
  ui.stop();
  server.close();
  server.closeAllConnections?.();
  process.stdout.write(JSON.stringify({ frames }));
  process.exit(0);
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
